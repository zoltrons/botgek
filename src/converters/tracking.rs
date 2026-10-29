//! Kaynak mesajın düzenlenmesi ve silinmesi: kartı kaynağıyla birlikte tutar.
//!
//! Kart, kullanıcının link attığı mesajdan üretilir ve `store` kaydında o mesajın kimliğiyle
//! eşlenir. Eşleme yalnızca orijinal mesaj **korunduğunda** (`DELETE_ORIGINAL=false`) tutulur;
//! orijinal bilerek siliniyorsa silme olayı kartı da götürmesin diye eşleme yazılmaz.
//!
//! - Kaynak silinirse kart da silinir.
//! - Kaynak düzenlenirse: bağlantı değişmediyse hiçbir şey yapılmaz; bağlantı değiştiyse eski
//!   kart silinir ve yeni bağlantı varsa normal akıştan yeniden dönüştürülür.

use std::sync::Arc;
use std::time::Duration;

use serenity::all::{Context, MessageId, MessageType, MessageUpdateEvent};
use tokio::sync::RwLock;
use tracing::{info, warn};

use super::parser;
use crate::config::Config;
use crate::settings::AppSettings;

/// Bu süreden eski mesajların düzenlemesi dikkate alınmaz: eski bir mesajı düzenleyip kanalın
/// ortasında yeni kart üretmek beklenmedik olurdu.
const EDIT_WINDOW: Duration = Duration::from_secs(3600);

/// Silinen mesajdan üretilmiş kart varsa onu da siler.
pub async fn source_deleted(ctx: &Context, source_id: MessageId) {
    let Some(card) = crate::store::blocking(move || crate::store::card_for_source(source_id)).await
    else {
        return;
    };
    delete_card(ctx, &card).await;
}

/// Toplu silmede (`message_delete_bulk`) her kaynak için [`source_deleted`] çalışır.
pub async fn sources_deleted(ctx: &Context, source_ids: &[MessageId]) {
    for id in source_ids {
        source_deleted(ctx, *id).await;
    }
}

async fn delete_card(ctx: &Context, card: &crate::store::SourceCard) {
    match card
        .channel_id
        .delete_message(&ctx.http, card.card_id)
        .await
    {
        Ok(()) => info!(card = %card.card_id, "Kaynak mesaj değişti; kart silindi"),
        // Kart zaten yok (🗑️ ile silinmiş olabilir): kayıt yine temizlenir.
        Err(err) if is_not_found(&err) => {}
        Err(err) => {
            warn!(card = %card.card_id, "Kart silinemedi: {err:?}");
            return;
        }
    }
    crate::converters::builtin::forget_author(card.card_id).await;
}

fn is_not_found(err: &serenity::Error) -> bool {
    matches!(
        err,
        serenity::Error::Http(serenity::all::HttpError::UnsuccessfulRequest(response))
            if response.status_code.as_u16() == 404
    )
}

/// Kaynak mesaj düzenlendiğinde kartı günceller (bkz. modül belgesi).
pub async fn source_edited(
    ctx: &Context,
    event: &MessageUpdateEvent,
    client: &reqwest::Client,
    config: &Config,
    settings: &Arc<RwLock<AppSettings>>,
) {
    // Gömme çözümleme gibi içerik değiştirmeyen güncellemelerde `edited_timestamp` yoktur;
    // yalnızca gerçek düzenlemeler işlenir (aksi halde ilk gönderim sırasında çift dönüşüm olurdu).
    let (Some(content), Some(edited_at)) = (event.content.as_deref(), event.edited_timestamp)
    else {
        return;
    };
    if event.author.as_ref().is_some_and(|author| author.bot)
        || matches!(event.webhook_id, Some(Some(_)))
        || event
            .kind
            .is_some_and(|kind| !matches!(kind, MessageType::Regular | MessageType::InlineReply))
    {
        return;
    }
    if too_old(event.id) {
        return;
    }

    let parsed = parser::parse_message(content);
    let id = event.id;
    let existing = crate::store::blocking(move || crate::store::card_for_source(id)).await;

    match (&existing, &parsed) {
        (None, None) => return,
        (Some(card), Some(parsed)) if card.source_url == parsed.original_url => return,
        _ => {}
    }
    info!(message = %event.id, edited_at = %edited_at, "Kaynak mesaj düzenlendi");

    if let Some(card) = &existing {
        delete_card(ctx, card).await;
    }
    if parsed.is_none() {
        return;
    }

    // Olay yalnızca değişen alanları taşır; dönüşüm hattı tam mesaj ister.
    let mut message = match event.channel_id.message(&ctx.http, event.id).await {
        Ok(message) => message,
        Err(err) => {
            warn!(message = %event.id, "Düzenlenen mesaj alınamadı: {err:?}");
            return;
        }
    };
    if message.author.bot || message.author.id == ctx.cache.current_user().id {
        return;
    }
    message.guild_id = message.guild_id.or(event.guild_id);
    if message.member.is_none() {
        message.member = event.member.clone().flatten();
    }

    let parent_id =
        crate::handler::discord_thread_parent(ctx, message.guild_id, message.channel_id).await;
    let message_settings =
        settings
            .read()
            .await
            .get_message_settings(message.guild_id, message.channel_id, parent_id);
    let _ = super::enqueue_message(ctx, &message, client, config, &message_settings).await;
}

/// Mesaj [`EDIT_WINDOW`]'dan eskiyse `true`.
fn too_old(message_id: MessageId) -> bool {
    let created_ms = message_id.created_at().unix_timestamp().max(0) as u64 * 1_000;
    crate::utils::unix_millis().saturating_sub(created_ms) > EDIT_WINDOW.as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_snowflake_is_inside_the_edit_window() {
        const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;
        let now = crate::utils::unix_millis();
        let fresh = MessageId::new((now - DISCORD_EPOCH_MS) << 22);
        assert!(!too_old(fresh));

        let old = MessageId::new((now - DISCORD_EPOCH_MS - 2 * 3_600_000) << 22);
        assert!(too_old(old));
    }
}
