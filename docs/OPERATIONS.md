# İşletim

Bu belge dağıtım, güncelleme, yedekleme, izleme ve performans ölçümü pratiklerini kapsar. Yapılandırma
anahtarları için [CONFIGURATION.md](CONFIGURATION.md) dosyasına bakın.

## Docker ile dağıtım

Depodaki `docker-compose.yml` imajı kaynak koddan derler ve şu güvenlik önlemlerini uygular:

- Container root olmayan `1000:1000` kullanıcısıyla çalışır.
- Kök dosya sistemi salt okunurdur; yalnızca `/app/data` volume'u yazılabilir. `/tmp` 16 MB'lık tmpfs'tir.
- Tüm Linux yetenekleri bırakılır (`cap_drop: ALL`) ve `no-new-privileges` açıktır.
- Panel portu yalnızca **host 127.0.0.1** üzerinde yayınlanır; container içinde `WEB_BIND=0.0.0.0`
  kullanılır.
- Bellek sınırı 256 MB'tır; json-file logları 10 MB × 3 dosya ile döndürülür.
- `restart: unless-stopped` etkindir.

Uzak makinede panele erişmek için SSH tüneli kullanın:

```sh
ssh -L 3000:127.0.0.1:3000 kullanici@sunucu
```

Üretimde HTTPS için bir ters proxy (Caddy, Nginx, Traefik) koyun ve `WEB_SECURE_COOKIE=true` ayarlayın.

### Güncelleme

```sh
git pull
docker compose up -d --build
```

`docker compose pull` yalnızca `image:` kullanan bir Compose tanımı için yeni imajı çeker; bu dosyada imaj
`build:` ile üretilir. Yayınlanmış GHCR imajına geçildiğinde `pull && up -d` kullanılabilir.

### Mevcut kurulumdan geçiş

- Kök dizindeki `settings.json`, salt okunur `/app/settings.legacy.json` olarak bağlanır.
- İlk açılışta `/app/data/settings.json` yoksa bu dosya içe aktarılır.
- Sonraki panel kayıtları `botgek-data` volume'undaki dosyaya gider; hosttaki eski dosya artık
  güncellenmez.
- Güncellemeden önce ayar ve SQLite dosyalarının yedeğini alın.
- `docker compose down -v` kalıcı volume'u siler; normal güncellemede kullanmayın.

Volume adı varsayılan olarak `botgek_botgek-data`'dır ve proje dizini değişse bile sabit kalır. Başka bir
mevcut volume kullanıyorsanız `.env` içine `BOTGEK_DATA_VOLUME=<gerçek-volume-adı>` yazın. Bot
kullanıcısına (Docker: UID/GID 1000) dizin yazma izni verin; `chmod 666` kullanmayın.

### Yedekleme ve geri yükleme

```sh
# Ayar dosyası (host kurulumu)
./backup-settings.sh

# Docker
docker compose exec botgek cat /app/data/settings.json > settings.json.bak

# SQLite (tutarlı kopya için container'ı kısa süre durdurun)
docker compose stop botgek
docker run --rm -v botgek_botgek-data:/data -v "$PWD":/backup alpine \
  tar czf /backup/botgek-data.tar.gz -C /data .
docker compose start botgek
```

Geri yüklemede güncel ayar dosyasını `BOTGEK_SETTINGS_PATH` ile gösterilen yola koyun.

## Sağlık uçları ve izleme

| Uç | Kimlik doğrulama | Anlamı |
| --- | --- | --- |
| `GET /api/health` | Yok | Public liveness; yalnızca HTTP sunucusunun ayakta olduğunu söyler. `OK` döner. |
| `GET /api/ready` | Gerekir | Discord bağlantısını ve kalıcı SQLite erişimini kontrol eder; bozuk durumda `503`. |
| `GET /api/metrics` | Gerekir | Platform bazlı native başarı/hata, kümülatif gecikme histogramı, cache-hit ve kapasite aşımı; ayrıca Discord REST rota/status/429/retry, dönüşüm fazları, upstream hostları, önbellek, SQLite, gateway ve süreç/kaynak metrikleri. MCP `bot_diagnostics` bölüm bazında okur. Süreç yeniden başlayınca sıfırlanır; kullanıcı/veri/token içermez. |
| `POST /mcp` | `MCP_TOKEN` (Bearer) | Geliştirme/teşhis MCP endpoint'i; token boşsa `503`. Ayrıntılar: [MCP.md](MCP.md). |

`/api/health` tek başına botun Discord'a bağlı olduğunu kanıtlamaz; orkestrasyon readiness için
`/api/ready` kullanmalıdır.

### Loglama

`RUST_LOG=botgek=debug,serenity=warn` ile veri/medya/gönderim süreleri görünür. Loglar gönderi URL'leri
içerebilir; özel tutun. Discord 429 rate limit ve 2 saniyeyi aşan REST çağrıları `warn` seviyesinde
ayrıca loglanır.

Dönüşüm satırı `toplam … ms — hedef 1 sn aşıldı` ekiyle biter. Veri fazı gönderim payını aştığında
`Veri fazı … ms sürdü (veri …, medya …)` uyarısı, taklit webhook'u zamanında hazır olmadığında
`Taklit webhook'u … ms içinde hazır olmadı` bilgisi yazılır. Bu üç satır yavaş bir kartın hangi fazda
geciktiğini (kaynak / medya / webhook / Discord POST) gösterir.

## Performans ölçümü

Tekrarlanabilir offline CPU başlangıç ölçümü:

