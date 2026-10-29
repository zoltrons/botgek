# Pixiv

## Görev

Pixiv eserlerini başlık, açıklama, etiketler, sanatçı, tarih, sayılar ve tüm sayfalarıyla karta çevirir.

## Dosyalar

`builtin/pixiv.rs`.

## Girdi ve çıktı

- `pixiv::fetch(client, &ParsedPost) -> Result<BuiltinPost, String>`
- `pixiv::image_headers(url)`: `i.pximg.net` için `Referer` başlığı.
- `pixiv::mirror(url)`: aynı yolu sunan `i.pixiv.re` adresi.

## Nasıl çalışır

- Veri sitenin web istemcisinin `/ajax/illust/{id}` ve `/pages` uçlarından, oturumsuz okunur.
- En fazla 10 sayfa (`MAX_PAGES`) ve 6 etiket (`MAX_TAGS`) alınır. Açıklama HTML'den temizlenir
  (`comment_text`).
- R-18 eserler hassas (`sensitive`) işaretlenir.
- `i.pximg.net` Pixiv `Referer`'ı olmadan 403 verir ve Discord'un medya vekili bu başlığı gönderemez. Görseller
  bu yüzden gönderimde `image_headers` ile indirilip ek olarak konur (`render::attach_pixiv_images`,
  görsel başına sınır Discord dosya sınırının dörtte biri). İndirilemeyen görsel `i.pixiv.re` aynasından
  gösterilir. Ayna önbelleğinde olmayan görseli birkaç saniyede verdiği için kırık kalabilir, bu yüzden
  yalnızca yedektir.
- Uç nokta belgelenmemiştir. Okunamazsa genel OG önizlemesine düşülür (`preview::fetch`).

## Değişmezler

- Yalnızca `i.pximg.net` adresleri `Referer` ve ayna davranışı alır.
- Açıklama metni markdown kaçışından geçer (`preview::escape_preview_markdown`).

## Değiştirirken

Pixiv yanıt zarfı (`Envelope`, `unwrap`) değişirse hata iletisini koruyun, böylece önizleme yedeği devreye girer.

## Testler

`cargo test --locked pixiv`. `live_pixiv_artwork` ve `live_pixiv_images_are_attached` `#[ignore]` işaretlidir.
