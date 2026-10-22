# Mimari

Botgek tek bir Rust ikilisidir: Discord gateway istemcisi, dönüştürme motoru, yönetim paneli ve SQLite
deposu aynı süreçte çalışır. Bu belge istek yaşam döngüsünü, zaman bütçelerini, önbellek ve eşzamanlılık
modelini ve tasarım kararlarının nedenlerini açıklar.

## Modül haritası

| Yol | Sorumluluk |
| --- | --- |
| `src/main.rs` | Süreç başlangıcı, yapılandırma, gateway ve panelin ayağa kaldırılması. |
| `src/handler.rs` | Mesaj, etkileşim ve tepki olayları; AI tetikleme; orijinal mesajı silme politikası. |
| `src/commands/` | `/dönüştür` slash komutu ve **Apps → Dönüştür** mesaj komutu. |
| `src/converters/parser.rs` | Desteklenen URL'lerin ayrıştırılması, kanonikleştirme, spoiler ve kullanıcı yorumu. |
| `src/converters/mod.rs` | Dönüşüm kuyruğu, eşzamanlılık sınırları ve etkileşim akışı. |
| `src/converters/builtin/` | Platform dönüştürücüleri, medya, çeviri, önizleme, Render ve güvenli HTTP. |
| `src/converters/translation.rs` | Proxy mesajlarında bayrak tepkisiyle çeviri. |
| `src/settings.rs` | Ayar şeması, kapsam çözümleme, doğrulama, atomik yazma ve göç. |
| `src/store.rs` | Kart–paylaşan kullanıcı kaydı (SQLite, WAL). |
| `src/web/` | Axum API'si, oturum kimlik doğrulaması ve gömülü panel varlıkları. |
| `src/ai/` | OpenRouter istemcisi, model normalizasyonu ve yedek model zinciri. |
| `src/policy.rs` | Rol filtresi kararları. |
| `src/telemetry.rs` | Süreç içi sayaçlar ve gecikme histogramı. |
| `src/version.rs` | Derleme zamanında gömülen sürüm künyesi. |

## İstek yaşam döngüsü

1. Gateway olayı `handler`'a düşer. Bot mesajları varsayılan olarak yok sayılır; panelden güvenli,
   impersonation'sız bot dönüşümü açılabilir.
2. Mesajda desteklenen bir bağlantı varsa iş, **32 kapasiteli** dönüşüm kuyruğuna alınır. Kuyruk
   dolduğunda iş sessizce bekletilmez; orijinal mesaj korunarak reddedilir.
3. `parser` platformu, kanonik URL'yi, spoiler işaretini ve kullanıcı yorumunu çıkarır.
4. Ayarlar global → sunucu → kanal sırasıyla çözümlenir. Thread yalnızca parent kanalın ayarını miras
   alır; kategori kapsamı yoktur. Modül kapalıysa dönüşüm yapılmaz.
5. Rol filtresi uygulanır. Boş allowlist erişim vermez; rol sorgusu başarısız olursa etkin filtre erişimi
   reddeder.
6. Platform dönüştürücüsü veriyi getirir. Zenginleştirme (çeviri, alıntı, önizleme, görüntülenme) ayrı
   işlerde ve bütçeyle yürür.
7. Kart Components V2 bileşenleri olarak üretilir ve Discord REST üzerinden gönderilir.
8. Yalnızca gönderim başarılıysa `DELETE_ORIGINAL` etkinse orijinal mesaj silinir ve kart–paylaşan
   eşlemesi SQLite'a yazılır. Sonucu belirsiz bir POST otomatik yeniden gönderilmez.
9. Telemetri sayaçları ve gecikme histogramı güncellenir.

## Zaman bütçeleri

| Aşama | Sınır |
| --- | --- |
| Kart hedefi (veri + medya + gönderim) | **1 sn** |
| Veri geldikten sonra isteğe bağlı zenginleştirme | **650 ms** |
| Hiç veri gelmezse ilk yanıt | **3 sn** |
| Taklit webhook'unun hazır olmasını bekleme | en fazla **200 ms** |
| Discord POST'una ayrılan pay | **250 ms** |
| X bağlantı önizlemesi (toplam) | **500 ms** |
| Reddit video birleştirme | **5 sn** (çağırandan iptal sinyali) |
| Genel upstream isteği | 6 sn |
| Medya indirme isteği | 12 sn |

X akışında yedek kaynağa geçiş (hedge) gecikmesi 500 ms'dir. Bu değerler yapılandırılmış sınırlardır;
**ölçülmüş p95 veya RAM garantisi değildir**.

