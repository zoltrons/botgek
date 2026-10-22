//! Bellek içi, sınırlı log halkası.
//!
//! MCP üzerinden son çalışma zamanı loglarını görebilmek için `tracing` olayları
//! yakalanır. Halka **süreç ömrüyle** sınırlıdır ve kapasiteye ulaşınca en eski girdi
//! düşer; diske yazılmaz, kullanıcı kimliği/URL/token ayrıştırması yapılmaz. Amaç,
//! üretimde `docker compose logs` gerektirmeden son hataları ve yavaşlık uyarılarını
//! görebilmektir.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

/// Halkada tutulan olay sayısı. Yaklaşık 512 satır, kart başına birkaç log satırıyla
/// son birkaç dakikayı kapsar; RSS etkisi ölçülü kalır.
pub const CAPACITY: usize = 512;
const MAX_MESSAGE_CHARS: usize = 4_096;
const MAX_FIELD_CHARS: usize = 1_024;

/// Tek bir yakalanmış log olayı.
#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    /// Süreç içi artan sıra numarası (yeniden başlatmada sıfırlanır).
    pub seq: u64,
    /// Unix zaman damgası (milisaniye).
    pub at_ms: u64,
    /// `trace` / `debug` / `info` / `warn` / `error`.
    pub level: &'static str,
    /// `tracing` hedefi (modül yolu).
    pub target: String,
    pub message: String,
    /// Yapılandırılmış alanlar (`url=…`, `capacity=…`); boşsa atlanır.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,
}

impl LogEntry {
    pub fn level_rank(&self) -> u8 {
        rank_of(self.level)
    }
}

static ENTRIES: LazyLock<Mutex<VecDeque<LogEntry>>> =
    LazyLock::new(|| Mutex::new(VecDeque::with_capacity(CAPACITY)));
static SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// `limit` en yeni girdiyi, `min_level` ve üzerini döner (en yeniden eskiye).
///
/// `limit` üst sınırı `CAPACITY`'dir; çağıran taraf daha büyük değer isterse kırpılır.
pub fn recent(limit: usize, min_level: &str) -> Vec<LogEntry> {
    let minimum = parse_level(min_level).unwrap_or(2);
    let limit = limit.clamp(1, CAPACITY);
    let entries = ENTRIES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    entries
        .iter()
        .rev()
        .filter(|entry| entry.level_rank() >= minimum)
        .take(limit)
        .cloned()
        .collect()
}

/// Halkadaki toplam olay sayısı (teşhis/test).
pub fn len() -> usize {
    ENTRIES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .len()
}

/// `tracing` katmanı: her olayın seviyesini, hedefini ve alanlarını halkaya yazar.
///
/// Bu katman `tracing_subscriber::registry()` içine eklenir (bkz. `main.rs`); kendi
/// içinde log üretmez, böylece geri besleme döngüsü oluşmaz.
#[derive(Debug, Default, Clone, Copy)]
pub struct CaptureLayer;

impl<S: Subscriber> Layer<S> for CaptureLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);

        let metadata = event.metadata();
        let entry = LogEntry {
            seq: SEQUENCE.fetch_add(1, Ordering::Relaxed),
            at_ms: now_ms(),
            level: level_name(*metadata.level()),
            target: metadata.target().to_string(),
            message: truncate(visitor.message.unwrap_or_default(), MAX_MESSAGE_CHARS),
            fields: visitor
                .fields
                .into_iter()
                .map(|(key, value)| (key, truncate(value, MAX_FIELD_CHARS)))
                .collect(),
        };

        let mut entries = ENTRIES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if entries.len() >= CAPACITY {
            entries.pop_front();
        }
        entries.push_back(entry);
    }
}

#[derive(Default)]
struct FieldVisitor {
    message: Option<String>,
    fields: BTreeMap<String, String>,
}

impl FieldVisitor {
    fn record(&mut self, field: &Field, value: String) {
        if field.name() == "message" {
            self.message = Some(value);
        } else {
            self.fields.insert(field.name().to_string(), value);
        }
    }
}

impl Visit for FieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record(field, format!("{value:?}"));
    }
}

fn level_name(level: Level) -> &'static str {
    match level {
        Level::TRACE => "trace",
        Level::DEBUG => "debug",
        Level::INFO => "info",
        Level::WARN => "warn",
        Level::ERROR => "error",
    }
}

/// Metin seviyesini sayısal sıraya çevirir (karşılaştırma için).
pub fn parse_level(raw: &str) -> Option<u8> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "trace" => Some(0),
        "debug" => Some(1),
        "info" => Some(2),
        "warn" | "warning" => Some(3),
        "error" => Some(4),
        _ => None,
    }
}

fn rank_of(level: &str) -> u8 {
    parse_level(level).unwrap_or(2)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or_default()
}

/// UTF-8 sınırlarını bozmadan karakter bazlı kırpar.
fn truncate(mut text: String, max_chars: usize) -> String {
    if let Some((end, _)) = text.char_indices().nth(max_chars) {
        text.truncate(end);
        text.push('…');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::info;
    use tracing_subscriber::layer::SubscriberExt;

    #[test]
    fn captures_message_and_fields_with_level_filtering() {
        let subscriber = tracing_subscriber::registry().with(CaptureLayer);
        tracing::subscriber::with_default(subscriber, || {
            info!(url = "https://example.test/post/1", "test-log-capture");
        });

        let entries = recent(CAPACITY, "info");
        let found = entries
            .iter()
            .find(|entry| entry.message == "test-log-capture")
            .expect("yakalanan log bulunmalı");
        assert_eq!(found.level, "info");
        assert_eq!(
            found.fields.get("url").map(String::as_str),
            Some("https://example.test/post/1")
        );

        assert!(
            recent(1, "error")
                .iter()
                .all(|entry| entry.level_rank() >= 4),
            "min_level filtresi error'dan aşağısını döndürmemeli"
        );
    }

    #[test]
    fn level_names_are_parsed_and_unknown_values_rejected() {
        assert_eq!(parse_level("WARN"), Some(3));
        assert_eq!(parse_level(" warning "), Some(3));
        assert_eq!(parse_level("belki"), None);
        assert!(rank_of("error") > rank_of("info"));
    }

    #[test]
    fn truncation_respects_utf8_boundaries() {
        let text = truncate("çok uzun bir metin".repeat(10), 5);
        assert_eq!(text.chars().count(), 6, "5 karakter + kırpma işareti");
    }

    #[test]
    fn ring_is_bounded() {
        let subscriber = tracing_subscriber::registry().with(CaptureLayer);
        tracing::subscriber::with_default(subscriber, || {
            for index in 0..(CAPACITY + 40) {
                info!(index, "bounded-ring-test");
            }
        });
        assert!(len() <= CAPACITY);
    }
}
