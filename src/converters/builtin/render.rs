//! Medya planlama ve indirme katmanı (kart **yalnızca** Components V2 olarak gönderilir).
//!
//! Gönderinin medyası (video/görsel/GIF) bütçelerle paralel indirilir; indirilen dosyalar
//! `attachment://` referansıyla kartın **Media Gallery** bileşenine bağlanır. Klasik
//! `CreateEmbed` düzeni kaldırılmıştır.

use std::time::Duration;

use serenity::all::CreateAttachment;
use tracing::debug;

use super::media::{self, DISCORD_FILE_LIMIT};
use super::{BuiltinPost, Media, MediaKind};
use crate::settings::EngineSettings;

/// Video **birleştirme** bütçesi.
///
/// Kart "anında" görünmelidir; yalnızca sesi ayrı akışta gelen platformlarda (Reddit) video
/// indirilip sesiyle birleştirilir ve bu iş bütçeyi aşarsa video kartta uzak bağlantıyla
/// (sessiz) oynar. Diğer tüm medya **hiç indirilmez** — CDN bağlantısı yeterlidir.
pub const VIDEO_MERGE_BUDGET: Duration = Duration::from_millis(5_000);

/// Videosu **ayrı ses akışıyla** sunulan platformlar (Reddit): video ve ses ayrı medya
/// playlist'lerinde gelir, sesli oynatma ancak ikisi indirilip tek MP4'e birleştirilerek
/// mümkündür. Bu yüzden bu platformlarda video indirme **medya yöntemi ayarından bağımsız**
/// olarak zorunludur (görseller ayarı izlemeye devam eder).
fn video_requires_download(platform_key: &str) -> bool {
    platform_key == "reddit"
}

/// Ayarlardan türetilen render davranışı (**tek mod: doğrudan**).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderOptions {
    /// Video **zorunlu** indirilir (platform gereği): Reddit'te ses ayrı akışta tutulduğu
    /// için kart içi video ancak video+ses birleştirilerek sesli oynar.
    pub video_requires_download: bool,
    pub max_images: u8,
    pub max_video_mb: u8,
    pub show_metrics: bool,
    pub show_timestamp: bool,
    pub show_community_note: bool,
    pub show_quote: bool,
}

impl RenderOptions {
    /// Platforma göre uyarlanmış render seçenekleri.
    ///
    /// Kart tek modda çalışır: **hiçbir medya indirilmez** (görseller/GIF'ler CDN
    /// bağlantısıyla kartın içinde gösterilir). **Tek istisna** sesi ayrı akışta gelen
    /// Reddit videosudur ([`Self::video_requires_download`]).
    pub fn from_engine(engine: &EngineSettings, platform_key: &str) -> Self {
        Self {
            video_requires_download: video_requires_download(platform_key),
            max_images: engine.max_images,
            max_video_mb: engine.max_video_mb,
            show_metrics: engine.show_metrics,
            show_timestamp: engine.show_timestamp,
            show_community_note: engine.show_community_note,
            show_quote: engine.show_quote,
        }
    }

    /// Video indirme üst sınırı: ayar ne olursa olsun Discord limitini aşmaz.
    pub fn max_video_bytes(&self) -> u64 {
        (self.max_video_mb as u64 * 1024 * 1024).min(DISCORD_FILE_LIMIT)
    }

    /// Video birleştirme bütçesi (yalnızca zorunlu platformlarda kullanılır).
    pub fn video_budget(&self) -> Duration {
        VIDEO_MERGE_BUDGET
    }
}

/// İndirme planı (**tek mod: doğrudan**): hangi medya kartın içinde gösterilecek.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaPlan {
    /// Kartta gösterilecek görsel URL'leri (CDN bağlantısı; indirilmez).
    pub remote_images: Vec<String>,
    /// Kartın içinde oynayacak video URL'i.
    pub video: Option<String>,
    /// Animasyonlu GIF kaynakları (X: sessiz mp4) → kartta gerçek `.gif` olarak gösterilir.
    pub gifs: Vec<String>,
}

