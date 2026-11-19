# Render: medya planı, kart düzeni, indirme

## Görev

`BuiltinPost`'tan Discord'a gidecek Components V2 gövdesini ve ekleri üretir. Klasik embed yoktur.

## Dosyalar

| Dosya | İçerik |
| --- | --- |
| `builtin/render.rs` | `RenderOptions`, `plan_media`, `resolve_media`, görünürlük kararı (`apply_visibility`), sayı biçimleme, markdown kaçışı. |
| `builtin/v2.rs` | `V2Card` ve bileşen ağacı: metin, ayraç, bölüm, küçük görsel, medya galerisi, konteyner. |
| `builtin/media.rs` | Medya indirme, dosya adı ve MIME çıkarımı, Discord dosya sınırı, `fx_gif_url`. |

## Girdi ve çıktı

- `RenderOptions::from_engine(engine, platform_key)`: `video_requires_download`, `max_images`, `max_video_mb`,
  `show_metrics`, `show_timestamp`, `show_community_note`, `show_quote`.
- `plan_media(post, options) -> MediaPlan`, `resolve_media(...) -> ResolvedMedia`.
- `v2::build_payload` Components V2 JSON'u döner.

## Nasıl çalışır

### Medya planı

`plan_media` ilk videoyu tutar, görselleri `max_images` ile sınırlar, GIF'leri videolardan ayrı planlar.
Gönderide medya yoksa alıntının medyası kullanılır. Medyasız gönderide link önizleme görseli planlanır.

Varsayılan olarak hiçbir medya indirilmez. Görseller ve GIF'ler CDN bağlantısıyla kartta gösterilir
(`direct_media`). İndirme yalnızca şunlar için zorunludur (`video_requires_download`): Reddit, Spotify
ve Twitch (TikTok videosu fxTikTok adresiyle uzaktan oynar). Bunun yanında Pixiv görselleri `Referer` gerektirdiği için indirilip eklenir, indirilemezse
aynadan gösterilir. Facebook'un imzalı MP4'ü de dosya olarak eklenir.

`resolve_media` indirmeleri bütçeyle paralel yürütür. Kartlar bellekte toplam 100 MiB'lik bütçeyi paylaşır
(`MEDIA_BUDGET_MIB`); yer yoksa kart beklemeden uzak bağlantıya düşer. Tek Pixiv görseli en çok
`PIXIV_IMAGE_LIMIT` (5 MiB) olabilir. Bütçe dolarsa video kartta uzak bağlantıyla oynar
ya da hiç gösterilmez (platforma göre). İndirilen dosyalar `attachment://ad` ile Media Gallery'ye bağlanır.
Spoiler dosyaları `SPOILER_` önekiyle yüklenir ve referans adı eşleşmelidir.

Discord sınırı dosya başınadır: varsayılan `media::DISCORD_FILE_LIMIT` (20 MiB), boost 2. seviyede 50, 3. seviyede
100 MiB (`guild_file_limit`). Etkileşim yanıtında Discord'un bildirdiği `attachment_size_limit` kullanılır.
`max_video_mb` ayarı bu sınırı aşamaz. TikTok kalite seçimi önbelleğe girdiği için varsayılan sınıra göre yapılır.

Zipline açıksa (`ZIPLINE=true` ve üç `ZIPLINE_*` değeri; `zipline::domain()`), indirilen video Discord'a ek olarak gönderilmez: `attach_video`
onu Zipline'a yükler ve herkese açık adresini `attached_video`'ya yazar (kart onu `attachment://` referansı gibi
galeride oynatır). Bu durumda sınır Discord'unki değil `zipline::MAX_BYTES` (50 MB) olur
(`max_video_bytes`). Yükleme başarısızsa dosya `max_attachment_bytes`'a sığıyorsa eskisi gibi ek olur, sığmıyorsa
uzak (sessiz) bağlantıya düşülür.

### Görünürlük

`apply_visibility` hassas içerik, kaynak spoiler'ı, elle `||...||` ve NSFW kanal durumunu birleştirir.
Kurallar:

- Hassas içerik varsayılan olarak spoiler'dır, yalnızca NSFW kanalda ve ayar izin verirse açık gösterilir.
- Kaynak spoiler'ı (`source_spoiler`) ve elle işaretlenen spoiler NSFW kanalda da gizli kalır.
- Alıntıya da uygulanır, gizli alıntının medyası geri düşmez. Spoiler'a katlanan alıntı `> ` öneklerini
  kaybeder: Discord `||...||` içinde blok alıntı çizmez.

### Kart düzeni (`v2.rs`)

Kullanıcının linkle yazdığı not kartın **dışında**, üstte düz metin olarak gider. Components V2 bayrağı
`content` alanını yasakladığı için bu metin bir `Text Display` bileşenidir. Kartın içi sırayla:

