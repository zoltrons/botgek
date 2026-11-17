# Bluesky

## Görev

Bluesky (AT Protocol) gönderilerini, bağlantı ve görsel/video gömmelerini, alıntıyı ve cevap bağlamını
`BuiltinPost`'a çevirir.

## Dosyalar

`builtin/bluesky.rs`.

## Girdi ve çıktı

`bluesky::fetch(client, &ParsedPost) -> Result<BuiltinPost, String>`.

## Nasıl çalışır

- Resmî AppView'ın herkese açık XRPC uçları kullanılır (`public.api.bsky.app/xrpc`). Önce handle DID'e
  çözülür, sonra `getPostThread` ile gönderi çekilir.
- İplik sınırlıdır: derinlik 2 (`THREAD_DEPTH`), üst gönderi yüksekliği 1 (`THREAD_PARENT_HEIGHT`).
- Metindeki bağlantılar `facets` ile işaretlenir (`apply_link_facets`). Çakışan ya da çok baytlı karakterli
  facet'ler panik çıkarmaz.
- Gömme türleri: görseller (`images` ve `gallery#view` `items`), video, harici bağlantı kartı
  (`link_preview`: başlık, açıklama, site, küçük görsel), alıntı kaydı (`quote_from_embed_record`,
  `quote_from_post_view`), gönderi olmayan kayıtlar (başlangıç paketi, akış, liste: `graph_record_card`,
  bağlantı kartı olur).
- Video: API yalnızca HLS (`.m3u8`) verir ve alanlar gömmenin kendisinde durur (iç içe `video` nesnesi
  yoktur). Discord HLS oynatamadığı için oynatma listesindeki DID ve CID'den özgün MP4'ün blob adresi kurulur
  (`video_blob_url`, xsky.app/FxBluesky yöntemi): `bsky.social/xrpc/com.atproto.sync.getBlob`, kullanıcının
  PDS'ine yönlendirir ve `video/mp4` döner.
- Yanıtsa üst gönderi `reply_to`'ya konur (alıntı yuvasını işgal etmez); kart onu iplik bağlamı olarak çizer.
- Yalnızca yazarın kendi gönderisine yanıt (thread'in devamı) `thread_lookup` işaretlenir; yanıt olmayan
  gönderi (thread'in ilki dahil) hiç düzenlenmez. Kart gönderildikten sonra `enrich`,
  `app.bsky.unspecced.getPostThreadV2` yanıtındaki `opThread` işaretlerinden konumu (`🧵 4/5`) çıkarır ve
  kart güncellenir. İstek başarısızsa işaret açık kalır, sonraki paylaşım yeniden dener.
- Yetişkin içerik etiketi (`has_adult_label`) `sensitive` işaretler.
- Repost edilmişse karta `Repost • {kullanıcı}` etiketi eklenir. Cevap ilişkisi iplikteki üst gönderiden
  okunur; üst gönderi gelmediyse alt bilgide `Yanıt` etiketi kalır.
- Sayılar `Stats` içine: yanıt, repost, beğeni, alıntı. `replyCount` yoksa yanıt sayısı iplikte dönen
  yanıtlardan sayılır; alt bilgiye ayrıca `+N yanıt` yazılmaz.
- Görsel ve videonun `alt` metni `Media::alt`'a taşınır.

## Değişmezler

- Kimliksiz herkese açık uçlar kullanılır, oturum gerekmez.
- Alıntı zinciri tek seviyedir.
- Facet işleme UTF-8 sınırlarına saygı gösterir.

## Değiştirirken

Yeni gömme türü: `embed_to_parts` ve `EmbedView`. Yeni etiket kuralı: `has_adult_label`.

## Testler

`cargo test --locked bluesky`. İplik fixture'ı ve facet testleri vardır.
