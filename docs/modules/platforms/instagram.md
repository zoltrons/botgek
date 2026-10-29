# Instagram

## Görev

Instagram gönderi, reel ve IGTV bağlantılarından yazar, açıklama, sayılar ve medyayı çıkarır. Oturum çerezi
tanımlıysa video ve çoklu görseller de alınır.

## Dosyalar

`builtin/instagram.rs`.

## Girdi ve çıktı

`instagram::fetch(client, &ParsedPost, sessionid: Option<&str>) -> Result<BuiltinPost, String>`.

## Nasıl çalışır

Üç uç **paralel** istenir (`tokio::join!`), toplam gecikme tek isteğin gecikmesi kadardır:

| Uç | Verdiği |
| --- | --- |
| Embed sayfası `/{p\|reel\|tv}/{kod}/embed/captioned/` | Kullanıcı adı, doğrulama, avatar, açıklama, beğeni ve yorum sayısı. Tarayıcı olmayan User-Agent'a sunucu tarafında verilir. |
| Gönderi sayfası `/{tür}/{kod}/` | `og:title` (görünen ad), `og:url` (kullanıcı adı), `og:description` (sayılar, tarih), `og:image`. OG çözümleyici `opengraph.rs`. |
| Oturumlu uygulama API'si `/api/v1/media/{pk}/info/` | Yalnızca `INSTAGRAM_SESSIONID` varsa: video mp4 ve carousel görselleri. |

- Kısa kod, `shortcode_to_pk` ile sayısal kimliğe çevrilir (base64 benzeri alfabe).
- Oturumlu istek iki tabana sırayla denenir (`SESSION_API_BASES`), toplam 8 sn, yanıt 1 MiB ile sınırlı.
- Üç kaynak da yararsızsa (ör. giriş duvarı) hata döner. OG verisi yalnızca görsel ya da açıklama taşıyorsa
  yararlı sayılır (`graph_is_useful`).
- Çerez `%3A` ile kodlanmış olabilir, ham ve çözülmüş biçim sırayla denenir.
- Oturum yoksa videoda yalnızca kapak karesi gösterilir ve bağlantıya `Reel` ya da `Video` etiketi eşlik
  eder, çünkü Instagram mp4'ü sayfa kaynağına koymaz.

## Değişmezler

- `INSTAGRAM_SESSIONID` yalnızca `.env`'den okunur. `settings.json`, panel, MCP ve loglara girmez.
- Oturum çerezi olmadan çalışma yolu bozulmamalı. Oturum verisi yalnızca zenginleştirir.
- Üç kaynağın birleşimi `build_post` içinde yapılır. Bir kaynağın eksik olması diğerlerini engellemez.

## Değiştirirken

- Embed HTML ayrıştırması kırılgandır: `parse_embed` ve regex'leri gerçek HTML örnekleriyle test edin.
- Yeni bir Instagram URL türü için önce `parser.rs`, sonra `PostKind::from_path`.

## Testler

`cargo test --locked instagram`. Oturumlu canlı test özel çerez ister ve `#[ignore]` işaretlidir
(`live_` önekli, bkz. [DEVELOPMENT.md](../../DEVELOPMENT.md)).
