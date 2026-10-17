# 📓 AGENT.md — Bot İş Günlüğü

> Bu dosya **tüm bot/agent'ların ortak iş günlüğüdür.** Her bot bu repoda ne yaptıysa, yaptığını buraya kaydeder — böylece **"kim ne yaptı, ne zaman, hangi commit'te"** net biçimde görünür.
> Dosya git ile takip edilir; her kayıt, ilgili değişiklik commit'inin parçası olarak commit'lenir.

---

## 📌 Protokol (her bot bunu uygular)

Bir işi bitirip commit attıktan **sonra** aşağıdaki adımları uygula:

1. `<!-- LOG:START -->` işaretinin **hemen altına** yeni kaydını ekle (en yeni kayıt en üstte; geriye dönük geçmişe dokunmadan).
2. Kaydı kendi git kimliğiyle yaz: `git config user.name` değeri.
3. **Kayıt formatı:**

   ```
   ### [YYYY-MM-DD HH:mm] <git user.name> — <tek satır özet>
   - **Commit:** `80f274a..b26c4d6`            # birden fazlaysa: <sha1>..<sha2>
   - **Yapılan:** 1-3 madde (özet)
   - **Etkilenen:** <ana dosya / modül>   # opsiyonel
   ```

4. `agent.md` değişikliklerini iş commit'inin **aynı commit'inde** sakla:
   `git add agent.md <iş dosyaları> && git commit`
5. Yeni bir bot kimliği kullanıyorsan, yukarıdaki **Bilinen Kimlikler** tablosuna kendini de ekle.

> ⚠️ Sadece `<!-- LOG:START -->` ile `<!-- LOG:END -->` arasını düzenle.
> Aşağıdaki **Git Geçmişi** bölümü dokunulmaz geriye dönük arşivdir.

---

## 👥 Bilinen Kimlikler

| Ad | Tür | Git e-postası | Not |
| ---- | ---- | -------------- | --- |
| `eray` | insan | (repo sahibi) | Kurucu; el ile commit'ler |
| `zoltrons` | bot | `18362677+zoltrons@users.noreply.github.com` | Cline/agent; GitHub `zoltrons` hesabıyla commit atar |

> Yeni bir bot kimliği ortaya çıktığında bu tabloya satır ekle.

---

## 🕒 Güncel Faaliyet (en yeni en üstte)

<!-- LOG:START -->
### [2026-10-17 17:56] zoltrons — X veri çekimine içerik farkındalıklı paralel planlayıcı eklendi
- **Commit:** `bu-commit`
- **Yapılan:**
  1. Dil, hazır medya ve alıntı ilişkisine göre eksik görünür veriler planlanıyor; gereksiz okumalar iptal ediliyor ve kaynak/çeviri/alıntı işleri ortak 6 saniyelik bütçeyi paylaşıyor. Webhook taklidi değişmedi.
  2. Bilinen üçüncü alıntı doğrudan çekiliyor; yanıtlar ID ile birleştirilerek mevcut çeviri, medya ve notlar korunuyor. Kaynak sonuçları 600 saniye / 512 girdi ile tekrar kullanılıyor; eşzamanlı aynı istekler birleştiriliyor ve ayar kapsamları cache anahtarında ayrılıyor.
  3. Karar, iptal, hata, cache ve gecikme regresyon testleri ile tekrarlanabilir ölçüm komutları eklendi.
