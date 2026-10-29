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
- Gömme türleri: görseller, video, harici bağlantı kartı (metin boşsa kart metin ve kapakla doldurulur),
  alıntı kaydı (`quote_from_embed_record`, `quote_from_post_view`).
- Yetişkin içerik etiketi (`has_adult_label`) `sensitive` işaretler.
- Repost edilmişse karta `Repost • {kullanıcı}` etiketi eklenir. Cevap ilişkisi iplikteki üst gönderiden
  okunur.
- Sayılar `Stats` içine: yanıt, repost, beğeni, alıntı.

## Değişmezler

- Kimliksiz herkese açık uçlar kullanılır, oturum gerekmez.
- Alıntı zinciri tek seviyedir.
- Facet işleme UTF-8 sınırlarına saygı gösterir.

## Değiştirirken

Yeni gömme türü: `embed_to_parts` ve `EmbedView`. Yeni etiket kuralı: `has_adult_label`.

## Testler

`cargo test --locked bluesky`. İplik fixture'ı ve facet testleri vardır.
