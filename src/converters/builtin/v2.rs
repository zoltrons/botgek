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
//! │ **Ad <:verified:...> ([@kullanici](https://x.com/kullanici))**   ← kalın yazar satırı
//! │    (Reddit: `**r/alt • flair • Posted by [u/kullanici](profil)**`)   ← profil resmi yok
//! │ gönderi metni
//! │ ────────────────────── (varsa) ince ayraç çizgisi ─────────
//! │ > <:qutoe:...> [Quote](link) from **Ad** ([**@kullanici**](profil))
//! │ > alıntılanan metin            ← başlıkla aynı hizada (blok alıntı)
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
use serenity::all::UserId;

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
/// Tüm bileşenler genelinde toplam kümülatif metin sınırı (Discord Developer Docs: max 4,000 chars across all components).
pub const CUMULATIVE_TEXT_LIMIT: usize = 4000;
/// `Media Gallery` bileşeni en fazla 10 öğe alır.
pub const MEDIA_GALLERY_LIMIT: usize = 10;
/// Kart (Container) içindeki bileşen sayısı (Discord API sınırı 10).
const MAX_CONTAINER_CHILDREN: usize = 10;

/// Kart alt bilgisindeki **Sil** butonunun `custom_id` değeri.
///
/// Buton her kartta aynı kimliği taşır: hangi mesajın silineceği, etkileşimin `message`
/// alanından okunur (bkz. `handler::card_delete`). Discord `custom_id` sınırı 100 karakterdir.
pub const CARD_DELETE_ID: &str = "card:delete";

// Bileşen türleri
const TYPE_BUTTON: u64 = 2;
const TYPE_SECTION: u64 = 9;
const TYPE_TEXT_DISPLAY: u64 = 10;
const TYPE_THUMBNAIL: u64 = 11;
const TYPE_MEDIA_GALLERY: u64 = 12;
const TYPE_SEPARATOR: u64 = 14;
const TYPE_CONTAINER: u64 = 17;

/// Kartı oluşturmak için gereken tüm girdiler.
pub struct V2Card<'a> {
    pub post: &'a BuiltinPost,
    pub options: &'a RenderOptions,
    pub resolved: &'a ResolvedMedia,
    /// Modern kartta kullanılacak özel emoji seti.
    pub emojis: &'a EmojiSettings,
    /// Kullanıcı linki `||spoiler||` içine aldı mı? (kart içeriğini gizler)
    pub spoiler: bool,
    /// Kart bot adına gönderiliyor (yanıtta taklit kapalı) ve kartın üstüne paylaşanın
    /// etiketi konacaksa o kullanıcı. Etiket ping atmaz: `allowed_mentions` listesinde
    /// bulunmadığı için düz `@kullanıcı` metni olarak görünür.
    pub sender: Option<UserId>,
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
fn total_text_chars(top: &[Value], children: &[Value]) -> usize {
    let count_comp = |comp: &Value| -> usize {
        if comp["type"] == TYPE_TEXT_DISPLAY {
            comp["content"]
                .as_str()
                .map(|s| s.chars().count())
                .unwrap_or(0)
        } else if comp["type"] == TYPE_SECTION {
            comp["components"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter(|c| c["type"] == TYPE_TEXT_DISPLAY)
                        .filter_map(|c| c["content"].as_str())
                        .map(|s| s.chars().count())
                        .sum()
                })
                .unwrap_or(0)
        } else {
            0
        }
    };

    top.iter().chain(children.iter()).map(count_comp).sum()
}

fn enforce_cumulative_text_limit(top: &mut [Value], children: &mut [Value]) {
    while total_text_chars(top, children) > CUMULATIVE_TEXT_LIMIT {
        let overflow = total_text_chars(top, children) - CUMULATIVE_TEXT_LIMIT;
        let mut displays: Vec<&mut Value> = Vec::new();
        for comp in top.iter_mut().chain(children.iter_mut()) {
            if comp["type"] == TYPE_TEXT_DISPLAY {
                displays.push(comp);
            } else if comp["type"] == TYPE_SECTION
                && let Some(arr) = comp["components"].as_array_mut()
            {
                for child in arr.iter_mut() {
                    if child["type"] == TYPE_TEXT_DISPLAY {
                        displays.push(child);
                    }
                }
            }
        }

        let largest = displays.into_iter().max_by_key(|c| {
            c["content"]
                .as_str()
                .map(|s| s.chars().count())
                .unwrap_or(0)
        });

        if let Some(comp) = largest {
            let current = comp["content"].as_str().unwrap_or_default();
            let current_len = current.chars().count();
            if current_len == 0 {
                break;
            }
            // Discord boş metin bileşenini reddeder: en kısa hâli spoiler + `…` sığacak kadardır.
            let new_len = current_len.saturating_sub(overflow).max(5);
            let shortened = truncate_display(current, new_len);
            if shortened.chars().count() >= current_len {
                break;
            }
            comp["content"] = json!(shortened);
        } else {
            break;
        }
    }
}

pub fn build_payload(card: &V2Card<'_>) -> Value {
    // Kullanıcı notu kartın dışına çıkar: alıntı bloğuna (`>`) çevrilmez, olduğu gibi yazılır.
    let mut top: Vec<Value> = card
        .user_comment
        .map(str::trim)
        .filter(|comment| !comment.is_empty())
        .map(text)
        .into_iter()
        .collect();

    // Gönderen etiketi üstte ayrı bir bileşen olarak **konmaz**: kart bot adına gönderildiğinde
    // paylaşanın adı en alt satırda `Gönderen: @kullanıcı` olarak görünür (bkz. [`meta_line`]
    // ve [`V2Card::sender`]).

    let accent = card
        .post
        .custom_card
        .as_ref()
        .and_then(|custom| custom.accent_color);

    let mut children: Vec<Value> = match &card.post.custom_card {
        // Platforma özel kart (ör. Spotify): çocuk bileşenler platform modülünde hazır üretilir.
        Some(custom) => custom.components.clone(),
        None => generic_children(card),
    };
    // Custom cards are cached as source data and must receive the same per-send decision as
    // generic cards. This covers Spotify title/body text and thumbnail/media components.
    if hidden(card) && card.post.custom_card.is_some() {
        apply_spoiler_to_components(&mut children);
    }

    enforce_cumulative_text_limit(&mut top, &mut children);

    // Kart her zaman son bileşendir: kullanıcı metni (varsa) onun üstünde durur.
    let mut container = json!({
        "type": TYPE_CONTAINER,
        "components": children,
    });
    if let Some(color) = accent {
        container["accent_color"] = json!(color);
    }
    top.push(container);

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

/// Gönderinin içindeki harici bağlantının önizlemesi (X'in "link kartı").
///
/// X'in arayüzünde linkin altında görünen başlık (bağlantılı), açıklama ve site adı kartın
/// içinde tek bir metin bloğu olarak durur; önizleme görseli medya galerisinde büyük olarak
/// gösterilir (bkz. [`super::render::plan_media`]). Hiçbir şey indirilmez.
fn link_preview_component(card: &V2Card<'_>) -> Option<Value> {
    let preview = card
        .post
        .link_preview
        .as_ref()
        .filter(|preview| !preview.is_empty())?;

    // Reddit: Discord'un yerleşik önizlemesi dış bağlantıyı kartın ortasında **tek satırda**
    // `Site | Başlık` olarak gösterir (açıklama/görsel/görsel yuvası yok).
    if card.post.platform == Platform::Reddit {
        let site = preview
            .site_name
            .as_deref()
            .map(str::trim)
            .filter(|site| !site.is_empty());
        let title = preview
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty());
        // Parçalar ayrı ayrı kaçırılır; ayırıcı ` | ` düz kalır (kaçış kartta ters bölü gösterir).
        let label = match (site, title) {
            (Some(site), Some(title)) if site != title => {
                format!("{} | {}", escape_markdown(site), escape_markdown(title))
            }
            (Some(site), _) => escape_markdown(site),
            (None, Some(title)) => escape_markdown(title),
            (None, None) => escape_markdown(&preview.url),
        };
        let line = format!("**🔗 [{label}]({})**", preview.url);
        return Some(text(spoiler_wrap(&line, hidden(card))));
    }

    let mut lines: Vec<String> = Vec::new();
    let label = preview.title.as_deref().unwrap_or(preview.url.as_str());
    lines.push(format!(
        "**🔗 [{}]({})**",
        escape_markdown(label),
        preview.url
    ));
    if let Some(description) = preview.description.as_deref() {
        for line in description
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .take(5)
        {
            lines.push(format!("> {line}"));
        }
    }
    if let Some(site_name) = preview.site_name.as_deref() {
        lines.push(format!("-# {site_name}"));
    }

    Some(text(spoiler_wrap(&lines.join("\n"), hidden(card))))
}

