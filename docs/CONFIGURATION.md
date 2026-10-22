# Yapılandırma

Botgek iki katmanlı yapılandırma kullanır:

- **Ortam değişkenleri (`.env`)**: tokenlar, portlar, gizli değerler ve proxy alan adları.
- **`settings.json`**: panelden yönetilen modüller, rol filtreleri, AI personası ve motor ayarları.

Gerçek `.env` dosyasını ve gizli değerleri asla Git'e eklemeyin. Depodaki `.env.example` yalnızca
şablondur.

## Ortam değişkenleri

| Değişken | Varsayılan | Açıklama |
| --- | --- | --- |
| `DISCORD_TOKEN` | — (**zorunlu**) | Discord bot tokenı. |
| `ADMIN_TOKEN` | boş | Yönetim paneli sırrı. **32–512 karakter**, başında/sonunda boşluk olmadan. Boşsa panel açılmaz; bot çalışır. |
| `MCP_TOKEN` | boş | `/mcp` MCP sunucusu için Bearer token. **32–512 karakter**, `ADMIN_TOKEN`'dan farklı olmalı. Boş veya geçersizse endpoint kapalıdır (503); panel ve bot çalışır. |
| `WEB_PORT` | `3000` | Panel HTTP portu. Geçersiz değer uyarıyla `3000`'e döner. |
| `WEB_BIND` | `127.0.0.1` | Panelin bağlanacağı adres. Docker'da container içinde `0.0.0.0` kullanılır, port host tarafında yalnızca loopback'e yayınlanır. |
| `WEB_SECURE_COOKIE` | `false` | HTTPS ters proxy arkasında `true` yapın; oturum çerezine `Secure` ekler. |
| `OPENROUTER_API_KEY` | boş | AI sohbetini etkinleştirir. Boşsa AI kapalıdır; dolu her değer gerçek anahtar sayılır (yer tutucu metin 401 üretir). |
| `DELETE_ORIGINAL` | `true` | Başarılı dönüşümden sonra orijinal mesajı siler. **Manage Messages** yetkisi gerekir. |
| `TIKTOK_DOMAIN` | `tnktok.com` | TikTok proxy alan adı. |
| `BLUESKY_DOMAIN` | `fxbsky.app` | Bluesky proxy alan adı. |
| `SPOTIFY_DOMAIN` | `open.fixspotify.com` | Spotify proxy alan adı. |
| `FXTUMBLR_DOMAIN` | `tpmblr.com` | fxtumblr uyumlu Tumblr render hizmeti. Boş bırakılırsa public OG fallback kullanılır. |
| `INSTAGRAM_SESSIONID` | boş | Instagram oturum çerezi. Reels videolarının kart içinde oynaması için gerekir; gizlidir, `settings.json`'a ve panele yazılmaz. |
| `REDDIT_CLIENT_ID` | gömülü public kimlik | Reddit `installed_client` OAuth kimliği. Sır değildir; normalde değiştirmek gerekmez. |
| `BOTGEK_DB_PATH` | `botgek.db` | Kart kayıtları için SQLite dosyası. Docker'da `/app/data/botgek.db`. |
| `BOTGEK_SETTINGS_PATH` | türetilir | Ayar dosyası. Boşsa `BOTGEK_DB_PATH` bir dizin içeriyorsa o dizindeki `settings.json`, aksi hâlde çalışma dizinindeki `settings.json` kullanılır. |
| `BOTGEK_SETTINGS_SEED` | boş | Yalnızca hedef ayar dosyası yoksa bir kez okunan ilk içe aktarma kaynağı. |
| `RUST_LOG` | `botgek=info,serenity=warn` | Log filtresi. Ayrıntılı gecikme logları için `botgek=debug,serenity=warn`. |
| `BOTGEK_DATA_VOLUME` | `botgek_botgek-data` | Yalnızca Docker Compose: kullanılacak kalıcı volume adı. |
| `BOTGEK_VERSION` / `BOTGEK_SHA` | boş | Kaynak imaj derlemesinde sürüm künyesini gömmek için build argümanları. |

> Proxy alan adları yalnızca proxy motoru kullanılabilen platformlar içindir. X/Twitter, Instagram,
> Reddit, Twitch, Pixiv, Tumblr, Facebook ve YouTube Community Post **native-only**'dir; bu platformlarda
> proxy modu ve yedek alan adı yoktur. Panelde dolu bir alan adı `.env` değerini geçersiz kılar.

