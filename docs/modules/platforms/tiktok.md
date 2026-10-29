# TikTok

## Görev

TikTok video ve fotoğraf gönderilerinden yazar, açıklama, sayılar, tarih ve medyayı çıkarır. Videoyu mesaja
dosya olarak ekler.

## Dosyalar

`builtin/tiktok.rs`.

## Girdi ve çıktı

`tiktok::fetch(client, &ParsedPost) -> Result<BuiltinPost, String>`. Ayrıca `video_headers(url)`: medya
indirmesi için gereken `Cookie` ve `Referer` başlıkları.

## Nasıl çalışır

1. Gönderi sayfası alınır ve `__UNIVERSAL_DATA_FOR_REHYDRATION__` verisi okunur (`item_from_html`): yazar,
   açıklama, sayılar, tarih, fotoğraflar ve h264 MP4 seçenekleri.
2. `best_video` video dosya sınırına (`MAX_VIDEO_BYTES` = Discord dosya sınırı) sığan en iyi h264 seçeneğini
   seçer.
3. Video CDN adresi yalnızca sayfayı veren isteğin `tt_chain_token` çerezi ve TikTok `Referer`'ıyla
   verilir. Çerez `remember_token` ile adrese bağlanarak saklanır (en fazla 512 kayıt), `video_headers` ile
   indirme isteğine eklenir. Discord bu başlıkları gönderemediği için video mesaja dosya olarak eklenir.
   TikTok `video_requires_download` listesindedir.
4. Video indirilemezse kartta kapak gösterilir (`video_to_poster`).
5. Sayfa okunamazsa oEmbed'e düşülür: yalnızca başlık, yazar ve kapak verir.

## Değişmezler

- Çerez ve belirteç loglanmaz.
- Çerez önbelleği sınırlıdır (`MAX_TOKENS`).
- Video yalnızca dosya sınırına sığıyorsa eklenir.

## Değiştirirken

TikTok sayfa yapısı değişirse `item_from_html` ve `Item` tiplerini güncelleyin, fixture HTML ile test edin.

## Testler

`cargo test --locked tiktok`.
