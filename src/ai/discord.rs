//! Discord'dan AI bağlamı toplama: yanıt zinciri, yanıtlanan mesaj, görseller ve sunucu
//! emojileri.
//!
//! En az veri ilkesi:
//! - Kullanıcı kimliği, kullanıcı adı veya takma ad sağlayıcıya gitmez. Konuşanlar ve metindeki
//!   etiketler istek içinde tutarlı takma adlara çevrilir ("Kullanıcı", "Kişi A", …); rol ve
//!   kanal etiketleri genel yer tutuculara dönüşür.
//! - Dosya adları gönderilmez; yalnızca kaç ek olduğu söylenir.
//! - Görseller Discord medya vekilinden küçültülmüş ve WebP'ye yeniden kodlanmış kopya olarak
//!   indirilir ve `data:` adresi olarak gömülür; sağlayıcı Discord CDN bağlantısını görmez.
//!   Yalnızca Discord'un kendi alan adlarından indirme yapılır.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serenity::all::{Context, GuildId, Message, MessageReferenceKind, UserId};
use tokio::task::JoinSet;
use tracing::debug;

use super::{ContextMessage, Conversation, ImageInput};
use crate::utils::truncate_chars;

/// Yanıtlanan mesajdan geriye doğru izlenen en fazla mesaj.
const MAX_HISTORY: usize = 4;
/// İsteğe eklenen en fazla görsel.
const MAX_IMAGES: usize = 3;
/// İndirilen tek görselin üst sınırı (yeniden kodlanmış kopya genelde çok daha küçüktür).
const MAX_IMAGE_BYTES: usize = 3 * 1024 * 1024;
/// Medya vekilinden istenen en uzun kenar.
const MAX_IMAGE_SIDE: u32 = 1024;
const IMAGE_TIMEOUT: Duration = Duration::from_secs(8);
/// Mesaj başına bağlama alınan en fazla embed.
const MAX_EMBEDS: usize = 3;
const MAX_EMBED_CHARS: usize = 800;
/// Sistem istemine yazılan en fazla sunucu emojisi.
const MAX_EMOJIS: usize = 60;
/// Tek yanıtta gösterilen en fazla özel emoji; model abartırsa fazlası atılır.
const MAX_EMOJIS_PER_REPLY: usize = 2;

/// Kullanıcı yalnızca botu etiketleyip bir mesaja/görsele yanıt verdiğinde gönderilen istem.
const BARE_MENTION_PROMPT: &str = "(Kullanıcı yalnızca seni etiketledi; yanıtladığı mesaja veya görsele bağlama uygun kısa bir yorum yap.)";

/// İstek içinde tutarlı takma adlar: aynı kişi her yerde aynı adla görünür.
struct Pseudonyms {
    bot: UserId,
    asker: UserId,
    others: Vec<UserId>,
}

impl Pseudonyms {
    fn new(bot: UserId, asker: UserId) -> Self {
        Self {
            bot,
            asker,
            others: Vec::new(),
        }
    }

    /// Konuşan kişinin etiketi.
    fn speaker(&mut self, user: UserId) -> String {
        if user == self.bot {
            "Sen".to_string()
        } else {
            self.alias(user)
        }
    }

    /// Metindeki etiketin yerine geçen ad.
    fn alias(&mut self, user: UserId) -> String {
        if user == self.bot {
            return "bot".to_string();
        }
        if user == self.asker {
            return "Kullanıcı".to_string();
        }
        let index = match self.others.iter().position(|other| *other == user) {
            Some(index) => index,
            None => {
                self.others.push(user);
                self.others.len() - 1
            }
        };
        match u8::try_from(index).ok().filter(|i| *i < 26) {
            Some(i) => format!("Kişi {}", char::from(b'A' + i)),
            None => format!("Kişi {}", index + 1),
        }
    }
}

