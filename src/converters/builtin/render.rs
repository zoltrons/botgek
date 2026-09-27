//! Native Discord embed üretimi.
//!
//! `CreateEmbed` alanları Discord'un karakter sınırlarına göre kesilir; medya ekleri
//! `attachment://` referansıyla embed içinde gösterilir.

use serenity::all::{CreateAttachment, CreateEmbed, CreateEmbedAuthor, CreateEmbedFooter, Timestamp};

use super::media::{self, DISCORD_FILE_LIMIT};
use super::{platform_color, BuiltinPost, Stats};
use crate::settings::EngineSettings;
use crate::utils::truncate_chars;

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
        Self {
            attach_images: engine.attach_images,
            attach_videos: engine.attach_videos,
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
pub fn plan_media(post: &BuiltinPost, options: &RenderOptions) -> MediaPlan {
    let mut plan = MediaPlan::default();
    let max_images = options.max_images as usize;

    for media in &post.media {
        if media.is_video() {
            // Tek video desteklenir (ilk video kazanır).
            if plan.video.is_none() {
                plan.video = Some(media.url.clone());
            }
            continue;
        }

        if plan.images.len() + plan.remote_images.len() >= max_images {
            continue;
        }
        if options.attach_images {
            plan.images.push(media.url.clone());
        } else {
            plan.remote_images.push(media.url.clone());
        }
    }

    plan
}

/// Planı uygular: görselleri/videoyu indirir, indirilemeyenler için uzak URL'e düşer.
/// İndirmeler sırayla yapılır (ek crate bağımlılığı olmadan bellek kullanımı düşük kalır).
pub async fn resolve_media(
    client: &reqwest::Client,
    post: &BuiltinPost,
    plan: &MediaPlan,
    options: &RenderOptions,
) -> ResolvedMedia {
    let mut resolved = ResolvedMedia::default();

    for url in &plan.images {
        match media::download(client, url, MAX_IMAGE_BYTES).await {
            Some(file) => {
                let name = attachment_name(&file.filename, post.sensitive);
                resolved.attachments.push(media::to_attachment(file, post.sensitive));
                resolved.images.push(format!("attachment://{name}"));
            }
            // İndirilemeyen görsel uzak URL olarak gösterilir (CDN erişilebilir kalır).
            None => resolved.images.push(url.clone()),
        }
    }
    resolved.images.extend(plan.remote_images.iter().cloned());

    if let Some(video_url) = &plan.video {
        let mut attached = None;
        if options.attach_videos {
            if let Some(file) = media::download(client, video_url, options.max_video_bytes()).await {
                let name = attachment_name(&file.filename, post.sensitive);
                resolved.attachments.push(media::to_attachment(file, post.sensitive));
                attached = Some(format!("attachment://{name}"));
            }
        }
        match attached {
            Some(reference) => resolved.attached_video = Some(reference),
            None => resolved.video_link = Some(video_url.clone()),
        }
    }

    resolved
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

    if options.show_quote {
        if let Some(quote) = post.quote.as_deref() {
            if let Some(embed) = build_quote_embed(quote) {
                embeds.push(embed);
            }
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

    if options.show_community_note {
        if let Some(note) = &post.community_note {
            let value = truncate_chars(note, EMBED_FIELD_VALUE_LIMIT);
            if !value.is_empty() {
                embed = embed.field("🔎 Topluluk Notu", value, false);
            }
        }
    }

    // Kapak görseli: eklenen ilk görsel, yoksa videonun kapağı
    if let Some(first) = resolved.images.first() {
        embed = match first.strip_prefix("attachment://") {
            Some(filename) => embed.attachment(filename),
            None => embed.image(first.clone()),
        };
    } else if resolved.attached_video.is_some() || resolved.video_link.is_some() {
        if let Some(poster) = post.poster_url() {
            embed = embed.image(poster);
        }
    }

    if options.show_timestamp {
        if let Some(created_at) = post.created_at {
            if let Ok(timestamp) = Timestamp::from_unix_timestamp(created_at) {
                embed = embed.timestamp(timestamp);
            }
        }
    }

    embed.footer(CreateEmbedFooter::new(truncate_chars(
        &footer_text(post, options),
        EMBED_FOOTER_LIMIT,
    )))
}

/// Alt bilgi: platform + ek bilgi + çeviri notu + etkileşim metrikleri.
fn footer_text(post: &BuiltinPost, options: &RenderOptions) -> String {
    let mut parts = vec![format!("{} • Botgek", post.platform.display_name())];
    if let Some(extra) = &post.extra_label {
        if !extra.trim().is_empty() {
            parts.push(extra.trim().to_string());
        }
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
    if let Some(views) = stats.views {
        if views > 0 {
            parts.push(format!("👁️ {}", format_count(views)));
        }
    }
    parts.join("  •  ")
}

fn build_quote_embed(quote: &BuiltinPost) -> Option<CreateEmbed> {
    let text = quote.text.trim().to_string();
    let image = quote
        .media
        .iter()
        .find(|m| m.is_image())
        .map(|m| m.url.clone());
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
        RenderOptions::from_engine(&EngineSettings::default())
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
    fn attached_images_are_referenced_and_extra_images_become_gallery() {
        let resolved = ResolvedMedia {
            images: vec![
                "attachment://photo.jpg".to_string(),
                "https://cdn.example/second.jpg".to_string(),
            ],
            attachments: Vec::new(),
            attached_video: None,
            video_link: None,
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