```sh
cargo test --release --locked offline_card_baseline -- --ignored --nocapture
```

Bu test 10.000 örnekte parse+render p50/p95 ölçer; **ağ, toplam Discord gecikmesi veya RSS ölçmez**.

Gerçek Raspberry Pi üzerinde aynı sürüm ve derleyiciyle şu senaryoları ayrıca ölçün:

- sıcak ve soğuk önbellek,
- büyük Reddit videosu,
- aynı ve farklı URL'lerde eşzamanlı yük,
- yavaş upstream,
- 429 yanıtı.

RSS/CPU, upstream istek sayısı, timeout sonrası aktif iş ve p95 değerlerini değişiklik öncesi/sonrası
karşılaştırın. Donanım üzerinde ölçülmeden hızlanma veya bellek tasarrufu yüzdesi iddia edilmez.

X akışının karar, hata, iptal ve gecikme karşılaştırmalarını tekrarlamak için:

```sh
cargo test --locked converters::builtin::twitter
cargo test --locked scripted_latency_comparison -- --ignored --nocapture
cargo test --locked twitter::benchmarks -- --ignored --nocapture
cargo test --locked live_fetch_three_level_quote -- --ignored --nocapture
```

`scripted_latency_comparison`, eski `facfe5a` akışıyla yeni planı aynı gecikmeli uç simülasyonlarında,
cache'siz ve üçer örnekle karşılaştırır; çıktı eşitliğini de doğrular. Canlı testler salt okunurdur,
Discord'a mesaj göndermez; upstream cache/konum/ağ etkileri nedeniyle süreleri kontrollü
karşılaştırmadan ayrı tutulmalıdır.

## CI/CD

| Workflow | Ne zaman | Ne yapar |
| --- | --- | --- |
| `build-release.yml` | `main` push, `v*` etiketi, PR, elle | Test, fmt, Clippy, panel testleri; ardından arm64 paket ve Docker imajı. PR'lar yalnızca derler. |
| `quality.yml` | Kod değiştiren PR/push | RustSec `cargo audit`; ana dalda ve elle başlatmada bilgi amaçlı LCOV kapsam raporu (eşik yok). |
| `release.yml` | Elle | Sürüm bump'ı, `Cargo.toml`/`Cargo.lock` güncellemesi, testler, `chore(release): vX.Y.Z` commit'i, annotated tag ve yayın zinciri. |

- `main` push'u ve `main` dalından elle başlatma `latest` imajını; `vX.Y.Z` etiketi `X.Y.Z` + `latest`
  etiketlerini yayınlar. Özellik dalından elle başlatma yalnızca derler, imajı yayınlamaz.
- Sürüm `build.rs` ile git etiketinden gömülür; `SOURCE_DATE_EPOCH` tekrar üretilebilir tarih içindir.
- Kaynak imaj derlemesinde `.git` bulunmaz. Sürüm künyesini `BOTGEK_VERSION` ve `BOTGEK_SHA` build
  argümanlarıyla verin.
- Runtime imajı root olmayan kullanıcıdır; ffmpeg kurulmaz ve kullanılmaz.
- Depo public olduğu için Actions artefakt/cache ve public GHCR depolaması ücretsizdir. Artefaktlar
  1 gün saklanır; cache'ler GitHub tarafından 10 GB sınırında en eski girdiler atılarak budanır.

Yerel sürüm akışı:

```sh
./release.sh patch
./release.sh minor --push
```

Temiz bir çalışma ağacı gerekir.

## TLS ve bağımlılık notu

Serenity 0.12'nin Rustls gateway arka ucu eski `rustls-webpki 0.102` sürümünü sabitlediği için
Serenity'nin desteklenen **native-TLS** arka ucu kullanılır; Linux/ARM'da OpenSSL vendored derlenir
(build ortamında C toolchain, Perl ve make gerekir), Windows'ta OS TLS kullanılır. Botun kendi HTTP
istemcileri **Rustls 0.23** kullanmayı sürdürür.

`RUSTSEC-2026-0049/0098/0099/0104` istisnayla susturulmaz. Upstream gateway güncellendiğinde bu uyumluluk
tercihi yeniden değerlendirilmelidir. Bağımlılık denetiminin gerçekten geçtiğini ilgili CI sonucu olmadan
varsaymayın.

## Sorun giderme

| Belirti | Olası neden | Çözüm |
| --- | --- | --- |
| Panel açılmıyor | `ADMIN_TOKEN` yok, 32 karakterden kısa veya geçersiz `WEB_BIND` | `.env` değerini düzeltin, botu yeniden başlatın; logdaki `WebUI disabled` satırına bakın. |
| `/api/ready` 503 | SQLite açılamıyor veya Discord bağlantısı kopuk | Volume izinlerini ve logları kontrol edin; bellek içi fallback kalıcı değildir. |
| Kart gönderilmiyor | Message Content Intent kapalı, eksik izin veya kapasite dolu | Portal izinlerini ve `kapasite` loglarını kontrol edin. |
| Orijinal mesaj silinmiyor | **Manage Messages** yok veya `DELETE_ORIGINAL=false` | İzni verin veya ayarı açın. |
| Reels videosu kart içinde oynamıyor | `INSTAGRAM_SESSIONID` boş veya süresi dolmuş | Yeni `sessionid` çerezi girin; süre dolduğunda logda oturum hatası görünür. |
| Komutlar görünmüyor | Davet bağlantısında `applications.commands` yok | Botu doğru kapsamla yeniden davet edin. |