/// Discord biçimli etiketleri takma ada/yer tutucuya çevirir; özel emoji kodları `:ad:` olur.
fn scrub(text: &str, names: &mut Pseudonyms) -> String {
    static TOKENS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"<@!?(\d+)>|<@&\d+>|<#\d+>|<a?:(\w{2,32}):\d+>").unwrap());
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for caps in TOKENS.captures_iter(text) {
        let whole = caps.get(0).unwrap();
        out.push_str(&text[last..whole.start()]);
        let replacement = if let Some(id) = caps.get(1) {
            match id.as_str().parse::<u64>().ok().filter(|id| *id != 0) {
                Some(id) => format!("@{}", names.alias(UserId::new(id))),
                None => "@biri".to_string(),
            }
        } else if let Some(emoji) = caps.get(2) {
            format!(":{}:", emoji.as_str())
        } else if whole.as_str().starts_with("<@&") {
            "@rol".to_string()
        } else {
            "#kanal".to_string()
        };
        out.push_str(&replacement);
        last = whole.end();
    }
    out.push_str(&text[last..]);
    out
}

/// Mesajın modele gidecek metni: içerik, embed metinleri ve ek sayısı (dosya adları yok).
fn message_text(msg: &Message, names: &mut Pseudonyms) -> String {
    let mut parts = Vec::new();
    let content = scrub(msg.content.trim(), names);
    if !content.is_empty() {
        parts.push(content);
    }
    for embed in msg.embeds.iter().take(MAX_EMBEDS) {
        let mut lines = Vec::new();
        if let Some(author) = &embed.author {
            lines.push(author.name.clone());
        }
        if let Some(title) = &embed.title {
            lines.push(title.clone());
        }
        if let Some(description) = &embed.description {
            lines.push(description.clone());
        }
        for field in &embed.fields {
            lines.push(format!("{}: {}", field.name, field.value));
        }
        if let Some(footer) = &embed.footer {
            lines.push(footer.text.clone());
        }
        let text = scrub(lines.join("\n").trim(), names);
        if !text.is_empty() {
            parts.push(format!(
                "[Bağlantı önizlemesi]\n{}",
                truncate_chars(&text, MAX_EMBED_CHARS)
            ));
        }
    }
    let images = msg.attachments.iter().filter(|a| is_image(a)).count();
    let files = msg.attachments.len() - images;
    if images > 0 {
        parts.push(format!("[{images} görsel]"));
    }
    if files > 0 {
        parts.push(format!("[{files} dosya eki]"));
    }
    parts.join("\n")
}

fn is_image(attachment: &serenity::all::Attachment) -> bool {
    match attachment.content_type.as_deref() {
        Some(kind) => kind.starts_with("image/"),
        None => {
            let name = attachment.filename.to_ascii_lowercase();
            [".png", ".jpg", ".jpeg", ".webp", ".gif"]
                .iter()
                .any(|ext| name.ends_with(ext))
        }
    }
}

/// Mesaj görsel içeriyor mu? (ek veya embed görseli) — boş etiketin yanıtlanıp
/// yanıtlanmayacağına karar vermek için ucuz kontrol.
pub fn has_images(msg: &Message) -> bool {
    !image_urls(msg).is_empty()
}

/// Medya vekili adresine küçültme ve WebP yeniden kodlama parametreleri ekler.
fn proxied(url: &str, width: Option<u32>, height: Option<u32>) -> String {
    let mut params = vec!["format=webp".to_string()];
    if let (Some(w), Some(h)) = (width, height)
        && w > 0
        && h > 0
    {
        let longest = w.max(h);
        let (w, h) = if longest > MAX_IMAGE_SIDE {
            (
                (u64::from(w) * u64::from(MAX_IMAGE_SIDE) / u64::from(longest)).max(1),
                (u64::from(h) * u64::from(MAX_IMAGE_SIDE) / u64::from(longest)).max(1),
            )
        } else {
            (u64::from(w), u64::from(h))
        };
        params.push(format!("width={w}&height={h}"));
    }
    let separator = if url.contains('?') { '&' } else { '?' };
    format!("{url}{separator}{}", params.join("&"))
}

