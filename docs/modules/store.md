# Depo (SQLite)

## Görev

Her kart için hangi mesajın hangi kullanıcı tarafından paylaşıldığını kalıcı tutar. İki işe yarar:
🗑️ tepkisiyle silme izni ve kartlara verilen yanıtlarda AI sohbetinin tetiklenmemesi.

## Dosyalar

`src/store.rs`.

## Nasıl çalışır

Tek tablo vardır:

```sql
cards (message_id INTEGER PRIMARY KEY, channel_id INTEGER, author_id INTEGER, created_at INTEGER,
       source_id INTEGER, source_url TEXT)   -- kaynak sütunları eski dosyalara açılışta eklenir
```

- Dosya yolu `BOTGEK_DB_PATH`, yoksa `botgek.db`. Docker'da `/app/data/botgek.db`.
- SQLite WAL modunda çalışır.
- Kayıt sayısı `MAX_RECORDS` (200.000) ile sınırlıdır. Her `PRUNE_EVERY` (512) yazımda `prune` sınırı aşan eski kayıtları siler.
- `Db::remember`, `forget`, `author`, `contains`, `count` ve `prune` senkrondur. Async koddan
  `store::blocking` ile çağrılır: tek izinli bir semafor ve `spawn_blocking` kullanır, SQLite işleri sırayla
  yürür. Modül düzeyindeki `remember_card`, `forget_card`, `card_author`, `is_card` kısayollarıdır.
- Üretimde `PersistencePolicy::RequirePersistent` geçerlidir: dosya açılamazsa başlangıç hata verir.
  Testler `:memory:` yolunu kullanır.

Dışa açık işlevler: `path`, `record_count`, `file_bytes`, `file_sizes`, `healthy`, `query_readonly`,
`query_readonly_blocking`.

### Salt okunur sorgu

MCP `db_query` için `query_readonly(sql, limit)`: yalnızca tek bir `SELECT` ya da `WITH` ifadesi, en fazla
200 satır, 2 sn sorgu zaman aşımı. Yazan ifade reddedilir.

## Değişmezler

- Kayıt yalnızca kart başarıyla gönderildikten sonra yazılır. Gönderim sonucu belirsizse yazılmaz.
- Depo erişimi ana akışı bloklamaz. Doğrudan `Db` çağrısı yerine `blocking` ya da `builtin::remember_author`
  gibi sarmalayıcıları kullanın.
- Tabloda mesaj içeriği yoktur. Yeni sütun eklerken kişisel veri saklamamaya dikkat edin.

## Değiştirirken

Şema değişikliği `connect` içindeki `CREATE TABLE IF NOT EXISTS` ile yapılır. Mevcut dosyalar için geçiş
gerekiyorsa `ALTER TABLE` adımı ekleyin ve eski dosyayla test edin.

## Testler

`cargo test --locked store`.
