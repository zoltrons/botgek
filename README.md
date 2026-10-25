<div align="center">

# Botgek

**Discord'da paylaşılan sosyal medya bağlantılarını zengin, etkileşimli kartlara dönüştüren açık kaynak bot.**

*An open-source, self-hosted Discord bot that turns social media links into native Components V2 cards.*

[![CI](https://github.com/zoltrons/botgek/actions/workflows/build-release.yml/badge.svg)](https://github.com/zoltrons/botgek/actions/workflows/build-release.yml)
[![Bağımlılık denetimi](https://github.com/zoltrons/botgek/actions/workflows/quality.yml/badge.svg)](https://github.com/zoltrons/botgek/actions/workflows/quality.yml)
[![Lisans: MIT](https://img.shields.io/badge/lisans-MIT-blue.svg)](LICENSE)
[![Rust 1.98.1](https://img.shields.io/badge/rust-1.98.1-orange.svg?logo=rust)](rust-toolchain.toml)
[![Docker Compose](https://img.shields.io/badge/docker-compose-2496ED.svg?logo=docker&logoColor=white)](docker-compose.yml)

[Rust](https://www.rust-lang.org/) · tek ikili · Docker · arm64 / Raspberry Pi

</div>

Botgek, bir kanalda desteklenen bir bağlantı gördüğünde platformdan veriyi çeker, medyayı hazırlar ve mesajı
Discord'un **Components V2** bileşenleriyle tek bir karta dönüştürür. Discord botu, dönüştürme motoru ve
yönetim paneli **tek bir Rust ikilisinde** çalışır; ffmpeg, harici veritabanı sunucusu veya ayrı bir web
servisi gerekmez.

## Öne çıkanlar

- **Native kartlar:** X/Twitter, Instagram, Reddit, Twitch, Pixiv, Tumblr, Facebook, YouTube Community
  Post ve Letterboxd bağlantıları Components V2 kartına dönüşür. TikTok, Bluesky ve Spotify native modda aynı kartı
  kullanır; bu üç platform için proxy modu da seçilebilir. Klasik embed fallback'i yoktur.
- **Saf Rust medya hattı:** Reddit videolarında görüntü ve ses saf Rust ile tek MP4'e birleştirilir;
  ffmpeg veya yerel bir yardımcı ikili kullanılmaz.
- **Gecikme bütçesi:** Kart için hedef **1 saniyedir**. Veri, medya, çeviri, alıntı ve bağlantı önizlemesi
  paralel yürür; kaynak yavaşladığında kart düşmez, eldeki veriyle gönderilir.
- **Kontrollü eşzamanlılık:** Otomatik akış 8, native dönüşüm 16, etkileşim 4 eşzamanlı işle sınırlıdır.
  Kuyruk dolduğunda orijinal mesaj korunur.
- **Dayanıklı gönderim:** Orijinal mesaj yalnızca kart başarıyla gönderildikten sonra silinir; sonucu belirsiz
  bir Discord isteği yeniden denenmez ve 429 bekleme süresi kısaltılmaz.
- **Yönetim paneli:** Global, sunucu ve kanal kapsamında modül, rol filtresi, motor ve çeviri ayarları;
  token ile korunan tek sayfa panel.
- **MCP teşhis sunucusu:** WebUI ile aynı portta `/mcp`; sürüm, metrik, log, ayar, Discord cache'i,
  salt okunur SQL ve Discord'a göndermeyen link önizlemesi. Ayrı servis veya port gerekmez.
- **İsteğe bağlı AI sohbeti:** OpenRouter üzerinden; bot etiketlendiğinde veya botun mesajına yanıt
  verildiğinde çalışır. Yanıtlanan mesajı, yanıt zincirini ve görselleri bağlam olarak okur ("@bot
  özetle", "@bot görselde ne var?"), sunucunun özel emojilerini yanıtlarında kullanır. Gizlilik için
  kullanıcı kimliği/adı gönderilmez (takma adlar), yalnızca veri saklamayan sağlayıcılar kullanılır.
  Rol filtresi, kullanıcı başına bekleme ve süreç geneli kapasite sınırı vardır.
- **Çeviri:** X gönderileri için otomatik çeviri (`tr`, `en`, `de` veya herhangi bir BCP-47 kodu);
  proxy mesajlarında bayrak tepkisiyle anlık çeviri.
- **Gizlilik kontrolleri:** Hassas içerik varsayılan olarak spoiler'dır; yalnızca NSFW kanallarında
  isteğe bağlı açılabilir.
- **Tek komut:** `/dönüştür` DM'ler dâhil her yerde çalışır; **Apps → Dönüştür** mesaj komutu aynı işi
  sağ tık menüsünden yapar.
- **Dağıtım:** Docker Compose (non-root, salt okunur kök dosya sistemi), arm64/Raspberry Pi imajı ve
  GitHub Actions CI.

## Desteklenen platformlar

| Platform | Bağlantı türü | Motor |
| --- | --- | --- |
| X (Twitter) | Gönderi, alıntı, medya | Native (VxTwitter / FxTwitter / syndication) |
| Instagram | Gönderi, Reels | Native |
| Reddit | Gönderi, video | Native (saf Rust video birleştirme) |
| Bluesky | Gönderi | Native / proxy |
| TikTok | Video | Native / proxy |
| Spotify | Parça, albüm, çalma listesi, sanatçı, kısa link | Native / proxy |
| Twitch | Klip (kart içinde video) | Native |
| Pixiv | Çizim | Native |
| Tumblr | Gönderi | Native |
| Facebook | Gönderi, video | Native |
| YouTube | Topluluk gönderisi (Community Post) | Native |
| Letterboxd | Film, kullanıcı incelemesi, kullanıcı profili, `boxd.it` kısa link | Native |

Native-only platformlarda proxy motoru ve proxy yedeği yoktur. TikTok, Bluesky ve Spotify için panelden
native veya proxy seçilebilir. X verisi için `vxtwitter` (varsayılan), `fxtwitter`, `syndication` ya da
`hybrid` kaynağı kullanılabilir.

## Hızlı başlangıç

### Docker (önerilen)

```sh
git clone https://github.com/zoltrons/botgek.git
cd botgek
cp .env.example .env
# .env içinde DISCORD_TOKEN ve ADMIN_TOKEN doldurun
docker compose up -d --build
```

Panel: <http://127.0.0.1:3000>

Güçlü bir panel tokenı üretmek için: `openssl rand -hex 32`. İlk derleme birkaç dakika sürebilir;
sonraki güncellemelerde `docker compose up -d --build` yeterlidir.

### Kaynaktan

```sh
cp .env.example .env
# .env içinde DISCORD_TOKEN ve ADMIN_TOKEN doldurun
cargo run --locked
```

Depo, Rust **1.98.1** sürümünü `rust-toolchain.toml` ile sabitler; kaynak dil tabanı Rust **1.88**'dir
(edition 2024). Linux'ta C derleyicisi/linker gerekir, çünkü SQLite bundled olarak derlenir.

### Discord kurulumu

1. Botu [Discord Developer Portal](https://discord.com/developers/applications)'da oluşturun.
2. **Bot → Privileged Gateway Intents** altında **Message Content Intent**'i açın.
3. Botu şu izinlerle davet edin: **Send Messages**, **Embed Links**, **Read Message History**,
   **Add Reactions**. Orijinal mesajlar silinecekse **Manage Messages**, kullanıcı adına gönderim
   (impersonation) kullanılacaksa **Manage Webhooks** ekleyin.
4. Davet bağlantısında `applications.commands` kapsamı bulunmalıdır; aksi hâlde slash ve mesaj komutları
   görünmez.

Ayrıntılı kurulum ve güvenlik notları: [docs/CONFIGURATION.md](docs/CONFIGURATION.md).

## Kullanım

| Yol | Ne yapar |
| --- | --- |
| `/dönüştür link:<bağlantı>` | Bağlantıyı her yerde, DM'ler dâhil dönüştürür. Açıkça çağrıldığında platform modülü kapalı olsa bile çalışır; X çevirisi varsayılan olarak açıktır. |
| Mesaja sağ tık → **Apps → Dönüştür** | O mesajdaki bağlantıyı dönüştürür. |
| `/uyum kişi:<@biri> [ikinci:<@biri>]` | İki kişinin uyum kartı: ship adı, avatarlar, skor ve alt başlıklar. Skor sabittir; karttaki seçiciyle herkes kendi uyumunu dener. Kullanıcıya sağ tık → **Apps → Uyumumuz** da aynı işi yapar. |
| `/düello rakip:<@biri>` | Gizli seçimli Taş · Kağıt · Makas; ilk iki turu alan kazanır, rövanş butonu vardır. Bota meydan okunabilir. |
| `/sayı-tahmini [zorluk]` | Bot bir sayı tutar; herkes butondan açılan pencereyle tahmin eder, kart daralan aralığı ve sıcak/soğuk ipuçlarını gösterir. |
| `/8top soru:<soru>` | Sihirli 8 top cevabı. |
| `/bomba rakip:<@biri>` | Sırayla kablo kesilir; her güvenli kablo bombanın uzaklığını (bitişik / iki kablo / uzak) söyler, bombayı kesen kaybeder. Bota meydan okunursa bot ipuçlarını okuyarak oynar. |
| `/refleks` | Işık rastgele bir anda yeşile döner; ilk basan kazanır, erken basan elenir. Kazananın tepki süresi gösterilir. |
| `/fal [kişi]` | Kişiye ve güne göre sabit kahve falı: iki fincan şekli, aşk/para/sağlık, şanslı sayı ve renk. Türkiye saatiyle gece yarısı tazelenir. |
| 🗑️ tepkisi | Kartı, kartı paylaşan kullanıcı veya **Manage Messages** yetkisi olan bir yönetici siler. |
| Bayrak tepkisi (🇹🇷, 🇬🇧, …) | Proxy mesajını seçilen dile çevirir. Native kartlar kendi çeviri ayarını kullanır. |
| @bot etiketleme veya bot mesajına yanıt | AI sohbetini tetikler (`OPENROUTER_API_KEY` ve `ai_chat` modülü açıksa). |

`/dönüştür` açıkça çağrıldığında X çevirisi her durumda uygulanır: `auto_translate` kapalı olsa bile
İngilizce dâhil tweetler hedef dile çevrilir; hedef dil ayarı korunur. Discord, slash komutu
etkileşiminde yanıtlanan mesajı göndermediği için "mesaja yanıt verip `/dönüştür` yazma" desteklenmez;
aynı ihtiyaç için **Apps → Dönüştür** kullanılır.

## Yönetim paneli

Panel varsayılan olarak `127.0.0.1:3000` üzerinde dinler ve `ADMIN_TOKEN` ile korunur. Modüller, rol
filtreleri, AI personası ve dönüştürme motoru tek sayfadan yönetilir; ayarlar global, sunucu ve kanal
kapsamında çözümlenir.

- Token **32–512 karakter** arasında, yüksek entropili bir sır olmalıdır. Token yoksa panel açılmaz;
  Discord botu çalışmaya devam eder.
- Oturum **8 saat** sürer; çerez `HttpOnly` ve `SameSite=Strict`'tir, token tarayıcı depolamasına
  yazılmaz. Durum değiştiren istekler `X-Botgek-Request: 1` başlığı ister.
- İstemci adresi başına dakikada **20** başarısız girişten sonra giriş reddedilir; eşzamanlı oturum
  sayısı **64** ile sınırlıdır.
- Uzaktan erişimde **HTTPS ters proxy** veya SSH tüneli kullanın; panel TLS sonlandırmaz. HTTPS için
  `WEB_SECURE_COOKIE=true` ayarlayın.

## MCP sunucusu (geliştirme ve teşhis)

Bot, WebUI ile aynı adreste ve portta `/mcp` yolunda bir
[Model Context Protocol](https://modelcontextprotocol.io) sunucusu yayınlar. Geliştirme ve teşhis
sırasında sürüm, readiness, metrikler, son loglar, ayarlar, Discord cache'i, salt okunur SQL ve link
önizlemesi ayrı bir servis veya port gerektirmeden tek noktadan okunur.

- Endpoint `MCP_TOKEN` ile korunur; token boş veya geçersizse kapalıdır (**503**). `MCP_TOKEN`,
  `ADMIN_TOKEN`'dan farklı ve **32–512 karakter** olmalıdır.
- `settings_patch` kalıcıdır ve botu anında etkiler; `db_query` salt okunurdur; `convert_preview`
  Discord'a mesaj göndermez.

| Araç | Ne verir |
| --- | --- |
| `bot_status` | Sürüm, uptime, Discord readiness, veritabanı/ayar durumu. |
| `bot_metrics` | Dönüşüm kuyruğu, platform başarı/hata ve gecikme kovaları. |
| `bot_diagnostics` | REST 429/retry/status, fazlar, host, cache, SQLite, süreç metrikleri. |
| `bot_logs` | Bellekteki son loglar (en fazla 512 olay). |
| `settings_get` | Etkin ayarlar ve içerik revizyonu. |
| `settings_patch` | JSON merge patch; botu anında etkiler. |
| `discord_overview` | Serenity cache'indeki sunucu/kanal/rol listesi. |
| `db_stats` | Kart veritabanı yolu, kayıt sayısı ve dosya boyutu. |
| `db_query` | Tek bir salt okunur `SELECT`/`WITH` (en fazla 200 satır). |
| `convert_preview` | Bağlantıyı çözer, verisini çeker ve render planını döner. |
| `discord_test_card` | Test sunucusunda gerçek kart gönderir; REST/medya telemetrisi için. |

Kurulum, DSH istemci kaydı, curl/PowerShell örnekleri, sınırlar ve sorun giderme:
[docs/MCP.md](docs/MCP.md).

## Yapılandırma

Tüm ayarlar ortam değişkenleri ve `settings.json` üzerinden yapılır. En sık kullanılanlar:

| Değişken | Zorunlu | Açıklama |
| --- | --- | --- |
| `DISCORD_TOKEN` | Evet | Discord bot tokenı. |
| `ADMIN_TOKEN` | Panel için | 32–512 karakterlik yönetim sırrı. |
| `MCP_TOKEN` | MCP için | `/mcp` endpoint'inin Bearer tokenı; boşsa endpoint kapalı. |
| `WEB_PORT` / `WEB_BIND` | Hayır | Panel portu ve bağlanma adresi (varsayılan `3000` / `127.0.0.1`). |
| `OPENROUTER_API_KEY` | Hayır | AI sohbetini etkinleştirir. |
| `DELETE_ORIGINAL` | Hayır | Başarılı dönüşümden sonra orijinal mesajı siler. |
| `BOTGEK_DB_PATH` | Hayır | Kart kayıtları için SQLite yolu (varsayılan `botgek.db`). |
| `BOTGEK_SETTINGS_PATH` | Hayır | Ayar dosyası yolu; Docker'da `/app/data/settings.json`. |
| `FXTUMBLR_DOMAIN` | Hayır | Tumblr render hizmeti (varsayılan `tpmblr.com`). |
| `INSTAGRAM_SESSIONID` | Hayır | Reels videolarının kart içinde oynaması için Instagram oturum çerezi. |
| `REDDIT_CLIENT_ID` | Hayır | Reddit OAuth istemci kimliği (sır değildir). |

Değişkenlerin tamamı, `settings.json` şeması, kapsam çözümleme ve kalıcılık:
[docs/CONFIGURATION.md](docs/CONFIGURATION.md).

## Mimari

```text
Discord mesajı
      │
      ▼
  parser ──► rol filtresi / modül kontrolü ──► dönüştürücü
                                                  ├─ kaynak verisi (Vx/Fx, platform API'si, OG)
                                                  ├─ medya, çeviri, alıntı (paralel + bütçeli)
                                                  └─ Components V2 kartı
      ◄──────────────────────────────────────────── Discord REST
```

| Bileşen | Sorumluluk |
| --- | --- |
| `src/handler.rs` | Gateway olayları, komutlar, tepkiler, AI tetikleme. |
| `src/converters/` | URL ayrıştırma, platform dönüştürücüleri, medya, çeviri, kart üretimi. |
| `src/settings.rs` | Ayar şeması, kapsam çözümleme, doğrulama ve göç. |
| `src/store.rs` | SQLite tabanlı kart–paylaşan kaydı. |
| `src/web/` | Axum API'si, kimlik doğrulama ve panel. |
| `src/mcp.rs` | `/mcp` MCP sunucusu, kimlik doğrulama ve araçlar. |
| `src/logs.rs` | MCP için 512 olaylık bellek içi log halkası. |
| `src/ai/` | OpenRouter istemcisi. |

Tasarım kararları, istek yaşam döngüsü ve bütçe ayrıntıları: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Performans ve güvenilirlik

- Kart hedefi **1 sn**; veri geldikten sonra isteğe bağlı zenginleştirme **650 ms**, webhook beklemesi en
  fazla **200 ms**, Discord POST'una **250 ms** pay ayrılır. Hiç veri gelmezse ilk yanıt için **3 sn**
  beklenir.
- Gönderi ve kaynak önbelleği **600 sn / 512 girdi** ile sınırlıdır; aynı anahtara gelen istekler tek
  çekimde birleştirilir.
- X bağlantı önizlemesi yalnızca public HTTP(S), 80/443 portları ve doğrulanan DNS adreslerine gider;
  yönlendirmeler yeniden doğrulanır ve toplam bütçe **500 ms**'dir.
- SQLite WAL kullanılır; kart kayıtları **200.000** girdiyle sınırlıdır ve otomatik budanır.
- Bu değerler yapılandırılmış sınırlardır; ölçülmüş p95 veya bellek garantisi değildir.

İşletim, izleme ve ölçüm ayrıntıları: [docs/OPERATIONS.md](docs/OPERATIONS.md).

## Geliştirme

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
node --check src/web/ui/app.js
node --check src/web/ui/login.js
node --test tests/web-ui.test.cjs
```

Normal test koşusu ağdaki sosyal medya servislerine bağlanmaz; HTTP testleri loopback üzerindeki fixture
sunucularını kullanır. Canlı testler `--ignored` ile açıkça çalıştırılır. Ayrıntılar:
[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

## Dokümantasyon

| Belge | İçerik |
| --- | --- |
| [docs/CONFIGURATION.md](docs/CONFIGURATION.md) | Ortam değişkenleri, `settings.json`, kapsamlar, yönetim paneli güvenliği. |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Modül haritası, istek yaşam döngüsü, bütçeler, önbellek, hata semantiği. |
| [docs/OPERATIONS.md](docs/OPERATIONS.md) | Docker, yedekleme, sağlık uçları, loglama, performans ölçümü, CI/CD. |
| [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) | Yerel kurulum, testler, canlı testler, sürüm akışı, yeni platform ekleme. |
| [docs/MCP.md](docs/MCP.md) | MCP endpoint'i, istemci kaydı, araçlar, sınırlar ve sorun giderme. |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Katkı akışı ve PR beklentileri. |
| [SECURITY.md](SECURITY.md) | Güvenlik açığı bildirimi ve dağıtım sertleştirme listesi. |

## Katkıda bulunma

Hata bildirimleri ve özellik önerileri için GitHub Issues kullanılır. Pull request göndermeden önce
[CONTRIBUTING.md](CONTRIBUTING.md) dosyasını okuyun; değişikliklerin `cargo fmt`, `cargo clippy` ve test
kontrollerinden geçmesi beklenir. Güvenlik açıklarını lütfen herkese açık issue olarak değil,
[SECURITY.md](SECURITY.md) içindeki yolla bildirin.

## Lisans

[MIT](LICENSE) © 2026 Eray