/// Mesajdaki görsellerin indirilecek adresleri (önce vekil kopyası, sonra özgün dosya).
fn image_urls(msg: &Message) -> Vec<[String; 2]> {
    let mut urls: Vec<[String; 2]> = msg
        .attachments
        .iter()
        .filter(|a| is_image(a))
        .map(|a| [proxied(&a.proxy_url, a.width, a.height), a.url.clone()])
        .collect();
    for embed in &msg.embeds {
        let image = embed
            .image
            .as_ref()
            .and_then(|i| Some((i.proxy_url.as_deref()?, i.width, i.height)))
            .or_else(|| {
                embed
                    .thumbnail
                    .as_ref()
                    .and_then(|t| Some((t.proxy_url.as_deref()?, t.width, t.height)))
            });
        if let Some((proxy, width, height)) = image {
            urls.push([proxied(proxy, width, height), proxy.to_string()]);
        }
    }
    urls
}

/// Yalnızca Discord'un kendi CDN / medya vekili alan adlarına HTTPS ile gidilir.
fn is_discord_media(raw: &str) -> bool {
    let Ok(url) = url::Url::parse(raw) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str().is_some_and(|host| {
            host == "cdn.discordapp.com"
                || host == "media.discordapp.net"
                || host.ends_with(".discordapp.net")
        })
}

async fn download_image(client: &reqwest::Client, url: &str) -> Option<String> {
    if !is_discord_media(url) {
        return None;
    }
    let mut response = client
        .get(url)
        .timeout(IMAGE_TIMEOUT)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)?
        .to_str()
        .ok()?
        .split(';')
        .next()?
        .trim()
        .to_ascii_lowercase();
    if !matches!(
        mime.as_str(),
        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
    ) {
        return None;
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_IMAGE_BYTES as u64)
    {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if body.len() + chunk.len() > MAX_IMAGE_BYTES {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    if body.is_empty() {
        return None;
    }
    Some(format!(
        "data:{mime};base64,{}",
        crate::utils::base64_encode(&body)
    ))
}

/// Görselleri paralel indirir; sıra korunur. Dönen: (görseller, eklenemeyenlerin sayısı).
async fn collect_images(
    client: &reqwest::Client,
    sources: Vec<([String; 2], bool)>,
) -> (Vec<ImageInput>, usize) {
    let total = sources.len();
    let mut tasks = JoinSet::new();
    for (index, (urls, from_reply)) in sources.into_iter().take(MAX_IMAGES).enumerate() {
        let client = client.clone();
        tasks.spawn(async move {
            for url in &urls {
                if let Some(data_url) = download_image(&client, url).await {
                    return Some((
                        index,
                        ImageInput {
                            data_url,
                            from_reply,
                        },
                    ));
                }
            }
            debug!("AI bağlamı için görsel indirilemedi");
            None
        });
    }
    let mut images: Vec<(usize, ImageInput)> = Vec::new();
    while let Some(result) = tasks.join_next().await {
        if let Ok(Some(image)) = result {
            images.push(image);
        }
    }
    images.sort_by_key(|(index, _)| *index);
    let skipped = total - images.len();
    (
        images.into_iter().map(|(_, image)| image).collect(),
        skipped,
    )
}

/// Mesajın yanıtladığı mesaj (iletilen mesajlar zincir sayılmaz).
async fn parent(ctx: &Context, msg: &Message) -> Option<Message> {
    if let Some(parent) = msg.referenced_message.as_deref() {
        return Some(parent.clone());
    }
    let reference = msg.message_reference.as_ref()?;
    if reference.kind != MessageReferenceKind::Default {
        return None;
    }
    let id = reference.message_id?;
    if let Some(cached) = ctx.cache.message(reference.channel_id, id) {
        return Some(cached.clone());
    }
    reference.channel_id.message(&ctx.http, id).await.ok()
}

/// AI isteğine gidecek kullanıcı metni ve çevresi.
pub struct Gathered {
    pub query: String,
    pub conversation: Conversation,
}

/// Kullanıcının mesajından AI bağlamını toplar. Sorulacak bir şey yoksa `None`.
pub async fn gather(
    ctx: &Context,
    client: &reqwest::Client,
    msg: &Message,
    replied: Option<&Message>,
    bot_id: UserId,
    emojis: &GuildEmojis,
) -> Option<Gathered> {
    let mut names = Pseudonyms::new(bot_id, msg.author.id);

    let without_mention = msg
        .content
        .replace(&format!("<@{bot_id}>"), "")
        .replace(&format!("<@!{bot_id}>"), "");
    let mut query = scrub(without_mention.trim(), &mut names);

    let mut conversation = Conversation {
        emojis: emojis.names(),
        ..Conversation::default()
    };
    let mut image_sources: Vec<([String; 2], bool)> = image_urls(msg)
        .into_iter()
        .map(|urls| (urls, false))
        .collect();

    if let Some(replied) = replied {
        conversation.replied = Some(ContextMessage {
            speaker: names.speaker(replied.author.id),
            text: message_text(replied, &mut names),
        });
        image_sources.extend(image_urls(replied).into_iter().map(|urls| (urls, true)));

        let mut history = Vec::new();
        let mut current = parent(ctx, replied).await;
        while let Some(message) = current {
            if history.len() >= MAX_HISTORY {
                break;
            }
            history.push(ContextMessage {
                speaker: names.speaker(message.author.id),
                text: message_text(&message, &mut names),
            });
            current = parent(ctx, &message).await;
        }
        history.reverse();
        conversation.history = history;
    }

    if query.is_empty() {
        if conversation.replied.is_none() && image_sources.is_empty() {
            return None;
        }
        query = BARE_MENTION_PROMPT.to_string();
    }

    let (images, skipped) = collect_images(client, image_sources).await;
    conversation.images = images;
    conversation.skipped_images = skipped;
    Some(Gathered {
        query,
        conversation,
    })
}

// ─── Sunucu emojileri ─────────────────────────────────────────────────────────

/// Botun bu sunucuda kullanabileceği özel emojiler: ad → Discord biçimi (`<:ad:id>`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GuildEmojis {
    entries: Vec<(String, String)>,
}

