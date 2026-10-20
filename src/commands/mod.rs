//! Slash komutları ve mesaj (sağ tık) komutu.
//!
//! Discord, slash komutu etkileşiminde **yanıtlanan mesajı göndermez**: slash komutu
//! `message_reference` taşımaz (yalnızca mesaj komutları hedef mesajı `resolved.messages`
//! içinde iletir). Bu yüzden "bir mesaja yanıt verip `/dönüştür` yazma" akışı Discord API'sinde
//! doğrudan desteklenmez; onun yerine aynı dönüşümü iki yerden sunuyoruz:
//!
//! - `/dönüştür link:<bağlantı>`: bağlantıyı doğrudan komut argümanı olarak alır.
//! - **Mesaja sağ tık → Apps → Dönüştür**: hedef mesajın metnindeki bağlantıyı dönüştürür.
//!
//! Her iki akış da aynı dönüşüm motorunu ve rol filtrelerini kullanır; WebUI'daki platform
//! modülü açık/kapalı ayarından bağımsızdır (komut açıkça çağrıldığında her zaman dönüştürür).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serenity::all::{
    Command, CommandData, CommandDataOptionValue, CommandInteraction, CommandOptionType,
    CommandType, Context, CreateCommand, CreateCommandOption, CreateInteractionResponse,
    CreateInteractionResponseMessage, EditInteractionResponse, Http, Interaction, MessageId, User,
};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::converters::{self, ConvertError};
use crate::settings::AppSettings;

/// Sohbet girdisi komutunun adı. Discord komut adları küçük harf olmalıdır.
pub const CONVERT_COMMAND: &str = "dönüştür";
/// Mesaja sağ tıklandığında görünen komutun adı.
pub const CONVERT_MESSAGE_COMMAND: &str = "Dönüştür";
/// Bağlantıyı taşıyan sohbet komutu seçeneği.
pub const LINK_OPTION: &str = "link";

/// Komut yokken gösterilen kısa kullanım metni.
pub const USAGE: &str = "Bir bağlantı ver: `/dönüştür link:https://x.com/kullanici/status/123`\n\
Bir mesajı dönüştürmek için: sağ tık → **Apps → Dönüştür**. \
(Discord, slash komutlarına yanıtlanan mesajı eklemez; bu yüzden mesaj komutu kullanılır.)";

static COMMANDS_REGISTERED: AtomicBool = AtomicBool::new(false);

/// Global komutları (DM'ler dâhil her yerde görünür) kaydeder.
///
/// Süreç başına bir kez çalışır: her gateway yeniden bağlanmasında Discord'a aynı komutları
/// yeniden yazmamak için `AtomicBool` ile korunur. Kayıt başarısız olursa bayrak geri alınır.
pub async fn register(http: impl AsRef<Http>) {
    if COMMANDS_REGISTERED.swap(true, Ordering::SeqCst) {
        return;
    }

    // Geçici ağ hataları komutların tüm süreç ömrü boyunca eksik kalmasına yol açmasın:
    // kısa aralıklarla üç kez denenir. Kalıcı hata bayrağı geri alır (sonraki ready tekrar dener).
    for attempt in 0..3 {
        match Command::set_global_commands(http.as_ref(), definitions()).await {
            Ok(commands) => {
                info!(
                    count = commands.len(),
                    "Global komutlar kaydedildi (/{} ve mesaj komutu {}); Discord istemcilerine \
                     yayılması birkaç dakika sürebilir",
                    CONVERT_COMMAND,
                    CONVERT_MESSAGE_COMMAND
                );
                return;
            }
            Err(err) => {
                warn!(attempt, "Slash komutları kaydedilemedi: {err:?}");
                if attempt < 2 {
                    tokio::time::sleep(std::time::Duration::from_secs(2 * (attempt + 1))).await;
                }
            }
        }
    }

    COMMANDS_REGISTERED.store(false, Ordering::SeqCst);
    error!("Slash komutları Discord'a kaydedilemedi; sonraki bağlantıda yeniden denenecek");
}