## Eşzamanlılık ve iptal

| Akış | Eşzamanlı iş |
| --- | --- |
| Otomatik mesaj dönüşümü | 8 (kuyruk kapasitesi 32) |
| Native dönüşüm işleyicisi | 16 |
| Etkileşim (`/dönüştür`, mesaj komutu) | 4 |
| Reddit video birleştirme | 2 |
| AI isteği | 2 |

Etkileşimler otomatik kuyruktan ayrı ve küçük bir kapasiteyle sınırlanır; yoğunlukta komut anında
"meşgul" yanıtı alır, kuyruktaki konuşmaların kartları gecikmez. Her dönüşüm kendi görevinde yürür: bir
işteki panik yalnızca o kartı düşürür. Reddit birleştirmeye çağıran taraftan iptal sinyali verilir.

## Önbellek ve istek birleştirme

- Gönderi önbelleği **600 saniye / 512 girdi** ile sınırlıdır. Anahtara kaynak, çeviri, alıntı ve video
  bütçesi dâhildir.
- X kaynak sonuçları ayrıca **600 saniye / 512 girdi** ile önbelleklenir; aynı gönderi başka bir
  gönderide alıntılandığında tekrar kullanılabilir.
- Aynı anahtara gelen eşzamanlı istekler tek çekimde birleştirilir.
- Başarısız kaynak denemeleri kısa ömürlü (30 sn) bir hata önbelleğinde tutulur.
- NSFW/spoiler kararı kaynak önbelleğine yazılmaz.
- Metrikler önbellek ömrü boyunca eski olabilir.

## X veri planı

VxTwitter/otomatik çeviri yolu gelen veriye göre yeniden planlanır; kart gönderimi veya webhook taklidi
değişmez.

- Soğuk okumada orijinal Vx verisi, syndication metadatası ve çeviri açıksa dil ekli Vx yanıtı paralel
  istenir. Metrikler açıksa ve kaynakta görüntülenme yoksa FxTwitter yalnızca bu eksiği tamamlamak için
  paralel çağrılır.
- Dil ekli yanıt spekülatiftir: İngilizce/Türkçe metinde beklenmez; yabancı dilde ikinci bir seri ağ turu
  oluşmaz. Gereksizleşen okumalar iptal edilir.
- `qrtURL: null` ile eksik `qrtURL` birbirinden ayrılır. Bilinen üçüncü gönderi doğrudan çekilir; sırf
  keşif için orta gönderi yeniden istenmez.
- Yanıtlar gönderi kimliğiyle birleştirilir; boş/geç gelen yanıt mevcut çeviriyi, medyayı, topluluk
  notunu veya tamamlanmış alıntıyı silmez. Zaman aşımında mevcut orijinal korunur.
- Kaynak, çeviri ve alıntı işleri iki kademeli bütçeyi paylaşır: veri geldikten sonra zenginleştirme
  650 ms, hiç veri gelmezse ilk yanıt için 3 sn. Ayrı ayrı zaman aşımı süreleri birikmez.
- X bağlantı önizlemesi bu bütçenin içinde, kaynak istekleriyle paralel çalışır; ayrıca 500 ms sınırı
  vardır.
- Soğuk önbellekte en fazla dört kök GET oluşabilir. Bu bir hız garantisi değil, kritik yoldaki seri
  beklemeleri kaldırma tercihidir.

## Platform motorları

- **Native** motor, botun kendi çözümlemesini ve Components V2 kartını kullanır.
- **Proxy** motoru, bağlantıyı platformun proxy alan adına yönlendirir. Yalnızca TikTok, Bluesky ve
  Spotify için seçilebilir; bu platformlarda `fallback_to_proxy` ile native başarısızlığında proxy'ye
  düşülebilir.
- X, Instagram, Reddit, Twitch, Pixiv, Tumblr, Facebook ve YouTube Community Post **native-only**'dir;
  proxy modu ve proxy yedeği bu platformlarda yoktur.
- Tumblr önce fxtumblr uyumlu renderer'ı (`FXTUMBLR_DOMAIN`, varsayılan `tpmblr.com`) dener; böylece
  NPF/reblog zinciri ve çoklu medya tek güvenli render görselinde korunur. Renderer erişilemezse public
  OG fallback kullanılır.
- X GIF'leri vxTwitter gifconvert üzerinden animasyonlu AVIF bağlantısına dönüştürülür; yerel ffmpeg yolu
  yoktur. Görseller ve GIF'ler doğrudan CDN bağlantılarıdır; X görsellerinde `name=orig` kullanılır.
