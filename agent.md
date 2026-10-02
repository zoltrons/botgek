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
   - **Commit:** `<sha>`            # birden fazlaysa: <sha1>..<sha2>
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

## 🕐 Güncel Faaliyet (en yeni en üstte)

<!-- LOG:START -->
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