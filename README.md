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
  - 🧱 **Modern kart (Components V2):** Gönderi, Discord'un yeni bileşen düzeniyle (`Container` + `Text Display` + `Media Gallery`) **tek kart** olarak, X'in kendi embed'ine benzer şekilde gönderilir:
    - hesabın **profil resmi** kartın sağ üstünde küçük resim (`Section` + `Thumbnail`) olarak durur
    - `**Ad <:verified:...> ([@kullanici](profil))**`
    - gönderi metni (+ varsa alıntı/kullanıcı notu)
    - `-# <:comment:...> 6  <:rt:...> 27  <:like:...> 386  <:metric:...> 12.3K`
    - `-# <:twitter:...> • <t:...:R> • [Link](gönderi)`
    - Video ve görseller **kartın içinde** oynar/görünür (klasik embed'lerde Discord bot mesajlarında `video` alanını göstermez); sol kenarda renkli çubuk kullanılmaz.
    - Kullanılan emojiler WebUI'dan (`<:ad:id>` biçiminde) değiştirilebilir; boş bırakılan alan Unicode emojiye düşer, böylece emojinin bulunmadığı sunucularda kart bozulmaz. Discord bileşenleri reddederse otomatik olarak klasik embed'e düşülür.
  - 🎭 **Kullanıcıyı taklit etme (webhook):** Mesaj, paylaşan kullanıcının **adı ve avatarıyla** gönderilir (`Manage Webhooks` izni gerekir). Böylece `<@kullanıcı>` etiketi ve `<link>` sarma hilesi gibi workaround'lara ihtiyaç kalmaz; mesaj sanki kullanıcı linki kendisi paylaşmış gibi görünür. Webhook kullanılamıyorsa mesaj bot adına gönderilir.
  - 🔀 **Platform bazlı motor seçimi:** Her platform için `Proxy` veya `Native Embed` seçilebilir; native embed başarısız olursa isteğe bağlı olarak proxy linkine düşülür (`fallback_to_proxy`).
  - 🌐 **Veri kaynakları:** X için `X Syndication` (üçüncü taraf/API anahtarı yok), `FxTwitter` (görüntülenme, repost, topluluk notu) veya `Hibrit`; Reddit için herkese açık JSON, Bluesky için XRPC, TikTok için oEmbed, Instagram için OpenGraph + proxy yedeği.
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
- 🎛️ **Cyber-Ops Minimal WebUI (`http://localhost:3000`):**
  - Tüm modülleri tek tıkla açıp kapatma.
  - **Link Dönüştürme Motoru paneli:** Platform bazlı `Proxy` / `Native Embed` seçimi, yedek proxy alan adları (`.env` değerini geçersiz kılar), X veri kaynağı, **otomatik Türkçe çeviri**, gönderim biçimi (**Components V2 kartı**, **kullanıcıyı taklit etme**), medya yükleme ve embed içeriği seçenekleri (metrik, tarih, topluluk notu, alıntı, maks. görsel sayısı, maks. video boyutu).
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