/// Medya indirme sonucu.
#[derive(Debug, Clone, Default)]
pub struct ResolvedMedia {
    /// Embed'de gösterilecek görseller (`attachment://...` veya uzak URL).
    pub images: Vec<String>,
    /// Mesaja eklenecek dosyalar.
    pub attachments: Vec<CreateAttachment>,
    /// Video dosya olarak eklenebildiyse `attachment://` referansı.
    pub attached_video: Option<String>,
    /// Video eklenemediyse doğrudan bağlantı (mesaj metnine yazılır).
    pub video_link: Option<String>,
}

/// Gönderi medyasını indirme planına çevirir.
///
/// Gönderiye **hiç medya eklenmemişse** (alıntılayan kişi görsel/video eklemediyse) alıntı
/// **zincirindeki** ilk medya plana alınır: önce alıntılanan gönderi, o da medyasızsa onun
/// alıntısı (3 seviyeye kadar). X'in kendi kartı da böyle yapar: alıntılayan medya
/// eklemediyse alıntının (veya alıntının alıntısının) videosu kartın içinde oynar.
pub fn plan_media(post: &BuiltinPost, options: &RenderOptions) -> MediaPlan {
    let mut plan = MediaPlan::default();
    let max_images = options.max_images as usize;

    for media in &post.media {
        push_media(&mut plan, media, max_images);
    }

    let mut next = post.quote.as_deref();
    while let Some(candidate) = next {
        if !options.show_quote || !plan_is_empty(&plan) {
            break;
        }
        for media in &candidate.media {
            push_media(&mut plan, media, max_images);
        }
        next = candidate.quote.as_deref();
    }

    // Link kartı (X'in link önizlemesi): gönderide gösterilecek medya yoksa önizleme görseli
    // galeriye eklenir. İndirilmez: `og:image` adresi doğrudan gösterilir.
    if max_images > 0
        && plan_is_empty(&plan)
        && let Some(image) = post
            .link_preview
            .as_ref()
            .filter(|preview| !preview.is_empty())
            .and_then(|preview| preview.image.as_deref())
    {
        plan.remote_images.push(image.to_string());
    }

    plan
}

/// Planda gösterilecek medya var mı?
fn plan_is_empty(plan: &MediaPlan) -> bool {
    plan.video.is_none() && plan.gifs.is_empty() && plan.remote_images.is_empty()
}

/// Tek bir medyayı plana ekler: GIF'ler ayrı tutulur, ilk video kazanır, görseller
/// `max_images` ile sınırlanır.
fn push_media(plan: &mut MediaPlan, media: &Media, max_images: usize) {
    // Animasyonlu GIF: sessiz mp4 olarak indirilir ve gerçek `.gif`'e çevrilir
    // (video oynatıcısı çıkmasın diye video alanına konmaz).
    // Ancak servis yalnızca belirli platformları (örn. Twitter) destekler;
    // dönüştürülemeyen (Reddit vb.) mp4 formatındaki GIF'ler video olarak işlenir
    // ki kartta kırık görsel yerine oynatılabilir bir video çıksın.
    if media.kind == MediaKind::Gif
        && media::gif_convert_url(&media.url).is_some() {
            if plan.gifs.len() < max_images {
                plan.gifs.push(media.url.clone());
            }
            return;
        }
        // Servisin desteklemediği GIF (örn. Reddit i.redd.it/*.mp4): video olarak devam et.

    if media.is_video() {
        if plan.video.is_none() {
            plan.video = Some(media.url.clone());
        }
        return;
    }

    if plan.remote_images.len() >= max_images {
        return;
    }
    plan.remote_images.push(media.url.clone());
}

