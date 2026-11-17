//! Bluesky (AT Protocol) gönderi verisi.
//!
//! Resmî AppView'ın herkese açık XRPC uç noktaları kullanılır (`public.api.bsky.app`):
//! önce handle → DID çözümlenir, sonra gönderi ipliği `getPostThread` ile çekilir.

use regex::Regex;
use serde::Deserialize;
use std::sync::LazyLock;

use super::{Author, BuiltinPost, LinkPreview, Media, MediaKind, Stats, parse_timestamp};
use crate::converters::parser::{ParsedPost, Platform};

const API_BASE: &str = "https://public.api.bsky.app/xrpc";
const THREAD_DEPTH: u8 = 2;
const THREAD_PARENT_HEIGHT: u8 = 1;
/// İplik konumu (`🧵 2/4`) için yazar ipliğinin aşağıda en çok kaç gönderisi okunur.
const OP_THREAD_BELOW: u8 = 20;
/// İplik konumu isteğinin süresi. Kart gönderildikten sonra çalışır; gelmezse kart konumsuz kalır.
const OP_THREAD_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(4_000);

static BSKY_URL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"bsky\.app/profile/([^/\s]+)/post/([A-Za-z0-9]+)").expect("geçerli bluesky regex")
});

/// Gönderi verisini Bluesky AppView'dan çeker.
pub async fn fetch(client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    let captures = BSKY_URL_REGEX
        .captures(&parsed.original_url)
        .ok_or_else(|| {
            format!(
                "Bluesky gönderi adresi çözümlenemedi: {}",
                parsed.original_url
            )
        })?;
    let handle = captures
        .get(1)
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| format!("Bluesky kullanıcı adı bulunamadı: {}", parsed.original_url))?;
    let rkey = captures
        .get(2)
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| {
            format!(
                "Bluesky gönderi kimliği bulunamadı: {}",
                parsed.original_url
            )
        })?;

    // AppView at-URI'deki handle'ı kendisi çözer: ayrı bir `getProfile` (handle → DID) isteği
    // gönderiyi bir tur gidiş-dönüş geciktirirdi. DID verilmişse aynı biçim onunla da çalışır.
    let uri = format!("at://{handle}/app.bsky.feed.post/{rkey}");
    let url = format!(
        "{API_BASE}/app.bsky.feed.getPostThread?depth={THREAD_DEPTH}&parentHeight={THREAD_PARENT_HEIGHT}&uri={}",
        urlencoding::encode(&uri)
    );
    let value: serde_json::Value = super::get_json(client, &url).await?;

    let response: ThreadResponse = serde_json::from_value(value)
        .map_err(|e| format!("Bluesky yanıtı çözümlenemedi ({rkey}): {e}"))?;

    response.thread.post.as_ref().ok_or_else(|| {
        format!("Bluesky gönderisi bulunamadı ({rkey}); silinmiş veya engellenmiş olabilir")
    })?;

    let mut post = thread_view_to_post(&response.thread, &handle);
    // Konum kart gönderildikten sonra çözülür (`enrich`): iplik isteği kartı geciktirmesin.
    post.thread_lookup = in_own_thread(&response.thread, &post);
    Ok(post)
}

/// Kart gönderildikten sonra iplik konumunu çözer; gönderi iplikte değilse `None`.
pub async fn enrich(client: &reqwest::Client, post: &BuiltinPost) -> Option<BuiltinPost> {
    if !post.thread_lookup {
        return None;
    }
    let captures = BSKY_URL_REGEX.captures(&post.url)?;
    let uri = format!("at://{}/app.bsky.feed.post/{}", &captures[1], &captures[2]);
    let mut enriched = post.clone();
    enriched.thread_lookup = false;
    enriched.thread_position = op_thread_position(client, &uri).await;
    Some(enriched)
}

