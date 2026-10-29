//! Komut etkileşimlerinin ortak giriş/çıkış noktası: kanca (hook) günlükleri, panik koruması ve
//! kullanıcıya dönen sabit hata metni.
//!
//! Her etkileşim ayrı bir görevde çalışır; görev panik ederse `JoinError` buradan okunur,
//! panik günlüğe yazılır, sayaç artar ve kullanıcıya "etkileşim başarısız" yerine kısa bir
//! gizli (ephemeral) uyarı gider.

use std::any::Any;
use std::time::Duration;

use serenity::all::{Context, Interaction};
use tracing::{debug, error, info, warn};

/// Panik durumunda kullanıcıya gösterilen sabit metin; panik ayrıntısı yalnızca loga gider.
pub const FAILURE_MESSAGE: &str = "Bir şeyler ters gitti; biraz sonra tekrar dene.";

/// Etkileşimin günlüklerde görünen kısa adı ve türü. Bileşen/modal kimliklerinde oyun durumu
/// bulunabileceği için yalnızca ilk parça (`fun`, `spotify`...) alınır.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub kind: &'static str,
    pub name: String,
}

/// Etkileşimin etiketi; komut, bileşen ve modal dışındakiler (ping, otomatik tamamlama) için `None`.
pub fn label(interaction: &Interaction) -> Option<Label> {
    Some(match interaction {
        Interaction::Command(command) => Label {
            kind: "command",
            name: command.data.name.clone(),
        },
        Interaction::Component(component) => Label {
            kind: "component",
            name: custom_id_head(&component.data.custom_id),
        },
        Interaction::Modal(modal) => Label {
            kind: "modal",
            name: custom_id_head(&modal.data.custom_id),
        },
        _ => return None,
    })
}

fn custom_id_head(custom_id: &str) -> String {
    custom_id.split(':').next().unwrap_or_default().to_string()
}

/// İşleyici sonucu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    Panicked(String),
    Cancelled,
}

/// Komuttan önce çalışır.
pub fn pre_command(label: &Label) {
    debug!(kind = label.kind, command = %label.name, "Etkileşim işleniyor");
}

/// Komuttan sonra, başarıyla ya da başarısız bitmesinden bağımsız çalışır.
pub fn post_command(label: &Label, outcome: &Outcome, elapsed: Duration) {
    let elapsed_ms = elapsed.as_millis() as u64;
    match outcome {
        Outcome::Completed => {
            info!(kind = label.kind, command = %label.name, elapsed_ms, "Etkileşim tamamlandı");
        }
        Outcome::Panicked(reason) => {
            crate::telemetry::COMMAND_PANICS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            error!(
                kind = label.kind, command = %label.name, elapsed_ms,
                "Etkileşim işleyicisi panik etti: {reason}"
            );
        }
        Outcome::Cancelled => {
            warn!(kind = label.kind, command = %label.name, elapsed_ms, "Etkileşim görevi iptal edildi");
        }
    }
}

/// Panik yükünü okunabilir metne çevirir.
pub fn panic_reason(payload: Box<dyn Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(text) => *text,
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(text) => (*text).to_string(),
            Err(_) => "bilinmeyen panik yükü".to_string(),
        },
    }
}

/// Kullanıcıya hatayı bildirir. Etkileşim henüz yanıtlanmadıysa ilk yanıt olarak, yanıtlandıysa
/// (ertelenmiş dahil) takip mesajı olarak gizli gider; ikisi de başarısızsa yalnızca loglanır.
pub async fn notify_failure(ctx: &Context, interaction: &Interaction) {
    let (id, token) = match interaction {
        Interaction::Command(command) => (command.id, command.token.as_str()),
        Interaction::Component(component) => (component.id, component.token.as_str()),
        Interaction::Modal(modal) => (modal.id, modal.token.as_str()),
        _ => return,
    };
    let initial = serde_json::json!({
        "type": 4,
        "data": { "content": FAILURE_MESSAGE, "flags": 64 },
    });
    if ctx
        .http
        .create_interaction_response(id, token, &initial, Vec::new())
        .await
        .is_ok()
    {
        return;
    }
    let followup = serde_json::json!({ "content": FAILURE_MESSAGE, "flags": 64 });
    if let Err(err) = ctx
        .http
        .create_followup_message(token, &followup, Vec::new())
        .await
    {
        warn!(interaction = %id, "Hata bildirimi gönderilemedi: {err:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_ids_keep_only_the_prefix() {
        assert_eq!(custom_id_head("fun:duel:123:456"), "fun");
        assert_eq!(custom_id_head("spotify"), "spotify");
        assert_eq!(custom_id_head(""), "");
    }

    #[test]
    fn panic_payloads_are_readable() {
        assert_eq!(panic_reason(Box::new("kötü")), "kötü");
        assert_eq!(panic_reason(Box::new(String::from("kötü2"))), "kötü2");
        assert_eq!(panic_reason(Box::new(42_u8)), "bilinmeyen panik yükü");
    }
}
