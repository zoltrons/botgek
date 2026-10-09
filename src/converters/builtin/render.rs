//! Native Discord embed üretimi.
//!
//! `CreateEmbed` alanları Discord'un karakter sınırlarına göre kesilir; medya ekleri
//! `attachment://` referansıyla embed içinde gösterilir.

use std::time::Duration;

use serenity::all::{CreateAttachment, CreateEmbed, CreateEmbedAuthor, CreateEmbedFooter, Timestamp};
use tracing::debug;

use super::media::{self, DISCORD_FILE_LIMIT};
use super::{platform_color, BuiltinPost, Media, MediaKind, Stats};
use crate::settings::{EngineSettings, MediaDelivery};
use crate::utils::truncate_chars;

/// Medya indirmelerinin **süre bütçesi**: kart "anında" görünmelidir, bu yüzden indirme
/// bütçeyi aşarsa görev iptal edilir ve ilgili medya **uzak bağlantı** olarak kartta
/// gösterilir (video kartın içinde oynamaya devam eder, görsel CDN'den yüklenir).
///
/// Bu değerler **`auto`** modun içindir: hızlıysa dosya eki, değilse bağlantı. `direct`
/// modunda hiç indirme yapılmaz (bütçe kullanılmaz), `attach` modunda daha geniş bütçeler
/// ([`ATTACH_VIDEO_BUDGET`] vb.) geçerlidir.
pub const IMAGE_BUDGET: Duration = Duration::from_millis(600);
/// GIF yarışının toplam bütçesi (FxEmbed GIF servisi + yerel `ffmpeg` yedeği).
pub const GIF_BUDGET: Duration = Duration::from_millis(900);
/// Video indirme bütçesi (bütçe dolunca video kartta uzak bağlantıyla oynar).
pub const VIDEO_BUDGET: Duration = Duration::from_millis(1_200);
/// **Dosya eki** modunda tam çözünürlük için tanınan daha geniş bütçeler.
pub const ATTACH_IMAGE_BUDGET: Duration = Duration::from_millis(2_500);
pub const ATTACH_GIF_BUDGET: Duration = Duration::from_millis(3_000);
pub const ATTACH_VIDEO_BUDGET: Duration = Duration::from_millis(5_000);
/// FxEmbed GIF servisinin tek deneme sınırı (servis CDN'de önbellekli gerçek GIF verir).
const FX_GIF_TIMEOUT: Duration = Duration::from_millis(600);
/// FxEmbed servisi yarışta öne alınır: genelde CDN'den anında gelir, bu yüzden yerel
/// `ffmpeg` yedeği ancak bu gecikmeden sonra başlar (boşa bant genişliği harcanmaz).
const GIF_HEAD_START: Duration = Duration::from_millis(150);
/// GIF yarışında iç bütçenin üstüne eklenen güvenlik payı (dış zaman aşımı).
const GIF_RACE_GRACE: Duration = Duration::from_millis(200);

/// Embed toplamı 6000 karakteri geçemez: başlık/yazar/footer/alanlar için pay bırakılır.
const EMBED_DESCRIPTION_LIMIT: usize = 3500;
const EMBED_FIELD_VALUE_LIMIT: usize = 480;
const EMBED_TITLE_LIMIT: usize = 256;
const EMBED_AUTHOR_LIMIT: usize = 200;
const EMBED_FOOTER_LIMIT: usize = 1024;
/// Ana embed dışında galeri olarak eklenecek ek görsel embed'i sayısı.
const MAX_EXTRA_IMAGE_EMBEDS: usize = 3;
/// Tek bir görsel için indirme üst sınırı.
const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;

/// Ayarlardan türetilen render davranışı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderOptions {
    /// Medya yöntemi: ışık hızı (bağlantı) / bütçeli indirme / her zaman dosya eki.
    pub media_delivery: MediaDelivery,
    pub attach_images: bool,
    pub attach_videos: bool,
    pub max_images: u8,
    pub max_video_mb: u8,
    pub show_metrics: bool,
    pub show_timestamp: bool,
    pub show_community_note: bool,
    pub show_quote: bool,
}

impl RenderOptions {
    pub fn from_engine(engine: &EngineSettings) -> Self {
        // **Işık hızı (`direct`)**: hiçbir dosya indirilmez ve Discord'a yüklenmez. Bu yüzden
        // "dosya eki kapalı" gibi davranılır: plan doğrudan uzak bağlantı üretir, indirme
        // görevi hiç başlamaz ve kart veri gelir gelmez gönderilir.
        let downloads = engine.media_delivery.downloads_media();
        Self {
            media_delivery: engine.media_delivery,
            attach_images: engine.attach_images && downloads,
            attach_videos: engine.attach_videos && downloads,
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

    /// Görsel indirme bütçesi (moda göre).
    pub fn image_budget(&self) -> Duration {
        match self.media_delivery {
            MediaDelivery::Attach => ATTACH_IMAGE_BUDGET,
            _ => IMAGE_BUDGET,
        }
    }

    /// GIF yarışı bütçesi (moda göre).
    pub fn gif_budget(&self) -> Duration {
        match self.media_delivery {
            MediaDelivery::Attach => ATTACH_GIF_BUDGET,
            _ => GIF_BUDGET,
        }
    }

    /// Video indirme bütçesi (moda göre).
    pub fn video_budget(&self) -> Duration {
        match self.media_delivery {
            MediaDelivery::Attach => ATTACH_VIDEO_BUDGET,
            _ => VIDEO_BUDGET,
        }
    }
}

/// İndirme planı: hangi medya dosya olarak eklenecek, hangisi uzak URL olarak gösterilecek.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaPlan {
    /// Dosya olarak eklenecek görseller (sıralı).
    pub images: Vec<String>,
    /// İndirme kapalıysa doğrudan embed'e konacak uzak görsel URL'leri.
    pub remote_images: Vec<String>,
    /// Eklenecek/bağlanacak video URL'i.
    pub video: Option<String>,
    /// Animasyonlu GIF kaynakları (X: sessiz mp4). `ffmpeg` varsa gerçek `.gif`'e çevrilir,
    /// yoksa mp4 olarak eklenir; hiçbir durumda video oynatıcısı olarak gösterilmez.
    pub gifs: Vec<String>,
    /// Medya alıntılanan gönderiden alındı (gönderiye medya eklenmemişse).
    pub quote_media: bool,
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
    /// Medya alıntılanan gönderiden alındı (gönderiye medya eklenmemişse).
    pub quote_media: bool,
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
        push_media(&mut plan, media, options.attach_images, max_images);
    }

