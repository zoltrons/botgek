//! Kalıcı kayıt deposu (SQLite).
//!
//! Bot, dönüştürdüğü her kart için **hangi mesajı hangi kullanıcının paylaştığını** saklar.
//! Bu kayıt üç yerde kullanılır:
//!
//! - 🗑️ **Reaksiyonla silme izni:** kartı paylaşan kullanıcı (taklit webhook'unda mesajın
//!   yazarı bot/kullanıcı olmadığı için) bu kayıtla bulunur.
//! - 🛡️ **AI koruması:** kartlara verilen yanıtlarda AI sohbeti tetiklenmez.
//! - 🇹🇷 **Çeviri:** bayrak reaksiyonunun kart mı proxy mesajı mı olduğunun ayrımı.
//!
//! Kayıtlar **dosyada** tutulur; bot yeniden başladığında, çöktüğünde veya güncellendiğinde
//! hiçbir kayıt kaybolmaz (eskiden bu bilgi bellekteki bir `HashMap` içindeydi ve yeniden
//! başlatmada siliniyordu).
//!
//! Dosya yolu `BOTGEK_DB_PATH` ortam değişkeniyle verilir; tanımlı değilse çalışma
//! dizinindeki `botgek.db` kullanılır (systemd: `/var/lib/botgek/botgek.db`,
//! Docker: `BOTGEK_DB_PATH=/app/data/botgek.db` — kalıcı `botgek-data` volume'u).
//! Testler bellek içi veritabanında (`:memory:`) çalışır, gerçek dosyaya dokunmaz.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use rusqlite::{Connection, OptionalExtension, params};
use serenity::all::{ChannelId, MessageId, UserId};
use tracing::{info, warn};

/// Kayıt dosyasının varsayılan adı (çalışma dizinine göre).
pub const DEFAULT_DB_FILE: &str = "botgek.db";
/// Bu sayıdan fazla kayıt birikirse **en eski** kayıtlar budanır: mesaj kimlikleri zaman
/// sıralı Snowflake olduğu için en büyük kimlikler en yeni kartlardır. Böylece dosya
/// sınırsız büyümez, ama pratikte hiçbir kayıt unutulmaz.
const MAX_RECORDS: i64 = 200_000;
/// Budama kontrolü her bu kadar kayıtta bir yapılır (her yazmada sorgu çalıştırmamak için).
const PRUNE_EVERY: u64 = 512;

/// Tek paylaşılan veritabanı bağlantısı.
static DB: Lazy<Db> = Lazy::new(|| Db::open(&default_path()));
/// Son budamadan bu yana yapılan yazma sayısı.
static WRITES: AtomicU64 = AtomicU64::new(0);

/// SQLite veritabanı sarmalayıcısı. Sorgular tek satır (birincil anahtar) üzerinde
/// çalıştığı için çağrılar mikrosaniye düzeyindedir; bu yüzden API senkron tutulur.
pub struct Db {
    connection: Mutex<Connection>,
    path: String,
}

impl Db {
    /// Veritabanını açar, şemayı kurar ve performans pragmalarını uygular.
    /// Dosya açılamazsa (izin/yol hatası) bot çalışmaya devam etsin diye bellek içi
    /// veritabanına düşülür ve uyarı log'lanır.
    pub fn open(path: &str) -> Self {
        let (connection, path) = match Self::connect(path) {
            Ok(connection) => (connection, path.to_string()),
            Err(err) => {
                warn!("Kayıt veritabanı açılamadı ({}): {} — bellekte çalışılacak", path, err);
                let connection =
                    Self::connect(":memory:").expect("bellek içi veritabanı her zaman açılmalı");
                (connection, ":memory:".to_string())
            }
        };

        Self {
            connection: Mutex::new(connection),
            path,
        }
    }