/// Planı uygular: **hiçbir medya indirilmez** (görseller/GIF'ler CDN bağlantısıyla kartta
/// gösterilir). **Tek istisna** sesi ayrı akışta gelen Reddit videosudur: sesli oynatma için
/// video+ses birleştirilir ve sonuç mesaja **ek** olarak gönderilir; bütçe dolarsa video
/// kartta uzak bağlantıyla (sessiz) oynar.
pub async fn resolve_media(
    client: &reqwest::Client,
    post: &BuiltinPost,
    plan: &MediaPlan,
    options: &RenderOptions,
) -> ResolvedMedia {
    let mut resolved = direct_media(plan);
    if !options.video_requires_download {
        return resolved;
    }
    let Some(video_url) = plan.video.as_ref() else {
        return resolved;
    };

    // `download_video`: Reddit videolarında video+ses birleştirmesini dener (ffmpeg'siz).
    let max_bytes = options.max_video_bytes();
    let handle = {
        let client = client.clone();
        let url = video_url.clone();
        tokio::spawn(async move { media::download_video(&client, &url, max_bytes).await })
    };

    match within(options.video_budget(), handle).await {
        Some(file) => {
            let name = attachment_name(&file.filename, post.sensitive);
            resolved
                .attachments
                .push(media::to_attachment(file, post.sensitive));
            resolved.attached_video = Some(format!("attachment://{name}"));
            // Video artık kartın içinde (ek olarak) oynar: uzak bağlantıya gerek yok.
            resolved.video_link = None;
        }
        // İndirilemedi ya da bütçe doldu: video kartın içinde uzak bağlantıyla (sessiz) oynar.
        None => debug!(
            "Video ek olarak indirilemedi (bütçe {} ms), uzak bağlantı kullanılıyor: {video_url}",
            options.video_budget().as_millis()
        ),
    }

    resolved
}

/// Medyayı **hiç indirmeden** karta bağlar (kartın tek modu).
///
/// - Görseller: CDN'in hafif `large` varyantı (kart anında yüklenir).
/// - Video: veri aşamasında seçilen mp4 varyantı kartın içinde oynar.
/// - GIF: vxTwitter gifconvert servisi → gerçek animasyonlu görsel olarak kartta oynar.
///
/// Hiçbir ağ isteği yapılmaz: bu yüzden medya fazı ~0 ms'dir ve Discord'a dosya yüklenmez.
pub fn direct_media(plan: &MediaPlan) -> ResolvedMedia {
    let mut resolved = ResolvedMedia {
        // Video kartın içinde oynar (medya galerisinin ilk öğesi).
        video_link: plan.video.clone(),
        ..ResolvedMedia::default()
    };

    resolved
        .images
        .extend(plan.remote_images.iter().map(|url| media::card_image_url(url)));
    resolved.images.extend(plan.gifs.iter().map(|url| remote_gif_url(url)));

    resolved
}

/// Dosya eki olarak indirilemeyen veya doğrudan moddaki GIF'in uzak bağlantısı:
/// vxTwitter gifconvert servisi (`gifconvert.vxtwitter.com`) varsa animasyonlu AVIF
/// bağlantısı kullanılır (kartta animasyonlu görsel gibi oynar), yoksa orijinal bağlantı korunur.
pub fn remote_gif_url(url: &str) -> String {
    media::gif_convert_url(url).unwrap_or_else(|| url.to_string())
}

/// Bir indirme görevini **süre bütçesi** içinde bekler. Bütçe dolarsa görev iptal edilir
/// (yarıda kalan indirme ağ trafiğini de tüketmez) ve `None` döner; çağıran taraf uzak
/// bağlantıya düşerek kartı yine anında gönderir.
async fn within<T>(budget: Duration, mut handle: tokio::task::JoinHandle<Option<T>>) -> Option<T> {
    match tokio::time::timeout(budget, &mut handle).await {
        Ok(Ok(value)) => value,
        // Görev panikledi: dönüşüm bozulmaz, medya uzak bağlantıya düşer.
        Ok(Err(_)) => None,
        Err(_) => {
            handle.abort();
            None
        }
    }
}

/// 999 → `999`, 1_234 → `1.2K`, 12_000 → `12K`, 1_500_000 → `1.5M`
pub fn format_count(value: u64) -> String {
    fn scaled(value: u64, divisor: f64, suffix: &str) -> String {
        let text = format!("{:.1}", value as f64 / divisor);
        let text = text.trim_end_matches(".0").to_string();
        format!("{text}{suffix}")
    }

    match value {
        0..=999 => value.to_string(),
        1_000..=999_999 => scaled(value, 1_000.0, "K"),
        1_000_000..=999_999_999 => scaled(value, 1_000_000.0, "M"),
        _ => scaled(value, 1_000_000_000.0, "B"),
    }
}