/// Gönderi yazarın kendi ipliğinde mi: üst gönderi ya da doğrudan bir yanıt aynı yazardan.
fn in_own_thread(thread: &ThreadView, post: &BuiltinPost) -> bool {
    let author = post.author.handle.trim();
    let same = |handle: &str| !author.is_empty() && handle.eq_ignore_ascii_case(author);
    post.reply_to
        .as_deref()
        .is_some_and(|parent| same(&parent.author.handle))
        || thread.replies.iter().flatten().any(|reply| {
            reply
                .post
                .as_ref()
                .and_then(|post| post.author.as_ref())
                .and_then(|author| author.handle.as_deref())
                .is_some_and(same)
        })
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ThreadV2 {
    thread: Vec<ThreadV2Item>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ThreadV2Item {
    depth: i32,
    value: ThreadV2Value,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ThreadV2Value {
    #[serde(rename = "opThread")]
    op_thread: bool,
}

/// `getPostThreadV2` yazarın ipliğindeki gönderileri `opThread` ile işaretler: üsttekiler ve
/// gönderinin kendisi konumu, hepsi toplamı verir.
async fn op_thread_position(client: &reqwest::Client, uri: &str) -> Option<(u32, u32)> {
    let url = format!(
        "{API_BASE}/app.bsky.unspecced.getPostThreadV2?below={OP_THREAD_BELOW}&branchingFactor=1&anchor={}",
        urlencoding::encode(uri)
    );
    match tokio::time::timeout(OP_THREAD_TIMEOUT, super::get_json::<ThreadV2>(client, &url)).await {
        Ok(Ok(thread)) => op_position(&thread),
        Ok(Err(err)) => {
            tracing::debug!("Bluesky iplik konumu alınamadı: {err}");
            None
        }
        Err(_) => {
            tracing::debug!("Bluesky iplik konumu zamanında gelmedi");
            None
        }
    }
}

fn op_position(thread: &ThreadV2) -> Option<(u32, u32)> {
    let op: Vec<&ThreadV2Item> = thread
        .thread
        .iter()
        .filter(|item| item.value.op_thread)
        .collect();
    if !op.iter().any(|item| item.depth == 0) || op.len() < 2 {
        return None;
    }
    let index = op.iter().filter(|item| item.depth <= 0).count();
    Some((index as u32, op.len() as u32))
}

// =====================================================================================
// DÖNÜŞTÜRME
// =====================================================================================

fn post_view_to_post(post: &PostView, fallback_handle: &str) -> BuiltinPost {
    let profile = post.author.as_ref();
    let handle = profile
        .and_then(|p| p.handle.clone())
        .unwrap_or_else(|| fallback_handle.to_string());

    let mut author = Author::new(
        profile
            .and_then(|p| p.display_name.clone())
            .unwrap_or_else(|| handle.clone()),
        handle.clone(),
    );
    author.avatar = profile.and_then(|p| p.avatar.clone());
    author.verified = profile
        .and_then(|p| p.verification.as_ref())
        .and_then(|v| v.verified_status.as_deref())
        == Some("valid");
    author.profile_url = Some(format!("https://bsky.app/profile/{handle}"));

    let mut built = BuiltinPost::new(Platform::Bluesky, web_url(post, &handle), author);

    let record = post.record.as_ref();
    let text = record.and_then(|r| r.text.clone()).unwrap_or_default();
    built.text = apply_link_facets(&text, record.and_then(|r| r.facets.as_deref()));
    built.created_at = record
        .and_then(|r| r.created_at.as_deref())
        .and_then(parse_timestamp)
        .or_else(|| post.indexed_at.as_deref().and_then(parse_timestamp));
    built.stats = Stats {
        replies: post.reply_count.unwrap_or(0),
        reposts: post.repost_count.unwrap_or(0),
        likes: post.like_count.unwrap_or(0),
        quotes: post.quote_count.unwrap_or(0),
        views: None,
    };
    built.sensitive = has_adult_label(post.labels.as_deref())
        || has_adult_label(profile.and_then(|p| p.labels.as_deref()));

    if let Some(embed) = post.embed.as_ref() {
        let parts = embed_to_parts(embed);
        built.media = parts.media;

        // Bağlantı kartı X'teki gibi başlık + açıklama + site adıyla çizilir; küçük görseli gönderinin
        // kendi medyası yoksa galeriye girer (`plan_media`).
        if let Some(card) = parts.external.as_ref()
            && let Some(uri) = card.uri.as_deref().filter(|uri| uri.starts_with("http"))
        {
            let clean = |value: &Option<String>| {
                value
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            };
            built.link_preview = Some(LinkPreview {
                url: uri.to_string(),
                title: clean(&card.title),
                description: clean(&card.description),
                image: clean(&card.thumb),
                site_name: url::Url::parse(uri).ok().and_then(|url| {
                    url.host_str()
                        .map(|host| host.trim_start_matches("www.").to_string())
                }),
            });
        }

        built.quote = parts.quote.map(Box::new);
    }

    built
}

/// Converts the requested post plus the single bounded parent/repost relation returned by
/// AppView. Replies are represented as a compact label because the shared model has one quote
/// slot and no thread collection.
fn thread_view_to_post(thread: &ThreadView, fallback_handle: &str) -> BuiltinPost {
    let Some(post) = thread.post.as_ref() else {
        return BuiltinPost::new(
            Platform::Bluesky,
            format!("https://bsky.app/profile/{fallback_handle}"),
            Author::new(fallback_handle, fallback_handle),
        );
    };
    let mut built = post_view_to_post(post, fallback_handle);

    // Yanıtlanan gönderi alıntı değildir: ayrı `reply_to` alanında taşınır, böylece gönderinin kendi
    // alıntısı korunur ve kart üst gönderiyi bağlam olarak gösterir.
    if let Some(parent) = thread
        .parent
        .as_deref()
        .and_then(|parent| parent.post.as_ref())
    {
        built.reply_to = quote_from_post_view(parent).map(Box::new);
    } else if post
        .record
        .as_ref()
        .and_then(|record| record.reply.as_ref())
        .is_some()
    {
        built.extra_label = Some("Yanıt".to_string());
    }

    // Sınırlı derinlikte çekilen iplik yanıtların yalnızca bir kısmını döndürür; gerçek sayı `replyCount`
    // alanındadır. Sayı metrik satırında gösterilir; alt bilgide ayrıca `+N yanıt` yazılmaz.
    if post.reply_count.is_none() {
        built.stats.replies = thread
            .replies
            .as_ref()
            .map_or(0, |replies| replies.len() as u64);
    }

    if let Some(reason) = thread.reason.as_ref().or(post.reason.as_ref())
        && reason
            .kind
            .as_deref()
            .is_some_and(|kind| kind.contains("reasonRepost"))
    {
        let by = reason
            .by
            .as_ref()
            .and_then(|profile| profile.handle.as_deref())
            .map(|handle| format!("@{handle}"))
            .unwrap_or_else(|| "başka kullanıcı".to_string());
        let repost_label = format!("Repost • {by}");
        built.extra_label = Some(if thread.parent.is_some() {
            match built.extra_label.take() {
                Some(label) => format!("{repost_label} • {label}"),
                None => repost_label,
            }
        } else {
            // A repost reason is authoritative when AppView does not provide a parent.
            // The record may still carry a reply reference from the original post; do not
            // present that stale relation as if the repost itself were a reply.
            repost_label
        });
    }

    built
}

/// `at://…` gönderi URI'sinden paylaşılabilir web adresi üretir.
fn web_url(post: &PostView, handle: &str) -> String {
    let rkey = post
        .uri
        .as_deref()
        .and_then(|uri| uri.rsplit('/').next())
        .unwrap_or_default();
    if rkey.is_empty() {
        format!("https://bsky.app/profile/{handle}")
    } else {
        format!("https://bsky.app/profile/{handle}/post/{rkey}")
    }
}

/// Facet verisindeki bağlantıları markdown bağlantısına çevirir (byte aralıkları sondan
/// başa işlenir; böylece önceki aralıkların offset'leri bozulmaz).
fn apply_link_facets(text: &str, facets: Option<&[Facet]>) -> String {
    let Some(facets) = facets else {
        return text.to_string();
    };

    let mut ranges: Vec<(usize, usize, String)> = Vec::new();
    for facet in facets {
        let index = match facet.index.as_ref() {
            Some(index) => index,
            None => continue,
        };
        let (start, end) = match (index.byte_start, index.byte_end) {
            (Some(start), Some(end)) => (start, end),
            _ => continue,
        };
        let uri = facet
            .features
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|feature| {
                // "#link" dışındaki türler (mention/tag) atlanır; tip bilgisi yoksa
                // uri alanının varlığı belirleyici olur.
                feature
                    .kind
                    .as_deref()
                    .is_none_or(|kind| kind.ends_with("#link"))
            })
            .and_then(|feature| feature.uri.clone());

        if let Some(uri) = uri
            && start < end
            && end <= text.len()
            && text.is_char_boundary(start)
            && text.is_char_boundary(end)
        {
            ranges.push((start, end, uri));
        }
    }

    ranges.sort_by_key(|range| std::cmp::Reverse((range.0, range.1)));

    // Aralıklar özgün metne göre doğrulandığından yeniden yazılmış metne yalnızca daha önce uygulanmış aralıkla çakışmayanlar dokunabilir; yinelenen ya da iç içe facet'ler çok baytlı metinde karakter sınırı dışında dilimlemeye yol açar.
    let mut out = text.to_string();
    let mut applied_start = usize::MAX;
    for (start, end, uri) in ranges {
        if end > applied_start {
            continue;
        }
        applied_start = start;
        let label = out[start..end].replace(']', "\\]");
        let uri = uri.replace(['<', '>'], "");
        out.replace_range(start..end, &format!("[{label}](<{uri}>)"));
    }
    out
}

/// Bluesky'nin yetişkin içerik etiketleri.
fn has_adult_label(labels: Option<&[Label]>) -> bool {
    labels.unwrap_or_default().iter().any(|label| {
        matches!(
            label.val.as_deref(),
            Some("porn") | Some("sexual") | Some("nudity") | Some("graphic-media")
        )
    })
}

/// Embed verisinin ayrıştırılmış hâli.
#[derive(Debug, Default)]
struct EmbedParts {
    media: Vec<Media>,
    external: Option<EmbedExternal>,
    quote: Option<BuiltinPost>,
}

/// `post.embed` görünümünü medya / bağlantı kartı / alıntı parçalarına ayırır.
fn embed_to_parts(embed: &EmbedView) -> EmbedParts {
    let mut parts = EmbedParts::default();
    let kind = embed.kind.as_deref().unwrap_or_default();

    // Kayıt (alıntı) embed'i API'de "#record#view" / "#recordWithMedia#view" olarak gelir;
    // alan mevcudiyeti de yedek sinyal olarak kullanılır.
    let record_with_media =
        kind.contains("recordWithMedia") || (embed.record.is_some() && embed.media.is_some());
    let record_only = !record_with_media && (kind.contains("record") || embed.record.is_some());

    if record_with_media {
        if let Some(media) = embed.media.as_deref() {
            let nested = embed_to_parts(media);
            parts.media.extend(nested.media);
            parts.external = nested.external;
        }
        parts.quote = embed.record.as_ref().and_then(quote_from_embed_record);
        return parts;
    }

    if record_only {
        parts.quote = embed.record.as_ref().and_then(quote_from_embed_record);
        if parts.quote.is_none() {
            parts.external = embed.record.as_ref().and_then(graph_record_card);
        }
        return parts;
    }

    if let Some(external) = embed.external.as_ref() {
        parts.external = Some(external.clone());
    }

    for images in [embed.images.as_deref(), embed.items.as_deref()]
        .into_iter()
        .flatten()
    {
        for image in images {
            if let Some(url) = image.fullsize.as_deref() {
                let (width, height) = aspect_dimensions(image.aspect_ratio.as_ref());
                parts.media.push(Media {
                    alt: image.alt.clone(),
                    kind: MediaKind::Image,
                    url: url.to_string(),
                    poster: None,
                    width,
                    height,
                    duration: None,
                });
            }
        }
    }

    let video = &embed.video;
    if let Some(url) = video.playlist.as_deref().and_then(video_blob_url) {
        let (width, height) = aspect_dimensions(video.aspect_ratio.as_ref());
        parts.media.push(Media {
            alt: video.alt.clone(),
            kind: MediaKind::Video,
            url,
            poster: video.thumbnail.clone(),
            width,
            height,
            duration: None,
        });
    }

    parts
}

/// Bluesky videoyu yalnızca HLS (`.m3u8`) olarak sunar; Discord bunu oynatamaz. Oynatma listesi
/// adresindeki DID ve CID'den özgün MP4'ün blob adresi kurulur (FxBluesky/xsky.app yöntemi):
/// `bsky.social` isteği kullanıcının PDS'ine yönlendirir ve `video/mp4` döner.
fn video_blob_url(playlist: &str) -> Option<String> {
    let path = playlist.split("/watch/").nth(1)?;
    let mut parts = path.split('/');
    let did = urlencoding::decode(parts.next()?).ok()?;
    let cid = parts.next()?;
    let valid = |value: &str| {
        !value.is_empty()
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'.' | b'_' | b'-')
            })
    };
    (did.starts_with("did:") && valid(&did) && valid(cid))
        .then(|| format!("https://bsky.social/xrpc/com.atproto.sync.getBlob?did={did}&cid={cid}"))
}

