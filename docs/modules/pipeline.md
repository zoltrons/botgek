# Dönüştürme hattı (`converters/builtin`)

## Görev

Bir `ParsedPost`'u alır, platform modülünden veri çeker, medyayı hazırlar, Components V2 kartını kurar ve
Discord'a gönderir. Platformdan bağımsız ortak mantık burada durur.

## Dosyalar

| Dosya | İçerik |
| --- | --- |
| `builtin/mod.rs` | `fetch`, `handle`, ortak HTTP yardımcıları, sabitler, taklit webhook'u, kart tanıma. |
| `builtin/model.rs` | Platformlar arası veri tipleri. G/Ç, ayar ya da Discord istemcisi içermez. |
| `builtin/cache.rs` | Gönderi önbelleği ve hata önbelleği. |
| `builtin/coordination.rs` | Anahtar başına tekil uçuş kilidi (single-flight). |
| `builtin/transport.rs` | Discord gönderim taşıma katmanı: sınırlı yanıt, rota ve global bekleme, kör POST yeniden denemesi yok. |
| `builtin/raw.rs` | Ham Discord REST: kanal mesajı, webhook, etkileşim yanıtı, multipart gövde. |

## Girdi ve çıktı

- `fetch(client, &ParsedPost, &EngineSettings, &Config) -> Result<Arc<BuiltinPost>, String>`
- `handle(...) -> HandleResult`: `Sent`, `Failed` (orijinal kalır), `Busy` (kapasite dolu) ya da `Ambiguous`
  (gönderim sonucu belirsiz, yeniden denenmez).

## Nasıl çalışır

### Veri modeli (`model.rs`)

`BuiltinPost` platformdan bağımsızdır: `platform`, `url`, `author` (`Author`), `title`, `text`,
`created_at` (Unix sn), `stats` (`Stats`: replies, reposts, likes, quotes, views), `media` (`Media` listesi),
`quote` (tek seviye), `community_note`, `source_spoiler`, `sensitive`, `extra_label`, `language`,
`translation`, `custom_card` (hazır V2 bileşenleri, Spotify, Twitch, Letterboxd), `link_preview`.

`Media` türü `Image`, `Video` ya da `Gif` olabilir. Reddit skoru `likes` alanında taşınır.
`source_spoiler` kaynağın açıkça işaretlediği spoiler'dır ve NSFW kanalda bile otomatik açılmaz.

### `fetch`

1. Anahtar `post_cache_key(parsed, engine)`: kaynak, çeviri, alıntı ve video bütçesi dahil.
2. `coordination::lock(anahtar)`: aynı anahtara eşzamanlı istekler tek çekimde birleşir.
3. `cache::get` isabet ederse ağ isteği yapılmaz (`CACHE_HITS`).
4. `cache::recent_failure` 30 sn içinde aynı anahtar başarısız olduysa aynı hatayı döner.
5. Platform dağıtımı: `twitter::fetch`, `bluesky::fetch`, `reddit::fetch`, ... `Platform` varyantına göre.
6. Başarısızlık hata önbelleğine yazılır. Başarı `cache::put` ile 600 sn / 512 girdi sınırıyla saklanır.
   X, eksik sonucu (çeviri beklenip gelmediyse ya da yedek kaynak kullanıldıysa) önbelleğe yazdırmaz
   (`Fetched::cacheable`).

NSFW ve spoiler kararı kaynak önbelleğine yazılmaz. Aynı gönderi NSFW olan ve olmayan kanalda farklı
gösterilebildiği için `Arc` değiştirilmez, görünürlük her gönderimde ayrıca uygulanır.

### `handle`

Sıra, gerçek kodda ayrıntılı yorumlanmıştır. Özet:

