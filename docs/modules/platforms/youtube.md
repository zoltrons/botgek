# YouTube

## Görev

YouTube **topluluk gönderilerini** karta çevirir: kanal, metin, beğeni sayısı, görseller, paylaşılan video ve
anket. Video bağlantıları bu modülün konusu değildir.

## Dosyalar

`builtin/youtube.rs`.

## Girdi ve çıktı

`youtube::fetch(client, &ParsedPost) -> Result<BuiltinPost, String>`.

## Nasıl çalışır

- Gönderi sayfası `hl=en` ile istenir. Sunucu tarafında gömülü `ytInitialData` nesnesinden
  `backstagePostRenderer` bulunur (`initial_data`, `find`). Oturum gerekmez.
- Kanal adı ve kullanıcı adı, bağlantıları korunmuş metin (`content_markdown`), tam beğeni sayısı
  (`like_count`), görsel ya da çoklu görsel (en fazla 10, genişlik 2048), paylaşılan video ve anket
  seçenekleri karta taşınır (`attachment`).
- YouTube yalnızca göreli tarih verir (`4 days ago`). Kartın zamanı bundan türetilir ve yaklaşıktır
  (`relative_time`).
- Gönderi okunamazsa (üyelere özel, kaldırılmış) genel önizlemeye düşülür (`preview::fetch`).

## Değişmezler

- Metin ve bağlantılar kaçışlanır (`escape_run`).
- Zaman her zaman yaklaşık kabul edilir, kesin tarih varsayılmaz.

## Değiştirirken

YouTube sayfa yapısı değişirse `initial_data` ve `find` yolları ile fixture'ı güncelleyin.

## Testler

`cargo test --locked youtube`. Beğeni ve göreli zaman geri dönüşleri için testler vardır.
