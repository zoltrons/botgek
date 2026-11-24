# Trendyol

## Görev

Trendyol ürün bağlantılarını (`trendyol.com` / `m.trendyol.com` `/{marka}/{ad}-p-{id}`) Amazon ile aynı
ürün kartına çevirir.

## Dosyalar

`builtin/trendyol.rs`; kart `builtin/amazon.rs` içindeki `Product` + `build_card`.

## Girdi ve çıktı

`trendyol::fetch(client, &ParsedPost) -> Result<BuiltinPost, String>`: `CustomCard` (Components V2), medya yok.

## Nasıl çalışır

- Parser sorgu parametrelerini (`boutiqueId`, `merchantId`, izleme) atar.
- Trendyol botun isteğine 403 döner. Sayfa ortak `get_markdown` ile datacat vekilinden
  (`https://datacat.2g8nbyrb.workers.dev/{url}`, Amazon da kullanır) Markdown olarak alınır; alanlar satır düzeninden okunur:
  - Başlık `# [**Marka**](…) Ürün adı`; başlık yoksa (silinmiş ürün, "Sayfa Bulunamadı") kart çıkmaz.
  - Puan başlıktan sonraki ilk satır, değerlendirme `**N** Değerlendirme`.
  - Fiyat: başlıktan "Ek Hizmetler" / "Şimdi Al"a kadar yalnız tutardan oluşan satırlar. İlk tutar ödenecek
    fiyat, son tutar (daha yüksekse) eski fiyat; indirim oranı bu ikisinden hesaplanır. "Trendyol Plus'a Özel"
    fiyat yalnız üyelere geçerli olduğu için atlanır. Taksit ve ek hizmet tutarları okunmaz.
  - "Kampanyalı Fiyat" varsa 🏷️ rozet; "Son N Ürün!" → **Son N adet**.
  - Teslimat: "…kapında" satırı; "Ücretsiz Kargo" varsa ücretsiz, koşullu kargoda ("350 TL ve Üzeri") yalnız tarih.
  - Satıcı ilk `/magaza/` linki; künye ilk "… Özellikleri" bölümünün anahtar/değer çiftleri (en çok 8, uzunlar atlanır).
  - Görsel: ilk `/prod/` ürün görselinin `mnresize/G/Y/` atılmış özgün boyutu.
  - Buton adresi değerlendirme linkindeki ürün yoludur (eski id başka ürüne yönlenebilir).
- Kart: Amazon konseptinin aynısı, turuncu `#F27A1A`, "Trendyol'da aç" butonu. İade süresi sayfada yok.

## Değişmezler

- Kart düzeni Amazon'la ortaktır; Trendyol'a özel kart dalı açılmaz, gerekirse `Product` alanı eklenir.
- Plus üyelerine özel fiyat ana fiyat olarak gösterilmez.

## Değiştirirken

Seçiciler datacat'in Markdown çıktısına bağlıdır. Yeni kural önce gerçek bir sayfanın datacat çıktısında
denenip testteki `SAMPLE` güncellenir. `ty.gl` kısa linkleri desteklenmez (örnek yok).

## Testler

`cargo test --locked trendyol` (parser ve kart). Ağ testi yoktur.
