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
- Kartın görünme hedefi **1 saniyedir** (veri + medya + gönderim). Veri hazır olduktan sonra
  çeviri/alıntı derinliği/link kartı gibi isteğe bağlı işler için **650 ms**, webhook beklemesi için
  kalan bütçe (en fazla **200 ms**) ayrılır; Discord POST'una **250 ms** pay bırakılır. Kaynak
  (vxTwitter) yavaşladığında kart, mevcut syndication verisiyle gönderilir.
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
- Global **`/dönüştür link:<bağlantı>`** komutu DM'ler dâhil her yerde çalışır; ayrıca bir mesaja sağ tıklayıp
  **Apps → Dönüştür** ile o mesajdaki bağlantı dönüştürülür. Her iki akış da kanal/thread motor ayarlarını ve
  rol filtrelerini uygular; **WebUI'daki platform modülü açık/kapalı ayarından bağımsızdır** (komut açıkça
  çağrıldığında her zaman dönüştürür). X çevirisi komutta **varsayılan olarak açıktır**: `auto_translate`
  kapalı olsa bile tweetler hedef dile çevrilir; hedef dil ayarı korunur.
  Discord, slash komutu etkileşiminde yanıtlanan mesajı göndermediği için "mesaja yanıt verip `/dönüştür` yazma"
  doğrudan desteklenmez; mesaj komutu bu ihtiyacın Discord'daki resmî karşılığıdır.

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
Slash ve mesaj komutları için davet bağlantısında **`applications.commands`** kapsamı da bulunmalıdır.

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

Ayar yedeği: `./backup-settings.sh` önce `BOTGEK_SETTINGS_PATH`'i kullanır.
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

- `build-release.yml`: test, fmt, Clippy, panel testleri; ardından arm64 paket ve imaj. `main` push'u ve elle başlatma `latest`'i, sürüm etiketi `X.Y.Z` + `latest`'i yayınlar; PR yalnızca derler.
- `quality.yml`: RustSec audit kod değiştiren her PR/push'ta; LCOV raporu yalnızca ana dalda ve elle başlatmada üretilir (bilgi amaçlı, eşik yok).
- `release.yml`: elle sürüm bump/etiket akışı. Yerelde `./release.sh patch` veya
  `./release.sh minor --push`. Temiz çalışma ağacı gerekir.
- Sürüm `build.rs` ile git etiketinden gömülür; `SOURCE_DATE_EPOCH` tekrar üretilebilir tarih içindir.
- Depo public olduğu için Actions artefakt/cache ve public GHCR depolaması ücretsizdir; günlük temizlik workflow'ları kaldırıldı. Artefaktlar 1 gün saklanır, cache'ler GitHub tarafından 10 GB sınırında en eski girdiler atılarak budanır.

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
- Dönüşüm satırı `toplam … ms — hedef 1 sn aşıldı` ekiyle biter; veri fazı gönderim payını
  aştığında ayrıca `Veri fazı … ms sürdü (veri …, medya …)` uyarısı yazılır. Taklit webhook'u
  zamanında hazır olmazsa `Taklit webhook'u … ms içinde hazır olmadı` bilgisi düşer.
  Bu üç satır, yavaş bir kartın hangi fazda geciktiğini (kaynak / medya / webhook / Discord POST)
  doğrudan gösterir.

Tekrarlanabilir offline CPU başlangıç ölçümü:

```sh
cargo test --release --locked offline_card_baseline -- --ignored --nocapture
```

Bu 10.000 örnekte parse+render p50/p95 ölçer; **ağ, toplam Discord gecikmesi veya RSS ölçmez**.
Gerçek Raspberry Pi üzerinde aynı sürüm/derleyiciyle sıcak-soğuk önbellek, büyük Reddit videosu,
eşzamanlı aynı/farklı URL, yavaş upstream ve 429 senaryolarını ayrıca ölçün. RSS/CPU, upstream istek sayısı,
timeout sonrası aktif iş ve p95 değerlerini değişiklik öncesi/sonrası karşılaştırın. Donanım üzerinde
ölçülmeden hızlanma veya bellek tasarrufu yüzdesi iddia edilmez.

