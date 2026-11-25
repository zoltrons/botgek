# Amazon

## Görev

Amazon ürün bağlantılarını (`amazon.com.tr` / `amazon.com` `/dp/{ASIN}` ve `/gp/product/{ASIN}`,
`amzn.eu/d/…` ve `amzn.to/…` kısa linkleri) temiz bir ürün kartına çevirir. Discord'un kendi önizlemesi
başlığı kırpar, fiyat göstermez ve küçük görsel olarak Amazon'un tanıtım şeridini koyar.

## Dosyalar

`builtin/amazon.rs`.

## Girdi ve çıktı

`amazon::fetch(client, &ParsedPost) -> Result<BuiltinPost, String>`: `CustomCard` (Components V2), medya yok.

## Nasıl çalışır

- Parser `/dp/ASIN` bağlantısını izleme parametresiz `https://www.{alan}/dp/{ASIN}` adresine indirger. Kısa
  link olduğu gibi kalır; ASIN, yönlendirme sonrası sayfadan (künye tablosundaki `ASIN` satırı ya da marka
  linkindeki `lp_asin`) okunur.
- Amazon botun doğrudan isteğine 503 ya da captcha döner. Sayfa ortak `get_markdown` ile datacat vekilinden
  Markdown olarak çekilir (~100–450 KB; HTML seçeneği yok). Datacat önbellekte olmayan sayfayı
  6,5–15 sn'de döndürdüğü için istek sınırı ortak 6 sn değil, 20 sn'dir (`DATACAT_TIMEOUT`). Captcha atlatılmaya çalışılmaz: `is_captcha`
  açık hata döner, kart üretilmez ve özgün mesaj kalır.
- Datacat iki biçim üretebilir (temiz Markdown ya da HTML artıklı); kalıplar satır başı boşluklarına ve
  `*`/`_` farkına dayanıklıdır. Ürün başlığı sayfadaki ilk tek `# ` başlığıdır. Satın alma kutusu (teslimat,
  stok, satıcı, ürün görseli) başlıktan önce, fiyat bloğu başlıktan sonra "Taksitler"e kadar gelir.
- Ödenecek fiyat başlıktan sonraki ilk tutar satırıdır; indirimde satır "Yüzde 3 tasarruf ile 38.999,00 TL
  \-%3 38.999,00TL" olur, son tutar alınır, indirim `-%N`'den. Eski fiyat "Önceki Fiyat:" satırı. Taksit
  tablosu (dört sütunlu) ve öneri kartlarının fiyatları okunmaz.
- Fırsat rozeti geri sayım şablonundan önceki metin ("Gülümseten Kasım fırsatı NO\_OF\_HOURS…").
- Görsel: alt metni ürün adıyla başlayan ilk görsel; boyut eki atılır (`71l4M2y8zwL._AC_SX679_.jpg` →
  `71l4M2y8zwL.jpg`, özgün tam boy).
- Başlık ilk `, `'de bölünür: baş kısım ürün adı, kalan parçalar (en çok 4) künye satırı. Başlıkta künye
  yoksa iki sütunlu künye tablolarından en çok 3 satır alınır; kesilmeden sığmayan satır atlanır.
- Kart: ortak mağaza kartı (v1 embed), bkz. [shop.md](shop.md). Amazon'a özgü: turuncu `#FF9900`,
  "Amazon'da aç" butonu (temiz `/dp/ASIN`), alt bilgide iade süresi.

## Değişmezler

- Sıradan "Stokta var" gösterilmez; yalnız az stok / tükendi. Stok yalnızca başlıktan önceki
  satın alma kutusundan okunur (başlıktan sonraki öneri kartlarında başka ürünlerin stoğu yazar).
- Eski fiyat ve indirim yalnızca Amazon sayfada gösterdiğinde çıkar; bot fiyat geçmişi tutmaz.

## Değiştirirken

Kalıplar datacat'in Markdown çıktısına ve Amazon'un düzenine bağlıdır; düzen değişirse ilgili alan sessizce
boş kalır, kart yine çıkar. Yeni kalıbı önce gerçek bir sayfanın datacat çıktısında deneyip testteki
`SAMPLE`'ı güncelleyin (iki biçimi de deneyin).

## Testler

`cargo test --locked amazon` (parser ve kart). Ağ testi yoktur.