/// Kullanıcı kaynaklı metinlerde Discord markdown'ını etkisiz hâle getirir.
///
/// **`_` kaçırılmaz:** Discord (CommonMark) **kelime içindeki** alt çizgileri vurgu
/// başlatıcı saymaz; `world_record_egg` gibi kullanıcı adları olduğu gibi görünür.
/// Kaçırıldığında kart, kullanıcı adında olmayan bir `\` gösteriyordu
/// (`world\_record\_egg`).
pub fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '\\' | '*' | '~' | '`' | '|' | '>' | '[' | ']') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Spoiler dosyalarında `attachment://` referansının gerçek dosya adını verir.
fn attachment_name(filename: &str, spoiler: bool) -> String {
    if spoiler {
        format!("SPOILER_{filename}")
    } else {
        filename.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::converters::builtin::{Author, LinkPreview, Media, MediaKind};
    use crate::converters::parser::Platform;
    use crate::settings::EngineSettings;

    fn options() -> RenderOptions {
        RenderOptions::from_engine(&EngineSettings::default(), "twitter")
    }

    fn image(url: &str) -> Media {
        Media {
            kind: MediaKind::Image,
            url: url.to_string(),
            poster: None,
            width: Some(1200),
            height: Some(800),
            duration: None,
        }
    }

    fn video(url: &str, poster: &str) -> Media {
        Media {
            kind: MediaKind::Video,
            url: url.to_string(),
            poster: Some(poster.to_string()),
            width: Some(1280),
            height: Some(720),
            duration: Some(30),
        }
    }

    fn gif(url: &str) -> Media {
        Media {
            kind: MediaKind::Gif,
            url: url.to_string(),
            poster: Some("https://cdn/poster.jpg".to_string()),
            width: Some(540),
            height: Some(370),
            duration: None,
        }
    }

    fn sample_post() -> BuiltinPost {
        let mut post = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/kullanici/status/1",
            Author::new("Ad", "kullanici"),
        );
        post.text = "merhaba dünya".to_string();
        post
    }

    #[test]
    fn only_reddit_requires_a_video_download() {
        let engine = EngineSettings::default();
        assert!(
            RenderOptions::from_engine(&engine, "reddit").video_requires_download,
            "Reddit: ses ayrı akışta → video zorunlu indirilir"
        );
        for platform in ["twitter", "instagram", "tiktok", "bluesky", "spotify"] {
            assert!(
                !RenderOptions::from_engine(&engine, platform).video_requires_download,
                "{platform}: indirme yok"
            );
        }
    }

    #[test]
    fn plan_keeps_first_video_and_limits_images() {
        let mut post = sample_post();
        post.media = vec![
            image("https://cdn/1.jpg"),
            video("https://cdn/v1.mp4", "https://cdn/v1.jpg"),
            video("https://cdn/v2.mp4", "https://cdn/v2.jpg"),
            image("https://cdn/2.jpg"),
            image("https://cdn/3.jpg"),
        ];
        let mut options = options();
        options.max_images = 2;

        let plan = plan_media(&post, &options);
        assert_eq!(
            plan.video.as_deref(),
            Some("https://cdn/v1.mp4"),
            "ilk video kazanır"
        );
        assert_eq!(
            plan.remote_images,
            vec![
                "https://cdn/1.jpg".to_string(),
                "https://cdn/2.jpg".to_string()
            ],
            "görseller `max_images` ile sınırlanır ve bağlantı olarak kalır"
        );
    }

    #[test]
    fn gifs_are_planned_separately_from_videos() {
        let mut post = sample_post();
        post.media = vec![gif("https://video.twimg.com/tweet_video/x.mp4")];

        let plan = plan_media(&post, &options());
        assert_eq!(plan.gifs.len(), 1, "GIF'ler ayrı tutulur");
        assert!(plan.video.is_none(), "GIF video alanına konmaz");
    }

    #[test]
    fn quote_media_is_used_when_the_post_has_none() {
        let mut post = sample_post();
        let mut quote = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/diger/status/2",
            Author::new("Diğer", "diger"),
        );
        quote.media = vec![video("https://cdn/q.mp4", "https://cdn/q.jpg")];
        post.quote = Some(Box::new(quote));

        let plan = plan_media(&post, &options());
        assert_eq!(plan.video.as_deref(), Some("https://cdn/q.mp4"));
    }

    #[test]
    fn direct_media_never_downloads_anything() {
        let mut post = sample_post();
        post.media = vec![
            image("https://cdn/1.jpg"),
            gif("https://video.twimg.com/tweet_video/x.mp4"),
            video("https://cdn/v1.mp4", "https://cdn/v1.jpg"),
        ];
        let plan = plan_media(&post, &options());
        let resolved = direct_media(&plan);

        assert!(resolved.attachments.is_empty(), "Discord'a dosya yüklenmez");
        assert!(resolved.attached_video.is_none());
        assert_eq!(resolved.video_link.as_deref(), Some("https://cdn/v1.mp4"));
        assert_eq!(
            resolved.images,
            vec![
                "https://cdn/1.jpg".to_string(),
                "https://gifconvert.vxtwitter.com/convert.avif?url=https%3A%2F%2Fvideo.twimg.com%2Ftweet_video%2Fx.mp4".to_string(),
            ],
            "görsel CDN bağlantısı + GIF servisi bağlantısı"
        );
    }

    #[tokio::test]
    async fn non_reddit_videos_are_not_downloaded() {
        let client = reqwest::Client::new();
        let mut post = sample_post();
        post.media = vec![video("https://cdn/v1.mp4", "https://cdn/v1.jpg")];
        let options = options();
        let plan = plan_media(&post, &options);

        let resolved = resolve_media(&client, &post, &plan, &options).await;
        assert!(resolved.attachments.is_empty(), "indirme yok");
        assert_eq!(resolved.video_link.as_deref(), Some("https://cdn/v1.mp4"));
    }

    #[test]
    fn video_size_limit_never_exceeds_discord_limit() {
        let mut options = options();
        options.max_video_mb = u8::MAX;
        assert!(options.max_video_bytes() <= DISCORD_FILE_LIMIT);
        assert_eq!(options.video_budget(), VIDEO_MERGE_BUDGET);
    }

    #[test]
    fn counts_and_markdown_are_formatted_for_discord() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(1_234), "1.2K");
        assert_eq!(format_count(2_500_000), "2.5M");
        assert_eq!(
            escape_markdown("**kalın** [link]"),
            "\\*\\*kalın\\*\\* \\[link\\]"
        );
        assert_eq!(escape_markdown("normal metin"), "normal metin");
        // Discord kelime içi `_`yi vurgu saymaz: kaçış eklenirse kartta `\` görünür.
        assert_eq!(escape_markdown("world_record_egg"), "world_record_egg");
    }

    #[test]
    fn link_preview_image_is_planned_only_for_media_less_posts() {
        let options = options();
        let mut post = sample_post();
        post.link_preview = Some(link_preview(Some("https://cdn.example.com/onizleme.jpg")));

        assert_eq!(
            plan_media(&post, &options).remote_images,
            vec!["https://cdn.example.com/onizleme.jpg".to_string()],
            "medyasız gönderide önizleme görseli galeriye girer"
        );

        // Gönderi kendi medyasını taşıyorsa link kartı görseli plana alınmaz.
        post.media = vec![image("https://cdn/1.jpg")];
        assert_eq!(
            plan_media(&post, &options).remote_images,
            vec!["https://cdn/1.jpg".to_string()]
        );
    }

    #[test]
    fn link_preview_without_image_leaves_the_plan_untouched() {
        let options = options();
        let mut post = sample_post();
        post.link_preview = Some(link_preview(None));

        let plan = plan_media(&post, &options);
        assert!(plan.remote_images.is_empty(), "görsel yoksa galeri boş kalır");
        assert!(plan_is_empty(&plan));
    }

    /// Örnek link kartı (X'in harici bağlantı önizlemesi).
    fn link_preview(image: Option<&str>) -> LinkPreview {
        LinkPreview {
            url: "https://example.com/haber".to_string(),
            title: Some("Haber başlığı".to_string()),
            description: Some("Kısa açıklama".to_string()),
            image: image.map(str::to_string),
            site_name: Some("example.com".to_string()),
        }
    }



}
