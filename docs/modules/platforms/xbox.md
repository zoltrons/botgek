# Xbox

## Görev

Xbox oyun kliplerini (`xbox.com/play/media/{id}`, dil önekiyle ya da öneksiz) oyuncu etiketi, oyun ve
tarihle karta çevirir; videoyu indirmeden doğrudan MP4 adresiyle kartın içinde oynatır. Discord'un kendi
önizlemesi sayfada `og:video` olmadığı için klibi oynatamaz.

## Dosyalar

`builtin/xbox.rs`.

## Girdi ve çıktı

`xbox::fetch(client, &ParsedPost) -> Result<BuiltinPost, String>`: genel kart, tek `MediaKind::Video`.

## Nasıl çalışır

- Kanonik adres `https://www.xbox.com/play/media/{id}`; sayfa oturumsuz çekilir.
- `window.__PRELOADED_STATE__` nesnesi okunur. `xcloud.platform.xboxSocialClientReduxState` bir JSON
  **dizesidir**; içindeki `feed.linkPreviews[id].data.item` klibin kendisidir.
- `originalContentUri` imzalı (yaklaşık 45 dakika geçerli) MP4 adresidir ve medya olarak doğrudan kullanılır.
  Video indirilmez; imza dolduktan sonra eski mesajlarda oynatma Discord'un medya önbelleğine kalır.
- Klip bulunamazsa ya da öğe `GameClip` değilse hata döner (genel önizleme yedeği yoktur).

## Değişmezler

- Kimlik yalnızca kanonik URL'den alınır ve alfanümerik olmalıdır.
- `http://` görsel adresleri `https://`'e çevrilir; başka şemalar atılır.

## Değiştirirken

Xbox sayfa durumunun yolu değişirse `find_clip` ve `Clip` tipleri birlikte güncellenir.

## Testler

`cargo test --locked xbox`; canlı: `cargo test xbox -- --ignored`.
