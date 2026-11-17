//! `/letterboxd takip|bırak|liste`: kanalın takip ettiği Letterboxd hesaplarını yönetir.
//! Yeni günlük kayıtlarını kanala gönderen yoklama [`crate::letterboxd_feed`] içindedir.
//!
//! Komut yalnızca sunucu kanalında, botun üye olduğu sunucuda çalışır (kartı bot gönderir).
//! Varsayılan olarak Sunucuyu Yönet yetkisi ister; sunucu yöneticisi Discord'un entegrasyon
//! ayarından bunu değiştirebilir.

use serenity::all::{
    CommandDataOptionValue, CommandInteraction, CommandOptionType, CommandType, Context,
    CreateCommand, CreateCommandOption, EditInteractionResponse, InstallationContext,
    InteractionContext, Permissions,
};
use tracing::{debug, error, warn};

use super::{bot_is_in_channel, reply_ephemeral};
use crate::converters::builtin::letterboxd::{self, DiaryError};
use crate::converters::parser::letterboxd_profile_user;
use crate::store::{self, FollowOutcome};

pub const LETTERBOXD_COMMAND: &str = "letterboxd";
const FOLLOW: &str = "takip";
const UNFOLLOW: &str = "bırak";
const LIST: &str = "liste";
const USER_OPTION: &str = "kullanıcı";
/// Sunucu başına takip sınırı: yoklama turunun uzunluğunu sınırlar.
const MAX_FOLLOWS_PER_GUILD: i64 = 50;

pub fn definitions() -> Vec<CreateCommand> {
    let user = || {
        CreateCommandOption::new(
            CommandOptionType::String,
            USER_OPTION,
            "Letterboxd kullanıcı adı ya da profil bağlantısı",
        )
        .required(true)
        .max_length(200)
    };
    vec![
        CreateCommand::new(LETTERBOXD_COMMAND)
            .description("Letterboxd hesaplarının izlediklerini bu kanala gönderir.")
            .default_member_permissions(Permissions::MANAGE_GUILD)
            .add_integration_type(InstallationContext::Guild)
            .add_context(InteractionContext::Guild)
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    FOLLOW,
                    "Hesabın yeni izlediklerini bu kanala gönderir",
                )
                .add_sub_option(user()),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    UNFOLLOW,
                    "Hesabın bu kanaldaki takibini bırakır",
                )
                .add_sub_option(user()),
            )
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                LIST,
                "Bu kanalda takip edilen hesapları listeler",
            )),
    ]
}

pub fn is_letterboxd_command(command: &CommandInteraction) -> bool {
    command.data.kind == CommandType::ChatInput && command.data.name == LETTERBOXD_COMMAND
}

