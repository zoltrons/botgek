# Botgek ↔ Embedded feature-parity ana raporu

**Tarih:** 13 Ekim 2026  
**Referans ürün:** [Embedded](https://embedded.gallery/)  
**Platform referansı:** [Embedded platform listesi](https://embedded.gallery/#features)  
**İncelenen kaynak:** Botgek çalışma ağacı (Rust + Serenity + Axum WebUI)  
**Çalışma yöntemi:** Embedded ana sayfası ve platform sayfaları ile Botgek'in parser, native adapter, render, handler, settings ve WebUI sözleşmeleri karşılaştırıldı.

> Bu rapor Embedded'in herkese açık ürün sayfalarındaki beyanları karşılaştırma kabulü olarak kullanır.
> Siteye ait kapalı kaynak kod, gerçek runtime davranışı veya premium ödeme akışı doğrulanmadı.
> Dolayısıyla aşağıdaki “Embedded destekliyor” ifadeleri ürün sayfası beyanıdır; “Botgek mevcut” ifadeleri
> kaynak koddan doğrulanmıştır.

## Uygulama sonrası durum (13 Ekim 2026)

Bu raporun ilk boşluk analizinden sonra premium/branding kapsamı kullanıcı tarafından bilinçli olarak
çıkarıldı. EG-01–EG-06 kapsamındaki ilk uygulama paketi çalışma ağacına eklendi:

- Global → guild → thread parent → channel sparse ayar çözümlemesi ve WebUI kanal seçimi.
- NSFW/sensitive görünürlük kararı, V2/metin/galeri/attachment ve quote-media yollarına uygulandı.
- Twitch, Pixiv, Tumblr, Facebook ve YouTube Community Posts için bounded public-preview adapter'ları.
- Reddit direct comment context + Redgifs güvenli canonical/media fallback'i.
- Bluesky bounded parent/reply/repost context'i.
- TikTok optional rich metadata/video alanları.
- Yapılandırılabilir X translation target.
- Opt-in, impersonation'sız ve duplicate-guard'lı bot mesajı dönüşümü.

EG-07 (premium/branding/entitlement) intentionally uygulanmadı.

## 1. Yönetici özeti

Botgek'in mevcut çekirdeği X, Instagram, Reddit, Bluesky ve TikTok bağlantılarını işler; Spotify da
ek bir platform olarak desteklenir. Components V2 kartı, medya galerisi, kullanıcı taklidi, guild bazlı
ayar ve rol filtresi gibi güçlü parçalar zaten vardır. Embedded ile feature parity için en büyük açıklar:

1. **Platform kapsamı:** Twitch, Pixiv, Tumblr, Facebook ve YouTube Community Posts adapter'ları yok.
2. **Kanal kapsamı:** Ayarlar guild ve global seviyede; Embedded'in iddia ettiği kanal bazlı override yok.
3. **Güvenlik/uygunluk politikası:** Sensitive/NSFW içerik tespiti kısmen var, ancak Discord kanalının NSFW
   durumuna göre otomatik gösterim politikası yok.
4. **Çeviri:** Botgek'te X için otomatik Türkçe ve proxy mesajlar için sabit bayrak dili akışı var;
   hedef dili yapılandırılabilir “herhangi bir dil” özelliği yok.
5. **Platform bağlamı:** Reddit doğrudan yorum önizlemesi ve Redgifs ses desteği; Bluesky reply/repost
   bağlamı; TikTok zengin video/istatistik verisi eksik veya yalnızca proxy'ye bırakılmış.
6. **Bot/premium ürün özellikleri:** Bot mesajlarına yanıt verme, sunucuya özel bot avatarı, watermark'sız
   taklit, container renkleri ve premium/abonelik kotası modeli yok.

## 2. Mevcut durumun doğrulanmış özeti

| Alan | Botgek'te görülen durum | Parity durumu |
|---|---|---|
| X | Native adapter; medya/GIF, metrik, timestamp, quote, Community Note ve X kaynaklı çeviri | Büyük ölçüde mevcut |
| Reddit | Native post/title/selftext/gallery/video/vote; sesli video birleştirme akışı mevcut; doğrudan comment preview yok | Kısmi |
| Bluesky | Native profil, medya, metrik, external/quote ve adult label | Kısmi: reply thread/repost bağlamı yok |
| TikTok | oEmbed native kartı yalnızca başlık/yazar/thumbnail verir; varsayılan proxy video için kullanılabilir | Kısmi |
| Instagram | Public embed/OG; sessionid varsa reel mp4 ve carousel zenginleşir | Koşullu/kısmi |
| Spotify | Native adapter ve Components V2 özel kartı var | Embedded parity için ekstra |
| Twitch/Pixiv/Tumblr/Facebook/YouTube Posts | Parser/platform enum/adapter yok | Eksik |
| Medya galerisi | Components V2 Media Gallery; bir video + görseller; spoiler işaretleme | Büyük ölçüde mevcut |
| Mixed media | Model `Vec<Media>` taşıyor ve galeri video+görsel gösterebiliyor; birden fazla video/özel sıralama sınırlı | Kısmi |
| Guild ayarı | Global + sparse guild override (`modules`, `role_filters`, `ai`, `engine`) | Mevcut |
| Kanal ayarı | Kanal ID tabanlı ayar modeli/arayüzü yok | Eksik |
| NSFW/spoiler | X/Reddit/Bluesky bazı sensitive işaretlerini kartta spoiler yapıyor; kullanıcı `||...||` de destekleniyor | Kısmi |
| Çeviri | Bayrak reaksiyonlarında sabit dil listesi; X için Türkçe auto-translate | Kısmi |
| Bot yanıtları | `msg.author.bot` mesajları erken eleniyor | Eksik |
| Premium | Ürün/paket/abonelik/kota/entitlement modeli yok | Opsiyonel eksik |

## 3. Önceliklendirme

| ID | Rapor | Öncelik | Gerekçe |
|---|---|---:|---|
| EG-01 | Platform adapter paketi | P0 | Rakibin public platform kapsamının yarısından fazlası eksik |
| EG-02 | Kanal bazlı ayar ve override | P0 | Embedded'in açıkça öne çıkardığı configurable-per-channel davranış |
| EG-03 | NSFW/sensitive kanal politikası | P0 | Güvenlik ve içerik görünürlüğü doğrudan kullanıcı etkiler |
| EG-04 | Her hedef dile çeviri | P1 | Public feature claim; mevcut akış yalnızca kısmi |
| EG-05 | Platform bağlamı ve zengin medya | P1 | Reddit comment, Redgifs, Bluesky thread/repost, TikTok veri kaybı |
| EG-06 | Bot mesajlarına yanıt | P1 | Premium sayfada açık özellik; mevcut guard ters yönde |
| EG-07 | Premium/branding/entitlement | P2 | Core parity dışı; yalnızca ticari parity istenirse uygulanmalı |

## 4. Uygulama sırası

1. **EG-02 + EG-03:** Policy ve ayar katmanını kanal/guild sınırlarıyla kurun. Platform adapter'ları
   tamamlanmadan önce bu ortak katmanın stabil olması gerekir.
2. **EG-01:** Ortak `BuiltinPost` modelini genişletmeden önce adapter capability sözleşmesini tanımlayın.
   Beş platformu tek PR'da değil, adapter başına ayrı dilimlerde ekleyin.
3. **EG-05:** Reddit/Bluesky/TikTok için mevcut adapter'ları geriye dönük testlerle zenginleştirin.
4. **EG-04:** Translation provider, hedef dil validasyonu, quota ve cache anahtarını ortaklaştırın.
5. **EG-06:** Bot mesajı işleme için loop guard ve idempotency marker ekleyin; mevcut `msg.author.bot` guard'ını
   doğrudan kaldırmayın.
6. **EG-07:** Ancak ürün kararında premium kapsamı onaylandıktan sonra ödeme/entitlement entegrasyonuna geçin.

## 5. Ortak ajan çalışma kuralları

- Mevcut Components V2 gönderim modelini klasik embed'e geri döndürmeyin.
- Adapter'lar yalnızca veri/model üretmeli; Discord REST gönderimi `builtin/mod.rs` ve transport katmanında
  kalmalı.
- Harici HTTP yanıtlarında `REQUEST_TIMEOUT`, `MAX_BODY_BYTES`, explicit status kontrolü ve güvenli redirect
  politikasını paylaşın. Yeni adapter kendi sınırsız `response.text()` yolunu açmamalı.
- Kullanıcı içeriğini markdown/mention açısından escape edin; LLM, platform metni veya HTML'den gelen
  `@everyone`/rol mention'larını güvenilir kabul etmeyin.
- Secret, cookie veya token'ı ayar JSON'una, fixture'a, rapora ve log'a yazmayın.
- Her yeni davranış için: parser fixture'ı, adapter JSON/HTML fixture'ı, model dönüşüm testi, render testi,
  hata/timeout testi ve mevcut `cargo test --locked` regresyonu ekleyin.
- Canlı upstream testi yalnızca açıkça ignore edilen test olarak yazılmalı; normal test koşusu internete
  güvenmemeli.

## 6. Raporların kullanım biçimi

Her alt rapor bir AI coding agent'a doğrudan verilebilecek şekilde şu sözleşmeyi içerir:

- hedef davranış ve kapsam dışı maddeler,
- mevcut kod kanıtı ve önerilen dosya sınırı,
- veri/API sözleşmesi,
- adım adım uygulama planı,
- kabul kriterleri ve test matrisi,
- güvenlik, rollback ve karar verilmesi gereken noktalar.

Alt raporlar:

- [EG-01 Platform adapter paketi](embedded-gallery-gap-platform-adapters.md)
- [EG-02 Kanal bazlı ayar ve override](embedded-gallery-gap-channel-policies.md)
- [EG-03 NSFW/sensitive politika](embedded-gallery-gap-content-safety.md)
- [EG-04 Her hedef dile çeviri](embedded-gallery-gap-translation.md)
- [EG-05 Platform bağlamı ve zengin medya](embedded-gallery-gap-context.md)
- [EG-06 Bot mesajlarına yanıt](embedded-gallery-gap-bot-response.md)
- [EG-07 Premium/branding/entitlement](embedded-gallery-gap-premium.md)

## 7. Kaynaklar

- [Embedded ana sayfa](https://embedded.gallery/): görsel tutarlılık, NSFW/spoiler yönetimi, kanal bazlı ayar,
  tüm dillere çeviri, platform listesi, mixed media ve gallery karşılaştırması.
- [X platform sayfası](https://embedded.gallery/platforms/twitter): media, translation, timestamp/metrics,
  Community Notes, quote posts.
- [Reddit platform sayfası](https://embedded.gallery/platforms/reddit): post content, Redgifs/video sound,
  vote score, direct comment preview.
- [Bluesky platform sayfası](https://embedded.gallery/platforms/bluesky): profile, metadata, quote,
  reply threads, reposts, link previews.
- [TikTok platform sayfası](https://embedded.gallery/platforms/tiktok): video preview, creator info,
  description/hashtags, likes/comments/shares/views.
- [Instagram platform sayfası](https://embedded.gallery/platforms/instagram): galleries, reels/videos,
  profile, captions/tags.
- [Twitch platform sayfası](https://embedded.gallery/platforms/twitch), [Pixiv](https://embedded.gallery/platforms/pixiv),
  [Tumblr](https://embedded.gallery/platforms/tumblr), [Facebook](https://embedded.gallery/platforms/facebook),
  [YouTube Posts](https://embedded.gallery/platforms/youtube): public platform ve content-type iddiaları.
- [Embedded Premium](https://embedded.gallery/#premium): custom avatar, watermark'sız impersonation,
  container color, bot response, higher translation limit.
