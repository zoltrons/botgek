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
