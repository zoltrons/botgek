# MCP sunucusu

## Görev

Botun çalıştığı süreçte geliştirme ve hata ayıklama için bir Model Context Protocol uç noktası sunar.
Kurulum, istemci kaydı ve çağrı örnekleri için [MCP.md](../MCP.md).

## Dosyalar

`src/mcp.rs` (taşıma, kimlik doğrulama, araçlar). `/mcp` yolu `web::build_router` içinde bağlanır.

## Girdi ve çıktı

Tek `POST /mcp` uç noktası, `application/json` yanıt (Streamable HTTP, oturumsuz). Kimlik doğrulama
`Authorization: Bearer <MCP_TOKEN>`.

## Nasıl çalışır

### Taşıma ve sürüm

- Modern sürüm `2026-07-28` (`server/discover`, oturumsuz). Eski `initialize` akışı `2025-11-25`,
  `2025-06-18`, `2025-03-26` ve `2024-11-05` sürümleriyle desteklenir.
- `Origin` başlığı varsa `Host` ile aynı olmalıdır (DNS rebinding koruması).
- `MCP_TOKEN` tanımsız ya da geçersizse (başında ya da sonunda boşluk olmayan 32 ile 512 karakter ve `ADMIN_TOKEN`'dan farklı) uç nokta 503 döner.
  Panel ve Discord botu etkilenmez. Token sabit zamanlı karşılaştırılır ve yanıtlarda yankılanmaz.

### Araçlar

| Araç | İşlev |
| --- | --- |
| `bot_status` | Sürüm, çalışma süresi, Discord readiness, veritabanı ve ayar durumu. |
| `bot_metrics` | `telemetry` platform ve kuyruk anlık görüntüsü. |
| `bot_diagnostics` | REST, fazlar, host, önbellek, SQLite, süreç metrikleri (`section` argümanı). |
| `bot_logs` | `logs::recent` (en fazla 512 olay, `min_level` filtresi). |
| `settings_get` | Etkin ayarlar ve revizyon. |
| `settings_patch` | JSON merge patch uygular, `sanitize_for_save` sonrası diske yazar ve botu anında etkiler. |
| `discord_overview` | Serenity önbelleğindeki sunucu, kanal ve roller. |
| `db_stats` | Veritabanı yolu, kayıt sayısı, dosya boyutu. |
| `db_query` | Tek bir salt okunur `SELECT`/`WITH`, en fazla 200 satır. |
| `convert_preview` | Bağlantıyı çözer, veriyi çeker, render planını döner. Discord'a mesaj göndermez. |
| `discord_test_card` | Test sunucusuna gerçek kart gönderir (REST ve medya telemetrisi için). |

Her araç `tool_definitions` içinde şemasıyla tanımlıdır, `call_tool` dağıtır. Yanıtlar `tool_result` ile
`content` ve hata bayrağına sarılır.

`convert_preview` panelle aynı HTTP istemcisini ve `builtin::fetch` yolunu kullanır. Çıktı `post_json` ile
özetlenir ve alıntı derinliği sınırlıdır.

## Değişmezler

- `settings_patch` kalıcıdır. Merge patch semantiği: `null` alanı siler, nesneler özyinelemeli birleşir.
- `db_query` yazan ifadeyi reddeder.
- Araç çıktısında token, oturum çerezi ya da `INSTAGRAM_SESSIONID` bulunmaz.
- Yeni araç eklerken hem `tool_definitions` hem `call_tool` güncellenmeli, çıktı sınırlı olmalı.

## Değiştirirken

Yeni araç: `tool_definitions`'a şema, `call_tool`'a eşleme, ayrı `async fn tool_...` ve
[MCP.md](../MCP.md) tablosu. Çıktıyı sınırlandırın (`limit` ve karakter kırpma).

## Testler

`cargo test --locked mcp`. Canlı doğrulama için `docs/MCP.md` içindeki curl örnekleri kullanılabilir.