### X: içerik farkındalıklı veri planı

VxTwitter/otomatik çeviri yolu gelen veriye göre yeniden planlanır; kart gönderimi veya
webhook taklidi değişmez. İlk soğuk okumada orijinal Vx verisi, syndication metadatası ve
çeviri açıksa dil ekli Vx yanıtı paraleldir. Metrikler açıksa ve kaynakta görüntülenme yoksa
FxTwitter da yalnızca bu eksiği tamamlamak için paralel istenir. Dil ekli yanıt spekülatiftir:
İngilizce/Türkçe metinde beklenmez; yabancı dilde ise dil tespitinden sonra ikinci bir seri
ağ turu oluşmaz. Gereksizleşen okumalar iptal edilir. Bunun bedeli, soğuk önbellekte en fazla
dört kök GET'tir; bu bir hız garantisi değil, kritik yoldaki seri beklemeleri kaldırma tercihidir.

- Yalnızca görünür, boş olmayan ve henüz çevrilmemiş metin için ek çeviri istenir.
- `qrtURL: null` ile eksik `qrtURL` birbirinden ayrılır. Bilinen üçüncü gönderi doğrudan
  çekilir; sırf keşif için orta gönderi yeniden istenmez. Bilgi yoksa kısa, best-effort
  inceleme korunur. İlk alıntının medyalı olması, bilinen üçüncü alıntının metnini düşürmez.
- Görüntülenme sayısını VxTwitter ve syndication vermez. Metrikler açıkken eksik değer
  FxTwitter'dan isteğe bağlı olarak tamamlanır; mevcut yanıt/beğeni/repost değerleri ezilmez.
  Bütçe dolarsa kart görüntülenmesiz gönderilir.
- Yanıtlar gönderi ID'siyle birleştirilir; boş/geç gelen yanıt mevcut çeviriyi, medyayı,
  topluluk notunu veya tamamlanmış alıntıyı silmez. Zaman aşımında mevcut orijinal korunur.
- Kaynak sonuçları ayrıca **600 saniye / 512 girdi** ile önbelleklenir; aynı ID başka bir
  gönderide alıntılandığında da tekrar kullanılabilir. Kaynak/dil/hedef/video bütçeleri
  ayrıdır; eşzamanlı aynı istekler tek çekimde birleşir. NSFW/spoiler kararı bu cache'e yazılmaz.
- Kaynak/çeviri/alıntı işleri **iki kademeli bütçeyi** paylaşır: kart verisi geldikten sonra
  isteğe bağlı zenginleştirme **650 ms** ile sınırlıdır, hiç veri gelmezse ilk yanıt için
  **3 saniyeye** kadar beklenir (kart büsbütün düşmesin). Ayrı ayrı tam zaman aşımı süreleri
  birikmez. Harici link önizlemesi bu bütçenin **içinde**, kaynak istekleriyle paralel çalışır;
  ayrıca 500 ms sınırı vardır.

Karar, hata, iptal ve gecikme karşılaştırmalarını tekrarlamak için:

```sh
cargo test --locked converters::builtin::twitter
cargo test --locked scripted_latency_comparison -- --ignored --nocapture
cargo test --locked twitter::benchmarks -- --ignored --nocapture
cargo test --locked live_fetch_three_level_quote -- --ignored --nocapture
```

`scripted_latency_comparison`, eski `facfe5a` akışıyla yeni planı aynı gecikmeli uç
simülasyonlarında, cache'siz ve üçer örnekle karşılaştırır; çıktı eşitliğini de doğrular.
Canlı testler salt okunurdur, Discord'a mesaj göndermez; upstream cache/konum/ağ etkileri
nedeniyle bunların süreleri kontrollü karşılaştırmadan ve üretim ölçümlerinden ayrı tutulmalıdır.
Windows'ta MSVC linker hazır değilse mevcut GNU/LLVM toolchain'iyle aynı kontroller
`cargo +stable-x86_64-pc-windows-gnullvm ...` şeklinde çalıştırılabilir; global ayar değişmez.