impl GuildEmojis {
    pub fn from_cache(ctx: &Context, guild_id: Option<GuildId>) -> Self {
        let Some(guild) = guild_id.and_then(|id| ctx.cache.guild(id)) else {
            return Self::default();
        };
        let mut emojis: Vec<_> = guild
            .emojis
            .values()
            // Rol kısıtlı emojileri bot kullanamayabilir; bot boostu düşen emojiyi de.
            .filter(|emoji| emoji.available && emoji.roles.is_empty())
            .map(|emoji| (emoji.id, emoji.name.clone(), emoji.animated))
            .collect();
        emojis.sort_by_key(|(id, _, _)| *id);
        Self::new(
            emojis
                .into_iter()
                .take(MAX_EMOJIS)
                .map(|(id, name, animated)| (name, id.get(), animated)),
        )
    }

    fn new(emojis: impl IntoIterator<Item = (String, u64, bool)>) -> Self {
        Self {
            entries: emojis
                .into_iter()
                .map(|(name, id, animated)| {
                    let prefix = if animated { "a" } else { "" };
                    let rendered = format!("<{prefix}:{name}:{id}>");
                    (name, rendered)
                })
                .collect(),
        }
    }

    pub fn names(&self) -> Vec<String> {
        self.entries.iter().map(|(name, _)| name.clone()).collect()
    }