fn aspect_dimensions(ratio: Option<&AspectRatio>) -> (Option<u32>, Option<u32>) {
    match ratio {
        Some(ratio) => (ratio.width, ratio.height),
        None => (None, None),
    }
}

/// Embed'in `record` alanını alıntıya çevirir.
///
/// `record#view` içinde bu alan doğrudan `viewRecord`'dur; `recordWithMedia#view` içinde ise `{ "record": viewRecord }` biçiminde bir `record#view` sarmalayıcısıdır.
fn quote_from_embed_record(value: &serde_json::Value) -> Option<BuiltinPost> {
    let view_record = value
        .get("record")
        .filter(|inner| inner.is_object())
        .unwrap_or(value);
    let view = PostView::deserialize(view_record).ok()?;
    quote_from_post_view(&view)
}

/// Gönderi olmayan gömülü kayıt (başlangıç paketi, akış, liste) bağlantı kartı olarak gösterilir.
fn graph_record_card(value: &serde_json::Value) -> Option<EmbedExternal> {
    let text = |value: Option<&serde_json::Value>| {
        value
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let uri = value.get("uri")?.as_str()?;
    let mut segments = uri.strip_prefix("at://")?.split('/');
    let (owner, collection, rkey) = (segments.next()?, segments.next()?, segments.next()?);
    let creator = text(value.pointer("/creator/handle")).unwrap_or_else(|| owner.to_string());
    let (path, label, name, description) = match collection {
        "app.bsky.graph.starterpack" => (
            format!("starter-pack/{creator}/{rkey}"),
            "Başlangıç paketi",
            text(value.pointer("/record/name")),
            text(value.pointer("/record/description")),
        ),
        "app.bsky.feed.generator" => (
            format!("profile/{creator}/feed/{rkey}"),
            "Akış",
            text(value.get("displayName")),
            text(value.get("description")),
        ),
        "app.bsky.graph.list" => (
            format!("profile/{creator}/lists/{rkey}"),
            "Liste",
            text(value.get("name")),
            text(value.get("description")),
        ),
        _ => return None,
    };
    let safe = |part: &str| {
        !part.is_empty()
            && part.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':')
            })
    };
    if !safe(&creator) || !safe(rkey) {
        return None;
    }
    Some(EmbedExternal {
        uri: Some(format!("https://bsky.app/{path}")),
        title: Some(match name {
            Some(name) => format!("{label}: {name}"),
            None => label.to_string(),
        }),
        description,
        thumb: text(value.get("avatar")),
    })
}

