# Spotify

## Görev

Spotify parça, albüm, çalma listesi, sanatçı, bölüm ve program bağlantılarını Spotify yeşili, butonlu bir
karta çevirir. 30 saniyelik önizlemeyi kapak görseliyle sesli MP4'e çevirip kartta oynatır.

## Dosyalar

| Dosya | İçerik |
| --- | --- |
| `builtin/spotify.rs` | Veri çekme, hedef çözümleme, `CustomCard` bileşenleri, parça listesi sayfalaması. |
| `builtin/spotify_video.rs` | MP3 önizleme ve kapak görselinden MP4 üretimi. |
| `commands/mod.rs` (`spotify_page`) | Sayfalama butonlarının etkileşim işleyicisi. |

## Girdi ve çıktı

- `spotify::fetch(client, &ParsedPost) -> Result<BuiltinPost, String>`: `custom_card` doludur.
- `spotify_video`: önizleme MP3'ünü ve kapağı indirir, `spotify-preview.mp4` üretir.

## Nasıl çalışır

### Veri

- `open.spotify.com/embed/{tür}/{id}` HTML'indeki `__NEXT_DATA__` JSON'u okunur: ad, sanatçılar, 30 sn
  `audioPreview.url`, albüm görseli, çıkış tarihi. API anahtarı gerekmez.
- `spotify.link` kısa bağlantıları hedefe çözülür (`resolve_target`).
- Zaman aşımı 8 sn.

### Kart

`CustomCard`: `accent_color` Spotify yeşili (`0x1DB954`), `Section` ve `Thumbnail`, `Separator`, Spotify / Apple
Music / YouTube Music butonları bir `Action Row` içinde. Parça listesi olan türlerde (albüm, çalma listesi)
liste sayfa başına 10 parçadır (`TRACKS_PER_PAGE`), başlık 60 karakterde kesilir. Sayfalama bileşen
kimlikleri sabittir (`TRACK_LIST_COMPONENT_ID` 7301, `PAGER_COMPONENT_ID` 7302); sayfa göstergesi
`spotify:noop` özel kimliğini taşır. Video varsa kapak videoda göründüğü için `video_components`
kullanılır (küçük görselsiz, kompakt başlık).

### Önizleme videosu (`spotify_video.rs`)

ffmpeg yerine saf Rust zinciri:

1. Kapak JPEG'i `zune-jpeg` ile çözülür, BT.601 YUV 4:2:0'a çevrilir. En fazla 1 megapiksel, 2 MiB.
2. Görüntü `less-avc` ile tek kayıpsız I-kare (I_PCM, Baseline) olarak kodlanır. Karenin süresi sesin
   tamamıdır.
3. Ses `symphonia` ile çözülür ve `rusty_aac` ile AAC-LC'ye (128 kbps) kodlanır. MP4 içinde MP3 iOS Discord'da
   sessiz kaldığı için AAC seçilmiştir. En fazla 60 sn, 2 MiB.
4. `shiguredo_mp4` ile `moov` başta (faststart) MP4 üretilir.

Üretim yalnızca kart gönderilirken (`render::resolve_media`) ve video bütçesi içinde yapılır. Başarısızsa
kart videosuz, kapak küçük görseliyle gönderilir. Spotify `video_requires_download` listesindedir.

## Değişmezler

- Spotify kartı uzak video bağlantısı kullanmaz. Üretilemezse video yoktur.
- Parça listesi sayfalaması bileşen kimliklerine bağlıdır. Kimlikleri değiştirirken `commands` içindeki
  işleyiciyi de güncelleyin.
- Önizleme dosyaları ve çözülen görüntü boyutları sınırlıdır (bellek).

## Değiştirirken

Yeni Spotify türü: `Target` çözümü ve veri ayrıştırma. Buton eklemek: `CustomCard` bileşenleri ve emoji
sabitleri (`SPOTIFY_EMOJI` vb.).

## Testler

`cargo test --locked spotify`. `spotify_video` testleri `tests/fixtures/sine-440hz-0.5s.mp3` dosyasını kullanır.
