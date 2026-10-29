# Ayarlar ve rol politikası

## Görev

`settings.json` şemasını tanımlar, global, sunucu ve kanal katmanlarını tek bir etkin ayara çözer,
doğrular, atomik yazar ve eski dosyaları göç ettirir. `policy.rs` rol filtresi kararını verir.

## Dosyalar

| Dosya | İçerik |
| --- | --- |
| `src/settings.rs` | `AppSettings`, `GuildSettings`, `GuildOverride`, `ChannelOverride`, `PlatformModules`, `EngineSettings`, `AiSettings`, `EmojiSettings`, `RoleFilterSettings`, çözümleme, yükleme ve kaydetme. |
| `src/policy.rs` | `allows(...)`: rol filtresinin tek karar noktası. Dönüşüm ve AI sohbeti aynı fonksiyonu kullanır. |
| `tests/fixtures/settings-resolution.json` | Rust ve panelin JS kodunun paylaştığı çözümleme sözleşmesi. |

## Girdi ve çıktı

- Girdi: `settings.json` dosyası, panelden gelen `POST /api/settings`, MCP `settings_patch`.
- Çıktı: `GuildSettings`, yani bir mesaj için çözümlenmiş etkin ayar.

## Nasıl çalışır

### Şema

`AppSettings` alanları: `schema_version`, `modules`, `role_filters`, `ai`, `engine`, `guilds`.
`SETTINGS_SCHEMA_VERSION` şu an 3'tür.

- `modules` (`PlatformModules`): her platform için bir anahtar ve `ai_chat`. Hepsi varsayılan açıktır.
- `role_filters`: modül başına `RoleFilterSettings` (`mode`: `disabled`, `blacklist` ya da `allowlist`; en fazla 200 rol kimliği).
- `engine` (`EngineSettings`): `respond_to_bots`, `sensitive_mode`, `max_images` (1 ile 10), `max_video_mb`
  (1 ile 50), `show_metrics`, `show_timestamp`, `show_community_note`, `show_quote`, `auto_translate`,
  `translation_target`, `impersonate_user`, `impersonate_skip_replies`, `emojis`.
- `ai` (`AiSettings`): model ve sistem istemi (persona).
- `guilds`: sunucu kimliğine göre seyrek override. Sunucu içinde kanal bazlı `ChannelOverride` vardır.

`translate_english` ve `x_alternative_source` `#[serde(skip)]` alanlardır. Dosyaya yazılmaz, yalnızca
`/dönüştür` etkileşimi çalışma zamanında türetir.

### Kapsam çözümleme

`resolve_settings(global, guild)` sunucu katmanını, `resolve_channel_layer` kanal katmanını uygular.
Mesaj için giriş noktası `AppSettings::get_message_settings(guild, channel, thread_parent)`:

1. Global ayar.
2. Sunucu override (varsa) global üzerine biner.
3. Kanal override (varsa) sunucu sonucunun üzerine biner. Thread kendi override'ı yoksa üst kanalınkini alır.
4. Kategori kapsamı yoktur.

Override seyrektir: yalnızca bilerek değiştirilen bölümler yazılır, kaydın kendisi yoksa üst katman geçerlidir.
Bu mantık `src/web/ui/app.js` içinde de uygulanır. İkisi aynı fixture ile test edildiği için ayrışmamalıdır.

### Doğrulama ve kayıt

- `sanitize` yöntemleri değerleri sınırlara çeker (görsel sayısı, video MB, emoji biçimi, ID biçimi).
  Kayıttan önce `sanitize_for_save` çalışır.
- `save_to` önce yedek dosyası alır, geçici dosyaya yazar ve atomik değiştirir (Windows'ta
  `MoveFileEx`). Geçersiz dosya yüklemede hata verir, bozuk ayarla başlanmaz.
- Eksik alanlar varsayılana düşer. Boş `{}` dosyası geçerlidir.
- Dosya yolu: `BOTGEK_SETTINGS_PATH`, yoksa kalıcı veritabanının dizini, yoksa `settings.json`.

### Göç

- Şema sürümü 0 olan dosyalarda `migrate_legacy_guild_overrides` ve `migrate_legacy_role_filter` bir kez
  çalışır. Eski tek `role_filter` modül filtrelerine kopyalanır ve bir daha yazılmaz.
- `twitter_source` eski bir alandır ve etkisizdir.

### Rol politikası

`policy::allows` ve `RoleFilterSettings::allows_roles`:

- Kapalı filtre herkese izin verir.
- Allowlist: yalnızca listedeki roller. Boş allowlist kimseye erişim vermez.
- Blacklist: listedeki rollerden birine sahip olan reddedilir, diğerleri geçer.
- Rol sorgusu başarısız olursa etkin filtre erişimi reddeder (kapalı başarısızlık).
- Rol kimlikleri `sanitize` ile sayıya çevrilir, sıfır ve geçersizler atılır, sıralanıp tekilleştirilir.
- Üye rolleri çağıran taraftan gelmediyse `policy::allows` sunucudan üyeyi sorar. DM'de rol listesi boştur.

### Emoji ayarları

`EmojiSettings` kart metriklerindeki özel emojileri taşır. Geçersiz değer Unicode karşılığa düşer. Yalnızca
ad verilirse (`verified` gibi) botun kendi emojisine bağlanır. Varsayılan emojiler kaynakta sabittir.

## Değişmezler

- Etkin ayarı okumanın tek yolu `get_guild_settings` / `get_message_settings` olmalı. Doğrudan `engine`
  alanına global üzerinden bakan kod sunucu ve kanal override'ını yok sayar.
- `schema_version` değişirse göç testi eklenmeli. Yeni dosya güncel sürümle doğar, göç çalışmaz.
- Rol filtresinde belirsiz durum erişimi reddeder.
- Gizli değerler (`ADMIN_TOKEN`, `INSTAGRAM_SESSIONID`) `settings.json`'a girmez.

## Değiştirirken

Yeni platform modülü anahtarı:

1. `PlatformModules` alanı ve `Default`.
2. `ModuleRoleFilters` alanı, `sanitize` listesi ve `get_role_filter` eşlemesi.
3. `builtin::platform_key` ve `converters` içindeki modül kontrolü.
4. Panel: `src/web/ui/index.html` ve `app.js` anahtarları, `tests/web-ui.test.cjs`.

Yeni motor ayarı: `EngineSettings` alanı, `Default`, `sanitize`, panel alanı, fixture ve
[CONFIGURATION.md](../CONFIGURATION.md).

## Testler

```sh
cargo test --locked settings
cargo test --locked policy
node --test tests/web-ui.test.cjs
```
