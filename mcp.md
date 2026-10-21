# Botgek MCP sunucusu

Bot, WebUI ile **aynı adreste ve portta** `/mcp` yolunda bir MCP (Model Context Protocol,
Streamable HTTP) sunucusu yayınlar. Geliştirme ve teşhis sırasında sürüm, readiness,
metrikler, son loglar, ayarlar, Discord cache'i, kart veritabanı ve Discord'a göndermeyen
link önizlemesi buradan okunur.

Ayrı container, ayrı port veya ek servis yoktur: endpoint WebUI ile birlikte açılır.
Kullanıcıya dönük özet README'nin **MCP sunucusu** bölümündedir; bu dosya kurulum, istemci
kaydı, çağrı biçimi ve araçlar için operasyonel başvurudur.

## Gereksinimler

- Çalışan bot ve WebUI. WebUI `ADMIN_TOKEN` olmadan açılmadığı için MCP de açılmaz.
- `.env` içinde `MCP_TOKEN`: 32–512 karakter, baş/son boşluk yok ve `ADMIN_TOKEN`'dan
  **farklı** olmalı.
- Docker kullanılıyorsa Compose `MCP_TOKEN`'ı container'a aktarır; ek port eşlemesi
  gerekmez (`docker-compose.yml`).
- İstemci tarafında HTTP POST ve Bearer header destekleyen bir MCP istemcisi.

## Kurulum

1. Token üretin ve `.env` içine yazın:

   ```sh
   openssl rand -hex 32
   ```

   ```dotenv
   MCP_TOKEN=<openssl rand -hex 32 çıktısı>
   ```

   Tokenı Git'e, sohbete veya loglara koymayın. `.env` zaten `.gitignore` kapsamındadır.

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

`MCP_TOKEN` boş veya geçersizse endpoint **503** döner ve kapalı kalır; WebUI ile Discord
botu etkilenmez. Tokenı değiştirdikten sonra botu yeniden başlatın.

## Endpoint ve kimlik doğrulama

- **URL:** `http://<bot-host>:<WEB_PORT>/mcp`; varsayılan yerel adres
  `http://127.0.0.1:3000/mcp`.
- **Metot:** yalnızca `POST`. `GET`/`DELETE` → **405**.
- **Kimlik doğrulama:** `Authorization: Bearer <MCP_TOKEN>`; eksik/yanlış → **401**.
- **Origin:** istekte `Origin` varsa `Host` ile aynı olmalı; değilse **403** (DNS
  rebinding koruması).
- **Sürüm:** modern akışta `MCP-Protocol-Version: 2026-07-28` başlığı ve gövdedeki
  `params._meta["io.modelcontextprotocol/protocolVersion"]` aynı değeri taşımalıdır.
  Eski istemciler `initialize` + `notifications/initialized` akışını kullanabilir; sunucu
  iki akışı da destekler.

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

DSH yeniden başlatıldığında araçlar `mcp__botgek__bot_status` gibi `mcp__botgek__*`
adıyla görünür.

Tokenı YAML'a gömmek istemiyorsanız istemci ortamında `MCP_TOKEN` tanımlayıp şu satırı
kullanın:

```yaml
headers: !!js "(process.env.MCP_TOKEN ? { Authorization: 'Bearer ' + process.env.MCP_TOKEN } : {})"
```

Profil dosyası makineye özeldir; token repo'ya commit edilmez.

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

Yanıt zarfı `{"jsonrpc":"2.0","id":...,"result":...}` biçimindedir. Araç çıktısı
`result.content[0].text` içinde JSON metni olarak döner; `result.isError` hata durumunu
söyler. Araç hatası HTTP hatası değildir: protokol yanıtı `200` döner, hata `isError: true`
ile taşınır.

