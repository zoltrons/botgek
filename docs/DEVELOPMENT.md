# Geliştirme

## Gereksinimler

| Araç | Sürüm / not |
| --- | --- |
| Rust | `rust-toolchain.toml` **1.98.1** sürümünü sabitler; kaynak dil tabanı **1.88** (edition 2024). |
| C derleyicisi / linker | Linux'ta gerekir; SQLite bundled olarak derlenir. |
| Node.js | Yalnızca panel sözdizimi ve panel testleri için. |
| Docker + Compose v2 | Yalnızca container ile çalıştırmak için (isteğe bağlı). |

## Yerel çalıştırma

```sh
cp .env.example .env
# DISCORD_TOKEN ve (panel için) ADMIN_TOKEN doldurun
cargo run --locked
```

Sürüm ve kullanım:

```sh
cargo run --locked -- --version
cargo run --locked -- --help
```

`--version` çıktısı sürüm, git SHA, derleme tarihi, hedef ve etiket bilgisini içerir; künye `build.rs`
tarafından gömülür.

## Testler

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked

node --check src/web/ui/app.js
node --check src/web/ui/login.js
node --test tests/web-ui.test.cjs
```

Normal test koşusu **ağdaki sosyal medya servislerine bağlanmaz**. Yerel HTTP testleri yalnızca loopback
üzerinde fixture sunucuları açar; dosya testleri geçici dosyalar kullanır. Rust ve JavaScript ayar
çözümleme testleri `tests/fixtures/settings-resolution.json` sözleşmesini paylaşır.

Rust panel testleri gömülü varlıkları ve API uçlarını gerçek HTTP isteğiyle doğrular; panelin yalnızca
sunucuda tanımlı API yollarını çağırdığını da kontrol eder. Node testleri ise panelin ayar çözümleme
mantığını ve kayıt/yarış davranışını tarayıcı bağlamında çalıştırır.

MCP endpoint'inin kimlik doğrulama, `tools/list`, `tools/call`, `settings_patch` ve eski `initialize`
akışını kapsayan entegrasyon testi:

```sh
cargo test --locked web::tests::mcp_endpoint_authenticates_and_serves_tools
```

MCP kurulumu, araçlar ve sınırlar: [MCP.md](MCP.md).

### Canlı testler

Canlı testler açıkça ignore edilir ve yalnızca bilerek çalıştırılır:

```sh
cargo test --locked live_ -- --ignored --nocapture
```

Bu komut gerçek upstream erişimi ister; Instagram oturum testi ayrıca özel çerez ister. Eksik önkoşul
veya başarısız birleştirme **başarılı test gibi gösterilmez**. Üretim sırlarını public PR testlerine
aktarmayın.

### Performans ve karşılaştırma testleri

```sh
cargo test --release --locked offline_card_baseline -- --ignored --nocapture
cargo test --locked twitter::benchmarks -- --ignored --nocapture
cargo test --locked live_fetch_main_and_alternative_sources -- --ignored --nocapture
```

`twitter::benchmarks` gerçek gönderilerde X veri süresini, görüntülenme/çeviri varlığını ve sonucun
önbelleğe uygunluğunu yazdırır. Ayrıntılar: [OPERATIONS.md](OPERATIONS.md#performans-ölçümü).

## Proje yapısı

```text
src/
├── main.rs            # süreç başlangıcı
├── config.rs          # ortam değişkenleri
├── handler.rs         # gateway olayları, AI tetikleme, silme
├── commands/          # slash ve mesaj komutları
├── converters/
│   ├── parser.rs      # URL ayrıştırma ve kanonikleştirme
│   └── builtin/       # platform dönüştürücüleri ve kart üretimi
├── settings.rs        # şema, kapsam çözümleme, göç
├── store.rs           # SQLite
├── web/               # API + panel (gömülü HTML/CSS/JS)
└── ai/                # OpenRouter istemcisi
tests/
├── web-ui.test.cjs    # panel testleri
└── fixtures/          # Rust ve JS'in paylaştığı sözleşme
```

## Yeni platform ekleme

Bir platform eklerken tipik dokunuş noktaları:

1. `src/converters/parser.rs`: URL kalıpları, `Platform` varyantı, kanonikleştirme ve spoiler desteği.
2. `src/converters/builtin/`: platform modülü; `fetch` ve gerekirse `render` yolu.
3. `src/converters/builtin/mod.rs`: `fetch` dağıtımı ve platform anahtarı.
4. `src/settings.rs`: modül anahtarı ve rol filtresi.
5. `src/web/ui/` ve `settings.json`: panel anahtarları ve varsayılanlar.
6. Testler: parser birim testleri, mümkünse loopback fixture'ı ve panel sözleşmesi.

Yeni ağ erişimi ekliyorsanız zaman aşımı, yanıt boyutu sınırı ve hata davranışını tanımlayın; kullanıcı
girdisinden gelen URL'lerde [ARCHITECTURE.md](ARCHITECTURE.md#önizleme-güvenliği) içindeki public adres
kurallarını uygulayın.

## Derleme profilleri

- `profile.dev`: hata ayıklama bilgisi yalnızca satır tablolarıyla sınırlıdır; geri izlemeler çalışır ama
  `target/` hacmi ve link süresi küçülür.
- `profile.release`: `opt-level = 3`, ince LTO, 16 codegen unit, `strip = true`. Panik = unwind; harici
  bir yanıttaki tek bir panik yalnızca o tokio görevini düşürür, tüm süreci öldürmez.
- Docker derlemesinde `RUSTFLAGS="-C link-arg=-fuse-ld=lld"` kullanılır.

## Windows

MSVC toolchain için Visual C++ Build Tools ve Windows SDK gerekir. Önceden kurulu GNU/LLVM toolchain
kullanıyorsanız aynı kontrolleri şu biçimde çalıştırabilirsiniz:

```sh
cargo +stable-x86_64-pc-windows-gnullvm test --locked
```

Derleyici sürümünü `rustc +stable-x86_64-pc-windows-gnullvm --version` ile doğrulayın. Global rustup
ayarını değiştirmeniz gerekmez.

## Katkı akışı

1. Değişiklik için bir GitHub issue açın veya mevcut issue'yu seçin.
2. Küçük ve odaklı bir dal oluşturun.
3. Kod ile birlikte test ekleyin; davranış değişikliğini ilgili belgede güncelleyin.
4. `cargo fmt`, `cargo clippy -- -D warnings` ve ilgili testleri çalıştırın.
5. Commit mesajlarında depo geleneğini izleyin: `fix(commands): …`, `feat(card): …`, `docs: …`.
6. PR açıklamasında değişikliği, testleri ve varsa davranış/performans etkisini belirtin.

Ayrıntılar: [CONTRIBUTING.md](../CONTRIBUTING.md).

## Sürüm akışı

Sürüm numarası `Cargo.toml` ve git etiketlerinden türetilir; elle iki yerde tutulmaz.

```sh
./release.sh patch
./release.sh minor --push
```

`release.sh` temiz çalışma ağacı ister, `Cargo.toml`/`Cargo.lock` günceller, testleri çalıştırır ve
annotated tag atar. GitHub Actions'ta aynı akış `release.yml` ile elle başlatılır. Paketleme ve imaj
ayrıntıları: [OPERATIONS.md](OPERATIONS.md#cicd).
