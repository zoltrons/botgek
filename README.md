# Botgek

Rust ile yazılmış Discord sosyal medya bağlantı dönüştürücüsü ve isteğe bağlı OpenRouter sohbet botu.
Twitter/X, Instagram, Reddit, Bluesky, TikTok, Spotify, Twitch, Pixiv, Tumblr, Facebook ve YouTube
Community Posts desteklenir.

## Güncel davranış

- Twitter/X, Instagram, Reddit, Twitch, Pixiv, Tumblr, Facebook ve YouTube Community Posts
  **native-only**; TikTok, Bluesky ve Spotify için native/proxy seçilebilir.
- Ayarlar global, guild ve kanal kapsamlarında çözümlenir. Thread'ler yalnızca parent kanal ayarını miras alır;
  kategori ayarı yoktur. Kanal kapsamları WebUI'daki authenticated Discord kanal listesinden seçilir.
- Hassas içerik varsayılan olarak spoiler'dır. İstenirse yalnızca NSFW kanallarda açılabilir; kaynak spoiler'ı
  ve kullanıcı tarafından `||...||` ile işaretlenen linkler NSFW kanalda da gizli kalır.
- Başka botların desteklenen linkleri varsayılan olarak yok sayılır; WebUI'dan güvenli, impersonation'sız
  bot dönüşümü açılabilir. Bot mesajları AI sohbetini tetiklemez ve duplicate kaynaklar TTL guard ile elenir.
- Otomatik X çevirisinin hedef dili WebUI'dan `tr`, `en`, `de` veya BCP-47 uyumlu başka bir kod olarak
  seçilebilir. Proxy bayrak çevirileri mevcut davranışını korur.
- Native kartlar **yalnızca Discord Components V2** kullanır; klasik embed fallback'i veya kart stili seçimi yoktur.
- Tumblr önce fxtumblr uyumlu renderer'ı (`FXTUMBLR_DOMAIN`, varsayılan `tpmblr.com`) dener; böylece NPF/reblog
  zinciri ve çoklu medya tek güvenli render görselinde korunur. Renderer erişilemezse public OG fallback kullanılır.
- Görseller ve GIF'ler doğrudan CDN bağlantılarıdır; X görsellerinde `name=orig` kullanılır.
- X GIF'leri vxTwitter gifconvert üzerinden animasyonlu AVIF bağlantısına dönüştürülür; yerel ffmpeg yolu yoktur.
- Reddit videosunda video/ses saf Rust ile MP4'e birleştirilir. Aynı anda en fazla **iki** birleştirme;
  **5 saniye** bütçe ve çağırandan iptal sinyali vardır. Başarısızlıkta uzak, sessiz video korunur.
- Native dönüşümler süreç başına **16** eşzamanlı işle sınırlıdır; kapasite dolduğunda orijinal mesaj korunur.
- Gönderi önbelleği **600 saniye / 512 girdi** ile sınırlıdır; kaynak, çeviri, alıntı ve video bütçesi anahtara dâhildir.
  Aynı anahtara gelen istekler birleştirilir. Metrikler önbellek ömrü boyunca eski olabilir.
- X bağlantı önizlemesi yalnızca public HTTP(S), 80/443 portları ve doğrulanan DNS adreslerine gider.
  Yönlendirmeler yeniden doğrulanır; toplam bütçe **500 ms**. İstek ortam proxy'sini kullanmaz.
- Orijinal mesaj yalnızca başarılı gönderimden sonra silinir. Sonucu belirsiz Discord POST'u otomatik yeniden
  gönderilmez; bu durumda orijinal mesaj korunur ve hata loglanır. 429 bekleme süresi kısaltılmaz.
- Rol filtreleri, AI ve çeviri reaksiyonu ekleme/kaldırma aynı politikayı kullanır. Boş allowlist erişim vermez;
  rol sorgusu başarısız olursa etkin filtre erişimi reddeder.

Bu bütçeler yapılandırılmış sınırlardır; **ölçülmüş p95 veya RAM garantisi değildir**.

## Yerel kurulum

1. Rust toolchain: `rust-toolchain.toml` **1.98.1** sürümünü sabitler; kaynak dil tabanı Rust 1.88'dir.
   Linux için C derleyicisi/linker gerekir (SQLite bundled olarak derlenir).
