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
- Sayfa tarayıcı UA'sıyla çekilir (bot UA'sına içeriği boşaltılmış sayfa dönebilir). Gövde
  `MAX_BODY_BYTES` (2 MiB) ile sınırlıdır; tipik ürün sayfası ~1,5 MB.
- Ödenecek fiyat `apex-pricetopay-accessibility-label` içinden okunur. Sayfada daha önce geçen başka
  fiyatlar (taksit, benzer ürün) okunmaz. Üstü çizili eski fiyat yalnızca `a-text-price` +
  `data-a-strike="true"` ile, indirim `savingsPercentage` ile; ikisi de fiyat bloğundan.
- Başlık ilk `, `'de bölünür: baş kısım ürün adı, kalan parçalar (en çok 4) künye satırı. Başlıkta künye
  yoksa teknik tablodan (`prodDetSectionEntry`) ilk anlamlı satırlar alınır.
- Kart: üst satırda mağaza (host'tan) · marka, kalın ürün adı, fiyat satırı, puan · değerlendirme ·
  stok, ayraç, künye; kartın altında tek "Amazon'da aç" butonu (temiz `/dp/ASIN` linki).

## Değişmezler

- Section (tip 9) aksesuarsız gönderilmez: Discord reddeder. Görsel yoksa metin bölümsüz konur.
- Sıradan "Stokta var" gösterilmez; yalnız az stok / tükendi.
- Eski fiyat ve indirim yalnızca Amazon sayfada gösterdiğinde çıkar; bot fiyat geçmişi tutmaz.
- Fiyat bloğu bayt sınırında değil, harf sınırında kesilir (Türkçe harf ortasında panik).

## Değiştirirken

Seçiciler Amazon'un HTML düzenine bağlıdır; düzen değişirse ilgili alan sessizce boş kalır, kart yine
çıkar. Yeni seçiciyi önce gerçek bir sayfaya karşı deneyip testteki `SAMPLE`'ı güncelleyin.

## Testler

`cargo test --locked amazon` (parser ve kart). Ağ testi yoktur.