pub async fn handle(ctx: &Context, command: &CommandInteraction, http_client: &reqwest::Client) {
    let Some(guild_id) = command.guild_id.filter(|_| bot_is_in_channel(command)) else {
        reply_ephemeral(
            ctx,
            command,
            "Letterboxd takibi yalnızca botun ekli olduğu sunucu kanallarında çalışır.",
        )
        .await;
        return;
    };
    let Some(sub) = command.data.options.first() else {
        return;
    };
    let raw_user = match &sub.value {
        CommandDataOptionValue::SubCommand(options) => options
            .iter()
            .find(|option| option.name == USER_OPTION)
            .and_then(|option| option.value.as_str()),
        _ => None,
    };
    // Takip, hesabı doğrulamak için akışı çeker: üç saniyelik pencere aşılabilir.
    if let Err(err) = command.defer_ephemeral(&ctx.http).await {
        warn!(interaction = %command.id, "Etkileşim ertelenemedi: {err:?}");
        return;
    }
    let channel_id = command.channel_id;

    let content = match (sub.name.as_str(), raw_user.map(username)) {
        (LIST, _) => {
            let follows =
                store::blocking(move || store::letterboxd_follows(Some(channel_id))).await;
            if follows.is_empty() {
                "Bu kanal henüz bir Letterboxd hesabı takip etmiyor.".to_string()
            } else {
                let names: Vec<String> = follows
                    .iter()
                    .map(|f| format!("`{}`", f.username))
                    .collect();
                format!("Bu kanalda takip edilenler: {}", names.join(", "))
            }
        }
        (_, Some(None)) => {
            "Geçerli bir Letterboxd kullanıcı adı ya da profil bağlantısı ver.".to_string()
        }
        (FOLLOW, Some(Some(user))) => match letterboxd::diary(http_client, &user).await {
            Ok(entries) => {
                // Var olan kayıtlar gönderilmez; yalnızca bundan sonrakiler gelir.
                let last = entries.iter().map(|entry| entry.id).max().unwrap_or(0);
                let name = user.clone();
                let outcome = store::blocking(move || {
                    store::follow_letterboxd(
                        guild_id,
                        channel_id,
                        &name,
                        last,
                        MAX_FOLLOWS_PER_GUILD,
                    )
                })
                .await;
                match outcome {
                    FollowOutcome::Added => {
                        format!("✅ `{user}` takip ediliyor. Yeni izledikleri bu kanala gelecek.")
                    }
                    FollowOutcome::Exists => {
                        format!("Bu kanal `{user}` hesabını zaten takip ediyor.")
                    }
                    FollowOutcome::Full => format!(
                        "Bu sunucu en fazla {MAX_FOLLOWS_PER_GUILD} Letterboxd hesabı takip edebilir."
                    ),
                    FollowOutcome::Failed => {
                        "Takip kaydedilemedi, biraz sonra yeniden dene.".to_string()
                    }
                }
            }
            Err(DiaryError::NotFound) => format!("`{user}` adlı bir Letterboxd hesabı bulunamadı."),
            Err(err) => {
                debug!("Letterboxd akışı doğrulanamadı ({user}): {err:?}");
                "Letterboxd şu an yanıt vermiyor, biraz sonra yeniden dene.".to_string()
            }
        },
        (UNFOLLOW, Some(Some(user))) => {
            let name = user.clone();
            if store::blocking(move || store::unfollow_letterboxd(channel_id, &name)).await {
                format!("`{user}` artık bu kanalda takip edilmiyor.")
            } else {
                format!("Bu kanal `{user}` hesabını takip etmiyor.")
            }
        }
        _ => return,
    };

    if let Err(err) = command
        .edit_response(&ctx.http, EditInteractionResponse::new().content(content))
        .await
    {
        error!(interaction = %command.id, "Etkileşim yanıtı güncellenemedi: {err:?}");
    }
}

/// Kullanıcı adı ya da profil bağlantısı → küçük harfli Letterboxd kullanıcı adı.
fn username(raw: &str) -> Option<String> {
    let raw = raw.trim().trim_start_matches('@');
    let url = if raw.starts_with("http://") || raw.starts_with("https://") {
        raw.to_string()
    } else if raw.contains("letterboxd.com/") {
        format!("https://{raw}")
    } else {
        format!("https://letterboxd.com/{raw}/")
    };
    letterboxd_profile_user(&url).map(|user| user.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_accepts_names_and_profile_links() {
        assert_eq!(
            username("  @DavidEhrlich ").as_deref(),
            Some("davidehrlich")
        );
        assert_eq!(
            username("https://letterboxd.com/davidehrlich/").as_deref(),
            Some("davidehrlich")
        );
        assert_eq!(username("letterboxd.com/ana_b").as_deref(), Some("ana_b"));
        assert_eq!(username("film/parasite"), None);
        assert_eq!(username("films"), None);
        assert_eq!(username("a b"), None);
    }

    #[test]
    fn command_is_guild_only_with_three_subcommands() {
        let json = serde_json::to_value(&definitions()[0]).expect("komut JSON'a çevrilmeli");
        assert_eq!(json["contexts"], serde_json::json!([0]));
        assert_eq!(json["integration_types"], serde_json::json!([0]));
        assert_eq!(json["options"].as_array().map(Vec::len), Some(3));
        assert_eq!(json["options"][0]["options"][0]["name"], USER_OPTION);
    }
}
