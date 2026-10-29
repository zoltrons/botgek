# Proje haritası

Ajanlar için yönlendirme belgesi: bir işin hangi dosyada başladığını, hangi belgeye bakılacağını ve nasıl
doğrulanacağını verir. Derin anlatım modül belgelerindedir ([docs/modules/README.md](../modules/README.md)).
Çalışma yöntemi: [WORKFLOW.md](WORKFLOW.md). Bu belge `scripts/outline.sh --check` ile depoyla
karşılaştırılır; dosya ekler, silerseniz ya da taşırsanız burayı da güncelleyin.

## Bir bakışta

Botgek tek Rust ikilisidir (serenity + tokio + axum + SQLite). Discord'da paylaşılan sosyal medya
bağlantısını yakalar, gönderiyi kendisi çeker, Components V2 kartı üretip gönderir. Aynı süreçte yönetim
paneli (`/`) ve geliştirme MCP'si (`/mcp`) çalışır.

```text
handler.rs ─► converters::enqueue_message ─► kuyruk ─► builtin::handle
   parser (URL) · ayar çözümleme · rol filtresi        ├─ builtin::fetch ─► <platform>::fetch ─► BuiltinPost
                                                       ├─ render::plan_media / resolve_media
                                                       ├─ v2::build_payload (Components V2 JSON)
                                                       └─ raw / transport ─► Discord REST ─► store
```

Slash ve mesaj komutları: `handler::interaction_create` ─► `commands::handle` (kanca + panik koruması) ─►
komut işleyicisi ─► `converters::convert_now`.

## Görev ─► ilk bakılacak yer

