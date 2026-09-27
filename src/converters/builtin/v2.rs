//! Modern bileşen düzeni (Components V2) ile native kart üretimi.
//!
//! Klasik embed'lerde Discord `video` alanını bot/webhook mesajlarında göstermez; bu yüzden
//! indirilen video mesajın dışında ayrı bir oynatıcı olarak belirir. Components V2
//! (`1 << 15`) ile gelen **Media Gallery** bileşeni hem videoyu hem görselleri kartın
//! içinde gösterir. Kart, X'in kendi embed'ine benzeyecek şekilde kurulur:
//!
//! ```text
//! kullanıcının yazdığı metin                                     ← kartın DIŞINDA (üstünde)
//! ┌────────────────────────────────────────────────────────────┐
//! │ **Ad <:verified:...> ([@kullanici](https://x.com/kullanici))**   ← profil resmi yok
//! │ gönderi metni
//! │ -# <:comment:...> 6  <:rt:...> 27  <:like:...> 386  <:metric:...> 12.3K
//! │ -# <:twitter:...> • <t:1714998896:R> • [Link](https://x.com/kullanici/status/1)
//! └────────────────────────────────────────────────────────────┘
//! ```
//!
//! Kullanıcının linkle birlikte yazdığı metin kartın **içine konmaz**: bileşenlerin en
//! üstünde düz metin (`Text Display`) olarak gönderilir, böylece taklit (webhook) açıkken
//! mesaj kullanıcının kendi adıyla gittiği için tam olarak "kullanıcı yazmış gibi" görünür.
//! Components V2 bayrağı `content` alanını yasakladığı için mesaj metninin yerini bu
//! bileşen alır.
//!
//! Sol kenarda renkli çubuk (container `accent_color`) kullanılmaz.

use serde_json::{Value, json};

use super::render::{RenderOptions, ResolvedMedia, escape_markdown, format_count};
use super::{BuiltinPost, Impersonation};
use crate::converters::parser::Platform;
use crate::settings::EmojiSettings;
use crate::utils::truncate_chars;

/// Discord'un Components V2 bayrağı (`1 << 15`). Mesajdaki `content`/`embeds` alanları
/// yerine bileşenler kullanılır.
pub const COMPONENTS_V2_FLAG: u64 = 1 << 15;
/// Tek bir `Text Display` bileşeninin Discord sınırı.
pub const TEXT_DISPLAY_LIMIT: usize = 4000;
/// `Media Gallery` bileşeni en fazla 10 öğe alır.
pub const MEDIA_GALLERY_LIMIT: usize = 10;
/// Kart içindeki bileşen sayısı (Discord sınırı 40).
const MAX_CONTAINER_CHILDREN: usize = 12;

// Bileşen türleri
const TYPE_TEXT_DISPLAY: u64 = 10;
const TYPE_MEDIA_GALLERY: u64 = 12;
const TYPE_SEPARATOR: u64 = 14;
const TYPE_CONTAINER: u64 = 17;

/// Alıntı bloğunun girintisi.
///
/// Discord, satır başındaki normal boşlukları (`U+0020`) markdown işlerken daralttığı için
/// girinti **daraltılmayan em boşluğu** (`U+2003`) ile verilir; böylece alıntı, kartın
/// gövde metnine göre biraz sağda durur (Reddit/Twitter alıntı hiyerarşisi). Ayraç
/// karakterler satırın **başında** değil blok işaretinden (`-# ` / `> `) sonra gelir;
/// aksi halde markdown alt metin/alıntı kuralları eşleşmez.
const QUOTE_INDENT: &str = "\u{2003}\u{2003}";

/// Kartı oluşturmak için gereken tüm girdiler.
pub struct V2Card<'a> {
    pub post: &'a BuiltinPost,
    pub options: &'a RenderOptions,
    pub resolved: &'a ResolvedMedia,
    /// Modern kartta kullanılacak özel emoji seti.
    pub emojis: &'a EmojiSettings,
    /// Kullanıcının linkin altına yazdığı ek metin (varsa).
    pub user_comment: Option<&'a str>,
    /// Taklit (webhook) kimliği; `None` ise mesaj bot adına gider.
    pub impersonation: Option<&'a Impersonation>,
    /// Discord `allowed_mentions` gövdesi (serde ile üretilir).
    pub allowed_mentions: Option<Value>,
}