/// Alıntılanan gönderiyi (embed kaydı) gönderi modeline çevirir.
fn quote_from_post_view(view: &PostView) -> Option<BuiltinPost> {
    let record = view.record.as_ref().or(view.value.as_ref())?;
    let text = record.text.clone().unwrap_or_default();

    // `viewRecord` medyayı `embeds` dizisinde, `postView` ise tekil `embed` alanında taşır.
    let media: Vec<Media> = view
        .embed
        .iter()
        .chain(view.embeds.iter().flatten())
        .flat_map(|embed| embed_to_parts(embed).media)
        .collect();
    if text.trim().is_empty() && media.is_empty() {
        return None;
    }

    let handle = view
        .author
        .as_ref()
        .and_then(|author| author.handle.clone())
        .unwrap_or_default();

    let mut author = Author::new(
        view.author
            .as_ref()
            .and_then(|author| author.display_name.clone())
            .unwrap_or_else(|| handle.clone()),
        handle.clone(),
    );
    author.avatar = view
        .author
        .as_ref()
        .and_then(|author| author.avatar.clone());
    author.verified = view
        .author
        .as_ref()
        .and_then(|author| author.verification.as_ref())
        .and_then(|verification| verification.verified_status.as_deref())
        == Some("valid");

    author.profile_url = (!handle.is_empty()).then(|| format!("https://bsky.app/profile/{handle}"));
    let mut quote = BuiltinPost::new(Platform::Bluesky, web_url(view, &handle), author);
    quote.text = apply_link_facets(&text, record.facets.as_deref());
    quote.media = media;

    Some(quote)
}