Windows PowerShell 5.1, `curl.exe -d` argümanındaki JSON tırnaklarını bozabilir ve yanıtı
Latin-1 çözebilir. Bu ortamda gövdeyi `ConvertTo-Json` ile üretip `Invoke-RestMethod`
kullanın (veya Git Bash/WSL/PowerShell 7):

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
Invoke-RestMethod -Uri "http://127.0.0.1:3000/mcp" -Method Post -Headers $headers -ContentType "application/json" -Body $body
```

## Araçlar

| Araç | Amaç |
| --- | --- |
| `bot_status` | Sürüm/künye, uptime, Discord readiness, DB/ayar durumu, sır içermeyen config özeti |
| `bot_metrics` | Dönüşüm kuyruğu, platform başarı/hata ve gecikme kovaları |
| `bot_logs` | Bellekteki son tracing logları (`limit`, `min_level`; en fazla 512 olay) |
| `settings_get` | Etkin ayarlar + içerik revizyonu |
| `settings_patch` | JSON merge patch; aynı normalizasyonla diske yazar, botu anında etkiler |
| `discord_overview` | Serenity cache'indeki sunucu/kanal/rol listesi (Discord'a istek atmaz) |
| `db_stats` | Kart veritabanı yolu, kayıt sayısı, dosya boyutu |
| `db_query` | Tek bir salt okunur `SELECT`/`WITH` (en fazla 200 satır) |
| `convert_preview` | Bağlantıyı çözer, veriyi çeker, render planını döner; Discord'a mesaj göndermez |

Varsayılanlar: `bot_logs.limit=120`, `bot_logs.min_level=info`, `db_query.limit=50`,
`convert_preview` için 20 saniye veri çekme bütçesi.

## Sınırlar ve güvenlik

- `db_query` yalnızca tek `SELECT`/`WITH` kabul eder; yazma ifadeleri reddedilir, BLOB
  içeriği dökülmez. Sonuç en fazla 200 satır döner.
- `settings_patch` kalıcıdır ve botu anında etkiler. Eşzamanlı panel kayıtlarına karşı
  `expected_revision` verin; çakışmada yazma reddedilir.
- `convert_preview` yalnızca veri ve render planı üretir; Discord'a hiçbir mesaj göndermez.
- `bot_logs` süreç ömrüyle sınırlıdır ve URL içerebilir; logları paylaşırken dikkat edin.
- Endpoint WebUI'nin bind adresini kullanır. Uzaktan erişimde SSH tüneli veya HTTPS ters
  proxy kullanın; MCP portunu doğrudan internete açmayın.
- `MCP_TOKEN` ile `ADMIN_TOKEN` ayrı sırlar olmalıdır; token yanıtlarda ve loglarda
  görünmez.

## Sorun giderme

| Belirti | Olası neden |
| --- | --- |
| `401 Unauthorized` | `Authorization: Bearer <token>` eksik/yanlış; token yeniden başlatma sonrası değişmiş olabilir |
| `503 Service Unavailable` | `MCP_TOKEN` tanımlı değil veya geçersiz (32–512 karakter); bot `.env`'i okumamış olabilir |
| `400 ... JSON çözümlenemedi` | Gövde geçerli JSON değil; PowerShell 5.1 tırnakları bozmuş olabilir |
| `400 ... protocolVersion uyuşmuyor` | `MCP-Protocol-Version` başlığı ile gövdedeki `_meta` sürümü farklı |
| `404 ... Bilinmeyen MCP metodu` | Modern akışta desteklenmeyen metot |
| `405 Method Not Allowed` | `POST` dışında bir metot kullanılmış |
| `403` | `Origin` başlığı `Host` ile uyuşmuyor |
| Boş/eksik araç listesi | Yanlış endpoint veya token; istemci sunucuya bağlanamamış |

## Geliştirme ve test

MCP endpoint'inin kimlik doğrulama, `tools/list`, `tools/call`, `settings_patch` ve eski
`initialize` akışını kapsayan entegrasyon testi:

```sh
cargo test --locked web::tests::mcp_endpoint_authenticates_and_serves_tools
```

Tam kontrol:

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```
