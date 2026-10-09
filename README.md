# 🤖 Botgek - Modern Discord Sosyal Medya Bağlantı Dönüştürücüsü & AI Asistanı (Rust)

Discord sunucularında paylaşılan sosyal medya bağlantılarını (Twitter/X, TikTok, Instagram, Reddit, Bluesky) Discord'da kusursuz video, görsel ve metin önizlemesi sunan servislerle dönüştüren, **Rust (Serenity 0.12)** ile yazılmış yüksek performanslı Discord botu.

Ayrıca **Cyber-Ops WebUI Kontrol Paneli**, **Rol Tabanlı Filtreleme (Allowlist/Blacklist)**, **Bayrak Reaksiyonuyla Çeviri** ve **OpenRouter AI ile Sarcastic Sohbet Asistanı** içerir.

Raspberry Pi (Debian ARM64 / ARMv7) üzerinde çok düşük kaynak (~15-25 MB RAM) tüketerek 7/24 stabil çalışacak şekilde tasarlanmıştır.

---

## ✨ Temel Özellikler

- 🔗 **5 Sosyal Medya Platformu** (ok işaretinden sonraki adresler, proxy modelinde kullanılan yedek önizleme servisleridir):
  - 🐦 **Twitter / X** ➔ `fixupx.com`
  - 🎵 **TikTok** ➔ `tnktok.com` (fxTikTok)
  - 📷 **Instagram** ➔ `ddinstagram.com` (varsayılan artık **native embed**; proxy yalnızca yedek)
  - 🤖 **Reddit** ➔ `vxreddit.com`
  - 🦋 **Bluesky** ➔ `fxbsky.app`
