# Ajan çalışma kılavuzu

Amaç: bir işe en az okumayla başlamak. Kaynak ~43 bin satır; en büyük dosyalar 2000 satırın üstünde.
Tahmin yürütmek ya da dosyaları baştan okumak pahalıdır. Aşağıdaki sıra bunu önler.

Yönlendirme tablosu ve dosya haritası: [MAP.md](MAP.md).

## 1. Okuma sırası

1. **`AGENTS.md`** otomatik yüklenir; tekrar okumayın.
2. **[MAP.md](MAP.md) ─► "Görev ─► ilk bakılacak yer"** tablosunda işinize uyan satırı bulun. Başlangıç
   dosyası, belge, arama çapası ve doğrulama komutu o satırdadır.
3. **Modül belgesinin yalnızca ilgili bölümünü** okuyun (`Değişmezler`, `Değiştirirken`). Başlıkları görmek
   için `grep -n '^#' docs/modules/<belge>.md`, sonra `Read` ile `offset`/`limit`.
4. **Kodun çatısı:** `scripts/outline.sh <dosya>`. Çıktı fonksiyon/tip adlarını satır numaralarıyla verir.
5. **Hedef aralığı okuyun:** `Read` ile `offset` + `limit` (hedef satır ±60). Dosyanın tamamı ancak
   300 satırın altındaysa okunur.
6. **Düzenlemeden önce** o fonksiyonun çağıranlarına bakın: `scripts/outline.sh --refs <ad>`.

Sırayı atlamayın: 2. adım çoğu işte hedef dosyayı doğrudan verir, 4-5. adım da onu birkaç yüz token'a indirir.

## 2. Token tasarrufu kuralları

- **Okumayın:** `target/`, `Cargo.lock`, `docs/` altını topluca, `ARCHITECTURE.md`'yi baştan sona (başlığa
  atlayın), 1000 satır üstü kaynak dosyayı tamamen, `src/web/ui/app.js`'i tamamen.
- **Önce ara, sonra oku.** Aşağıdaki komutlar bir dosyayı açmaktan ucuzdur.
- **Çıktıyı sınırlayın:** `| head -40`, `git grep -n ... -- 'src/*.rs'`. `target/` aramaya girmesin diye
  her zaman `git grep` kullanın.
- **Aynı dosyayı iki kez okumayın.** İlk okumada gereken aralığı geniş alın, sonra düzenleyin.
- **Belgeyi koda tercih edin:** "X nasıl çalışır?" sorusu modül belgesinde cevaplanır. Koda yalnızca
  belge yetersizse ya da doğrulamak için inin; çelişki varsa kod doğrudur, belgeyi düzeltin.
- **Geniş tarama için alt ajan** (`Explore`): 5'ten fazla dosya ya da bilinmeyen bir kavram aranıyorsa.
  Görevi şöyle verin: arama sorusu + MAP satırı + "yalnızca `dosya:satır` ve tek cümle döndür, kod parçası
  dökme". Bilinen 1-3 dosyayı okumak için alt ajan açmayın.

## 3. Arama tarifleri

```sh
scripts/outline.sh --tree                  # tüm kaynak dosyalar: satır sayısı + amaç (60 satır)
scripts/outline.sh src/settings.rs         # bir dosyanın çatısı, satır numaralarıyla
scripts/outline.sh src/commands            # klasördeki tüm dosyaların çatısı (büyük olabilir; tek dosya tercih edin)
scripts/outline.sh --find resolve_settings # tanım nerede
scripts/outline.sh --refs BuiltinPost      # hangi dosyalar kullanıyor, kaç kez
git grep -n 'SETTINGS_SCHEMA_VERSION' -- 'src/*' 'docs/*'
git grep -n '\.route(' -- src/web/mod.rs   # panel API uçları
git grep -n 'live_' -- 'src/*.rs'          # canlı (ağa çıkan) testler
```

Çatı çıktısındaki `@testler A-B (N test)` satırı, o dosyanın test modülünün nerede olduğunu söyler;
kodu okurken o aralığı atlayın, test yazarken oraya gidin.

## 4. Değişiklik döngüsü

