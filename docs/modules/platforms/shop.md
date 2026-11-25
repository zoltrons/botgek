# Mağaza ürün kartı (ortak)

## Görev

Amazon, Trendyol ve Hepsiburada ürün sayfalarından okunan `Product`'ı tek, ortak kartta gösterir.

## Dosyalar

`builtin/shop.rs`: `Product`, `product_post`, `build_card`, tutar yardımcıları (`value`, `savings`), ortak
`MAX_TITLE_CHARS` ve Trendyol/Hepsiburada'nın aynı puan/değerlendirme desenleri (`RATING`, `REVIEWS`).
Mağaza modülleri (`amazon.rs`, `trendyol.rs`, `hepsiburada.rs`) yalnız sayfayı `Product`'a çevirir.

## Kart

Klasik (v1) embed + altında tek link butonu. Kart Components V2 değildir: `CustomCard.embed` doluyken
`v2::build_payload` mesajı `embeds` + `components` (buton satırı) olarak kurar, V2 bayrağı konmaz;
kullanıcının linkin altına yazdığı not mesaj metni (`content`) olur. Kullanıcı embed'i seçti: satır içi
alanlar gerçek sütundur ve zaman damgasını izleyicinin cihazı yerel saatle yazar.

- Yazar satırı (küçük): mağaza · marka.
- Başlık: ürün adı (link değil; link butonda, izleme parametresiz).
- Küçük resim: ürün görseli (tam boy adres).
- Açıklama: `#` fiyat (kartın en büyük yazısı); altında tek `-#` satırda ~~eski fiyat~~ −%indirim ·
  fırsat rozeti; ★ puan · değerlendirme (düz yıldız, emoji değil); teslimat · **Son N adet**.
- Satır içi alanlar: künye, en çok 6 (`Etiket` / değer).
- Alt bilgi: satıcı (markadan farklıysa) · iade; zaman damgası = fiyatın alındığı an (yalnız fiyat varken).
- Renk mağazanın rengi; buton "Amazon'da aç" / "Trendyol'da aç" / "Hepsiburada'da aç".
- Kategori sırası, aylık satış ve 5 yıldız oranı bilerek gösterilmez (kullanıcı kararı).

## Değişmezler

- Kalın yalnız aciliyette ("Son N adet"); künye, rozet, teslimat düz yazı (testle korunur).
- Boş alan kendi parçasını düşürür; kart yine çıkar (açıklama boşsa alan hiç konmaz: Discord boş dizeyi reddeder).
- Spoiler/hassas kartta (`v2::spoil_embed`) küçük resim, yazar satırı (mağaza · marka) ve alt bilgi
  (satıcı) atılır; başlık, açıklama ve künye (`Etiket: değer`) başlık işaretleri (`#`, `-#`) atılmış düz
  satırlar olarak tek `||…||` bloğuna konur. Önbellekteki kart değişmez.
- Kullanıcı notu 2000 karakteri aşarsa (Nitro) mesaj metni yerine kartın üstünde yalnız açıklamalı bir
  embed olur (4096'ya kadar): asıl mesaj silindiği için not kesilmez.
- Mağazaya özel kart dalı açılmaz; gerekirse `Product` alanı eklenir. Kartı değiştirmek üç mağazayı birden değiştirir.

## Değiştirirken

Kart değişikliğini gerçek bir sayfanın datacat çıktısıyla test kanalında görün (bkz. `amazon.md`, `trendyol.md`,
`hepsiburada.md` örnekleri). Eski V2 mağaza kartları düzenlenirse Discord V2 bayrağının kaldırılmasını
reddeder; o mesaj güncellenmez (yeni kartlar etkilenmez).

## Testler

`cargo test --locked shop` (kart), `cargo test --locked v2::tests::embed_card` (gönderim gövdesi, spoiler).
