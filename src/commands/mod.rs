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
//! X çevirisi komutta varsayılan olarak açıktır; hedef dil WebUI ayarından gelir.
//!
//! Eğlence komutları (`/uyum`, `/düello`, `/sayı-tahmini`, `/8top`) [`fun`] modülündedir.

pub mod fun;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serenity::all::{
    Command, CommandData, CommandDataOptionValue, CommandInteraction, CommandOptionType,
    CommandType, ComponentInteraction, Context, CreateCommand, CreateCommandOption,
    CreateInteractionResponse, CreateInteractionResponseMessage, EditInteractionResponse, Http,
    InstallationContext, Interaction, InteractionContext, MessageId,
};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::converters::builtin::{raw, spotify};
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
        let commands = definitions()
            .into_iter()
            .chain(fun::definitions())
            .collect();
        match Command::set_global_commands(http.as_ref(), commands).await {
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
///
/// `integration_types` **açıkça** `GUILD_INSTALL + USER_INSTALL` olarak gönderilir. Portal'da
/// iki kurulum bağlamı da açık; alanı atlamak Discord API'sinde güvenilir biçimde user install
/// bağlamını eklemiyor ve komut kullanıcı hesabına kurulmuş uygulamada DM'lerde görünmüyordu.
///
/// `contexts` de açıkça gönderilir: `GUILD`, `BOT_DM`, `PRIVATE_CHANNEL`. Alan atlanınca
/// Discord komutu `contexts: null` ile saklıyor ve bu da DM'lerde görünmemesine yol açıyor.
fn definitions() -> Vec<CreateCommand> {
    vec![
        CreateCommand::new(CONVERT_COMMAND)
            .description("Bir sosyal medya bağlantısını dönüştürür (X, Instagram, Reddit, TikTok, Bluesky ve daha fazlası).")
            .add_integration_type(InstallationContext::Guild)
            .add_integration_type(InstallationContext::User)
            .add_context(InteractionContext::Guild)
            .add_context(InteractionContext::BotDm)
            .add_context(InteractionContext::PrivateChannel)
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
            // Mesaj komutu da DM/grup DM'de sağ tık → Apps menüsünde görünmeli: user install
            // açıkken hedef mesaj `resolved.messages` ile gelir. Yalnızca GUILD yazmak
            // komutu DM'lerdeki sağ tık menüsünden kaldırıyordu.
            .add_integration_type(InstallationContext::Guild)
            .add_integration_type(InstallationContext::User)
            .add_context(InteractionContext::Guild)
            .add_context(InteractionContext::BotDm)
            .add_context(InteractionContext::PrivateChannel)
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
    let command = match interaction {
        Interaction::Command(command) if fun::is_fun_command(command) => {
            return fun::handle_command(ctx, command).await;
        }
        Interaction::Component(component)
            if component.data.custom_id.starts_with(fun::CUSTOM_ID_PREFIX) =>
        {
            return fun::handle_component(ctx, component).await;
        }
        Interaction::Modal(modal) if modal.data.custom_id.starts_with(fun::CUSTOM_ID_PREFIX) => {
            return fun::handle_modal(ctx, modal).await;
        }
        Interaction::Component(component)
            if component
                .data
                .custom_id
                .starts_with(spotify::PAGE_CUSTOM_ID_PREFIX) =>
        {
            return spotify_page(ctx, component, http_client).await;
        }
        Interaction::Command(command) => command,
        _ => return,
    };
    if !is_convert_command(command) {
        return;
    }

    // 1. Hedef içerik: sohbet komutunda `link`, mesaj komutunda hedef mesajın metni.
    let Some(source) = source(&command.data) else {
        let content = match command.data.kind {
            CommandType::ChatInput => USAGE,
            _ => ConvertError::Unsupported.message(),
        };
        reply_ephemeral(ctx, command, content).await;
        return;
    };

    // 2. Üç saniyelik etkileşim penceresini kaçırmamak için dönüşümden **önce** ertelenir.
    //    DM/grup DM'de kart etkileşim yanıtının kendisi olacağı için yanıt herkese görünür
    //    (non-ephemeral) ertelenir; sunucu kanalında kart ayrı mesaj olarak gider.
    let private = command.guild_id.is_none();
    let deferred = if private {
        command.defer(&ctx.http).await
    } else {
        command.defer_ephemeral(&ctx.http).await
    };
    if let Err(err) = deferred {
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

    // DM/grup DM'de bot kanal üyesi değildir: kart normal kanal mesajı olarak gönderilemez,
    // etkileşim yanıtı düzenlenerek teslim edilir.
    let delivery = private.then(|| converters::builtin::InteractionDelivery {
        application_id: command.application_id.get(),
        token: command.token.clone(),
    });

    // Kart her iki akışta da komutu kullanan kişiyi gönderen gösterir; silme yetkisi de ona
    // kaydedilir (mesaj komutunda hedef mesajın yazarına değil).
    let result = converters::convert_now(
        ctx,
        &source.content,
        &command.user,
        command.member.as_deref(),
        command.guild_id,
        command.channel_id,
        source.reply_to,
        http_client,
        config,
        &settings,
        delivery.as_ref(),
    )
    .await;

    // Özel kanalda kart, ertelenen etkileşim yanıtının kendisine yazıldı; ayrıca onay
    // mesajı göndermek kartı ikinci kez göndermek olurdu.
    if private && result.is_ok() {
        return;
    }

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

/// Komuttan dönüştürülecek metni ve yanıtlanacak mesajı çıkarır.
struct Source {
    content: String,
    /// Mesaj komutunda hedef mesaj; sohbet komutunda `None`.
    reply_to: Option<MessageId>,
}

fn source(data: &CommandData) -> Option<Source> {
    match data.kind {
        CommandType::ChatInput => {
            let link = string_option(data, LINK_OPTION)?;
            (!link.trim().is_empty()).then(|| Source {
                content: link.to_string(),
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
/// Spotify kartındaki parça listesi sayfa butonu: mesaj yerinde güncellenir.
///
/// Önce "güncelleme ertelendi" (type 6) yanıtı verilir (embed yeniden çekilirken üç saniyelik
/// pencere kaçmasın), mesaj ham JSON olarak okunur, yalnızca liste + sayfa butonları değiştirilir
/// ve `@original` düzenlenir. Sayfa herkes için değişir; tıklayan kişiye yetki aranmaz.
async fn spotify_page(ctx: &Context, component: &ComponentInteraction, client: &reqwest::Client) {
    let deferred = ctx
        .http
        .create_interaction_response(
            component.id,
            &component.token,
            &serde_json::json!({ "type": 6 }),
            Vec::new(),
        )
        .await;
    if let Err(err) = deferred {
        warn!(interaction = %component.id, "Spotify sayfa etkileşimi ertelenemedi: {err:?}");
        return;
    }

    let application_id = component.application_id.get();
    let result = async {
        let message = raw::get_interaction_original(client, application_id, &component.token)
            .await
            .map_err(|err| format!("mesaj okunamadı: {err:?}"))?;
        let mut components = message
            .get("components")
            .cloned()
            .ok_or("mesajda bileşen yok")?;
        spotify::turn_page(client, &component.data.custom_id, &mut components).await?;
        let payload = serde_json::json!({
            "components": components,
            "allowed_mentions": { "parse": [] },
        });
        raw::edit_interaction_response(client, application_id, &component.token, &payload, &[])
            .await
            .map_err(|err| format!("mesaj düzenlenemedi: {err:?}"))
    }
    .await;

    if let Err(err) = result {
        warn!(interaction = %component.id, "Spotify sayfası değiştirilemedi: {err}");
        let body = serde_json::json!({
            "content": "Sayfa değiştirilemedi, biraz sonra tekrar dene.",
            "flags": 64,
        });
        let _ = ctx
            .http
            .create_followup_message(&component.token, &body, Vec::new())
            .await;
    }
}

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

        let source = source(&data).expect("link seçeneği bulunmalı");
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

        assert!(source(&data).is_none());
    }

    #[test]
    fn message_command_uses_the_target_message() {
        use serenity::all::{ChannelId, Message};

        let mut target = Message::default();
        target.id = MessageId::new(42);
        target.channel_id = ChannelId::new(7);
        target.content = "bak https://x.com/u/status/1".to_string();

        let data: CommandData = serde_json::from_value(serde_json::json!({
            "id": "1",
            "name": CONVERT_MESSAGE_COMMAND,
            "type": 3,
            "target_id": "42",
            "resolved": { "messages": { "42": target } }
        }))
        .expect("mesaj komutu verisi çözümlenmeli");

        let source = source(&data).expect("hedef mesaj bulunmalı");
        assert_eq!(source.content, "bak https://x.com/u/status/1");
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
    fn slash_command_is_available_in_guild_and_user_dm_contexts() {
        let definitions = definitions();
        let chat = serde_json::to_value(&definitions[0]).expect("komut JSON'a çevrilmeli");

        // contexts alanı atlanırsa Discord `null` saklıyor ve komut DM'lerde görünmüyor.
        assert_eq!(chat["contexts"], serde_json::json!([0, 1, 2]));
        // Kullanıcı hesabına kurulum (user install) açık: USER_INSTALL açıkça gönderilmeli,
        // aksi halde komut kullanıcı bağlamlarında/DM'lerde görünmüyor.
        assert_eq!(chat["integration_types"], serde_json::json!([0, 1]));

        let message = serde_json::to_value(&definitions[1]).expect("komut JSON'a çevrilmeli");
        // Mesaj komutu DM/grup DM'deki sağ tık menüsünde de görünmeli.
        assert_eq!(message["contexts"], serde_json::json!([0, 1, 2]));
        assert_eq!(message["integration_types"], serde_json::json!([0, 1]));
    }

    #[test]
    fn usage_mentions_both_entry_points() {
        assert!(USAGE.contains("/dönüştür"));
        assert!(USAGE.contains("Apps"));
        assert!(USAGE.contains("link:"));
    }
}