### Discord Developer Portal

- **Bot → Privileged Gateway Intents → Message Content Intent** açık olmalıdır.
- Gerekli izinler: **Send Messages**, **Embed Links**, **Read Message History**, **Add Reactions**.
- Orijinal mesaj silinecekse **Manage Messages**; kullanıcı adına gönderim (impersonation) kullanılacaksa
  **Manage Webhooks**.
- Slash ve mesaj komutları için davet bağlantısında `applications.commands` kapsamı bulunmalıdır.

## Ayar dosyası (`settings.json`)

Depodaki `settings.json` ilk açılışta tohum olarak kullanılabilir. Panel kayıtları aynı dizinde geçici
dosya, flush ve atomik replacement ile yazılır.

### Şema (sürüm 3)

```jsonc
{
  "schema_version": 3,
  "modules": { "twitter": true, "tiktok": true /* ... */ },
  "role_filters": {
    "twitter": { "mode": "disabled", "roles": [] }
  },
  "ai": {
    "model": "google/gemma-4-31b-it:free",
    "system_prompt": "…"
  },
  "engine": {
    "respond_to_bots": false,
    "sensitive_mode": "hide_always",
    "modes": { "tiktok": "proxy", "bluesky": "builtin", "spotify": "builtin" },
    "domains": { "tiktok": "", "bluesky": "", "spotify": "" },
    "twitter_source": "vxtwitter",
    "max_images": 4,
    "max_video_mb": 8,
    "show_metrics": true,
    "show_timestamp": true,
    "show_community_note": true,
    "show_quote": true,
    "auto_translate": true,
    "translation_target": "tr",
    "fallback_to_proxy": true,
    "impersonate_user": true,
    "impersonate_skip_replies": true,
    "emojis": { "verified": "<:verified:…>", "…": "…" }
  },
  "guilds": {
    "<sunucu-id>": {
      "modules": { /* … */ },
      "role_filters": { /* … */ },
      "ai": { /* … */ },
      "engine": { /* … */ },
      "channels": {
        "<kanal-id>": { "modules": { /* … */ } }
      }
    }
  }
}
```

### Kapsam çözümleme

1. **Global** varsayılan her zaman geçerlidir.
2. **Sunucu** kaydı yalnızca kullanıcının o sunucuda bilerek değiştirdiği bölümleri saklar. Kayıt
   `sparse`'tır: `modules`, `role_filters`, `ai` ve `engine` bölümlerinden yalnızca dolu olanlar geneli
   ezer.
3. **Kanal** kaydı aynı bölümleri kanal düzeyinde ezer ve yalnızca o kanalda geçerlidir.
4. **Thread** yalnızca parent kanalın ayarını miras alır. Kategori kapsamı yoktur.

Granülarite **bölüm** düzeyindedir: bir bölüm override edildiğinde içindeki tüm anahtarlar sabitlenir,
alan düzeyinde kısmi miras yoktur. Bu nedenle override bloğu daima tam yazılır; eksik alt anahtarlar
genelin o anki değeriyle değil kod varsayılanıyla doldurulur.

Kanal kapsamları paneldeki kimliği doğrulanmış Discord kanal listesinden seçilir; elle kanal kimliği
yazmak gerekmez.

### Önemli alanlar