    let mut next = post.quote.as_deref();
    while let Some(candidate) = next {
        if !options.show_quote || !plan_is_empty(&plan) {
            break;
        }
        for media in &candidate.media {
            push_media(&mut plan, media, options.attach_images, max_images);
        }
        if !plan_is_empty(&plan) {
            plan.quote_media = true;
        }
        next = candidate.quote.as_deref();
    }

    plan
}

/// Planda gösterilecek medya var mı?
fn plan_is_empty(plan: &MediaPlan) -> bool {
    plan.video.is_none()
        && plan.gifs.is_empty()
        && plan.images.is_empty()
        && plan.remote_images.is_empty()
}

/// Tek bir medyayı plana ekler: GIF'ler ayrı tutulur, ilk video kazanır, görseller
/// `max_images` ile sınırlanır.
fn push_media(plan: &mut MediaPlan, media: &Media, attach_images: bool, max_images: usize) {
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

    if plan.images.len() + plan.remote_images.len() >= max_images {
        return;
    }
    if attach_images {
        plan.images.push(media.url.clone());
    } else {
        plan.remote_images.push(media.url.clone());
    }
}

/// Kartta gösterilecek video kapağı: gönderinin kapağı, yoksa alıntı zincirindeki ilk kapak
/// (video alıntıdan — veya alıntının alıntısından — geldiğinde kapak da oradan alınır).
pub fn video_poster(post: &BuiltinPost) -> Option<String> {
    let mut current = Some(post);
    while let Some(candidate) = current {
        if let Some(poster) = candidate.poster_url() {
            return Some(poster);
        }
        current = candidate.quote.as_deref();
    }
    None
}

/// Planı uygular. **Tüm indirmeler (video + görseller + GIF'ler) paralel** yapılır; böylece
/// mesaj gecikmesi dosyaların toplamı değil, en yavaş tek indirme kadardır.
/// İndirilemeyen medya uzak URL'e düşer (mesaj gönderimi bozulmaz).
pub async fn resolve_media(
    client: &reqwest::Client,
    post: &BuiltinPost,
    plan: &MediaPlan,
    options: &RenderOptions,
) -> ResolvedMedia {
    let spoiler = post.sensitive;

    // Işık hızı (`direct`): hiçbir medya indirilmez. Kart, görsel/video/GIF'i bağlantı
    // olarak taşır (video kartın içinde oynar, GIF FxEmbed'in GIF servisinden gelir), bu
    // yüzden medya fazı **~0 ms**'dir ve Discord'a yükleme yapılmaz.
    if !options.media_delivery.downloads_media() {
        return direct_media(plan);
    }

    // 1) Video (en büyük dosya) ve görseller aynı anda indirilir. Her indirme kendi **süre
    //    bütçesiyle** sınırlıdır: bütçe dolan indirme iptal edilir, kart beklemez.
    let max_video_bytes = options.max_video_bytes();
    let video_handle = plan.video.as_ref().filter(|_| options.attach_videos).map(|url| {
        let client = client.clone();
        let url = url.clone();
        // `download_video`: Reddit videolarında önce sesli birleştirmeyi dener (ffmpeg'siz).
        tokio::spawn(async move { media::download_video(&client, &url, max_video_bytes).await })
    });

    let mut image_handles = Vec::with_capacity(plan.images.len());
    for url in &plan.images {
        let client = client.clone();
        let url = url.clone();
        image_handles.push(tokio::spawn(async move {
            media::download(&client, &url, MAX_IMAGE_BYTES).await
        }));
    }

    // 2) GIF'ler: FxEmbed'in hazır GIF servisi → yerel ffmpeg → mp4 (hepsi paralel görevde).
    //    Dosya eki kapalıyken indirme hiç başlatılmaz: uzak bağlantı yeterlidir.
    let mut gif_handles = Vec::with_capacity(plan.gifs.len());
    if options.attach_images {
        for url in &plan.gifs {
            let client = client.clone();
            let url = url.clone();
            let budget = options.gif_budget();
            gif_handles.push(tokio::spawn(async move { download_gif(&client, &url, budget).await }));
        }
    }

    let mut resolved = ResolvedMedia {
        quote_media: plan.quote_media,
        ..ResolvedMedia::default()
    };

    if let Some(video_url) = &plan.video {
        match video_handle {
            Some(handle) => match within(options.video_budget(), handle).await {
                Some(file) => {
                    let name = attachment_name(&file.filename, spoiler);
                    resolved.attachments.push(media::to_attachment(file, spoiler));
                    resolved.attached_video = Some(format!("attachment://{name}"));
                }
                None => {
                    // İndirilemedi ya da bütçe doldu: video kartın içinde uzak bağlantıyla oynar.
                    debug!(
                        "Video dosya olarak eklenemedi (bütçe {} ms), uzak bağlantı kullanılıyor: {video_url}",
                        options.video_budget().as_millis()
                    );
                    resolved.video_link = Some(video_url.clone());
                }
            },
            // İndirme kapalı: video doğrudan bağlantı olarak mesaja yazılır.
            None => resolved.video_link = Some(video_url.clone()),
        }
    }

    for (url, handle) in plan.images.iter().zip(image_handles) {
        match within(options.image_budget(), handle).await {
            Some(file) => {
                let name = attachment_name(&file.filename, spoiler);
                resolved.attachments.push(media::to_attachment(file, spoiler));
                resolved.images.push(format!("attachment://{name}"));
            }
            // İndirilemeyen (veya bütçeyi aşan) görsel uzak URL olarak gösterilir; kartı
            // yavaşlatan büyük yükleme yerine CDN'in hafif varyantı istenir.
            None => resolved.images.push(media::card_image_url(url)),
        }
    }

    // Dosya eki kapalıyken planlanan uzak görseller doğrudan kullanılır.
    resolved.images.extend(plan.remote_images.iter().cloned());

    if !options.attach_images {
        // Dosya eki kapalı: GIF'ler de uzak URL olarak gösterilir (.gif formatında).
        resolved.images.extend(plan.gifs.iter().map(|url| remote_gif_url(url)));
    } else {
        for (url, handle) in plan.gifs.iter().zip(gif_handles) {
            match within(options.gif_budget() + GIF_RACE_GRACE, handle).await {
                Some(file) => {
                    let name = attachment_name(&file.filename, spoiler);
                    resolved.attachments.push(media::to_attachment(file, spoiler));
                    resolved.images.push(format!("attachment://{name}"));
                }
                None => resolved.images.push(remote_gif_url(url)),
            }
        }
    }

    resolved
}

