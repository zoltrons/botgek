# Komutlar

## Görev

Slash komutlarını, mesaj ve kullanıcı sağ tık komutlarını kaydeder ve işler: `/dönüştür`, `/gif-yap`,
eğlence oyunları.

## Dosyalar

| Dosya | İçerik |
| --- | --- |
| `src/commands/mod.rs` | Komut tanımları, kayıt, `/dönüştür` ve **Apps → Dönüştür**, Spotify sayfalama bileşeni. |
| `src/commands/gif.rs` | `/gif-yap` ve **Apps → GIF yap**. |
| `src/commands/fun.rs` | `/uyum`, **Apps → Uyumumuz**, `/düello`, `/sayı-tahmini`, `/8top`, `/bomba`, `/refleks`, `/fal`. |

## Girdi ve çıktı

Girdi: `CommandInteraction` ve `ComponentInteraction`/modal etkileşimleri (`handler` yönlendirir). Çıktı:
etkileşim yanıtı ya da düzenlemesi. Dönüştürme kartı için `converters::convert_now`.

## Nasıl çalışır

### Kayıt

`register(http)` `ready` olayında arka planda çağrılır ve tanımları global olarak yazar. Komutlar her yerde, DM'ler
dahil, kullanılabilsin diye `everywhere` yardımcısı hem sunucu hem kullanıcı kurulumu bağlamlarını açar.
Slash adları Türkçe karakter içerir (`dönüştür`, `sayı-tahmini`).

### `/dönüştür` ve mesaj komutu

- Discord slash etkileşiminde yanıtlanan mesajı iletmez. Bu yüzden iki giriş vardır: `link:` argümanı ve
  mesaja sağ tık komutu (hedef mesaj `resolved.messages` içindedir).
- Dönüşüm `converters::convert_now` ile **beklenerek** yapılır. Platform modülü kapalı olsa bile çalışır.
  Rol filtresi, kanal ve thread motor ayarları uygulanır.
- X çevirisi bu akışta açıktır, `auto_translate` kapalı olsa bile. Hedef dil ayardan gelir.
- `kaynak:` seçeneği `vxTwitter (yedek)` değerini alırsa X verisi vxTwitter'dan çekilir.
- Sonuç `ConvertError` ile döner: `Unsupported`, `RoleDenied`, `Busy`, `Failed`. Kullanıcıya sabit,
  sır içermeyen kısa metin gösterilir. Hata metinleri gizli (ephemeral) gider. Yanıt önce ertelenir (defer):
  sunucu kanalında gizli ertelenir ve kart ayrı mesaj olarak gönderilir, DM'de kart etkileşim yanıtı olur.
- Etkileşim dönüşümleri 4 eşzamanlı işle sınırlıdır; dolunca anında "meşgul" döner.

### `/gif-yap`

X gönderisindeki videoyu ya da GIF'i FxEmbed GIF servisine (`gif.fxtwitter.com`) yönlendiren bir bağlantı
gönderir (`media::fx_gif_url`). Hedef gönderi sırayla: verilen `link`, linksiz komutta kanaldaki son 25
mesajda bulunan en yeni X bağlantısı ya da bot kartı, mesaj komutunda hedef mesaj. Bot kartında bağlantı
Components V2 metninde durur ve serenity modelinde tutulmadığı için mesaj REST'ten ham JSON olarak okunur.

### Eğlence oyunları (`fun.rs`)

- Etkileşim kimlikleri `fun:` önekiyle başlar. `handle_component` bileşen ve modal etkileşimlerini yönetir.
- Oyun durumu bellektedir: en fazla 500 oyun, 15 dakika yaşam süresi. Yeniden başlatmada kaybolur.
- Yanıtlar ham JSON olarak Discord'a gönderilir (`respond`), çünkü Components V2 tipleri serenity'de yoktur.
- `/uyum` skoru kullanıcı kimliklerinden deterministiktir, sıra fark etmez. `/fal` kişi ve güne göre sabittir,
  Türkiye saatiyle gece yarısı değişir.
- `/düello` ilk iki turu alanı kazanan ilan eder. `/bomba` ve `/düello` botla oynanabilir.

## Değişmezler

- Komut adı sabitleri (`CONVERT_COMMAND`, `GIF_COMMAND`, ...) hem kayıtta hem eşlemede kullanılır. Değiştirirken
  ikisini birlikte değiştirin ve `command_payloads_match_discord_schema` testini güncelleyin.
- Dönüşüm komutları kullanıcı mesajı silmez ve taklit yapmaz. Komutu kullanan kişi `actor` olarak kartın
  silme yetkisini alır.
- Etkileşim yanıtını 3 sn içinde vermek gerekir. Uzun işlerden önce yanıtı erteleyin (`defer`).

## Değiştirirken

Yeni oyun: `fun.rs` içine komut sabiti, tanım (`definitions`), işleyici ve bileşen kimliği öneki ekleyin.
Yeni slash komutu için `definitions` listesine ekleyip `handler` içindeki dağıtımı genişletin.

## Testler

`cargo test --locked commands`.
