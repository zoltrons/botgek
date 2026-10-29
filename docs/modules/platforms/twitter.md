# X (Twitter)

## Görev

X gönderisini, ilk seviye alıntısını, medyasını, metriklerini ve topluluk notunu `BuiltinPost`'a çevirir.
İstenirse gönderiyi X'in kendi çevirisiyle hedef dile çevirir.

## Dosyalar

| Dosya | İçerik |
| --- | --- |
| `builtin/twitter.rs` | Kaynaklar, paralel çekim, birleştirme, bağlantı önizlemesi, GIF bağlantıları. |
| `builtin/translate.rs` | Yalnızca "hangi gönderi çevrilir" kararı ve Türkçe çeviri notu. Ağ isteği yoktur. |
| `builtin/twitter/benchmarks.rs` | Canlı süre ölçümü (`#[ignore]`). |

## Girdi ve çıktı

- `twitter::fetch(client, &ParsedPost, &EngineSettings) -> Result<Fetched, String>`
- `Fetched { post: BuiltinPost, cacheable: bool }`. `cacheable` eksik sonuçta `false`'tur.
- Yardımcılar: `find_tweet_url(text)`, `gif_links(client, url)` (`/gif-yap` için).

## Nasıl çalışır

Kaynaklar:

| Kaynak | Uç | Rol |
| --- | --- | --- |
| FxTwitter | `api.fxtwitter.com` | Ana kaynak. Tam metin, medya, alıntı, görüntülenme dahil metrikler, topluluk notu. |
| Syndication | `cdn.syndication.twimg.com/tweet-result` | X'in kendi ucu. Yedek. Görüntülenme ve çeviri yok, uzun metin kesik. |
| vxTwitter | `api.vxtwitter.com` | Yalnızca `/dönüştür kaynak:vxTwitter (yedek)` ile elle seçilir. |

Akış (`acquire`, `first_data`):

1. FxTwitter ve syndication baştan **paralel** istenir.
2. Çeviri açıksa dil ekli FxTwitter ucu (`/tr` gibi) düz yanıtla paralel istenir.
3. FxTwitter `PRIMARY_PATIENCE` (800 ms) içinde yanıt vermez ya da hata verirse syndication verisi kullanılır
   ve sonuç önbelleğe yazılmaz.
4. Yabancı dilli gönderide çeviri için `TRANSLATION_BUDGET` (1,2 sn) beklenir. Dolarsa kart orijinal metinle
   gider. İngilizce ve Türkçe gönderiler varsayılan akışta çevrilmez, `/dönüştür` İngilizceyi de çevirir.
5. İlk veri 3 sn içinde gelmezse (`FETCH_FIRST_DATA_BUDGET`) vazgeçilir ve orijinal mesaj korunur.
6. Gönderide medya yoksa ve dış bir bağlantı varsa bağlantı önizlemesi eklenir. Süre `LINK_PREVIEW_TIMEOUT`
   (500 ms), başlık 180, açıklama 400 karakter. İstek `safe_http` üzerinden gider
   ([preview.md](../preview.md)).

Alıntı yalnızca ilk seviyedir, alıntının alıntısı çekilmez. Sondaki `t.co` / `pic.x.com` medya bağlantıları
metinden temizlenir (`strip_trailing_media_links`).

### Çeviri kararı (`translate.rs`)

Belirsiz diller (`und`, `zxx`, `mul`, ...) bilinmeyen sayılır. Hedef dil Türkçe ya da İngilizceyse o dildeki
gönderi olduğu gibi kalır. Karta Türkçe, ünlü uyumuna uygun bir çeviri notu eklenir ("İngilizceden çevrildi"
gibi, dil adı için tablo vardır).

## Değişmezler

- X verisi FxTwitter'dan gelir. `settings::TwitterSource` eski ve etkisizdir.
- Eksik sonuç önbelleğe yazılmaz: sonraki paylaşım ana kaynağı yeniden dener.
- Alıntı zinciri tek seviyedir.
- Çeviri yalnızca X'in kendi çevirisidir (sağlayıcı alanı ör. `grok`).

## Değiştirirken

- Yeni bütçe: sabiti `twitter.rs` başında tutun ve [ARCHITECTURE.md](../../ARCHITECTURE.md) zaman bütçeleri
  tablosunu güncelleyin.
- Yeni dil adı: `TURKISH_LANGUAGE_NAMES` tablosuna ekleyin, ünlü uyumu testini genişletin.
- vxTwitter yolu: `from_vxtwitter`, `vx_to_post`, `vx_media`.

## Testler

```sh
cargo test --locked twitter
cargo test --locked translate
cargo test --locked twitter::benchmarks -- --ignored --nocapture
```