/// Işık hızı (`direct`) modunda medyayı **hiç indirmeden** karta bağlar.
///
/// - Görseller: CDN'in hafif `large` varyantı (kart anında yüklenir).
/// - Video: veri aşamasında seçilen en uygun mp4 varyantı kartın içinde oynar.
/// - GIF: FxEmbed'in GIF servisi (`gif.fxtwitter.com`) → gerçek `.gif` olarak karta bağlanır.
///
/// Hiçbir ağ isteği yapılmaz: bu yüzden medya fazı ~0 ms'dir ve Discord'a dosya yüklenmez.
pub fn direct_media(plan: &MediaPlan) -> ResolvedMedia {
    let mut resolved = ResolvedMedia {
        quote_media: plan.quote_media,
        // Video kartın içinde oynar (medya galerisinin ilk öğesi).
        video_link: plan.video.clone(),
        ..ResolvedMedia::default()
    };

    resolved
        .images
        .extend(plan.images.iter().map(|url| media::card_image_url(url)));
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


/// GIF'i mümkün olan en hızlı kaynaktan **yarıştırarak** alır:
///
/// 1. **vxTwitter gifconvert servisi** (`gifconvert.vxtwitter.com`) — hazır, CDN'de önbellekli animasyonlu AVIF
///    (tercih edilen kaynak; [`GIF_HEAD_START`] kadar öne alınır),
/// 2. yerel `ffmpeg` dönüşümü — servis gecikirse başlar ve **gerçek GIF** ürettiyse kazanır,
/// 3. hiçbiri olmazsa mp4 (Discord oynatıcı gösterir — eski davranış).
///
/// Yarış `budget` (moda göre: `auto` → [`GIF_BUDGET`], `attach` → [`ATTACH_GIF_BUDGET`]) ile
/// sınırlıdır: eskiden iki servis denemesi sırayla 2 × 45 sn bekleyebiliyordu (üstüne mp4
/// indirme + ffmpeg dönüşümü); artık bütçe dolduğunda yarış biter (yerel mp4 hazırsa o,
/// değilse uzak bağlantı kullanılır) ve kart beklemez.
async fn download_gif(
    client: &reqwest::Client,
    url: &str,
    budget: Duration,
) -> Option<media::MediaFile> {
    // 1) vxTwitter GIF servisi: hemen başlar (CDN'de önbellekli, genelde en hızlı kaynak).
    let mut fx = media::gif_convert_url(url).map(|gif_url| {
        let client = client.clone();
        tokio::spawn(async move {
            // Servis tek seferde yanıt vermeyebilir: iki deneme, her biri kısa süreli.
            for _ in 0..2 {
                let Ok(Some(file)) = tokio::time::timeout(
                    FX_GIF_TIMEOUT,
                    media::download(&client, &gif_url, media::DISCORD_FILE_LIMIT),
                )
                .await
                else {
                    continue;
                };
                if media::is_animated_image(&file) {
                    return Some(file);
                }
            }
            None
        })
    });

    // 2) Yerel yedek: mp4 indirilir, `ffmpeg` ile gerçek GIF'e çevrilir; `ffmpeg` yoksa
    //    mp4 döner (yalnızca vxTwitter servisi tamamen başarısız olursa kullanılır).
    let mut local = Some({
        let client = client.clone();
        let url = url.to_string();
        tokio::spawn(async move {
            // Tercih edilen kaynağa öncelik: yerel yedek boşa bant genişliği harcamasın.
            tokio::time::sleep(GIF_HEAD_START).await;
            let file = media::download(&client, &url, MAX_IMAGE_BYTES).await?;
            match media::to_gif(&file.bytes).await {
                Some(bytes) => Some(media::MediaFile {
                    bytes,
                    filename: media::replace_extension(&file.filename, "gif"),
                    mime: "image/gif".to_string(),
                }),
                None => Some(file),
            }
        })
    });

    let deadline = tokio::time::Instant::now() + budget;

    loop {
        // Bekleyen kaynaklar bu turda sabitlenir: `select!` ön koşulu yalnızca hazır
        // görevleri bekletir (iptal edilen görev tekrar beklenmez).
        let awaiting_fx = fx.is_some();
        let awaiting_local = local.is_some();
        if !awaiting_fx && !awaiting_local {
            return None;
        }

        tokio::select! {
            biased;
            // Tercih edilen kaynak: dönüştürülmüş görsel geldiyse yarış biter.
            result = async { fx.as_mut().expect("vxTwitter GIF kaynağı bekleniyor").await },
                if awaiting_fx =>
            {
                match result {
                    Ok(Some(gif)) => {
                        if let Some(handle) = local.take() {
                            handle.abort();
                        }
                        return Some(gif);
                    }
                    _ => fx = None,
                }
            }
            result = async { local.as_mut().expect("yerel GIF yedeği bekleniyor").await },
                if awaiting_local =>
            {
                match result {
                    // Yerel dönüşüm gerçek animasyonlu görsel ürettiyse tercih edilen kaynağı beklemeden kazanır.
                    Ok(Some(file)) if media::is_animated_image(&file) => {
                        if let Some(handle) = fx.take() {
                            handle.abort();
                        }
                        return Some(file);
                    }
                    _ => {}
                }

                local = None;
            }
            _ = tokio::time::sleep_until(deadline) => {
                // Bütçe doldu: iki kaynak da iptal edilir, kart bekletilmez.
                if let Some(handle) = fx.take() {
                    handle.abort();
                }
                if let Some(handle) = local.take() {
                    handle.abort();
                }
                return None;
            }
        }
    }
}

/// Embed listesini üretir: ana embed + (varsa) galeri görselleri + alıntı embed'i.
pub fn build_embeds(post: &BuiltinPost, options: &RenderOptions, resolved: &ResolvedMedia) -> Vec<CreateEmbed> {
    let mut embeds = vec![build_main_embed(post, options, resolved)];

    for url in resolved.images.iter().skip(1).take(MAX_EXTRA_IMAGE_EMBEDS) {
        embeds.push(
            CreateEmbed::new()
                .color(platform_color(post.platform))
                .url(post.url.clone())
                .image(url.clone()),
        );
    }

    if options.show_quote
        && let Some(quote) = post.quote.as_deref() {
            // Alıntının medyası ana embed'e alındıysa (gönderinin kendi medyası yokken)
            // alıntı embed'inde tekrar gösterilmez.
            if let Some(embed) = build_quote_embed(quote, !resolved.quote_media) {
                embeds.push(embed);
            }

            // Gönderiye **medya eklenmemişse** alıntının alıntısı da gösterilir (X'in kendi
            // kartı medyasız zincirlerde üç gönderiyi birlikte gösterir).
            if post.media.is_empty()
                && let Some(nested) = quote.quote.as_deref()
                    && let Some(embed) = build_quote_embed(nested, !resolved.quote_media) {
                        embeds.push(embed);
                    }
        }

    embeds
}

fn build_main_embed(post: &BuiltinPost, options: &RenderOptions, resolved: &ResolvedMedia) -> CreateEmbed {
    let mut embed = CreateEmbed::new().color(platform_color(post.platform)).url(post.url.clone());

    if let Some(title) = &post.title {
        let title = truncate_chars(&escape_markdown(title), EMBED_TITLE_LIMIT);
        if !title.is_empty() {
            embed = embed.title(title);
        }
    }

    let mut author_label = escape_markdown(&post.author.label());
    if post.author.verified {
        author_label.push_str(" ☑");
    }
    if !author_label.is_empty() {
        let mut author = CreateEmbedAuthor::new(truncate_chars(&author_label, EMBED_AUTHOR_LIMIT));
        // Profil resmi (author icon) kullanılmaz: kartta avatar gösterilmez.
        if let Some(profile) = &post.author.profile_url {
            author = author.url(profile.clone());
        }
        embed = embed.author(author);
    }

    let mut description = post.text.trim().to_string();
    if post.sensitive && !description.is_empty() {
        description = format!("||{description}||");
    }
    if !description.is_empty() {
        embed = embed.description(truncate_chars(&description, EMBED_DESCRIPTION_LIMIT));
    }

    if options.show_community_note
        && let Some(note) = &post.community_note {
            let value = truncate_chars(note, EMBED_FIELD_VALUE_LIMIT);
            if !value.is_empty() {
                embed = embed.field("🔎 Topluluk Notu", value, false);
            }
        }

    // Kapak görseli: eklenen ilk görsel, yoksa videonun kapağı
    if let Some(first) = resolved.images.first() {
        embed = match first.strip_prefix("attachment://") {
            Some(filename) => embed.attachment(filename),
            None => embed.image(first.clone()),
        };
    } else if resolved.attached_video.is_some() || resolved.video_link.is_some() {
        // Kapak: gönderinin kapağı, video alıntıdan geldiyse alıntının kapağı.
        if let Some(poster) = video_poster(post) {
            embed = embed.image(poster);
        }
    }


    if options.show_timestamp
        && let Some(created_at) = post.created_at
            && let Ok(timestamp) = Timestamp::from_unix_timestamp(created_at) {
                embed = embed.timestamp(timestamp);
            }

    embed.footer(CreateEmbedFooter::new(truncate_chars(
        &footer_text(post, options),
        EMBED_FOOTER_LIMIT,
    )))
}

/// Klasik embed alt bilgisindeki marka imzası. Kart tanıma (🗑️ silme izni ve
/// "kartlara verilen yanıt AI'yı tetiklemesin" kuralı) bu metinle yapılır.
pub const CARD_SIGNATURE: &str = "Botgek";

/// Alt bilgi: platform + ek bilgi + çeviri notu + etkileşim metrikleri.
fn footer_text(post: &BuiltinPost, options: &RenderOptions) -> String {
    let mut parts = vec![format!("{} • {}", post.platform.display_name(), CARD_SIGNATURE)];
    if let Some(extra) = &post.extra_label
        && !extra.trim().is_empty() {
            parts.push(extra.trim().to_string());
        }
    if let Some(translation) = &post.translation {
        parts.push(match translation.source.as_deref() {
            Some(source) => format!("🌐 {source} → tr"),
            None => "🌐 Türkçeye çevrildi".to_string(),
        });
    }
    if options.show_metrics && !post.stats.is_empty() {
        parts.push(stats_line(&post.stats));
    }
    parts.join("  •  ")
}

/// `❤️ 1.2K  •  🔁 300  •  💬 42` biçiminde metrik satırı.
pub fn stats_line(stats: &Stats) -> String {
    let mut parts = Vec::new();
    if stats.likes > 0 {
        parts.push(format!("❤️ {}", format_count(stats.likes)));
    }
    if stats.reposts > 0 {
        parts.push(format!("🔁 {}", format_count(stats.reposts)));
    }
    if stats.replies > 0 {
        parts.push(format!("💬 {}", format_count(stats.replies)));
    }
    if stats.quotes > 0 {
        parts.push(format!("❝ {}", format_count(stats.quotes)));
    }
    if let Some(views) = stats.views
        && views > 0 {
            parts.push(format!("👁️ {}", format_count(views)));
        }
    parts.join("  •  ")
}

fn build_quote_embed(quote: &BuiltinPost, include_media: bool) -> Option<CreateEmbed> {
    let text = quote.text.trim().to_string();
    // Medya ana embed'de gösteriliyorsa (gönderiye medya eklenmemişse) burada tekrarlanmaz.
    let image = if include_media {
        quote
            .media
            .iter()
            .find(|m| m.is_image())
            .map(|m| m.url.clone())
            .or_else(|| quote.poster_url())
    } else {
        None
    };
    if text.is_empty() && image.is_none() {
        return None;
    }

    let mut author_label = format!("↪ {}", escape_markdown(&quote.author.label()));
    if quote.author.verified {
        author_label.push_str(" ☑");
    }

    let author = CreateEmbedAuthor::new(truncate_chars(&author_label, EMBED_AUTHOR_LIMIT));
    // Alıntı embed'inde de profil resmi kullanılmaz.

    let mut embed = CreateEmbed::new()
        .color(dim_colour(platform_color(quote.platform)))
        .url(quote.url.clone())
        .author(author);

    let mut description = text;
    if quote.sensitive && !description.is_empty() {
        description = format!("||{description}||");
    }
    if !description.is_empty() {
        embed = embed.description(truncate_chars(&description, EMBED_DESCRIPTION_LIMIT));
    }
    if let Some(image) = image {
        embed = embed.image(image);
    }

    Some(embed)
}

/// Alıntı embed'i için rengi koyulaştırır.
fn dim_colour(colour: u32) -> u32 {
    let dim = |channel: u32| (channel * 3) / 5;
    (dim((colour >> 16) & 0xFF) << 16) | (dim((colour >> 8) & 0xFF) << 8) | dim(colour & 0xFF)
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
pub fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '\\' | '*' | '_' | '~' | '`' | '|' | '>' | '[' | ']') {
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
    use crate::converters::builtin::{Author, Media, MediaKind};
    use crate::converters::parser::Platform;

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

    /// X'in animasyonlu GIF'leri sessiz mp4 olarak gelir.
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
        post.created_at = Some(1_714_998_896);
        post.stats = Stats {
            replies: 5,
            reposts: 12_000,
            likes: 1_234,
            quotes: 0,
            views: Some(2_500_000),
        };
        post
    }

    fn options() -> RenderOptions {
        // Testler **bütçeli indirme** modunu ölçer: motorun varsayılanı artık ışık hızı
        // (`direct`) ve o modda hiçbir dosya indirilmez, plan yalnızca bağlantı taşır.
        RenderOptions {
            media_delivery: MediaDelivery::Auto,
            attach_images: true,
            attach_videos: true,
            max_images: 4,
            max_video_mb: 8,
            show_metrics: true,
            show_timestamp: true,
            show_community_note: true,
            show_quote: true,
        }
    }

    #[test]
    fn direct_mode_never_downloads_media() {
        let mut engine = EngineSettings {
            media_delivery: MediaDelivery::Direct,
            ..Default::default()
        };
        // Kullanıcı dosya eki seçmiş olsa bile ışık hızı modu indirme yapmaz.
        engine.attach_images = true;
        engine.attach_videos = true;

        let options = RenderOptions::from_engine(&engine);
        assert!(!options.attach_images, "ışık hızı: görsel indirme kapalı");
        assert!(!options.attach_videos, "ışık hızı: video indirme kapalı");

        let mut post = sample_post();
        post.media = vec![
            image("https://cdn/1.jpg"),
            gif("https://video.twimg.com/tweet_video/x.mp4"),
            video("https://cdn/v1.mp4", "https://cdn/v1.jpg"),
        ];
        let plan = plan_media(&post, &options);
        assert!(plan.images.is_empty(), "indirilecek görsel yok");
        assert_eq!(plan.remote_images, vec!["https://cdn/1.jpg"]);

        // Medya hiç indirilmeden karta bağlanır: dosya eki yok, bağlantılar hazır.
        let resolved = direct_media(&plan);
        assert!(resolved.attachments.is_empty(), "Discord'a dosya yüklenmez");
        assert_eq!(resolved.video_link.as_deref(), Some("https://cdn/v1.mp4"));
        assert_eq!(
            resolved.images,
            vec![
                "https://cdn/1.jpg".to_string(),
                "https://gifconvert.vxtwitter.com/convert.avif?url=https%3A%2F%2Fvideo.twimg.com%2Ftweet_video%2Fx.mp4".to_string()
            ],
            "GIF, vxTwitter'ın GIF servisinden gelir (.avif)"
        );
    }

    #[test]
    fn main_embed_carries_text_metrics_and_timestamp() {
        let embeds = build_embeds(&sample_post(), &options(), &ResolvedMedia::default());
        assert_eq!(embeds.len(), 1);

        let value = serde_json::to_value(&embeds[0]).expect("embed serileşmeli");
        assert_eq!(value["type"], "rich");
        assert_eq!(value["description"], "merhaba dünya");
        assert_eq!(value["author"]["name"], "Ad (@kullanici)");
        assert_eq!(value["color"], 0x1D9BF0);

        let footer = value["footer"]["text"].as_str().unwrap();
        assert!(footer.contains("Twitter • Botgek"), "footer: {footer}");
        assert!(footer.contains("❤️ 1.2K"), "footer: {footer}");
        assert!(footer.contains("👁️ 2.5M"), "footer: {footer}");
        assert!(value["timestamp"].is_string());
    }

    #[test]
    fn author_section_shows_no_profile_picture() {
        let mut post = sample_post();
        post.author.avatar = Some("https://pbs.twimg.com/profile_images/1/kucuk.jpg".to_string());
        post.author.profile_url = Some("https://x.com/kullanici".to_string());
        post.author.verified = true;

        let embeds = build_embeds(&post, &options(), &ResolvedMedia::default());
        let value = serde_json::to_value(&embeds[0]).expect("embed serileşmeli");

        // Profil resmi kartta gösterilmez; yalnızca ad + profil linki kalır.
        assert!(value["author"]["icon_url"].is_null(), "avatar ikonu eklenmemeli");
        assert_eq!(value["author"]["url"], "https://x.com/kullanici");
        assert_eq!(value["author"]["name"], "Ad (@kullanici) ☑");

        let footer = value["footer"]["text"].as_str().unwrap();
        assert!(footer.contains("🔁 12K"), "footer: {footer}");
        assert!(footer.contains("👁️ 2.5M"), "footer: {footer}");
    }

    #[test]
    fn animated_gifs_are_planned_separately_from_videos() {
        let mut post = sample_post();
        post.media = vec![gif("https://video.twimg.com/tweet_video/x.mp4")];

        let plan = plan_media(&post, &options());

        assert_eq!(
            plan.gifs,
            vec!["https://video.twimg.com/tweet_video/x.mp4"],
            "GIF ayrı listede planlanır (gerçek .gif'e çevrilir)"
        );
        assert!(plan.video.is_none(), "GIF video alanına konmaz: oynatıcı çıkmasın");
        assert!(plan.images.is_empty());
    }

    #[test]
    fn quote_media_is_planned_when_the_post_has_no_media() {
        let mut post = sample_post();
        let mut quote = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/diger/status/2",
            Author::new("Diğer", "diger"),
        );
        quote.media = vec![video("https://cdn/q.mp4", "https://cdn/q.jpg")];
        post.quote = Some(Box::new(quote));

        let plan = plan_media(&post, &options());
        assert_eq!(
            plan.video.as_deref(),
            Some("https://cdn/q.mp4"),
            "alıntılayan kişi medya eklemediyse alıntının videosu karta alınır"
        );
        assert!(plan.quote_media, "medyanın alıntıdan geldiği işaretlenir");
        assert_eq!(
            video_poster(&post).as_deref(),
            Some("https://cdn/q.jpg"),
            "kapak görseli de alıntıdan alınır"
        );
    }

    #[test]
    fn post_media_wins_over_the_quote_media() {
        let mut post = sample_post();
        post.media = vec![image("https://cdn/own.jpg")];
        let mut quote = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/diger/status/2",
            Author::new("Diğer", "diger"),
        );
        quote.media = vec![video("https://cdn/q.mp4", "https://cdn/q.jpg")];
        post.quote = Some(Box::new(quote));

        let plan = plan_media(&post, &options());
        assert!(plan.video.is_none(), "gönderiye medya eklenmişse alıntının videosu alınmaz");
        assert_eq!(plan.images, vec!["https://cdn/own.jpg"]);
        assert!(!plan.quote_media);
    }

    /// Medyasız zincirin en içindeki gönderi (C).
    fn inner_quote_with_video() -> BuiltinPost {
        let mut inner = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/enic/status/3",
            Author::new("En İç", "enic"),
        );
        inner.text = "en içteki metin".to_string();
        inner.media = vec![video("https://cdn/inner.mp4", "https://cdn/inner.jpg")];
        inner
    }

    /// Alıntılayan (A) ve alıntılanan (B) medyasız; B, C'yi alıntılar.
    fn media_less_quote_chain() -> BuiltinPost {
        let mut post = sample_post();
        let mut middle = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/orta/status/2",
            Author::new("Orta", "orta"),
        );
        middle.text = "ortadaki metin".to_string();
        middle.quote = Some(Box::new(inner_quote_with_video()));
        post.quote = Some(Box::new(middle));
        post
    }

    #[test]
    fn quote_chain_media_is_used_when_outer_posts_have_none() {
        let post = media_less_quote_chain();

        let plan = plan_media(&post, &options());

        assert_eq!(
            plan.video.as_deref(),
            Some("https://cdn/inner.mp4"),
            "A ve B medyasızsa C'nin medyası karta alınır (X davranışı)"
        );
        assert!(plan.quote_media, "medyanın alıntı zincirinden geldiği işaretlenir");
        assert_eq!(
            video_poster(&post).as_deref(),
            Some("https://cdn/inner.jpg"),
            "kapak görseli de zincirdeki gönderiden alınır"
        );
    }

    #[test]
    fn classic_embeds_show_three_posts_in_a_media_less_quote_chain() {
        let post = media_less_quote_chain();

        let embeds = build_embeds(&post, &options(), &ResolvedMedia::default());
        assert_eq!(embeds.len(), 3, "ana + alıntı + alıntının alıntısı");

        let nested = serde_json::to_value(&embeds[2]).expect("embed serileşmeli");
        assert_eq!(nested["description"], "en içteki metin");
        assert_eq!(nested["author"]["name"], "↪ En İç (@enic)");
    }

    #[test]
    fn quote_chain_stays_two_levels_when_the_post_has_media() {
        let mut post = media_less_quote_chain();
        post.media = vec![image("https://cdn/own.jpg")];

        let embeds = build_embeds(&post, &options(), &ResolvedMedia::default());
        assert_eq!(embeds.len(), 2, "gönderide medya varsa zincir iki seviyede kalır");
    }

    #[test]
    fn attached_images_are_referenced_and_extra_images_become_gallery() {
        let resolved = ResolvedMedia {
            images: vec![
                "attachment://photo.jpg".to_string(),
                "https://cdn.example/second.jpg".to_string(),
            ],
            attachments: Vec::new(),
            attached_video: None,
            video_link: None,
            quote_media: false,
        };

        let embeds = build_embeds(&sample_post(), &options(), &resolved);
        assert_eq!(embeds.len(), 2);

        let main = serde_json::to_value(&embeds[0]).unwrap();
        assert_eq!(main["image"]["url"], "attachment://photo.jpg");

        let gallery = serde_json::to_value(&embeds[1]).unwrap();
        assert_eq!(gallery["image"]["url"], "https://cdn.example/second.jpg");
    }

    #[test]
    fn media_plan_respects_image_limit_and_prefers_first_video() {
        let mut post = sample_post();
        post.media = vec![
            image("https://cdn/1.jpg"),
            image("https://cdn/2.jpg"),
            image("https://cdn/3.jpg"),
            video("https://cdn/v1.mp4", "https://cdn/v1.jpg"),
            image("https://cdn/4.jpg"),
            video("https://cdn/v2.mp4", "https://cdn/v2.jpg"),
        ];

        let limited = RenderOptions {
            max_images: 2,
            ..options()
        };
        let plan = plan_media(&post, &limited);
        assert_eq!(plan.images, vec!["https://cdn/1.jpg", "https://cdn/2.jpg"]);
        assert_eq!(plan.video.as_deref(), Some("https://cdn/v1.mp4"));
        assert!(plan.remote_images.is_empty());

        // İndirme kapalıysa görseller uzak URL olarak işaretlenir
        let without_attachment = RenderOptions {
            max_images: 2,
            attach_images: false,
            ..options()
        };
        let plan = plan_media(&post, &without_attachment);
        assert!(plan.images.is_empty());
        assert_eq!(plan.remote_images, vec!["https://cdn/1.jpg", "https://cdn/2.jpg"]);
    }

    #[test]
    fn video_poster_is_used_when_video_is_linked_instead_of_attached() {
        let mut post = sample_post();
        post.media = vec![video("https://cdn/v1.mp4", "https://cdn/v1.jpg")];

        let resolved = ResolvedMedia {
            video_link: Some("https://cdn/v1.mp4".to_string()),
            ..ResolvedMedia::default()
        };
        let embeds = build_embeds(&post, &options(), &resolved);
        let value = serde_json::to_value(&embeds[0]).unwrap();
        assert_eq!(value["image"]["url"], "https://cdn/v1.jpg");
    }

    #[test]
    fn sensitive_posts_are_wrapped_and_files_prefixed() {
        let mut post = sample_post();
        post.sensitive = true;

        let embeds = build_embeds(&post, &options(), &ResolvedMedia::default());
        let value = serde_json::to_value(&embeds[0]).unwrap();
        assert_eq!(value["description"], "||merhaba dünya||");
        assert_eq!(attachment_name("photo.jpg", true), "SPOILER_photo.jpg");
        assert_eq!(attachment_name("photo.jpg", false), "photo.jpg");
    }

    #[test]
    fn quote_post_gets_its_own_embed() {
        let mut post = sample_post();
        let mut quote = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/diger/status/2",
            Author::new("Diğer", "diger"),
        );
        quote.text = "alıntılanan metin".to_string();
        quote.media = vec![image("https://cdn/q.jpg")];
        post.quote = Some(Box::new(quote));

        let embeds = build_embeds(&post, &options(), &ResolvedMedia::default());
        assert_eq!(embeds.len(), 2);

        let quote_value = serde_json::to_value(&embeds[1]).unwrap();
        assert_eq!(quote_value["description"], "alıntılanan metin");
        assert_eq!(quote_value["author"]["name"], "↪ Diğer (@diger)");
        assert_eq!(quote_value["image"]["url"], "https://cdn/q.jpg");

        // Alıntı ayarı kapalıysa ikinci embed üretilmez
        let without_quote = RenderOptions {
            show_quote: false,
            ..options()
        };
        assert_eq!(build_embeds(&post, &without_quote, &ResolvedMedia::default()).len(), 1);
    }

    #[test]
    fn counts_and_markdown_are_formatted_for_discord() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_234), "1.2K");
        assert_eq!(format_count(9_999), "10K");
        assert_eq!(format_count(12_000), "12K");
        assert_eq!(format_count(2_500_000), "2.5M");
        assert_eq!(format_count(1_200_000_000), "1.2B");

        assert_eq!(escape_markdown("**kalın** [link]"), "\\*\\*kalın\\*\\* \\[link\\]");
        assert_eq!(escape_markdown("normal metin"), "normal metin");
    }

    #[test]
    fn translated_post_is_marked_in_footer() {
        let mut post = sample_post();
        post.translation = Some(crate::converters::builtin::Translation {
            source: Some("de".to_string()),
        });

        let embeds = build_embeds(&post, &options(), &ResolvedMedia::default());
        let value = serde_json::to_value(&embeds[0]).unwrap();
        let footer = value["footer"]["text"].as_str().unwrap();
        assert!(footer.contains("🌐 de → tr"), "footer: {footer}");

        let unknown_source = {
            let mut post = sample_post();
            post.translation = Some(crate::converters::builtin::Translation { source: None });
            build_embeds(&post, &options(), &ResolvedMedia::default())
        };
        let value = serde_json::to_value(&unknown_source[0]).unwrap();
        let footer = value["footer"]["text"].as_str().unwrap();
        assert!(footer.contains("🌐 Türkçeye çevrildi"), "footer: {footer}");
    }

    #[test]
    fn video_size_limit_never_exceeds_discord_limit() {
        let huge = RenderOptions {
            max_video_mb: 50,
            ..options()
        };
        assert_eq!(huge.max_video_bytes(), DISCORD_FILE_LIMIT);

        let small = RenderOptions {
            max_video_mb: 2,
            ..options()
        };
        assert_eq!(small.max_video_bytes(), 2 * 1024 * 1024);
    }

    #[tokio::test]
    async fn media_budget_keeps_slow_downloads_out_of_the_card() {
        // Bütçe içinde biten indirme dosya olarak eklenir...
        let fast = tokio::spawn(async { Some("hızlı".to_string()) });
        assert_eq!(within(IMAGE_BUDGET, fast).await.as_deref(), Some("hızlı"));

        // ...bütçeyi aşan indirme iptal edilir: kart beklemez, medya uzak bağlantıya düşer.
        let slow = tokio::spawn(async {
            tokio::time::sleep(VIDEO_BUDGET * 4).await;
            Some("yavaş".to_string())
        });
        assert_eq!(within(Duration::from_millis(20), slow).await, None);
    }

    #[tokio::test]
    async fn panicked_download_tasks_degrade_to_remote_urls() {
        let broken: tokio::task::JoinHandle<Option<String>> =
            tokio::spawn(async { panic!("indirme görevi çöktü") });
        assert_eq!(within(IMAGE_BUDGET, broken).await, None);
    }

    #[test]
    fn media_budgets_bound_the_card_latency() {
        // Kart "anında" görünmeli: en kötü durumda beklenen süre tek bir bütçedir.
        let worst_case = [IMAGE_BUDGET, GIF_BUDGET + GIF_RACE_GRACE, VIDEO_BUDGET]
            .into_iter()
            .max()
            .expect("bütçe listesi boş olamaz");
        assert!(
            worst_case <= Duration::from_millis(2_500),
            "en kötü medya beklemesi: {worst_case:?}"
        );
    }

    #[test]
    fn remote_gif_urls_prefer_the_vxtwitter_service() {
        // vxTwitter GIF servisi varsa uzak bağlantı da animasyonlu `.avif` olur.
        assert_eq!(
            remote_gif_url("https://video.twimg.com/tweet_video/HTEtNFcXEAAY4Dv.mp4"),
            "https://gifconvert.vxtwitter.com/convert.avif?url=https%3A%2F%2Fvideo.twimg.com%2Ftweet_video%2FHTEtNFcXEAAY4Dv.mp4"
        );
        assert_eq!(
            remote_gif_url("https://cdn.example/clip.mp4"),
            "https://cdn.example/clip.mp4"
        );
    }
}
