# Rust Geliştirme Kuralları

## Context7 Kullanımı (ZORUNLU)

Rust kodu yazmadan, değiştirmeden veya hata ayıklamadan önce Context7 MCP sunucusunu kullan:

1. Bir crate'in API'sini kullanacaksan (tokio, axum, serde, sqlx, clap, reqwest, tracing vb.)
   önce Context7'de crate'i bul (resolve library id), sonra ilgili dokümanı çek.
2. Hafızandaki API bilgisine güvenme. Crate API'leri sürümler arasında değişir;
   Context7'den gelen doküman ile hafızan çelişirse Context7'yi esas al.
3. Cargo.toml'daki sürümü kontrol et ve doküman sorgularında o sürümü hedefle.
   Sürüm belirtilmemişse en güncel kararlı sürümü varsay ve bunu belirt.
4. Derleme hatası "no method named...", "unresolved import", "trait bound not satisfied"
   gibi bir API uyumsuzluğuna işaret ediyorsa, tahmin yürütmeden önce
   Context7'den ilgili dokümanı tekrar çek.
5. Yeni bir crate eklemeden önce Context7'de var olup olmadığını ve
   güncel kullanım örneğini kontrol et.
6. Doc.rs'i aktif kullan context7'de.

## Doküman bulunamazsa
- Context7'de crate yoksa bunu açıkça söyle, sonra docs.rs üzerinden
  veya `cargo doc` / kaynak koddan doğrula. Uydurma API kullanma.

## Kod Yazım Sonrası Doğrulama
- Her değişiklikten sonra sırasıyla çalıştır:
  `cargo check` → `cargo clippy -- -D warnings` → `cargo fmt --check` → `cargo test`
- Hata çıkarsa düzelt ve tekrar çalıştır; derlenmeyen kodu "bitti" olarak bırakma.

## Genel Rust Tercihleri
- Kütüphane kodunda `unwrap()`/`expect()` yerine `Result` ve anlamlı hata türleri kullan
  (`thiserror`, uygulamalarda `anyhow`).
- Gereksiz `clone()` ve `unsafe` kullanma; kullanman gerekiyorsa nedenini yorumla açıkla.
- Public öğelere kısa `///` doküman yorumu ekle.