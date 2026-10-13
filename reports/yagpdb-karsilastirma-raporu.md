# Botgek ↔ YAGPDB entegrasyon karşılaştırma raporu

**Tarih:** 13 Ekim 2026
**İncelenen Botgek commit'i:** `e9e486a` (`fix: show Reddit post titles in native cards`)
**İncelenen YAGPDB snapshot'ı:** `e267c77643036fdcf77d2b14ff8153908d2ee8e3`
**YAGPDB kaynağı:** [botlabs-gg/yagpdb](https://github.com/botlabs-gg/yagpdb)

## 1. Yönetici özeti

Botgek'in mevcut kodu küçük/orta ölçekli, tek süreçli bir Discord link dönüştürücü için
beklenenden daha dikkatli güvenlik önlemleri içeriyor. Özellikle aşağıdakiler doğru yönde:

- dış önizleme isteklerinde SSRF/DNS rebinding koruması (`safe_http.rs`),
- Discord 429 ve belirsiz POST sonucunda körlemesine tekrar göndermeme,
- `settings.json` için geçici dosya + `sync_all` yaklaşımı,
- SQLite ile kart/yazar ilişkisinin yeniden başlatma sonrasında korunması,
- WebUI'da HttpOnly/SameSite cookie, CSRF, CSP ve gövde limiti,
- Discord/AI mesajlarında mention güvenliği ve içerik uzunluğu sınırları.

Ana sorun, bu iyi önlemlerin tüm entegrasyon yollarında aynı standarda uygulanmamış olmasıdır.
YAGPDB ile karşılaştırınca en önemli açıklar şunlardır:

1. **Spotify kısa link çözümleme yolu**, normal HTML yolundaki gövde/redirect/statü
   kontrollerini paylaşmıyor.
2. **Components V2 kart tanıma imzası fazla geniş**; başka bot/webhook mesajları Botgek kartı
   sanılabilir.
3. **Proxy dönüşüm mesajı tanıma sezgisel**; normal bir mention + kalın Markdown mesajı
   dönüşüm sanılabilir.
4. **AI için yalnızca kullanıcı başına 3 saniyelik cooldown var**; giriş boyutu, guild/global
   bütçe ve eşzamanlı istek sınırı yok.
5. **SQLite açılamadığında bot çalışmaya devam ediyor**; kalıcı yetki verisi kaybolurken
   yalnızca readiness 503 dönüyor.
6. **Gateway olayları uygulama seviyesinde bounded queue/backpressure katmanından geçmiyor**;
   dış HTTP ve Discord gönderimleri doğrudan event callback'i içinde başlıyor.
7. **Windows'ta mevcut settings dosyasının üzerine atomik yazma davranışı doğrulanmamış**;
   `fs::rename` mevcut hedef dosyayı değiştirmeyebilir.

YAGPDB'nin shard-aware event pipeline'ını, PostgreSQL/Redis mimarisini veya tüm platformunu
aynen kopyalamak doğru hedef değildir. Önce mevcut tek süreçli botun sınırlarını sağlamlaştırmak,
ardından ihtiyaç oluşursa queue/worker ve dağıtık koordinasyon eklemek daha düşük risklidir.

## 2. Kapsam ve yöntem

İnceleme read-only yapıldı. Botgek'in gateway başlangıcı, event handler'ları, converter akışı,
Discord REST/webhook transport'u, harici HTTP istemcileri, AI akışı, ayar/veri saklama, WebUI,
test ve CI dosyaları incelendi. YAGPDB'de özellikle şu alanlar karşılaştırıldı:

- `bot/` ve `bot/eventsystem/`: shard, queue, event sıralaması ve retry,
- `commands/` ve `customcommands/`: middleware, permission, cooldown ve execution limitleri,
- `common/cacheset/`, `lib/dstate/`, PostgreSQL ve Redis kullanımı,
- Discord REST, YouTube WebSub ve diğer HTTP entegrasyonları,
- `web/`: OAuth, session, access level ve form doğrulama,
- test/CI ve deployment yaklaşımı.

Bu rapor iki tür bulguyu ayırır:

- **Doğrulanmış kusur/risk:** Kaynak kodun mevcut davranışı doğrudan gösteriyor.
- **Mimari boşluk:** Bugün bozuk olmak zorunda değil; yük, restart, çoklu instance veya API
  değişikliği altında güvence eksikliği yaratıyor.

## 3. Mimari karşılaştırma

| Alan | Botgek | YAGPDB | Değerlendirme |
|---|---|---|---|
| Gateway | Serenity callback'leri, tek süreç/tek bot akışı | Shard manager, shard başına event queue, READY/RESUME tracker | Botgek küçük kurulum için yeterli; yoğunlukta event backpressure eksik |
| Event sıralaması | Framework callback sırasına büyük ölçüde bırakılmış | Pre-state, post-state ve async-post-state fazları | YAGPDB'nin state yarışlarını azaltan net kontratı yok |
| Hata izolasyonu | HTTP transport'ta bilinçli retry/no-retry; callback seviyesinde sınırlı | Handler retry, exponential backoff ve panic recovery | Botgek'te uzun süren/çöken işlerin kuyruk davranışı eksik |
| Discord REST | Ortak reqwest client, route/global cooldown, bounded response, belirsiz POST'ta resend yok | REST concurrency, retry, global 429 yayılımı, metrikler | Botgek iyi bir temel kurmuş; çoklu node ve kapsamlı ölçüm yok |
| Harici HTTP | Platform başına farklı istemci desenleri; `safe_http` güçlü | YouTube WebSub güçlü; bazı eski istemciler daha gevşek | Botgek'te güvenlik kontrolleri Spotify/AI yollarına da taşınmalı |
| Kalıcı veri | Ayarlar atomik JSON; kart sahipliği için SQLite | PostgreSQL ağır/kalıcı veri, Redis cache/coordination | Botgek operasyonu basit; fallback ve çoklu instance garantisi zayıf |
| Cache/single-flight | Yerel keyed lock ve bazı semaphore'lar | TTL cache, duplicate fetch birleşimi, Redis pub/sub invalidation | Botgek tek süreçte iyi; processler arası koordinasyon yok |
| Yetki | Modül bazlı guild role allow/deny | Komut + bot permission + role/channel override + cooldown | Botgek kapsamına göre yeterli; komut/kanal seviyesinde daha az ifade gücü var |
| WebUI güvenliği | Token login, CSRF, CSP, cookie güvenliği, body limit | Discord OAuth state, role-based read/write access, origin koruması | İki taraf da güçlü; YAGPDB'nin yetki modeli daha zengin |
| Gözlemlenebilirlik | Süreç içi sayaç/histogram, health/ready/metrics | Prometheus, queue pressure, cache, REST ve shard metrikleri | Botgek kritik queue/AI bütçe metriklerinden yoksun |
| Dağıtım | Tek binary + WebUI, SQLite/JSON | Standalone, Docker, ayrı process'ler, shard orchestrator | Botgek'in sadeliği avantaj; yatay ölçek hedefleniyorsa temel eksik |

## 4. Öncelikli bulgular

### B-01 — Çalışma ağacında gerçek sırlar bulunuyor (Kritik)

**Kanıt:** `.env` çalışma ağacında mevcut; incelemede Discord tokenı ve OpenRouter anahtarı
bulunduğu tespit edildi. Dosya Git index'inde görünmese ve `.gitignore` ile dışlansa da bu,
secret'ın makinede/backup/log geçmişinde bulunmadığı anlamına gelmez.

**Etkisi:** Discord tokenı bot hesabının tamamen ele geçirilmesine, OpenRouter anahtarı ise
yetkisiz harcamaya yol açabilir.

**Düzeltme:**

1. Discord bot tokenını ve OpenRouter anahtarını hemen döndürün.
2. Git geçmişi, CI artifact/log'ları ve yedeklerde secret taraması yapın.
3. `.env` yerine deployment secret store veya erişim izinleri sıkı bir servis environment dosyası
   kullanın.
4. Secret'ın kendisini rapora, test fixture'ına veya loga yazmayın.

Bu konu YAGPDB karşılaştırmasından bağımsız, doğrudan operasyonel güvenlik engelidir.

### B-02 — Spotify kısa link yolu standart HTTP güvenlik katmanını atlıyor (Yüksek)

**Dosya:** `src/converters/builtin/spotify.rs:112-155`

`resolve_target()` kısa linki çözerken ortak client'ın varsayılan redirect davranışını kullanıyor,
yanıt status'ünü doğrulamadan `response.text()` çağırıyor ve gövdeyi sınırlamıyor. Aynı dosyadaki
`get_html()` (`:170-210`) ise status, `Content-Length` ve akış boyutunu kontrol ediyor.

**Etkisi:** Beklenmeyen redirect zinciri, güvenilmeyen hedefe erişim ve sınırsız response body
tüketimi riski oluşuyor. Spotify normalde güvenilir bir servis olsa da entegrasyon sınırı
uzak servisin davranışına bırakılmış.

**YAGPDB kıstası:** YouTube WebSub istemcisi explicit timeout/status/retry/backoff kullanıyor;
YAGPDB'nin bazı eski HTTP istemcileri de gevşek olsa bile bu, gevşek yolu kopyalamak için gerekçe
değil.

**Düzeltme:** Kısa link için de `safe_http` benzeri özel client kullanın: redirect'i kapatıp elle
en fazla 3 kez takip edin, her hedefte scheme/port/public-IP doğrulayın, status kontrol edin ve
akış gövdesini ortak `MAX_BODY_BYTES` ile okuyun. `response.text()` doğrudan kullanılmamalı.

### B-03 — Components V2 kart imzası yanlış pozitif üretebilir (Yüksek)

**Dosya:** `src/converters/builtin/mod.rs:667-714`

`has_card_signature()` şu üç özelliği yeterli sayıyor:

- mesaj bot veya herhangi bir webhook tarafından gönderilmiş,
- `content` boş,
- embed yok.

Bu yapı Botgek kartlarına özgü değil. `is_card_message()` içinde Components V2 flag'i ve SQLite
kaydı için daha güçlü yollar var; ancak fallback imzası halen başka bot/webhook mesajlarını da
kapsıyor.

**Etkisi:** Başka bir webhook'un boş Components V2 mesajına verilen cevapta AI devre dışı
kalabilir; reaksiyon/çeviri akışları da yanlış sınıflandırılabilir.

**Düzeltme:** Botgek kartlarına doğrulanabilir bir marker ekleyin ve yalnızca onu kabul edin:
Components V2 flag'i + Botgek'e özgü component/custom-id veya güvenilir webhook kimliği gibi
alanları birlikte kontrol edin. Marker taşımayan eski mesajlarda kesin olmayan tahmin yerine
yalnızca kalıcı kart ID kaydına güvenmek daha güvenlidir.

### B-04 — Proxy dönüşüm mesajı içerik sezgisiyle tanınıyor (Yüksek)

**Dosya:** `src/handler.rs:483-510`

`is_link_conversion_message()` mesajın `<@...>` ile başlamasına ve `[**`, `[:](` veya birkaç
platform label'ından birinin geçmesine bakıyor. Örneğin `<@123> [**normal konu**]` ile başlayan
geçerli bir kullanıcı mesajı dönüşüm sanılabilir.

**Etkisi:** Bu mesaja verilen reply'larda AI yanlışlıkla susturulur; yanlış negatiflerde ise
dönüşüm kartına reply AI'ı tetikleyebilir.

**Düzeltme:** Heuristic yerine dönüşüm sırasında yazılan mesaj ID'sini SQLite'a kaydedin ve
`referenced_message_id` üzerinden sorgulayın. Restart sonrası eski proxy mesajları için de
strict parser kullanın: ilk mention ID'si + bilinen platform + geçerli URL/host + beklenen
mesaj şeması birlikte doğrulanmalı. `content.contains("[**")` tek başına karar kriteri olmamalı.

### B-05 — AI akışında maliyet ve eşzamanlılık koruması yetersiz (Yüksek)

**Dosyalar:** `src/handler.rs:146-177`, `src/ai/mod.rs:169-307`

Mevcut koruma yalnızca process içi kullanıcı başına 3 saniye cooldown. Şunlar yok:

- temizlenmiş prompt için karakter/token sınırı,
- guild veya global rate limit/bütçe,
- eşzamanlı OpenRouter istekleri için semaphore,
- fallback zincirinin toplam çağrı bütçesi,
- processler arası limit.

İlk OpenRouter isteği başarısız veya parse edilemezse istemci tarafında ikinci fallback isteği de
gönderilebiliyor. Çok sayıda mention spam'i, API kotasını ve maliyeti hızla tüketebilir.

**YAGPDB kıstası:** YAGPDB komut middleware'inde user/guild cooldown, execution lock ve çıktı/
template limitleri merkezi uygulanıyor; custom command başına çalıştırma sayısı da sınırlı.

**Düzeltme sırası:**

1. Prompt'u sabit bir karakter/token limitinde kes veya reddet.
2. Global ve guild bucket rate limit ekleyin.
3. OpenRouter çağrılarını bounded `Semaphore` ile sınırlayın.
4. Fallback'ı tek bir toplam deneme bütçesine bağlayın ve 429 için `Retry-After`/backoff uygulayın.
5. `ai_requests_total`, `ai_rejected_total`, `ai_inflight`, tahmini token/maliyet ve latency
   metriklerini ekleyin.

### B-06 — Kalıcı SQLite açılamadığında kritik yetki verisi kayboluyor (Yüksek)

**Dosyalar:** `src/store.rs:81-102`, `src/main.rs:140-142`, `src/web/mod.rs:223-238`

`Db::open()` dosyayı açamazsa sessizce `:memory:` veritabanına düşüyor ve bot çalışmaya devam
ediyor. Readiness 503 döndürse de Discord event handler'ları mesaj işlemeyi sürdürüyor.

**Etkisi:** Restart sonrası kart-yazar kayıtları kaybolur; taklit webhook kartlarında silme yetkisi
ve AI koruması yanlış çalışabilir. Operatör yalnızca readiness'i izlemiyorsa sistem sağlıklı
sanılabilir.

**YAGPDB kıstası:** Kalıcı konfigürasyon/ağır veri PostgreSQL'de, ephemeral koordinasyon Redis'te
tutuluyor; kalıcı backend yokken normal işleyişe devam etme varsayımı yok.

**Düzeltme:** Production modunda kalıcı DB açılamazsa startup'ı fail-fast yapın. Fallback zorunluysa
`persistence_degraded` durumunu global bir circuit breaker olarak kullanıp kart silme/AI/impersonation
gibi kayıt gerektiren akışları durdurun; bunu Discord'a ve alerting'e açıkça yansıtın.

### B-07 — Windows'ta settings replacement atomik olmayabilir (Yüksek, doğrulama gerekli)

**Dosya:** `src/settings.rs:1018-1077`

Geçici dosya yazılıp `sync_all` sonrasında doğrudan `fs::rename(&temp, path)` çağrılıyor. Unix'te
beklenen replacement davranışı vardır; Windows'ta hedef dosya mevcutken `rename` çoğu durumda
üzerine yazmaz. Proje Windows'u desteklediği için WebUI'daki ikinci ve sonraki kayıtlar 500 ile
sonuçlanabilir.

**Düzeltme:** Windows'ta `ReplaceFileW`/`MoveFileExW(MOVEFILE_REPLACE_EXISTING | WRITE_THROUGH)`
ve Unix'te mevcut rename yolunu ayırın veya bunu güvenilir biçimde yapan küçük bir atomic-write
abstraction kullanın. CI'da mevcut `settings.json` üzerine iki ardışık save testi ekleyin.

### B-08 — Event ve outbound iş akışında bounded queue yok (Yüksek mimari boşluk)

**Dosyalar:** `src/handler.rs:66-229`, `src/converters/mod.rs:30-167`

Gelen mesaj callback'i doğrudan parser, harici HTTP, medya indirme, Discord gönderimi ve
reaksiyon ekleme zincirini çalıştırıyor. Native tarafta bazı semaphore/single-flight korumaları
var; ancak proxy yolu, kanal başına sıralı gönderim ve tüm event'ler için global bounded queue
kontratı görünmüyor.

**YAGPDB kıstası:** `bot/eventsystem/eventsystem.go` shard başına kuyruk kullanıyor; presence/
typing gibi düşürülebilir event'leri ayırıyor, kritik event'lerde backpressure uyguluyor.
Komut/message gönderiminde de kanal sırasını koruyan queue var.

**Etkisi:** Yavaş upstream veya Discord 429 sırasında aynı kanaldan çok sayıda mesaj aynı anda
işleme alınabilir. Memory pressure, gecikmiş event'ler, out-of-order dönüşüm ve provider spam'i
oluşabilir.

**Düzeltme:** Önce basit tek-process çözüm ekleyin:

- bounded `mpsc` ile dönüşüm işleri kuyruğu,
- queue dolunca düşük değerli linkleri reddetme/ölçme,
- kanal başına keyed serial worker veya lock,
- native/proxy/media için ayrı concurrency bütçeleri,
- queue depth, wait time, dropped/rejected count metrikleri.

Bu, YAGPDB'nin tüm shard orkestrasyonunu getirmeden en yüksek faydayı sağlar.

### B-09 — AI response body için açık boyut sınırı yok (Orta)

**Dosya:** `src/ai/mod.rs:215-229`, `:283-297`

AI endpoint yanıtları `response.text()` ile doğrudan belleğe alınıyor. Log preview'i 400 karakterle
sınırlıyor; bu iyi bir log önlemidir ancak ağ yanıtının bellekteki boyutunu sınırlamaz.

**Düzeltme:** Status kontrolünden önce veya sonra response body'yi stream ederek örneğin 1–2 MiB
ile sınırlandırın; limit aşımını retry/fallback'e körlemesine taşımayın. Aynı helper, tüm JSON
entegrasyonları tarafından paylaşılmalı.

### B-10 — Webhook ile reply semantiği kayboluyor (Orta)

**Dosyalar:** `src/converters/builtin/raw.rs:101-121`, `src/converters/builtin/mod.rs:1128-1163`

`execute_webhook()` Discord webhook endpoint'i desteklemediği için `message_reference` alanını
payload'dan siliyor. Native akışta impersonation açıkken webhook kullanılabiliyor; ayar reply'ların
webhook ile gönderilmesine izin verdiğinde mesaj başarılı biçimde gönderilse bile Discord reply
olarak görünmeyebilir.

**Düzeltme:** Reply olan mesajlarda webhook yerine bot REST gönderimini zorunlu kılın veya WebUI
ayarını bu kombinasyonda reddedin/açık uyarı gösterin. Davranışı thread/forum ve DM için ayrı
contract testleriyle doğrulayın.

### B-11 — Discord gerçek contract/E2E testleri eksik (Orta)

**Mevcut durum:** `raw.rs` ve `transport.rs` yerel HTTP fixture testlerine sahip; README normal
testlerin sosyal upstream'lere bağlanmadığını doğru biçimde belirtiyor. Buna karşılık canlı Discord
kanalı, gerçek webhook oluşturma/yenileme, Components V2 kabulü ve forum/thread gönderimi CI'da
doğrulanmıyor.

**YAGPDB kıstası:** YAGPDB `go test ./...` ile geniş paket testleri çalıştırıyor; event sırası,
permission, cache/GC, form validation ve rate-limit davranışları için ayrı testleri var.

**Düzeltme:** Secret içermeyen iki katman ekleyin:

1. Discord API response fixture'larıyla raw JSON/multipart contract testleri.
2. Ayrı, manuel tetiklenen canary bot/guild testi: normal kanal, thread, forum, webhook token
   yenileme, reply, 429 ve Components V2.

Canlı testleri public PR'da zorunlu yapmayın; ancak release öncesi raporlayın.

### B-12 — Sürüm sözleşmesi ile Cargo metadata'sı ayrışmış (Orta/düşük)

**Dosyalar:** `Cargo.toml:1-5`, `src/version.rs:8-15`, `build.rs`

`Cargo.toml` sürümü `0.1.0`; `version.rs` sözleşmesi ise `0.2.0` bekliyor. Build script uyumsuzluğu
warning olarak bırakıyor. Bu durum `/api/info`, `--version`, paket metadata'sı, git tag ve release
notlarının farklı sürümler göstermesine yol açabilir.

**Düzeltme:** Tek sürüm kaynağı seçin. Release akışında Cargo sürümünü tag ile eşitleyin ve
uyumsuzluğu warning yerine CI failure yapın.

## 5. YAGPDB'nin doğru yaptığı ve Botgek'e aktarılabilecek desenler

### 5.1 Event pipeline ve state sınırları

YAGPDB event'i üç aşamaya ayırıyor: state öncesi senkron, state sonrası senkron, async sonrası.
Bu sayede permission/kanal/member state okunurken hangi snapshot'ın geçerli olduğu belli oluyor.
Botgek için karşılığı, önce mesajın immutable bir iş tanımına dönüştürülmesi; sonra dış HTTP ve
Discord gönderiminin worker'a devredilmesidir. Handler içinde doğrudan uzun IO yapılmamalı.

### 5.2 Backpressure ve kontrollü kayıp

Her event'i sonsuza dek kuyruğa almak da doğru değil. YAGPDB presence/typing gibi düşük değerli
event'leri düşürebiliyor, kritik event'lerde gateway reader'ı bloke ediyor. Botgek de:

- aynı linkin duplicate dönüşümlerini birleştirebilir,
- typing gibi yardımcı işlemleri düşürebilir,
- kullanıcı mesajı/dönüşüm işinde bounded queue dolunca açık bir `busy`/drop politikası
  uygulayabilir.

### 5.3 Kalıcı veri, cache ve koordinasyonu ayırma

YAGPDB PostgreSQL'i kalıcı ayar/ağır veri, Redis'i TTL/cache/lock/pub-sub için kullanıyor. Botgek
tek süreçte SQLite + in-memory lock ile sadeliği koruyabilir; fakat ikinci process veya shard
gündeme geldiğinde şu ayrım net olmalı:

- SQLite: kart sahipliği ve audit gibi kalıcı kayıt,
- memory: yalnızca process-local cache,
- dağıtık lock/rate-limit: Redis veya başka açıkça seçilmiş backend.

Yerel lock'u dağıtıkmış gibi değerlendirmeyin.

### 5.4 Merkezi permission/cooldown middleware'i

YAGPDB komut çalıştırmadan önce kullanıcı permission'ı, bot permission'ı, role/channel override,
NSFW, cooldown ve concurrency lock'u tek middleware'de kontrol ediyor. Botgek'in modül bazlı
`policy::allows()` yaklaşımı iyi bir başlangıç; AI için guild/global quota ve dönüşüm için kanal
override'ı aynı politika katmanına eklenmeli.

### 5.5 Idempotency ve retry ayrımı

Botgek'in belirsiz Discord POST sonucunda tekrar göndermemesi doğru bir karardır; aksi halde çift
mesaj üretilebilir. YAGPDB'deki retry kararı da handler/domain seviyesinde veriliyor. Benimsenmesi
gereken kural:

- GET/status sorgusu: timeout/backoff ile tekrar denenebilir,
- 429: `Retry-After` ve global/route cooldown,
- POST sonucu belirsiz: otomatik resend yok; reconciliation/log/manuel takip,
- webhook 401/404: token refresh bir kez, sonra fallback,
- parse/schema hatası: sonsuz retry yok.

### 5.6 Gözlemlenebilirlik

Mevcut `/api/metrics` faydalı, ancak YAGPDB seviyesine yaklaşmak için şu ölçümler eklenmeli:

- event queue depth, wait p95/p99, dropped/rejected count,
- platform/endpoint bazında HTTP latency, timeout, 429 ve schema error,
- Discord route/global cooldown süresi,
- AI inflight, rejected, fallback, latency ve yaklaşık maliyet,
- SQLite persistence degraded süresi,
- webhook refresh/fallback sayısı,
- kanal başına out-of-order veya duplicate suppression sayısı.

## 6. YAGPDB'den aynen alınmaması gerekenler

YAGPDB güçlü bir platformdur; fakat Botgek'in kapsamı için tüm mimariyi kopyalamak aşırı mühendislik
olur.

- PostgreSQL + Redis + shard orchestrator, tek bot/tek makine için operasyon yükünü artırır.
- YAGPDB'nin bazı ikincil HTTP entegrasyonlarında explicit timeout/status kontrolü eksikliği
  görülüyor; standardımız en iyi yolu kopyalamak olmalı, her kodu örnek almak değil.
- YAGPDB README'si Go sürümü konusunda `go.mod` ile geride kalmış görünüyor; dokümantasyon drift'i
  onların projesinde de var.
- Çok geniş custom command/template sistemi Botgek'in link dönüştürücü ürün hedefi değil.

Önerilen hedef: **YAGPDB'nin operasyonel prensiplerini, Botgek'in mevcut küçük yüzey alanını
koruyarak uygulamak.**

## 7. Önerilen düzeltme sırası

### Faz 0 — Hemen

- [ ] Discord ve OpenRouter sırlarını döndür; geçmiş/log/backup taraması yap.
- [ ] `cargo test` için Windows MSVC Build Tools/linker eksikliğini çöz veya CI runner'da test et.
- [ ] Production'da SQLite fallback kararını fail-fast/circuit-breaker olarak netleştir.

### Faz 1 — Entegrasyon güvenliği

- [ ] Spotify short-link resolver'ı bounded, status-checked, manually-followed safe HTTP helper'a
  taşı.
- [ ] Components V2 için Botgek'e özgü doğrulanabilir marker kullan.
- [ ] Proxy mesajlarını içerik sezgisi yerine persisted message ID + strict parser ile tanı.
- [ ] AI input/output body limitlerini ve global/guild/in-flight limitlerini ekle.
- [ ] Webhook reply davranışını düzelt veya ayarı güvenli kombinasyonlarla sınırla.

### Faz 2 — Dayanıklılık

- [ ] Bounded conversion queue ve kanal başına seri gönderim ekle.
- [ ] Retry/429/ambiguous sonuç politikasını tüm dış HTTP istemcileri için ortaklaştır.
- [ ] Queue, AI ve persistence metriklerini ekle; readiness'i alert'e bağla.
- [ ] Windows settings replacement testini CI'a ekle.

### Faz 3 — Ölçek gerekiyorsa

- [ ] Shard/autoshard ve multi-process ihtiyacını gerçek guild sayısı/throughput ile ölç.
- [ ] Gerekirse Redis tabanlı distributed lock/rate-limit ve PostgreSQL migration'a geç.
- [ ] YAGPDB benzeri shard ownership, READY/RESUME ve state recovery tasarla.

## 8. Doğrulama durumu

Çalıştırılan komut:

```text
cargo test --all-targets
```

Sonuç: Testler başlamadan derleme ortamı `link.exe` bulunamadığı için durdu. Hata koddan gelen
bir test assertion'ı değil; Windows MSVC linker/Visual Studio Build Tools eksikliği:

```text
error: linker `link.exe` not found
```

Dolayısıyla bu rapor için **testler geçti** denemez. YAGPDB araştırması ve Botgek kaynak incelemesi
read-only yapıldı; uygulama kodu değiştirilmedi. Bu rapor dosyası dışında mevcut değişikliklere
dokunulmadı.

## 9. Son karar

Botgek'in temel yönü yanlış değil; özellikle `safe_http`, Discord transport ve WebUI güvenliği
iyi mühendislik işaretleri. Yanlış olan, entegrasyon sözleşmelerinin bazı yollarda gevşemesi ve
kalıcılık/queue/AI kaynak sınırlarının yalnızca process-local varsayımlara dayanmasıdır.

**İlk üretim önceliği:** B-01, B-02, B-03, B-04, B-05 ve B-06.
**İlk mimari iyileştirme:** B-08'deki bounded queue + kanal sıralaması.
**YAGPDB'den alınacak ana fikir:** event'leri, state'i, permission'ı, retry'ı ve gözlemlenebilirliği
tek tek handler'lara bırakmak yerine merkezi, ölçülebilir kontratlar haline getirmek.

## 10. Uygulama durumu

Bu rapordaki kodlanabilir öneriler uygulandı:

- Spotify kısa linkleri güvenli, bounded ve manuel redirect doğrulamalı HTTP yoluna taşındı.
- Components V2 ve webhook kart tanıma daha muhafazakâr hale getirildi.
- AI prompt/response boyutları ve process içi eşzamanlı çağrı sayısı sınırlandı.
- AI için global ve guild bazlı 10 saniyelik bütçe eklendi.
- Production başlangıcında kalıcı SQLite zorunlu hale getirildi.
- Windows settings replacement için `MoveFileExW` replacement/write-through yolu eklendi.
- Sürüm dokümantasyonundaki sabit `0.2.0` örnekleri `Cargo.toml` tabanlı `X.Y.Z` sözleşmesine
  çekildi; paket sürümü release script'inin tek kaynağı olarak değiştirilmedi.
- Dönüşüm işlerine 32 kapasiteli FIFO bounded queue ve saturation telemetrisi eklendi.
- İlgili birim/sınır testleri eklendi.

Kod dışında kalan manuel işler:

- Gerçek Discord/OpenRouter secret'larının döndürülmesi.
- Visual Studio Build Tools + Windows SDK kurulumu.
- Canlı Discord canary/contract testi.

Son uygulama doğrulamasında `cargo fmt --all -- --check`, `cargo metadata --no-deps` ve
`git diff --check` başarılıdır. `cargo test --all-targets` ise test koduna ulaşmadan Windows
ortamında `link.exe` bulunamadığı için durmaktadır.