/// Discord `POST /channels/{id}/messages` (veya webhook) gövdesini üretir.
///
/// Yapı: `components[0]` = (varsa) kullanıcının yazdığı metin, `components[1]` = kart
/// (`Container`). Kullanıcı metni kartın **içinde** değil **üstünde**, düz mesaj metni
/// olarak durur; taklit (webhook) açıkken mesaj kullanıcının adıyla gittiği için tam
/// olarak kullanıcı yazmış gibi görünür.
pub fn build_payload(card: &V2Card<'_>) -> Value {
    // Kullanıcı notu kartın dışına çıkar: alıntı bloğuna (`>`) çevrilmez, olduğu gibi yazılır.
    let mut top: Vec<Value> = card
        .user_comment
        .map(str::trim)
        .filter(|comment| !comment.is_empty())
        .map(text)
        .into_iter()
        .collect();

    let mut children: Vec<Value> = vec![author_component(card.post, card.emojis)];

    if !card.post.text.trim().is_empty() {
        children.push(text(spoiler_wrap(card.post.text.trim(), card.post.sensitive)));
    }
    if card.options.show_community_note {
        if let Some(note) = card
            .post
            .community_note
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            children.push(text(format!("-# 🔎 Topluluk Notu\n{}", note)));
        }
    }
    if card.options.show_quote {
        if let Some(quote) = card.post.quote.as_deref() {
            if let Some(component) = quote_component(quote, card.emojis) {
                // Hiyerarşi: alıntı bloğundan önce görünmez küçük bir dikey boşluk bırakılır,
                // böylece alıntı kartın gövdesinden ayrı bir bölüm olarak okunur.
                children.push(separator());
                children.push(component);
            }
        }
    }
    if let Some(gallery) = media_gallery(card) {
        children.push(gallery);
    }
    if let Some(metrics) = metrics_line(card.post, card.options, card.emojis) {
        children.push(text(metrics));
    }
    children.push(text(meta_line(card)));

    children.truncate(MAX_CONTAINER_CHILDREN);

    // Kart her zaman son bileşendir: kullanıcı metni (varsa) onun üstünde durur.
    top.push(json!({
        "type": TYPE_CONTAINER,
        "components": children,
    }));

    let mut payload = json!({
        "flags": COMPONENTS_V2_FLAG,
        "components": top,
        "attachments": attachments_json(card.resolved),
    });

    if let Some(allowed_mentions) = &card.allowed_mentions {
        payload["allowed_mentions"] = allowed_mentions.clone();
    }

    // Taklit: mesaj kullanıcının adı/avatarı ile gönderilir.
    if let Some(identity) = card.impersonation {
        payload["username"] = json!(identity.username);
        if let Some(avatar) = &identity.avatar_url {
            payload["avatar_url"] = json!(avatar);
        }
    }

    payload
}

/// `{"type": 10, "content": ...}` — markdown destekli metin bileşeni.
fn text(content: impl Into<String>) -> Value {
    json!({
        "type": TYPE_TEXT_DISPLAY,
        "content": truncate_chars(&content.into(), TEXT_DISPLAY_LIMIT),
    })
}

/// Yazar satırı: `**Ad <:verified:...> ([@kullanici](profil))**`.
///
/// Profil resmi kartta gösterilmez: Discord Components V2 küçük resmi yalnızca bölümün
/// sağına koyabildiği için modelin solunda küçük avatar mümkün değildir.
fn author_component(post: &BuiltinPost, emojis: &EmojiSettings) -> Value {
    text(author_line(post, emojis))
}

/// `**Ad <:verified:...> ([@kullanici](profil))**` biçimindeki yazar satırı.
fn author_line(post: &BuiltinPost, emojis: &EmojiSettings) -> String {
    let name = post.author.name.trim();
    let name = if name.is_empty() { post.author.handle.trim() } else { name };

    let mut inner = escape_markdown(name);
    if post.author.verified {
        inner.push(' ');
        inner.push_str(emojis.verified());
    }

    let handle = post.author.handle.trim();
    let body = if handle.is_empty() {
        inner
    } else {
        let handle = escape_markdown(handle);
        match post.author.profile_url.as_deref().map(str::trim).filter(|url| !url.is_empty()) {
            Some(url) => format!("{inner} ([@{handle}]({url}))"),
            None => format!("{inner} (@{handle})"),
        }
    };

    format!("**{body}**")
}

