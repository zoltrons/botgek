# Çalışma zamanı çekirdeği

## Görev

Süreci başlatır, ortam değişkenlerini okur, sürüm bilgisini taşır, ortak yardımcıları ve bellek içi log
halkasını sağlar.

## Dosyalar

| Dosya | İçerik |
| --- | --- |
| `src/main.rs` | CLI bayrakları, loglama kurulumu, ayar ve veritabanı yükleme, HTTP istemcisi, Discord istemcisi, panelin başlatılması, kapanış sinyali. |
| `src/config.rs` | `Config`: ortam değişkenlerinden okunan sabit yapılandırma. `parse_bool` ve `env_bool` yardımcıları. |
| `src/version.rs` | Derleme zamanında gömülen sürüm künyesi ve User-Agent dizeleri. |
| `build.rs`, `build-support/version-parse.rs` | Sürümü git etiketlerinden türetir. Mantık `build.rs` ile `version.rs` arasında paylaşılır (`include!`). |
| `src/utils/mod.rs` | `truncate_chars`, `secret_matches`, `base64_encode`, `unix_millis`, `unix_secs`, `DISCORD_CONTENT_LIMIT`, `MAX_SECRET_LEN`. |
| `src/utils/mentions.rs` | `build_selective_allowed_mentions`: yalnızca orijinal mesajda etiketlenenler bildirim alır. |
| `src/logs.rs` | `tracing` olaylarını yakalayan 512 olaylık bellek içi halka. |

## Nasıl çalışır

### Başlangıç sırası (`main`)

1. CLI bayrağı yorumlanır (`--version`, `--help`). Bayraklar yapılandırmadan önce işlenir, bu yüzden
   `.env` olmadan da çalışır. Tanınmayan bayrak çıkış kodu 2 verir.
2. `.env` yüklenir. Loglama kurulur: terminal katmanı ve `logs::CaptureLayer`. Varsayılan filtre
   `botgek=info,serenity=warn`.
3. `Config::from_env` ve `AppSettings::load` çalışır. Ayarlar `Arc<RwLock<AppSettings>>` içinde tutulur.
4. SQLite deposu açılır (`store`). Üretimde kalıcılık zorunludur.
5. Tek bir `reqwest::Client` kurulur: rustls, HTTP/2, 30 sn toplam zaman aşımı, 8 sn bağlantı zaman aşımı,
   300 sn havuz boşta süresi. Panel, MCP ve dönüştürücüler aynı istemciyi paylaşır.
6. Discord, X, Reddit ve diğer uçlar arka planda ısıtılır (`prewarm_connections`). Isıtma aralığı
   (45 sn) havuz boşta süresinden kısa olmak zorundadır, aksi halde kazanç kaybolur.
7. Gateway istemcisi başlar. Panel ayrı bir `tokio` görevinde, Discord `Http` ve önbelleğiyle kalkar.
8. Ctrl+C ve SIGTERM dinlenir, shard yöneticisi temiz kapatılır.

### Yapılandırma (`Config`)

Alanlar ve ortam değişkenleri için [CONFIGURATION.md](../CONFIGURATION.md). Önemli noktalar:

- `INSTAGRAM_SESSIONID` yalnızca `.env`'den okunur, `settings.json`'a ve panele yazılmaz. `Debug` çıktısı
  gizli değerleri göstermez.
- `REDDIT_CLIENT_ID` sır değildir. Varsayılan `DEFAULT_REDDIT_CLIENT_ID`.
- Mantıksal değişkenler `yes/no`, `on/off`, `1/0`, `true/false` biçimlerini kabul eder. Tanınmayan değer
  uyarıyla varsayılana düşer.

### Sürüm (`version`)

`SEMVER`, `DISPLAY`, `FULL`, `GIT_SHA`, `IS_RELEASE`, `IS_DIRTY`, `USER_AGENT`, `DISCORD_USER_AGENT`
sabitleri derleme anında gömülür. Git bulunamazsa (Docker, tarball) `Cargo.toml` sürümüne `+local` eklenir
ve derleme başarısız olmaz. `USER_AGENT` dış servislere botu tanıtır, `DISCORD_USER_AGENT` Discord REST
için standart biçimdedir.

### Log halkası (`logs`)

`recent(limit, min_level)` en yeni olayları döner. Kapasite 512, mesaj 4096 ve alan 1024 karakterle
sınırlıdır. Halka diske yazılmaz, süreç ömrü boyunca yaşar ve içeriği ayrıştırmaz (URL, kullanıcı kimliği
ya da token temizlenmez). `CaptureLayer` kendi içinde log üretmez, böylece geri besleme döngüsü olmaz.

## Değişmezler

- `KEEPALIVE_INTERVAL` her zaman `POOL_IDLE_TIMEOUT` değerinden küçüktür. Test bunu doğrular.
- Gizli değerler (`ADMIN_TOKEN`, `MCP_TOKEN`, `INSTAGRAM_SESSIONID`, `DISCORD_TOKEN`) loglanmaz ve
  yanıtlarda yankılanmaz. Karşılaştırma `utils::secret_matches` ile sabit zamanlıdır.
- `truncate_chars` bayt değil karakter sınırında keser ve kısaltınca `…` ekler. Kart bileşenleri bunu her
  gönderimde kullandığı için hızlı yolu korunmalıdır.

## Değiştirirken

- Yeni ortam değişkeni: `Config` alanı, `from_env`, `.env.example`, [CONFIGURATION.md](../CONFIGURATION.md).
  Sırsa `Debug` uygulamasında gizleyin.
- Yeni ısıtılacak uç: `KEEPALIVE_TARGETS` dizisine ekleyin.
- Yeni modül: `main.rs` başındaki `mod` listesine ekleyin.

## Testler

`cargo test --locked version`, `cargo test --locked logs`, `cargo test --locked config`. Sürüm testleri
gömülen sabitlerle çalışma zamanında yeniden çözülen değerlerin aynı olduğunu da denetler.
