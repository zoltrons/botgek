# MCP sunucusu

Botgek, WebUI ile **aynı adreste ve portta** `/mcp` yolunda bir
[Model Context Protocol](https://modelcontextprotocol.io) (Streamable HTTP) sunucusu yayınlar.
Geliştirme ve teşhis sırasında sürüm, readiness, metrikler, son loglar, ayarlar, Discord cache'i,
kart veritabanı ve link önizlemesine ayrı bir kabuk, `docker exec` veya Discord üzerinden değil, MCP
istemcisinden tek noktadan erişilir. Ayrı container, port veya servis yoktur.

Kısa özet ve araç listesi: [README](../README.md). Yapılandırma anahtarları:
[CONFIGURATION.md](CONFIGURATION.md).

## Gereksinimler

- Çalışan bot ve WebUI. WebUI `ADMIN_TOKEN` olmadan açılmadığı için MCP de açılmaz.
- `.env` içinde `MCP_TOKEN`: **32–512 karakter**, başında/sonunda boşluk olmadan ve
  `ADMIN_TOKEN`'dan **farklı**. Boş veya geçersizse endpoint kapalı kalır (**503**); WebUI ve
  Discord botu etkilenmez.
- Docker kullanılıyorsa Compose `MCP_TOKEN` değişkenini container'a aktarır; ek port eşlemesi
  gerekmez.
- HTTP POST ve Bearer header destekleyen bir MCP istemcisi.

## Kurulum

1. Ayrı bir token üretin ve `.env` dosyasına yazın:

   ```sh
   openssl rand -hex 32
   ```

   ```dotenv
   MCP_TOKEN=<openssl rand -hex 32 çıktısı>
   ```

   Tokenı Git'e, sohbete, loglara veya ekran görüntüsüne koymayın. `.env` zaten `.gitignore`
   kapsamındadır.

2. Botu başlatın:

   ```sh
   docker compose up -d --build
   # veya yerelde:
   cargo run --locked
   ```

3. Endpoint'i doğrulayın:

   ```sh
   BASE=http://127.0.0.1:3000/mcp

   curl -s "$BASE" \
     -H "Authorization: Bearer $MCP_TOKEN" \
     -H "Content-Type: application/json" \
     -H "MCP-Protocol-Version: 2026-07-28" \
     -d '{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}'
   ```

   Beklenen yanıt: `result.supportedVersions` içinde `2026-07-28` ve
   `result._meta["io.modelcontextprotocol/serverInfo"].name` değeri `botgek`.

Token değiştirildikten sonra bot yeniden başlatılmalıdır.

## Endpoint ve kimlik doğrulama

| Özellik | Davranış |
| --- | --- |
| URL | `http://<bot-host>:<WEB_PORT>/mcp`; varsayılan `http://127.0.0.1:3000/mcp`. |
| Metot | Yalnızca `POST`. `GET`/`DELETE` → **405** ve `Allow: POST`. |
| Kimlik doğrulama | `Authorization: Bearer <MCP_TOKEN>`; eksik/yanlış → **401** ve `WWW-Authenticate`. |
| Kapalı endpoint | `MCP_TOKEN` tanımsız veya geçersiz → **503**. |
| Origin | İstekte `Origin` varsa `Host` ile aynı olmalı; değilse **403** (DNS rebinding koruması). |
| Sürüm | Modern akışta `MCP-Protocol-Version: 2026-07-28` başlığı ile gövdedeki `params._meta["io.modelcontextprotocol/protocolVersion"]` aynı olmalı; çelişki **400**. |
| Bildirimler | `notifications/*` veya `id` alanı olmayan istekler **202 Accepted** döner. |

Sunucu hem `2026-07-28` oturumsuz akışını (`server/discover`) hem de eski `initialize` +
`notifications/initialized` el sıkışmasını destekler. Kabul edilen eski sürümler:
`2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05`.

## İstemci kaydı (DSH)

`~/.dsh/profiles/desktop/cordis.patch.yml` gibi profil dosyasına:

```yaml
- insert:
    - id: mcp-botgek
      name: '@deepseek-ai/dsh-mcp-client'
      config:
        serverName: botgek
        transport: streamable-http
        url: http://<bot-host>:<WEB_PORT>/mcp
        headers:
          Authorization: "Bearer <MCP_TOKEN>"
        toolCallTimeoutMs: 60000
        failOnStartupError: false
```

DSH yeniden başlatıldığında araçlar `mcp__botgek__bot_status` gibi `mcp__botgek__*` adıyla görünür.

Tokenı YAML'a gömmek istemiyorsanız istemci ortamında `MCP_TOKEN` tanımlayıp şu satırı kullanın:

```yaml
headers: !!js "(process.env.MCP_TOKEN ? { Authorization: 'Bearer ' + process.env.MCP_TOKEN } : {})"
```

Profil dosyası makineye özeldir; token depoya commit edilmez.

## Çağrı

Her JSON-RPC isteği tek bir HTTP POST'tur. Gövde bir istek veya bildirimdir; yanıt
`application/json` olarak döner.

Gerekli başlıklar:

- `Authorization: Bearer <MCP_TOKEN>`
- `Content-Type: application/json`
- Modern akışta `MCP-Protocol-Version: 2026-07-28`

```sh
BASE=http://127.0.0.1:3000/mcp

# Keşif: sürümler, yetenekler, sunucu kimliği
curl -s "$BASE" \
  -H "Authorization: Bearer $MCP_TOKEN" \
  -H "Content-Type: application/json" \
  -H "MCP-Protocol-Version: 2026-07-28" \
  -d '{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}'

# Araç listesi
curl -s "$BASE" \
  -H "Authorization: Bearer $MCP_TOKEN" \
  -H "Content-Type: application/json" \
  -H "MCP-Protocol-Version: 2026-07-28" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}'

# Araç çağrısı
curl -s "$BASE" \
  -H "Authorization: Bearer $MCP_TOKEN" \
  -H "Content-Type: application/json" \
  -H "MCP-Protocol-Version: 2026-07-28" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"bot_status","arguments":{},"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}'
```

Yanıt zarfı `{"jsonrpc":"2.0","id":…,"result":…}` biçimindedir. Araç çıktısı
`result.content[0].text` içinde JSON metni olarak döner; `result.isError` hata durumunu söyler.
Araç hatası HTTP hatası değildir: protokol yanıtı `200` döner, hata `isError: true` ile taşınır.

Windows PowerShell 5.1, `curl.exe -d` argümanındaki JSON tırnaklarını bozabilir ve yanıtı Latin-1
çözebilir. Bu ortamda gövdeyi `ConvertTo-Json` ile üretip `Invoke-RestMethod` kullanın (veya Git
Bash/WSL/PowerShell 7):

```powershell
$headers = @{
  Authorization          = "Bearer $env:MCP_TOKEN"
  "MCP-Protocol-Version" = "2026-07-28"
}
$body = @{
  jsonrpc = "2.0"; id = 3; method = "tools/call"
  params  = @{
    name      = "bot_status"
    arguments = @{}
    _meta     = @{ "io.modelcontextprotocol/protocolVersion" = "2026-07-28" }
  }
} | ConvertTo-Json -Depth 8
Invoke-RestMethod -Uri "http://127.0.0.1:3000/mcp" -Method Post -Headers $headers `
  -ContentType "application/json" -Body $body
```

## Araçlar

| Araç | Amaç |
| --- | --- |
| `bot_status` | Sürüm/künye, uptime, Discord readiness, veritabanı/ayar durumu ve sır içermeyen yapılandırma özeti. |
| `bot_metrics` | Dönüşüm kuyruğu, platform bazlı başarı/hata ve gecikme kovaları. |
| `bot_diagnostics` | Ayrıntılı telemetri: REST rota/status/429/retry, fazlar, upstream hostları, önbellek, SQLite, gateway ve süreç/kaynak metrikleri (`section` ile filtrelenir). |
| `bot_logs` | Bellekteki son tracing logları (`limit`, `min_level`; en fazla 512 olay). |
| `settings_get` | Etkin ayarlar ve içerik revizyonu. |
| `settings_patch` | JSON merge patch; aynı normalizasyonla diske yazar ve botu **anında** etkiler. |
| `discord_overview` | Serenity cache'indeki sunucu/kanal/rol listesi (Discord'a istek atmaz). |
| `db_stats` | Kart veritabanı yolu, kayıt sayısı, dosya boyutu ve 200.000 kayıt sınırı. |
| `db_query` | Tek bir **salt okunur** `SELECT`/`WITH`; en fazla 200 satır. |
| `convert_preview` | Bağlantıyı çözer, verisini çeker ve render planını döner; **Discord'a mesaj göndermez**. |

Varsayılanlar: `bot_logs.limit=120`, `bot_logs.min_level=info`, `db_query.limit=50`,
`convert_preview` için 20 saniye veri çekme bütçesi.

### Telemetri kapsamı

`bot_diagnostics` bölümleri:

- `rest`: rota sınıfı başına istek/deneme, 2xx/4xx/5xx, 429, retry, rate-limit beklemesi,
  toplam ve deneme gecikmesi, durum kodu dağılımı, yavaş çağrılar.
- `pipeline`: kuyruk beklemesi, veri, render, medya, webhook çözümleme/bekleme/gönderim,
  kart gönderimi, silme, reaksiyon ve toplam süre histogramları.
- `http`: VxTwitter, FxTwitter, syndication, Discord, Twitter CDN, Instagram, Reddit,
  Spotify ve diğer hostlar için istek/başarı/hata/bayt/gecikme.
- `cache`: post, source ve webhook önbellekleri için hit/miss/expired/put/evict/negative.
- `db`: SQLite işlem türü başına çağrı/hata/gecikme; `db_stats` ayrıca WAL/SHM boyutu verir.
- `gateway`: olay sayıları, son olay yaşı.
- `process`: RSS/peak RSS, sanal bellek, thread, açık fd, I/O, CPU, cgroup bellek/CPU/pids.
- `conversion`: mevcut platform başarı/hata ve kuyruk sayaçları.

429 ve 2 saniyeyi aşan Discord REST çağrıları `warn` seviyesinde loglanır; böylece
`bot_logs` ile metrikler aynı olayı gösterebilir.

## Sınırlar ve güvenlik

- `db_query` yalnızca tek bir `SELECT`/`WITH` kabul eder; yazma ifadeleri reddedilir, BLOB içeriği
  dökülmez ve sonuç en fazla 200 satır döner.
- `settings_patch` kalıcıdır ve botu anında etkiler. Eşzamanlı panel kayıtlarına karşı
  `expected_revision` verin; revizyon uyuşmazsa yazma reddedilir.
- `convert_preview` yalnızca veri ve render planı üretir; Discord'a hiçbir mesaj göndermez.
- `bot_logs` süreç ömrüyle sınırlıdır ve URL içerebilir; logları paylaşırken dikkat edin.
- Endpoint, WebUI'nin bağlandığı adresi (`WEB_BIND`, varsayılan `127.0.0.1`) kullanır. Uzaktan
  erişimde SSH tüneli veya HTTPS ters proxy kullanın; MCP portunu doğrudan internete açmayın.
- `MCP_TOKEN` ile `ADMIN_TOKEN` ayrı sırlar olmalıdır. Token sabit zamanlı karşılaştırılır ve
  yanıtlarda/loglarda görünmez.

## Sorun giderme

| Belirti | Olası neden |
| --- | --- |
| `401 Unauthorized` | `Authorization: Bearer <token>` eksik/yanlış veya token yeniden başlatma sonrası değişmiş. |
| `503 Service Unavailable` | `MCP_TOKEN` tanımsız veya geçersiz (32–512 karakter); bot `.env` dosyasını okumamış olabilir. |
| `400 … JSON çözümlenemedi` | Gövde geçerli JSON değil; PowerShell 5.1 tırnakları bozmuş olabilir. |
| `400 … protocolVersion uyuşmuyor` | `MCP-Protocol-Version` başlığı ile gövdedeki `_meta` sürümü farklı. |
| `403` | `Origin` başlığı `Host` ile uyuşmuyor. |
| `405 Method Not Allowed` | `POST` dışında bir metot kullanılmış. |
| `404` (modern akış) | Desteklenmeyen MCP metodu; eski akışta aynı hata `200` + JSON-RPC hatası olarak döner. |
| Boş/eksik araç listesi | Yanlış endpoint veya token; istemci sunucuya bağlanamamış. |

## Geliştirme ve test

Kimlik doğrulama, `tools/list`, `tools/call`, `settings_patch` ve eski `initialize` akışını kapsayan
entegrasyon testi:

```sh
cargo test --locked web::tests::mcp_endpoint_authenticates_and_serves_tools
```

Tam kontrol:

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```