1. Taklit kimliği (`Impersonation`, webhook çözümü) ve kanalın NSFW durumu veri çekmeyle **paralel** çözülür.
2. Veri gelir. Medya planı (`render::plan_media`) ve indirmeler bütçeyle yürür (`render::resolve_media`).
3. `v2::build_payload` Components V2 gövdesini kurar.
4. Gönderim: taklit açıksa kullanıcının adı ve avatarıyla webhook (`execute_webhook`), değilse kanal mesajı
   (`send_channel_message`), etkileşimde etkileşim yanıtı (`InteractionDelivery`). Webhook belirsizse ya da
   token eskiyse bir kez yenilenir.
5. Başarıdan sonra `finish()`: `DELETE_ORIGINAL` açıksa orijinal mesaj silinir, `store` kaydı yazılır,
   telemetri güncellenir.

Orijinal mesaj gönderimden **önce** silinmez. Erken silme, göndericinin istemcisinde "hayalet mesaj"
bırakıyordu.

### Zaman bütçeleri

Sabitler `builtin/mod.rs` başında:

| Sabit | Değer | Anlamı |
| --- | --- | --- |
| `LATENCY_TARGET` | 1 sn | Kart hedefi |
| `FETCH_FIRST_DATA_BUDGET` | 3 sn | Hiç veri gelmezse vazgeçme |
| `REQUEST_TIMEOUT` | 6 sn | Genel upstream isteği |
| `MEDIA_TIMEOUT` | 12 sn | Medya indirme isteği |
| `MAX_BODY_BYTES` | 2 MiB | Metin ve JSON yanıt sınırı |

Veri gecikmesi gönderime ayrılan payı yememelidir; yerse nedeni logda görünür (kaynak mı yavaş, medya mı).

### Taşıma (`transport.rs`, `raw.rs`)

- Yanıt gövdesi sınırlıdır. Hata gövdesi loga en fazla 600 bayt girer.
- Rota ve global 429 beklemeleri uygulanır. Sunucunun verdiği bekleme süresi **kısaltılmaz**,
  sonlu olmayan değerler reddedilir.
- Hata sınıfları: `Rejected(durum)`, `RateLimited`, `Ambiguous`, `InvalidPayload`, `Unavailable`.
  `safe_fallback` yalnızca kesin ret ya da geçersiz yükte `true`'dur. `Ambiguous` (yanıtsız POST) yeniden
  denenmez, çünkü mükerrer mesaj oluşturabilir.
- `expired_webhook` 401 ve 404 için doğrudur, webhook yenilemeyi tetikler.
- Components V2 serenity builder'larıyla üretilemediğinden gövde `serde_json` ile kurulur. Dosya yükleme
  için `multipart/form-data` elle oluşturulur, dosya adı kaçışlanır, sınır dizesi benzersizdir.

### Kart tanıma

`is_card_message(msg, bot_id)` ve `has_card_signature`: V2 bayrağı ve botun kimliği ya da kalıcı kayıt
(`store`). Kaydı olmayan rastgele webhook mesajları kart sayılmaz.

## Değişmezler

- Orijinal mesaj yalnızca başarılı gönderimden sonra silinir. Belirsiz sonuç yeniden denenmez.
- 429 bekleme süresi kısaltılmaz.
- Kapasite dolunca orijinal mesaj korunur.
- Paylaşılan `Arc<BuiltinPost>` değiştirilmez. Görünürlük her gönderimde ayrı hesaplanır.
- Platform modülü `BuiltinPost` döner, başka bir şey bilmez. Discord ve ayar bağımlılığı eklemeyin.
- Hata metinleri kullanıcıya ya da loga sır taşımaz.

## Değiştirirken

- Yeni platform: [converters.md](converters.md) değişmezler listesi, `fetch` dağıtımı ve
  [platforms/](platforms/) altında yeni belge.
- Yeni önbellek anahtarı bileşeni: `post_cache_key` içine ekleyin. Aksi halde farklı ayarlar aynı kartı
  paylaşır.
- Yeni bütçe sabiti: yukarıdaki tabloyu ve [ARCHITECTURE.md](../ARCHITECTURE.md) tablosunu güncelleyin.

## Testler

```sh
cargo test --locked builtin
cargo test --locked transport
cargo test --locked raw
```
