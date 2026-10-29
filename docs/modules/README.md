# Modül belgeleri

Her modülün ne yaptığı, hangi dosyalarda durduğu, hangi kurallara uyduğu ve değiştirirken nelere dikkat
edileceği burada. Sistem düzeyindeki anlatım için [ARCHITECTURE.md](../ARCHITECTURE.md), ayar ve işletim
için [CONFIGURATION.md](../CONFIGURATION.md) ve [OPERATIONS.md](../OPERATIONS.md) okunur.

## Bir mesajın yolu

```text
handler ─► converters::enqueue_message ─► kuyruk (32) ─► builtin::handle
              │ parser, ayar, rol filtresi        │
              │                                   ├─ builtin::fetch ─► <platform>::fetch ─► BuiltinPost
              │                                   ├─ render::plan_media / resolve_media
              │                                   ├─ v2::build_payload ─► Components V2 JSON
              │                                   └─ raw / transport ─► Discord REST ─► store (kart kaydı)
```

## Dizin

| Belge | Kapsadığı kod |
| --- | --- |
| [runtime.md](runtime.md) | `main.rs`, `config.rs`, `version.rs`, `utils/`, `logs.rs` |
| [handler.md](handler.md) | `handler.rs` |
| [settings.md](settings.md) | `settings.rs`, `policy.rs` |
| [store.md](store.md) | `store.rs` |
| [telemetry.md](telemetry.md) | `telemetry.rs`, `benchmarks.rs` |
| [commands.md](commands.md) | `commands/` |
| [converters.md](converters.md) | `converters/mod.rs`, `converters/parser.rs` |
| [pipeline.md](pipeline.md) | `builtin/mod.rs`, `model.rs`, `cache.rs`, `coordination.rs`, `transport.rs`, `raw.rs` |
| [rendering.md](rendering.md) | `builtin/render.rs`, `v2.rs`, `media.rs` |
| [preview.md](preview.md) | `builtin/preview.rs`, `opengraph.rs`, `safe_http.rs` |
| [web.md](web.md) | `web/` |
| [mcp.md](mcp.md) | `mcp.rs` |
| [ai.md](ai.md) | `ai/` |
| [platforms/](platforms/) | Platform başına bir dosya, aşağıya bakın |

### Platformlar

| Platform | Belge | Kod |
| --- | --- | --- |
| X (Twitter) | [twitter.md](platforms/twitter.md) | `twitter.rs`, `translate.rs` |
| Reddit | [reddit.md](platforms/reddit.md) | `reddit.rs`, `reddit_gql.rs`, `reddit_video.rs` |
| Instagram | [instagram.md](platforms/instagram.md) | `instagram.rs` |
| TikTok | [tiktok.md](platforms/tiktok.md) | `tiktok.rs` |
| Bluesky | [bluesky.md](platforms/bluesky.md) | `bluesky.rs` |
| Spotify | [spotify.md](platforms/spotify.md) | `spotify.rs`, `spotify_video.rs` |
| Twitch | [twitch.md](platforms/twitch.md) | `twitch.rs` |
| Pixiv | [pixiv.md](platforms/pixiv.md) | `pixiv.rs` |
| Tumblr | [tumblr.md](platforms/tumblr.md) | `tumblr.rs` |
| Facebook | [facebook.md](platforms/facebook.md) | `facebook.rs` |
| YouTube | [youtube.md](platforms/youtube.md) | `youtube.rs` |
| Letterboxd | [letterboxd.md](platforms/letterboxd.md) | `letterboxd.rs` |

Platform dosyaları `src/converters/builtin/` altındadır.

## Her belgenin yapısı

Belgeler aynı başlıkları kullanır, böylece bir ajan ya da geliştirici aradığı bilgiyi hep aynı yerde bulur.

- **Görev:** Modül ne yapar, tek paragraf.
- **Dosyalar:** Hangi dosya neyi taşır.
- **Girdi ve çıktı:** Dışarıdan ne alır, ne verir.
- **Nasıl çalışır:** Akış, kararlar, sınırlar.
- **Değişmezler:** Bozulursa hata çıkaran kurallar. Bir şeyi değiştirmeden önce bunlara bakın.
- **Değiştirirken:** Sık yapılan değişikliklerde dokunulacak yerler.
- **Testler:** İlgili testlerin yeri ve çalıştırma komutu.

## Kod geleneği

- Kod yorumları ve kullanıcıya dönen metinler Türkçedir. Yeni kod aynı dili kullanır.
- Hatalar çoğunlukla `Result<_, String>` olarak taşınır. Kullanıcıya gösterilen metinler sır içermez,
  ayrıntı yalnızca loga gider.
- Platform anahtarı küçük harf bir dizedir (`twitter`, `reddit`, ...). `builtin::platform_key` üretir ve
  ayarlar, telemetri ve loglar bunu kullanır.
- Platform modülleri yalnızca veri çeker ve `BuiltinPost` döner. Discord, ayar ya da kart düzeni bilmezler.
- Ağ erişimi olan her kod zaman aşımı, yanıt boyutu sınırı ve hata davranışı tanımlar.
- Testler ağa çıkmaz. Canlı testler `live_` önekiyle `#[ignore]` işaretlidir ve `--ignored` ile çalışır.

## Doğrulama

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
node --test tests/web-ui.test.cjs
```

Küçük metin değişiklikleri için derleme ya da test çalıştırmak şart değildir.
