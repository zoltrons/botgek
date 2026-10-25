//! Kalıcı kayıt deposu (SQLite).
//!
//! Bot, dönüştürdüğü her kart için **hangi mesajı hangi kullanıcının paylaştığını** saklar.
//! Bu kayıt iki yerde kullanılır:
//!
//! - 🗑️ **Reaksiyonla silme izni:** kartı paylaşan kullanıcı (taklit webhook'unda mesajın
//!   yazarı bot/kullanıcı olmadığı için) bu kayıtla bulunur.
//! - 🛡️ **AI koruması:** kartlara verilen yanıtlarda AI sohbeti tetiklenmez.
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
pub const MAX_RECORDS: i64 = 200_000;
/// Budama kontrolü her bu kadar kayıtta bir yapılır (her yazmada sorgu çalıştırmamak için).
const PRUNE_EVERY: u64 = 512;
/// Teşhis sorgusunun (MCP `db_query`) en uzun süresi. Sorgu kart yazımlarıyla aynı tek bağlantıyı
/// tutar; süre sınırı olmadan sonsuz bir `WITH RECURSIVE` sorgusu kayıtları süresiz kilitlerdi.
const QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

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

/// Salt okunur teşhis sorgusunun sonucu (MCP `db_query` aracı).
///
/// Sütun adları korunur; değerler JSON'a çevrilir. BLOB'lar içerik sızdırmamak için
/// yalnızca boyutlarıyla temsil edilir. `truncated`, `limit` nedeniyle satır atlandığını
/// söyler (sorgu yine de salt okunurdur, veri değişmez).
#[derive(Debug, Clone, serde::Serialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub returned: usize,
    pub truncated: bool,
}

/// Kayıt veritabanında **salt okunur** bir sorgu çalıştırır.
///
/// Yalnızca tek bir `SELECT`/`WITH` ifadesi kabul edilir; `rusqlite` ifadenin
/// salt okunur olduğunu ayrıca doğrular (`Statement::readonly`). Yazma denemeleri
/// `InvalidQuery` ile reddedilir. Sonuç satırı `limit` ile sınırlıdır (en fazla 200).
pub fn query_readonly(sql: &str, limit: usize) -> Result<QueryResult, String> {
    let trimmed = sql.trim();
    let without_trailing = trimmed.trim_end_matches(';').trim();
    let lowered = without_trailing.to_ascii_lowercase();
    if without_trailing.is_empty()
        || !(lowered.starts_with("select") || lowered.starts_with("with"))
    {
        return Err("yalnızca tek bir SELECT/WITH sorgusu kabul edilir".to_string());
    }
    if without_trailing.contains(';') {
        return Err(
            "tek sorgu gönderin: noktalı virgülle ayrılmış ikinci ifade reddedildi".to_string(),
        );
    }

    let db = database().ok_or_else(|| "kayıt veritabanı açık değil".to_string())?;
    let limit = limit.clamp(1, 200);
    db.query_readonly(without_trailing, limit)
}

/// Veritabanı dosyalarının bayt cinsinden boyutları (`main`/`wal`/`shm`).
///
/// SQLite WAL modunda yeni satırlar önce `-wal` dosyasına gider; yalnızca ana dosyaya
/// bakmak disk kullanımını olduğundan küçük gösterir. `None`: bellek içi veya kapalı.
pub fn file_sizes() -> Option<serde_json::Value> {
    let path = database()?.path();
    if path == ":memory:" {
        return None;
    }
    let size = |candidate: &str| {
        std::fs::metadata(candidate)
            .ok()
            .map(|metadata| metadata.len())
    };
    Some(serde_json::json!({
        "main": size(path),
        "wal": size(&format!("{path}-wal")),
        "shm": size(&format!("{path}-shm"))
    }))
}

/// Veritabanı dosyasının bayt cinsinden boyutu (`:memory:` ise `None`).
pub fn file_bytes() -> Option<u64> {
    let path = database()?.path();
    if path == ":memory:" {
        return None;
    }
    std::fs::metadata(path).ok().map(|metadata| metadata.len())
}