/// Alıntılanan gönderi: `[Quote](link) from **Ad** ([**@kullanici**](profil))` başlığı + blok alıntı.
fn quote_component(quote: &BuiltinPost, emojis: &EmojiSettings) -> Option<Value> {
    let body = quote.text.trim();
    let has_image = quote.media.iter().any(|media| media.is_image());
    if body.is_empty() && !has_image {
        return None;
    }

    // Başlık: alıntı emojisi + `[Quote](alıntı linki) from **Ad** ([**@kullanici**](profil))`
    // (fxembed düzeni). Alıntılanan gönderide medya varsa galeride gösterilir.
    let author = quote.author.name.trim();
    let author = if author.is_empty() {
        quote.author.handle.trim().to_string()
    } else {
        escape_markdown(author)
    };

    let profile = quote.author.profile_url.as_deref().map(str::trim).filter(|url| !url.is_empty());
    let handle = quote.author.handle.trim();
    let from = match (handle.is_empty(), profile) {
        (false, Some(profile)) => {
            format!("from **{author}** ([**@{}**]({profile}))", escape_markdown(handle))
        }
        (false, None) => format!("from **{author}** (@{})", escape_markdown(handle)),
        (true, _) => format!("from **{author}**"),
    };

    let url = quote.url.trim();
    let quote_link = if url.is_empty() {
        "Quote".to_string()
    } else {
        format!("[Quote]({url})")
    };

    let mut lines = vec![format!("-# {QUOTE_INDENT}{} {quote_link} {from}", emojis.quote())];
    if !body.is_empty() {
        for line in body.lines().take(8) {
            lines.push(format!("> {QUOTE_INDENT}{line}"));
        }
    }

    Some(text(lines.join("\n")))
}

/// Görünür çizgi olmadan dikey boşluk: alıntı bloğu kartın gövdesinden ayrılsın diye
/// (`spacing: 1` = küçük, `divider: false` = çizgi yok).
fn separator() -> Value {
    json!({ "type": TYPE_SEPARATOR, "divider": false, "spacing": 1 })
}

/// `Media Gallery`: video + görseller; hepsi kartın içinde.
///
/// Video **tek başına** gösterilir: ayrıca kapak görseli eklenmez. Kapak + video birlikte
/// eklendiğinde galeri iki öğeyi **yan yana** (thumbnail + video) çiziyordu.
fn media_gallery(card: &V2Card<'_>) -> Option<Value> {
    let post = card.post;
    let mut items: Vec<Value> = Vec::new();

    match (&card.resolved.attached_video, &card.resolved.video_link) {
        // Video dosya olarak eklendiyse doğrudan kart içinde oynar.
        (Some(reference), _) => items.push(media_item(reference, post.sensitive)),
        // İndirilemediyse uzak video bağlantısı kartın içinde oynar (Discord videoyu
        // dış bağlantıdan da oynatır); kapak görseli eklenmez, aksi halde galeri
        // kapak + video olarak iki öğe gösterirdi.
        (None, Some(link)) => items.push(media_item(link, post.sensitive)),
        (None, None) => {}
    }

    for url in &card.resolved.images {
        items.push(media_item(url, post.sensitive));
    }

    // Alıntının görseli yalnızca ana karta alınmadıysa eklenir; gönderiye medya
    // eklenmemişse alıntının medyası zaten yukarıda yer alıyor (tekrar gösterilmez).
    if card.options.show_quote && !card.resolved.quote_media {
        if let Some(quote) = card.post.quote.as_deref() {
            if let Some(image) = quote.media.iter().find(|media| media.is_image()) {
                items.push(media_item(&image.url, quote.sensitive));
            }
        }
    }

    if items.is_empty() {
        return None;
    }

    items.truncate(MEDIA_GALLERY_LIMIT);
    Some(json!({"type": TYPE_MEDIA_GALLERY, "items": items}))
}

fn media_item(url: &str, spoiler: bool) -> Value {
    json!({"media": {"url": url}, "spoiler": spoiler})
}


