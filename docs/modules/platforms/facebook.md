# Facebook

## Görev

Facebook gönderi, video ve reel bağlantılarından metin, izlenme ve tepki sayıları, kapak ve oynatılabilir
video çıkarır. Oturum gerekmez.

## Dosyalar

`builtin/facebook.rs`.

## Girdi ve çıktı

`facebook::fetch(client, &ParsedPost) -> Result<BuiltinPost, String>`.

## Nasıl çalışır

İki herkese açık sayfa okunur:

- **Gönderi sayfası OpenGraph etiketleri:** metin (`og:description`), izlenme ve tepki sayıları (`og:title`
  önekindeki `2.7M views · 66K reactions | ...`, `split_title`), kapak ve sayfa adresi (`og:url`, ör.
  `/Marvel/videos/...`, `page_slug`).
- **Gömme oynatıcı** (`/plugins/video.php`), yalnızca video ve reel için: sayfanın görünen adı ve
  doğrudan MP4 adresi (`hd_src` / `sd_src`). Adres imzalıdır ve süresi dolar, bu yüzden video gönderimde dosya
  olarak eklenir (`render::resolve_media`).

Gönderi sayfası büyüktür (yaklaşık 450 KB) ve yavaş gelebilir. Genel önizlemenin sınırı yerine paylaşılan
istemcinin süresi kullanılır. Tarayıcı User-Agent'ı 2 MB'ı aşan tam uygulamayı döndürdüğü için botun kendi
User-Agent'ı kullanılır. Okunamazsa genel önizlemeye düşülür (`preview::fetch`). Video adresi yalnızca
`fbcdn` alan adlarından kabul edilir (`fbcdn`).

## Değişmezler

- Medya adresi `fbcdn` doğrulamasından geçmeden kullanılmaz.
- Metin markdown kaçışından geçer.
- Uç noktalar belgelenmemiştir. Parse hatası kartı düşürmez, önizlemeye geri düşer.

## Değiştirirken

`og:title` biçimi değişirse `split_title` ve testini güncelleyin.

## Testler

`cargo test --locked facebook`. `live_facebook_reel` `#[ignore]` işaretlidir.
