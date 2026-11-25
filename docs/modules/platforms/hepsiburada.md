# Hepsiburada

## Görev

Hepsiburada ürün bağlantılarını (`hepsiburada.com` / `m.hepsiburada.com` `/{ad}-p-{SKU}` ve `/{ad}-pm-{SKU}`)
ortak mağaza kartına ([shop.md](shop.md)) çevirir.

## Dosyalar

`builtin/hepsiburada.rs`; kart `builtin/shop.rs`.

## Girdi ve çıktı

`hepsiburada::fetch(client, &ParsedPost) -> Result<BuiltinPost, String>`: `CustomCard` (v1 embed), medya yok.

## Nasıl çalışır

- Parser sorgu parametrelerini atar; ad küçük harfe, SKU büyük harfe çevrilir (`HBCV0000FFASMJ`).
- Sayfa ortak `get_markdown` ile datacat vekilinden Markdown olarak alınır (vekil arada 403 döndürebilir;
  hata 30 sn önbelleğe girer, özgün mesaj kalır). Alanlar satır düzeninden okunur:
  - Başlık `# [Marka](/marka) Ürün adı`; başlık yoksa kart çıkmaz.
  - Puan başlıktan sonraki ilk satır, değerlendirme `**N** Değerlendirme`, satıcı ilk `Satıcı: [ad](/magaza/…)`.
  - Fiyat: başlıktan "Ek hizmet ister misin?" / "Sepete ekle"ye kadar yalnız tutardan oluşan ilk satır;
    hemen altındaki daha yüksek tutar eski fiyattır (sonraki "3.920 TL" kazanç tutarıdır). İndirim bu ikisinden.
  - Teslimat: "Teslimat Seçenekleri"nden sonraki "…kargoda" satırı ("Bugün kargoda", "Tahmini … günü
    kargoda"); geri sayım satırı atlanır. "300 TL üzeri kargo bedava" koşullu olduğu için ücretsiz sayılmaz.
  - Künye: seçili varyantlar ("SSD Kapasitesi: 512 GB"; parantez içi atılır) ve "Ürün Bilgileri"
    çiftleri; "Yok" değerleri ve uzunlar atlanır, en çok 6.
  - Görsel: galerinin ilk `productimages.hepsiburada.net/s/N/{boyut}/ID.jpg` adresi, boyut `1500` yapılır.
- Renk `#FF6000`, "Hepsiburada'da aç" butonu. İade süresi ve stok sayfada okunmaz.

## Değiştirirken

Seçiciler datacat'in Markdown çıktısına bağlıdır. Yeni kural önce gerçek bir sayfanın datacat çıktısında
denenip testteki `SAMPLE` güncellenir. Kısa paylaşım linkleri desteklenmez (örnek yok).

## Testler

`cargo test --locked hepsiburada` (parser ve sayfa okuma). Ağ testi yoktur.