2. `.env.example` dosyasını `.env` olarak kopyalayın. Gerçek `.env` dosyasını Git'e eklemeyin.
3. `DISCORD_TOKEN` ve yönetim paneli için `ADMIN_TOKEN` değerlerini doldurun.
4. `cargo run --locked` çalıştırın. Sürüm sorgusu: `cargo run --locked -- --version`.

Discord Developer Portal'da **Message Content Intent** açılmalıdır. Gerekli bot izinleri:
Send Messages, Embed Links, Read Message History, Add Reactions; orijinal mesajları silmek için
Manage Messages, kullanıcı taklidi için Manage Webhooks. Yanıtlarda varsayılan olarak bot kimliği kullanılır.

### Windows

MSVC toolchain için Visual C++ Build Tools ve Windows SDK gerekir. Önceden kurulu GNU/LLVM toolchain
kullanıyorsanız örneğin `cargo +stable-x86_64-pc-windows-gnullvm test --locked` çalıştırın;
bu seçeneğin derleyici sürümünü `rustc +stable-x86_64-pc-windows-gnullvm --version` ile doğrulayın.
Global rustup ayarını değiştirmeniz gerekmez.

## Yönetici girişi

`.env` içinde:

```dotenv
ADMIN_TOKEN=
WEB_BIND=127.0.0.1
WEB_PORT=3000
WEB_SECURE_COOKIE=false
```

`ADMIN_TOKEN` için **32–512 karakterlik, yüksek entropili rastgele bir sır** kullanın.
Örneğin kendi terminalinizde `openssl rand -hex 32` ile üretebilirsiniz. Çıktıyı `.env` dosyanıza
yerel olarak yazın; sohbet, log, ekran görüntüsü veya Git'e koymayın.

- Token yok/geçersizse **WebUI açılmaz**; Discord botunun çalışması etkilenmez.
- `http://127.0.0.1:3000` giriş sayfasına yönlenir. Token yalnızca giriş isteğinde gönderilir;
  localStorage/sessionStorage'a yazılmaz.
- Oturum **8 saat** sürer; rastgele kimlik, `HttpOnly`, `SameSite=Strict` çerezi kullanılır.
  Yeniden başlatma tüm oturumları iptal eder. Panelde **Çıkış yap** düğmesi vardır.
- Durum değiştiren istekler `X-Botgek-Request: 1` ister; CORS açılmamıştır.
- Giriş denemeleri süreç genelinde **20/dakika**, oturum sayısı **64** ile sınırlıdır.
- Uzak erişimde **HTTPS ters proxy** veya SSH tüneli kullanın. HTTPS için `WEB_SECURE_COOKIE=true`
  ayarlayın; botun HTTP portunu doğrudan internete açmayın. Uygulama TLS sonlandırmaz.

Tokenı döndürmek için `.env` değerini değiştirip botu yeniden başlatın. Token API yanıtlarında ve loglarda görünmez.

## Yapılandırma ve kalıcılık

| Değişken | Davranış |
| --- | --- |
| `BOTGEK_SETTINGS_PATH` | Ayar dosyası; boşsa `BOTGEK_DB_PATH` bir dizin içeriyorsa (ör. `/app/data/botgek.db`) o dizindeki `settings.json`, aksi hâlde çalışma dizinindeki `settings.json` |
| `BOTGEK_SETTINGS_SEED` | Yalnızca hedef dosya yoksa ilk içe aktarım kaynağı |
| `BOTGEK_DB_PATH` | Kart–paylaşan kullanıcı kaydı; varsayılan `botgek.db` |
| `OPENROUTER_API_KEY` | İsteğe bağlı AI erişimi |
| `FXTUMBLR_DOMAIN` | Tumblr NPF/render hizmeti; varsayılan `tpmblr.com`, boş bırakılırsa public OG fallback kullanılır |
| `INSTAGRAM_SESSIONID` | İsteğe bağlı Instagram reel/video çerezi; yalnızca ortamda saklanır |
| `REDDIT_CLIENT_ID` | İsteğe bağlı public OAuth istemci kimliği; sır değildir |
| `DELETE_ORIGINAL` | Başarılı dönüşüm sonrası orijinal mesajı silme |

