# 🤖 Botgek - Modern Discord Sosyal Medya Bağlantı Dönüştürücüsü & AI Asistanı (Rust)

Discord sunucularında paylaşılan sosyal medya bağlantılarını (Twitter/X, TikTok, Instagram, Reddit, Bluesky) Discord'da kusursuz video, görsel ve metin önizlemesi sunan servislerle dönüştüren, **Rust (Serenity 0.12)** ile yazılmış yüksek performanslı Discord botu.

Ayrıca **Cyber-Ops WebUI Kontrol Paneli**, **Rol Tabanlı Filtreleme (Allowlist/Blacklist)**, **Bayrak Reaksiyonuyla Çeviri** ve **OpenRouter AI ile Sarcastic Sohbet Asistanı** içerir.

Raspberry Pi (Debian ARM64 / ARMv7) üzerinde çok düşük kaynak (~15-25 MB RAM) tüketerek 7/24 stabil çalışacak şekilde tasarlanmıştır.

---

## ✨ Temel Özellikler

- 🔗 **5 Sosyal Medya Platformu:**
  - 🐦 **Twitter / X** ➔ `fixupx.com`
  - 🎵 **TikTok** ➔ `tnktok.com` (fxTikTok)
  - 📷 **Instagram** ➔ `ddinstagram.com`
  - 🤖 **Reddit** ➔ `vxreddit.com`
  - 🦋 **Bluesky** ➔ `fxbsky.app`
