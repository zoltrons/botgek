# EG-01 — Embedded platform adapter parity

**Öncelik:** P0  
**Hedef:** Embedded'in public listesinde olup Botgek'te bulunmayan Twitch, Pixiv, Tumblr, Facebook ve
YouTube Community Posts içeriklerini ortak native kart akışına almak.

## Mevcut durum ve kanıt

Botgek'in `Platform` enum'u yalnızca `Twitter`, `TikTok`, `Instagram`, `Reddit`, `Bluesky`, `Spotify`
içeriyor (`src/converters/parser.rs`). `src/converters/builtin/` altında da bu platformlara karşılık
gelen adapter'lar var; yeni beş platform için parser regex'i, adapter, module setting, role filter ve
WebUI alanı yok. Spotify rakip listesinde bulunmasa da mevcut davranış korunmalıdır.

Embedded public sayfaları yeni kapsamı şöyle tanımlıyor:

- Twitch: clips, streamer info, duration, game context.
- Pixiv: illustrations, manga, ugoira, artist info, tags/metadata, series.
- Tumblr: text/photo/quote/link/audio/video, blog info, tags/notes, reblog chain.
- Facebook: photo posts, videos/reels, profile, post text/reactions.
- YouTube Posts: community post text/image, channel info, links.

## Hedef sözleşme

Her adapter `BuiltinPost` üretmelidir. Minimum alanlar:

```text
platform, canonical_url, author{name, handle, avatar?, profile_url?},
text/title, created_at?, stats, media[], quote?, sensitive, extra_label,
link_preview?
```

Platforma özel alanlar generic modele zorla eklenmemeli; gerekiyorsa `extra_label` veya sınırlı bir
`PlatformContext` enum'u tasarlanmalıdır. `CustomCard` yalnızca gerçekten platforma özgü layout gerekiyorsa
kullanılmalıdır.

## Uygulama planı

1. `Platform` enum'una beş yeni değer ekleyin; `display_name`, `supports_translation`, `platform_key`
   ve tüm exhaustive match noktalarını derleyici yardımıyla güncelleyin.
2. Her platform için parser'ı önce canonical public URL formatlarıyla yazın. Kısa/share linklerini
   ayrı regex fixture'larıyla kapsayın; parser dışındaki kod URL biçimi tahmin etmemeli.
3. Her adapter'ı ayrı modülde uygulayın. Kaynak seçiminde önce resmi public endpoint/oEmbed/OG verisini,
   sonra güvenilir fallback'i kullanın. Login/cookie gerektiren yol varsa bunu explicit optional config
   olarak belgeleyin; cookie'u `settings.json` veya WebUI'a koymayın.
4. Medya için `MediaKind`, poster, boyut ve duration alanlarını doldurun. Ugoira/animated format,
   Twitch clip veya Facebook video için doğrudan oynatma mümkün değilse poster + canonical link fallback'i
   üretin; başarısızlığı kartın tamamını düşürmek için kullanmayın.
5. `PlatformModules`, `ModuleRoleFilters`, `MODULE_KEYS`, `MODULE_LABELS`, `ENGINE_PLATFORMS` ve WebUI
   module/role filter ekranını güncelleyin.
6. `parse_message_with_domains`, `builtin::fetch`, cache key ve telemetry platform etiketlerini güncelleyin.
7. README'de platform matrisi, gerekli permission/cookie ve degraded davranışı ekleyin.

## Önerilen dosya sınırı

`src/converters/parser.rs`, yeni `src/converters/builtin/{twitch,pixiv,tumblr,facebook,youtube}.rs`,
`src/converters/builtin/mod.rs`, `src/converters/builtin/model.rs`, `src/settings.rs`,
`src/web/ui/app.js`, `src/web/ui/index.html`, ilgili test fixture'ları ve README.

## Test matrisi

- Her platform için normal, kısa/share, spoiler ve metin yanında kullanıcı notu parser testi.
- Her adapter için minimal başarılı fixture, eksik alan fixture, invalid JSON/HTML ve 404/429/timeout testi.
- Multi-image/mixed-media, video poster ve link-only fallback testleri.
- Sensitive flag ve mention/markdown escape testi.
- WebUI sözleşme testi: Rust default şeması ↔ JS default/module/role listesi.
- Cache key platform + canonical URL bazında ayrışıyor mu testi.
- `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D warnings`,
  `cargo test --locked`, `node --test tests/web-ui.test.cjs`.

## Kabul kriterleri

- Beş platformun her biri linki tanır, kart üretir ve adapter hatasında orijinal kullanıcı mesajını
  bozmadan güvenli fallback verir.
- Her platform WebUI'da bağımsız açılıp kapanabilir ve rol filtresi uygulanabilir.
- Yeni adapter'lar sınırsız response body, açık redirect veya secret loglamaz.
- Public fixture testleri ağ olmadan çalışır; canlı testler zorunlu değildir.

## Karar gerektiren noktalar

Pixiv ugoira zip'inin dönüştürülmesi, Facebook giriş duvarı, Tumblr API anahtarı ve YouTube Community
Post HTML değişkenliği için her adapter başlamadan önce “poster/link fallback yeterli mi, yoksa tam medya
zorunlu mu?” kararı verilmelidir. Agent bu kararı varsaymamalı; seçilen davranışı adapter raporuna ve
README'ye yazmalıdır.
