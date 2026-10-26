//! `/gif-yap` ve mesaja sağ tık → **Apps → GIF yap**: X gönderisindeki videoyu (veya GIF'i)
//! FxEmbed'in GIF servisine (`gif.fxtwitter.com`) yönlendiren bağlantıyı gönderir.
//!
//! vxTwitter gifconvert yalnızca X GIF'lerini (`tweet_video`) dönüştürür; gerçek videolar
//! orijinal mp4'e geri yönlendirilir. FxEmbed servisi videoları da GIF'e çevirdiği için bu
//! komut onu kullanır (bkz. [`media::fx_gif_url`]).
//!
//! Hedef gönderi şu sırayla bulunur:
//! - `/gif-yap link:<bağlantı>`: verilen bağlantı.
//! - Linksiz `/gif-yap`: kanaldaki en yeni X bağlantısı ya da bot kartı. Discord slash
//!   komutuna yanıtlanan mesajı eklemediği için "karta yanıt verip komutu yazmak" bu yolla
//!   karşılanır; mesaj geçmişi okunamazsa (bot kanalda değilse) kullanım gösterilir.
//! - Mesaj komutu: hedef mesajın metni; bot kartında bağlantı Components V2 metnindedir ve
//!   Serenity modelinde korunmadığından mesaj REST üzerinden ham JSON olarak okunur.

use serenity::all::{
    CommandDataOptionValue, CommandInteraction, CommandOptionType, CommandType, Context,
    CreateCommand, CreateCommandOption, CreateInteractionResponseFollowup, EditInteractionResponse,
    InstallationContext, InteractionContext, MessageId,
};
use tracing::{error, warn};

use crate::config::Config;
use crate::converters::builtin::{raw, twitter};

/// Sohbet girdisi komutunun adı.
pub const GIF_COMMAND: &str = "gif-yap";
/// Mesaja sağ tıklandığında görünen komutun adı.
pub const GIF_MESSAGE_COMMAND: &str = "GIF yap";
const LINK_OPTION: &str = "link";
/// Linksiz komutta geriye doğru taranan mesaj sayısı.
const HISTORY_LIMIT: u8 = 25;

const USAGE: &str = "Videolu bir X gönderisi bulamadım. Bağlantı ver: \
`/gif-yap link:https://x.com/kullanici/status/123`\n\
Ya da karta/mesaja sağ tık → **Apps → GIF yap**.";
const NO_VIDEO: &str = "Bu gönderide video ya da GIF yok.";

pub fn definitions() -> Vec<CreateCommand> {
    vec![
        everywhere(CreateCommand::new(GIF_COMMAND))
            .description(
                "X gönderisindeki videoyu GIF'e çevirir (link yoksa kanaldaki son gönderi).",
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    LINK_OPTION,
                    "X gönderi bağlantısı (boşsa kanaldaki son X gönderisi/kartı kullanılır)",
                )
                .required(false)
                .max_length(2048),
            ),
        // Discord, MESSAGE komutlarında açıklamayı boş string olarak bekler.
        everywhere(CreateCommand::new(GIF_MESSAGE_COMMAND))
            .kind(CommandType::Message)
            .description(""),
    ]
}

fn everywhere(command: CreateCommand) -> CreateCommand {
    command
        .add_integration_type(InstallationContext::Guild)
        .add_integration_type(InstallationContext::User)
        .add_context(InteractionContext::Guild)
        .add_context(InteractionContext::BotDm)
        .add_context(InteractionContext::PrivateChannel)
}

/// Bu komut etkileşimi GIF komutlarından biri mi?
pub fn is_gif_command(command: &CommandInteraction) -> bool {
    match command.data.kind {
        CommandType::ChatInput => command.data.name == GIF_COMMAND,
        CommandType::Message => command.data.name == GIF_MESSAGE_COMMAND,
        _ => false,
    }
}