/// Metrik satırı: `-# <:comment:...> 6  <:rt:...> 27  <:like:...> 386  <:metric:...> 12.3K`
fn metrics_line(post: &BuiltinPost, options: &RenderOptions, emojis: &EmojiSettings) -> Option<String> {
    if !options.show_metrics || post.stats.is_empty() {
        return None;
    }

    let stats = &post.stats;
    let mut parts: Vec<String> = Vec::new();
    if stats.replies > 0 {
        parts.push(format!("{} {}", emojis.replies(), format_count(stats.replies)));
    }
    if stats.reposts > 0 {
        parts.push(format!("{} {}", emojis.reposts(), format_count(stats.reposts)));
    }
    if stats.likes > 0 {
        parts.push(format!("{} {}", emojis.likes(), format_count(stats.likes)));
    }
    if let Some(views) = stats.views.filter(|views| *views > 0) {
        parts.push(format!("{} {}", emojis.views(), format_count(views)));
    }
    // X'in kendi şeridinde alıntı sayısı yok; varsa alıntı emojisiyle gösterilir.
    if stats.quotes > 0 {
        parts.push(format!("{} {}", emojis.quote(), format_count(stats.quotes)));
    }

    if parts.is_empty() {
        None
    } else {
        Some(format!("-# {}", parts.join("  ")))
    }
}

/// Kartın son satırı: `-# <:twitter:...> • <t:...:R> • [Link](gönderi)`
fn meta_line(card: &V2Card<'_>) -> String {
    let post = card.post;
    let mut parts: Vec<String> = Vec::new();

    if let Some(translation) = &post.translation {
        let marker = card.emojis.translation();
        parts.push(match translation.source.as_deref() {
            Some(source) => format!("{marker} {source} → tr"),
            None => format!("{marker} Türkçeye çevrildi"),
        });
    }

    // Platform göstergesi: X için özel emoji, diğer platformlarda adı yazılır.
    parts.push(match post.platform {
        Platform::Twitter => card
            .emojis
            .twitter()
            .map(str::to_string)
            .unwrap_or_else(|| post.platform.display_name().to_string()),
        _ => post.platform.display_name().to_string(),
    });

    if let Some(extra) = post.extra_label.as_deref().map(str::trim).filter(|e| !e.is_empty()) {
        parts.push(extra.to_string());
    }
    if card.options.show_timestamp {
        if let Some(created_at) = post.created_at {
            parts.push(format!("<t:{created_at}:R>"));
        }
    }

    // Gönderi linki kartın içinde durur.
    let link = format!("[Link]({})", post.url);
    parts.push(spoiler_wrap(&link, post.sensitive));

    // İndirilemeyen video için açma bağlantısı.
    if let Some(video) = &card.resolved.video_link {
        parts.push(format!("[🎬 Videoyu aç]({video})"));
    }

    format!("-# {}", parts.join(" • "))
}

/// Hassas gönderilerde metni spoiler içine alır.
fn spoiler_wrap(text: &str, spoiler: bool) -> String {
    if spoiler && !text.is_empty() {
        format!("||{text}||")
    } else {
        text.to_string()
    }
}