Ayarlar global varsayılan + sunucu başına bölüm override'ı (`modules`, `role_filters`, `ai`, `engine`) olarak
çözümlenir. Dosya yazımı aynı dizinde geçici dosya, flush ve atomik replacement kullanır. WebUI kayıtları
seri işlenir; disk işlemi async worker'ı bloklamaz. Bozuk ayar dosyası **varsayılan erişim politikasıyla
başlatılmaz**: düzeltmeniz veya sağlam yedeği geri koymanız gerekir. Göç öncesi yedek alınamıyorsa göç durur.

SQLite WAL kullanılır; işlemler sınırlı blocking worker üzerinden yürür. 200.000 kart sınırı ve budama vardır.
Dosya açılamazsa bellek içi fallback loglanır ve readiness başarısız olur; bunu kalıcı çalışma saymayın.

## Docker ve güncelleme

Kaynak koddan derleyen bu Compose dosyasında güncelleme komutu:

```sh
docker compose up -d --build
```

`docker compose pull` yalnızca `image:` kullanan bir Compose tanımı için yeni imajı
çeker; bu dosyada imaj `build:` ile üretilir. Yayınlanmış GHCR imajına geçildiğinde
`pull && up -d` kullanılabilir.

Compose yönetim portunu yalnızca **host 127.0.0.1** üzerinde yayınlar; container içinde `WEB_BIND=0.0.0.0`
kullanılır. Uzak makinede SSH tüneli örneği: `ssh -L 3000:127.0.0.1:3000 sunucu`.

**Mevcut kurulumdan geçiş:** `./settings.json` salt okunur `/app/settings.legacy.json` olarak bağlanır.
İlk açılışta `/app/data/settings.json` yoksa bu dosya içe aktarılır. Sonraki WebUI kayıtları `botgek-data`
volume'undaki dosyaya gider; hosttaki eski dosya artık güncellenmez. Güncellemeden önce ayar ve SQLite
dosyalarının yedeğini alın. `docker compose down -v` kalıcı volume'u siler; normal güncellemede kullanmayın.

Atomik ayar replacement'ı için tek yazılabilir dosya bind-mount'u yerine **dizin/volume** kullanın.
Compose volume adı varsayılan olarak `botgek_botgek-data`'dır ve proje dizini değişse bile sabit kalır;
başka bir mevcut volume kullanıyorsanız `.env` içine `BOTGEK_DATA_VOLUME=<gerçek-volume-adı>` yazın.
Bot kullanıcısına (Docker: UID/GID 1000) dizin yazma izni verin; `chmod 666` kullanmayın.
Yedek/geri yükleme işlemlerinde artık `BOTGEK_SETTINGS_PATH` ile gösterilen güncel dosyayı hedefleyin.

Kaynak imaj derlemesinde `.git` bulunmaz. Sürüm künyesini `BOTGEK_VERSION` ve `BOTGEK_SHA` build arg'larıyla
verebilirsiniz. Runtime root olmayan kullanıcıdır; ffmpeg kurulmaz ve kullanılmaz.

Debian paketinde servis: `botgek.service`; çalışma dizini `/var/lib/botgek`, ortam dosyası `/etc/botgek/botgek.env`.
Paketin `postinst` betiği `botgek` sistem kullanıcısını/grubunu ve `/var/lib/botgek`'i oluşturur, ortam dosyası yoksa şablondan `root:root 0600` olarak üretir.
`DISCORD_TOKEN`'ı girdikten sonra `systemctl enable --now botgek`; paket yükseltmesi çalışan servisi yeniden başlatır.
Log: `journalctl -u botgek -f`. Kaynak tarball kurulumu (ve güncellemesi) için `deploy-rpi.sh` mevcuttur.

Servis `ProtectSystem=strict` ile çalışır: yalnızca `/var/lib/botgek` (`StateDirectory=`) yazılabilir.
Varsayılan `botgek.db` ve `settings.json` oraya yazılır; `BOTGEK_DB_PATH`/`BOTGEK_SETTINGS_PATH` başka bir dizini gösterecekse `systemctl edit botgek` ile `ReadWritePaths=` ekleyin.

