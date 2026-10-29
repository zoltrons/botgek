# Önizleme ve güvenli HTTP

## Görev

Kullanıcıdan gelen ya da sayfalardan çıkan URL'leri güvenle çekmek, sayfaların OpenGraph etiketlerini
okumak ve API'si olmayan platformlar için genel önizleme kartı üretmek.

## Dosyalar

| Dosya | İçerik |
| --- | --- |
| `builtin/safe_http.rs` | `public_get`: DNS ve yönlendirmeleri doğrulayan, adresi sabitleyen HTTP istemcisi. |
| `builtin/preview.rs` | Beş OG-tabanlı platformun ortak önizleme sınırı. |
| `builtin/opengraph.rs` | `og:` ve `twitter:` meta etiketi çözümleyicisi. |

## Girdi ve çıktı

- `safe_http::public_get(...)`: güvenli GET, sınırlı gövde.
- `preview::fetch(parsed, platform) -> Result<BuiltinPost, String>`: genel önizleme gönderisi.
- `preview::from_external_html(url, platform, html)`: HTML elde olan çağıranlar için (Tumblr).
- `opengraph`: meta etiketlerini anahtar/değer olarak döner.

## Nasıl çalışır

### `safe_http`

Amaç SSRF'yi önlemektir. İstek öncesi ve her yönlendirmede:

- Uç nokta `allowed_endpoint` denetiminden geçer: herkese açık HTTP(S), yalnızca 80 ve 443 portları.
- Ana makine çözülür; sonuç boş olmamalı, 16 adresi aşmamalı ve **tüm** adresler herkese açık olmalı. Loopback, özel, link-local, paylaşılan,
  belgeleme, çok noktaya yayın ve IPv6 geçiş aralıkları reddedilir (`public_v4`, IPv6 eşdeğeri).
- Bağlantı doğrulanan adrese **sabitlenir**: aynı ana makine için DNS ikinci kez çözülmez (DNS rebinding
  engeli). Sabitlenmiş istemciler 90 sn ve en fazla 32 girdiyle önbelleklenir ve yalnızca aynı doğrulanmış
  adres kümesi için yeniden kullanılır.
- En fazla 3 yönlendirme izlenir, her yönlendirme baştan doğrulanır. İstek başına zaman aşımı çağıran tarafça verilir.
- `public_get_with` her adımda ek başlık gönderebilir (TikTok çerezi ve Referer için).

### `preview`

Facebook, Pixiv, Tumblr ve YouTube adaptörleri kendi veri yolu okunamadığında buraya düşer (kaynak yorumu
"yalnızca OG kullanan adaptörler" der). Toplam süre `REQUEST_TIMEOUT` (6 sn) ile sınırlıdır. Sayfa taşıması DNS'i her yönlendirmede doğrular.

Reddedilen sayfalar: yalnızca giriş isteyen, genel "bu içeriği görmek için giriş yapın" sayfaları
(`generic`), farklı ana makineye yönlenenler, kimlik bilgisi ya da `javascript:` içerenler. Medya
adresinin yalnızca sözdizimi ve DNS'i yayından önce denetlenir. Bu, medyanın sonra oynatılabileceğinin
garantisi değildir. Medya indirilmez.

Önizleme metni ortak markdown sözleşmesini izler (`escape_preview_markdown`), kullanıcı adı yoksa yazar
nötr kalır.

### `opengraph`

HTML'de `<meta property="og:*">` ve `twitter:*` etiketlerini okur. Tırnak karakteri içeren değerler,
kaçak tırnaklar ve önek benzeri öznitelik adları (`data-og:title` gibi) doğru işlenir. `og:title` yoksa
`<title>` kullanılır.

## Değişmezler

- Kullanıcı kaynaklı her URL `safe_http` üzerinden gider. Doğrudan `reqwest` çağırmayın.
- `og:url` güvenilmez. `LinkPreview.url` yalnızca DNS ve yönlendirme doğrulamasından geçen son URL'dir.
- Önizleme görseli indirilmez.
- Loopback test sunucuları normal akışta reddedilir. Testler fixture için ayrı yol kullanır.

## Değiştirirken

Yeni OG tabanlı platform: `preview::fetch(parsed, Platform::X)` çağıran küçük bir adaptör yazın. Genel
sayfa filtresini (`generic`) gevşetmeyin, yanlış önizlemeler buradan sızar.

## Testler

`cargo test --locked safe_http`, `cargo test --locked preview`, `cargo test --locked opengraph`. Özel ve
ayrılmış adres reddi, yönlendirme doğrulaması ve istemci yeniden kullanımı için testler vardır.
