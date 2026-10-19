//! Kalıcı kayıt deposu (SQLite).
//!
//! Bot, dönüştürdüğü her kart için **hangi mesajı hangi kullanıcının paylaştığını** saklar.
//! Bu kayıt üç yerde kullanılır:
//!
//! - **Sil butonunun izni:** kartı paylaşan kullanıcı (taklit webhook'unda mesajın
//!   yazarı bot olmadığı için) bu kayıtla bulunur.
//! - 🛡️ **AI koruması:** kartlara verilen yanıtlarda AI sohbeti tetiklenmez.
//! - 🇹🇷 **Çeviri:** bayrak reaksiyonunun kart mı proxy mesajı mı olduğunun ayrımı.
//!
//! Kayıtlar **dosyada** tutulur; bot yeniden başladığında, çöktüğünde veya güncellendiğinde
//! hiçbir kayıt kaybolmaz (eskiden bu bilgi bellekteki bir `HashMap` içindeydi ve yeniden
//! başlatmada siliniyordu).
//!
//! Dosya yolu `BOTGEK_DB_PATH` ortam değişkeniyle verilir; tanımlı değilse çalışma
//! dizinindeki `botgek.db` kullanılır (Docker: `BOTGEK_DB_PATH=/app/data/botgek.db` — kalıcı `botgek-data` volume'u).
//! Testler bellek içi veritabanında (`:memory:`) çalışır, gerçek dosyaya dokunmaz.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use serenity::all::{ChannelId, MessageId, UserId};
use std::sync::LazyLock;
use tracing::{error, info, warn};

/// Kayıt dosyasının varsayılan adı (çalışma dizinine göre).
pub const DEFAULT_DB_FILE: &str = "botgek.db";
/// Bu sayıdan fazla kayıt birikirse **en eski** kayıtlar budanır: mesaj kimlikleri zaman
/// sıralı Snowflake olduğu için en büyük kimlikler en yeni kartlardır. Böylece dosya
/// sınırsız büyümez, ama pratikte hiçbir kayıt unutulmaz.
const MAX_RECORDS: i64 = 200_000;
/// Budama kontrolü her bu kadar kayıtta bir yapılır (her yazmada sorgu çalıştırmamak için).
const PRUNE_EVERY: u64 = 512;

/// Controls whether opening a database may fall back to an in-memory store.
/// Production startup uses [`PersistencePolicy::RequirePersistent`]; allowing
/// the fallback remains useful for isolated tests and explicit local tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistencePolicy {
    RequirePersistent,
    AllowEphemeral,
}

/// Başlatma politikası: üretimde kalıcılık **zorunludur**; testler bellek içi yedeğe izin verir
/// (testler zaten `:memory:` yolunu kullanır, gerçek dosyaya dokunmaz).
fn startup_policy() -> PersistencePolicy {
    if cfg!(test) {
        PersistencePolicy::AllowEphemeral
    } else {
        PersistencePolicy::RequirePersistent
    }
}

/// Tek paylaşılan veritabanı bağlantısı.
///
/// Açılış **hata döndürebilir** ve bu hata `LazyLock` içinde panikletilmez: release profilinde
/// `panic = "abort"` olduğu için bir panik, startup tanılamasını (ve `exit(1)`'i) atlayıp süreci
/// sessizce sonlandırırdı. Hata saklanır; [`init`] onu startup'a **aynen** iletir, kayıt
/// gerektiren akışlar ise paniklemeden çalışmaya devam eder.
static DB: LazyLock<Result<Db, String>> = LazyLock::new(|| {
    Db::open_with_policy(&default_path(), startup_policy()).inspect_err(|err| {
        error!("Kalıcı kayıt veritabanı açılamadı: {err}");
    })
});
/// Son budamadan bu yana yapılan yazma sayısı.
static WRITES: AtomicU64 = AtomicU64::new(0);
static DB_SLOT: LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| std::sync::Arc::new(tokio::sync::Semaphore::new(1)));

