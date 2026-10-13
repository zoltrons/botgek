# EG-05 — Platform bağlamı ve zengin medya parity'si

**Öncelik:** P1  
**Hedef:** Mevcut platformları Embedded'in öne çıkardığı bağlam seviyesine yaklaştırmak: Reddit direct
comment + Redgifs sound, Bluesky reply/repost context ve TikTok zengin metadata/video.

## Mevcut durum

### Reddit

Parser post URL'lerini ve share/short linklerini tanıyor; comment ID'li bir hedef modeli yok. Reddit
adapter post title/selftext/score/comments/gallery/video üretir. `reddit_video.rs` sesli MP4 birleştirme
akışına sahip olsa da Redgifs host'u için ayrı resolver yok.

### Bluesky

`getPostThread?depth=0` çağrısı ana postu alıyor. Quote embed'i parse ediliyor; ancak reply chain,
repost marker ve original author attribution için model/renderer alanı yok.

### TikTok

Native adapter TikTok oEmbed'den title, author ve thumbnail alıyor. Description/hashtags, follower,
likes/comments/shares/views ve doğrudan video medya alanları oEmbed'de olmadığı için kartta yok.
Video için mevcut default proxy davranışı bir fallback'tir; native feature parity değildir.

## Ortak model önerisi

`BuiltinPost`'a gereksiz platform alanları eklemek yerine:

```text
PlatformContext =
  Reddit { subreddit, flair?, comment_path?, parent_comment? }
  Bluesky { relation: reply|repost|quote, thread_depth, original_author? }
  TikTok { hashtags[], follower_count?, share_count? }
```

Büyük thread'leri sınırlamak için `context_depth`, `max_items` ve toplam text budget zorunludur.

## Uygulama planı

1. Parser'a Reddit comment URL formlarını ekleyin: `/comments/post/comment`, `/r/sub/comments/post/title/comment`;
   spoiler ve query stripping kurallarını koruyun.
2. Reddit'te comment endpoint/listing'i güvenli şekilde fetch edin. Hedef comment + parent chain + post
   summary üretin. Deleted/removed comment'ı boş metin olarak değil, explicit unavailable durumu olarak
   modelleyin.
3. Redgifs URL'lerini yalnızca allowlisted host/scheme/port ile çözün. Poster/video/audio metadata
   bulunamazsa canonical Redgifs link + poster fallback'i kullanın; arbitrary redirect fetch etmeyin.
4. Bluesky request depth'ini ayarlanabilir bounded değere çıkarın. `reply` ve `repost` kayıtlarını
   canonical URL, author, text, media ile sınırlı zincir olarak normalize edin. Cycle/depth/body limitleri
   ekleyin.
5. TikTok için seçilmiş güvenilir endpoint oEmbed dışında veri veriyorsa adapter'a ekleyin; sözleşme
   değişkenliği nedeniyle tüm alanları optional tutun. Video direct URL alınamazsa poster + canonical
   link kartı korunmalı.
6. `v2.rs` içinde relation/context component'ini generic, maksimum child/text limitlerine tabi üretin.
   Context medyası ana medya galerisini taşırmamalı; ana post media önceliklidir.
7. Settings'e `context_depth`, `show_platform_context`, `show_metrics` gibi alanları ekleyip EG-02
   çözümlemesine bağlayın; default mevcut kart görünümünü korusun.

## Kabul kriterleri

- Reddit direct comment linki post + hedef comment + sınırlı parent context ile açılır; normal post kartı
  geriye dönük aynı görünür.
- Redgifs video/ses bulunduğunda kartta oynar veya güvenli canonical link fallback'i gösterir; işlem
  hiçbir zaman arbitrary host'a yönelmez.
- Bluesky reply/repost karta relation ve original author bilgisiyle gelir; thread sonsuz büyümez.
- TikTok adapter mevcut title/thumbnail kartlarını bozmaz; richer fixture alanları varsa gösterir.
- Context fetch timeout olduğunda ana post yine gönderilir.

## Test matrisi

Reddit URL/parser variants; target comment deleted/NSFW/long/HTML; Redgifs host/redirect/403/timeout;
Bluesky depth 0/1/3, quote+reply, repost, cycle; TikTok minimal/rich/missing JSON; V2 text/component
limit; source URL cache key; no-network fixture tests; live tests ignored.
