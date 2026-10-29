# Reddit

## Görev

Reddit gönderisini (başlık, metin, skor, yorum sayısı, galeri ve video, crosspost) ve tek yorum
bağlantılarının bağlam metnini `BuiltinPost`'a çevirir. Videonun görüntü ve sesini tek MP4'te birleştirir.

## Dosyalar

| Dosya | İçerik |
| --- | --- |
| `builtin/reddit.rs` | OAuth belirteci, `by_id` çekimi, gönderi ve medya dönüşümü, RedGifs, bağlantı önizlemesi. |
| `builtin/reddit_gql.rs` | Android GraphQL `PostComments` işleminden yorum bağlamı. |
| `builtin/reddit_video.rs` | `v.redd.it` video ve ses akışlarını saf Rust ile sesli MP4'e birleştirme. |

## Girdi ve çıktı

- `reddit::fetch(client, &ParsedPost, client_id) -> Result<BuiltinPost, String>`
- `reddit_video::merge(client, video_url, max_bytes) -> Option<MediaFile>`

## Nasıl çalışır

### Veri

- Reddit kimliksiz `.json` isteklerini 403 ile engeller. Bot `installed_client` OAuth akışını kullanır:
  kullanıcı izni gerekmez, herkese açık istemci kimliği yeterlidir. Kimlik `REDDIT_CLIENT_ID`,
  varsayılanı `config::DEFAULT_REDDIT_CLIENT_ID`.
- Belirteç bellekte tutulur ve süresi dolmadan 120 sn önce yenilenir. Alınamazsa 60 sn geri çekilme uygulanır.
  Geçersiz belirteç `invalidate_token` ile atılır.
- Gönderi `oauth.reddit.com/by_id/t3_{id}.json` ile alınır. `/comments/` yolu anonim istemciye kapalıdır.
- Gönderi kimliği URL'den çıkarılır (`/comments/{id}` ve `redd.it/{id}`). Mobil paylaşım linki
  (`/r/{sub}/s/{kod}`) tarayıcı User-Agent'ıyla istenir, varılan kalıcı adresten kimlik okunur
  (`extract_post_id`).
- Yorum bağlantısında (`.../comment/{id}`) yorumun metni `reddit_gql` ile alınır ve gönderiye alıntı
  satırlarıyla eklenir, bağlam en fazla 1200 karakter.
- Medya: tek görsel, galeri, GIF, Reddit videosu (`fallback_url`, `secure_media`), RedGifs. Yalnızca izinli
  RedGifs varlık adresleri kabul edilir (`allowed_redgifs_asset`).
- `source_spoiler` Reddit'in `spoiler` bayrağından, `sensitive` NSFW'den gelir. `extra_label` flair ve
  benzerini taşır. Skor `Stats.likes` alanındadır.
- Dış bağlantı paylaşımında hedef sayfanın önizlemesi (500 ms) eklenir.

### Video birleştirme (`reddit_video.rs`)

Reddit video ve sesi ayrı akışta tutar (`CMAF_{h}` video, `CMAF_AUDIO_*` ses). Birleştirme yeniden kodlama
olmadan, saf Rust `hls-transmux` ile yapılır:

1. Master playlist çözülür, çözünürlük merdiveni çıkarılır (`variant_ladder`), boyut tahmini yapılır.
2. Video ve ses medya playlist'leri iki girdi olarak tek oturuma verilir, çıktı tek MP4 olur.
3. Parça başına 16 MiB üst sınır, bağlantı zaman aşımı 2 sn. Ayrı bir işçi görevi ve eşzamanlılık sınırı (2)
   vardır. Çağıran taraftan iptal sinyali gelir (`cancellable_merge`).
4. Sığmazsa ya da hata olursa `None` döner. Çağıran taraf kartta sessiz uzak videoya düşer.

Birleştirme yalnızca video ek olarak gönderilecekken çağrılır (`render::resolve_media`, süre bütçesi 5 sn,
bkz. [rendering.md](../rendering.md)).

## Değişmezler

- Reddit videosu `video_requires_download` listesindedir.
- Birleştirme başarısızsa kart bozulmaz, sessiz uzak video korunur.
- GraphQL kalıcı sorgu karması (`POST_COMMENTS_HASH`) Reddit tarafında değişirse yorum bağlamı sessizce
  boş döner. Bu durumda gönderi kartı yine de üretilir.

## Değiştirirken

- Yeni medya türü: `single_media` ve `reddit_media`.
- Yeni video kalitesi kuralı: `variant_ladder` ve `estimated_bytes`.
- Yorum bağlamı sınırı: `MAX_COMMENT_CONTEXT_CHARS`, `COMMENT_COUNT`, `MAX_DEPTH`.

## Testler

```sh
cargo test --locked reddit
cargo test --locked reddit_video
cargo test --locked reddit_gql
```