/// Serialize blocking SQLite work off the Tokio workers; cancellation cannot release
/// the permit while the native operation is still running.
pub async fn blocking<T: Send + Default + 'static>(job: impl FnOnce() -> T + Send + 'static) -> T {
    let Ok(permit) = DB_SLOT.clone().acquire_owned().await else {
        return T::default();
    };
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        job()
    })
    .await
    {
        Ok(result) => result,
        Err(error) => {
            warn!("SQLite worker failed: {error}");
            T::default()
        }
    }
}

/// Açılabilmiş veritabanı; açılış başarısızsa `None` (hata ilk erişimde bir kez log'lanır).
fn database() -> Option<&'static Db> {
    DB.as_ref().ok()
}

pub fn healthy() -> bool {
    DB.as_ref().is_ok_and(|db| {
        db.path() != ":memory:"
            && db
                .with_connection(|connection| {
                    connection.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
                })
                .is_ok()
    })
}

/// SQLite veritabanı sarmalayıcısı. Sorgular tek satır (birincil anahtar) üzerinde
/// çalıştığı için çağrılar mikrosaniye düzeyindedir; bu yüzden API senkron tutulur.
pub struct Db {
    connection: Mutex<Connection>,
    path: String,
}

impl Db {
    /// Veritabanını açar, şemayı kurar ve performans pragmalarını uygular.
    /// Opens SQLite according to an explicit persistence policy.
    pub fn open_with_policy(path: &str, policy: PersistencePolicy) -> Result<Self, String> {
        match Self::connect(path) {
            Ok(connection) => Ok(Self {
                connection: Mutex::new(connection),
                path: path.to_string(),
            }),
            Err(err) if policy == PersistencePolicy::AllowEphemeral => {
                warn!(
                    "Kayıt veritabanı açılamadı ({}): {} — bellekte çalışılacak",
                    path, err
                );
                let connection = Self::connect(":memory:").map_err(|memory_err| {
                    format!("SQLite bellek içi açılış hatası: {memory_err}")
                })?;
                Ok(Self {
                    connection: Mutex::new(connection),
                    path: ":memory:".to_string(),
                })
            }
            Err(err) => Err(format!(
                "kalıcı SQLite veritabanı açılamadı ({}): {}",
                path, err
            )),
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
            connection
                .execute(
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
        self.query_one(
            "SELECT author_id FROM cards WHERE message_id = ?1",
            message_id,
        )
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
                .query_row(sql, params![message_id.get() as i64], |row| {
                    row.get::<_, i64>(0)
                })
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
        // Zehirlenmiş kilit kurtarılır: SQLite bağlantısı bir sorgunun ortasında panik olsa
        // bile tutarlıdır; aksi hâlde tek bir panik depoyu yeniden başlatmaya dek kapatırdı.
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        query(&connection).map_err(|err| err.to_string())
    }
}

/// Kart kaydı (mesaj → paylaşan kullanıcı) yazar. Her yazmada hafif bir sayaç artar;
/// sayaç eşiğe gelince en eski kayıtlar budanır.
pub fn remember_card(message_id: MessageId, channel_id: ChannelId, author_id: UserId) {
    let Some(db) = database() else {
        return;
    };
    db.remember(message_id, channel_id, author_id);

    if WRITES.fetch_add(1, Ordering::Relaxed) + 1 >= PRUNE_EVERY {
        WRITES.store(0, Ordering::Relaxed);
        prune_db(db);
    }
}

/// Kaydı siler (mesaj Discord'dan silindiğinde).
pub fn forget_card(message_id: MessageId) {
    if let Some(db) = database() {
        db.forget(message_id);
    }
}

/// Kartı paylaşan kullanıcı.
pub fn card_author(message_id: MessageId) -> Option<UserId> {
    database().and_then(|db| db.author(message_id))
}

/// Bu mesaj için kayıt var mı?
pub fn is_card(message_id: MessageId) -> bool {
    database().is_some_and(|db| db.contains(message_id))
}

/// Kayıt veritabanını hazırlar (şema + budama) ve özet bilgiyi log'lar.
///
/// Açılış başarısızsa **hata döner**; çağıran startup'ı durdurmalıdır (panik/abort yoktur).
pub fn init() -> Result<(), String> {
    init_with(DB.as_ref())
}

/// [`init`]'in karar çekirdeği: saf ve test edilebilir (açılış hatası startup'a aynen iletilir).
fn init_with(database: Result<&Db, &String>) -> Result<(), String> {
    let db = database.map_err(Clone::clone)?;
    if db.path() == ":memory:" && !cfg!(test) {
        return Err(
            "kalıcı SQLite açılamadı; ephemeral bellek deposuyla başlatma reddedildi".to_string(),
        );
    }
    info!(
        "Kayıt veritabanı hazır: {} ({} kart kaydı)",
        db.path(),
        db.count()
    );
    prune_db(db);
    Ok(())
}

/// En eski kayıtları budar (kayıt sayısı sınırı aşılmadıysa hiçbir şey silinmez).
fn prune_db(db: &Db) {
    let removed = db.prune(MAX_RECORDS);
    if removed > 0 {
        info!(
            "Kayıt veritabanı budandı: {} eski kart kaydı silindi",
            removed
        );
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

    fn open_test(path: &str) -> Db {
        Db::open_with_policy(path, PersistencePolicy::AllowEphemeral).expect("test database")
    }

    #[test]
    fn records_survive_reopening_the_file() {
        let path = test_db_path("persistence");
        cleanup(&path);
        let path_str = path.display().to_string();

        {
            let db = open_test(&path_str);
            db.remember(MessageId::new(1_000), ChannelId::new(9), UserId::new(42));
            assert_eq!(db.count(), 1);
        }

        // Bot yeniden başlatıldı: yeni bağlantı aynı kaydı görmeli.
        let reopened = open_test(&path_str);
        assert_eq!(
            reopened.author(MessageId::new(1_000)),
            Some(UserId::new(42))
        );
        assert!(reopened.contains(MessageId::new(1_000)));
        assert_eq!(reopened.author(MessageId::new(2_000)), None);
        assert!(!reopened.contains(MessageId::new(2_000)));

        cleanup(&path);
    }

    #[test]
    fn forgetting_a_card_removes_only_that_record() {
        let path = test_db_path("forget");
        cleanup(&path);
        let db = open_test(&path.display().to_string());

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
        let db = open_test(&path.display().to_string());

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
    fn explicit_persistence_policy_rejects_unknown_paths() {
        let path = std::env::temp_dir()
            .join(format!("botgek-store-missing-{}", std::process::id()))
            .join("database.db");
        assert!(
            Db::open_with_policy(
                &path.display().to_string(),
                PersistencePolicy::RequirePersistent
            )
            .is_err()
        );
    }

    #[test]
    fn explicit_ephemeral_policy_remains_available_for_tests() {
        let db = Db::open_with_policy(
            "/olmayan-dizin/veritabani.db",
            PersistencePolicy::AllowEphemeral,
        )
        .expect("test fallback");
        assert_eq!(db.path(), ":memory:");
        db.remember(MessageId::new(5), ChannelId::new(1), UserId::new(2));
        assert!(
            db.contains(MessageId::new(5)),
            "bellek içi veritabanı çalışmalı"
        );
    }

    #[test]
    fn startup_returns_open_failures_instead_of_aborting() {
        // Üretimde `RequirePersistent` açılış hatası üretir; `init` bunu startup'a **aynen**
        // iletmek zorundadır (panik/abort startup tanılamasını gizlerdi).
        let failure =
            "kalıcı SQLite veritabanı açılamadı (/olmayan-dizin/x.db): unable to open".to_string();
        assert_eq!(init_with(Err(&failure)), Err(failure));
    }

    #[test]
    fn startup_accepts_the_memory_store_used_by_tests() {
        let db = open_test(":memory:");
        assert!(init_with(Ok(&db)).is_ok());
    }
}