/// Yüklenen dosyalar `attachment://<dosya adı>` ile referans verildiği için
/// payload'da dosya listesi (id + ad) bulunmak zorundadır.
fn attachments_json(resolved: &ResolvedMedia) -> Value {
    Value::Array(
        resolved
            .attachments
            .iter()
            .enumerate()
            .map(|(id, file)| json!({"id": id, "filename": file.filename}))
            .collect(),
    )
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::converters::builtin::{Author, Media, MediaKind, Stats, Translation};
    use crate::settings::EngineSettings;

    fn image(url: &str) -> Media {
        Media {
            kind: MediaKind::Image,
            url: url.to_string(),
            poster: None,
            width: None,
            height: None,
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
            duration: Some(12),
        }
    }

    fn sample_post() -> BuiltinPost {
        let mut post = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/kullanici/status/1",
            Author::new("Culture Base", "Culture3ase"),
        );
        post.text = "merhaba dünya".to_string();
        post.author.verified = true;
        post.author.profile_url = Some("https://x.com/Culture3ase".to_string());
        post.created_at = Some(1_714_998_896);
        post.stats = Stats {
            replies: 6,
            reposts: 27,
            likes: 386,
            quotes: 0,
            views: Some(12_300),
        };
        post
    }

    fn options() -> RenderOptions {
        RenderOptions::from_engine(&EngineSettings::default())
    }

    fn emojis() -> EmojiSettings {
        EmojiSettings::default()
    }

    fn card<'a>(
        post: &'a BuiltinPost,
        options: &'a RenderOptions,
        resolved: &'a ResolvedMedia,
        emojis: &'a EmojiSettings,
    ) -> V2Card<'a> {
        V2Card {
            post,
            options,
            resolved,
            emojis,
            user_comment: None,
            impersonation: None,
            allowed_mentions: None,
        }
    }

    /// Kart, bileşen listesindeki `Container` bileşenidir (kullanıcı notu varsa üstünde
    /// düz metin bileşeni bulunur, bu yüzden sabit indeks kullanılmaz).
    fn container(payload: &Value) -> &Value {
        payload["components"]
            .as_array()
            .expect("bileşen listesi")
            .iter()
            .find(|component| component["type"] == TYPE_CONTAINER)
            .expect("kart (container) bileşeni")
    }

    /// Kartın **dışındaki** (container dışı) üst düzey bileşenler.
    fn top_level(payload: &Value) -> Vec<String> {
        payload["components"]
            .as_array()
            .expect("bileşen listesi")
            .iter()
            .filter(|component| component["type"] == TYPE_TEXT_DISPLAY)
            .map(|component| component["content"].as_str().unwrap().to_string())
            .collect()
    }

    fn components(payload: &Value) -> &Vec<Value> {
        container(payload)["components"].as_array().expect("bileşen listesi")
    }

    fn texts(payload: &Value) -> Vec<String> {
        components(payload)
            .iter()
            .filter(|component| component["type"] == TYPE_TEXT_DISPLAY)
            .map(|component| component["content"].as_str().unwrap().to_string())
            .collect()
    }

    fn gallery_urls(payload: &Value) -> Vec<String> {
        components(payload)
            .iter()
            .find(|component| component["type"] == TYPE_MEDIA_GALLERY)
            .map(|gallery| {
                gallery["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| item["media"]["url"].as_str().unwrap().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn card_is_a_container_without_accent_colour() {
        let post = sample_post();
        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));

        assert_eq!(payload["flags"], COMPONENTS_V2_FLAG);
        assert_eq!(container(&payload)["type"], TYPE_CONTAINER);
        assert!(
            container(&payload).get("accent_color").is_none(),
            "sol renkli çubuk kaldırıldı: {}",
            container(&payload)
        );
        assert!(payload.get("content").is_none(), "v2 mesajlarında content kullanılmaz");
        assert_eq!(payload["attachments"], json!([]));
    }

    #[test]
    fn author_line_matches_the_x_layout() {
        let post = sample_post();
        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));

        assert_eq!(
            texts(&payload)[0],
            "**Culture Base <:verified:1553709836859084841> ([@Culture3ase](https://x.com/Culture3ase))**"
        );
        // Avatar/görsel bileşeni (Section/Thumbnail) kullanılmaz
        assert!(
            !components(&payload).iter().any(|c| c["type"] == 9 || c["type"] == 11),
            "yazar satırında section/thumbnail olmamalı"
        );
    }

    #[test]
    fn author_line_omits_verified_marker_and_link_when_missing() {
        let mut post = sample_post();
        post.author.verified = false;
        post.author.profile_url = None;

        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));
        assert_eq!(texts(&payload)[0], "**Culture Base (@Culture3ase)**");
    }


    #[test]
    fn user_comment_is_sent_above_the_card_like_a_user_message() {
        let post = sample_post();
        let options = options();
        let emojis = emojis();
        let resolved = ResolvedMedia::default();

        let mut card = card(&post, &options, &resolved, &emojis);
        card.user_comment = Some("  şuna bak\nmükemmel  ");
        let payload = build_payload(&card);

        // Kullanıcı metni kartın **dışında**, en üstte ve alıntı bloğuna (`>`) çevrilmeden durur.
        assert_eq!(top_level(&payload), vec!["şuna bak\nmükemmel".to_string()]);
        assert_eq!(
            payload["components"][0]["type"], TYPE_TEXT_DISPLAY,
            "kullanıcı metni kartın üstünde olmalı"
        );
        assert_eq!(
            payload["components"][1]["type"], TYPE_CONTAINER,
            "kart kullanıcı metninin altında olmalı"
        );
        assert!(
            !texts(&payload).iter().any(|text| text.contains("şuna bak")),
            "kullanıcı metni kartın içine konmaz: {:?}",
            texts(&payload)
        );
    }

    #[test]
    fn card_stays_alone_when_the_user_wrote_nothing() {
        let post = sample_post();
        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));

        assert!(top_level(&payload).is_empty(), "yorum yoksa üstte metin bileşeni olmaz");
        assert_eq!(payload["components"].as_array().unwrap().len(), 1, "yalnızca kart");
    }

    #[test]
    fn text_metrics_and_meta_lines_are_inside_the_card() {
        let post = sample_post();
        let options = options();
        let emojis = emojis();
        let resolved = ResolvedMedia::default();

        let payload = build_payload(&card(&post, &options, &resolved, &emojis));

        let texts = texts(&payload);
        assert!(texts.iter().any(|t| t == "merhaba dünya"), "gönderi metni: {texts:?}");

        assert!(
            texts.iter().any(|t| t
                == "-# <:comment:1553712370302451782> 6  <:rt:1553712344939495424> 27  <:like:1553712395057365102> 386  <:metric:1553712416242798712> 12.3K"),
            "metrik satırı: {texts:?}"
        );

        let meta = texts.last().expect("son satır");
        assert_eq!(
            meta,
            "-# <:twitter:1476642012072920355> • <t:1714998896:R> • [Link](https://x.com/kullanici/status/1)"
        );
        assert!(!meta.contains("Botgek"), "kart marka satırı içermemeli: {meta}");
        assert!(!meta.contains("<https://"), "link hilesi olmamalı: {meta}");
    }

    #[test]
    fn metrics_and_timestamp_follow_the_settings() {
        let post = sample_post();
        let quiet = RenderOptions {
            show_metrics: false,
            show_timestamp: false,
            ..options()
        };
        let emojis = emojis();
        let payload = build_payload(&card(&post, &quiet, &ResolvedMedia::default(), &emojis));
        let texts = texts(&payload);

        assert!(!texts.iter().any(|t| t.contains("<:comment:")), "metrik kapalı: {texts:?}");
        assert_eq!(texts.last().unwrap(), "-# <:twitter:1476642012072920355> • [Link](https://x.com/kullanici/status/1)");
    }

    #[test]
    fn non_twitter_platforms_use_their_name_and_extra_label() {
        let mut post = BuiltinPost::new(
            Platform::Reddit,
            "https://www.reddit.com/r/rust/comments/abc/x/",
            Author::new("rustfan", "rustfan"),
        );
        post.text = "reddit gönderisi".to_string();
        post.extra_label = Some("r/rust • 128 yorum".to_string());
        post.stats = Stats {
            replies: 128,
            reposts: 0,
            likes: 42,
            quotes: 0,
            views: None,
        };

        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));
        let meta = texts(&payload).last().unwrap().to_string();

        assert!(meta.contains("Reddit"), "platform adı: {meta}");
        assert!(meta.contains("r/rust • 128 yorum"), "ek bilgi: {meta}");
        assert!(!meta.contains("twitter:"), "X emojisi kullanılmamalı: {meta}");
    }

    #[test]
    fn translation_marker_is_shown_before_the_platform() {
        let mut post = sample_post();
        post.translation = Some(Translation {
            source: Some("pt".to_string()),
        });

        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));
        let meta = texts(&payload).last().unwrap().to_string();

        assert!(
            meta.starts_with("-# <:translation:1476614397249061056> pt → tr • <:twitter:1476642012072920355> • "),
            "çeviri işareti: {meta}"
        );
    }

    #[test]
    fn unicode_fallbacks_are_used_when_custom_emojis_are_empty() {
        let mut post = sample_post();
        let options = options();
        let emojis = EmojiSettings {
            verified: String::new(),
            replies: String::new(),
            reposts: String::new(),
            likes: String::new(),
            views: String::new(),
            translation: String::new(),
            twitter: String::new(),
            quote: String::new(),
        };
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));
        let lines = texts(&payload);

        assert_eq!(
            lines[0],
            "**Culture Base ☑ ([@Culture3ase](https://x.com/Culture3ase))**"
        );
        assert!(lines.iter().any(|t| t.contains("💬 6")), "metrik: {lines:?}");
        assert!(
            lines.last().unwrap().contains("Twitter • <t:"),
            "platform adı: {:?}",
            lines.last()
        );

        // Emoji kapalıysa yalnızca istatistik alanları ile çeviri notu Unicode'a düşer
        post.translation = Some(Translation { source: None });
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));
        assert!(texts(&payload).last().unwrap().contains("🌐 Türkçeye çevrildi"));
    }


    #[test]
    fn attached_video_plays_inside_the_gallery() {
        let mut post = sample_post();
        post.media = vec![video("https://cdn/v1.mp4", "https://cdn/poster.jpg")];

        let resolved = ResolvedMedia {
            attachments: vec![serenity::all::CreateAttachment::bytes(vec![1, 2, 3], "video.mp4")],
            attached_video: Some("attachment://video.mp4".to_string()),
            ..ResolvedMedia::default()
        };
        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &resolved, &emojis));

        assert_eq!(gallery_urls(&payload), vec!["attachment://video.mp4"]);
        assert_eq!(
            payload["attachments"],
            json!([{"id": 0, "filename": "video.mp4"}]),
            "yüklenen dosya payload'da listelenmeli"
        );

        let meta = texts(&payload).pop().unwrap();
        assert!(!meta.contains("Videoyu aç"), "video kart içindeyse link gerekmez: {meta}");
    }

    #[test]
    fn remote_video_is_shown_once_inside_the_gallery() {
        let mut post = sample_post();
        post.media = vec![video("https://cdn/v1.mp4", "https://cdn/poster.jpg")];

        let resolved = ResolvedMedia {
            video_link: Some("https://cdn/v1.mp4".to_string()),
            ..ResolvedMedia::default()
        };
        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &resolved, &emojis));

        assert_eq!(
            gallery_urls(&payload),
            vec!["https://cdn/v1.mp4"],
            "galeri kapak + video olarak iki öğe göstermemeli (yan yana çıkıyordu)"
        );
        let meta = texts(&payload).pop().unwrap();
        assert!(meta.contains("[🎬 Videoyu aç](https://cdn/v1.mp4)"), "son satır: {meta}");
    }

    #[test]
    fn gallery_keeps_post_images_and_quote_image() {
        let mut post = sample_post();
        let mut quote = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/diger/status/2",
            Author::new("Diğer", "diger"),
        );
        quote.text = "alıntı".to_string();
        quote.media = vec![image("https://cdn/quote.jpg")];
        post.quote = Some(Box::new(quote));

        let resolved = ResolvedMedia {
            images: vec![
                "attachment://photo.jpg".to_string(),
                "https://cdn/second.jpg".to_string(),
            ],
            ..ResolvedMedia::default()
        };
        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &resolved, &emojis));

        assert_eq!(
            gallery_urls(&payload),
            vec!["attachment://photo.jpg", "https://cdn/second.jpg", "https://cdn/quote.jpg"]
        );

        let quote_text = texts(&payload)
            .into_iter()
            .find(|text| text.contains("<:qutoe:"))
            .expect("alıntı bileşeni");
        assert!(quote_text.contains("> \u{2003}\u{2003}alıntı"), "alıntı girintili olmalı: {quote_text}");
        assert!(
            quote_text.contains("[Quote](https://x.com/diger/status/2) from **Diğer** (@diger)"),
            "alıntı başlığı fxembed düzeninde olmalı: {quote_text}"
        );
        assert_eq!(
            quote_text.lines().next().unwrap(),
            "-# \u{2003}\u{2003}<:qutoe:1553728214998847569> [Quote](https://x.com/diger/status/2) from **Diğer** (@diger)"
        );
    }

    #[test]
    fn quote_header_links_the_handle_when_the_profile_is_known() {
        let mut post = sample_post();
        let mut quote = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/Daykerpriest/status/2103751786901144035",
            Author::new("Daykerpriest🐉", "Daykerpriest"),
        );
        quote.text = "alıntılanan metin".to_string();
        quote.author.profile_url = Some("https://x.com/daykerpriest".to_string());
        post.quote = Some(Box::new(quote));

        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));

        let quote_text = texts(&payload)
            .into_iter()
            .find(|text| text.contains("<:qutoe:"))
            .expect("alıntı bileşeni");

        assert_eq!(
            quote_text.lines().next().unwrap(),
            "-# \u{2003}\u{2003}<:qutoe:1553728214998847569> [Quote](https://x.com/Daykerpriest/status/2103751786901144035) \
             from **Daykerpriest🐉** ([**@Daykerpriest**](https://x.com/daykerpriest))"
        );
        assert!(
            quote_text.contains("> \u{2003}\u{2003}alıntılanan metin"),
            "blok alıntı girintili olmalı: {quote_text}"
        );
    }

    /// Alıntı, kartın gövdesinden hem dikey boşlukla ayrılır (görünmez `Separator`)
    /// hem de sağa girintilenir (Reddit/Twitter alıntı hiyerarşisi).
    #[test]
    fn quote_block_is_separated_and_indented_from_the_card_body() {
        let mut post = sample_post();
        let mut quote = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/diger/status/2",
            Author::new("Diğer", "diger"),
        );
        quote.text = "satır bir\nsatır iki".to_string();
        post.quote = Some(Box::new(quote));

        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));

        let children = components(&payload);
        let quote_index = children
            .iter()
            .position(|component| {
                component["type"] == TYPE_TEXT_DISPLAY
                    && component["content"].as_str().unwrap_or_default().contains("<:qutoe:")
            })
            .expect("alıntı bileşeni");

        let before = &children[quote_index - 1];
        assert_eq!(before["type"], TYPE_SEPARATOR, "alıntıdan önce ayraç olmalı");
        assert_eq!(before["divider"], false, "ayraç çizgi çizmemeli");
        assert_eq!(before["spacing"], 1, "ayraç küçük boşluk bırakmalı");

        let quote_text = children[quote_index]["content"].as_str().unwrap();
        let lines: Vec<&str> = quote_text.lines().collect();
        assert!(
            lines[0].starts_with("-# \u{2003}"),
            "başlık alt metin olarak kalmalı ama girintilenmeli: {}",
            lines[0]
        );
        for line in &lines[1..] {
            assert!(
                line.starts_with("> \u{2003}\u{2003}"),
                "alıntı satırı blok alıntı + girinti ile başlamalı: {line}"
            );
        }
        assert_eq!(lines.len(), 3, "başlık + iki alıntı satırı");

        // Alıntı yoksa gereksiz ayraç eklenmez.
        let plain = build_payload(&card(
            &sample_post(),
            &options,
            &ResolvedMedia::default(),
            &emojis,
        ));
        assert_eq!(
            components(&plain)
                .iter()
                .filter(|component| component["type"] == TYPE_SEPARATOR)
                .count(),
            0,
            "alıntı yokken ayraç eklenmez"
        );
    }

    #[test]
    fn quoted_video_is_shown_when_the_post_has_no_media() {
        let mut post = sample_post();
        let mut quote = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/diger/status/2",
            Author::new("Diğer", "diger"),
        );
        quote.text = "alıntı".to_string();
        quote.media = vec![video("https://cdn/q.mp4", "https://cdn/q.jpg")];
        post.quote = Some(Box::new(quote));

        // İndirilemeyen (alıntıdan gelen) video kartın içinde tek öğe olarak gösterilir.
        let resolved = ResolvedMedia {
            video_link: Some("https://cdn/q.mp4".to_string()),
            quote_media: true,
            ..ResolvedMedia::default()
        };
        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &resolved, &emojis));

        assert_eq!(
            gallery_urls(&payload),
            vec!["https://cdn/q.mp4"],
            "alıntının videosu kartın içinde (kapağıyla birlikte değil, tek başına) gösterilmeli"
        );
        let meta = texts(&payload).pop().unwrap();
        assert!(meta.contains("[🎬 Videoyu aç](https://cdn/q.mp4)"), "son satır: {meta}");
    }

    #[test]
    fn sensitive_posts_are_wrapped_and_gallery_items_are_blurred() {
        let mut post = sample_post();
        post.sensitive = true;

        let resolved = ResolvedMedia {
            images: vec!["attachment://SPOILER_photo.jpg".to_string()],
            ..ResolvedMedia::default()
        };
        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &resolved, &emojis));

        assert_eq!(components(&payload)[1]["content"], "||merhaba dünya||");
        let gallery = components(&payload)
            .iter()
            .find(|component| component["type"] == TYPE_MEDIA_GALLERY)
            .expect("galeri");
        assert_eq!(gallery["items"][0]["spoiler"], true);

        let meta = texts(&payload).pop().unwrap();
        assert!(meta.contains("||[Link](https://x.com/kullanici/status/1)||"), "spoiler link: {meta}");
    }

    #[test]
    fn impersonation_sets_username_and_avatar() {
        let post = sample_post();
        let options = options();
        let emojis = emojis();
        let resolved = ResolvedMedia::default();
        let identity = Impersonation {
            username: "Eray".to_string(),
            avatar_url: Some("https://cdn.example/eray.png".to_string()),
        };

        let mut card = card(&post, &options, &resolved, &emojis);
        card.impersonation = Some(&identity);
        let payload = build_payload(&card);

        assert_eq!(payload["username"], "Eray");
        assert_eq!(payload["avatar_url"], "https://cdn.example/eray.png");
    }

    #[test]
    fn long_texts_are_truncated_to_discord_limits() {
        let mut post = sample_post();
        post.text = "a".repeat(TEXT_DISPLAY_LIMIT + 500);
        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));
        let body = components(&payload)[1]["content"].as_str().unwrap();
        assert_eq!(body.chars().count(), TEXT_DISPLAY_LIMIT);
    }
}