/// Genel kart düzeni: yazar satırı, (Reddit için) başlık, gövde metni,
/// (varsa) topluluk notu, alıntı(lar),
/// medya galerisi, metrikler ve alt bilgi. Platforma özel kartı olmayan tüm gönderiler
/// bu yolu kullanır.
fn generic_children(card: &V2Card<'_>) -> Vec<Value> {
    let mut children: Vec<Value> = Vec::new();
    let header = header_line(card.post, card.emojis);
    if !header.is_empty() {
        children.push(text(header));
    }

    if card.post.platform == Platform::Reddit
        && let Some(title) = card
            .post
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
    {
        children.push(text(spoiler_wrap(
            &format!("**{}**", escape_markdown(title)),
            hidden(card),
        )));
    }
    if !card.post.text.trim().is_empty() {
        children.push(text(spoiler_wrap(card.post.text.trim(), hidden(card))));
    }
    if card.options.show_community_note
        && let Some(note) = card
            .post
            .community_note
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
    {
        children.push(text(community_note_content(card.post.platform, note)));
    }
    if let Some(preview) = link_preview_component(card) {
        children.push(preview);
    }
    let mut nested_has_thumbnail = false;
    if card.options.show_quote
        && let Some(quote) = card.post.quote.as_deref()
    {
        // Gönderiye **medya eklenmemişse** alıntının alıntısı da gösterilir: X'in kendi
        // kartı medyasız zincirlerde üç gönderiyi (ana + alıntı + alıntının alıntısı)
        // birlikte gösterir. Medya varsa kart medyaya odaklanır, zincir iki seviyede kalır.
        let nested = if card.post.media.is_empty() {
            quote.quote.as_deref()
        } else {
            None
        };

        if let Some(component) = quote_component(quote, card.emojis, hidden(card)) {
            // Hiyerarşi: alıntı bloğundan önce ince bir ayraç çizgisi ve büyük bir dikey
            // boşluk bırakılır, böylece alıntı kartın gövdesinden ayrı okunur.
            children.push(separator());
            children.push(component);
        }
        if let Some(nested) = nested
            && let Some(component) =
                quote_component(nested, card.emojis, hidden(card) || quote.sensitive)
        {
            // İç içe alıntı kendi ayracıyla ayrılır: iki farklı gönderi olduğu
            // başlık satırından (`[Quote](link) from **Ad**`) anlaşılır.
            children.push(separator());

            // 3'lü alıntıda medya en alttaki orijinal tweete ait tek bir görselse
            // ayrı bir büyük galeri yerine alıntı metninin yanında küçük görsel (Section
            // Thumbnail aksesuarı) olarak gösterilir (X'in arayüz düzeni).
            let is_nested_image = card.post.media.is_empty()
                && quote.media.is_empty()
                && !nested.media.is_empty()
                && card.resolved.attached_video.is_none()
                && card.resolved.video_link.is_none()
                && card.resolved.images.len() == 1;

            if is_nested_image && let Some(image_url) = card.resolved.images.first() {
                let accessory = thumbnail(image_url, hidden(card));
                children.push(section(vec![component], accessory));
                nested_has_thumbnail = true;
            } else {
                children.push(component);
            }
        }
    }
    if let Some(gallery) = media_gallery(card, nested_has_thumbnail) {
        children.push(gallery);
    }
    if let Some(metrics) = metrics_line(card.post, card.options, card.emojis) {
        children.push(text(metrics));
    }
    children.push(meta_component(card));

    if children.len() > MAX_CONTAINER_CHILDREN {
        let last = children.pop();
        children.truncate(MAX_CONTAINER_CHILDREN - 1);
        if let Some(meta) = last {
            children.push(meta);
        }
    }

    children
}

/// X'in "Readers added context" bloğu (topluluk notu).
///
/// Notun tamamı **tek blok alıntı** içinde durur ve her satır `> ` ile üretilir. **Hiçbir
/// satır çıplak `>` ya da boş alıntı satırı olmaz**: Discord bunları kartta ayrı/boş bir şerit
/// — kullanıcının gördüğü fazladan `>` işareti — olarak çiziyor (aynı gerekçeyle
/// `quote_component` başlıktan sonra boş alıntı satırı eklemez). Notun kendi boş satırları
/// paragraf aralığı olarak değil, süzülerek atlanır (bkz. `link_preview_component`).
fn community_note_content(platform: Platform, note: &str) -> String {
    if platform != Platform::Twitter {
        return format!("-# 🔎 Topluluk Notu\n{note}");
    }

    let mut lines = vec!["> 👥 **Readers added context**".to_string()];
    lines.extend(
        note.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| format!("> {line}")),
    );
    lines.join("\n")
}

/// `{"type": 10, "content": ...}` — markdown destekli metin bileşeni.
///
/// Sınıra sığan içerik **kopyalanmaz** (kart başına onlarca bileşen üretildiği için ikinci
/// ayırmadan kaçınılır); yalnızca gerçekten uzun içerik kısaltılır.
fn text(content: impl Into<String>) -> Value {
    let content = content.into();
    let content = if content.len() <= TEXT_DISPLAY_LIMIT {
        content
    } else {
        truncate_display(&content, TEXT_DISPLAY_LIMIT)
    };
    json!({
        "type": TYPE_TEXT_DISPLAY,
        "content": content,
    })
}

/// Kartın **üst satırı** (yazar/kimlik satırı).
///
/// Kullanıcı adı **kalın ve büyük** gösterilir (önceki tasarım): X'te
/// `**Ad <:verified:> ([@kullanıcı](profil))**`, Reddit'te
/// `**r/alt • flair • Posted by [u/kullanıcı](profil)**`. Dolgu/alt bilgi satırları
/// (`-# `) yalnızca metrik, çeviri notu ve link için kullanılır.
///
/// Profil resmi (avatar) hiçbir platformda gösterilmez: Discord kartın sağ üstünde ayrı bir
/// görsel yuvası açar ve bu alan istenmiyor.
fn header_line(post: &BuiltinPost, emojis: &EmojiSettings) -> String {
    let body = if post.platform == Platform::Reddit {
        reddit_header_body(post)
    } else {
        author_line(post, emojis)
    };
    if body.trim().is_empty() {
        return String::new();
    }
    format!("**{body}**")
}

/// Reddit üst satırının gövdesi:
/// `r/alt • <flair> • Posted by [u/kullanici](profil)` (yerleşik Reddit önizlemesi düzeni).
///
/// Topluluk etiketi (`extra_label`) ve flair **burada** gösterilir; bu yüzden alt bilgide
/// tekrarlanmaz. Profil bağlantısı biliniyorsa `u/kullanici` tıklanabilir olur.
fn reddit_header_body(post: &BuiltinPost) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(label) = post
        .extra_label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
    {
        parts.push(label.to_string());
    }
    let name = post.author.name.trim();
    let author = if name.is_empty() {
        post.author.handle.trim()
    } else {
        name
    };
    if !author.is_empty() {
        let author = escape_markdown(author);
        let profile = post
            .author
            .profile_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty());
        parts.push(match profile {
            Some(url) => format!("Posted by [u/{author}]({url})"),
            None => format!("Posted by u/{author}"),
        });
    }
    parts.join(" • ")
}

/// `Ad <:verified:...> ([@kullanici](profil))` biçimindeki yazar metni (kalınlık/ölçek çağıran
/// katmanda belirlenir).
fn author_line(post: &BuiltinPost, emojis: &EmojiSettings) -> String {
    let name = post.author.name.trim();
    let name = if name.is_empty() {
        post.author.handle.trim()
    } else {
        name
    };

    let mut inner = escape_markdown(name);
    if post.author.verified {
        inner.push(' ');
        inner.push_str(emojis.verified());
    }

    let handle = post.author.handle.trim();
    if handle.is_empty() {
        return inner;
    }
    let handle = escape_markdown(handle);
    match post
        .author
        .profile_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
    {
        Some(url) => format!("{inner} ([@{handle}]({url}))"),
        None => format!("{inner} (@{handle})"),
    }
}

