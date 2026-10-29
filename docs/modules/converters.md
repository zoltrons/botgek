# Dönüştürücü girişi: kuyruk ve ayrıştırıcı

## Görev

Mesajdaki desteklenen bağlantıyı bulur, kimin, nerede, hangi ayarla dönüştürebileceğine karar verir ve işi
sınırlı bir kuyruğa koyar.

## Dosyalar

| Dosya | İçerik |
| --- | --- |
| `src/converters/parser.rs` | `Platform`, `ParsedPost`, `parse_message`: URL kalıpları ve kanonikleştirme. |
| `src/converters/mod.rs` | Kuyruk (`enqueue_message`), etkileşim dönüşümü (`convert_now`), `ConvertError`. |

## Girdi ve çıktı

- `parse_message(raw_content) -> Option<ParsedPost>`
- `ParsedPost { platform, is_spoiler, user_comment, original_url }`

`original_url` kanonik biçimdir (ör. `https://x.com/kullanici/status/123`). `user_comment` kullanıcının
linke eklediği nottur ve kartın dışında, üstünde düz metin olarak gösterilir. `is_spoiler` link `||...||`
içindeyse doğrudur.

## Nasıl çalışır

### Ayrıştırma

Her platform için `LazyLock<Regex>` kalıpları vardır, spoiler biçimi için ayrı kalıp bulunur. `Platform`
enum'u: Twitter, TikTok, Instagram, Reddit, Bluesky, Spotify, Twitch, Pixiv, Tumblr, Facebook, YouTube,
Letterboxd. Çıktıdaki `original_url` tekil ve normalize olduğu için önbellek anahtarının kaynağıdır.
Letterboxd profil kullanıcı adı için `letterboxd_profile_user` yardımcıdır.

### Otomatik akış (`enqueue_message`)

Kuyruğa girmeden önce, yetkisiz işler kapasite tüketmesin diye şunlar kontrol edilir:

1. Mesajda desteklenen link var mı.
2. Platform modülü etkin mi (etkin ayardan).
3. Rol filtresi (`policy::allows`) izin veriyor mu.

Sonra iş `try_send` ile kuyruğa yazılır. Kapasite `CONVERSION_QUEUE_CAPACITY` = 32'dir. Dolu kuyruk `false`
döner, orijinal mesaj korunur, çağıran AI akışına devam edebilir (`OVERLOADS` sayaçı artar).

Kuyruğu tek bir tüketici görevi boşaltır. Boş yuva (semafor, `CONVERSION_WORKERS` = 8) varken işi alır,
her işi kendi `tokio::spawn` görevinde yürütür. Böylece bir işteki panik yalnızca o kartı düşürür. Yuva
yokken iş kanalda bekler ve geri basınç korunur. Her iş `builtin::handle` çağırır; o da kendi içinde 16
iş sınırı uygular.

### Etkileşim akışı (`convert_now`)

Otomatik akıştan farkları:

- Platform modülü kapalı olsa da dönüştürür.
- X çevirisi açık, `translate_english` ile İngilizce dahil (`interaction_settings`).
- Silinecek kaynak mesaj ve taklit edilecek gönderen yoktur: `delete_original` ve `impersonate_user` kapalıdır.
- `actor` komutu kullanan kişidir. Rol filtresi ondan geçer, kartın paylaşan kaydı ona yazılır.
- 4 eşzamanlı işle sınırlıdır, doluysa `ConvertError::Busy`.
- Sonuç çağırana döner ve komut yanıtı buna göre güncellenir.

## Değişmezler

- Yeni `Platform` varyantı eklendiğinde `display_name`, `builtin::platform_key`, `builtin::fetch`
  dağıtımı, `settings` modül anahtarı, `telemetry::PLATFORMS` ve panel birlikte güncellenir. Derleyici ilk
  üçünü yakalar, son üçünü yakalamaz.
- Kuyruk dolduğunda iş sessizce bekletilmez, açıkça reddedilir.
- `parse_message` yalnızca ağ dışı, saf bir işlevdir. Testleri çok sayıda gerçek URL biçimini kapsar.

## Değiştirirken

Yeni URL biçimi: ilgili platformun regex'ini genişletin ve kanonik URL'nin değişmediğini test edin. Kısa
linkler (ör. `spotify.link`, `boxd.it`) ayrıştırıcıda tanınır, hedefe çözümleme platform modülünde yapılır.

## Testler

`cargo test --locked parser`, `cargo test --locked converters`.