| İş | Başla | Belge | Arama çapası | Doğrula |
| --- | --- | --- | --- | --- |
| URL tanınmıyor / kanonik yanlış | `src/converters/parser.rs` | converters.md | `parse_message`, `Platform` | `cargo test parser` |
| Bir platformun verisi eksik/yanlış | `src/converters/builtin/<platform>.rs` | platforms/`<platform>`.md | `fn fetch` | `cargo test <platform>` |
| Yeni platform ekle | parser ─► platform modülü ─► `builtin/mod.rs` ─► `settings.rs` ─► panel | DEVELOPMENT.md (Yeni platform ekleme) | `platform_key`, `PlatformModules` | tam doğrulama |
| Kart görünümü (düzen, metrik, buton) | `src/converters/builtin/v2.rs` | rendering.md | `build_payload`, `V2Card` | `cargo test v2` |
| Medya sayısı, indirme, video boyutu | `render.rs`, `media.rs` | rendering.md | `plan_media`, `resolve_media` | `cargo test render` |
| Gönderim, webhook taklidi, silme | `builtin/mod.rs` (`handle`), `raw.rs`, `transport.rs` | pipeline.md | `send_via_webhook`, `delete_original` | `cargo test builtin` |
| Önbellek, tek uçuş, hata önbelleği | `builtin/cache.rs`, `coordination.rs` | pipeline.md | `post_cache_key` | `cargo test cache` |
| Kuyruk, eşzamanlılık, `/dönüştür` akışı | `src/converters/mod.rs` | converters.md | `enqueue_message`, `convert_now` | `cargo test converters::` |
| Kaynak mesaj düzenlendi/silindi | `src/converters/tracking.rs`, `handler.rs` | handler.md | `message_update`, `message_delete` | `cargo test handler` (tracking'in testi yok) |
| Slash/mesaj komutu ekle-değiştir | `src/commands/mod.rs` | commands.md | `register`, `everywhere` | `cargo test commands::` |
| Komut kancası, panik koruması | `src/commands/hooks.rs` | commands.md | `pre_command`, `post_command` | `cargo test hooks` |
| Eğlence oyunları (`/düello`, `/bomba`...) | `src/commands/fun.rs` | commands.md | `fun:` önekli `custom_id` | `cargo test fun::` |
| AI sohbeti | `handler.rs` (`reply_with_ai`), `src/ai/` | ai.md | `should_trigger_ai_chat` | `cargo test ai::` |
| Ayar alanı ekle/değiştir | `settings.rs` ─► `settings.json` ─► `web/ui/app.js` + `index.html` ─► fixture | settings.md, CONFIGURATION.md | `SETTINGS_SCHEMA_VERSION`, `sanitize` | rust + `node --test` |
| Rol filtresi | `src/policy.rs` | settings.md | `allows` | `cargo test policy` |
| Ortam değişkeni | `config.rs` ─► `.env.example` | CONFIGURATION.md | `from_env`, `env_bool` | `cargo test config` |
| Panel API / oturum | `src/web/mod.rs`, `src/web/auth.rs` | web.md | `.route(` | `cargo test web::` |
| Panel arayüzü | `src/web/ui/app.js`, `index.html`, `app.css` | web.md | `resolveSettings`, `apiFetch` | `node --check`, `node --test` |
| MCP aracı ekle | `src/mcp.rs` (`tool_definitions` + dağıtım) | MCP.md, mcp.md | `tool_definitions` | `cargo test mcp::` |
| Metrik, sayaç, gecikme | `src/telemetry.rs` | telemetry.md | sayaç adı | `cargo test telemetry` |
| Kalıcı kart kaydı (SQLite) | `src/store.rs` | store.md | `remember`, `card_for_source` | `cargo test store::` |
| Link önizleme / SSRF güvenliği | `preview.rs`, `safe_http.rs`, `opengraph.rs` | preview.md | `safe_http` | `cargo test preview` |
| Sürüm, derleme, CI, Docker | `build.rs`, `release.sh`, `.github/workflows/`, `Dockerfile` | OPERATIONS.md | | CI dosyası değişince elle incele |

Belge adları `docs/modules/` altındadır (platformlar `platforms/` altında); `ARCHITECTURE.md`,
`CONFIGURATION.md`, `DEVELOPMENT.md`, `OPERATIONS.md`, `MCP.md` `docs/` altındadır.

## Dosya haritası

Satır sayısı yaklaşık. **1000 satır üstü dosyayı baştan sona okumayın**: `scripts/outline.sh <dosya>`
çatıyı satır numaralarıyla verir, sonra yalnızca gereken aralığı okuyun. Testler çoğunlukla aynı dosyada
`mod tests` içindedir.

### Çekirdek

| Dosya | Görev |
| --- | --- |
| `src/main.rs` | Başlangıç sırası, CLI bayrakları, loglama, HTTP istemcisi, gateway, panel, kapanış. |
| `src/config.rs` | `Config`: ortam değişkenleri. `env_bool`. |
| `src/version.rs` | Derleme künyesi, User-Agent dizeleri (`build.rs` ile paylaşılır). |
| `src/handler.rs` | Gateway olayları: mesaj, düzenleme, silme, tepki, etkileşim, AI tetikleme, AI hız bütçesi. (~975) |
| `src/policy.rs` | Rol filtresinin tek karar noktası. |
| `src/settings.rs` | Ayar şeması, kapsam çözümleme (global ─► sunucu ─► kanal), doğrulama, atomik yazma, göç. (~2200) |
| `src/store.rs` | SQLite: kart ─► paylaşan kullanıcı kaydı, salt okunur sorgu. (~900) |
| `src/telemetry.rs` | Süreç içi sayaçlar ve gecikme histogramı. (~1260) |
| `src/benchmarks.rs` | Elle çalışan çevrimdışı ölçüm. |
| `src/logs.rs` | MCP için 512 olaylık bellek içi log halkası. |
| `src/utils/mod.rs` | `truncate_chars`, `secret_matches`, `base64_encode`, zaman yardımcıları. |
| `src/utils/mentions.rs` | `build_selective_allowed_mentions`. |

### Komutlar, AI, web, MCP

| Dosya | Görev |
| --- | --- |
| `src/commands/mod.rs` | Komut tanımları/kayıt, `/dönüştür`, Apps → Dönüştür, Spotify sayfalama. |
| `src/commands/hooks.rs` | `pre_command`/`post_command`, panik yakalama, sabit hata metni. |
| `src/commands/gif.rs` | `/gif-yap`, Apps → GIF yap. |
| `src/commands/fun.rs` | Mini oyunlar ve eğlence komutları. (~2400) |
| `src/ai/mod.rs` | OpenRouter istemcisi, model normalizasyonu, yedek zincir, hata türleri. (~1020) |
| `src/ai/discord.rs` | AI bağlamı: yanıt zinciri, yanıtlanan mesaj, görseller. |
| `src/web/mod.rs` | Axum yönlendirmesi, panel API'si, gömülü varlıklar, panel testleri. (~1520) |
| `src/web/auth.rs` | Yönetici girişi, oturum çerezi. |
| `src/web/ui/app.js` | Panel mantığı (ayar çözümleme Rust'la aynı olmalı). (~1965) |
| `src/web/ui/index.html` | Panel iskeleti. |
| `src/web/ui/app.css` | Panel stilleri. |
| `src/web/ui/login.html` | Giriş sayfası. |
| `src/web/ui/login.js` | Giriş sayfası betiği. |
| `src/mcp.rs` | `/mcp` endpoint'i, Bearer doğrulama, araç tanımları ve dağıtım. (~1190) |

### Dönüştürme hattı

| Dosya | Görev |
| --- | --- |
| `src/converters/parser.rs` | `Platform`, `ParsedPost`, `parse_message`: URL kalıpları, kanonikleştirme, spoiler. (~1320) |
| `src/converters/mod.rs` | Kuyruk (`enqueue_message`), `convert_now`, `ConvertError`. |
| `src/converters/tracking.rs` | Kaynak mesaj ─► kart eşlemesi; düzenleme/silmede kartı kaynakla birlikte tutar. |
| `src/converters/builtin/mod.rs` | `fetch`, `handle`, ortak HTTP/zaman/URL yardımcıları, taklit webhook'u, kart tanıma. (~2130) |
| `src/converters/builtin/model.rs` | `BuiltinPost`, `Author`, `Stats`, `Media`: platformlar arası veri tipleri. |
| `src/converters/builtin/cache.rs` | Gönderi önbelleği (600 sn / 512) ve hata önbelleği (30 sn). |
| `src/converters/builtin/coordination.rs` | Anahtar başına tekil uçuş kilidi. |
| `src/converters/builtin/transport.rs` | Discord gönderim taşıması: sınırlı yanıt, rota bekleme, kör POST yeniden denemesi yok. |
| `src/converters/builtin/raw.rs` | Ham Discord REST: kanal mesajı, webhook, etkileşim yanıtı, multipart. |
| `src/converters/builtin/render.rs` | Medya planı, indirme, görünürlük kararı, sayı biçimleme. |
| `src/converters/builtin/v2.rs` | Components V2 kart gövdesi (`build_payload`). (~2460) |
| `src/converters/builtin/media.rs` | Medya indirme, dosya adı/MIME, Discord dosya sınırı, `fx_gif_url`. |
| `src/converters/builtin/preview.rs` | Yalnızca OG kullanan beş adaptörün ortak genel önizleme sınırı. |
| `src/converters/builtin/opengraph.rs` | OpenGraph/twitter meta çözümleyici. |
| `src/converters/builtin/safe_http.rs` | Güvenilmeyen URL'leri DNS ve yönlendirmeye güvenmeden çekme (SSRF koruması). |
| `src/converters/builtin/translate.rs` | X çevirisi için dil politikası. |

### Platform modülleri (`src/converters/builtin/`, yalnızca veri çeker, `BuiltinPost` döner)

| Dosya | Platform / not |
| --- | --- |
| `src/converters/builtin/twitter.rs` | X: FxTwitter + syndication yarışı, çeviri, alıntı. (~2830) |
| `src/converters/builtin/twitter/benchmarks.rs` | X canlı ölçümü (`--ignored`). |
| `src/converters/builtin/reddit.rs` | Reddit JSON. (~1850) |
| `src/converters/builtin/reddit_gql.rs` | Reddit GraphQL yorum bağlamı. |
| `src/converters/builtin/reddit_video.rs` | v.redd.it video+ses ─► tek MP4. (~1030) |
| `src/converters/builtin/instagram.rs` | Instagram. (~1300) |
| `src/converters/builtin/tiktok.rs` | TikTok sayfa gömülü verisi. |
| `src/converters/builtin/bluesky.rs` | Bluesky XRPC. |
| `src/converters/builtin/spotify.rs` | Spotify önizleme ve kart. (~1080) |
| `src/converters/builtin/spotify_video.rs` | Spotify 30 sn MP3 ─► kapaklı MP4. |
| `src/converters/builtin/twitch.rs` | Twitch klibi (GraphQL). |
| `src/converters/builtin/pixiv.rs` | Pixiv eseri. |
| `src/converters/builtin/tumblr.rs` | Tumblr (fxtumblr renderer + OG yedeği). |
| `src/converters/builtin/facebook.rs` | Facebook gönderi/video/reel. |
| `src/converters/builtin/youtube.rs` | YouTube topluluk gönderisi. |
| `src/converters/builtin/letterboxd.rs` | Letterboxd film/inceleme/profil kartları. (~2200) |

### Dışı

| Yol | Görev |
| --- | --- |
| `tests/web-ui.test.cjs` | Panelin ayar çözümleme ve kayıt davranışı (Node). |
| `tests/fixtures/settings-resolution.json` | Rust ve JS'in paylaştığı çözümleme sözleşmesi. |
| `settings.json` | Varsayılan ayar dosyası (kök). |
| `.env.example` | Ortam değişkeni örneği. |
| `build.rs`, `build-support/version-parse.rs` | Sürümü git etiketinden türetir. |
| `release.sh` | Sürüm etiketleme. |
| `scripts/outline.sh` | Gezinme aracı (çatı, tanım arama, harita denetimi). |

## Birlikte değişenler

Bunlardan birine dokunan hepsine bakar:

- **Ayar şeması:** `src/settings.rs` ↔ `settings.json` ↔ `src/web/ui/app.js` ↔ `tests/fixtures/settings-resolution.json` ↔ `docs/CONFIGURATION.md`.
- **Platform ekleme/çıkarma:** `parser.rs` (`Platform`) ↔ `builtin/mod.rs` (`fetch`, `platform_key`) ↔ `settings.rs` (`PlatformModules`) ↔ panel ↔ `docs/modules/platforms/`.
- **Ortam değişkeni:** `src/config.rs` ↔ `.env.example` ↔ `docs/CONFIGURATION.md` ↔ `docker-compose.yml`.
- **MCP aracı:** `src/mcp.rs` ↔ `docs/MCP.md`.
- **Süre/boyut bütçeleri:** kod sabiti ↔ `docs/ARCHITECTURE.md` tabloları.

## Sözlük

| Terim | Anlam |
| --- | --- |
| Kart | Botun gönderdiği Components V2 mesajı. Klasik embed yoktur. |
| `Platform` | Desteklenen 12 kaynak (`parser.rs`). Anahtarı küçük harf dize: `platform_key`. |
| `ParsedPost` | Ayrıştırılmış bağlantı: platform, kanonik URL, spoiler, kullanıcı yorumu. |
| `BuiltinPost` | Platformdan bağımsız çekilmiş gönderi verisi (`builtin/model.rs`). |
| `EngineSettings` / `GuildSettings` | Bir mesaj için çözümlenmiş etkin ayar. |
| Etkileşim akışı | `/dönüştür` ve mesaj komutu: bekleyerek çalışır, modül kapalıyken de dönüştürür. |
| Taklit (impersonation) | Kartı kullanıcının adı/avatarıyla webhook üzerinden göndermek. |
| Yedek kaynak | X için syndication (FxTwitter yanıt vermezse) ya da elle seçilen vxTwitter. |