0. (Yanıtsa) iplik bağlamı (`reply_context`): küçük gri `-#` satırlar ve ince ayraç. Kendi ipliğinin
   devamında `╭ Thread'in devamı · [önceki gönderi]`, başkasına yanıtta `╭ [Yanıtlanan]: **Ad**
   @kullanıcı`. Platform üst gönderinin metnini verdiyse (X, Bluesky) en çok 3 satır / 220 karakter altında
   durur; gizli kartta metin gösterilmez. Veri `BuiltinPost::reply_to`'dan gelir, alıntı yuvasını kullanmaz.
1. Yazar satırı: ad kalın, `@kullanıcı` normal ağırlıkta (`**Ad** ✓ [@kullanıcı](profil)`). Ad kullanıcı
   adıyla aynıysa yalnızca bağlantılı ad yazılır; yazarı yalnızca platform adı olan (OG) gönderide satır
   yoktur. Reddit'te üst satır küçük gridir: `-# **r/alt** • flair • Posted by u/kullanıcı`.
2. Gönderi metni. Reddit başlığı `### Başlık` (gizliyken kalın), gövdesi `REDDIT_BODY_LIMIT` (700)
   karakterde `… [Devamını oku]` ile kesilir. Düz metin kaynaklarında satır başındaki `>`, `#`, `-#`
   kaçırılır (`escape_block_markers`); Reddit'in markdown'ı korunur.
3. Gönderinin kendi medya galerisi (varsa) alıntının **üstünde** durur.
4. Ayraç ve alıntı bloğu: `> ✎ [Alıntılanan](link): **Ad** [@kullanıcı](profil)`, boş satırsız en çok
   `QUOTE_LINES` (6) satır, fazlası `> …`. Gönderinin medyası yokken alıntının tek görseli alıntının
   yanında küçük görsel (Section + Thumbnail) olur; video ve çoklu görsel galeride alıntının altında kalır.
5. Platformun verdiği alt metin (`Media::alt`, X/Bluesky/Instagram) galeri öğesinin `description`'ı
   olur (en çok 1024 karakter); gizli kartta eklenmez. Bağlantı önizlemesinin açıklaması 200 karakterdir.
6. Metrik satırı (`-#` küçük yazı): yanıt, repost, beğeni, görüntülenme. Reddit'te `1 upvote` / `N upvotes`.
7. Alt bilgi: platform emojisi, etiket, göreli zaman, `[Link]`.

Sınırlar: `TEXT_DISPLAY_LIMIT` 4000, `CUMULATIVE_TEXT_LIMIT` 4000 (toplam), `MEDIA_GALLERY_LIMIT` 10.
Discord kapsayıcı başına çocuk sınırını Nisan 2025'te kaldırdı; mesaj başına toplam 40 bileşen geçerlidir,
kart üst düzeyde en çok `MAX_CONTAINER_CHILDREN` (30) çocuk taşır. Metin sığmazsa `truncate_chars` ile `…` eklenerek kısaltılır.
`custom_card` (Spotify, Twitch, Letterboxd) varsa genel düzen yerine bu bileşenler kullanılır.

Özel emojiler `EmojiSettings`'ten gelir (bkz. [settings.md](settings.md)).

## Değişmezler

- Toplam metin sınırı kartın tamamı içindir. Yeni metin bileşeni eklerken `enforce_cumulative_text_limit`
  tarafından kesilmeyeceğini doğrulayın.
- Markdown kaçışı (`escape_markdown`) kullanıcı ve platform metnine uygulanır. Ham metni doğrudan bileşene
  koymayın.
- Component tip numaraları `v2.rs` içinde sabittir (`TYPE_SECTION = 9`, `TYPE_TEXT_DISPLAY = 10`,
  `TYPE_THUMBNAIL = 11`, `TYPE_MEDIA_GALLERY = 12`, `TYPE_SEPARATOR = 14`, `TYPE_CONTAINER = 17`).
- Spoiler ek adı ile galeri referansı aynı olmalı.
- İndirme zorunluluğu olan platformların listesi `video_requires_download` içindedir. Ekleme yaparken testi
  (`only_reddit_spotify_twitch_and_tiktok_require_a_video_download`) güncelleyin.

## Değiştirirken

- Yeni metrik ya da alt bilgi alanı: `metrics_line` / `meta_line` ve ilgili `RenderOptions` bayrağı. Bayrak
  `EngineSettings`'ten gelir (bkz. [settings.md](settings.md)).
- Yeni platforma özgü düzen: önce genel düzenin yetip yetmediğine bakın. Gerekirse platform modülü
  `CustomCard` üretir, `v2.rs`'e özel dal eklemek son çaredir.

## Testler

`cargo test --locked render`, `cargo test --locked v2`, `cargo test --locked media`. Pixiv canlı testi
`live_pixiv_images_are_attached` `#[ignore]` işaretlidir.