/// Alıntılanan gönderi: tek satırlık başlık
/// `[Quote](link) from **Ad** ([**@kullanici**](profil))` ve alıntılanan metin, **aynı blok
/// alıntının** içinde ve başlığın hemen ardından gelir (X/fxembed düzeni).
fn quote_component(quote: &BuiltinPost, emojis: &EmojiSettings, spoiler: bool) -> Option<Value> {
    let body = quote.text.trim();
    let has_media = !quote.media.is_empty();
    if body.is_empty() && !has_media {
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

    let profile = quote
        .author
        .profile_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty());
    let handle = quote.author.handle.trim();
    let from = match (handle.is_empty(), profile) {
        (false, Some(profile)) => {
            format!(
                "from **{author}** ([**@{}**]({profile}))",
                escape_markdown(handle)
            )
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

    // Başlık da alıntı bloğunun **içinde** durur: böylece başlık ile alıntılanan metin aynı
    // hizada başlar ve alıntının tamamı tek bir sol şeritle gruplanır (X/fxembed düzeni).
    // Başlıktan sonra **boş bir alıntı satırı eklenmez**: o satır Discord'da ayrı, boş bir
    // şerit parçası gibi görünüyordu; ardışık `> ` satırları tek alıntı sayılır. Ek
    // girinti/boşluk karakteri ve `-# ` alt metin işareti de kullanılmaz, çünkü Discord
    // blok alıntı içindeki satırları yalnızca satır içi kurallarla işler (`-# ` işareti
    // alıntı içinde düz metin olarak görünürdü).
    let mut lines = vec![format!("> {} {quote_link} {from}", emojis.quote())];
    if !body.is_empty() {
        for line in body.lines().take(8) {
            lines.push(format!("> {line}"));
        }
    }

    Some(text(spoiler_wrap(&lines.join("\n"), spoiler)))
}

/// İnce ayraç çizgisi + büyük dikey boşluk: alıntı bloğu kartın gövdesinden ayrı bir bölüm
/// olarak okunur (`divider: true` çizgiyi çizer, `spacing: 2` = büyük boşluk).
fn separator() -> Value {
    json!({ "type": TYPE_SEPARATOR, "divider": true, "spacing": 2 })
}

/// `Media Gallery`: video + görseller; hepsi kartın içinde.
///
/// Video **tek başına** gösterilir: ayrıca kapak görseli eklenmez. Kapak + video birlikte
/// eklendiğinde galeri iki öğeyi **yan yana** (thumbnail + video) çiziyordu.
///
/// Gönderiye medya eklenmemişse alıntının medyası zaten [`super::render::plan_media`]
/// tarafından `card.resolved` içine alınır ve galeride gösterilir. Gönderinin kendi medyası
/// varsa yalnızca o gösterilir (alıntılanan gönderinin medyası galeriye eklenmez).
/// `skip_images` doğru olduğunda (ör. alıntı görseli Section Thumbnail olarak gösterildiğinde)
/// görseller galeriye eklenmez.
fn media_gallery(card: &V2Card<'_>, skip_images: bool) -> Option<Value> {
    let hidden = hidden(card);
    let mut items: Vec<Value> = Vec::new();

    match (&card.resolved.attached_video, &card.resolved.video_link) {
        // Video dosya olarak eklendiyse doğrudan kart içinde oynar.
        (Some(reference), _) => items.push(media_item(reference, hidden)),
        // İndirilemediyse uzak video bağlantısı kartın içinde oynar (Discord videoyu
        // dış bağlantıdan da oynatır); kapak görseli eklenmez, aksi halde galeri
        // kapak + video olarak iki öğe gösterirdi.
        (None, Some(link)) => items.push(media_item(link, hidden)),
        (None, None) => {}
    }

    if !skip_images {
        for url in &card.resolved.images {
            items.push(media_item(url, hidden));
        }
    }

    if items.is_empty() {
        return None;
    }

    items.truncate(MEDIA_GALLERY_LIMIT);
    Some(json!({"type": TYPE_MEDIA_GALLERY, "items": items}))
}

/// `{"type": 2, "style": 4, ...}` — **kırmızı** (Danger) "Sil" butonu.
///
/// Discord metni renklendirmez; kırmızı görünüm buton stilinden gelir. Etiket (label) her
/// zaman beyazdır. Buton, bir Action Row ya da Section aksesuarı içinde durmak zorundadır.
pub fn delete_button() -> Value {
    json!({
        "type": TYPE_BUTTON,
        "style": 4,
        "label": "Sil",
        "custom_id": CARD_DELETE_ID,
    })
}

/// Alt bilgi satırı (`-# … Gönderen: @kullanıcı`) ve **hemen sağında** kırmızı "Sil" butonu.
///
/// Discord bileşenleri blok düzeyindedir: buton metnin *içine* gömülemez. Bu yüzden satır bir
/// `Section` olur ve buton onun `accessory` alanına konur; böylece buton o satırın hizasında,
/// kartın sağ ucunda durur ve kartın yüksekliği artmaz.
fn meta_component(card: &V2Card<'_>) -> Value {
    section(vec![text(meta_line(card))], delete_button())
}

/// Bölüm bileşeni (Section): solunda metin, sağında aksesuar (küçük görsel/thumbnail).
fn section(components: Vec<Value>, accessory: Value) -> Value {
    json!({
        "type": TYPE_SECTION,
        "components": components,
        "accessory": accessory,
    })
}

/// Küçük görsel aksesuarı (Thumbnail).
fn thumbnail(url: &str, spoiler: bool) -> Value {
    json!({
        "type": TYPE_THUMBNAIL,
        "media": { "url": url },
        "spoiler": spoiler,
    })
}

fn media_item(url: &str, spoiler: bool) -> Value {
    json!({"media": {"url": url}, "spoiler": spoiler})
}

/// Metrik satırı: `-# <:comment:...> 6  <:rt:...> 27  <:like:...> 386  <:metric:...> 12.3K`
/// Reddit yanıtları özel emojiyle, mevcut skoru (`likes`) iki oy oku arasında gösterilir.
fn metrics_line(
    post: &BuiltinPost,
    options: &RenderOptions,
    emojis: &EmojiSettings,
) -> Option<String> {
    if !options.show_metrics || post.stats.is_empty() {
        return None;
    }

    let stats = &post.stats;

    // Reddit: Discord'un yerleşik önizlemesiyle aynı düzen — `⬆ N upvotes • 💬 M comments`
    // (oy önce, yorum sonra; aşağı oy sayısı gösterilmez). Emojiler bizim özel setimizdir.
    if post.platform == Platform::Reddit {
        const REDDIT_UPVOTE: &str = "<:upvote:1558392173311823973>";
        const REDDIT_COMMENTS: &str = "<:redditcomments:1558393193018564718>";
        let mut reddit_parts: Vec<String> = Vec::new();
        if stats.likes > 0 {
            reddit_parts.push(format!(
                "{REDDIT_UPVOTE} {} upvotes",
                format_count(stats.likes)
            ));
        }
        if stats.replies > 0 {
            reddit_parts.push(format!(
                "{REDDIT_COMMENTS} {} comments",
                format_count(stats.replies)
            ));
        }
        if reddit_parts.is_empty() {
            return None;
        }
        return Some(format!("-# {}", reddit_parts.join(" • ")));
    }

    let mut parts: Vec<String> = Vec::new();
    if stats.replies > 0 {
        parts.push(format!(
            "{} {}",
            emojis.replies(),
            format_count(stats.replies)
        ));
    }
    if stats.reposts > 0 {
        parts.push(format!(
            "{} {}",
            emojis.reposts(),
            format_count(stats.reposts)
        ));
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

    // Platform göstergesi **her zaman en solda**: X, Instagram, Reddit ve Bluesky için özel
    // emoji, diğerlerinde platform adı yazılır.
    parts.push(match post.platform {
        Platform::Twitter => card
            .emojis
            .twitter()
            .map(str::to_string)
            .unwrap_or_else(|| post.platform.display_name().to_string()),
        Platform::Instagram => card
            .emojis
            .instagram()
            .map(str::to_string)
            .unwrap_or_else(|| post.platform.display_name().to_string()),
        Platform::Reddit => card
            .emojis
            .reddit()
            .map(str::to_string)
            .unwrap_or_else(|| post.platform.display_name().to_string()),
        Platform::Bluesky => "<:bluesky:1558392427729920100>".to_string(),
        Platform::Twitch
        | Platform::Pixiv
        | Platform::Tumblr
        | Platform::Facebook
        | Platform::YouTube => post.platform.display_name().to_string(),
        _ => post.platform.display_name().to_string(),
    });

    // Topluluk etiketi (`extra_label`) Reddit'te **üst satırda** gösterilir (bkz.
    // `reddit_header_line`); alt bilgide tekrarlanmaz.
    if post.platform != Platform::Reddit
        && let Some(extra) = post
            .extra_label
            .as_deref()
            .map(str::trim)
            .filter(|e| !e.is_empty())
    {
        parts.push(extra.to_string());
    }
    if card.options.show_timestamp
        && let Some(created_at) = post.created_at
    {
        parts.push(format!("<t:{created_at}:R>"));
    }

    // Çeviri notu **satır içi kod** (ters tırnak) olarak **saatin hemen yanında** durur:
    //   `Portekizce'den çevrildi`
    if let Some(translation) = &post.translation {
        parts.push(format!(
            "`{}`",
            super::translate::translated_from_note(translation.source.as_deref())
        ));
    }

    // Gönderi linki kartın içinde durur.
    let link = format!("[Link]({})", post.url);
    parts.push(spoiler_wrap(&link, hidden(card)));

    // Gönderen: kart bot adına gönderildiğinde (taklit kapalı) linki atan kişi **kartın
    // sağ altında** düz `@kullanıcı` olarak görünür. Etiket `allowed_mentions` ile
    // ping atmaz (bkz. `card_allowed_mentions`).
    if let Some(sender) = card.sender {
        parts.push(format!("Gönderen: <@{sender}>"));
    }

    // Not: indirilemeyen video için ayrıca "🎬 Videoyu aç" bağlantısı **eklenmez**; video
    // zaten kartın içindeki Media Gallery'de oynar (öğe başına tek gösterim yeterli).
    format!("-# {}", parts.join(" • "))
}

/// Hassas gönderilerde metni spoiler içine alır.
/// Metni kısaltır; `||...||` ile gizlenmiş metinde kapanış işaretini korur.
///
/// Kısaltma `||`'yi kesseydi gizli (hassas/spoiler) metin kartta açıkça görünürdü.
fn truncate_display(content: &str, max: usize) -> String {
    match content
        .strip_prefix("||")
        .and_then(|rest| rest.strip_suffix("||"))
    {
        Some(inner) if content.chars().count() > max => {
            format!(
                "||{}||",
                truncate_chars(inner, max.saturating_sub(4).max(1))
            )
        }
        _ => truncate_chars(content, max),
    }
}

fn spoiler_wrap(text: &str, spoiler: bool) -> String {
    if spoiler && !text.is_empty() {
        // Metindeki `||` gizlemeyi erken kapatıp devamını açığa çıkarmasın.
        format!("||{}||", text.replace("||", "|\u{200b}|"))
    } else {
        text.to_string()
    }
}

/// Kart içeriği gizlenmeli mi? Gönderi hassas olarak işaretliyse (`post.sensitive`) **veya**
/// kullanıcı linki `||spoiler||` içine aldıysa (`is_spoiler`) içerik spoiler olur: metin
/// `||...||` içine alınır, galeri öğeleri `spoiler` işaretlenir (klasik embed akışındaki
/// `||link||` davranışının modern kart karşılığı).
fn hidden(card: &V2Card<'_>) -> bool {
    card.post.sensitive || card.spoiler
}

/// Applies native Discord spoiler markers to an already-built custom component tree without
/// mutating the cached `CustomCard`. Components V2 uses `content` for text and `media` for
/// thumbnails/gallery items, so walking the JSON tree also covers future Spotify card details.
fn apply_spoiler_to_components(components: &mut [Value]) {
    for component in components {
        apply_spoiler_to_value(component);
    }
}

fn apply_spoiler_to_value(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        if let Some(content) = object.get("content").and_then(Value::as_str) {
            let content = content.to_string();
            object.insert(
                "content".to_string(),
                Value::String(spoiler_wrap(&content, true)),
            );
        }
        if object.contains_key("media") {
            object.insert("spoiler".to_string(), Value::Bool(true));
        }
        for child in object.values_mut() {
            apply_spoiler_to_value(child);
        }
    } else if let Some(array) = value.as_array_mut() {
        for child in array {
            apply_spoiler_to_value(child);
        }
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
    use crate::converters::builtin::{Author, LinkPreview, Media, MediaKind, Stats, Translation};
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
        RenderOptions::from_engine(&EngineSettings::default(), "twitter")
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
            spoiler: false,
            sender: None,
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
        container(payload)["components"]
            .as_array()
            .expect("bileşen listesi")
    }

    #[test]
    fn sensitive_custom_card_hides_text_and_media_without_mutating_source_card() {
        let mut post = sample_post();
        let source_components = vec![json!({
            "type": TYPE_SECTION,
            "components": [{"type": TYPE_TEXT_DISPLAY, "content": "Spotify başlık"}],
            "accessory": {"type": TYPE_THUMBNAIL, "media": {"url": "https://cdn/cover.jpg"}}
        })];
        post.custom_card = Some(crate::converters::builtin::CustomCard {
            components: source_components.clone(),
            accent_color: None,
        });
        post.sensitive = true;
        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));
        let container = container(&payload);
        let section = &container["components"][0];
        assert_eq!(section["components"][0]["content"], "||Spotify başlık||");
        assert_eq!(section["accessory"]["spoiler"], true);
        assert_eq!(
            post.custom_card.as_ref().unwrap().components,
            source_components,
            "cached custom card is not mutated"
        );
    }

    fn texts(payload: &Value) -> Vec<String> {
        let mut out = Vec::new();
        for component in components(payload) {
            if component["type"] == TYPE_TEXT_DISPLAY {
                out.push(component["content"].as_str().unwrap().to_string());
            } else if component["type"] == TYPE_SECTION
                && let Some(arr) = component["components"].as_array()
            {
                for c in arr {
                    if c["type"] == TYPE_TEXT_DISPLAY {
                        out.push(c["content"].as_str().unwrap().to_string());
                    }
                }
            }
        }
        out
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
        assert!(
            payload.get("content").is_none(),
            "v2 mesajlarında content kullanılmaz"
        );
        assert_eq!(payload["attachments"], json!([]));
    }

    #[test]
    fn author_line_matches_the_x_layout() {
        let post = sample_post();
        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));

        // Discord'un yerleşik X önizlemesi gibi üst satır **soluk/küçük** (`-# `) olur.
        assert_eq!(
            texts(&payload)[0],
            "**Culture Base <:verified:1553709836859084841> ([@Culture3ase](https://x.com/Culture3ase))**"
        );
        // Avatar/görsel bileşeni kullanılmaz: yazar satırı düz metindir.
        assert!(
            !components(&payload)
                .iter()
                .any(|c| c["type"] == TYPE_THUMBNAIL),
            "yazar satırında thumbnail olmamalı"
        );
        // Tek Section, alt bilgi satırındaki **Sil** butonunun aksesuarıdır.
        let sections: Vec<&Value> = components(&payload)
            .iter()
            .filter(|c| c["type"] == TYPE_SECTION)
            .collect();
        assert_eq!(sections.len(), 1, "yalnızca Sil butonu için section olmalı");
        assert_eq!(sections[0]["accessory"]["custom_id"], CARD_DELETE_ID);
        assert_eq!(components(&payload)[0]["type"], TYPE_TEXT_DISPLAY);
    }

    #[test]
    fn underscored_usernames_stay_clean_in_the_author_line() {
        // Regresyon: `_` kaçırıldığında kart `world\_record\_egg` yazıyordu (kullanıcı adında
        // olmayan `\`). Discord kelime içi `_`yi vurgu saymadığı için kaçış gerekmez.
        let mut post = sample_post();
        post.author.name = "Just An Egg".to_string();
        post.author.handle = "world_record_egg".to_string();
        post.author.profile_url = Some("https://x.com/world_record_egg".to_string());

        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));
        let author = texts(&payload)[0].to_string();

        assert!(
            author.contains("world_record_egg"),
            "kullanıcı adı: {author}"
        );
        assert!(
            !author.contains('\\'),
            "kullanıcı adında kaçış karakteri görünmemeli: {author}"
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

        assert!(
            top_level(&payload).is_empty(),
            "yorum yoksa üstte metin bileşeni olmaz"
        );
        assert_eq!(
            payload["components"].as_array().unwrap().len(),
            1,
            "yalnızca kart"
        );
    }

    #[test]
    fn text_metrics_and_meta_lines_are_inside_the_card() {
        let post = sample_post();
        let options = options();
        let emojis = emojis();
        let resolved = ResolvedMedia::default();

        let payload = build_payload(&card(&post, &options, &resolved, &emojis));

        let texts = texts(&payload);
        assert!(
            texts.iter().any(|t| t == "merhaba dünya"),
            "gönderi metni: {texts:?}"
        );

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
        assert!(
            !meta.contains("Botgek"),
            "kart marka satırı içermemeli: {meta}"
        );
        assert!(!meta.contains("<https://"), "link hilesi olmamalı: {meta}");
    }

    #[test]
    fn meta_line_carries_the_red_delete_button_on_its_right() {
        let post = sample_post();
        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));

        let meta_section = container(&payload)["components"]
            .as_array()
            .unwrap()
            .iter()
            .find(|component| component["accessory"]["custom_id"] == CARD_DELETE_ID)
            .expect("alt bilgi satırı Sil butonunu taşımalı");

        assert_eq!(
            meta_section["type"], TYPE_SECTION,
            "buton satır aksesuarı olmalı"
        );
        assert_eq!(meta_section["accessory"]["type"], TYPE_BUTTON);
        assert_eq!(
            meta_section["accessory"]["style"], 4,
            "buton kırmızı olmalı"
        );
        assert_eq!(meta_section["accessory"]["label"], "Sil");
        assert_eq!(meta_section["accessory"]["custom_id"], CARD_DELETE_ID);
        assert!(
            meta_section["components"][0]["content"]
                .as_str()
                .unwrap()
                .starts_with("-# "),
            "buton alt bilgi satırının hizasında durmalı"
        );
    }

    #[test]
    fn twitter_community_note_matches_the_readers_context_layout() {
        let mut post = sample_post();
        // Notun kendi boş satırı ("aralarda > karakteri" hatasının kaynağı) dahil edilir.
        post.community_note = Some(
            "İlk satır.\n\nİkinci satır: https://example.com/note\n   \nÜçüncü satır.".to_string(),
        );
        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));

        let notes: Vec<String> = texts(&payload)
            .into_iter()
            .filter(|text| text.contains("Readers added context"))
            .collect();
        assert_eq!(
            notes.len(),
            1,
            "tek topluluk notu bileşeni olmalı: {notes:?}"
        );
        let note = &notes[0];
        assert_eq!(
            note,
            "> 👥 **Readers added context**\n> İlk satır.\n> İkinci satır: https://example.com/note\n> Üçüncü satır."
        );
        // Çıplak `>` ve boş alıntı satırı **üretilmez**: Discord bunları kartta fazladan `>`
        // işareti / ayrı boş şerit olarak çiziyordu (bkz. `quote_component` ile aynı kural).
        assert!(
            !note.lines().any(|line| line.trim() == ">"),
            "çıplak/boş alıntı satırı üretilmemeli: {note}"
        );
        // Notun tamamı tek blok alıntı kalır (satırların hepsi `> ` ile başlar).
        assert!(
            note.lines().all(|line| line.starts_with("> ")),
            "notun tamamı blok alıntı olmalı: {note}"
        );
    }

    #[test]
    fn non_twitter_community_note_keeps_the_existing_layout() {
        let mut post = sample_post();
        post.platform = Platform::Reddit;
        post.community_note = Some("Reddit notu\nİkinci satır".to_string());
        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));

        assert!(
            texts(&payload)
                .iter()
                .any(|text| text == "-# 🔎 Topluluk Notu\nReddit notu\nİkinci satır")
        );
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

        assert!(
            !texts.iter().any(|t| t.contains("<:comment:")),
            "metrik kapalı: {texts:?}"
        );
        assert_eq!(
            texts.last().unwrap(),
            "-# <:twitter:1476642012072920355> • [Link](https://x.com/kullanici/status/1)"
        );
    }

    #[test]
    fn reddit_title_precedes_body_and_escapes_markdown() {
        let mut post = BuiltinPost::new(
            Platform::Reddit,
            "https://www.reddit.com/comments/abc/",
            Author::new("reddituser", ""),
        );
        post.title = Some("  Kulaklık **önerisi** [link]  ".to_string());
        post.text = "Gövde metni".to_string();
        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));
        let lines = texts(&payload);
        assert_eq!(lines[1], r"**Kulaklık \*\*önerisi\*\* \[link\]**");
        assert_eq!(lines[2], "Gövde metni");
    }

    #[test]
    fn reddit_title_respects_sensitive_and_user_spoilers() {
        let mut post = BuiltinPost::new(
            Platform::Reddit,
            "https://www.reddit.com/comments/abc/",
            Author::new("reddituser", ""),
        );
        post.title = Some("Başlık || metin".to_string());
        let options = options();
        let emojis = emojis();
        let resolved = ResolvedMedia::default();
        for (sensitive, spoiler) in [(true, false), (false, true)] {
            post.sensitive = sensitive;
            let mut card = card(&post, &options, &resolved, &emojis);
            card.spoiler = spoiler;
            let payload = build_payload(&card);
            assert_eq!(texts(&payload)[1], r"||**Başlık \|\| metin**||");
        }
    }

    #[test]
    fn absent_reddit_titles_and_other_platforms_keep_the_existing_layout() {
        let mut post = sample_post();
        let options = options();
        let emojis = emojis();
        let resolved = ResolvedMedia::default();
        let original = build_payload(&card(&post, &options, &resolved, &emojis));
        post.title = Some("Başka platform başlığı".to_string());
        assert_eq!(
            build_payload(&card(&post, &options, &resolved, &emojis)),
            original
        );
        post.platform = Platform::Reddit;
        post.title = None;
        let without_title = build_payload(&card(&post, &options, &resolved, &emojis));
        for title in ["", " \n\t "] {
            post.title = Some(title.to_string());
            assert_eq!(
                build_payload(&card(&post, &options, &resolved, &emojis)),
                without_title
            );
        }
    }

    #[test]
    fn reddit_uses_its_own_emoji_and_extra_label() {
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

        assert!(
            meta.contains("<:reddit:1558112156195495956>"),
            "Reddit kendi emojisini kullanmalı: {meta}"
        );
        // Topluluk etiketi (r/alt • flair) **üst satıra** taşındı: alt bilgide tekrarlanmaz.
        assert!(
            !meta.contains("r/rust • 128 yorum"),
            "ek bilgi üst satırda olmalı, alt bilgide tekrarlanmamalı: {meta}"
        );
        assert!(
            !meta.contains("twitter:"),
            "X emojisi kullanılmamalı: {meta}"
        );
        assert!(
            meta.contains("[Link]("),
            "gönderi bağlantısı korunur: {meta}"
        );
        // Üst satır: `r/alt • flair • Posted by u/kullanici` (yerleşik Reddit önizlemesi düzeni).
        assert_eq!(
            texts(&payload)[0],
            "**r/rust • 128 yorum • Posted by u/rustfan**"
        );
        // Metrikler: oy önce, yorum sonra; aşağı oy gösterilmez (yerleşik düzen).
        assert!(texts(&payload).iter().any(|line| line
            == "-# <:upvote:1558392173311823973> 42 upvotes • <:redditcomments:1558393193018564718> 128 comments"));
    }

    #[test]
    fn reddit_external_link_line_is_compact_and_carries_no_image() {
        let mut post = BuiltinPost::new(
            Platform::Reddit,
            "https://www.reddit.com/r/GlobalOffensive/comments/abc/x/",
            Author::new("walk3", "walk3"),
        );
        post.title = Some("Release Note for 3/8/2024".to_string());
        post.extra_label = Some("r/GlobalOffensive".to_string());
        post.link_preview = Some(LinkPreview {
            url: "https://blog.counter-strike.net/index.php/2024/03/steam-event/".to_string(),
            title: Some("Steam event".to_string()),
            description: None,
            image: None,
            site_name: Some("Counter-Strike.net".to_string()),
        });

        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));
        let lines = texts(&payload);

        assert!(
            lines.iter().any(|line| line
                == "**🔗 [Counter-Strike.net | Steam event](https://blog.counter-strike.net/index.php/2024/03/steam-event/)**"),
            "harici bağlantı tek satırda gösterilmeli: {lines:?}"
        );
        // Site adı ayrı satırda tekrarlanmaz ve link satırı görsel galerisi üretmez.
        assert!(
            !lines
                .iter()
                .any(|line| line.starts_with("-# Counter-Strike.net")),
            "site adı ayrı satırda olmamalı: {lines:?}"
        );
        assert!(
            !components(&payload)
                .iter()
                .any(|component| component["type"] == TYPE_MEDIA_GALLERY),
            "link satırı görsel yuvası/galerisi üretmemeli"
        );
    }

    #[test]
    fn every_platform_uses_the_bold_header_line() {
        let options = options();
        let emojis = emojis();
        for platform in [
            Platform::Twitter,
            Platform::TikTok,
            Platform::Instagram,
            Platform::Reddit,
            Platform::Bluesky,
            Platform::Spotify,
            Platform::Twitch,
            Platform::Pixiv,
            Platform::Tumblr,
            Platform::Facebook,
            Platform::YouTube,
        ] {
            let post = BuiltinPost::new(
                platform,
                "https://example.com/gonderi",
                Author::new("Ad", "kullanici"),
            );
            let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));
            let header = texts(&payload).first().cloned().unwrap_or_default();
            // Yazar/kullanıcı adı satırı **kalın** olur (dolgu satırları `-# ` ile ayrı kalır).
            assert!(
                header.starts_with("**") && header.ends_with("**"),
                "{platform:?} üst satırı kalın olmalı: {header}"
            );
            assert!(
                !header.starts_with("-# "),
                "{platform:?} üst satırı soluk olmamalı: {header}"
            );
        }
    }

    #[test]
    fn reddit_header_links_the_profile_and_stays_bold() {
        let mut post = BuiltinPost::new(
            Platform::Reddit,
            "https://www.reddit.com/r/rust/comments/abc/x/",
            Author::new("rustfan", "rustfan"),
        );
        post.extra_label = Some("r/rust".to_string());
        post.author.profile_url = Some("https://www.reddit.com/user/rustfan".to_string());

        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));

        assert_eq!(
            texts(&payload)[0],
            "**r/rust • Posted by [u/rustfan](https://www.reddit.com/user/rustfan)**"
        );
    }

    #[test]
    fn reddit_metrics_respect_zero_counts_and_settings() {
        let mut post = BuiltinPost::new(
            Platform::Reddit,
            "https://www.reddit.com/r/rust/comments/abc/x/",
            Author::new("rustfan", "rustfan"),
        );
        let mut options = options();
        let emojis = emojis();

        assert_eq!(metrics_line(&post, &options, &emojis), None);
        post.stats.likes = 42;
        assert_eq!(
            metrics_line(&post, &options, &emojis).as_deref(),
            Some("-# <:upvote:1558392173311823973> 42 upvotes")
        );
        post.stats.replies = 128;
        assert_eq!(
            metrics_line(&post, &options, &emojis).as_deref(),
            Some(
                "-# <:upvote:1558392173311823973> 42 upvotes • <:redditcomments:1558393193018564718> 128 comments"
            )
        );
        post.stats.likes = 0;
        assert_eq!(
            metrics_line(&post, &options, &emojis).as_deref(),
            Some("-# <:redditcomments:1558393193018564718> 128 comments")
        );
        post.stats.likes = 42;
        options.show_metrics = false;
        assert_eq!(metrics_line(&post, &options, &emojis), None);
    }

    #[test]
    fn bluesky_uses_its_own_platform_emoji() {
        let post = BuiltinPost::new(
            Platform::Bluesky,
            "https://bsky.app/profile/alice.bsky.social/post/3k6abc",
            Author::new("alice", "alice"),
        );
        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));

        assert_eq!(
            texts(&payload).last().unwrap(),
            "-# <:bluesky:1558392427729920100> • [Link](https://bsky.app/profile/alice.bsky.social/post/3k6abc)"
        );
    }

    /// Emojisi olmayan platformlarda (TikTok/Spotify) platform **adı** yazılır.
    #[test]
    fn platforms_without_an_emoji_use_their_name() {
        let mut post = BuiltinPost::new(
            Platform::TikTok,
            "https://www.tiktok.com/@alice/video/123",
            Author::new("alice", "alice"),
        );
        post.text = "tiktok gönderisi".to_string();
        post.extra_label = Some("3 yanıt".to_string());

        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));
        let meta = texts(&payload).last().unwrap().to_string();

        assert!(meta.contains("TikTok"), "platform adı: {meta}");
        assert!(meta.contains("3 yanıt"), "ek bilgi: {meta}");
        assert!(
            !meta.contains("twitter:"),
            "X emojisi kullanılmamalı: {meta}"
        );
    }

    #[test]
    fn translation_note_is_inline_code_next_to_the_timestamp() {
        let mut post = sample_post();
        post.translation = Some(Translation {
            source: Some("pt".to_string()),
        });
        post.created_at = Some(1_714_998_896);

        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));
        let meta = texts(&payload).last().unwrap().to_string();

        // Tek satır: platform emojisi en solda, saat ve hemen yanında **kod biçimli** çeviri
        // notu (`Portekizce'den çevrildi`). Blok alıntı kullanılmaz.
        assert!(
            meta.starts_with(
                "-# <:twitter:1476642012072920355> • <t:1714998896:R> • `Portekizce'den çevrildi`"
            ),
            "çeviri notu satır içi kod olarak saatin yanında olmalı: {meta}"
        );
        assert!(
            !meta.contains("\n> "),
            "çeviri notu blok alıntı olmamalı: {meta}"
        );
        assert!(
            !meta.contains("<:translation:"),
            "çeviri emojisi artık kullanılmamalı: {meta}"
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
            instagram: String::new(),
            reddit: String::new(),
            quote: String::new(),
        };
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));
        let lines = texts(&payload);

        assert_eq!(
            lines[0],
            "**Culture Base ☑ ([@Culture3ase](https://x.com/Culture3ase))**"
        );
        assert!(
            lines.iter().any(|t| t.contains("💬 6")),
            "metrik: {lines:?}"
        );
        assert!(
            lines.last().unwrap().contains("Twitter • <t:"),
            "platform adı: {:?}",
            lines.last()
        );

        // Emoji kapalıysa yalnızca istatistik alanları ile çeviri notu Unicode'a düşer
        post.translation = Some(Translation { source: None });
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));
        assert!(texts(&payload).last().unwrap().contains("Çevrildi"));
    }

    #[test]
    fn attached_video_plays_inside_the_gallery() {
        let mut post = sample_post();
        post.media = vec![video("https://cdn/v1.mp4", "https://cdn/poster.jpg")];

        let resolved = ResolvedMedia {
            attachments: vec![serenity::all::CreateAttachment::bytes(
                vec![1, 2, 3],
                "video.mp4",
            )],
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
        assert!(
            !meta.contains("Videoyu aç"),
            "video kart içindeyse link gerekmez: {meta}"
        );
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
        assert!(
            !meta.contains("Videoyu aç"),
            "video kart içinde oynadığı için ayrı açma bağlantısı eklenmez: {meta}"
        );
        assert!(
            !meta.contains("🎬"),
            "video açma işareti kullanılmamalı: {meta}"
        );
    }

    #[test]
    fn gallery_ignores_quote_media_when_post_has_media() {
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

        // Alıntılayan kişi medya eklediyse alıntının görseli galeriye dahil edilmez.
        assert_eq!(
            gallery_urls(&payload),
            vec!["attachment://photo.jpg", "https://cdn/second.jpg"]
        );

        let quote_text = texts(&payload)
            .into_iter()
            .find(|text| text.contains("<:qutoe:"))
            .expect("alıntı bileşeni");
        assert!(
            quote_text.contains("> alıntı"),
            "alıntı blok alıntı olmalı: {quote_text}"
        );
        assert!(
            quote_text.contains("[Quote](https://x.com/diger/status/2) from **Diğer** (@diger)"),
            "alıntı başlığı fxembed düzeninde olmalı: {quote_text}"
        );
        assert_eq!(
            quote_text.lines().next().unwrap(),
            "> <:qutoe:1553728214998847569> [Quote](https://x.com/diger/status/2) from **Diğer** (@diger)"
        );
    }

    #[test]
    fn quoted_image_is_shown_when_the_post_has_no_media() {
        let mut post = sample_post();
        let mut quote = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/diger/status/2",
            Author::new("Diğer", "diger"),
        );
        quote.text = "alıntı".to_string();
        quote.media = vec![image("https://cdn/quote.jpg")];
        post.quote = Some(Box::new(quote));

        let options = options();
        let plan = crate::converters::builtin::render::plan_media(&post, &options);
        let resolved = crate::converters::builtin::render::direct_media(&plan);
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &resolved, &emojis));

        assert_eq!(
            gallery_urls(&payload),
            vec!["https://cdn/quote.jpg"],
            "alıntılayan medya eklemediyse alıntının görseli galeride gösterilmeli"
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
            "> <:qutoe:1553728214998847569> [Quote](https://x.com/Daykerpriest/status/2103751786901144035) \
             from **Daykerpriest🐉** ([**@Daykerpriest**](https://x.com/daykerpriest))"
        );
        assert!(
            quote_text.contains("> alıntılanan metin"),
            "alıntılanan metin blok alıntı içinde olmalı: {quote_text}"
        );
    }

    /// Alıntı, kartın gövdesinden ince bir ayraç çizgisiyle ayrılır ve **tamamı** blok alıntı
    /// içinde durur: başlık ile alıntılanan metin aynı hizada başlar (X/fxembed düzeni),
    /// arada boş alıntı satırı olmadan, ek girinti/boşluk karakteri kullanılmadan.
    #[test]
    fn quote_block_is_quoted_and_separated_from_the_card_body() {
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
                    && component["content"]
                        .as_str()
                        .unwrap_or_default()
                        .contains("<:qutoe:")
            })
            .expect("alıntı bileşeni");

        let before = &children[quote_index - 1];
        assert_eq!(
            before["type"], TYPE_SEPARATOR,
            "alıntıdan önce ayraç olmalı"
        );
        assert_eq!(before["divider"], true, "ayraç ince çizgi çizmeli");
        assert_eq!(before["spacing"], 2, "ayraç büyük boşluk bırakmalı");

        let quote_text = children[quote_index]["content"].as_str().unwrap();
        let lines: Vec<&str> = quote_text.lines().collect();
        assert_eq!(
            lines.len(),
            3,
            "başlık + iki alıntı satırı bekleniyordu (arada boş `>` satırı olmamalı): {quote_text}"
        );
        assert!(
            lines[0].starts_with("> ") && lines[0].contains("[Quote]"),
            "başlık blok alıntının ilk satırı olmalı: {}",
            lines[0]
        );
        assert_eq!(lines[1], "> satır bir");
        assert_eq!(lines[2], "> satır iki");
        assert!(
            !lines.iter().any(|line| line.trim() == ">"),
            "boş alıntı satırı eklenmemeli: {quote_text}"
        );

        // Başlık ve alıntılanan metin aynı hizada: her satır blok alıntı olmalı ve girinti
        // için fazladan boşluk karakteri (em boşluğu / kırılmaz boşluk) kullanılmamalı.
        for line in &lines {
            assert!(
                line.starts_with('>'),
                "her satır blok alıntı olmalı: {line}"
            );
            assert!(
                !line.contains('\u{2003}') && !line.contains('\u{00A0}'),
                "girinti için boşluk karakteri kullanılmamalı: {line}"
            );
            assert!(
                !line.starts_with("-# "),
                "başlık alt metin olarak yazılmamalı: {line}"
            );
        }

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

    /// Kullanıcının gönderdiği ekran görüntüsündeki (X alıntı düzeni) çıktının **birebir**
    /// aynısı: başlık bloğun ilk satırı, ardından alıntı metni satır satır (boş satır yok).
    #[test]
    fn quote_block_matches_the_reference_x_layout() {
        let mut post = sample_post();
        post.text = "Neresi burası ya arkada cs açık tüm parayı basardık mekana".to_string();
        let mut quote = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/gerginimhelll/status/2104000000000000000",
            Author::new("durusal", "gerginimhelll"),
        );
        quote.text = "Durusal neşesi gerçek değil\nO sırada durusal neşesi".to_string();
        quote.author.profile_url = Some("https://x.com/gerginimhelll".to_string());
        post.quote = Some(Box::new(quote));

        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));
        let quote_text = texts(&payload)
            .into_iter()
            .find(|text| text.contains("<:qutoe:"))
            .expect("alıntı bileşeni");

        assert_eq!(
            quote_text,
            "> <:qutoe:1553728214998847569> [Quote](https://x.com/gerginimhelll/status/2104000000000000000) \
             from **durusal** ([**@gerginimhelll**](https://x.com/gerginimhelll))\n\
             > Durusal neşesi gerçek değil\n\
             > O sırada durusal neşesi"
        );
    }

    #[test]
    fn three_level_quote_chain_is_shown_when_the_post_has_no_media() {
        let mut post = sample_post();
        let mut inner = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/enic/status/3",
            Author::new("En İç", "enic"),
        );
        inner.text = "en içteki metin".to_string();
        let mut middle = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/orta/status/2",
            Author::new("Orta", "orta"),
        );
        middle.text = "ortadaki metin".to_string();
        middle.quote = Some(Box::new(inner));
        post.quote = Some(Box::new(middle));

        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));
        let body = texts(&payload).join("\n");

        assert!(body.contains("ortadaki metin"), "alıntı metni: {body}");
        assert!(
            body.contains("en içteki metin"),
            "medyasız zincirde alıntının alıntısı da gösterilir: {body}"
        );
        assert!(
            body.contains("https://x.com/enic/status/3"),
            "iç içe alıntının linki kartta olmalı: {body}"
        );
    }

    #[test]
    fn three_level_quote_with_bottom_media_uses_section_thumbnail() {
        let mut post = sample_post();
        let mut inner = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/enic/status/3",
            Author::new("En İç", "enic"),
        );
        inner.text = "en içteki metin".to_string();
        inner.media = vec![image("https://cdn/bottom.jpg")];

        let mut middle = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/orta/status/2",
            Author::new("Orta", "orta"),
        );
        middle.text = "ortadaki metin".to_string();
        middle.quote = Some(Box::new(inner));
        post.quote = Some(Box::new(middle));

        let options = options();
        let emojis = emojis();
        let resolved = ResolvedMedia {
            images: vec!["https://cdn/bottom.jpg".to_string()],
            ..ResolvedMedia::default()
        };
        let payload = build_payload(&card(&post, &options, &resolved, &emojis));
        let comps = components(&payload);

        // 3. seviyedeki alıntı bir Section (type 9) olarak oluşturulmalı ve Thumbnail (type 11) accessory taşımalı:
        let sec = comps
            .iter()
            .find(|c| c["type"] == TYPE_SECTION)
            .expect("en alttaki alıntı section olmalı");
        assert_eq!(sec["accessory"]["type"], TYPE_THUMBNAIL);
        assert_eq!(sec["accessory"]["media"]["url"], "https://cdn/bottom.jpg");

        // Alıntı metni Section'ın components dizisinde bulunmalı:
        let text_inside = sec["components"][0]["content"].as_str().unwrap();
        assert!(text_inside.contains("en içteki metin"));

        // Görsel thumbnail'a alındığı için ayrıca büyük media_gallery eklenmemeli:
        assert!(
            !comps.iter().any(|c| c["type"] == TYPE_MEDIA_GALLERY),
            "tek görsel thumbnail'a taşındığından ayrı media gallery olmamalı"
        );
    }

    #[test]
    fn quote_chain_stays_two_levels_when_the_post_has_media() {
        let mut post = sample_post();
        post.media = vec![image("https://cdn/1.jpg")];
        let mut inner = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/enic/status/3",
            Author::new("En İç", "enic"),
        );
        inner.text = "en içteki metin".to_string();
        let mut middle = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/orta/status/2",
            Author::new("Orta", "orta"),
        );
        middle.text = "ortadaki metin".to_string();
        middle.quote = Some(Box::new(inner));
        post.quote = Some(Box::new(middle));

        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));
        let body = texts(&payload).join("\n");

        assert!(body.contains("ortadaki metin"));
        assert!(
            !body.contains("en içteki metin"),
            "gönderide medya varsa kart medyaya odaklanır, zincir iki seviyede kalır: {body}"
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
        assert!(
            !meta.contains("Videoyu aç"),
            "alıntı videosu kartın içinde oynadığı için ayrı bağlantı eklenmez: {meta}"
        );
    }

    #[test]
    fn gif_in_direct_mode_plays_directly_in_media_gallery_without_video_button() {
        let mut post = sample_post();
        post.media = vec![crate::converters::builtin::Media {
            kind: crate::converters::builtin::MediaKind::Gif,
            url: "https://video.twimg.com/tweet_video/HTm_K6jaEAAMaNm.mp4".to_string(),
            poster: Some("https://pbs.twimg.com/tweet_video_thumb/HTm_K6jaEAAMaNm.jpg".to_string()),
            width: Some(1536),
            height: Some(1536),
            duration: Some(0),
        }];

        let options = RenderOptions { ..options() };
        let plan = crate::converters::builtin::render::plan_media(&post, &options);
        let resolved = crate::converters::builtin::render::direct_media(&plan);
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &resolved, &emojis));

        assert_eq!(
            gallery_urls(&payload),
            vec![
                "https://gifconvert.vxtwitter.com/convert.avif?url=https%3A%2F%2Fvideo.twimg.com%2Ftweet_video%2FHTm_K6jaEAAMaNm.mp4"
            ],
            "GIF, vxTwitter üzerinden AVIF formatında galeride gösterilir"
        );
        let meta = texts(&payload).pop().unwrap();
        assert!(
            !meta.contains("Videoyu aç"),
            "GIF için videoyu aç butonu çıkmamalı: {meta}"
        );
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
        assert!(
            meta.contains("||[Link](https://x.com/kullanici/status/1)||"),
            "spoiler link: {meta}"
        );
    }

    #[test]
    fn user_spoiler_hides_text_link_and_media() {
        let post = sample_post();
        let options = options();
        let emojis = emojis();
        let resolved = ResolvedMedia {
            images: vec!["attachment://photo.jpg".to_string()],
            ..ResolvedMedia::default()
        };

        let mut card = card(&post, &options, &resolved, &emojis);
        card.spoiler = true;
        let payload = build_payload(&card);

        assert_eq!(components(&payload)[1]["content"], "||merhaba dünya||");
        let gallery = components(&payload)
            .iter()
            .find(|component| component["type"] == TYPE_MEDIA_GALLERY)
            .expect("galeri");
        assert_eq!(
            gallery["items"][0]["spoiler"], true,
            "galeri öğesi de gizlenmeli"
        );

        let meta = texts(&payload).pop().unwrap();
        assert!(
            meta.contains("||[Link](https://x.com/kullanici/status/1)||"),
            "spoiler link: {meta}"
        );
    }

    #[test]
    fn sender_tag_is_shown_in_the_meta_line_only_for_bot_messages() {
        let post = sample_post();
        let options = options();
        let emojis = emojis();
        let resolved = ResolvedMedia::default();

        let mut bot_card = card(&post, &options, &resolved, &emojis);
        bot_card.sender = Some(UserId::new(42));
        bot_card.user_comment = Some("selam");
        let payload = build_payload(&bot_card);
        assert_eq!(
            top_level(&payload),
            vec!["selam".to_string()],
            "üstte yalnızca kullanıcı notu durur, etiket üstte gösterilmez"
        );
        let meta = texts(&payload).pop().unwrap();
        assert!(
            meta.trim_end().ends_with("Gönderen: <@42>"),
            "gönderen etiketi kartın sağ altında (meta satırı sonunda) durmalı: {meta}"
        );
        // Platform emojisi ve tarih de aynı satırdadır.
        assert!(
            meta.contains("<:twitter:1476642012072920355>"),
            "platform: {meta}"
        );
        assert!(meta.contains("<t:1714998896:R>"), "tarih: {meta}");

        // Taklit (webhook) açıkken mesaj kullanıcının adıyla gider: etiket eklenmez.
        let identity = Impersonation {
            username: "Eray".to_string(),
            avatar_url: None,
        };
        let mut webhook_card = card(&post, &options, &resolved, &emojis);
        webhook_card.impersonation = Some(&identity);
        let payload = build_payload(&webhook_card);
        assert!(
            top_level(&payload).is_empty(),
            "taklitte not/etiket olmamalı"
        );
        assert!(
            !texts(&payload).pop().unwrap().contains("Gönderen:"),
            "taklitte gönderen etiketi eklenmez"
        );
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
        assert!(body.chars().count() <= TEXT_DISPLAY_LIMIT);
        let total: usize = top_level(&payload)
            .iter()
            .chain(texts(&payload).iter())
            .map(|s| s.chars().count())
            .sum();
        assert!(total <= CUMULATIVE_TEXT_LIMIT);
    }

    #[test]
    fn container_children_never_exceeds_discord_limit_of_ten() {
        let mut post = sample_post();
        let mut inner = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/enic/status/3",
            Author::new("En İç", "enic"),
        );
        inner.text = "iç".to_string();
        let mut middle = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/orta/status/2",
            Author::new("Orta", "orta"),
        );
        middle.text = "orta".to_string();
        middle.quote = Some(Box::new(inner));
        post.quote = Some(Box::new(middle));
        post.community_note = Some("Topluluk Notu İçeriği".to_string());
        post.media = vec![image("https://cdn/1.jpg")];

        let options = options();
        let emojis = emojis();
        let payload = build_payload(&card(&post, &options, &ResolvedMedia::default(), &emojis));
        let children = components(&payload);
        assert!(children.len() <= MAX_CONTAINER_CHILDREN);
        assert_eq!(MAX_CONTAINER_CHILDREN, 10);
    }

    #[test]
    fn cumulative_text_across_components_never_exceeds_four_thousand() {
        let mut post = sample_post();
        post.text = "p".repeat(3000);
        let options = options();
        let emojis = emojis();
        let resolved = ResolvedMedia::default();
        let mut card_data = card(&post, &options, &resolved, &emojis);
        let comment = "c".repeat(2000);
        card_data.user_comment = Some(&comment);
        let payload = build_payload(&card_data);

        let total: usize = top_level(&payload)
            .iter()
            .chain(texts(&payload).iter())
            .map(|s| s.chars().count())
            .sum();
        assert!(total <= CUMULATIVE_TEXT_LIMIT);
    }

    #[test]
    fn link_preview_is_shown_below_the_text_with_its_image_in_the_gallery() {
        let mut post = sample_post();
        post.link_preview = Some(LinkPreview {
            url: "https://example.com/haber".to_string(),
            title: Some("Haber başlığı".to_string()),
            description: Some("İlk satır\n\nİkinci satır".to_string()),
            image: Some("https://cdn.example.com/onizleme.jpg".to_string()),
            site_name: Some("example.com".to_string()),
        });
        let options = options();
        let emojis = emojis();
        let resolved = ResolvedMedia {
            images: vec!["https://cdn.example.com/onizleme.jpg".to_string()],
            ..ResolvedMedia::default()
        };
        let payload = build_payload(&card(&post, &options, &resolved, &emojis));

        let texts = texts(&payload);
        let preview = texts
            .iter()
            .find(|text| text.starts_with("**🔗"))
            .expect("link kartı bloğu kartta durmalı");
        assert_eq!(
            preview,
            "**🔗 [Haber başlığı](https://example.com/haber)**\n> İlk satır\n> İkinci satır\n-# example.com",
            "başlık bağlantılı, açıklama alıntı bloğunda, site adı alt bilgide"
        );
        assert!(
            texts.iter().position(|text| text == preview)
                > texts.iter().position(|text| text == "merhaba dünya"),
            "link kartı gövde metninin altında durur"
        );
        assert_eq!(
            gallery_urls(&payload),
            vec!["https://cdn.example.com/onizleme.jpg".to_string()],
            "önizleme görseli kartın galerisinde gösterilir"
        );
    }

    #[test]
    fn empty_link_preview_adds_nothing_to_the_card() {
        let mut post = sample_post();
        post.link_preview = Some(LinkPreview {
            url: "https://example.com/haber".to_string(),
            ..LinkPreview::default()
        });
        let options = options();
        let emojis = emojis();
        let resolved = ResolvedMedia::default();
        let payload = build_payload(&card(&post, &options, &resolved, &emojis));

        assert!(
            !texts(&payload).iter().any(|text| text.contains("🔗")),
            "gösterilecek veri yoksa link kartı eklenmez"
        );
    }

    /// Uzun gizli metin kısaltılınca kapanış `||` korunmalı; aksi hâlde metin açığa çıkar.
    #[test]
    fn truncating_hidden_text_keeps_the_spoiler_closed() {
        let wrapped = spoiler_wrap(&"a".repeat(TEXT_DISPLAY_LIMIT * 2), true);
        let component = text(wrapped);
        let content = component["content"].as_str().unwrap();
        assert!(content.starts_with("||") && content.ends_with("||"));
        assert!(content.chars().count() <= TEXT_DISPLAY_LIMIT);

        let short = truncate_display(&spoiler_wrap(&"b".repeat(50), true), 10);
        assert!(short.starts_with("||") && short.ends_with("||"));
        assert!(short.chars().count() <= 10);
    }

    /// Gizli metnin içindeki `||` gizlemeyi erken kapatmamalı.
    #[test]
    fn inner_spoiler_bars_cannot_close_the_spoiler_early() {
        let wrapped = spoiler_wrap("a || gizli", true);
        assert_eq!(wrapped.matches("||").count(), 2);
    }
}