- 🧩 **Built-in (Native) Embed Motoru — Proxy'ye Alternatif:**
  - Bot bağlantıyı harici bir önizleme servisine yönlendirmek yerine gönderiyi **kendisi çözer**, Discord embed'ini kendisi üretir: yazar/ad/avatar, doğrulama işareti, gönderi metni, etkileşim metrikleri, gönderi tarihi, alıntılanan gönderi (ikinci embed), topluluk notu ve medya galerisi.
  - 🖼️ **Medya yükleme:** Görseller ve videolar (limit altındaysa) mesaja **dosya olarak** eklenir; böylece önizleme kaynak CDN'e (veya proxy servisine) bağımlı olmaz. İndirilemeyen medya otomatik olarak uzak URL'e düşer.
  - 🧱 **Kart stilleri:** WebUI'dan seçilir — `modern` (tek parça Components V2 kartı; varsayılan), `otomatik` (video içeren gönderilerde modern kart, diğerlerinde klasik embed) veya `fxembed` (her zaman klasik embed).
  - 🚫 **Profil resmi yok:** Kart, X'in aksine **avatar göstermez** (Discord Components V2 küçük resmi yalnızca sağa koyabildiği için modelin solunda küçük avatar mümkün değil); yalnızca ad, doğrulama işareti ve (varsa) profil linki yer alır.
  - 🧱 **Modern kart (Components V2):** Gönderi, Discord'un yeni bileşen düzeniyle (`Container` + `Text Display` + `Media Gallery`) **tek kart** olarak, X'in kendi embed'ine benzer şekilde gönderilir:
    - Kullanıcının linkle birlikte yazdığı metin kartın **dışında**, kartın üstünde düz mesaj metni olarak gönderilir (Components V2 bayrağı `content` alanını yasakladığı için mesaj metninin yerini üst düzey `Text Display` alır; taklit açıkken mesaj kullanıcının kendi adıyla gittiğinden tam olarak kullanıcı yazmış gibi görünür, `>` alıntı bloğuna çevrilmez).
    - `**Ad <:verified:...> ([@kullanici](profil))**`
    - gönderi metni (+ varsa alıntı)
    - `-# <:comment:...> 6  <:rt:...> 27  <:like:...> 386  <:metric:...> 12.3K`
    - `-# <:twitter:...> • <t:...:R> • [Link](gönderi)`
    - Video ve görseller **kartın içinde** oynar/görünür (klasik embed'lerde Discord bot mesajlarında `video` alanını göstermez); sol kenarda renkli çubuk kullanılmaz. Video indirilemediyse galeride **yalnızca video** gösterilir — kapak görseli ayrıca eklenmez (kapak + video yan yana iki öğe olarak çiziliyordu), oynatma bağlantısı kartın son satırında kalır.
    - Kullanılan emojiler WebUI'dan (`<:ad:id>` biçiminde) değiştirilebilir; **kimlik yazılmayı unutulursa** (`:twitter:` gibi) alan botun kendi emojisine bağlanır, boş bırakılan alan Unicode emojiye düşer — böylece kart hiçbir durumda bozuk emoji göstermez. Discord bileşenleri reddederse otomatik olarak klasik embed'e düşülür.
  - 💬 **Yanıt desteği:** Kullanıcı linki bir mesaja **yanıt olarak** attıysa bot da **aynı mesaja yanıt verir** (proxy motoruyla aynı davranış). Discord webhook mesajları yanıt olamadığı için taklit açıkken bile yanıtlarda mesaj **bot adına** gönderilir (WebUI: *“Yanıtlarda yanıtı koru”*; kapatılırsa taklit korunur, yanıt bağlantısı düşer).
  - ⚡ **Gecikme hissi yok:** `DELETE_ORIGINAL` açıkken kullanıcının mesajı **dönüşüm başında** (proxy yedeği açıksa veri çekilmeden önce) silinir; kullanıcı mesajı beklemeden kaybolur, kart hazır olunca gelir. Proxy motorunda da silme gönderimden hemen önce yapılır.
  - 🎭 **Kullanıcıyı taklit etme (webhook):** Mesaj, paylaşan kullanıcının **adı ve avatarıyla** gönderilir (`Manage Webhooks` izni gerekir). Böylece `<@kullanıcı>` etiketi ve `<link>` sarma hilesi gibi workaround'lara ihtiyaç kalmaz; mesaj sanki kullanıcı linki kendisi paylaşmış gibi görünür. Webhook kullanılamıyorsa (veya yanıt olarak atılan linklerde) mesaj bot adına gönderilir.
  - 🔁 **Alıntı medyası:** Gönderiye medya eklenmemişse **alıntılanan gönderinin videosu/görseli kartta gösterilir** (X'in kendi kartı gibi; video indirilebilirse kartın içinde oynar, değilse kapak + oynatma bağlantısı). Alıntı başlığı `qutoe` emojisiyle ve **fxembed düzeni**yle yazılır: `-# <:qutoe:...> [Quote](alıntı linki) from **Ad** ([**@kullanici**](profil))`, alıntılanan metin blok alıntı olarak altında durur.
  - 🎞️ **GIF desteği (FxEmbed'den):** X'in `animated_gif` medyası yalnızca **sessiz mp4** olarak sunulur; Discord mp4'leri oynatıcı olarak gösterdiği için GIF'ler video gibi görünürdü. Bot artık GIF'i doğrudan **FxEmbed'in GIF servisinden** çeker (`https://video.twimg.com/tweet_video/<ad>.mp4` → `https://gif.fxtwitter.com/tweet_video/<ad>.gif`; CDN'de önbellekli, `image/gif`) ve **kartın içinde GIF gibi** (döngülü, oynatıcısız) oynatır. Servis yanıt vermezse yerel `ffmpeg` dönüşümüne, o da yoksa mp4'e düşülür.
  - 🪶 **İmaj boyutu:** Docker imajlarında **`ffmpeg` kurulu değildir** (Debian'da tam paket ~75-95 MB bağımlılık çekiyor); GIF'ler zaten FxEmbed servisinden geldiği için buna gerek yok. Yerel yedeği de isterseniz: `docker build --build-arg WITH_FFMPEG=1 ...`. Sistem (deb) kurulumunda `ffmpeg` yalnızca **önerilir** (`Suggests`), zorunlu değildir.
  - 🔀 **Platform bazlı motor seçimi:** Her platform için `Proxy` veya `Native Embed` seçilebilir; native embed başarısız olursa isteğe bağlı olarak proxy linkine düşülür (`fallback_to_proxy`).
  - 🌐 **Veri kaynakları:** X için `X Syndication` (üçüncü taraf/API anahtarı yok), `FxTwitter` (görüntülenme, repost, topluluk notu) veya `Hibrit`; Reddit için herkese açık JSON, Bluesky için XRPC, TikTok için oEmbed, Instagram için OpenGraph + proxy yedeği.
  - 📊 **Eksik metrikler tamamlanır:** Syndication uç noktası repost ve görüntülenme vermez; eksik alanlar FxTwitter yanıtından doldurulur (mevcut değerler korunur).
  - ⚡ **Hızlandırılmış akış (milisaniye seviyesi):**
    - **Tek istek:** Çeviri açıkken (varsayılan) metin, X'in Türkçe çevirisi, metrikler, topluluk notu ve medya bilgisi **tek FxTwitter yanıtında** gelir — ek istek yok.
    - **Paralel istek:** Kaynak `syndication` iken X ucu ile FxTwitter **aynı anda** istenir; toplam gecikme tek istek kadardır.
    - **Paralel indirme:** Video + görseller + GIF'ler aynı anda indirilir; mesaj gecikmesi dosyaların toplamı değil en yavaş tek indirme kadardır.
    - **Önbellek:** Aynı link 10 dakika içinde tekrar paylaşılırsa ağ isteği hiç yapılmaz (anında yanıt).
    - **HTTP/2 + bağlantı havuzu:** Paralel istekler tek bağlantıda çoğullanır, TLS el sıkışması tekrarlanmaz.
    - Her dönüşümde süre log'lanır: `Hazırlık: veri X ms, medya Y ms` ve `toplam Z ms` (RUST_LOG=debug).
  - 🇹🇷 **Otomatik Türkçe çeviri:** Türkçe ve İngilizce **dışındaki** X gönderileri **X'in kendi çeviri servisiyle** otomatik Türkçeye çevrilir (embedded.gallery / fxembed ile aynı yöntem: dil ekli uç, sağlayıcı X'in çevirisi). Çevrilen gönderinin alt bilgisine `🌐 de → tr` notu eklenir; Türkçe/İngilizce gönderiler olduğu gibi bırakılır.
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
  - Tüm modülleri tek tıkla açıp kapatma.
  - **Link Dönüştürme Motoru paneli:** Platform bazlı `Proxy` / `Native Embed` seçimi, yedek proxy alan adları (`.env` değerini geçersiz kılar), X veri kaynağı, **kart stili** (`fxembed` / `otomatik` / `modern`), **otomatik Türkçe çeviri**, **kullanıcıyı taklit etme**, modern kart **emoji seti**, medya yükleme ve embed içeriği seçenekleri (metrik, tarih, topluluk notu, alıntı, maks. görsel sayısı, maks. video boyutu).
  - **Rol Tabanlı Filtreleme:** İstemeyen roller için `Blacklist` veya sadece belirli roller için `Allowlist`.
  - Sistem promptu ve AI modelini web arayüzünden anlık güncelleme.
  - Yapılan tüm değişiklikler `settings.json` ile anında kalıcı olarak kaydedilir.

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