/// Salt okunur teşhis sorgusunu blocking worker'da çalıştırır (MCP `db_query`).
///
/// Aynı veritabanı slotunu kullanır: teşhis sorgusu devam eden kart yazımını kilitlemez,
/// ancak SQLite bağlantısına paralel erişmez.
pub async fn query_readonly_blocking(sql: String, limit: usize) -> Result<QueryResult, String> {
    let Ok(permit) = DB_SLOT.clone().acquire_owned().await else {
        return Err("SQLite worker kullanılamıyor".to_string());
    };
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        query_readonly(&sql, limit)
    })
    .await
    {
        Ok(result) => result,
        Err(error) => Err(format!("SQLite worker failed: {error}")),
    }
}

/// Kayıt veritabanı yolu (`:memory:` olabilir; açık değilse `None`).
pub fn path() -> Option<String> {
    database().map(|db| db.path().to_string())
}

/// Kayıtlı kart sayısı (veritabanı açık değilse `None`).
pub fn record_count() -> Option<i64> {
    database().map(Db::count)
}

pub fn healthy() -> bool {
    DB.as_ref().is_ok_and(|db| {
        db.path() != ":memory:"
            && db
                .with_connection(crate::telemetry::DbOp::Healthy, |connection| {
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
        let result = self.with_connection(crate::telemetry::DbOp::Remember, |connection| {
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
        let result = self.with_connection(crate::telemetry::DbOp::Forget, |connection| {
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
            crate::telemetry::DbOp::Author,
            "SELECT author_id FROM cards WHERE message_id = ?1",
            message_id,
        )
        .and_then(|value| u64::try_from(value).ok())
        .map(UserId::new)
    }

    /// Bu mesaj için kayıt var mı? (kart tanıma)
    pub fn contains(&self, message_id: MessageId) -> bool {
        self.query_one(
            crate::telemetry::DbOp::Contains,
            "SELECT 1 FROM cards WHERE message_id = ?1",
            message_id,
        )
        .is_some()
    }

    /// Toplam kayıt sayısı (log/teşhis).
    pub fn count(&self) -> i64 {
        self.with_connection(crate::telemetry::DbOp::Count, |connection| {
            connection.query_row("SELECT COUNT(*) FROM cards", [], |row| row.get::<_, i64>(0))
        })
        .unwrap_or(0)
    }

    /// Salt okunur sorguyu çalıştırır; yazma ifadeleri `InvalidQuery` ile reddedilir.
    fn query_readonly(&self, sql: &str, limit: usize) -> Result<QueryResult, String> {
        self.with_connection(crate::telemetry::DbOp::Query, |connection| {
            // Bekçi iş parçacığı süre dolunca çalışan ifadeyi keser (`SQLITE_INTERRUPT`). Sorgu
            // bitmişse `interrupt` etkisizdir; bekçi bağlantı bırakılmadan önce birleştirilir.
            let interrupt = connection.get_interrupt_handle();
            let (finished, done) = std::sync::mpsc::channel::<()>();
            let watchdog = std::thread::spawn(move || {
                if done.recv_timeout(QUERY_TIMEOUT)
                    == Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                {
                    interrupt.interrupt();
                }
            });
            let result = run_readonly(connection, sql, limit);
            let _ = finished.send(());
            let _ = watchdog.join();
            result
        })
    }

    /// En yeni `keep` kaydı bırakır, daha eskilerini siler. Kayıt sayısı `keep`'ten azsa
    /// hiçbir şey silinmez. Silinen kayıt sayısını döner.
    pub fn prune(&self, keep: i64) -> usize {
        self.with_connection(crate::telemetry::DbOp::Prune, |connection| {
            connection.execute(
                "DELETE FROM cards
                  WHERE message_id < (SELECT message_id FROM cards ORDER BY message_id DESC LIMIT 1 OFFSET ?1)",
                params![keep - 1],
            )
        })
        .unwrap_or(0)
    }

    fn query_one(
        &self,
        op: crate::telemetry::DbOp,
        sql: &str,
        message_id: MessageId,
    ) -> Option<i64> {
        let result = self.with_connection(op, |connection| {
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
        op: crate::telemetry::DbOp,
        query: impl FnOnce(&Connection) -> Result<T, rusqlite::Error>,
    ) -> Result<T, String> {
        let started = std::time::Instant::now();
        // Zehirlenmiş kilit kurtarılır: SQLite bağlantısı bir sorgunun ortasında panik olsa
        // bile tutarlıdır; aksi hâlde tek bir panik depoyu yeniden başlatmaya dek kapatırdı.
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let result = query(&connection).map_err(|err| err.to_string());
        crate::telemetry::db_op(op, started.elapsed(), result.is_ok());
        result
    }
}

/// Kart kaydı (mesaj → paylaşan kullanıcı) yazar. Her yazmada hafif bir sayaç artar;
/// sayaç eşiğe gelince en eski kayıtlar budanır.
/// Hazırlanan ifadenin salt okunur olduğunu doğrular ve en fazla `limit` satırı JSON'a çevirir.
fn run_readonly(
    connection: &Connection,
    sql: &str,
    limit: usize,
) -> Result<QueryResult, rusqlite::Error> {
    let mut statement = connection.prepare(sql)?;
    if !statement.readonly() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let columns = statement
        .column_names()
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    let mut rows = statement.query([])?;
    let mut values = Vec::new();
    let mut truncated = false;
    while let Some(row) = rows.next()? {
        if values.len() >= limit {
            truncated = true;
            break;
        }
        values.push(
            (0..columns.len())
                .map(|index| value_to_json(row.get_ref(index)?))
                .collect::<Result<Vec<_>, rusqlite::Error>>()?,
        );
    }
    let returned = values.len();
    Ok(QueryResult {
        columns,
        rows: values,
        returned,
        truncated,
    })
}

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

/// SQLite değerini JSON'a çevirir. BLOB içeriği bilinçli olarak yalnızca boyutla temsil
/// edilir: teşhis sorguları ikili veri dökmemeli.
fn value_to_json(
    value: rusqlite::types::ValueRef<'_>,
) -> Result<serde_json::Value, rusqlite::Error> {
    use rusqlite::types::ValueRef;
    Ok(match value {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(number) => serde_json::Value::from(number),
        ValueRef::Real(number) => serde_json::Number::from_f64(number)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        ValueRef::Text(text) => {
            serde_json::Value::String(String::from_utf8_lossy(text).into_owned())
        }
        ValueRef::Blob(blob) => serde_json::Value::String(format!("<blob {} bytes>", blob.len())),
    })
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

    /// Sonsuz sorgu kesilir ve bağlantı sonraki işler için kullanılabilir kalır.
    #[test]
    fn runaway_diagnostic_query_is_interrupted() {
        let db = open_test(":memory:");
        let started = std::time::Instant::now();
        let result = db.query_readonly(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) \
             SELECT count(*) FROM c",
            10,
        );
        assert!(result.is_err());
        assert!(started.elapsed() < QUERY_TIMEOUT * 3);
        let ok = db
            .query_readonly("SELECT 1", 10)
            .expect("bağlantı kullanılabilir");
        assert_eq!(ok.rows, vec![vec![serde_json::json!(1)]]);
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

    #[test]
    fn readonly_query_returns_columns_and_rejects_writes() {
        let db = open_test(":memory:");
        db.remember(MessageId::new(11), ChannelId::new(1), UserId::new(2));

        let result = db
            .query_readonly(
                "SELECT message_id, author_id FROM cards ORDER BY message_id",
                10,
            )
            .expect("salt okunur sorgu çalışmalı");
        assert_eq!(result.columns, vec!["message_id", "author_id"]);
        assert_eq!(result.returned, 1);
        assert!(!result.truncated);
        assert_eq!(result.rows[0][1], serde_json::json!(2));

        assert!(
            db.query_readonly("DELETE FROM cards", 10).is_err(),
            "yazma ifadesi reddedilmeli"
        );
        assert!(
            db.query_readonly("SELECT 1; DROP TABLE cards", 10).is_err(),
            "çok ifadeli girdi reddedilmeli"
        );
    }

    #[test]
    fn readonly_query_truncates_at_the_requested_limit() {
        let db = open_test(":memory:");
        for id in 1..=5u64 {
            db.remember(MessageId::new(id), ChannelId::new(1), UserId::new(7));
        }
        let result = db
            .query_readonly("SELECT message_id FROM cards ORDER BY message_id", 2)
            .expect("sorgu çalışmalı");
        assert_eq!(result.returned, 2);
        assert!(result.truncated);
    }
}