    fn connect(path: &str) -> Result<Connection, rusqlite::Error> {
        let connection = Connection::open(path)?;
        // WAL: okuma/yazma paralel çalışır ve ani kapanmada veri kaybolmaz.
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS cards (
                 message_id INTEGER PRIMARY KEY,
                 channel_id INTEGER NOT NULL,
                 author_id  INTEGER NOT NULL,
                 created_at INTEGER NOT NULL
             );",
        )?;
        Ok(connection)
    }

    /// Kayıt dosyasının yolu (`:memory:` ise bellek içi).
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Kart kaydını yazar (aynı mesaj kimliği yeniden yazılırsa güncellenir).
    pub fn remember(&self, message_id: MessageId, channel_id: ChannelId, author_id: UserId) {
        let created_at = now_unix();
        let result = self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO cards (message_id, channel_id, author_id, created_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(message_id) DO UPDATE SET
                     channel_id = excluded.channel_id,
                     author_id  = excluded.author_id",
                params![
                    message_id.get() as i64,
                    channel_id.get() as i64,
                    author_id.get() as i64,
                    created_at
                ],
            )
            .map(|_| ())
        });

        if let Err(err) = result {
            warn!("Kart kaydı yazılamadı ({}): {}", message_id, err);
        }
    }

    /// Kaydı siler (kart Discord'dan silindiğinde).
    pub fn forget(&self, message_id: MessageId) {
        let result = self.with_connection(|connection| {
            connection
                .execute(
                    "DELETE FROM cards WHERE message_id = ?1",
                    params![message_id.get() as i64],
                )
                .map(|_| ())
        });

        if let Err(err) = result {
            warn!("Kart kaydı silinemedi ({}): {}", message_id, err);
        }
    }

    /// Kartı paylaşan kullanıcı (kayıt yoksa `None`).
    pub fn author(&self, message_id: MessageId) -> Option<UserId> {
        self.query_one("SELECT author_id FROM cards WHERE message_id = ?1", message_id)
            .and_then(|value| u64::try_from(value).ok())
            .map(UserId::new)
    }

    /// Bu mesaj için kayıt var mı? (kart tanıma)
    pub fn contains(&self, message_id: MessageId) -> bool {
        self.query_one("SELECT 1 FROM cards WHERE message_id = ?1", message_id)
            .is_some()
    }

    /// Toplam kayıt sayısı (log/teşhis).
    pub fn count(&self) -> i64 {
        self.with_connection(|connection| {
            connection.query_row("SELECT COUNT(*) FROM cards", [], |row| row.get::<_, i64>(0))
        })
        .unwrap_or(0)
    }

    /// En yeni `keep` kaydı bırakır, daha eskilerini siler. Kayıt sayısı `keep`'ten azsa
    /// hiçbir şey silinmez. Silinen kayıt sayısını döner.
    pub fn prune(&self, keep: i64) -> usize {
        self.with_connection(|connection| {
            connection.execute(
                "DELETE FROM cards
                  WHERE message_id < (SELECT message_id FROM cards ORDER BY message_id DESC LIMIT 1 OFFSET ?1)",
                params![keep - 1],
            )
        })
        .unwrap_or(0)
    }

    fn query_one(&self, sql: &str, message_id: MessageId) -> Option<i64> {
        let result = self.with_connection(|connection| {
            connection
                .query_row(sql, params![message_id.get() as i64], |row| row.get::<_, i64>(0))
                .optional()
        });

        match result {
            Ok(value) => value,
            Err(err) => {
                warn!("Kart kaydı okunamadı ({}): {}", message_id, err);
                None
            }
        }
    }

    /// Kilitli bağlantı üzerinde tek bir sorgu çalıştırır.
    fn with_connection<T>(
        &self,
        query: impl FnOnce(&Connection) -> Result<T, rusqlite::Error>,
    ) -> Result<T, String> {
        let connection = match self.connection.lock() {
            Ok(connection) => connection,
            Err(err) => return Err(format!("kilit alınamadı: {err}")),
        };
        query(&connection).map_err(|err| err.to_string())
    }
}

/// Kart kaydı (mesaj → paylaşan kullanıcı) yazar. Her yazmada hafif bir sayaç artar;
/// sayaç eşiğe gelince en eski kayıtlar budanır.
pub fn remember_card(message_id: MessageId, channel_id: ChannelId, author_id: UserId) {
    DB.remember(message_id, channel_id, author_id);

    if WRITES.fetch_add(1, Ordering::Relaxed) + 1 >= PRUNE_EVERY {
        WRITES.store(0, Ordering::Relaxed);
        prune_db(&DB);
    }
}

/// Kaydı siler (mesaj Discord'dan silindiğinde).
pub fn forget_card(message_id: MessageId) {
    DB.forget(message_id);
}

/// Kartı paylaşan kullanıcı.
pub fn card_author(message_id: MessageId) -> Option<UserId> {
    DB.author(message_id)
}