// =====================================================================================
// JSON MODELİ
// =====================================================================================

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ThreadResponse {
    thread: ThreadView,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ThreadView {
    #[serde(rename = "type", alias = "$type")]
    kind: Option<String>,
    post: Option<PostView>,
    parent: Option<Box<ThreadView>>,
    replies: Option<Vec<ThreadView>>,
    reason: Option<RepostReason>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PostView {
    uri: Option<String>,
    author: Option<ProfileView>,
    record: Option<PostRecord>,
    /// Alıntı embed'lerinde içerik `value` alanında gelir (ham kayıt).
    value: Option<PostRecord>,
    embed: Option<EmbedView>,
    /// Alıntı kayıtlarında (`viewRecord`) gömülü içerik dizi olarak gelir.
    embeds: Option<Vec<EmbedView>>,
    labels: Option<Vec<Label>>,
    #[serde(rename = "replyCount")]
    reply_count: Option<u64>,
    #[serde(rename = "repostCount")]
    repost_count: Option<u64>,
    #[serde(rename = "likeCount")]
    like_count: Option<u64>,
    #[serde(rename = "quoteCount")]
    quote_count: Option<u64>,
    #[serde(rename = "indexedAt")]
    indexed_at: Option<String>,
    reason: Option<RepostReason>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ProfileView {
    handle: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    avatar: Option<String>,
    verification: Option<Verification>,
    labels: Option<Vec<Label>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Verification {
    #[serde(rename = "verifiedStatus")]
    verified_status: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Label {
    val: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PostRecord {
    text: Option<String>,
    #[serde(rename = "createdAt")]
    created_at: Option<String>,
    facets: Option<Vec<Facet>>,
    reply: Option<ReplyRef>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ReplyRef {
    root: Option<ReplyTarget>,
    parent: Option<ReplyTarget>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ReplyTarget {
    uri: Option<String>,
    cid: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RepostReason {
    #[serde(rename = "$type", alias = "type")]
    kind: Option<String>,
    by: Option<ProfileView>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Facet {
    index: Option<FacetIndex>,
    features: Option<Vec<FacetFeature>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FacetIndex {
    #[serde(rename = "byteStart")]
    byte_start: Option<usize>,
    #[serde(rename = "byteEnd")]
    byte_end: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FacetFeature {
    #[serde(rename = "type", alias = "$type")]
    kind: Option<String>,
    uri: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct EmbedView {
    #[serde(rename = "type", alias = "$type")]
    kind: Option<String>,
    images: Option<Vec<EmbedImage>>,
    /// `gallery#view` görselleri (`images` ile aynı biçim).
    items: Option<Vec<EmbedImage>>,
    /// `video#view` alanları gömmenin kendisinde durur (iç içe `video` nesnesi yoktur).
    #[serde(flatten)]
    video: EmbedVideo,
    external: Option<EmbedExternal>,
    /// Embed türüne göre `viewRecord` ya da `record#view` sarmalayıcısı olduğundan ham tutulur.
    record: Option<serde_json::Value>,
    media: Option<Box<EmbedView>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct EmbedImage {
    fullsize: Option<String>,
    alt: Option<String>,
    #[serde(rename = "aspectRatio")]
    aspect_ratio: Option<AspectRatio>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct EmbedVideo {
    playlist: Option<String>,
    alt: Option<String>,
    thumbnail: Option<String>,
    #[serde(rename = "aspectRatio")]
    aspect_ratio: Option<AspectRatio>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct EmbedExternal {
    uri: Option<String>,
    title: Option<String>,
    description: Option<String>,
    thumb: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AspectRatio {
    width: Option<u32>,
    height: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread_from(json: &str) -> PostView {
        let response: ThreadResponse =
            serde_json::from_str(json).expect("örnek JSON geçerli olmalı");
        response.thread.post.expect("gönderi bulunmalı")
    }

    #[test]
    fn basic_post_with_facets_is_converted() {
        let json = r#"{
            "thread": {
                "type": "app.bsky.feed.defs#threadViewPost",
                "post": {
                    "uri": "at://did:plc:z72i7hdynmk6r22z27h6tvur/app.bsky.feed.post/3mw2cdr44fc2a",
                    "author": {
                        "handle": "bsky.app",
                        "displayName": "Bluesky",
                        "avatar": "https://cdn.bsky.app/img/avatar/x.jpg",
                        "verification": { "verifiedStatus": "valid" }
                    },
                    "record": {
                        "text": "Oylamaya bakın: https://example.com/oy 🌟",
                        "createdAt": "2026-09-21T18:04:06.128Z",
                        "facets": [{
                            "index": { "byteStart": 17, "byteEnd": 39 },
                            "features": [{ "$type": "app.bsky.richtext.facet#link", "uri": "https://example.com/oy" }]
                        }]
                    },
                    "replyCount": 87,
                    "repostCount": 84,
                    "likeCount": 712,
                    "quoteCount": 45,
                    "indexedAt": "2026-09-21T18:04:06.866Z"
                }
            }
        }"#;

        let post = post_view_to_post(&thread_from(json), "bsky.app");

        assert_eq!(post.platform, Platform::Bluesky);
        assert_eq!(
            post.url,
            "https://bsky.app/profile/bsky.app/post/3mw2cdr44fc2a"
        );
        assert_eq!(post.author.label(), "Bluesky (@bsky.app)");
        assert!(post.author.verified);
        assert!(
            post.text
                .contains("[https://example.com/oy](<https://example.com/oy>)")
        );
        assert_eq!(post.stats.likes, 712);
        assert_eq!(post.stats.reposts, 84);
        assert_eq!(post.stats.replies, 87);
        assert_eq!(post.stats.quotes, 45);
        assert_eq!(post.created_at, Some(1790013846));
        assert!(!post.sensitive);
        assert!(post.media.is_empty());
    }

    #[test]
    fn images_and_labels_are_extracted() {
        let json = r#"{
            "thread": { "post": {
                "uri": "at://did:plc:abc/app.bsky.feed.post/rkey1",
                "author": { "handle": "foto.bsky.social", "displayName": "Foto" },
                "record": { "text": "Manzaralar", "createdAt": "2024-05-06T12:34:56Z" },
                "labels": [{ "val": "graphic-media" }],
                "embed": {
                    "type": "app.bsky.embed.images#view",
                    "images": [
                        { "fullsize": "https://cdn.bsky.app/img/1.jpg", "aspectRatio": { "width": 1200, "height": 800 } },
                        { "fullsize": "https://cdn.bsky.app/img/2.jpg" }
                    ]
                }
            }}}
        "#;

        let post = post_view_to_post(&thread_from(json), "foto.bsky.social");
        assert!(post.sensitive, "yetişkin etiketi spoiler yapmalı");
        assert_eq!(post.media.len(), 2);
        assert_eq!(post.media[0].url, "https://cdn.bsky.app/img/1.jpg");
        assert_eq!(post.media[0].width, Some(1200));
        assert_eq!(post.media[1].width, None);
    }

    #[test]
    fn external_card_becomes_a_link_preview() {
        let json = r#"{
            "thread": { "post": {
                "uri": "at://did:plc:abc/app.bsky.feed.post/rkey2",
                "author": { "handle": "haber.bsky.social" },
                "record": { "text": "", "createdAt": "2024-05-06T12:34:56Z" },
                "embed": { "type": "app.bsky.embed.external#view", "external": {
                    "uri": "https://example.com/haber",
                    "title": "Önemli haber",
                    "description": "Haberin özeti",
                    "thumb": "https://cdn.bsky.app/thumb.jpg"
                } }
            }}}
        "#;

        let post = post_view_to_post(&thread_from(json), "haber.bsky.social");
        let preview = post
            .link_preview
            .expect("bağlantı kartı önizlemeye dönüşmeli");
        assert_eq!(preview.url, "https://example.com/haber");
        assert_eq!(preview.title.as_deref(), Some("Önemli haber"));
        assert_eq!(preview.description.as_deref(), Some("Haberin özeti"));
        assert_eq!(
            preview.image.as_deref(),
            Some("https://cdn.bsky.app/thumb.jpg")
        );
        assert_eq!(preview.site_name.as_deref(), Some("example.com"));
        // Açıklama gövdeye kopyalanmaz, küçük görsel medya olmaz: ikisi de önizlemede bir kez görünür.
        assert!(post.text.is_empty());
        assert!(post.media.is_empty());
    }

    #[test]
    fn quoted_post_is_added_as_quote() {
        let json = r#"{
            "thread": { "post": {
                "uri": "at://did:plc:abc/app.bsky.feed.post/main",
                "author": { "handle": "ana.bsky.social", "displayName": "Ana" },
                "record": { "text": "Buna bakın", "createdAt": "2024-05-06T12:34:56Z" },
                "embed": { "type": "app.bsky.embed.record#view", "record": {
                    "uri": "at://did:plc:def/app.bsky.feed.post/alinti",
                    "author": { "handle": "alintilanan.bsky.social", "displayName": "Alıntı" },
                    "value": { "text": "alıntılanan metin", "createdAt": "2024-05-05T10:00:00Z" }
                } }
            }}}
        "#;

        let post = post_view_to_post(&thread_from(json), "ana.bsky.social");
        let quote = post.quote.expect("alıntı eklenmeli");
        assert_eq!(quote.text, "alıntılanan metin");
        assert_eq!(quote.author.label(), "Alıntı (@alintilanan.bsky.social)");
        assert_eq!(
            quote.url,
            "https://bsky.app/profile/alintilanan.bsky.social/post/alinti"
        );
    }

    #[test]
    fn video_plays_as_mp4_blob_with_poster() {
        let json = r#"{
            "thread": { "post": {
                "uri": "at://did:plc:abc/app.bsky.feed.post/vid",
                "author": { "handle": "video.bsky.social" },
                "record": { "text": "video", "createdAt": "2024-05-06T12:34:56Z" },
                "embed": {
                    "$type": "app.bsky.embed.video#view",
                    "cid": "bafkreivid",
                    "playlist": "https://video.bsky.app/watch/did%3Aplc%3Aabc/bafkreivid/playlist.m3u8",
                    "thumbnail": "https://video.bsky.app/watch/did%3Aplc%3Aabc/bafkreivid/thumbnail.jpg",
                    "aspectRatio": { "width": 720, "height": 1280 }
                }
            }}}
        "#;

        let post = post_view_to_post(&thread_from(json), "video.bsky.social");
        assert_eq!(post.media.len(), 1);
        assert_eq!(post.media[0].kind, MediaKind::Video);
        assert_eq!(
            post.media[0].url,
            "https://bsky.social/xrpc/com.atproto.sync.getBlob?did=did:plc:abc&cid=bafkreivid"
        );
        assert_eq!(
            post.media[0].poster.as_deref(),
            Some("https://video.bsky.app/watch/did%3Aplc%3Aabc/bafkreivid/thumbnail.jpg")
        );
        assert_eq!(post.media[0].height, Some(1280));
    }

    #[test]
    fn gallery_items_become_images_with_alt_text() {
        let json = r#"{
            "thread": { "post": {
                "uri": "at://did:plc:abc/app.bsky.feed.post/gal",
                "author": { "handle": "galeri.bsky.social" },
                "record": { "text": "galeri" },
                "embed": { "$type": "app.bsky.embed.gallery#view", "items": [
                    { "fullsize": "https://cdn.bsky.app/img/1.jpg", "alt": "bir" },
                    { "fullsize": "https://cdn.bsky.app/img/2.jpg", "alt": "" }
                ] }
            }}}
        "#;

        let post = post_view_to_post(&thread_from(json), "galeri.bsky.social");
        assert_eq!(post.media.len(), 2);
        assert_eq!(post.media[0].url, "https://cdn.bsky.app/img/1.jpg");
        assert_eq!(post.media[0].alt.as_deref(), Some("bir"));
    }

    #[test]
    fn reply_that_quotes_keeps_both_parent_and_quote() {
        let json = r#"{
            "thread": {
                "parent": { "post": {
                    "uri": "at://did:plc:p/app.bsky.feed.post/p1",
                    "author": { "handle": "ust.bsky.social" },
                    "record": { "text": "üst" }
                }},
                "post": {
                    "uri": "at://did:plc:m/app.bsky.feed.post/m1",
                    "author": { "handle": "ana.bsky.social" },
                    "record": { "text": "yanıt ve alıntı", "reply": { "parent": { "uri": "at://did:plc:p/app.bsky.feed.post/p1" } } },
                    "embed": { "$type": "app.bsky.embed.record#view", "record": {
                        "uri": "at://did:plc:q/app.bsky.feed.post/q1",
                        "author": { "handle": "alinti.bsky.social" },
                        "value": { "text": "alıntı" }
                    } }
                }
            }
        }"#;
        let response: ThreadResponse = serde_json::from_str(json).expect("geçerli");
        let post = thread_view_to_post(&response.thread, "ana.bsky.social");
        assert_eq!(post.reply_to.expect("üst gönderi").text, "üst");
        assert_eq!(post.quote.expect("alıntı").text, "alıntı");
    }

    #[test]
    fn op_thread_position_counts_the_authors_chain() {
        let thread: ThreadV2 = serde_json::from_str(
            r#"{ "thread": [
                { "depth": -2, "value": { "opThread": true } },
                { "depth": -1, "value": { "opThread": true } },
                { "depth": 0, "value": { "opThread": true } },
                { "depth": 1, "value": { "opThread": true } },
                { "depth": 1, "value": { "opThread": false } },
                { "depth": 2, "value": { "opThread": false } }
            ] }"#,
        )
        .unwrap();
        assert_eq!(op_position(&thread), Some((3, 4)));

        // Başkasına yanıt (gönderi yazar ipliğinde değil) ve tek gönderilik iplik konum üretmez.
        let other: ThreadV2 = serde_json::from_str(
            r#"{ "thread": [
                { "depth": -1, "value": { "opThread": true } },
                { "depth": 0, "value": { "opThread": false } }
            ] }"#,
        )
        .unwrap();
        assert_eq!(op_position(&other), None);
        let single: ThreadV2 = serde_json::from_str(
            r#"{ "thread": [ { "depth": 0, "value": { "opThread": true } } ] }"#,
        )
        .unwrap();
        assert_eq!(op_position(&single), None);
    }

    #[test]
    fn starter_pack_embed_becomes_a_link_card() {
        let json = r#"{
            "thread": { "post": {
                "uri": "at://did:plc:abc/app.bsky.feed.post/sp",
                "author": { "handle": "bsky.app" },
                "record": { "text": "paket" },
                "embed": { "$type": "app.bsky.embed.record#view", "record": {
                    "uri": "at://did:plc:abc/app.bsky.graph.starterpack/3mzd",
                    "creator": { "handle": "bsky.app" },
                    "record": { "$type": "app.bsky.graph.starterpack", "name": "Seçim", "description": "Muhabirler" }
                } }
            }}}
        "#;
        let post = post_view_to_post(&thread_from(json), "bsky.app");
        assert!(post.quote.is_none());
        let preview = post.link_preview.expect("bağlantı kartı");
        assert_eq!(preview.url, "https://bsky.app/starter-pack/bsky.app/3mzd");
        assert_eq!(preview.title.as_deref(), Some("Başlangıç paketi: Seçim"));
        assert_eq!(preview.description.as_deref(), Some("Muhabirler"));
    }

    #[test]
    fn video_blob_url_rejects_unexpected_paths() {
        assert_eq!(video_blob_url("https://video.bsky.app/other/x"), None);
        assert_eq!(
            video_blob_url("https://video.bsky.app/watch/did%3Aplc%3Aa/b&c=1/playlist.m3u8"),
            None
        );
    }

    #[test]
    fn at_uri_query_component_is_encoded() {
        assert_eq!(
            urlencoding::encode("at://did:plc:abc/app.bsky.feed.post/1"),
            "at%3A%2F%2Fdid%3Aplc%3Aabc%2Fapp.bsky.feed.post%2F1"
        );
    }

    #[test]
    fn bounded_thread_fixture_preserves_parent_and_repost_relation() {
        let json = r#"{
            "thread": {
                "type": "app.bsky.feed.defs#threadViewPost",
                "reason": { "$type": "app.bsky.feed.defs#reasonRepost", "by": { "handle": "resharer.bsky.social" } },
                "parent": { "post": {
                    "uri": "at://did:plc:parent/app.bsky.feed.post/parent1",
                    "author": { "handle": "parent.bsky.social", "displayName": "Parent" },
                    "record": { "text": "Üst gönderi", "createdAt": "2024-05-06T12:34:56Z" }
                }},
                "replies": [
                    { "post": { "uri": "at://did:plc:reply/app.bsky.feed.post/reply1", "author": { "handle": "reply.bsky.social" }, "record": { "text": "yanıt" } } },
                    { "post": { "uri": "at://did:plc:reply/app.bsky.feed.post/reply2", "author": { "handle": "reply2.bsky.social" }, "record": { "text": "yanıt 2" } } }
                ],
                "post": {
                    "uri": "at://did:plc:main/app.bsky.feed.post/main1",
                    "author": { "handle": "main.bsky.social" },
                    "record": { "text": "Ana gönderi", "reply": { "parent": { "uri": "at://did:plc:parent/app.bsky.feed.post/parent1" } } }
                }
            }
        }"#;
        let response: ThreadResponse = serde_json::from_str(json).expect("thread fixture geçerli");
        let post = thread_view_to_post(&response.thread, "main.bsky.social");
        let parent = post
            .reply_to
            .expect("üst gönderi yanıt bağlamı olarak korunmalı");
        assert_eq!(parent.text, "Üst gönderi");
        assert_eq!(
            parent.author.profile_url.as_deref(),
            Some("https://bsky.app/profile/parent.bsky.social")
        );
        assert!(
            post.quote.is_none(),
            "üst gönderi alıntı yuvasını işgal etmemeli"
        );
        // Yanıt ilişkisi kartta bağlam olarak çizilir; alt bilgide yalnızca repost kalır.
        assert_eq!(
            post.extra_label.as_deref(),
            Some("Repost • @resharer.bsky.social")
        );
        assert_eq!(post.stats.replies, 2);

        let repost = RepostReason {
            kind: Some("app.bsky.feed.defs#reasonRepost".to_string()),
            by: Some(ProfileView {
                handle: Some("resharer.bsky.social".to_string()),
                ..Default::default()
            }),
        };
        let mut repost_thread = response.thread;
        repost_thread.parent = None;
        repost_thread.replies = None;
        repost_thread.reason = Some(repost);
        let repost_post = thread_view_to_post(&repost_thread, "main.bsky.social");
        assert_eq!(
            repost_post.extra_label.as_deref(),
            Some("Repost • @resharer.bsky.social")
        );
    }

    fn link_facet(start: usize, end: usize, uri: &str) -> Facet {
        Facet {
            index: Some(FacetIndex {
                byte_start: Some(start),
                byte_end: Some(end),
            }),
            features: Some(vec![FacetFeature {
                kind: Some("app.bsky.richtext.facet#link".to_string()),
                uri: Some(uri.to_string()),
            }]),
        }
    }

    #[test]
    fn duplicate_and_overlapping_facets_do_not_panic_on_multibyte_text() {
        let text = "abcde日日日";
        let duplicate = [
            link_facet(5, 14, "https://a.example"),
            link_facet(5, 14, "https://a.example"),
        ];
        assert_eq!(
            apply_link_facets(text, Some(&duplicate)),
            "abcde[日日日](<https://a.example>)"
        );

        let overlapping = [
            link_facet(5, 14, "https://a.example"),
            link_facet(0, 8, "https://b.example"),
        ];
        assert_eq!(
            apply_link_facets(text, Some(&overlapping)),
            "abcde[日日日](<https://a.example>)"
        );

        let disjoint = [
            link_facet(0, 2, "https://b.example"),
            link_facet(5, 14, "https://a.example"),
        ];
        assert_eq!(
            apply_link_facets(text, Some(&disjoint)),
            "[ab](<https://b.example>)cde[日日日](<https://a.example>)"
        );
    }

    #[test]
    fn record_with_media_keeps_quote_and_quoted_images() {
        let json = r#"{
            "thread": { "post": {
                "uri": "at://did:plc:abc/app.bsky.feed.post/main",
                "author": { "handle": "ana.bsky.social" },
                "record": { "text": "Fotoğraflı alıntı", "createdAt": "2024-05-06T12:34:56Z" },
                "embed": {
                    "$type": "app.bsky.embed.recordWithMedia#view",
                    "media": {
                        "$type": "app.bsky.embed.images#view",
                        "images": [{ "thumb": "https://cdn.bsky.app/t/1.jpg", "fullsize": "https://cdn.bsky.app/img/1.jpg", "alt": "" }]
                    },
                    "record": {
                        "$type": "app.bsky.embed.record#view",
                        "record": {
                            "$type": "app.bsky.embed.record#viewRecord",
                            "uri": "at://did:plc:def/app.bsky.feed.post/alinti",
                            "cid": "bafyreia",
                            "author": { "did": "did:plc:def", "handle": "alintilanan.bsky.social", "displayName": "Alıntı" },
                            "value": { "$type": "app.bsky.feed.post", "text": "alıntılanan metin", "createdAt": "2024-05-05T10:00:00Z" },
                            "labels": [],
                            "indexedAt": "2024-05-05T10:00:01Z",
                            "embeds": [{
                                "$type": "app.bsky.embed.images#view",
                                "images": [{ "thumb": "https://cdn.bsky.app/t/q.jpg", "fullsize": "https://cdn.bsky.app/img/q.jpg", "alt": "" }]
                            }]
                        }
                    }
                }
            }}}
        "#;

        let post = post_view_to_post(&thread_from(json), "ana.bsky.social");
        assert_eq!(post.media.len(), 1);
        assert_eq!(post.media[0].url, "https://cdn.bsky.app/img/1.jpg");
        let quote = post.quote.expect("recordWithMedia alıntısı korunmalı");
        assert_eq!(quote.text, "alıntılanan metin");
        assert_eq!(quote.author.label(), "Alıntı (@alintilanan.bsky.social)");
        assert_eq!(quote.media.len(), 1);
        assert_eq!(quote.media[0].url, "https://cdn.bsky.app/img/q.jpg");
    }

    #[test]
    fn quote_with_only_images_is_kept() {
        let json = r#"{
            "thread": { "post": {
                "uri": "at://did:plc:abc/app.bsky.feed.post/main",
                "author": { "handle": "ana.bsky.social" },
                "record": { "text": "Bakın", "createdAt": "2024-05-06T12:34:56Z" },
                "embed": {
                    "$type": "app.bsky.embed.record#view",
                    "record": {
                        "$type": "app.bsky.embed.record#viewRecord",
                        "uri": "at://did:plc:def/app.bsky.feed.post/foto",
                        "author": { "handle": "foto.bsky.social" },
                        "value": { "$type": "app.bsky.feed.post", "text": "", "createdAt": "2024-05-05T10:00:00Z" },
                        "embeds": [{
                            "$type": "app.bsky.embed.images#view",
                            "images": [
                                { "fullsize": "https://cdn.bsky.app/img/a.jpg" },
                                { "fullsize": "https://cdn.bsky.app/img/b.jpg" }
                            ]
                        }]
                    }
                }
            }}}
        "#;

        let post = post_view_to_post(&thread_from(json), "ana.bsky.social");
        let quote = post.quote.expect("yalnızca görselli alıntı korunmalı");
        assert!(quote.text.is_empty());
        assert_eq!(quote.media.len(), 2);
        assert_eq!(quote.media[1].url, "https://cdn.bsky.app/img/b.jpg");
    }

    #[test]
    fn reply_count_is_a_metric_not_a_footer_label() {
        let json = r#"{
            "thread": {
                "post": {
                    "uri": "at://did:plc:main/app.bsky.feed.post/main1",
                    "author": { "handle": "main.bsky.social" },
                    "record": { "text": "Ana gönderi" },
                    "replyCount": 87
                },
                "replies": [
                    { "post": { "uri": "at://did:plc:reply/app.bsky.feed.post/reply1", "record": { "text": "yanıt" } } }
                ]
            }
        }"#;
        let response: ThreadResponse = serde_json::from_str(json).expect("thread fixture geçerli");
        let post = thread_view_to_post(&response.thread, "main.bsky.social");
        assert_eq!(post.stats.replies, 87);
        assert_eq!(post.extra_label, None);
    }
}
