# TikTok

## Görev

TikTok video ve fotoğraf gönderilerinden yazar, açıklama, sayılar, tarih ve medyayı çıkarır. Videoyu fxTikTok
adresiyle kartta uzaktan oynatır.

## Dosyalar

`builtin/tiktok.rs`.

## Girdi ve çıktı

`tiktok::fetch(client, &ParsedPost) -> Result<BuiltinPost, String>`.

## Nasıl çalışır

1. Gönderi sayfası alınır ve `__UNIVERSAL_DATA_FOR_REHYDRATION__` verisi okunur (`item_from_html`): yazar,
   açıklama, sayılar, tarih, fotoğraflar, video boyutu ve kapak.
2. Sayfadaki CDN adresleri yalnızca o isteğin `tt_chain_token` çereziyle açılır; Discord çerez
   gönderemez. Kartta video adresi olarak fxTikTok'un `https://tnktok.com/generate/video/<kimlik>`
   adresi kullanılır: çerezsiz, imzalı bir MP4'e 302 verir ve Discord videoyu uzaktan oynatır. Bot
   videoyu indirmez ve yüklemez (önceden kart bu indirme+yükleme yüzünden saniyelerce bekliyordu).
3. tnktok.com çalışmazsa kartta video oynamaz; dosya eki yedeği yoktur.
4. Sayfa okunamazsa oEmbed'e düşülür: yalnızca başlık, yazar ve kapak verir.

## Değişmezler

- Video adresi her zaman `FXTIKTOK_VIDEO` + sayısal gönderi kimliğidir.

## Değiştirirken

TikTok sayfa yapısı değişirse `item_from_html` ve `Item` tiplerini güncelleyin, fixture HTML ile test edin.

## Testler

`cargo test --locked tiktok`.