/// Bu mesaj için kayıt var mı?
pub fn is_card(message_id: MessageId) -> bool {
    DB.contains(message_id)
}

/// Kayıt veritabanını hazırlar (şema + budama) ve özet bilgiyi log'lar.
pub fn init() {
    info!(
        "Kayıt veritabanı hazır: {} ({} kart kaydı)",
        DB.path(),
        DB.count()
    );
    prune_db(&DB);
}

/// En eski kayıtları budar (kayıt sayısı sınırı aşılmadıysa hiçbir şey silinmez).
fn prune_db(db: &Db) {
    let removed = db.prune(MAX_RECORDS);
    if removed > 0 {
        info!("Kayıt veritabanı budandı: {} eski kart kaydı silindi", removed);
    }
}

/// Veritabanı yolu: `BOTGEK_DB_PATH` → çalışma dizinindeki `botgek.db`.
/// Testler bellek içi veritabanı kullanır (gerçek dosya kirlenmez, kayıt testler arası sızmaz).
fn default_path() -> String {
    if cfg!(test) {
        return ":memory:".to_string();
    }

    std::env::var("BOTGEK_DB_PATH")
        .map(|path| path.trim().to_string())
        .ok()
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| DEFAULT_DB_FILE.to_string())
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

/// Geçici test dosyası yolu (testler bitince `cleanup` ile silinir).
#[cfg(test)]
fn test_db_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("botgek-store-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("geçici dizin");
    dir.join(format!("{name}.db"))
}

#[cfg(test)]
fn cleanup(path: &std::path::Path) {
    for suffix in ["", "-wal", "-shm"] {
        let candidate = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
        if candidate.exists() {
            let _ = std::fs::remove_file(candidate);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_survive_reopening_the_file() {
        let path = test_db_path("persistence");
        cleanup(&path);
        let path_str = path.display().to_string();

        {
            let db = Db::open(&path_str);
            db.remember(MessageId::new(1_000), ChannelId::new(9), UserId::new(42));
            assert_eq!(db.count(), 1);
        }

        // Bot yeniden başlatıldı: yeni bağlantı aynı kaydı görmeli.
        let reopened = Db::open(&path_str);
        assert_eq!(reopened.author(MessageId::new(1_000)), Some(UserId::new(42)));
        assert!(reopened.contains(MessageId::new(1_000)));
        assert_eq!(reopened.author(MessageId::new(2_000)), None);
        assert!(!reopened.contains(MessageId::new(2_000)));

        cleanup(&path);
    }

    #[test]
    fn forgetting_a_card_removes_only_that_record() {
        let path = test_db_path("forget");
        cleanup(&path);
        let db = Db::open(&path.display().to_string());

        db.remember(MessageId::new(11), ChannelId::new(1), UserId::new(2));
        db.remember(MessageId::new(12), ChannelId::new(1), UserId::new(3));
        db.forget(MessageId::new(11));

        assert!(!db.contains(MessageId::new(11)));
        assert_eq!(db.author(MessageId::new(12)), Some(UserId::new(3)));
        assert_eq!(db.count(), 1);

        cleanup(&path);
    }

    #[test]
    fn pruning_keeps_the_newest_records() {
        let path = test_db_path("prune");
        cleanup(&path);
        let db = Db::open(&path.display().to_string());

        for id in 1..=5u64 {
            db.remember(MessageId::new(id), ChannelId::new(1), UserId::new(7));
        }
        assert_eq!(db.prune(3), 2, "en eski iki kayıt silinmeli");

        assert!(!db.contains(MessageId::new(1)));
        assert!(!db.contains(MessageId::new(2)));
        for id in 3..=5u64 {
            assert!(db.contains(MessageId::new(id)), "{id} kalmalı");
        }
        // Sınırdan az kayıt varsa hiçbir şey silinmez.
        assert_eq!(db.prune(10), 0);

        cleanup(&path);
    }

    #[test]
    fn unknown_paths_fall_back_to_memory_instead_of_failing() {
        let db = Db::open("/olmayan-dizin/veritabani.db");
        assert_eq!(db.path(), ":memory:");
        db.remember(MessageId::new(5), ChannelId::new(1), UserId::new(2));
        assert!(db.contains(MessageId::new(5)), "bellek içi veritabanı çalışmalı");
    }
}
