# EG-03 — NSFW, sensitive ve spoiler kanal politikası

**Öncelik:** P0  
**Hedef:** Platformdan gelen `sensitive/NSFW/spoiler` sinyalini Discord kanalının NSFW durumuyla
birlikte değerlendirip metin, görsel, video ve attachment katmanlarında tutarlı görünürlük sağlamak.

## Mevcut durum ve kanıt

`BuiltinPost.sensitive` modeli var. X `possibly_sensitive`, Reddit `over_18 || spoiler`, Bluesky adult
label'larını kısmen dolduruyor. `v2.rs` içindeki `hidden()` sensitive veya kullanıcı linkinin
`||...||` ile işaretlenmesini kart metninde spoiler ve galeri öğelerinde spoiler olarak uyguluyor.
Bu, Embedded'in “auto mark NSFW/spoilers” kısmının bir bölümünü karşılıyor.

Eksik olanlar:

- Discord channel NSFW/sensitive flag'i okunup politika olarak kullanılmıyor.
- Sensitive olmayan platformlar için ortak classifier/metadata contract yok.
- “Kanal sensitive ise unblurred göster” veya “her zaman gizle” seçeneği yok.
- Attachment/video fallback ve external link'in aynı politika ile zorlandığına dair ortak contract yok.
- Kullanıcıya neden gizlendiği ve başarısız metadata durumunun davranışı belgeli bir ürün kuralı değil.

## Önerilen politika

```text
SensitiveMode =
  hide_always       # kaynak sensitive ise kanal durumundan bağımsız spoiler
  allow_in_nsfw     # kaynak sensitive ise yalnızca NSFW channel'da açık
  show_always       # yalnızca açıkça yönetici tarafından seçilirse; önerilmez
```

Varsayılan `hide_always` olmalıdır. Embedded parity için `allow_in_nsfw` kanal override'ı eklenebilir.
Kaynak sinyali bilinmiyorsa içerik “safe” ilan edilmemeli; detection unknown ayrı bir durum olarak
izlenmelidir. User-supplied `||url||` her zaman spoiler zorlamalıdır.

## Uygulama planı

1. `BuiltinPost` içine `sensitivity: Sensitivity { source_flag, user_spoiler, confidence }` veya
   geriye dönük uyumlu bir enum ekleyin. Mevcut `sensitive: bool` alanını tek release'te kaldırmayın;
   önce adapter'ları migrate edin.
2. Adapter'larda kaynak metadata'sını normalize edin: X/Reddit/Bluesky mevcut sinyalleri, Instagram
   ve yeni platformlar için mevcut public flag'leri, bilinmeyen için `Unknown`.
3. Discord channel metadata'sını cache'leyin; `Channel::nsfw` veya eşdeğer Serenity alanını message
   context'ine taşıyın. Cache miss/permission error güvenli varsayıma düşmeli.
4. EG-02'nin resolved channel policy'si üzerinden `sensitive_mode` çözün.
5. Render katmanında tek `VisibilityDecision` üretin ve bunu şuralarda kullanın:
   - Text Display içindeki title/body/quote/link preview,
   - Media Gallery item'larının spoiler flag'i,
   - `attachment://` dosya adının `SPOILER_` prefix'i,
   - video direct-link fallback'i.
6. Kart meta satırına isteğe bağlı kısa `Sensitive` etiketi ekleyin; gerçek NSFW ayrıntısını kullanıcı
   içeriğinden tahmin ederek loglamayın.
7. WebUI'da guild/channel policy alanını açıklama, preview ve reset davranışıyla ekleyin. “NSFW kanal
   otomatik açık” seçeneğini varsayılan kapalı tutun.

## Kabul kriterleri

- Sensitive kaynak + SFW kanal: metin ve tüm medya spoiler; attachment `SPOILER_` ile gönderilir.
- Sensitive kaynak + NSFW kanal + `allow_in_nsfw`: ürün kararına göre unblurred; link preview dahil tüm
  bileşenler aynı kararı kullanır.
- Kullanıcı `||...||` işaretlediyse NSFW kanalında bile spoiler kalır.
- Metadata unknown durumda sistem içeriği otomatik olarak safe kabul etmez.
- Reddit/X/Bluesky fixture'ları mevcut davranışını korur; false positive olmayan safe testleri vardır.
- Channel API/cache hatası kartı açık göndermek yerine güvenli fallback uygular.

## Test matrisi

`VisibilityDecision` için tablo bazlı unit test; V2 payload snapshot test; attachment filename test;
channel NSFW true/false/missing/permission error fixture; thread parent inheritance test; WebUI settings
round-trip; canlı Discord testini zorunlu olmayan canary olarak ekleyin.

## Güvenlik notu

Bu özellik bir içerik moderasyonu/garantisi değildir. README'de “platform metadata'sına dayalı otomatik
işaretleme” ifadesi kullanılmalı; görsel classifier yapılmadıysa görsel içerik güvenli diye sunulmamalı.