Ayar yedeği: `./backup-settings.sh` (Windows: `backup-settings.ps1`) önce `BOTGEK_SETTINGS_PATH`'i kullanır.
systemd kurulumunda dosya `/var/lib/botgek/settings.json`'dır (`sudo ./backup-settings.sh /var/lib/botgek/settings.json`).
Docker'da dosya volume içindedir: `docker compose exec botgek cat /app/data/settings.json > settings.json.bak` ile dışarı alın.

## Kontroller ve CI

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
node --check src/web/ui/app.js
node --check src/web/ui/login.js
node --test tests/web-ui.test.cjs
```

Normal test koşusu **ağdaki sosyal medya servislerine bağlanmaz**. Yerel HTTP testleri yalnızca loopback
üzerinde fixture sunucuları açar; dosya testleri geçici dosyalar kullanır. Rust ve JavaScript ayar çözümleme
testleri `tests/fixtures/settings-resolution.json` sözleşmesini paylaşır.

Canlı testler açıkça ignore edilir ve yalnızca bilerek çalıştırılır:

```sh
cargo test --locked live_ -- --ignored --nocapture
```

Bu komut gerçek upstream erişimi ister; Instagram oturum testi ayrıca özel çerez ister. Eksik önkoşul veya
başarısız birleştirme **başarılı test gibi gösterilmez**. Üretim sırlarını public PR testlerine aktarmayın.

- `build-release.yml`: test, fmt, Clippy, panel testleri; ardından ARM64/ARMv7/x86_64 paket ve imaj.
- `quality.yml`: RustSec audit ve LCOV artifact. Coverage ilk ölçümden itibaren izlenir; uydurma eşik yoktur.
- `release.yml`: elle sürüm bump/etiket akışı. Yerelde `./release.sh patch` veya
  `./release.sh minor --push`; Windows'ta `release.ps1`. Temiz çalışma ağacı gerekir.
- Sürüm `build.rs` ile git etiketinden gömülür; `SOURCE_DATE_EPOCH` tekrar üretilebilir tarih içindir.

Serenity 0.12'nin Rustls gateway arka ucu eski `rustls-webpki 0.102` sürümünü sabitlediği için
Serenity'nin desteklenen **native-TLS** arka ucu kullanılır; Linux/ARM'da OpenSSL vendored derlenir
(build ortamında C toolchain, Perl ve make gerekir), Windows'ta OS TLS kullanılır. Botun kendi HTTP
istemcileri **Rustls 0.23** kullanmayı sürdürür. RUSTSEC-2026-0049/0098/0099/0104 istisnayla susturulmaz.
Gelecekte upstream gateway güncellendiğinde bu uyumluluk tercihinin yeniden değerlendirilmesi gerekir.
Bağımlılık denetiminin gerçekten geçtiğini ilgili CI sonucu olmadan varsaymayın.

## Gözlemlenebilirlik ve performans

- `/api/health`: public liveness; yalnızca HTTP sunucusunun ayakta olduğunu söyler.
- `/api/ready`: giriş gerektirir; Discord bağlantısı ve kalıcı SQLite erişimini kontrol eder, bozuk durumda 503.
- `/api/metrics`: giriş gerektirir; platform bazlı native başarı/hata, kümülatif gecikme histogramı,
  cache-hit ve kapasite aşımı sayıları. Süreç yeniden başlayınca sıfırlanır; kullanıcı/veri/token içermez.
- `RUST_LOG=botgek=debug,serenity=warn`: veri/medya/gönderim süreleri. Logları özel tutun; gönderi URL'leri içerebilir.

Tekrarlanabilir offline CPU başlangıç ölçümü:

```sh
cargo test --release --locked offline_card_baseline -- --ignored --nocapture
```

Bu 10.000 örnekte parse+render p50/p95 ölçer; **ağ, toplam Discord gecikmesi veya RSS ölçmez**.
Gerçek Raspberry Pi üzerinde aynı sürüm/derleyiciyle sıcak-soğuk önbellek, büyük Reddit videosu,
eşzamanlı aynı/farklı URL, yavaş upstream ve 429 senaryolarını ayrıca ölçün. RSS/CPU, upstream istek sayısı,
timeout sonrası aktif iş ve p95 değerlerini değişiklik öncesi/sonrası karşılaştırın. Donanım üzerinde
ölçülmeden hızlanma veya bellek tasarrufu yüzdesi iddia edilmez.