- 🧩 **Built-in (Native) Embed Motoru — Proxy'ye Alternatif:**
  - Bot bağlantıyı harici bir önizleme servisine yönlendirmek yerine gönderiyi **kendisi çözer**, Discord embed'ini kendisi üretir: yazar/ad/avatar, doğrulama işareti, gönderi metni, etkileşim metrikleri, gönderi tarihi, alıntılanan gönderi (ikinci embed), topluluk notu ve medya galerisi.
  - 🖼️ **Medya yöntemi (kart hızını belirler):** WebUI'dan seçilir — `doğrudan` (**ışık hızı**, varsayılan), `otomatik` veya `dosya eki`. Işık hızında **hiçbir dosya indirilmez/yüklenmez**: görsel CDN'in hafif `medium` varyantından, video kartın içinde oynayan en uygun mp4 varyantından, GIF ise FxEmbed'in GIF servisinden gelir — kart veri gelir gelmez (~0 ms medya fazı) gönderilir; görselleri Discord kendisi X CDN'inden çeker (bu yüzden hafif varyant kullanılır). `otomatik` medyayı kısa bir süre bütçesi içinde indirip mesaja **dosya olarak** ekler (yetişmezse bağlantıya düşer), `dosya eki` her zaman indirir (tam çözünürlük + kaynak CDN'e bağımlı olmama; en yavaş seçenek).
  - 🧱 **Kart stilleri:** WebUI'dan seçilir — `modern` (tek parça Components V2 kartı; varsayılan), `otomatik` (video içeren gönderilerde modern kart, diğerlerinde klasik embed) veya `fxembed` (her zaman klasik embed).
  - 🚫 **Profil resmi yok:** Kart, X'in aksine **avatar göstermez** (Discord Components V2 küçük resmi yalnızca sağa koyabildiği için modelin solunda küçük avatar mümkün değil); yalnızca ad, doğrulama işareti ve (varsa) profil linki yer alır.
  - 🧱 **Modern kart (Components V2):** Gönderi, Discord'un yeni bileşen düzeniyle (`Container` + `Text Display` + `Media Gallery`) **tek kart** olarak, X'in kendi embed'ine benzer şekilde gönderilir:
    - Kullanıcının linkle birlikte yazdığı metin kartın **dışında**, kartın üstünde düz mesaj metni olarak gönderilir (Components V2 bayrağı `content` alanını yasakladığı için mesaj metninin yerini üst düzey `Text Display` alır; taklit açıkken mesaj kullanıcının kendi adıyla gittiğinden tam olarak kullanıcı yazmış gibi görünür, `>` alıntı bloğuna çevrilmez).
    - **Taklit kapalıyken** (ör. yanıtta) kart bot adına gittiği için kartın üstüne paylaşanın etiketi eklenir: kimin attığı görünür, ama ping atmaz (izinli etiket listesinden çıkarıldığı için düz `@kullanıcı` metni olarak çizilir — proxy/klasik embed akışındaki davranış).
    - `||link||` ile atılan **spoiler** gönderilerde kart içeriği gizli kalır: gönderi metni `||...||` içine alınır, karttaki `[Link](...)` spoiler olur ve galeri öğeleri `spoiler` olarak işaretlenir (klasik embed akışındaki gizleme davranışının modern kart karşılığı).
    - `**Ad <:verified:...> ([@kullanici](profil))**`
    - gönderi metni (+ varsa alıntı: alıntı bloğu **ince bir ayraç çizgisiyle** gövdeden ayrılır ve **tamamı blok alıntı** (`> `, sol şerit) içinde gösterilir — başlık satırı ile alıntılanan metin **aynı hizada** ve **arada boş satır olmadan** alt alta durur; X/fxembed düzeni)
    - `-# <:comment:...> 6  <:rt:...> 27  <:like:...> 386  <:metric:...> 12.3K`
    - `-# <:twitter:...> • <t:...:R> • [Link](gönderi)`
    - Video ve görseller **kartın içinde** oynar/görünür (klasik embed'lerde Discord bot mesajlarında `video` alanını göstermez); sol kenarda renkli çubuk kullanılmaz. Video indirilemediyse galeride **yalnızca video** gösterilir — kapak görseli ayrıca eklenmez (kapak + video yan yana iki öğe olarak çiziliyordu), oynatma bağlantısı kartın son satırında kalır.
    - Kullanılan emojiler WebUI'dan (`<:ad:id>` biçiminde) değiştirilebilir — kartın son satırındaki **X** ve **Instagram** göstergeleri ile metrikler (`comment`/`rt`/`like`/`metric`), çeviri notu ve alıntı işareti bu setten gelir; **kimlik yazılmayı unutulursa** (`:instagram:` gibi) alan botun kendi emojisine bağlanır, boş bırakılan alan Unicode emojiye düşer — böylece kart hiçbir durumda bozuk emoji göstermez. Discord bileşenleri reddederse otomatik olarak klasik embed'e düşülür.
  - 💬 **Yanıt desteği:** Kullanıcı linki bir mesaja **yanıt olarak** attıysa bot da **aynı mesaja yanıt verir** (proxy motoruyla aynı davranış). Discord webhook mesajları yanıt olamadığı için taklit açıkken bile yanıtlarda mesaj **bot adına** gönderilir (WebUI: *“Yanıtlarda yanıtı koru”*; kapatılırsa taklit korunur, yanıt bağlantısı düşer).
  - 🧹 **Zamanlı silme (hayalet mesaj yok):** `DELETE_ORIGINAL` açıkken orijinal mesaj **kart gönderildikten sonra**, arka planda silinir. Silme, mesaj en az **~1,2 sn** yaşına gelene kadar ertelenir; bu, gönderenin istemcisinin optimistic (`nonce`'lu) mesajını sunucu kopyasıyla eşleştirmesine zaman tanır ve mesajın **yalnızca gönderenin ekranında "hayalet" olarak kalmasını** (Discord istemci desync — discord-api-docs #850) engeller. Dönüşüm genelde zaten >1 sn sürdüğü için pratikte ek gecikme hissedilmez.
  - 🎭 **Kullanıcıyı taklit etme (webhook):** Mesaj, paylaşan kullanıcının **adı ve avatarıyla** gönderilir (`Manage Webhooks` izni gerekir). Böylece `<@kullanıcı>` etiketi ve `<link>` sarma hilesi gibi workaround'lara ihtiyaç kalmaz; mesaj sanki kullanıcı linki kendisi paylaşmış gibi görünür. Webhook kullanılamıyorsa (veya yanıt olarak atılan linklerde) mesaj bot adına gönderilir. **Alt başlıklar ve forum gönderileri de desteklenir:** Discord alt başlık kanallarında webhook oluşturmayı desteklemediği için webhook **üst kanalda** (metin/duyuru/forum) tutulur ve mesaj `thread_id` ile alt başlığa gönderilir (forum gönderileri de Discord'da birer alt başlıktır).
  - 🔁 **Alıntı medyası ve zincir:** Gönderiye medya eklenmemişse **alıntı zincirindeki ilk medya** karta alınır (X'in kendi kartına benzer: alıntının videosu kartın içinde oynar, değilse kapak + oynatma bağlantısı). Zincir 3 seviyeye kadar takip edilir: gönderi (A) ve alıntıladığı gönderi (B) medyasızsa B'nin alıntıladığı (C) medyası kullanılır. Gönderiye medya eklenmemişse **C de kartta gösterilir** (A + B + C birlikte; X'in yeni davranışı): modern kartta ikinci bir alıntı bloğu, klasik embed akışında üçüncü bir embed olarak. Alıntı başlığı `qutoe` emojisiyle ve **fxembed düzeni**yle, alıntı bloğunun **ilk satırı** olarak yazılır: `> <:qutoe:...> [Quote](alıntı linki) from **Ad** ([**@kullanici**](profil))`; alıntılanan metin aynı blok alıntının içinde, başlığın hemen ardından gelir.
  - 🎞️ **GIF desteği (FxEmbed'den):** X'in `animated_gif` medyası yalnızca **sessiz mp4** olarak sunulur; Discord mp4'leri oynatıcı olarak gösterdiği için GIF'ler video gibi görünürdü. Bot artık GIF'i doğrudan **FxEmbed'in GIF servisinden** çeker (`https://video.twimg.com/tweet_video/<ad>.mp4` → `https://gif.fxtwitter.com/tweet_video/<ad>.gif`; CDN'de önbellekli, `image/gif`) ve **kartın içinde GIF gibi** (döngülü, oynatıcısız) oynatır. Servis yanıt vermezse yerel `ffmpeg` dönüşümüne, o da yoksa mp4'e düşülür.
  - 🪶 **İmaj boyutu:** Docker imajlarında **`ffmpeg` kurulu değildir** (Debian'da tam paket ~75-95 MB bağımlılık çekiyor); GIF'ler zaten FxEmbed servisinden geldiği için buna gerek yok. Yerel yedeği de isterseniz: `docker build --build-arg WITH_FFMPEG=1 ...`. Sistem (deb) kurulumunda `ffmpeg` yalnızca **önerilir** (`Suggests`), zorunlu değildir.
  - 🔀 **Platform bazlı motor seçimi:** Her platform için `Proxy` veya `Native Embed` seçilebilir; native embed başarısız olursa isteğe bağlı olarak proxy linkine düşülür (`fallback_to_proxy`).
  - 🌐 **Veri kaynakları:** X için `X Syndication` (üçüncü taraf/API anahtarı yok), `FxTwitter` (görüntülenme, repost, topluluk notu) veya `Hibrit`; Reddit için herkese açık JSON, Bluesky için XRPC, TikTok için oEmbed, Instagram için Instagram'ın sunucu taraflı (SSR) embed sayfası + OpenGraph (native; proxy yedeği).
  - 📷 **Instagram native kart:** Yazar kullanıcı adı, görünen ad, doğrulama işareti, açıklama (caption), beğeni/yorum/görüntülenme ve gönderi tarihi gelir; `/p/`, `/reel/`, `/reels/` ve `/tv/` bağlantıları desteklenir. Instagram mp4'ü girişsiz istemcilere vermediği için **reel/video** gönderileri varsayılan olarak videonun **kapak karesini** + `Reel`/`Video` etiketini ve gönderi bağlantısını gösterir. `.env`'de `INSTAGRAM_SESSIONID` tanımlanırsa bot gönderiyi Instagram'ın uygulama API'sinden (`/api/v1/media/{pk}/info/`) çeker: **reel kartın içinde oynar**, carousel'in tüm görselleri galeriye girer ve metrikler (beğeni/yorum/görüntülenme) kesinleşir. Çerez yoksa veya süresi dolmuşsa davranış değişmez (kapak + etiket). Instagram giriş duvarı döndürürse akış hata verir ve ayar açıksa proxy motoruna düşülür.
  - 📊 **Eksik metrikler tamamlanır:** Syndication uç noktası repost ve görüntülenme vermez; eksik alanlar FxTwitter yanıtından doldurulur (mevcut değerler korunur).
  - ⚡ **Hızlandırılmış akış (milisaniye seviyesi / ışık hızı):**
    - **Sıfır medya indirmesi:** Varsayılan `doğrudan` modunda hiçbir dosya indirilmez veya Discord'a yüklenmez; kart yalnızca veri isteği + gönderimden oluşur (tipik **~0,3-0,7 sn**). Dosya yüklemek (8 MB'a kadar) en büyük gecikme kaynağıydı, artık kritik yolda değil.
    - **Bağlantı ön ısıtma:** Bot açılışında FxEmbed/FxTwitter API'sine, Discord REST'e ve medya CDN'ine **arka planda** ısıtma isteği yapılır; ilk kartta DNS + TLS el sıkışması beklenmez (~200-400 ms kazanç).
    - **Tek istek:** Çeviri açıkken (varsayılan) metin, X'in çevirisi, metrikler, topluluk notu ve medya bilgisi **tek FxTwitter yanıtında** gelir — ek istek yok.
    - **Gecikme sigortası (hedge):** FxTwitter **0,5 sn** içinde yanıt vermezse X syndication ucu **paralel** başlatılır ve ilk gelen veri kartı üretir; X ucu kazanırsa çeviri/metrikler için FxTwitter'a 0,3 sn ek süre tanınır. Askıda kalan bir uç kartı 6 sn'ye kadar (istek zaman aşımı) geciktiremez.
    - **Yedek istek tekrarı yok:** Hızlı yol hata verirse aynı istek yavaş yolda **ikinci kez** atılmaz (eskiden FxTwitter iki kez isteniyordu).
    - **Paralel istek:** Kaynak `syndication` iken X ucu ile FxTwitter **aynı anda** istenir; toplam gecikme tek istek kadardır.
    - **Ağırlıklı video varyantı (kalite ÷ boyut):** FxTwitter'ın `formats` listesinden (mp4 + bit hızı + boyut) **oynatıcıyı kilitlemeyen en iyi kalite** seçilir: 1 dakikalık gönderide 1080p (~80 MB) yerine sığan en yüksek kalite (~6 MB) kullanılır, video kartta **anında** oynamaya başlar. Hiçbiri bütçeye sığmazsa en küçük varyant (en hızlı oynatma) seçilir.
    - **Hafif görseller:** Kart görselleri `?name=orig`/`?name=large` (180 KB - birkaç MB) yerine **`?name=medium`** (1200 px, ~%35-40 daha az veri) ile gösterilir — Discord kartı görseli en fazla ~550 px çizdiği için gözle görülür kalite kaybı olmaz, görsel daha erken belirir. Dosya eki modunda indirme yolu yine **tam çözünürlük** ister (`name=orig`).
    - **Süre bütçesi (dosya eki modları):** `otomatik` modda görsel **0,6 sn**, GIF **0,9 sn**, video **1,2 sn**; `dosya eki` modunda 2,5 / 3 / 5 sn. Bütçe dolan indirme iptal edilir (yarıda kalan trafik de tükenmez) ve ilgili medya bağlantıya düşer (video kartın içinde oynamaya devam eder).
    - **Ağırlıklı GIF yarışı:** FxEmbed GIF servisi (CDN'de önbellekli) yarışta 150 ms öne alınır; servis gecikirse yerel `ffmpeg` yedeği başlar ve **gerçek GIF** ürettiyse yarışı kazanır. Hiçbiri olmazsa hazır mp4, o da yoksa uzak bağlantı kullanılır.
    - **Paralel yan işler:** Orijinal mesajı silme ve taklit webhook'unun çözümlenmesi (soğuk kanalda 1-2 REST çağrısı) **veri çekmeyle paralel** yürür; 🗑️ reaksiyonu ve gecikmiş silme mesaj gönderildikten sonra arka planda eklenir.
    - **Önbellek:** Aynı link 10 dakika içinde tekrar paylaşılırsa ağ isteği hiç yapılmaz (anında yanıt).
    - **HTTP/2 + bağlantı havuzu:** Paralel istekler tek bağlantıda çoğullanır, TLS el sıkışması tekrarlanmaz.
    - Her dönüşümde süre log'lanır: `Hazırlık: veri X ms, medya Y ms` ve `toplam Z ms` (RUST_LOG=debug).
    - **Ağırlıklı video varyantı (kalite ÷ tahmini boyut):** X birden fazla mp4 sunuyorsa **indirilebilecek en yüksek kaliteli** varyant seçilir; bütçeyi aşan dev varyant hiç seçilmez (eskiden en yüksek bit hızlı varyant seçilip indirilemediği için kart videoyu kaybediyordu). Hiçbiri bütçeye sığmıyorsa en küçük dosya seçilir — indirme en hızlı biçimde tamamlanır.
    - **Paralel yan işler:** Orijinal mesajı silme ve taklit webhook'unun çözümlenmesi (soğuk kanalda 1-2 REST çağrısı) **veri çekmeyle paralel** yürür; 🗑️ reaksiyonu ve gecikmiş silme mesaj gönderildikten sonra arka planda eklenir.
    - **Önbellek:** Aynı link 10 dakika içinde tekrar paylaşılırsa ağ isteği hiç yapılmaz (anında yanıt).
    - **HTTP/2 + bağlantı havuzu:** Paralel istekler tek bağlantıda çoğullanır, TLS el sıkışması tekrarlanmaz.
    - Her dönüşümde süre log'lanır: `Hazırlık: veri X ms, medya Y ms` ve `toplam Z ms` (RUST_LOG=debug).
  - 🇹🇷 **Otomatik Türkçe çeviri (yalnızca Grok):** Türkçe ve İngilizce **dışındaki** X gönderileri, dil ekli uçtan gelen **X'in kendi (Grok) çevirisiyle** otomatik Türkçeye çevrilir. Çeviri **Grok'tan gelmiyorsa kullanılmaz** (FxEmbed, X çevirisi yoksa kendi yedek servisine — ör. DeepL — düşer; o çeviri karta yazılmaz, gönderi orijinal metniyle kalır). Çevrilen gönderinin alt bilgisine `🌐 de → tr` notu eklenir; Türkçe/İngilizce gönderiler olduğu gibi bırakılır. Hangi sağlayıcıdan geldiği `RUST_LOG=debug` ile görülebilir.
- 🎯 **Kusursuz ve Sade Görünüm:**
  - Native embed mesajlarında görünür metin sadece `[Twitter](gönderi linki)` (+ varsa kullanıcı notu alıntısı `> not`) olur; linkler `< >` ile sarılmaz, mesaja `SUPPRESS_EMBEDS` bayrağı eklendiği için Discord ayrıca önizleme üretmez.
  - Proxy motorunda ise klasik `[:]({link}) [Platform](<orijinal>)` biçimi kullanılır.
  - 🙈 **Spoiler Desteği:** `||link||` olarak atılan mesajlar dönüştürüldüğünde de gizliliğini korur.
- 🇹🇷 **Bayrak Emojisiyle Anında Çeviri:**
  - Botun dönüştürdüğü mesaja bayrak emojisi (örn: 🇹🇷, 🇬🇧, 🇩🇪, 🇫🇷, 🇯🇵) bırakıldığında link otomatik olarak `/tr`, `/en` gibi dil parametrelerini alarak içeriği çevirir. Reaksiyon kaldırıldığında orijinal haline döner.
- 🗑️ **Reaksiyonla Temizleme:**
  - Mesaja eklenen `🗑️` emojisine orijinal mesaj sahibi veya sunucu yöneticisi/moderatörü bastığında mesaj anında silinir.
- 💬 **Sarcastic OpenRouter AI Asistanı:**
  - Bot etiketlendiğinde (`@botgekv2`) veya bota yanıt (Reply) verildiğinde akıllı, hazırcevap, hafif alaycı ve kısa (1-2 cümle) Türkçe cevaplar üretir.
  - 🛡️ **Dönüştürme kartları AI'yı tetiklemez:** Kartlara verilen yanıtlarda (etiketli yanıtlarda bile) AI çalışmaz; botun **kendi kartları** ayrıca gelen mesaj olarak hiç işlenmez. Kart tanıma mesaj metnine ek olarak **yapısal imzayla** yapılır: klasik embed'lerin alt bilgisindeki `Platform • Botgek` imzası ve modern (Components V2) kartların **içeriksiz/embedsiz** yapısı. Böylece bot yeniden başladığında (bellekteki mesaj kimliği kaydı boşaldığında) eski kartlara gelen yanıtlar da AI'yı tetiklemez.
- 💾 **Kalıcı Kayıtlar (SQLite):** Bot dönüştürdüğü her kart için *hangi mesajı hangi kullanıcının paylaştığını* **`botgek.db`** dosyasına yazar (WAL modu, tek tablo, birincil anahtar sorguları):
  - 🗑️ **Silme izni:** kartı paylaşan kullanıcı bu kayıtla bulunur; taklit webhook mesajlarında mesajın yazarı bot olmadığı için kayıt tek yoldur. Yeniden başlatma/güncelleme sonrası da çalışır.
  - 🛡️ **AI koruması** ve 🇹🇷 **çeviri reaksiyonu** ayrımı aynı kaydı kullanır (proxy mesajlarında gövdedeki `<@id>` etiketi yeterlidir).
  - Kayıt dosyası **hiçbir kaydı unutmaz**: bot kapansa, çökse veya güncellense bile veriler diskte kalır (eski davranış: bellekteki `HashMap` yeniden başlatmada siliniyordu). Yalnızca 200.000 kaydı aşan en eski satırlar budanır, kart silinince (🗑️) kaydı da silinir.
  - Yol: `BOTGEK_DB_PATH` → yoksa çalışma dizinindeki `botgek.db` (systemd: `/var/lib/botgek/botgek.db`, Docker: `/app/data/botgek.db` — `botgek-data` kalıcı volume'u). Dosya açılamazsa bot durmaz, bellek içi veritabanına düşüp uyarı log'lar.
- 🎛️ **Cyber-Ops Minimal WebUI (`http://localhost:3000`):**
  - Tüm modülleri (5 platform + bayrak çevirisi + AI sohbet) tek tıkla açıp kapatma; başlıkta gerçek sürüm ve **gerçek WebUI portu** gösterilir (`/api/info`).
  - **Tek sayfa düzeni:** solda ray (sürüm/port, telemetri LED'i, bölüm indeksi ve canlı sayaçlar, kapsam seçici, kayıt durumu), sağda dört bölüm — **Modüller**, **Rol filtresi**, **AI persona**, **Dönüştürme motoru**; kaydedilmemiş değişiklikler alt komut çubuğunda ve rayda işaretlenir.
  - 🔐 **Varlıklar ayrı servis edilir:** panel artık tek dev HTML yerine gömülü üç dosyadan oluşur — `ui/index.html` + `/app.css` + `/app.js` (üçü de `include_str!` ile ikiliye derlenir; çalışma anında `src/web/ui` dizini gerekmez). Panelde satır içi script/olay yöneticisi ve satır içi stil bulunmadığı için CSP **`script-src 'self'`** ile sıkı tutulur, tüm yanıtlarda `nosniff` / `X-Frame-Options: DENY` / `no-store` başlıkları döner; `/api/health` canlılık kontrolü içindir. Bu sözleşme (HTML kimlikleri ↔ istemci erişimleri, panel anahtarları ↔ ayar şeması, API yolları ↔ rotalar) testlerle doğrulanır.
  - **Sunucu seçici:** Ayarlar önce **Genel Varsayılan** olarak düzenlenir; bir sunucu seçildiğinde o sunucuya özel modül/rol/AI/motor ayarı tanımlanır (sunucu ayarı genel ayarı miras alır, "Varsayılana Sıfırla" ile silinir). Kaydedilmemiş değişiklik varken komut çubuğunda uyarı çıkar.
  - **Link Dönüştürme Motoru paneli:** Platform bazlı `Proxy` / `Native Embed` seçimi (**Native Embed seçiliyken yedek alan adı kutusu otomatik devre dışı kalır**), yedek proxy alan adları (kutu boşsa `.env` değeri geçerlidir ve ipucunda yazar), X veri kaynağı, **kart stili** (`fxembed` / `otomatik` / `modern`), **otomatik Türkçe çeviri**, **kullanıcıyı taklit etme**, modernde kart **emoji seti**, medya yükleme (görsel/video, maks. görsel sayısı, maks. video boyutu, proxy yedeği) ve embed içeriği (metrik, tarih, topluluk notu, alıntı) seçenekleri.
  - **Rol Tabanlı Filtreleme:** Her modül için ayrı `Devre Dışı` / `Kara Liste` / `Beyaz Liste`; roller sunucudan seçilir (Genel görünümünde rolün hangi sunucuya ait olduğu yazar) veya ID ile eklenir.
  - Sistem promptu ve AI modelini web arayüzünden anlık güncelleme (`Ctrl+S` ile kaydetme, hata durumunda sunucu mesajı gösterilir).
  - Yapılan tüm değişiklikler `settings.json` ile anında kalıcı olarak kaydedilir (eski tek `role_filter` alanı okunup modül filtresine taşınır, artık yazılmaz).

---

## 🚀 GitHub Actions ile Raspberry Pi Debian Derlemesi (CI/CD)

Projede iki ayrı GitHub Actions iş akışı (Worker) tanımlıdır:

1. **`build-release.yml` (Native Binary & Debian `.deb` Paketi):**
   - Kod her `push` edildiğinde veya `PR` açıldığında testleri (`cargo test`) çalıştırır.
   - `cross` aracıyla **Raspberry Pi 64-bit (`aarch64-unknown-linux-gnu`)** ve **Raspberry Pi 32-bit (`armv7-unknown-linux-gnueabihf`)** mimarilerine doğrudan Debian glibc uyumlu derleme yapar.
   - Her derleme için `.deb` Debian paketi ve `.tar.gz` arşivi üreterek GitHub Actions Artifacts sekmesine yükler.
   - Bir git etiketi (`v*`) basıldığında GitHub Releases sayfasına paketleri otomatik iliştirir.
2. **`docker.yml` (Çoklu Mimari Docker İmajı):**
   - `linux/amd64`, `linux/arm64` ve `linux/arm/v7` için Docker imajı derleyip GitHub Container Registry'ye (`ghcr.io`) yükler.

---

## 🍓 Raspberry Pi Debian Kurulumu

### Seçenek 1: Debian Paketi ile Kurulum (`.deb`) — *En Kolay Yol*

GitHub Actions sayfasından veya Releases sekmesinden mimarinize uygun `.deb` paketini indirin (`botgek_arm64.deb` veya `botgek_armhf.deb`):

```bash
# Paketi kurun
sudo dpkg -i botgek_arm64.deb

# Yapılandırma dosyanızı oluşturun ve düzenleyin
sudo cp /etc/botgek/botgek.env.example /etc/botgek/botgek.env
sudo nano /etc/botgek/botgek.env

# Bot kullanıcısı ve çalışma dizini (servis burada çalışır; kalıcı kayıt veritabanı
# botgek.db bu dizinde tutulur, böylece 🗑️ silme izni ve AI koruması kayıtları kalıcı olur)
sudo id -u botgek >/dev/null 2>&1 || sudo useradd -r -s /bin/false -d /var/lib/botgek botgek
sudo mkdir -p /var/lib/botgek
sudo chown botgek:botgek /var/lib/botgek

# Systemd servisini başlatın
sudo systemctl enable --now botgek

# Durum ve logları kontrol edin
sudo systemctl status botgek
sudo journalctl -u botgek -f
```

---

### Seçenek 2: Tarball Arşivi & Kurulum Betiği (`deploy-rpi.sh`)

```bash
# Arşivi çıkarın ve dizine girin
tar -xzvf botgek-raspberry-pi-64bit-arm64.tar.gz
cd botgek-raspberry-pi-64bit-arm64

# Otomatik kurulum betiğini root olarak çalıştırın
sudo bash deploy-rpi.sh

# .env dosyasını düzenleyip botu başlatın
sudo nano /etc/botgek/botgek.env
sudo systemctl start botgek
```

---

### Seçenek 3: Docker ile Çalıştırma

```bash
# Projeyi çekin
git clone <depo_adresi> botgek
cd botgek

# .env dosyasını ayarlayın
cp .env.example .env
nano .env

# Docker Compose ile arka planda başlatın
docker compose up -d --build
```

> **Ortam değişkenleri:** `.env` içindeki tüm değerler (`docker-compose.yml` → `env_file`) container'a aktarılır.
> Instagram reellerinin **kartın içinde oynaması** için `.env` dosyasına `INSTAGRAM_SESSIONID=<çerez>` satırını
> eklemeniz yeterlidir (bkz. yukarıdaki *Instagram native kart* notu); değer gizli tutulur —
> `settings.json`'a veya WebUI'a yazılmaz. Değeri doğrudan compose dosyasında/kabuk ortamında vermek
> isterseniz `docker-compose.yml` içindeki `INSTAGRAM_SESSIONID=${INSTAGRAM_SESSIONID:-}` satırı bunu destekler.

---

## 🛠️ Discord Developer Portal İzinleri

1. [Discord Developer Portal](https://discord.com/developers/applications) adresine gidin.
2. Botunuzun ayarlarından **Bot** sekmesine gelin:
   - **`MESSAGE CONTENT INTENT`** seçeneğini mutlaka **AÇIK (Enabled)** yapın.
3. **OAuth2 -> URL Generator** üzerinden botu sunucunuza şu yetkilerle davet edin:
   - `Send Messages`, `Manage Messages`, `Embed Links`, `Read Message History`, `Add Reactions`.

---

## 📤 Projeyi GitHub'a Gönderme (Push)

GitHub üzerinde yeni ve boş bir repository oluşturduktan sonra terminalinizde şu komutları çalıştırın:

```bash
# 1. Uzak depoyu ekleyin (kendi repo linkinizi yazın)
git remote add origin https://github.com/KULLANICI_ADI/REPO_ADI.git

# 2. Ana dala pushlayın
git branch -M main
git push -u origin main

# 3. Yeni bir sürüm oluşturup Raspberry Pi derlemelerini GitHub Releases'e göndermek için:
git tag v0.1.0
git push origin v0.1.0
```
