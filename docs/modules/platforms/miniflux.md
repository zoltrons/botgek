# Miniflux

## Görev

`https://miniflux.{alan}.{uzantı}/share/{40 hex}` paylaşım bağlantılarını kart yapar. Sayfada OpenGraph
etiketi olmadığı için Discord önizlemesi boş kalır.

## Dosyalar

`builtin/miniflux.rs`; buton işleyicisi `commands/mod.rs` içindeki `edit_card_component`.

## Nasıl çalışır

- Parser yalnızca `miniflux` alt alan adını tanır (`miniflux.site.url`); kök alan adı (`miniflux.site`), http
  ve alt dizin kurulumları tanınmaz.
- `safe_http::preview` ile sayfa çekilir; başlık, kaynak, yazar, ilk görsel ve `entry-content` okunur.
- Metin 1000 karaktere (`PREVIEW_CHARS`) kadar tam gösterilir, buton yalnızca "Makaleyi aç"tır (Miniflux share linki).
- Daha uzunsa 1000 karaktere kısaltılır ve "Devamını oku" (`miniflux:p0:{kod}:{sunucu}`) konur. Tıklayınca mesaj
  yerinde 2000 karakterlik sayfalara (`PAGE_CHARS`; paragraf/satır/boşluk sınırında bölünür) geçer:
  `◀ n/N ▶ Daralt` (tek sayfaysa yalnızca `Daralt`). `less` özete döner.
  Gövde ve buton satırı sabit `id`'lerle (7401, 7402) bulunur; spoiler sarmalı korunur.
- Makale HTML'i Discord markdown'ına çevrilir (`html_to_text`): başlık ve kalın/italik/kod/bağlantı/liste korunur,
  `blockquote` ve gömülü içerik (iframe, video, ses) `>` alıntısına dönüşür (iç içe alıntı düzleştirilir),
  `pre` kod bloğu olur, görsel/script atılır. Yalnızca http(s) bağlantılar alınır.

## Değişmezler

- `custom_id` 100 karakteri aşarsa sayfa butonları konmaz, yalnızca ilk sayfa gösterilir.
- Toggle sırasında sunucu adı ve kod yeniden doğrulanır.

## Testler

`cargo test --locked miniflux`.