| Alan | Değerler / sınır | Davranış |
| --- | --- | --- |
| `modules.*` | `true` / `false` | Platform, çeviri ve AI sohbeti modülleri. |
| `role_filters.*.mode` | `disabled`, `blacklist`, `allowlist` | Boş `allowlist` kimseye erişim vermez. Rol sorgusu başarısız olursa etkin filtre reddeder. |
| `role_filters.*.roles` | En fazla 200 Discord rol kimliği | Geçersiz kimlikler ayıklanır. |
| `engine.modes` | `proxy` / `builtin` | Yalnızca TikTok, Bluesky ve Spotify için geçerlidir. |
| `engine.twitter_source` | `vxtwitter`, `fxtwitter`, `syndication`, `hybrid` | X verisinin kaynağı. `hybrid` önce syndication'ı dener. |
| `engine.max_images` | `1..=10` (varsayılan 4) | Tek kartta eklenecek görsel sayısı. |
| `engine.max_video_mb` | `1..=50` (varsayılan 8) | Kart için video boyutu sınırı. |
| `engine.sensitive_mode` | `hide_always`, `allow_in_nsfw` | Kaynak spoiler'ı ve elle işaretlenen içerik her durumda gizli kalır. |
| `engine.auto_translate` | `true` / `false` | X gönderilerini X'in kendi çevirisiyle hedef dile çevirir. |
| `engine.translation_target` | `tr`, `en`, `de` veya BCP-47 kodu | Otomatik çeviri hedef dili. |
| `engine.emojis.*` | `<:ad:id>` biçimi | Alan boş bırakılırsa Unicode varsayılan kullanılır; özel emoji seti olmayan kurulumlar da kart üretebilir. |
| `ai.model` | En fazla 128 karakter | OpenRouter model adı; panel `sağlayıcı/model` biçimini kabul eder. |
| `ai.system_prompt` | En fazla 4.000 karakter | AI personası. |
| `engine.impersonate_user` | `true` / `false` | Kartı, paylaşan kullanıcının adı ve avatarıyla webhook üzerinden gönderir. **Manage Webhooks** gerekir. |

### Bozuk dosya ve göç

- Bozuk ayar dosyası **varsayılan erişim politikasıyla başlatılmaz**. Dosyayı düzeltmeniz veya sağlam
  yedeği geri koymanız gerekir.
- Şema göçü öncesi yedek alınamıyorsa göç durur.
- Sürüm 2 dosyaları okunurken eski "tam kopya" sunucu kayıtları bir kez sparse biçime taşınır ve sürüm
  damgalanır. Bu göç tekrar çalıştırılmaz; böylece bilerek sabitlenen bir bölüm sessizce mirasa dönmez.

## Yönetim paneli güvenliği

- `ADMIN_TOKEN` 32–512 karakter, yüksek entropili bir sır olmalıdır. Üretmek için:
  `openssl rand -hex 32`.
- Token yalnızca giriş isteğinde gönderilir; `localStorage`/`sessionStorage`'a yazılmaz ve API
  yanıtlarında/loglarda görünmez.
- Oturum **8 saat** sürer; rastgele 64 karakterlik kimlik, `HttpOnly`, `SameSite=Strict` çerez kullanılır.
  Bot yeniden başlatıldığında tüm oturumlar iptal edilir.
- Durum değiştiren istekler `X-Botgek-Request: 1` başlığı ister; CORS açılmamıştır.
- İstemci adresi başına 60 saniyede 20 başarısız girişten sonra giriş reddedilir; izlenen adres tablosu
  1.024 girdiyle sınırlıdır. Eşzamanlı oturum sayısı 64'tür.
- Tokenı döndürmek için `.env` değerini değiştirip botu yeniden başlatın.

Aynı adreste `/mcp` yolunda bir MCP endpoint'i de yayınlanır. Ayrı `MCP_TOKEN` ile korunur; token
tanımsız/geçersizse kapalıdır ve paneli etkilemez. Kurulum, araçlar ve sınırlar: [MCP.md](MCP.md).

Uygulama TLS sonlandırmaz. Uzak erişimde **HTTPS ters proxy** veya SSH tüneli kullanın ve
`WEB_SECURE_COOKIE=true` ayarlayın; botun HTTP portunu doğrudan internete açmayın.

## Kalıcılık ve yedekleme

- `settings.json` atomik yazılır; WebUI kayıtları seri işlenir ve disk işlemi async worker'ı bloklamaz.
- Atomik replacement için tek dosya bind-mount'u yerine **dizin veya volume** kullanın.
- Kart–paylaşan kaydı SQLite'ta, WAL modunda tutulur; işlemler sınırlı blocking worker üzerinden
  yürür. Kayıtlar 200.000 girdiyle sınırlıdır ve budanır.
- SQLite açılamazsa bellek içi fallback loglanır ve readiness başarısız olur; bunu kalıcı çalışma
  saymayın.
- Ayar yedeği: `./backup-settings.sh` önce `BOTGEK_SETTINGS_PATH` değerini kullanır.
- Docker'da dosya volume içindedir:
  `docker compose exec botgek cat /app/data/settings.json > settings.json.bak`.

Güncelleme, volume göçü ve yedekten geri dönme adımları: [OPERATIONS.md](OPERATIONS.md).