/// Discord'a gönderilen komut tanımları.
fn definitions() -> Vec<CreateCommand> {
    vec![
        CreateCommand::new(CONVERT_COMMAND)
            .description("Bir sosyal medya bağlantısını dönüştürür (X, Instagram, Reddit, TikTok, Bluesky ve daha fazlası).")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    LINK_OPTION,
                    "Dönüştürülecek bağlantı (boş bırakılırsa kullanım gösterilir)",
                )
                .required(false)
                .max_length(2048),
            ),
        CreateCommand::new(CONVERT_MESSAGE_COMMAND)
            .kind(CommandType::Message)
            // Discord, USER/MESSAGE komutlarında açıklamayı boş string olarak bekler.
            .description(""),
    ]
}

/// Etkileşim olayını yorumlar. Komut dışındaki etkileşimler sessizce yok sayılır.
pub async fn handle(
    ctx: &Context,
    interaction: &Interaction,
    http_client: &reqwest::Client,
    config: &Config,
    app_settings: &Arc<RwLock<AppSettings>>,
) {
    let Interaction::Command(command) = interaction else {
        return;
    };
    if !is_convert_command(command) {
        return;
    }

    // 1. Hedef içerik: sohbet komutunda `link`, mesaj komutunda hedef mesajın metni.
    let Some(source) = source(&command.data, &command.user) else {
        let content = match command.data.kind {
            CommandType::ChatInput => USAGE,
            _ => ConvertError::Unsupported.message(),
        };
        reply_ephemeral(ctx, command, content).await;
        return;
    };

    // 2. Üç saniyelik etkileşim penceresini kaçırmamak için dönüşümden **önce** ertelenir.
    //    Kullanıcı yalnızca kendisine görünen "düşünüyor" durumunu görür.
    if let Err(err) = command.defer_ephemeral(&ctx.http).await {
        warn!(interaction = %command.id, "Etkileşim ertelenemedi: {err:?}");
        return;
    }

    // 3. Kanal/thread kapsamlı ayarlar (otomatik mesaj akışıyla aynı çözümleme).
    let parent_id =
        crate::handler::discord_thread_parent(ctx, command.guild_id, command.channel_id).await;
    let settings = app_settings.read().await.get_message_settings(
        command.guild_id,
        command.channel_id,
        parent_id,
    );

    // Mesaj komutunda kart, bağlantıyı paylaşan kişiyi gösterir; rol filtresi ise komutu
    // kullanan kişiye uygulanır.
    let result = converters::convert_now(
        ctx,
        &source.content,
        &source.author,
        &command.user,
        command.member.as_deref(),
        command.guild_id,
        command.channel_id,
        source.reply_to,
        http_client,
        config,
        &settings,
    )
    .await;

    let content = match result {
        Ok(platform) => format!(
            "✅ {} bağlantısı karta dönüştürüldü.",
            platform.display_name()
        ),
        Err(err) => err.message().to_string(),
    };

    if let Err(err) = command
        .edit_response(&ctx.http, EditInteractionResponse::new().content(content))
        .await
    {
        error!(interaction = %command.id, "Etkileşim yanıtı güncellenemedi: {err:?}");
    }
}

/// Bu etkileşim bizim dönüştürme komutlarımızdan biri mi?
fn is_convert_command(command: &CommandInteraction) -> bool {
    match command.data.kind {
        CommandType::ChatInput => command.data.name == CONVERT_COMMAND,
        CommandType::Message => command.data.name == CONVERT_MESSAGE_COMMAND,
        _ => false,
    }
}

/// Komuttan dönüştürülecek metni ve kartta gösterilecek yazarı çıkarır.
struct Source {
    content: String,
    author: User,
    /// Mesaj komutunda hedef mesaj; sohbet komutunda `None`.
    reply_to: Option<MessageId>,
}

fn source(data: &CommandData, invoker: &User) -> Option<Source> {
    match data.kind {
        CommandType::ChatInput => {
            let link = string_option(data, LINK_OPTION)?;
            (!link.trim().is_empty()).then(|| Source {
                content: link.to_string(),
                author: invoker.clone(),
                reply_to: None,
            })
        }
        CommandType::Message => {
            let target_id = data.target_id?;
            let target = data
                .resolved
                .messages
                .get(&MessageId::new(target_id.get()))?;
            Some(Source {
                content: target.content.clone(),
                author: target.author.clone(),
                reply_to: Some(MessageId::new(target_id.get())),
            })
        }
        _ => None,
    }
}

