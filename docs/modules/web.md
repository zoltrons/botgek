# Web paneli

## Görev

Axum ile servis edilen yönetim paneli: ayarları okur ve kaydeder, Discord rol ve kanal listelerini verir,
sağlık ve metrik uçlarını sunar. Panel statik dosyalardan oluşur ve ikiliye gömülüdür. MCP uç noktası da
aynı sunucuda çalışır (bkz. [mcp.md](mcp.md)).

## Dosyalar

| Dosya | İçerik |
| --- | --- |
| `src/web/mod.rs` | Router, güvenlik başlıkları, ayar okuma ve kaydetme, Discord rol ve kanal uçları, varlık servisi. |
| `src/web/auth.rs` | Oturum, giriş, çıkış, CSRF başlığı kontrolü. Ana token bu dosyanın dışına çıkmaz. |
| `src/web/ui/index.html`, `app.css`, `app.js` | Panel arayüzü (derleme anında `include_str!` ile gömülür). |
| `src/web/ui/login.html`, `login.js` | Giriş sayfası. |
| `tests/web-ui.test.cjs` | Panel testleri (Node, bağımlılıksız). |
| `tests/fixtures/settings-resolution.json` | Rust ve JS çözümlemesinin ortak sözleşmesi. |

## Uç noktalar

Korumasız: `GET /login`, `/login.js`, `/app.css`, `POST /api/auth/login`, `GET /api/health`.
Oturum gerektirir:

| Uç | Ne yapar |
| --- | --- |
| `GET /` , `/app.js` | Panel. Oturum yoksa `/` giriş sayfasına yönlenir, diğerleri 401 döner. |
| `GET /api/settings` | Etkin global ayarlar. `ETag` başlığı ayar revizyonunu taşır. |
| `POST /api/settings` | Ayarları kaydeder. `If-Match` revizyonu uyuşmazsa 412 ve `conflict: true`. |
| `GET /api/info` | Sürüm ve port bilgisi. |
| `GET /api/discord/roles`, `/api/discord/channels` | Sunucu rolleri ve kanalları (Discord önbelleğinden). |
| `GET /api/ready`, `/api/metrics` | Readiness ve telemetri anlık görüntüsü. |
| `POST /api/auth/logout` | Oturumu kapatır. |

`/mcp` aynı router içinde, kendi Bearer kimlik doğrulamasıyla gelir.

## Nasıl çalışır

### Kimlik doğrulama

- Giriş `ADMIN_TOKEN` ile yapılır. Token 32 ile 512 karakter olmalı ve sabit zamanlı karşılaştırılır
  (`utils::secret_matches`). Token yoksa panel açılmaz, bot çalışmaya devam eder.
- Oturum 8 saat sürer, çerez adı `botgek_session`, `HttpOnly` ve `SameSite=Strict`. `WEB_SECURE_COOKIE=true`
  `Secure` ekler. En fazla 64 eşzamanlı oturum.
- Giriş hız sınırı: istemci adresi başına 60 sn içinde 20 başarısız deneme, izlenen adres tablosu en fazla
  1024 girdi. IPv6 adresler /64 önekine, IPv4'e eşlenmiş adresler IPv4'e indirgenir (`rate_key`).
- `auth::csrf`: GET, HEAD ve OPTIONS dışındaki her istek `X-Botgek-Request: 1` başlığını taşımalıdır.
  Yoksa 403.

### Ayar kaydetme

1. Gövde `AppSettings` olarak ayrıştırılır ve `sanitize_for_save` uygulanır. Panel ve MCP aynı normalizasyonu
   kullanır.
2. `If-Match` ile gelen revizyon, kilit altında mevcut ayarın hash'iyle karşılaştırılır. İki sekme aynı anda
   kaydederse yalnızca ilki geçer.
3. `schema_version` panelden gelmez, sunucudaki değer korunur.
4. Dosya atomik yazılır, sonra bellekteki ayar değiştirilir. İş HTTP istemcisi koparsa bile tamamlanır.

### Güvenlik başlıkları

`add_security_headers` her yanıta ekler: `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`,
`Referrer-Policy`, `Permissions-Policy` ve katı bir `Content-Security-Policy` (`script-src 'self'`, satır içi
betik yok, `frame-ancestors 'none'`). Statik varlıklar `ETag` ile yeniden doğrulanır, diğer yanıtlar
`no-store`.

### Arayüz

`app.js` ayarları yükler, global, sunucu ve kanal kapsamlarını çözer (Rust ile aynı mantık), düzenlemeleri
toplar ve `POST /api/settings` ile gönderir. Satır içi olay yöneticisi kullanılmaz, CSP buna dayanır.

## Değişmezler

- Panel, kimlik bilgisini tarayıcı depolamasına yazmaz.
- Yeni durum değiştiren uç `csrf` katmanı arkasında olmalı.
- Rust ve JS çözümleme mantığı aynı fixture ile test edilir. Birini değiştiren ikisini de günceller.
- `index.html`, `app.js` ve `app.css` arasındaki kimlik, sınıf ve rota uyumu Rust testinde kontrol edilir
  (`panel_assets_agree_on_ids_classes_controls_and_routes`).
- Yanıtlarda sır yoktur. Sistem istemi ve rol kimlikleri hassas sayıldığı için önbelleğe alınmaz.

## Değiştirirken

- Yeni ayar alanı: [settings.md](settings.md) kontrol listesi, sonra `index.html`, `app.js` ve testler.
- Yeni uç: `build_router` içinde doğru gruba (korumalı ya da korumasız) ekleyin.
- Yeni statik varlık: `include_str!` sabiti, `asset_etag` ve router kaydı.

## Testler

```sh
cargo test --locked web
node --check src/web/ui/app.js
node --check src/web/ui/login.js
node --test tests/web-ui.test.cjs
```
