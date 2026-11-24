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
  link olduğu gibi kalır; ASIN, yönlendirme sonrası sayfadan (`data-asin` / `"asin"`) okunur.
- Sayfa botun kendi kimliğiyle ve ortak `get_text` ile çekilir (`MAX_BODY_BYTES`, 2 MiB; tipik sayfa
  ~1 MB). Tarayıcı taklidi yapılmaz: sahte Chrome UA + rustls parmak izi captcha'yı tetikliyordu.
- Amazon robot engeli: botun kimliğiyle `amzn.eu` kısa link (paylaşım parametreli adrese yönlenir) tam sayfa
  verir; doğrudan `/dp/ASIN` isteğine 503 ya da captcha sayfası döner. Captcha atlatılmaya çalışılmaz:
  `is_captcha` açık hata döner, kart üretilmez ve özgün mesaj kalır.
- Ödenecek fiyat `apex-pricetopay-accessibility-label` içinden okunur; indirimde etiket "Yüzde 2
  tasarruf ile 685,00 TL" olduğu için metindeki son tutar alınır. Sayfada daha önce geçen başka fiyatlar
  (taksit, benzer ürün) okunmaz. Üstü çizili eski fiyat `a-text-price…` + `data-a-strike="true"`
  ("Önceki Fiyat"), indirim `savingsPercentage`; ikisi de fiyat bloğundan.
- Başlık ilk `, `'de bölünür: baş kısım ürün adı, kalan parçalar (en çok 4) künye satırı. Başlıkta künye
  yoksa teknik tablodan (`prodDetSectionEntry`) en çok 3 satır alınır; kesilmeden sığmayan satır atlanır.
- Kart (onaylı konseptin düzeni, `amazon.rs` başındaki şema):
  - Görselli bölüm: küçük mağaza · marka; `##` ürün adı (link değil, link butonda); 🏷️ fırsat rozeti;
    `#` fiyat (kartın en büyük yazısı); varsa küçük satırda ~~eski fiyat~~ −%indirim;
    ⭐ puan · değerlendirme; Ücretsiz teslimat · tarih · **Son N adet**.
  - Ayraç, iki sütunlu `Etiket: değer` künye (en çok 8: işlemci, ekran kartı, bellek, ekran, depolama
    başlıktan; ağırlık, klavye, garanti, bağlantı vb. tablodan), küçük alt satır: fiyatın alındığı an ·
    satıcı · iade.
  - Kartın altında tek "Amazon'da aç" butonu (temiz `/dp/ASIN`).
  - Kategori sırası, aylık satış ve 5 yıldız oranı bilerek gösterilmez (kullanıcı kararı).
- Discord sınırı: metin renklendirilemez (kırmızı rozet, yeşil teslimat, turuncu stok), künye gerçek
  sütun değildir (her satırda iki hücre `·` ile).

- Kart (`Product` + `build_card`) mağazadan bağımsızdır; Trendyol aynı kartı kullanır
  ([trendyol.md](trendyol.md)). Kartı değiştirmek iki mağazayı birden değiştirir.

## Değişmezler

- Section (tip 9) aksesuarsız gönderilmez: Discord reddeder. Görsel yoksa metin bölümsüz konur.
- Kalın beyaz yalnız başlık, fiyat ve aciliyette ("Son N adet"); künye, rozet, teslimat düz yazı
  (okunabilirlik; testle korunur).
- Sıradan "Stokta var" gösterilmez; yalnız az stok / tükendi. Stok yalnızca ürünün kendi
  `#availability` alanından okunur (sayfadaki öneri kartlarında başka ürünlerin stoğu yazar).
- Eski fiyat ve indirim yalnızca Amazon sayfada gösterdiğinde çıkar; bot fiyat geçmişi tutmaz.
- Fiyat bloğu bayt sınırında değil, harf sınırında kesilir (Türkçe harf ortasında panik).

## Değiştirirken

Seçiciler Amazon'un HTML düzenine bağlıdır; düzen değişirse ilgili alan sessizce boş kalır, kart yine
çıkar. Yeni seçiciyi önce gerçek bir sayfaya karşı deneyip testteki `SAMPLE`'ı güncelleyin.

## Testler

`cargo test --locked amazon` (parser ve kart). Ağ testi yoktur.