pub async fn handle(
    ctx: &Context,
    command: &CommandInteraction,
    http_client: &reqwest::Client,
    config: &Config,
) {
    // Kanal geçmişi ve FxTwitter isteği üç saniyelik pencereyi aşabilir. GIF bağlantısı
    // herkesin göreceği mesaj olarak gider; hata olursa bu yanıt silinip gizli mesaj atılır.
    if let Err(err) = command.defer(&ctx.http).await {
        warn!(interaction = %command.id, "Etkileşim ertelenemedi: {err:?}");
        return;
    }

    let result = match find_tweet(command, http_client, config).await {
        Some(url) => match twitter::gif_links(http_client, &url).await {
            Ok(links) if links.is_empty() => Err(NO_VIDEO.to_string()),
            Ok(links) => Ok(links.join("\n")),
            Err(err) => {
                warn!(%url, "GIF için gönderi okunamadı: {err}");
                Err("Gönderi okunamadı; silinmiş ya da gizli olabilir.".to_string())
            }
        },
        None => Err(USAGE.to_string()),
    };

    match result {
        Ok(content) => {
            if let Err(err) = command
                .edit_response(&ctx.http, EditInteractionResponse::new().content(content))
                .await
            {
                error!(interaction = %command.id, "GIF yanıtı gönderilemedi: {err:?}");
            }
        }
        Err(message) => {
            let _ = command.delete_response(&ctx.http).await;
            let followup = CreateInteractionResponseFollowup::new()
                .content(message)
                .ephemeral(true);
            if let Err(err) = command.create_followup(&ctx.http, followup).await {
                error!(interaction = %command.id, "GIF hata yanıtı gönderilemedi: {err:?}");
            }
        }
    }
}

/// Komutun hedeflediği tweet bağlantısı (bkz. modül belgesi).
async fn find_tweet(
    command: &CommandInteraction,
    http_client: &reqwest::Client,
    config: &Config,
) -> Option<String> {
    let data = &command.data;
    match data.kind {
        CommandType::ChatInput => {
            if let Some(link) = link_option(command).filter(|link| !link.trim().is_empty()) {
                return twitter::find_tweet_url(link);
            }
            let history = raw::get_channel_messages(
                http_client,
                &config.discord_token,
                command.channel_id,
                None,
                HISTORY_LIMIT,
            )
            .await
            .inspect_err(|err| warn!("Kanal geçmişi okunamadı: {err:?}"))
            .ok()?;
            // Discord en yeni mesajı ilk sırada döner.
            history.as_array()?.iter().find_map(message_tweet_url)
        }
        CommandType::Message => {
            let target_id = MessageId::new(data.target_id?.get());
            let target = data.resolved.messages.get(&target_id)?;
            if let Some(url) = serde_json::to_value(target)
                .ok()
                .as_ref()
                .and_then(message_tweet_url)
            {
                return Some(url);
            }
            let message = raw::get_channel_messages(
                http_client,
                &config.discord_token,
                command.channel_id,
                Some(target_id),
                1,
            )
            .await
            .inspect_err(|err| warn!("Hedef mesaj okunamadı: {err:?}"))
            .ok()?;
            message_tweet_url(&message)
        }
        _ => None,
    }
}

/// Ham mesaj JSON'undaki (metin, embed, Components V2 metni) tweet bağlantısı.
fn message_tweet_url(message: &serde_json::Value) -> Option<String> {
    twitter::find_tweet_url(&message.to_string())
}

fn link_option(command: &CommandInteraction) -> Option<&str> {
    command.data.options.iter().find_map(|option| {
        match (&option.value, option.name == LINK_OPTION) {
            (CommandDataOptionValue::String(value), true) => Some(value.as_str()),
            _ => None,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_json_resolves_to_the_posts_own_link_not_the_quote() {
        let card = serde_json::json!({
            "content": "",
            "components": [{"type": 17, "components": [
                {"type": 10, "content": "> [Quote](https://x.com/q/status/1) from **Q**"},
                {"type": 10, "content": "[Link](https://x.com/main/status/2) · Gönderen: <@3>"}
            ]}]
        });
        assert_eq!(
            message_tweet_url(&card).as_deref(),
            Some("https://x.com/main/status/2")
        );
    }

    #[test]
    fn plain_messages_and_mirror_domains_are_recognised() {
        let message = serde_json::json!({"content": "bak https://fxtwitter.com/u/status/9?s=20"});
        assert_eq!(
            message_tweet_url(&message).as_deref(),
            Some("https://x.com/u/status/9")
        );
        let gif_reply =
            serde_json::json!({"content": "https://gif.fxtwitter.com/tweet_video/a.gif"});
        assert!(message_tweet_url(&gif_reply).is_none());
    }

    #[test]
    fn definitions_are_available_everywhere() {
        let definitions = definitions();
        let chat = serde_json::to_value(&definitions[0]).expect("komut JSON'a çevrilmeli");
        assert_eq!(chat["name"], GIF_COMMAND);
        assert_eq!(chat["options"][0]["name"], LINK_OPTION);
        assert_eq!(chat["contexts"], serde_json::json!([0, 1, 2]));
        assert_eq!(chat["integration_types"], serde_json::json!([0, 1]));

        let message = serde_json::to_value(&definitions[1]).expect("komut JSON'a çevrilmeli");
        assert_eq!(message["name"], GIF_MESSAGE_COMMAND);
        assert_eq!(message["type"], 3);
    }
}
