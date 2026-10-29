# Twitch

## Görev

Twitch kliplerini başlık, yayıncı, oyun, izlenme, süre, klipleyen ve VOD anıyla bir karta çevirir ve
videoyu kartın içinde oynatır.

## Dosyalar

`builtin/twitch.rs`.

## Girdi ve çıktı

`twitch::fetch(client, &ParsedPost) -> Result<BuiltinPost, String>`: `custom_card` doludur.

## Nasıl çalışır

- Resmî Helix API klip videosunun adresini vermez. Sitenin kendi herkese açık istemci kimliğiyle
  `gql.twitch.tv/gql` adresine tek sorgu atılır (`CLIP_QUERY`). Sorgu klip bilgisini ve imzalı MP4 adresini
  (`sig` ve `token`, yaklaşık 24 saat geçerli) birlikte döner.
- En iyi kalite 720p ile sınırlıdır (`MAX_QUALITY`).
- Kart Twitch moru (`0x9146FF`) vurgulu bir `CustomCard`'dır: bölüm, küçük görsel, ayraç, link butonları
  (klip, kanal, varsa VOD anı).
- Video gönderimde dosya olarak eklenir ki imza süresi dolunca eski mesajlardaki oynatıcı bozulmasın.
  Discord sınırına ya da süre bütçesine sığmazsa imzalı adres kartta uzaktan oynar. Twitch
  `video_requires_download` listesindedir.
- Uç nokta belgelenmemiştir. Sorgu başarısız olursa sayfanın genel OG önizlemesine düşülür.

## Değişmezler

- Klip kimliği (`slug`) yalnızca URL'den çıkarılır (`clip_slug`).
- Link butonları yalnızca `https` adreslerle kurulur (`https` yardımcısı).
- İstemci kimliği sitenin herkese açık kimliğidir, sır değildir.

## Değiştirirken

Twitch GraphQL şeması değişirse `CLIP_QUERY` ve `Clip` tipleri birlikte güncellenir.

## Testler

`cargo test --locked twitch`.