    /// Modelin yazdığı `:ad:` (ya da yanlış kimlikli `<:ad:123>`) kodlarını sunucunun gerçek
    /// emojisine çevirir; tanınmayan kodlara dokunmaz. İlk [`MAX_EMOJIS_PER_REPLY`] emojiden
    /// sonrakiler (önlerindeki boşlukla birlikte) atılır.
    pub fn render(&self, text: &str) -> String {
        if self.entries.is_empty() {
            return text.to_string();
        }
        static CODES: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"( ?)(?:<a?:(\w{2,32}):\d*>|:(\w{2,32}):)").unwrap());
        let exact: HashMap<&str, &str> = self
            .entries
            .iter()
            .map(|(name, rendered)| (name.as_str(), rendered.as_str()))
            .collect();
        let folded: HashMap<String, &str> = self
            .entries
            .iter()
            .map(|(name, rendered)| (name.to_lowercase(), rendered.as_str()))
            .collect();
        let mut used = 0;
        CODES
            .replace_all(text, |caps: &regex::Captures<'_>| {
                let name = caps
                    .get(2)
                    .or_else(|| caps.get(3))
                    .map_or("", |m| m.as_str());
                let Some(rendered) = exact
                    .get(name)
                    .copied()
                    .or_else(|| folded.get(&name.to_lowercase()).copied())
                else {
                    return caps[0].to_string();
                };
                used += 1;
                if used > MAX_EMOJIS_PER_REPLY {
                    String::new()
                } else {
                    format!("{}{rendered}", &caps[1])
                }
            })
            .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: u64) -> UserId {
        UserId::new(id)
    }

    #[test]
    fn pseudonyms_are_stable_and_never_leak_ids() {
        let mut names = Pseudonyms::new(user(1), user(2));
        assert_eq!(names.speaker(user(1)), "Sen");
        assert_eq!(names.speaker(user(2)), "Kullanıcı");
        assert_eq!(names.speaker(user(30)), "Kişi A");
        assert_eq!(names.speaker(user(40)), "Kişi B");
        assert_eq!(names.speaker(user(30)), "Kişi A");

        let text = scrub(
            "<@30> ve <@!40>, <@1> ile <@&555> rolünü <#777> kanalında konuştu <:kekw:123456>",
            &mut names,
        );
        assert_eq!(
            text,
            "@Kişi A ve @Kişi B, @bot ile @rol rolünü #kanal kanalında konuştu :kekw:"
        );
        assert!(!text.chars().any(|c| c.is_ascii_digit()));
    }

    #[test]
    fn many_people_get_numbered_aliases() {
        let mut names = Pseudonyms::new(user(1), user(2));
        for id in 0..26 {
            names.alias(user(100 + id));
        }
        assert_eq!(names.alias(user(999)), "Kişi 27");
    }

    #[test]
    fn media_proxy_url_requests_a_small_webp_copy() {
        assert_eq!(
            proxied(
                "https://media.discordapp.net/attachments/1/2/a.png?ex=1&hm=2",
                Some(4000),
                Some(2000)
            ),
            "https://media.discordapp.net/attachments/1/2/a.png?ex=1&hm=2&format=webp&width=1024&height=512"
        );
        assert_eq!(
            proxied(
                "https://images-ext-1.discordapp.net/external/x/y.jpg",
                None,
                None
            ),
            "https://images-ext-1.discordapp.net/external/x/y.jpg?format=webp"
        );
    }

    #[test]
    fn only_discord_media_hosts_are_downloaded() {
        assert!(is_discord_media(
            "https://cdn.discordapp.com/attachments/1/2/a.png"
        ));
        assert!(is_discord_media(
            "https://media.discordapp.net/attachments/1/2/a.png"
        ));
        assert!(is_discord_media(
            "https://images-ext-2.discordapp.net/external/x"
        ));
        assert!(!is_discord_media("http://cdn.discordapp.com/a.png"));
        assert!(!is_discord_media(
            "https://evil.example/cdn.discordapp.com/a.png"
        ));
        assert!(!is_discord_media(
            "https://discordapp.net.evil.example/a.png"
        ));
        assert!(!is_discord_media("https://127.0.0.1/a.png"));
    }

    #[test]
    fn emoji_codes_are_rendered_for_the_guild() {
        let emojis = GuildEmojis::new([
            ("kekw".to_string(), 11, false),
            ("PartyBlob".to_string(), 22, true),
        ]);
        assert_eq!(emojis.names(), vec!["kekw", "PartyBlob"]);
        assert_eq!(
            emojis.render("Haha <:kekw:999> tebrikler :partyblob: :bilinmeyen: 10:30:45"),
            "Haha <:kekw:11> tebrikler <a:PartyBlob:22> :bilinmeyen: 10:30:45"
        );
        assert_eq!(GuildEmojis::default().render(":kekw:"), ":kekw:");
    }

    #[test]
    fn emoji_spam_is_capped() {
        let emojis = GuildEmojis::new([("kekw".to_string(), 11, false)]);
        assert_eq!(
            emojis.render("a :kekw: b :kekw: c :kekw: :kekw:"),
            "a <:kekw:11> b <:kekw:11> c"
        );
    }
}