/// Komut seçeneklerinden bir metin değeri okur.
fn string_option<'a>(data: &'a CommandData, name: &str) -> Option<&'a str> {
    data.options.iter().find_map(|option| {
        if option.name != name {
            return None;
        }
        match &option.value {
            CommandDataOptionValue::String(value) => Some(value.as_str()),
            _ => None,
        }
    })
}

/// Yalnızca komutu kullanan kişiye görünen kısa yanıt.
async fn reply_ephemeral(ctx: &Context, command: &CommandInteraction, content: &str) {
    let builder = CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new()
            .content(content)
            .ephemeral(true),
    );
    if let Err(err) = command.create_response(&ctx.http, builder).await {
        error!(interaction = %command.id, "Etkileşim yanıtı gönderilemedi: {err:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_names_are_discord_compatible() {
        assert!(CONVERT_COMMAND.len() <= 32);
        assert_eq!(CONVERT_COMMAND, CONVERT_COMMAND.to_lowercase());
        assert!(!CONVERT_COMMAND.contains(' '));
        assert!(CONVERT_MESSAGE_COMMAND.len() <= 32);

        let definitions = definitions();
        assert_eq!(definitions.len(), 2);
    }

    #[test]
    fn chat_input_link_option_is_extracted() {
        let data: CommandData = serde_json::from_value(serde_json::json!({
            "id": "1",
            "name": CONVERT_COMMAND,
            "type": 1,
            "options": [{ "name": LINK_OPTION, "type": 3, "value": "https://x.com/u/status/1" }]
        }))
        .expect("komut verisi çözümlenmeli");

        let source = source(&data, &User::default()).expect("link seçeneği bulunmalı");
        assert_eq!(source.content, "https://x.com/u/status/1");
    }

    #[test]
    fn blank_link_option_is_treated_as_missing() {
        let data: CommandData = serde_json::from_value(serde_json::json!({
            "id": "1",
            "name": CONVERT_COMMAND,
            "type": 1,
            "options": [{ "name": LINK_OPTION, "type": 3, "value": "   " }]
        }))
        .expect("komut verisi çözümlenmeli");

        assert!(source(&data, &User::default()).is_none());
    }

    #[test]
    fn message_command_uses_the_target_message() {
        use serenity::all::{ChannelId, Message};

        let mut target = Message::default();
        target.id = MessageId::new(42);
        target.channel_id = ChannelId::new(7);
        target.content = "bak https://x.com/u/status/1".to_string();
        target.author.id = serenity::all::UserId::new(99);

        let data: CommandData = serde_json::from_value(serde_json::json!({
            "id": "1",
            "name": CONVERT_MESSAGE_COMMAND,
            "type": 3,
            "target_id": "42",
            "resolved": { "messages": { "42": target } }
        }))
        .expect("mesaj komutu verisi çözümlenmeli");

        let source = source(&data, &User::default()).expect("hedef mesaj bulunmalı");
        assert_eq!(source.content, "bak https://x.com/u/status/1");
        assert_eq!(source.author.id.get(), 99);
        assert_eq!(source.reply_to, Some(MessageId::new(42)));
    }

    #[test]
    fn command_payloads_match_discord_schema() {
        let definitions = definitions();

        let chat = serde_json::to_value(&definitions[0]).expect("komut JSON'a çevrilmeli");
        assert_eq!(chat["name"], CONVERT_COMMAND);
        assert!(
            chat["description"]
                .as_str()
                .is_some_and(|text| !text.is_empty()),
            "sohbet komutu açıklama taşımalı"
        );
        assert_eq!(chat["options"][0]["name"], LINK_OPTION);
        assert_eq!(chat["options"][0]["type"], 3);

        let message = serde_json::to_value(&definitions[1]).expect("komut JSON'a çevrilmeli");
        assert_eq!(message["name"], CONVERT_MESSAGE_COMMAND);
        assert_eq!(message["type"], 3);
        assert!(
            message["options"].as_array().is_none_or(Vec::is_empty),
            "mesaj komutu seçenek taşımaz: {message}"
        );
    }

    #[test]
    fn usage_mentions_both_entry_points() {
        assert!(USAGE.contains("/dönüştür"));
        assert!(USAGE.contains("Apps"));
        assert!(USAGE.contains("link:"));
    }
}