1. İşi MAP'teki "Birlikte değişenler" listesine karşı tartın: eşlenik dosya var mı?
2. Küçük, odaklı düzenleme yapın. Mevcut kodun yorum yoğunluğuna ve adlandırmasına uyun.
3. **Doğrulama, işin boyutuna göre:**

   | Değişiklik | Komut |
   | --- | --- |
   | Metin, yorum, belge | Çalıştırma gerekmez |
   | Tek modül davranışı | `cargo fmt --all` + `cargo test --locked <modül>` (MAP'teki "Doğrula") |
   | Ayar şeması, platform, ortak hat | `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked` |
   | Panel (`web/ui/`) | `node --check src/web/ui/app.js` + `node --test tests/web-ui.test.cjs` |
   | Dosya ekleme/silme/taşıma | `scripts/outline.sh --check` |

4. Normal testler ağa çıkmaz. `live_` önekli testler `#[ignore]`'dur ve yalnızca istenirse çalıştırılır
   (`cargo test --locked live_ -- --ignored`). Bir canlı test ağ nedeniyle düşerse kodun hatası saymayın;
   bunu raporlayın.
5. Davranış değiştiyse ilgili modül belgesini aynı değişiklikte güncelleyin (CONTRIBUTING kuralı).
6. Rapor: ne değişti, neyi çalıştırdınız, neyi çalıştırmadınız. Çalıştırmadığınızı çalıştı demeyin.

## 5. Bu depoda sık düşülen tuzaklar

- **Dil:** kod yorumları ve kullanıcıya dönen metinler Türkçedir. Yeni kod aynı dili kullanır.
- **Hata metni:** kullanıcıya giden metin sır içermez; ayrıntı yalnızca loga yazılır.
- **Ağ kodu:** her yeni ağ çağrısı zaman aşımı, yanıt boyutu sınırı ve hata davranışı tanımlar.
  Kullanıcı girdisinden gelen URL'ler `safe_http` kurallarından geçer.
- **Platform modülleri** Discord, ayar ya da kart düzeni bilmez; yalnızca `BuiltinPost` döner.
- **Ayar çözümleme iki yerde yaşar:** `settings.rs` ve `web/ui/app.js`. Biri değişirse fixture
  (`tests/fixtures/settings-resolution.json`) ve ikinci uygulama da değişir.
- **Bütçeler belgelidir:** süre ve kapasite sabitlerini değiştirirseniz `docs/ARCHITECTURE.md` tablosunu da düzeltin.
- **Commit mesajı:** `fix(commands): …`, `feat(card): …`, `docs: …`.
- **Çalışma ağacı kirli olabilir:** başkasının yarım değişikliğini geri almayın, kendi dosyalarınızla sınırlı kalın.

## 6. Claude Code (bu belgeyi yazan ajan) nasıl çalışır

İş gelince, bu sırayla, her adımda önce ucuz olanı denerim:

1. **Yönlendirme:** isteği MAP'teki bir satıra eşlerim. Eşleşmezse `scripts/outline.sh --tree` ile dosya
   amaçlarına bakarım, gerekirse `--find`/`--refs` ile kavramı bulurum.
2. **Plan:** birden fazla dosya ya da komut gerektiren işte görev listesi açarım; ilk araç çağrısıyla birlikte.
3. **Bağlam:** modül belgesinin `Değişmezler` bölümü ─► hedef dosyanın çatısı ─► hedef aralık. Dosyanın
   tamamını okumam.
4. **Paralellik:** birbirinden bağımsız okuma ve aramaları tek turda yaparım.
5. **Düzenleme:** `Edit` ile en küçük değişiklik; yeni dosya yalnızca gerçekten gerekiyorsa.
6. **Doğrulama:** §4 tablosundaki en dar komut. Derlemeyi gereksiz yere çalıştırmam.
7. **Kapanış:** belgeyi güncellerim, harita değiştiyse `--check` çalıştırırım, sonucu dürüstçe özetlerim.

Geniş, belirsiz sorularda (ör. "şu özellik nerede kullanılıyor?") kendim dosya dosya gezmem; `Explore`
alt ajanına, yukarıdaki biçimde dar bir görev veririm ve yalnızca `dosya:satır` listesini alırım.
Bağımsız yazma işleri farklı dosyalara dokunuyorsa ayrı ajanlara bölünebilir; aynı dosyaya iki ajan yazmaz.

## 7. Belgeleri güncel tutma

- Yeni kaynak dosya ya da modül belgesi eklediniz mi: MAP.md dosya haritasına ve (belge ise)
  `docs/modules/README.md` dizinine ekleyin, ardından `scripts/outline.sh --check` çalıştırın.
- Check; haritada olmayan dosyayı, haritada olup var olmayan yolu ve dizinde olmayan modül belgesini yakalar.
  Satır numarası saklamadığımız için kod değiştikçe eskimez.
- Modül belgelerinin başlık düzeni sabittir ([docs/modules/README.md](../modules/README.md)): Görev,
  Dosyalar, Girdi ve çıktı, Nasıl çalışır, Değişmezler, Değiştirirken, Testler.