- **Test:** Rust 1.98.1 GNU/LLVM ile 366 Rust testi başarılı, 9 test varsayılan olarak atlandı; 15 panel testi, Clippy `-D warnings` ve format kontrolü başarılı. Canlı üçlü alıntı/medya testi geçti. Kontrollü uç simülasyonu medyanları (3 tekrar, cache'siz): İngilizce video 247 → 62 ms, üçlü görsel 499 → 140 ms, çevirili üçlü 496 → 372 ms; Japonca video 247 → 249 ms. Bunlar üretim gecikmesi garantisi değildir.
- **Etkilenen:** `src/converters/builtin/twitter.rs`, `src/converters/builtin/twitter/{plan,pipeline,source_cache,benchmarks}.rs`, `src/converters/builtin/mod.rs`, `README.md`

### [2026-10-17 16:47] zoltrons — Twitter veri fazındaki gereksiz beklemeler azaltıldı
- **Commit:** `bu-commit`
- **Yapılan:**
  1. VxTwitter hedge'i 500 ms yerine 125 ms'de başlıyor; FxTwitter'ın 500 ms davranışı korunuyor.
  2. İç içe alıntı bütçesi 700 ms'ye indirildi; ilk alıntıda medya veya mevcut iç içe alıntı varsa gereksiz ikinci istek atlanıyor.
  3. VxTwitter, syndication ve faz kırılımı debug ölçümleri açılışta ısıtılıyor/raporlanıyor.
- **Etkilenen:** `src/converters/builtin/twitter.rs`, `src/main.rs`

### [2026-10-17 16:07] zoltrons — Webhook warmup görevinin CI derleme hatası düzeltildi
- **Commit:** `bu-commit`
- **Yapılan:**
  1. `spawn_webhook_warmup` içindeki `JoinHandle` tail expression'ının yanlışlıkla `()` döndürmesine neden olan noktalı virgül kaldırıldı.
  2. Linux CI'nin `cargo test` derlemesiyle uyumlu dönüş tipi doğrulandı.
- **Etkilenen:** `src/converters/builtin/mod.rs`

### [2026-10-17 16:03] zoltrons — Discord Components V2 gönderim yolunda webhook ve HTTP/2 gecikmesi azaltıldı
- **Commit:** `bu-commit`
- **Yapılan:**
  1. Veri çekilirken başlatılan webhook warmup sonucu gerçek gönderime taşındı; thread/forum hedefi ikinci kez çözülmiyor.
  2. Discord HTTP/2 bağlantısı gövde tüketimi ve keep-alive ile yeniden kullanıma hazır tutuldu.
  3. REST payload kopyalama yalnızca webhook alanı temizlenecekse yapılıyor.
- **Etkilenen:** `src/converters/builtin/{mod,raw}.rs`, `src/main.rs`

### [2026-10-17 11:15] zoltrons — Twitter veri kaynağı vxtwitter'a geçti; çeviri yolu 6-13 sn'den <1 sn'ye
- **Commit:** `bu-commit`
- **Yapılan:**
  1. **Kök neden:** `auto_translate` açıkken kritik yol FxTwitter'a gidiyor ve çok aşamalı gecikmeler (hedge +
    topluluk notu için **sınırsız** FX beklemesi + 6 sn istek zaman aşımı) 6.6-12.8 sn kart gecikmelerine neden
    oluyordu. FX'in `/tr` çevirisi artık X'in çevirisi değil (kendi Polyglot/Grok servisi); vx API'si ise tek
    yanıtta X'in birebir Grok `translation` alanını döndürür (canlı doğrulandı: `grok_translated_post_with_availability`).
  2. **Geçiş:** `TwitterSource::Vxtwitter` eklendi; `from_vxtwitter` + `fetch_vx_fast` (FX akışıyla aynı hedge yapısı
    fakat **sınırsız not beklemesi yok** — hedge 500 ms + syndication'a 300 ms grace). `auto_translate` veya
    `twitter_source=vxtwitter` durumlarında **tek istekte** metin + medya (video poster/milisaniye-süre; GIF
    `tweet_video` mp4'ü mevcut gifconvert türetim akışını beslemeyi sürdürür) + metrikler + topluluk notu +
    X çevirisi alınıyor. Yanıt şeması tümü-`Option` toleranslı ayrıştırılır; `qrt` bir seviye gömülür, daha derin
    zincir mevcut `populate_nested_quote` ile tamamlanır; saf retweet iç içeriğe açılır. `fxtwitter` açıkça
    seçiliyse eski FX hızlı yolu; syndication+çeviri-kapalıysa eski syndication+FX birleştirme akışı korunur.
  3. **Panel/ayarlar:** panelde "vxTwitter · tek istekte metin, not ve X çevirisi (önerilen)" opsiyonu;
    `settings.json` `engine.twitter_source=vxtwitter`. 8 yeni vx birim testi (parse/medya/süre/çeviri/not/
    alıntı/retweet/tolerans). Canlı 3-seviye alıntı testi vx kaynağına taşındı ve eksik olan
    `.tls_backend_rustls()` eklenerek çalışır hâle getirildi — live test **geçiyor**.
- **Etkilenen:** `src/settings.rs`, `src/converters/builtin/twitter.rs`, `src/web/ui/index.html`, `settings.json`

### [2026-10-15 15:20] zoltrons — Çeviri notu satır içi koda alındı (saatin yanına)
- **Commit:** `bu-commit`
- **Düzeltme:** Önceki turda yanlış anlaşılıp **blok alıntı** yapılan çeviri notu, istenen biçime çevrildi: **satır içi kod** (ters tırnak) ve **saatin hemen yanında** —
  `-# <:twitter:…> • <t:…:R> • `Portekizce'den çevrildi` • [Link](…) • Gönderen: <@id>`
- **Etkilenen:** `src/converters/builtin/v2.rs` (+ test beklentisi)


### [2026-10-15 15:05] zoltrons — Çeviri notu blok alıntıya alındı
- **Commit:** `bu-commit`
- **Yapılan:** Çeviri notu (`Portekizce'den çevrildi`) artık alt bilgi satırının **hemen altında blok alıntı** olarak gösterilir (`> Portekizce'den çevrildi`); üstteki soluk satırda **platform emojisi (en solda)** ve saat kalır. Not emojisizdir, blok kendi sol şeridiyle ayrılır.
- **Test:** **314 Rust** + **13 panel** testi geçiyor; `check` · `clippy -D warnings` · `fmt --check` temiz.
- **Etkilenen:** `src/converters/builtin/v2.rs`


### [2026-10-15 14:40] zoltrons — Kart: kullanıcı adı yeniden kalın, çeviri notu emojisiz ve saatin yanında
- **Commit:** `bu-commit`
- **Yapılan:**
  1. **Üst satır (yazar/kimlik) yeniden kalın/büyük:** X'te `**Ad <:verified:> ([@kullanıcı](profil))**`, Reddit'te `**r/alt • flair • Posted by [u/kullanıcı](profil)**`. Dolgu/alt satırlar (`-# `) yalnızca metrik, çeviri notu ve link için kullanılır.
  2. **Alt bilgi düzeni:** **platform emojisi her zaman en solda**; çeviri notu **emojisiz** (`Japonca'dan çevrildi`) ve **saatin hemen yanında** (`<t:…:R> • Japonca'dan çevrildi`).
  3. Kullanılmayan `EmojiSettings::translation()` erişimcisi kaldırıldı; `translation` ayar alanı geriye dönük uyumluluk için şemada kaldı (WebUI'daki alan artık etkisiz — kaldırılması ayrı iş).
- **Test:** **314 Rust** + **13 panel** testi geçiyor; `cargo check` · `clippy -D warnings` · `fmt --check` temiz.
- **Etkilenen:** `src/converters/builtin/v2.rs`, `src/settings.rs`


### [2026-10-16 20:15] Ege Çelikçi — main push'u imaj yayınlar
- **Commit:** `458d158`
- **Yapılan:** `docker-publish` koşulu kaldırıldı: `main` push'u ve elle başlatma `latest`'i, sürüm etiketi `X.Y.Z` + `latest`'i yayınlar, PR yalnızca derler. Tek mimarili imajda her push'ta etiketsiz kalan eski sürümü `ghcr-cleanup.yml` günlük siler (`79e1dff` kota kaygısının yerine).
- **Etkilenen:** `.github/workflows/build-release.yml`

### [2026-10-16 20:00] Ege Çelikçi — İmaj elle başlatmada da yayınlanır; derleme ve imaj yalnızca arm64
- **Commit:** `d821b50..809763c`
- **Sorun:** `main`'den elle başlatılan `build-release.yml` koşusunda (`workflow_dispatch`) imaj işi atlanıyordu: `docker-publish` yalnızca `v*` etiketlerinde çalışıyordu (`79e1dff` kuralı).
- **Yapılan:**
  1. `docker-publish` artık elle başlatmada da yayınlar; daldan başlatmada `latest` taşınır, etikette `X.Y.Z` + `latest`. Sıradan `main` push'u hâlâ imaj üretmez (GHCR kotası).
  2. Derleme matrisi yalnızca `aarch64-unknown-linux-gnu`; armhf ve x86_64 kaldırıldı (kullanan yoktu). İmaj tek platform (`linux/arm64`); imaj içi `apt-get` için `setup-qemu-action` eklendi. Yalnızca x86_64'te koşabilen ikili sürüm doğrulaması kaldırıldı (test işi sürümü zaten ikiliden okur).
- **Etkilenen:** `.github/workflows/{build-release,release,ci-storage}.yml`, `Dockerfile.prebuilt`, `README.md`

### [2026-10-16 19:20] Ege Çelikçi — Crate güncellemeleri: rustls güvenlik açıkları kapandı, reqwest 0.13 ve axum 0.8
- **Commit:** `5155401..0004f1c`
- **Yapılan:**
  1. `cargo update`: `rustls` 0.23.45 → 0.23.46 (RUSTSEC-2026-0346/0347/0348/0349; CI `cargo audit` bu yüzden kırmızıydı), `tokio` 1.53.3, `zerocopy` 0.8.63.
  2. `reqwest` 0.12 → 0.13: serenity zaten 0.13 kullandığı için iki ayrı reqwest derleniyordu, artık tek. 0.13'ün `rustls` özelliği `aws-lc-rs` sağlayıcısı ve sistem kök sertifikalarıyla (`ca-certificates`, imajlarda ve `.deb` bağımlılığında var) çalışır; armv7 için `aws-lc-sys` hazır "universal" bağlamaları + cc derleyicisini kullanır (CMake/bindgen gerekmez). `use_rustls_tls()` → `tls_backend_rustls()`.
  3. `axum` 0.7 → 0.8: `Option<ConnectInfo>` artık isteğe bağlı çıkarılamadığından giriş sınırlayıcısı adresi `Option<Extension<ConnectInfo>>` ile okur.
  4. CI: kapsam raporu yüklemesi artefakt kotası dolunca işi düşürmez (`continue-on-error`).
- **Test:** 314 Rust testi geçiyor; `cargo audit --deny warnings` · `clippy -D warnings` · `fmt --check` temiz. armv7/arm64 çapraz derlemesi yerelde denenmedi, CI'da doğrulanacak.
- **Etkilenen:** `Cargo.toml`, `Cargo.lock`, `src/main.rs`, `src/converters/builtin/{safe_http,reddit_video}.rs`, `src/web/auth.rs`, `.github/workflows/quality.yml`

### [2026-10-16 18:48] Ege Çelikçi — Proje sahibinin kararları: CI cache geri açıldı, AI iç düşünce sızıntısı ve iki sekme çakışması giderildi
- **Commit:** `0a956f1..4f264fa`
- **Yapılan:**
  1. **CI cache:** önceki kayıttaki "Actions cache'i kotaya sayılıyor" varsayımı yanlıştı (cache depo başına ayrı 10 GB; 0,5 GB'lık kota artefakt/paketler için). `target/` cache'leri iş türü (`test`, `coverage`) ve sürüm hedefi (`release-<target>`) başına geri açıldı, kaydetme yalnızca ana dalda; `ci-storage.yml` bütçesi 8000 MB, erişilmeme süresi 7 gün.
  2. **AI:** içerik boşken modelin `reasoning` metni artık kanala yazılmıyor; yanıt boş sayılıp yedek modele geçiliyor.
  3. **Panel:** `GET /api/settings` içerikten türetilen `ETag` döner, kayıt `If-Match` ile gönderilir; arada başka sekme kaydettiyse sunucu `412` döner, panel güncel ayarları yükleme önerir ve yerel değişiklikleri kirli tutar.
  4. **Temizlik:** kullanılmayan `release.ps1` kaldırıldı (zaten `release.sh`'e devrediyordu).
- **Test:** 314 Rust + 15 panel testi geçiyor; `cargo fmt --check` · `clippy -D warnings` temiz.
- **Etkilenen:** `.github/workflows/{build-release,quality,release,ci-storage}.yml`, `src/ai/mod.rs`, `src/web/mod.rs`, `src/web/ui/app.js`, `tests/web-ui.test.cjs`, `release.ps1`, `README.md`

### [2026-10-16 18:31] Ege Çelikçi — Kapsamlı denetim: çökme, güvenlik, doğruluk, hız ve altyapı düzeltmeleri (her düzeltme ayrı commit)
- **Commit:** `85f4a7b..d1921bc` (68 commit)
- **Yapılan:**
  1. **Dayanıklılık ve hız:** dönüşüm kuyruğu tek tüketiciden 8 eşzamanlı işe geçti (bir yavaş iş tüm sunucuları bekletmiyor, panik yalnızca o kartı düşürüyor); sürüm profilindeki `panic = "abort"` kaldırıldı; Bluesky facet ve Spotify tarih dilimi panikleri giderildi; zehirlenen SQLite kilidi kurtarılıyor; başarısız çekimler 30 sn negatif önbellekte; bayraksız reaksiyonlar, DM'ler, AI kapalıyken yanıtlar ve bağlantısız sohbet mesajları REST/ayar çözümlemesine girmiyor.
  2. **Doğruluk:** silinmiş mesaja yanıtta AI/proxy kartı düşmüyor (`fail_if_not_exists=false`); eksi skorlu Reddit yorumu fetch'i bozmuyor; spoiler kısaltmada açılmıyor; OG değerleri kesme işaretinde kesilmiyor; HTML varlıkları tek geçişte çözülüyor; X hedge yarışında hızlı hata FxTwitter'ı düşürmüyor; Bluesky görselli alıntılar korunuyor; yeni URL biçimleri (mobile/i-web/kullanıcı önekli/büyük harf) tanınıyor; thread'lerde 🗑️ yetkisi üst kanaldan hesaplanıyor.
  3. **Güvenlik:** X link önizlemesinde sayfa kaynaklı Markdown/maskeli bağlantı enjeksiyonu kapatıldı; WebUI giriş sınırlayıcısı adres başına ve yalnızca başarısızlıkları sayıyor (yöneticiyi kilitleyen DoS giderildi); belirteç karşılaştırması uzunluk sızdırmıyor; oturum tablosu dolunca en eski oturum çıkarılıyor.
  4. **Panel:** sistem istemi karakterle kısaltılıyor; kaydet düğmesi geçici etikette takılmıyor; eşzamanlı ikinci kayıt yok sayılıyor; rol listesi hatası "yükleniyor"da kalmıyor; statik varlıklar ETag ile yeniden doğrulanıyor.
  5. **Altyapı:** `release.yml` zinciri `workflow_dispatch` ile gerçekten başlıyor; x86_64 bookworm glibc'siyle uyumlu derleniyor ve denetleniyor; ön sürüm `latest`'i taşımıyor; `.deb` kullanıcı/dizin hazırlıyor; systemd sandbox'ı sıkılaştı; `deploy-rpi.sh` çalışan kurulumu güncelleyebiliyor; Compose log döndürme + yetkisiz/salt okunur kök; yedek betikleri canlı dosyayı atomik yedekliyor; `release.sh` başarısızlıkta geri alıyor.
- **Test:** 314 Rust + 14 panel testi geçiyor; `cargo fmt --check` · `clippy -D warnings` temiz; her commit tek başına `cargo check` ile doğrulandı.
- **Bilinçli olarak yapılmadı:** hedef cache'lerinin yeniden açılması (kota kararı), distroless imaj, aksiyonların SHA'ya sabitlenmesi, healthcheck alt komutu, ayarlarda revizyon/409 çakışma denetimi, AI'ın `reasoning` alanına düşmesi (test edilmiş tasarım), `release.ps1`.
- **Etkilenen:** `src/**`, `tests/web-ui.test.cjs`, `.github/workflows/*`, `Cargo.toml`, `debian/`, `botgek.service`, `docker-compose.yml`, `Dockerfile*`, `deploy-rpi.sh`, `release.sh`, `backup-settings.*`, `.env.example`, `README.md`

### [2026-10-15 14:00] zoltrons — GHCR paket kotası: imajlar yalnızca sürüm etiketinde yayınlanır, paket geçmişi son 10 sürümde tutulur
- **Commit:** `79e1dff`
- **Sorun:** Depolama tüketiminin kaynağı Actions cache'i değil **GHCR paket sürümleriydi**: `docker-publish` her `main` push'unda çalışıyor, `docker/metadata-action` her derleme için `sha-xxxxxxx` etiketi üretiyordu; `latest`/`sha` taşındıkça eski paket sürümleri **etiketsiz** kalıyor ve hiç silinmediği için özel paket kotasını (500 MB) dolduruyordu.
- **Yapılan:**
  1. `docker-publish` artık yalnızca **sürüm etiketlerinde** (`v*`) yayınlar: PR'lar imajı yalnızca derler (push yok), `main` push'unda imaj üretilmez → `latest` = **son yayın sürümü**.
  2. `docker/metadata-action` etiketleri `latest` + `X.Y.Z` ile sınırlandı; `type=sha` **kaldırıldı** → her yayın **tek** paket sürümü olur, yeni etiketsiz artık birikmez.
  3. `ghcr-cleanup.yml`'e **yayın sürümlerini son 10'da tutan** adım eklendi (`release_keep`, varsayılan 10); `sha-*` adımı yalnızca eski birikimi temizler.
- **Uygulanan temizlik:** önce kuru koşumla doğrulandı (`78 manifest`), sonra gerçek koşu çalıştırıldı: **90 yetim etiketsiz manifest silindi**, yayın sürümlerinin hiçbiri budanmadı (pakette 10'un altında sürüm var).
- **Etkilenen:** `.github/workflows/build-release.yml`, `.github/workflows/ghcr-cleanup.yml`


### [2026-10-15 13:45] zoltrons — Minimalizasyon ve hız: ölü kod, kullanılmayan fixture, ignore sıkılaştırma, kart yolu optimizasyonu
- **Commit:** `8c74a28..a495563`
- **Yapılan:**
  1. **Minimalizasyon:** hiçbir kod/test tarafından okunmayan `builtin/fixtures/new_platform_previews.json` ve hiç çağrılmayan `Platform::supports_translation()` silindi; **yalnızca testlerden** kullanılan `parse_message()` ve `PlatformEngineModes::set()` `#[allow(dead_code)]` yerine `#[cfg(test)]`'e alındı (üretim ikilisine girmez). İzlenen dosya sayısı 85 → 84.
  2. **Ignore hijyeni:** `.gitignore` — `.env.*` tamamı yok sayılır (`!.env.example` korunur), kapsam/linker çıktıları (`*.profraw`, `*.profdata`, `lcov.info`), CI paketleme dizinleri (`/dist/`, `/bin/`), `*.tmp`, macOS `._*`, `.fleet/`, `*.swp`, `*~`. `.dockerignore` — `tests/` ve `.clinerules/` yapı bağlamından çıkarıldı (imaj bağlamı küçülür → derleme hızlanır).
  3. **Hız:** `utils::truncate_chars` bayt uzunluğu yetiyorsa hiç taramıyor ve kısaltmayı tek geçişte (`char_indices`) yapıyor; `v2::text` sınıra sığan içeriği **kopyalamıyor**; `[profile.dev] debug = "line-tables-only"` (geri izlemeler çalışır, `target/` hacmi ve link süresi küçülür). Ölçüm (`offline_card_baseline`, 10.000 örnek, 3 kolun medyanı): p50 **4041ns → 3833ns**, p95 **4584ns → 4250ns**.
  4. **Doğrulama:** 288 Rust + 13 panel testi geçiyor; `cargo check` · `clippy -D warnings` · `fmt --check` temiz. Kullanılmayan bağımlılık taraması yapıldı — `native-tls` **bilinçli** bırakıldı (serenity TLS yedeği + vendored OpenSSL ile çapraz derleme bağımsızlığı).
  5. **Not:** paralel yürüyen GHCR paket temizliği işleri (`1ac1c25`, `1ad41f0`) korunarak bu commit'lerin üzerine eklendi; onların dosyalarına dokunulmadı.
  6. **CI cache paylaşımı:** hedef cache'leri kapandıktan sonra bile her iş kendi ~400 MB'lık kayıt defteri cache'ini üretiyordu (5 iş ≈ 2 GB). `shared-key: "rust-registry"` + `add-job-id-key: "false"` ile tüm işler **tek** cache paylaşır; matris işindeki `key: ${{ matrix.target }}` kaldırıldı.
- **Etkilenen:** `src/utils/mod.rs`, `src/converters/builtin/v2.rs`, `src/converters/parser.rs`, `src/settings.rs`, `Cargo.toml`, `.gitignore`, `.dockerignore`


### [2026-10-15 13:15] zoltrons — Kartlar yerleşik Reddit/X düzenine hizalandı; WebUI kayıt hatası, topluluk notu ve Actions depolama sorunu çözüldü
- **Commit:** `b5ae905..f88c784`
- **Yapılan:**
  1. **Kart düzeni (referans görseller):** üst satır **tüm platformlarda** soluk/küçük (`-# `) — X'te `Ad <:verified:> ([@kullanıcı](profil))`, Reddit'te `r/alt • flair • Posted by [u/kullanıcı](profil)` (profil bağlantısı yeni). Reddit metrikleri `⬆ N upvotes • 💬 M comments` (aşağı oy kaldırıldı), topluluk etiketi alt bilgiden üst satıra taşındı, harici bağlantı **tek satır** (`Site | Başlık`, OG 2,5 sn bütçe, alınamazsa alan adı), çeviri notu Türkçe (`🌐 Japonca'dan çevrildi`; `translate.rs`'e kod → Türkçe dil adı tablosu + ünlü uyumlu ek). Sağ üstte profil resmi/görsel yuvası **yok**; özel emojiler, `Gönderen:` ve `[Link]` korunur.
  2. **Hata düzeltmeleri:** (a) WebUI'da bir bölüm ilk kez sabitlenirken rol listesi düzenlemesi kayboluyordu (`ensureBlockOverridden` sabitlemeden önce `getActiveConfig()` çağırıyordu) ve elle girilen rol kimlikleri sunucu kuralıyla uyuşmadığı için kaydetmede sessizce düşüyordu; (b) topluluk notunda ve proxy alıntısında **çıplak `>`** satırı kartta fazladan işaret olarak görünüyordu (2026-09-27 kararının ihlali); (c) `store::init()`'in fail-fast sözleşmesi erişilemezdi: kalıcı DB açılamazsa panik/abort oluyordu, artık hata startup'a iletilir; (d) preview'da `_` kaçışı ve `plan_media`/`spotify` içindeki ölü kod temizlendi.
  3. **CI depolama:** Actions kotası dolmuştu (**113 cache ≈ 8,2 GB**, **219 artefakt ≈ 1,7 GB**). Hedef cache'leri kapatıldı (`cache-targets: "false"`), `save-if` ile cache üretimi ana dal/etiketle sınırlandı, artefakt saklama 30 → **1 gün**, buildx `type=gha,mode=max` kaldırıldı ve **günlük** `ci-storage.yml` (artefakt/cache budama, 300 MB bütçe) eklendi; mevcut birikim REST API ile silindi.
- **Test:** **288 Rust testi** + **13 panel testi** geçiyor; `cargo check` · `clippy -D warnings` · `fmt --check` temiz.
- **Etkilenen:** `src/converters/builtin/{v2,reddit,translate,preview,render,spotify}.rs`, `src/converters/mod.rs`, `src/store.rs`, `src/web/ui/app.js`, `tests/web-ui.test.cjs`, `.github/workflows/*`


### [2026-10-09 19:20] zoltrons — X "link kartı": medyasız gönderide harici bağlantı önizlemesi karta eklendi
- **Commit:** `5a35305`
- **Sorun:** X, metninde harici bağlantı olan **medyasız** gönderilerde hedef sayfanın önizleme kartını (başlık + açıklama + görsel + site adı) gösterir; botun kartı bunu göstermiyordu.
- **Yapılan:**
  1. **Model:** `BuiltinPost.link_preview: Option<LinkPreview>` (`mod.rs`); `LinkPreview` = `url`, `title`, `description`, `image`, `site_name` + `is_empty()` (üç alan da boşsa karta hiç eklenmez).
  2. **Twitter (`twitter.rs`):** `attach_link_preview()` — yalnızca **medyasız ve alıntısız** gönderilerde çalışır; metindeki ilk **harici** adres seçilir (X/x.com/t.co/pic.twitter.com/twimg.com atlanır) ve sayfa `get_text` ile çekilip mevcut `opengraph::parse_open_graph` ile çözümlenir. `LINK_PREVIEW_TIMEOUT = 2.5 sn`: sayfa yanıt vermezse/hata verirse kart önizlemesiz gönderilir (kart asla bekletilmez). `site_name` yoksa alan adına düşülür (`www.` atılır). Başlık 180, açıklama 400 karaktere kesilir.
  3. **Kart:** başlık bağlantılı satır (`**🔗 [Başlık](url)**`), açıklama alıntı bloğu (`> satır`), site adı alt bilgi (`-# example.com`); **önizleme görseli medya galerisinde büyük olarak** gösterilir (indirilmez — `og:image` adresi doğrudan bağlanır, `max_images` ve spoiler kuralları geçerli). Gönderi kendi medyasını taşıyorsa önizleme görseli plana alınmaz.
  4. **Testler:** `external_link` (X adreslerini atlar), `host_label`, `meta_value`; `plan_media` (yalnızca medyasız gönderide görsel) ve kart bloğu/boş önizleme testleri — **7 yeni test, 230 test geçiyor** (uyarısız). Test botu yeni sürümle yeniden başlatıldı.
- **Etkilenen:** `src/converters/builtin/{mod,twitter,render,v2}.rs`


### [2026-10-09 22:30] zoltrons — Mikro etiketlerde all caps kaldırıldı (sentence case) — kullanıcı kararı: orta yol
- **Commit:** `ba81aae` (bu dal)
- **Karar:** Arc'ın "sentence case, all caps yok" satırı için **orta yol** seçildi: yalnızca mikro etiketler sentence case oldu; **display başlıklar** (`.sheet-h`, `.modal-title`), **display ağırlıkları** (700/800) ve **dekoratif ızgara/gren** korundu (panel kimliği).
- **Yapılan:** 14 kuraldan `text-transform: uppercase` kaldırıldı (`.micro`, `.chip`, `.telemetry .mono`, `.dirty-line`, `.sheet-meta`, `.mod-state`, `.studio-badge`, `.pill span`, `.field-label`, `.engine-head`, `.engine-note`, `.manual-head`, `.manual-item dt`, `.foot`). Büyük harf için ayarlanmış harf aralıkları sentence case'e göre daraltıldı (`.micro` 0.16→0.06em, `.field-label` 0.16→0.06em, `.engine-head`/`.manual-item dt`/`.manual-head`/`.engine-note` 0.14→0.05em, `.chip` 0.1→0.04em, `.pill span` 0.08→0.03em, `.foot` 0.12→0.04em): geniş aralık küçük harfte dağılmış görünüyordu. HTML'deki literal `AKTİF` (8 modül durumu) → `Aktif`; JS zaten `Aktif`/`Devre dışı` yazıyor.
- **Denetim:** Etiket metin kaynaklarının tamamı (`.micro`, `.field-label`, `.pill span`, `.engine-head`, `.manual-item dt`, `.foot`, `.chip`, `.studio-badge`, `.sheet-meta`) HTML ve JS'te zaten sentence case idi; tek düzeltme `AKTİF` oldu. Kalan `text-transform: uppercase` yalnızca 2 display başlıkta.
- **Test:** `cargo test` → **223 geçti**; `app.css` süslü parantez dengesi 269/269; `AKTİF` kalıntısı 0.
- **Etkilenen:** `src/web/ui/app.css`, `src/web/ui/index.html`

### [2026-10-09 22:00] zoltrons — Arc MCP kuralları panele uygulandı (erişilebilirlik, dokunmatik hedefler, token hijyeni)
- **Commit:** `9430dd4` (bu dal)
- **Yapılan:**
  1. **Kapsam sınırı:** Arc MCP'nin bileşenleri React + shadcn CLI + Tailwind + Motion istiyor; panel Rust binary'ye gömülü **vanilla** HTML/CSS/JS. Bu yüzden Arc **bileşenleri** kurulmadı (React'e geçiş = yeni toolchain + bağımlılıklar, onay gerektirir); Arc'ın *design / accessibility / responsive / copy / checklist* kuralları mevcut panele taşındı.
  2. **Erişilebilirlik:** sayfaya tek `h1` (görsel gizli); rol seçici gerçek `combobox` + `listbox` (`role=option` seçenek id'leri, arama alanında `aria-expanded`/`aria-controls`/`aria-activedescendant`, boş sonuçta bayat adres temizliği); onay penceresinde **odak tuzağı** ve kapanınca **odağın tetikleyiciye dönmesi**; ayar anahtarlarının başlık + açıklamayı `aria-labelledby`/`aria-describedby` ile işaretlemesi (id'ler JS'te üretilir, `aria-label` JS'siz yedek kalır); sol indekste aktif bölüm `aria-current`; kaydetme sırasında bölge `aria-busy`.
  3. **Durum ve etkileşim:** kaydetme başarısı birincil düğmede **yerinde** onaylanır ("Kaydedildi", 2,4 sn), bildirim artık tek kanıt değil; dokunmatikte (`pointer: coarse`) hedefler en az 44px (satır içi rol çipi 32px), anahtarın dokunma alanı görsel değişmeden büyür.
  4. **Token hijyeni:** `--paper-hover`, `--rule-hover`, `--ink-placeholder`, `--signal-*`, `--live-*`, `--alert-*`, `--grid-*`, `--scrim`, `--shadow-*` eklendi; **`:root` ve SVG data-URI dışında ham renk kalmadı (0)**. Süs temizliği: işaretli anahtardaki 3px renkli halo kaldırıldı, prose'daki em dash'ler temizlendi, modül satırındaki düğme fiil + nesne oldu ("filtreye git") ve modüle özel `aria-label` aldı.
  5. **Bilinçli olarak değiştirilmedi (kimlik kararı kullanıcıya bırakıldı):** `text-transform: uppercase` mikro etiketler (Arc: eyebrow/overline ve all caps yok), 700/800 display ağırlıkları (Arc: yalnızca 400/500) ve dekoratif zemin ızgarası/gren. Bunlar panelin "operasyon sayfası" kimliğini kuruyor ve Arc'ın sakin-nötr çubuğuyla çakışıyor.
- **Test:** `cargo test` → **223 geçti** (sözleşme + markup dengesi); `node --check` temiz; yeni JS erişilebilirlik harness'ı **18/18** (etiket bağlama, odak tuzağı, `aria-activedescendant`, `aria-current`, yerinde kayıt onayı).
- **Etkilenen:** `src/web/ui/index.html`, `src/web/ui/app.css`, `src/web/ui/app.js`

### [2026-10-09 20:15] zoltrons — Panel HTML dengesizliği düzeltildi (medya bölümünün dağılması)
- **Commit:** `eb92ff7` (bu dal) · `0b7faba` (`feat/versioning-system`, cherry-pick)
- **Yapılan:**
  1. **Kök neden:** `19fbd92` "medya yöntemi" seçimini kaldırırken etrafındaki `<div class="tile tile--static">` + `<div class="tile-body">` sarmalayıcıları ve tek `</div>` yerinde kaldı (**3 açılış / 1 kapanış**). Tarayıcının hata düzeltmesi, bölümün geri kalanını bir sonraki `</section>`'a kadar o kapsayıcıya hapsediyordu; `tile--static` satır yönlü bir flex olduğu için medya ızgarası, kaydırıcılar ve emoji alanları dar sütunlara sıkışıyordu (kullanıcının ekran görüntüsü).
  2. **Düzeltme:** artakalan sarmalayıcı kaldırıldı — "Proxy yedeği" artık doğrudan `.tile-grid` çocuğu; ölü `.tile--static` CSS kuralı silindi.
  3. **Regresyon testi:** yeni `panel_markup_is_balanced_and_nested_as_expected` panel HTML'ini yığın tabanlı doğrular: tüm etiketler dengeli ve doğru iç içe, üç ızgara (`tile-grid`/`range-grid`/`emoji-grid`) bir `tile` içine hapsolmamış, `data-engine-switch="fallback_to_proxy"` anahtarının **en yakın `div` atası** medya ızgarası. Test, düzeltme öncesi **başarısız** olarak hatayı birebir yakaladı (`<div> beklenirken </section>`).
- **Test:** `cargo test` → **223 test geçti** (düzeltme öncesi 222); düzeltme `feat/versioning-system` dalına da cherry-pick edildi (aynı içerik, çakışmasız birleşir).
- **Etkilenen:** `src/web/ui/index.html`, `src/web/ui/app.css`, `src/web/mod.rs`


### [2026-10-09 20:08] zoltrons — Sürüm sistemi: git etiketinden türetilen künye + `release.sh` + sürümlü paketler
- **Commit:** `cbe8a94..5ac1bdb`
- **Yapılan:**
  1. **Sürüm artık sabit değil:** "bot hep v0.1.0'de" durumu bitti. Sürüm **git etiketlerinden** türetilip **derleme zamanında ikiliye gömülür** (`build.rs` → `cargo:rustc-env`; dış bağımlılık yok, UTC tarih için `civil_from_days`, `SOURCE_DATE_EPOCH` ile tekrarlanabilir). Biçim: etiketin tam üstünde `0.2.0` (`is_release`), sonrasında `0.2.0+5.g8fc2692`, etiketsiz `0.2.0+dev.g8fc2692`, kirli ağaçta `.dirty`, git yoksa `+local`. Türetme mantığı `build-support/version-parse.rs` içinde **tek kaynak**: aynı dosyayı `build.rs` ve `src/version.rs` `include!` eder, "gömülen sürüm == çalışma zamanı sürümü" testi bunu kilitler.
  2. **Görünürlük + doğrulama:** yeni `botgek --version` (ve `--help`) — bot yapılandırmasından önce çalışır, saf `parse_cli_arg` fonksiyonu testlidir. Açılış logunda künye, `/api/info`'da `full_version/display_version/is_release/is_dirty/git_sha/git_tag/commits_since_tag/build_date/target`, panel rayında `v0.2.0` (yayın, yeşil) / `v0.2.0+5` (geliştirme, turuncu) + `title` içinde sha/tarih/hedef. Sürüm, `User-Agent` ve Discord UA'ları da buradan besleniyor.
  3. **Yayın akışı:** `release.sh` (tek kaynak) sürümü bump'lar, `Cargo.lock`'u senkronlar, testleri çalıştırır, `chore(release): vX.Y.Z` commit'i + **annotated tag** atar (`--push` ile yayın); `release.ps1` yalnızca Git-for-Windows bash'ine devreder. Yeni `release.yml` (elle tetiklenen workflow) aynı script'i çağırır; `build-release.yml` artık **sürüm künyesini derlenmiş ikiliden** okur (test job'ı çıktısı), `fetch-depth: 0` + `BOTGEK_BUILD_VERSION`/`BOTGEK_GIT_SHA` gömülür, x86_64 derlemesinde `--version` etiketle doğrulanır ve `.deb`/`.tar.gz` adları sürümü taşır. Kaynak Docker derlemesi için `BOTGEK_VERSION`/`BOTGEK_SHA` build arg'ları eklendi.
- **Test:** **222 test geçti** (206 → 222: 14 sürüm türetme/tutarlılık + 2 CLI testi); `cargo check --release` temiz; `node --check` temiz; `app.js` sürüm rozeti Node+`vm` harness'ıyla 11/11 doğrulandı. `release.sh` gerçek bir klonda uçtan uca denendi: `patch`/`minor` bump + `Cargo.toml`/`Cargo.lock` senkronu + commit + annotated tag + `--push` (izole bare repo) ✓ — ayrıca `awk -v next=…` gawk rezerve kelime hatası bu testte yakalanıp `newver` olarak düzeltildi.
- **Etkilenen:** `build.rs`, `build-support/version-parse.rs`, `src/version.rs`, `src/main.rs`, `src/converters/builtin/mod.rs`, `src/web/mod.rs`, `src/web/ui/{app.js,app.css}`, `release.sh`, `release.ps1`, `.github/workflows/{build-release,release}.yml`, `Dockerfile`, `docker-compose.yml`, `README.md`, `backup-settings.sh` (exec bit)


### [2026-10-09 19:45] zoltrons — Sunucu ayarları bölüm bazlı sparse override'a geçti (tek çözümleme + göç + miras/özel UI)
- **Commit:** `8fc2692..255deec`
- **Yapılan:**
  1. **Tek çözümleme:** saf `resolve_settings(global, override)` fonksiyonu; `get_guild_settings` yalnızca onu çağırır (bot/converters tarafı **hiç değişmedi**), panelde aynı semantik `resolveSettings`. Sunucu kaydı artık **bölüm bazlı sparse** (`GuildOverride`: modules / role_filters / ai / engine): yalnızca bilerek değiştirilen bölümler saklanır, diğerleri genelden miras alır — genel ayar sonradan değişince miras alan bölümler otomatik güncellenir (eski davranış: tek alan değişince tüm ayarların tam kopyası yazılıyordu ve genel değişimi sunucuya yansımıyordu).
  2. **Göç + yedek:** eski tam kopya kayıtlar yükleme anında **bir kez** indirgenir (`migrate_legacy_guild_overrides`); öncesinde `settings.json.bak-<zaman>` yedeği alınır ve `schema_version` damgalanır (damgalanmasa göç her açılışta tekrarlar ve bilerek sabitlenen bir bölüm, o an genelle aynı olduğu için sessizce mirasa dönebilirdi). Elle yedek için `backup-settings.sh` / `backup-settings.ps1` (göç öncesi durum fotoğrafı da basar).
  3. **UI:** her bölüm başlığında `miras/özel` rozeti + "genele sıfırla", rayda `N/4 bölüm özel` rozeti; kaydetme sunucuya **yalnızca sabitlenen bölümleri** gönderir. Ayrıca 5 panel tutarsızlığı düzeltildi: native-only satır etiketi (proxy değil native), ölü medya/kart stili notları, "yanıtlarda yanıtı koru" bağımlılığı (taklit kapalıyken devre dışı), "proxy yedeği" kapsam metni, rol seçici etiketi.
- **Test:** **206 test geçti** (12 yeni: resolve + göç + sparse serialize; 194 → 206); `node --check app.js` temiz; panel sözleşme testi sparse çözümleme izini de doğrular.
- **Etkilenen:** `src/settings.rs`, `src/web/mod.rs`, `src/web/ui/{app.js,index.html,app.css}`, `README.md`, `settings.json`, `backup-settings.sh`, `backup-settings.ps1`


### [2026-10-09 18:45] zoltrons — "Gönderen: @kullanıcı" etiketi kartın **meta satırına** taşındı
- **Commit:** `cd4929a`
- **Yapılan:**
  1. **Sorun:** Kullanıcı bir karta yanıt verdiğinde taklit (webhook) yapılamadığı için kart bot adına gidiyordu ve paylaşanı göstermek için kartın **üstünde** ayrı bir `<@id>` metin bileşeni ekleniyordu.
  2. **Değişiklik:** Üstteki etiket bileşeni kaldırıldı; paylaşan artık kartın **sağ altında**, platform emojisi/tarih/link ile **aynı meta satırında** `Gönderen: <@kullanıcı>` olarak görünüyor. `card_allowed_mentions` sayesinde etiket **ping atmaz**, düz `@kullanıcı` olarak görünür. Taklit (webhook) açıkken etiket hiç eklenmez.
  3. **Test:** `sender_tag_is_shown_in_the_meta_line_only_for_bot_messages` — üst satırda yalnızca kullanıcı notu kalır, meta satırı `Gönderen: <@42>` ile biter, platform emojisi ve tarih aynı satırdadır; taklitte etiket yok. **194 test geçti**; test botu yeniden başlatıldı.
- **Etkilenen:** `src/converters/builtin/v2.rs`


### [2026-10-09 18:35] zoltrons — Medyada **tek mod: doğrudan**; medya yöntemi/attach ayarları ve GIF-ffmpeg hattı kaldırıldı
- **Commit:** `19fbd92`
- **Yapılan:**
  1. **Ayarlar kaldırıldı:** `MediaDelivery` enum'ı ile `media_delivery`, `attach_images`, `attach_videos` alanları silindi (settings şeması, `settings.json`, WebUI'daki "Medya yöntemi" seçimi ve iki anahtar, `app.js` varsayılanları/etiketleri).
  2. **render.rs tek yola indirildi:** hiçbir medya indirilmez — görseller CDN'in hafif varyantıyla, GIF'ler vxTwitter gifconvert bağlantısıyla kartta gösterilir. **Tek istisna** sesi ayrı akışta gelen Reddit videosudur: `VIDEO_MERGE_BUDGET` (5 sn) içinde video+ses birleştirilir, sonuç mesaja ek olarak gönderilir; bütçe dolarsa video kartta uzak bağlantıyla (sessiz) oynar. `MediaPlan.images`, `image_budget`/`gif_budget` ve auto/attach bütçeleri kaldırıldı.
  3. **Ölü hat temizliği:** GIF yarışı + ffmpeg dönüşümü (`download_gif`, `to_gif`, `convert`, `run_ffmpeg`, `is_animated_image`, `replace_extension`, ilgili sabitler) silindi; `render.rs` testleri yeni davranışa göre yeniden yazıldı (plan/görsel sınırı/GIF/doğrudan mod/kota testleri).
  4. **194 test geçti** (uyarı yok); test botu yeni sürümle yeniden başlatıldı ve `/api/settings` üzerinden doğrulandı (`media_delivery` alanı artık yok).
- **Etkilenen:** `src/settings.rs`, `src/converters/builtin/{render,media,mod,twitter,v2}.rs`, `src/web/ui/{index.html,app.js}`, `settings.json`


### [2026-10-09 18:07] zoltrons — Reddit galeri görsellerinin **açıklamaları** karta eklendi (caption hatası)
- **Commit:** `e0819c3`
- **Yapılan:**
  1. **Kök neden:** Reddit galerilerinde her görselin üstüne açıklama yazılabiliyor (`gallery_data.items[*].caption` / `media_metadata[*].caption`) ama dönüştürücü bu alanları **hiç okumuyordu**; V2 kartın galeri bileşeni de yalnızca görsel taşıdığı için açıklamalar kartta görünmüyordu.
  2. **Düzeltme:** `gallery_captions()` eklendi (görsel sırasıyla, önce `gallery_data.items[i].caption`, yoksa `media_metadata[id].caption`) ve açıklamalar gönderi metnine **numaralı** olarak ekleniyor (`**Görsel açıklamaları**` bloğu). Açıklama yoksa metin aynen kalır (`gallery_without_captions_keeps_text_clean`).
  3. **204 test geçti** (yeni: `gallery_captions_are_appended_to_the_card_text`, `gallery_without_captions_keeps_text_clean`).
- **Etkilenen:** `src/converters/builtin/reddit.rs`


### [2026-10-09 18:03] zoltrons — Kartlar **yalnızca Components V2**: klasik embed ve fallback'leri koddan kaldırıldı
- **Commit:** `7479768`
- **Yapılan:**
  1. **Kart stili ayarı kaldırıldı** (`CardStyle`): kart **her zaman** modern V2 kartıdır. WebUI'daki "Kart stili" seçimi, `settings.json` alanı, istemci varsayılanları ve `use_modern_card` mantığı silindi.
  2. **Klasik embed üretimi silindi:** `render::build_embeds`/`build_main_embed`/`build_quote_embed`/`footer_text`/`stats_line`/`EMBED_*` limitleri/`CARD_SIGNATURE`/`platform_color` ve ilgili testler. **V2 gönderimi başarısız olursa klasik embed'e düşme yok** (`V2_UNAVAILABLE` kaldırıldı): hata loglanır ve dönülür. Klasik gönderim yolları (`send_classic`/`classic_payload`/`build_content`) ve testleri silindi.
  3. **Kart imzası** artık yalnızca yapısal kontrol ("içeriksiz + embedsiz mesaj"); eski embed alt bilgisi (`Platform • Botgek`) tanıması kaldırıldı. Ölü kod temizliği: `ResolvedMedia.quote_media`, `video_poster`/`poster_url`, kullanılmayan importlar; `Author::label` ve `Media::is_image` test-only yardımcılara dönüştü.
  4. **202 test geçti** (uyarı yok); test botu yeni sürümle yeniden başlatıldı ve GitHub'a push edildi.
- **Etkilenen:** `src/converters/builtin/{mod,render,v2}.rs`, `src/settings.rs`, `src/handler.rs`, `src/web/mod.rs`, `src/web/ui/{index.html,app.js}`, `settings.json`


### [2026-10-09 17:44] zoltrons — Kart/kullanıcı adı: `_` kaçışı kaldırıldı (görünen ters bölü) + GitHub'a push
- **Commit:** `0e703ea`
- **Yapılan:**
  1. **Kök neden:** `render::escape_markdown` `_` karakterini de kaçırıyordu; Discord (CommonMark) **kelime içi** alt çizgileri vurgu başlatıcı saymadığı için kaçış gereksizdi ve kart kullanıcı adında olmayan bir `\` gösteriyordu (`world\_record\_egg`).
  2. **Düzeltme:** `_` kaçışı kaldırıldı; biçimi gerçekten bozabilecek yıldız, tilde, ters tırnak, dikey çizgi, büyüktür, köşeli parantez ve ters bölü kaçışları korunuyor. Regresyon testleri: `escape_markdown` (underscore temiz, diğerleri hâlâ kaçırılır) + `underscored_usernames_stay_clean_in_the_author_line` (V2 yazar satırı).
  3. **217 test geçti**; test botu düzeltmeyle yeniden derlenip başlatıldı ve tüm yerel commit'ler GitHub `zoltrons/botgek` `main` dalına push edildi.
- **Etkilenen:** `src/converters/builtin/{render,v2}.rs`, `agent.md`

### [2026-10-09 17:41] zoltrons — Reddit: `direct` modda da **sesli video** (kritik hata) + Reddit emojisi + V2 "Videoyu aç" kaldırıldı
- **Commit:** `af402cc`
- **Yapılan:**
  1. **Kritik hata ("ses yok"):** `render::resolve_media` ışık hızı modunda `direct_media(plan)` ile **erken dönüyordu**; `attach_videos`/`video_requires_download` hiç değerlendirilmediği için Reddit videosu indirilmiyor ve kart sessiz kalıyordu (log: "medya 0 ms"). `RenderOptions::skips_all_downloads()` eklendi: yalnızca platform videosu zorunlu olan Reddit video indirme yoluna giriyor, **görseller bağlantı olarak kalıyor**. Yeni canlı test: `direct` modda gerçek `v.redd.it` videosu indirilip sesiyle birleştirildi, ekte `ftyp` + `mp4a` + `soun` doğrulandı.
  2. **Reddit emojisi:** `EmojiSettings.reddit` (varsayılan `<:reddit:1558112156195495956>`), `reddit()` erişimcisi, `v2::meta_line` Reddit dalı, WebUI "Reddit logosu" kutusu, `app.js` varsayılanları ve kök `settings.json` değeri.
  3. **"Videoyu aç" kaldırıldı:** V2 kartın son satırından ve klasik mesaj gövdesinden çıkarıldı (video galeride/kart içinde oynar; gönderi bağlantısı yeterli).
  4. **Eski `settings.json` uyumluluğu — migration gerekmez:** kaldırılan `twitter/instagram/reddit` motor modu ve yedek alan adları ile `emojis.reddit` yokken eski dosya hatasız yükleniyor (bilinmeyen anahtarlar yok sayılır, eksik yeni alan varsayılanını alır); kanıt testi: `legacy_settings_with_removed_engine_keys_load_without_migration`.
- **Etkilenen:** `src/converters/builtin/{render,v2,mod}.rs`, `src/settings.rs`, `src/web/ui/{index.html,app.js}`, `settings.json`


### [2026-10-09 17:31] zoltrons — Twitter/X, Instagram ve Reddit **native-only**; Reddit videosu her modda sesli birleşir (proxy kaldırıldı)
- **Commit:** `11813a6`
- **Yapılan:**
  1. **Proxy motoru kaldırıldı:** `NATIVE_ONLY_PLATFORMS = ["twitter", "instagram", "reddit"]` eklendi; `EngineSettings::mode_for` bu platformlarda daima `Builtin` döner ve `allows_proxy_fallback` proxy yedeğini yok sayar. `PlatformEngineModes`/`ProxyDomains`'ten bu üç platformun alanları, `Config`'ten `TWITTER_DOMAIN`/`INSTAGRAM_DOMAIN`/`REDDIT_DOMAIN`, WebUI'dan mod/yedek alan adı kontrolleri kaldırıldı. Parser artık native-only platformlarda proxy linki üretmez (`fixed_url` = `original_url`).
  2. **Reddit videosu:** `RenderOptions::from_engine(engine, platform_key)` eklendi. Reddit'te **video her zaman indirilip sesiyle birleştirilir** (geniş `ATTACH_VIDEO_BUDGET` bütçesi gerekir; `auto`'nun 1,2 sn'si birleştirmeye yetmez) — `direct` modda bile; **görseller ise medya yöntemi ayarını izler** (direct'te indirilmez, bağlantı olarak kalır). Log satırı görsel/video indirme durumunu ayrı yazar.
  3. **Doğrulama:** **213 test geçti** (yeni: `reddit_always_downloads_video_but_keeps_images_direct`, native-only mod/yedek ve parser beklentileri). Panel sözleşme testleri yeni şemaya uyarlandı (`data-domain-key`/`engine-mode-*` artık 3 platform, `env_domains` 3). Test botu yeni sürümle ve `media_delivery: direct` ile yeniden başlatıldı (Reddit: görsel bağlantı, video sesli ek).
- **Etkilenen:** `src/settings.rs`, `src/config.rs`, `src/converters/parser.rs`, `src/converters/mod.rs`, `src/converters/builtin/{render,mod,twitter,v2}.rs`, `src/handler.rs`, `src/web/mod.rs`, `src/web/ui/{index.html,app.js,app.css}`, `.env.example`, `settings.json`, `README.md`


### [2026-10-09 17:15] zoltrons — Reddit sesli video: `hls-transmux` çok-izli oturuma geçiş + `waiter`/`end_input` düzeltmeleri
- **Commit:** `ad16793`
- **Yapılan:**
  1. **Crate geçişi:** `hls-engine` 1.0.1 → **`hls-transmux` 0.10** (aynı motorun çok-izli oturum API'si: `MultiTrackInputs::with_audio` + `MultiTrackSession`). `hls-transmux`'ın *eski tek-girdili* kolaylık girişi (`transmux_hls_to_mp4_bytes` + `VariantSelection`) `EXT-X-MEDIA TYPE=AUDIO` rendition'ını çözmez — çok-izli oturum aynı pakette vardır. `default-features = false` + kendi `Source`'umuz ⇒ motorun isteğe bağlı `reqwest 0.13` bağımlılığı devre dışı kalır (botgek `reqwest 0.12`'de kalır).
  2. **İki kök neden (kaynak koddan kanıtlı):** (a) `ContinuousOptions::validate(inputs)` çok girdide (`inputs > 1 && waiter.is_none()`) **`waiter` zorunlu** kılıyordu ⇒ `InvalidOptions`; 30 sn'lik host waiter (`tokio::time::sleep`; motor future'ları `!Send`) eklendi. (b) Sonlu (VOD/ENDLIST) girdilerde `accept_snapshot` sonrası **`end_input`** çağrılmazsa oturum hiç tamamlanmıyordu ⇒ video ve ses için EOF bildirimi eklendi. Video girdisi `EmbeddedAudio::Exclude`, ses girdisi `TrackMetadata::new("und", "audio")`.
  3. **Doğrulama ve kart davranışı:** `cargo test` → **212 test geçti**; canlı birleşim (`ac17c5qqrcuh1`) → **5.735.062 bayt** tek MP4 (`ftyp` + `mp4a` + `soun`). Motorun istediği bayt aralıkları Reddit playlist'iyle birebir doğrulandı (`#EXT-X-BYTERANGE:65066@949`). Discord'da sesin gelmemesinin nedeni motor değil **ayardı**: `media_delivery = direct` (varsayılan) hiç indirme yapmadığı için kart içi video sessizdir; `attach` modunda birleştirme devrededir (video bütçesi 5 sn) — `auto`'nun 1,2 sn bütçesi büyük videolarda yetmeyebilir. Test botu (`local-test/settings.json` → `media_delivery: attach`) yeniden başlatıldı, sesli oynatma Discord'da doğrulandı.
- **Etkilenen:** `src/converters/builtin/reddit_video.rs`, `Cargo.toml`, `Cargo.lock`, `README.md`, `agent.md`


### [2026-10-09 17:07] zoltrons — Reddit videosu **sesli**: saf Rust `hls-transmux` ile video+ses birleşimi (ffmpeg yok)
- **Commit:** `93e890d`
- **Yapılan:**
  1. **Araştırma:** Reddit video ve sesi ayrı akışlarda tutar ve `v.redd.it` HLS master playlist'i (`HLSPlaylist.m3u8`) sesi **ayrı rendition** olarak verir (`#EXT-X-MEDIA:TYPE=AUDIO`); karşılık gelen dosyalar (`CMAF_{h}.mp4` video-only, `CMAF_AUDIO_*` mp4 **ses-only**) kimliksiz olarak **doğrudan indirilebiliyor** (403 yok) — yani birleştirme tamamen yerel, ek proxy gerekmez. Crate seçimi: `hls-transmux` 0.10'un **eski tek-girdili** kolaylık girişi (`transmux_hls_to_mp4_bytes`) `EXT-X-MEDIA TYPE=AUDIO` rendition'ını çözmez (`Unsupported`); **çok-izli oturum API'si** (`MultiTrackInputs::with_audio` + `MultiTrackSession`) aynı pakette bulunur ve `default-features = false` ile kendi `Source`'umuz verilebilir (ikinci bir reqwest sürümü gelmez).
  2. **Uygulama:** `reddit_video.rs` eklendi — master playlist çözümleme (ses gruplarının `AUDIO="grup"` eşleşmesi), varyant merdiveni (yüksek → orta → düşük; ek limitine sığan ilk sonuç), kendi `reqwest` `Source`'umuz ve `EngineInputs::new(video, Exclude).with_audio(audio, …)` ile tek MP4 üretimi. Motorun gelecekleri **`Send` değil**, bu yüzden birleştirme kendi current-thread runtime'ı olan ayrı bir iş parçacığında çalışıp sonucu kanalla taşır. `media::download_video` v.redd.it bağlantılarında önce birleşimi dener, olmazsa video-only mp4'e düşer (kart sessiz oynar; akış bozulmaz).
  3. **Engel ve çözüm (kanıtlı):** Motor `MediaValidation` ile ses parçalarını reddediyordu; kaynak kodundaki `validate_envelope` ("resource-only" profili) **`sbgp`/`sgpd`** kutularını yasaklıyor ve Reddit'in **AAC** parçaları `sbgp`(**roll**) taşıyor (video parçalarında yok — bayt düzeyinde doğrulandı: video `tfhd/tfdt/trun`, ses `tfhd/tfdt/trun/sbgp`). Bu kutular yalnızca primerleme ipucudur: `traf`/`moof` boyutları ve `trun.data_offset` düzeltilerek çıkarılıyor, **örnek verisi hiç değişmiyor**.
  4. **Doğrulama:** **212 test** geçiyor. Canlı test (`live_reddit_video_is_merged_with_audio`) gerçek bir gönderiyi bulup birleştiriyor ve çıktıda `ftyp`, `mp4a` (AAC) ile `soun` (ses izi) arıyor: `ac17c5qqrcuh1: sesli MP4 birleştirildi — 5.735.062 bayt` (video 720p + 128k ses; Discord'un 10 MB ek limitine sığıyor). Log'da video ve ses parçalarının **dönüşümlü** çekildiği görülüyor.
- **Etkilenen:** `src/converters/builtin/reddit_video.rs` (yeni), `src/converters/builtin/media.rs` (`download_video`), `src/converters/builtin/render.rs`, `src/converters/builtin/reddit.rs` (`pub(super)`), `Cargo.toml` (`hls-transmux` + `url`), `README.md`
### [2026-10-09 16:52] zoltrons — Reddit native: `installed_client` OAuth ile 403 çözümü + `secure_media` (ffmpeg yok)
- **Commit:** `654c466`
- **Yapılan:**
  1. **Araştırma:** Reddit, kimliksiz `.json` isteklerini **403** (JS meydan okuması) ile engelliyor; HTML sayfası da meydan okuma dönüyor. `redlib` (Rust) bu engeli **`installed_client` OAuth** (public client id + `Basic <id>:`) ve `wreq` TLS taklidiyle, `fxreddit` ise Cloudflare IP'sinden düz `.json` ile aşıyor. Video tarafı: `media.reddit_video.fallback_url` **video-only**'dir (indirilen `CMAF_720.mp4` içinde `moov`/`soun`/`mp4a` yok), ses ayrı akışta ve doğrudan **403**; fxreddit sesli sürümü yalnızca HTML sayfasındaki `packaged-media-json`'dan alıyor (o da tarayıcı gerektiriyor).
  2. Native motor artık **`installed_client`** akışıyla bearer token alıp **`oauth.reddit.com`** üzerinden okuyor: kullanıcı izni/parola gerekmez, istemci kimliği public (`REDDIT_CLIENT_ID` ile değiştirilebilir), token bellekte önbelleklenir ve süresi dolmadan yenilenir. Başarısızlıkta kimliksiz `www.reddit.com` yolu, o da olmazsa proxy yedeği devreye girer.
  3. **ffmpeg kullanılmaz:** video Reddit'in `fallback_url` mp4'üyle kartta oynar (sessiz — ses ayrı akışta) ve sesli izleme için gönderi bağlantısı verilir. Ayrıca `secure_media.reddit_video` desteği (bazı gönderilerde medya yalnızca orada), paylaşılan gövde okuyucu (`MAX_BODY_BYTES`) ve UTF-8 hata mesajı düzeltmesi. **207 test** geçiyor (yeni: base64 vektörleri, `secure_media` fallback, **canlı OAuth** testi `live_oauth_fetch_returns_post`).
- **Etkilenen:** `src/converters/builtin/reddit.rs`, `src/converters/builtin/mod.rs` (`fetch` artık `config` alır), `src/config.rs`, `src/settings.rs`, `src/converters/parser.rs`, `src/web/mod.rs`, `.env.example`, `README.md`
### [2026-10-09 16:25] zoltrons — Instagram platform göstergesi özel emoji + `INSTAGRAM_SESSIONID` Docker env
- **Commit:** `09b7b8e`
- **Yapılan:**
  1. Kartın son satırındaki platform göstergesi, X/Twitter'daki gibi Instagram için de **özel emoji** kullanır: `EmojiSettings.instagram` (varsayılan `<:instagram:1558099674731122758>`), `instagram()` erişimcisi ve `v2::meta_line` Instagram dalı. Emoji boş/tanınmazsa yine platform adı yazılır (bozuk emoji çıkmaz).
  2. WebUI emoji setine **“Instagram logosu”** kutusu (`data-emoji-key="instagram"`), `app.js` varsayılanları ve alan listesi eklendi; kök `settings.json` emoji nesnesine değer yazıldı. Panel sözleşme testi (HTML kutuları ↔ ayar şeması) tekrar geçiyor.
  3. `INSTAGRAM_SESSIONID` **Docker ortam değişkeni** olarak `docker-compose.yml` `environment` bloğuna eklendi (`${INSTAGRAM_SESSIONID:-}`; `env_file: .env` zaten aktarır, boş değerde davranış değişmez) ve README'nin Docker bölümü bilgilendirildi.
  4. **204 test** geçiyor; çalışan süreçte `/api/settings` üzerinden doğrulandı: `emojis.instagram = <:instagram:1558099674731122758>`.
- **Etkilenen:** `src/settings.rs`, `src/converters/builtin/v2.rs`, `src/web/ui/index.html`, `src/web/ui/app.js`, `settings.json`, `docker-compose.yml`, `README.md`
### [2026-10-09 16:12] zoltrons — Instagram: oturum çereziyle reel/video **mp4** (kart içinde oynar) + carousel + kesin metrikler
- **Commit:** `a70c2e8`
- **Yapılan:**
  1. **Araştırma:** Instagram reel/video akışını (mp4) girişsiz istemcilere **vermiyor** — embed sayfasının `contextJSON` alanı `null`, `/api/v1/media/{pk}/info/` giriş sayfasına **302**, `POST /api/graphql` HTML/`ssr_disabled_reason: fail_ssr_disabled`, `?__a=1` → **404**. `seirenkr/OGInstagram` kodu incelendi: aynı veriyi **proxy oturum havuzu** (`fetchViaProxy`) üzerinden alıyor; yt-dlp ise `curl_cffi` **TLS parmak izi taklidi** + çerez kullanıyor. Yani Rust/reqwest için tek pratik yol kimlikli oturum veya proxy.
  2. `.env`'de `INSTAGRAM_SESSIONID` tanımlıysa bot gönderiyi Instagram **uygulama API'sinden** (`/api/v1/media/{pk}/info/`, iOS app-id + mobil UA + çerez; iki uç ve iki çerez biçimi denenir) **paralel** çeker: **reel/video kartın içinde oynar**, carousel'in **tüm** görselleri galeriye girer, metrikler (beğeni/yorum/**görüntülenme**) ile görünen ad/doğrulama kesinleşir. Çerez yoksa veya süresi dolmuşsa davranış değişmez (kapak karesi + `Reel` etiketi) — akış hiçbir durumda bozulmaz.
  3. Kısa kod → medya kimliği (pk) dönüşümü gerçek Instagram `data-media-id` değerleriyle birebir doğrulandı; çerez **sır** olarak yalnızca `.env`'den okunur (`Config::Debug` redakte eder, `settings.json`/WebUI'a yazılmaz). **204 test** geçiyor (yeni: pk dönüşümü, oturum JSON ayrıştırma + carousel, video yerleştirme, geçersiz çerez ve geçerli çerez canlı testleri).
  4. **Canlı Discord doğrulaması:** reel kartı API'den okundu — galeri öğesi `type=video/mp4, 720x1280` (Instagram CDN mp4), `-# Instagram • Reel • … • [Link]` ve `:metric: 170.4M` görüntülenme ile birlikte. Log: `Instagram oturum verisi alındı … 1 medya, video: true`.
- **Etkilenen:** `src/converters/builtin/instagram.rs`, `src/converters/builtin/mod.rs`, `src/config.rs`, `src/handler.rs`, `src/settings.rs`, `src/converters/parser.rs`, `src/web/mod.rs`, `.env.example`, `README.md`
### [2026-10-09 15:46] zoltrons — Instagram için native (Components V2) kart motoru: SSR embed + OpenGraph
- **Commit:** `cd52c12`
- **Yapılan:**
  1. Instagram'ın varsayılan motoru proxy'den (`ddinstagram.com`, artık DNS'te çözülmüyor) **native embed**'e alındı. Yeni `src/converters/builtin/instagram.rs` gönderiyi Instagram'ın herkese açık **SSR embed** sayfasından (`/{p|reel|reels|tv}/{kod}/embed/captioned/`) ve gönderi sayfasının **OpenGraph** meta etiketlerinden **paralel** çözer (toplam gecikme tek istek kadar): yazar kullanıcı adı, görünen ad, doğrulama işareti, avatar, açıklama, beğeni/yorum, tarih ve görsel.
  2. Instagram video/reel akışını (mp4) sayfa kaynağına koymadığı için kart videonun **kapak karesini** + `Reel`/`Video` etiketi ve gönderi bağlantısını gösterir. İki uç da başarısız olursa (giriş duvarı) akış hata verir ve ayar açıksa proxy yedeğine düşülür.
  3. Eski `opengraph.rs` yalnızca paylaşılan meta çözümleyicisine indirgendi (`og:url` alanı eklendi); varsayılan motor ayarı, WebUI varsayılanı, `settings.json`, README ve `.env.example` güncellendi. Canlı kaynak testi (`live_fetch_photo_and_reel`) dahil **198 test** geçiyor.
- **Etkilenen:** `src/converters/builtin/instagram.rs` (yeni), `src/converters/builtin/opengraph.rs`, `src/converters/builtin/mod.rs`, `src/settings.rs`, `src/web/ui/app.js`, `settings.json`, `README.md`, `.env.example`
### [2026-10-07 19:56] zoltrons — origin/main (PR #1–3 "modernize-rust-2024") birleştirildi; Spotify kodu std::sync::LazyLock'a taşındı
- **Commit:** `df1c2d1`
- **Yapılan:**
  1. Uzak `main` (PR #1–3 modernize-rust-2024) yerel Spotify işiyle çakışmasız birleştirildi.
  2. Uzak taraf `once_cell` bağımlılığını kaldırıp `std::sync::LazyLock`'a geçtiği için yeni `spotify.rs` statikleri ve parser'daki Spotify regex'leri de `LazyLock`'a taşındı.
- **Etkilenen:** `src/converters/builtin/spotify.rs`, `src/converters/parser.rs`
### [2026-10-07 19:52] zoltrons — Spotify ilk V2 karta döndü; önizleme butonu yerine Apple Music / YouTube Music
- **Commit:** `8b4e18d`
- **Yapılan:**
  1. Discord V2 mesajları satır içi ses oynatmadığı ve V2'de link unfurl çalışmadığı için Spotify varsayılanı yeniden **native (Components V2 kart)** yapıldı; `fxspotify` proxy denemesi geri alındı (proxy + fxspotify hâlâ opsiyonel).
  2. Karttaki "▶ 30 sn önizleme" butonu kaldırıldı; yerine **Apple Music** ve **YouTube Music** arama bağlantıları eklendi (Spotify butonu korundu). Etiketler yalnızca platform isimleri.
  3. mp3 indirme / V2 `File` bileşeni ve `CustomCard.attachment` + `media` ses desteği geri alındı (ilk hâle dönüş).
- **Etkilenen:** `src/converters/builtin/spotify.rs`, `src/converters/builtin/mod.rs`, `src/converters/builtin/v2.rs`, `src/converters/builtin/media.rs`, `src/converters/parser.rs`, `src/settings.rs`, `src/config.rs`, `src/web/mod.rs`, `src/web/ui/app.js`, `settings.json`, `.env.example`
### [2026-10-07 19:40] zoltrons — Spotify önizlemesi artık Discord içinde oynuyor (fxspotify proxy + unfurl video embed)
- **Commit:** `f629b12`
- **Yapılan:**
  1. Araştırma: Discord, bot embed'lerinin `video` alanını **yok sayıyor** (yalnızca link unfurl'u doldurur) ve mp3 ekine satır içi oynatıcı vermiyor. fxspotify (`templates/track.html`) çözümü: `og:video` = 30 sn mp3 (video/mp4 ilan) + `og:image` = kapak + `twitter:card=player`.
  2. Spotify varsayılanı **proxy → `fxspotify.com`** yapıldı. Bot `[:](https://fxspotify.com/track/ID)` gönderince Discord sayfayı unfurl edip kapağı poster, önizlemeyi **oynatılabilir `video` embed** olarak gösteriyor (video üretimi/ffmpeg yok).
  3. `SPOTIFY_DOMAIN` varsayılanı `fxspotify.com`; parser/settings/WebUI/settings.json ve testler güncellendi. Native (Components V2 kart) modu opsiyonel kaldı.
  4. Native kartta 30 sn önizleme mp3'ü `File` bileşeniyle dosya olarak da eklenir (Discord'da indirilebilir; oynatıcı vermez) — asıl satır içi oynatma varsayılan proxy (fxspotify unfurl) ile sağlanır.
- **Etkilenen:** `src/converters/parser.rs`, `src/settings.rs`, `src/config.rs`, `src/web/mod.rs`, `src/web/ui/app.js`, `settings.json`, `.env.example`
### [2026-10-07 19:15] zoltrons — Spotify dönüşümü eklendi: fxspotify önizleme çekme + fixspotify modern (Components V2) kartı
- **Commit:** `40a5eae`
- **Yapılan:**
  1. Yeni `spotify` platformu: `open.spotify.com/(intl-xx/)?(track|album|playlist|artist|episode|show)/id` ve `spotify.link` kısa linkleri algılanır; kısa linkler native motorda `spotify.app.link` üzerinden hedefe çözülür.
  2. **Önizleme çekme (fxspotify yaklaşımı):** `open.spotify.com/embed/{kind}/{id}` HTML sayfasındaki `__NEXT_DATA__` JSON'undan parça adı, sanatçılar, 30 sn `audioPreview.url`, albüm kapağı ve çıkış tarihi **API anahtarı olmadan** okunur.
  3. **Modern embed (fixspotify yaklaşımı):** veri Discord Components V2 tek kartı olarak gönderilir — Spotify yeşili `accent_color` (`0x1DB954`), `Section` + `Thumbnail` (albüm kapağı), `Separator` ve `Action Row` içinde "▶ 30 sn önizleme" + "Spotify'da aç" link butonları.
  4. `BuiltinPost`'a platforma özel hazır `custom_card` alanı eklendi; kart stili ne olursa olsun Spotify bu modern kartı zorunlu kullanır (V2 reddedilirse klasik embed'e düşer).
  5. Ayar/WebUI entegrasyonu: modül anahtarı, motor seçimi (native/proxy), proxy alan adı (`SPOTIFY_DOMAIN`, varsayılan `open.fixspotify.com`), rol filtresi, panel modül satırı/sekmesi ve motor satırı eklendi.
- **Etkilenen:** `src/converters/builtin/spotify.rs` (yeni), `src/converters/parser.rs`, `src/converters/builtin/mod.rs`, `src/converters/builtin/v2.rs`, `src/converters/mod.rs`, `src/settings.rs`, `src/config.rs`, `src/web/mod.rs`, `src/web/ui/index.html`, `src/web/ui/app.js`, `settings.json`, `.env.example`, `.gitignore` (`/local-test/` yerel test klasörü yok sayılır)
### [2026-10-04 13:25] zoltrons — GitHub Actions, test ve Docker derleme adımları dokümantasyon standartlarına uyarlandı
- **Commit:** `49c9604`
- **Yapılan:**
  1. GitHub Actions Security Best Practice: İş akışı seviyesindeki aşırı yetkiler `contents: read` olarak sınırlandı; `test` ve `build-and-package` işleri en az yetki ilkesine (least privilege) uyarlandı, `packages: write` izni yalnızca `docker-publish`'e verildi.
  2. Concurrency denetimi eklendi (`cancel-in-progress`); gereksiz koşular ve koşu dakikası israfı engellendi.
  3. Matris içerisindeki release yarış durumu (race condition ve 422 çakışması) giderildi: `release` adımı bağımsız bir işe taşındı, `actions/download-artifact@v4` `merge-multiple: true` ile tüm derleme çıktılarını tekil ve güvenli bir GitHub Release'e toplar hale getirildi.
  4. Docker CI/CD güçlendirildi: PR'larda da Docker derleme testi (`push: false`) çalışacak şekilde ayarlandı, GHCR girişi yalnızca push durumlarında çalıştırıldı; `Organize binaries` adımına `chmod +x` ve Dockerfile'lara `--chmod=755` eklenerek ikili dosya çalıştırma izinleri güvenceye alındı; POSIX uyumlu paket kurulum koşulu ve temiz `.dockerignore` filtreleri uygulandı. Raspberry Pi tarball'ına eksik olan `deploy-rpi.sh` dahil edildi.
- **Etkilenen:** `.github/workflows/build-release.yml`, `Dockerfile`, `Dockerfile.prebuilt`, `.dockerignore`
### [2026-10-04 13:07] zoltrons — Discord API ve Geliştirici Dokümantasyonu uyumluluk düzeltmeleri
- **Commit:** `80f274a..b26c4d6`
- **Yapılan:**
  1. Discord Developer Documentation standartlarına uygun resmi HTTP User-Agent başlığı eklendi (`DiscordBot ($url, $versionNumber)`).
  2. Components V2 Container (`type: 17`) azami alt bileşen limiti 12'den 10'a çekildi; toplam kümülatif bileşen metin boyutu 4.000 karakterle sınırlandırıldı.
  3. Webhook'larda `message_reference` alanı Discord REST uç noktasının 400 hatası vermemesi için temizlendi; 429 Too Many Requests yanıtları için Retry-After tabanlı yeniden deneme eklendi ve 429'un kalıcı özellik kapatma (V2_UNAVAILABLE / None webhook) tetiklemesi engellendi.
  4. Klasik akışta botun kendi zengin embed'lerini gizleyen `SUPPRESS_EMBEDS` bayrağı kaldırıldı, URL Discord Markdown `<URL>` standardı ile sarıldı; webhook kullanıcı adı filtreleme case-insensitive yasaklı kelimeler ("discord", "clyde") ve reserved isimler ("everyone", "here") için güçlendirildi.
  5. Kanal düzeyinde izin ezmelerini (channel overwrites) gözeten kanonik Serenity izin kontrolü uygulandı; DM reaksiyon intent'i eklendi.
- **Etkilenen:** `src/converters/builtin/raw.rs`, `src/converters/builtin/v2.rs`, `src/converters/builtin/mod.rs`, `src/handler.rs`, `src/main.rs`
### [2026-10-04 11:01] zoltrons — Orijinal mesajı silme zamanlaması düzeltildi (hayalet mesaj / client desync)
- **Commit:** `0fe58fd`
- **Yapılan:**
  1. Silme, kart gönderildikten sonra ve mesaj en az ~1,2 sn yaşına gelene kadar ertelenir hale getirildi; çok hızlı silinen mesaj, gönderenin istemcisi optimistic (nonce'lu) mesajını sunucu kopyasıyla eşleştirmeden silindiği için yalnızca gönderenin ekranında "hayalet" olarak kalıyordu (Discord istemci desync — discord-api-docs #850).
  2. Proxy yolunda silme, gönderimden hemen önce yerine gönderimden sonra arka plana alındı; native akıştaki `delete_early` erken silme kaldırıldı, silme `finish()` içine taşındı.
  3. `delete_wait()` mesaj yaşını snowflake ID'den hesaplar (serenity'nin chrono/time arka ucundan bağımsız); yeni birim test eklendi.
- **Etkilenen:** `src/converters/mod.rs`, `src/converters/builtin/mod.rs`, `README.md`
### [2026-10-02 16:07] zoltrons — Reddit mobil paylaşım (/s/) linkleri için yönlendirme desteği eklendi
- **Commit:** `c0b2d96`
- **Yapılan:**
  1. Reddit mobil paylaşım linklerinin (`/r/{sub}/s/{kod}`) kalıcı kimlik içermemesi sebebiyle User-Agent ile hedef adrese yönlendirme izlenmesi sağlandı.
  2. Reddit JSON uç noktası bot engellerini (403) aşacak tarayıcı User-Agent'ı ve boyut sınırlarıyla korundu.
  3. URL kimlik çıkarımı için birim testler eklendi.
- **Etkilenen:** `src/converters/builtin/reddit.rs`
### [2026-10-02 16:03] zoltrons — GIF'ler her zaman gif.fxtwitter.com üzerinden .gif formatına ayarlandı
- **Commit:** `1392d85`
- **Yapılan:**
  1. Kullanıcı direktifi doğrultusunda GIF'lerin doğrudan/uzak modda her zaman `.gif` formatında kalması ve `gif.fxtwitter.com` adresinin kullanılması sağlandı (`remote_gif_url`).
  2. `media::fx_gif_url` hazır `gif.fxtwitter.com` linklerini de idempotent olarak tanıyacak şekilde güncellendi.
  3. `download_gif` indirme/çevirme başarısız olduğunda MP4 fallback dosyası eklemek yerine `None` dönerek kartın gerçek `.gif` bağlantısına düşmesi güvenceye alındı; birim testler güncellendi.
- **Etkilenen:** `src/converters/builtin/media.rs`, `src/converters/builtin/render.rs`, `src/converters/builtin/v2.rs`

### [2026-10-02 15:35] zoltrons — ışık hızında ve modern v2'de GIF'lerin görünmeme sorunu çözüldü
- **Commit:** `0aff66e`
- **Yapılan:**
  1. Işık hızı (`direct`) ve indirme yapılmayan modlarda GIF'lerin harici `gif.fxtwitter.com` servisine dönüştürülmesi kaldırıldı (FxEmbed fastgif servisi 500 Internal Server Error vererek çöktüğü için Discord kartında GIF boş kalıyordu).
  2. Twitter'ın doğrudan yüksek erişilebilirlikli ve hızlı `video.twimg.com/tweet_video/*.mp4` adresi doğrudan kullanılarak Modern V2 (`Media Gallery`) kartında yerel ve anında oynatılması sağlandı.
  3. Klasik embed fallback'inde MP4 içeren GIF'lerin bozuk görsel yerine poster görseliyle (`video_poster`) görüntülenmesi sağlandı; tüm birim testler güncellendi ve yenisi eklendi.
- **Etkilenen:** `src/converters/builtin/render.rs`, `src/converters/builtin/v2.rs`

### [2026-10-02 14:48] zoltrons — agent.md iş günlüğü oluşturuldu (geçmiş geriye dönük dolduruldu)
- **Commit:** bu dosyayı tanıtan ilk commit
- **Yapılan:** `agent.md` bot iş günlüğü kuruldu; `main` üzerindeki 40 commit geriye dönük dolduruldu. Protokol, "Bilinen Kimlikler" tablosu ve log bölümü eklendi.
<!-- LOG:END -->

---

## 📜 Git Geçmişi (geriye dönük · en yeni'den eski'ye)

> `main` üzerindeki 40 commit. Kimlik anahtarı: **`eray` = insan (repo sahibi)**, **`zoltrons` = bot/agent.**
> Aşağıda yalnızca `main`'deki anlamlı commit'ler listelenmiştir; otomatik `cline checkpoint` commit'leri (ayrı ref'te) burada yer almaz.

### 2026-10-01
- `9cdd5d4` · **zoltrons** · 18:52 — perf(cards): kart görsellerini hafif varyanta indir, iç içe alıntı zincirini göster

### 2026-09-30
- `8657fce` · **zoltrons** · 20:06 — perf(cards): ışık hızı medya modu, FxEmbed öncelikli varyantlar ve Grok-only çeviri
- `7d63eb3` · **zoltrons** · 19:38 — fix(v2-card): spoiler gizlemesini ve bot adına giden kartta paylasan etiketini ekle

### 2026-09-29
- `80ace78` · **zoltrons** · 21:19 — perf(cards): modern kartı anında gönder, fallback ve ağırlık algoritmalarını optimize et

### 2026-09-28
- `95cc5c5` · **eray** · 20:02 — feat(webui): paneli gömülü varlıklara böl, CSP'yi sıkılaştır
- `4536cc1` · **eray** · 19:56 — fix(impersonation): alt başlık ve forum gönderilerinde taklit webhook'unu üst kanalda tut

### 2026-09-27
- `d0209f8` · **eray** · 22:59 — fix(v2-card): alıntı bloğundaki boş '>' satırını kaldır
- `2eb8671` · **eray** · 22:52 — fix(v2-card): alıntı bloğunu tek blok alıntıya çevir, em boşluğu girintisini kaldır
- `3f7541e` · **eray** · 22:40 — fix(webui): açılır menünün panel arkasında kalması ve kesilen sekmeler
- `5f71fa0` · **eray** · 22:34 — test(webui): panel ve API uçları için gerçek HTTP entegrasyon testi
- `b94e7b4` · **eray** · 22:31 — feat(v2-card): alıntı bloğuna hiyerarşi boşluğu ve sağa girinti ekle
- `76aaf5d` · **eray** · 22:22 — refactor(webui): ölü/çakışan ayarları kaldır, panel hatalarını düzelt
- `af5c75b` · **eray** · 18:03 — feat(store): kart kayıtlarını SQLite ile kalıcı yap, kart yanıtlarında AI'yı engelle
- `1e97750` · **eray** · 17:46 — fix(v2-card): kullanıcı metnini kartın üstüne taşı, video kapak tekrarını kaldır
- `50d11b8` · **eray** · 17:31 — fix(converters): yanıt bağlantısını koru ve orijinal mesajı erken sil
- `f76f5ad` · **eray** · 14:57 — fix(docker): ffmpeg'i isteğe bağlı yap, ince runtime imajına dön
- `76f4c95` · **eray** · 14:41 — feat(converters): fxembed GIF servisi, tek istek/paralel akış, alıntı medyası ve önbellek
- `ed74812` · **eray** · 13:56 — fix(native-embed): karttan profil resmini kaldır, varsayılan kart stili modern
- `d58d4c9` · **eray** · 13:34 — feat(native-embed): X tarzı Components V2 kartı, özel emoji seti ve ham REST gönderimi
- `9d18754` · **eray** · 12:56 — feat(converters): built-in native embed engine with X auto-translation
- `16647b4` · **eray** · 11:49 — fix: harden message building, settings persistence & error logging

### 2026-09-11
- `5f6d4b4` · **zoltrons** · 17:55 — fix(ai): add multi-tier failover & automatic retry for upstream rate limits and 504 timeouts
- `0483386` · **zoltrons** · 17:51 — perf(build): optimize build times with ThinLTO, BuildKit cache mounts, LLD linker & parallel CI
- `98942be` · **zoltrons** · 17:44 — fix(handler): prevent AI replies to link conversions even with reply-mentions; update settings.json model
- `d3e3a60` · **zoltrons** · 17:41 — refactor: codebase quality, stability, and security enhancements
- `033ed93` · **zoltrons** · 17:03 — fix: OpenRouter model -> google/gemma-4-31b-it:free & link-conversion reply filter

### 2026-09-08
- `849a604` · **zoltrons** · 21:38 — fix(reddit): switch default domain to vxreddit.com and support /s/ mobile share and short links

### 2026-09-06
- `5e59ad8` · **zoltrons** · 20:13 — chore: save active settings
- `fbe0fab` · **zoltrons** · 20:10 — fix(ai): sanitize @everyone/@here mentions, fix ai toggle switch and prompt sync
- `13406c0` · **zoltrons** · 20:02 — fix(security): configure non-root user 1000:1000 across Dockerfile, Dockerfile.prebuilt and docker-compose.yml
- `1405c88` · **zoltrons** · 19:53 — fix(docker): resolve permission denied on bind-mounted settings.json by removing non-root restriction and adding user 0:0
- `9c554e3` · **zoltrons** · 19:41 — feat: per-server (per-guild) settings architecture and WebUI server switcher
- `99ff101` · **zoltrons** · 19:36 — chore: update settings.json with per-module role filter schema
- `0bf7fd5` · **zoltrons** · 19:34 — feat: per-module role filtering and visual Discord server role picker
- `198bfba` · **zoltrons** · 19:28 — fix(settings): handle Docker bind mount inode preservation and folder mount fallback
- `adcd7ab` · **zoltrons** · 19:10 — fix(deb): fix cross-compilation strip error and debian library dependencies for ARM
- `a6cb05f` · **zoltrons** · 19:09 — chore: add port mapping and settings volume persistence to docker-compose.yml
- `6aebbee` · **zoltrons** · 19:02 — fix(ci): fix cargo-deb TOML license error and speed up multi-arch Docker builds by 10x
- `c45bce9` · **zoltrons** · 19:00 — fix(security): patch mention injection, stored DOM XSS, body limits, and add security headers
- `6e1a540` · **zoltrons** · 18:47 — feat: initial commit - Botgek Discord link converter, WebUI & Raspberry Pi CI/CD

---