- Reddit videolarında video ve ses saf Rust ile MP4'e birleştirilir. Başarısızlıkta uzak, sessiz video
  korunur.

## Önizleme güvenliği

X bağlantı önizlemesi ve OG tabanlı adaptörler yalnızca public HTTP(S), 80/443 portları ve doğrulanan DNS
adreslerine gider:

- Özel, loopback, link-local, multicast ve dokümantasyon adresleri reddedilir.
- Yönlendirmeler yeniden doğrulanır; en fazla 3 yönlendirme izlenir.
- İstek ortam proxy'sini kullanmaz (`.no_proxy()`).
- Medya URL'lerinin sözdizimi ve DNS'i yayın öncesi kontrol edilir; bu, gelecekte oynatılabilirlik
  garantisi değildir.

## Render ve kalıcılık

- Native kartlar **yalnızca Discord Components V2** kullanır; klasik embed fallback'i veya kart stili
  seçimi yoktur.
- Özel emoji alanları boş bırakılırsa Unicode varsayılan kullanılır; özel emoji seti olmayan
  kurulumlar da kart üretebilir.
- `settings.json` yazımı aynı dizinde geçici dosya, flush ve atomik replacement ile yapılır. WebUI
  kayıtları seri işlenir; disk işlemi async worker'ı bloklamaz.
- Bozuk ayar dosyası varsayılan erişim politikasıyla başlatılmaz: dosya düzeltilmeli veya sağlam yedek
  geri konmalıdır. Göç öncesi yedek alınamıyorsa göç durur.
- SQLite WAL kullanır; işlemler sınırlı blocking worker üzerinden yürür. Kart kayıtları 200.000 girdiyle
  sınırlıdır ve budanır.
- SQLite açılamazsa bellek içi fallback loglanır ve readiness başarısız olur; bu kalıcı çalışma sayılmaz.

## WebUI ve kimlik doğrulama

- Panel gömülü varlıklarla servis edilir; ayrı bir frontend sunucusu yoktur.
- `ADMIN_TOKEN` 32–512 karakter olmalıdır. Token yoksa/geçersizse panel açılmaz; bot çalışmaya devam eder.
- Oturum 8 saat sürer; rastgele 64 karakterlik kimlik, `HttpOnly` ve `SameSite=Strict` çerez kullanılır.
  Yeniden başlatma tüm oturumları iptal eder.
- Durum değiştiren istekler `X-Botgek-Request: 1` başlığı ister; CORS açılmamıştır. Token sabit zamanlı
  karşılaştırılır ve yanıtlarda/loglarda görünmez.
- İstemci adresi başına 60 saniyede 20 başarısız girişten sonra giriş reddedilir; izlenen adres tablosu
  1024 girdiyle sınırlıdır. Eşzamanlı oturum sayısı 64'tür.
- Uygulama TLS sonlandırmaz; uzak erişimde HTTPS ters proxy veya SSH tüneli gerekir.

## AI ve çeviri

- AI sohbeti yalnızca `OPENROUTER_API_KEY` varsa ve `ai_chat` modülü açıksa çalışır. Bot etiketlendiğinde
  veya botun mesajına yanıt verildiğinde tetiklenir; bot mesajları AI'ı tetiklemez.
- Model adı OpenRouter biçimine normalize edilir; birincil model hata verirse yedek model zinciri denenir.
- Kullanıcı başına 3 saniye bekleme, süreç genelinde 2 eşzamanlı istek sınırı vardır. İstem 8.000
  karakterle, yanıt 8.000 karakter ve 1 MiB ile sınırlıdır.
- X otomatik çevirisinin hedef dili panelden `tr`, `en`, `de` veya BCP-47 uyumlu başka bir kod olarak
  seçilebilir.
- Proxy mesajlarında bayrak tepkisi anlık çeviri yapar; native kartlar kendi çeviri ayarını kullanır.
- Kart dönüşümü, AI sohbeti ve çeviri tepkileri aynı rol filtresi politikasını kullanır.

## Hata semantiği

- Orijinal mesaj yalnızca başarılı gönderimden sonra silinir. Sonucu belirsiz Discord POST'u otomatik
  yeniden gönderilmez; orijinal korunur ve hata loglanır.
- 429 bekleme süresi kısaltılmaz.
- Kapasite dolduğunda orijinal mesaj korunur.
- Kaynak yavaşladığında kart mevcut syndication verisiyle gönderilir.
- Reddit video birleştirme başarısız olursa sessiz uzak video korunur.
- Başarısız veya eksik önkoşullu testler başarılı gibi gösterilmez.
