# Katkıda Bulunma

Botgek'e katkıda bulunmak istediğiniz için teşekkürler. Bu proje MIT lisanslıdır ve katkılar aynı
lisans altında kabul edilir.

## Başlamadan önce

- Hata bildirimi veya özellik önerisi için **GitHub Issues** kullanın. Yeni platform veya büyük davranış
  değişiklikleri için önce bir issue açıp yaklaşımı konuşun.
- Güvenlik açıklarını **herkese açık issue olarak açmayın**; [SECURITY.md](SECURITY.md) içindeki özel
  bildirim yolunu kullanın.
- Sorunları Türkçe veya İngilizce yazabilirsiniz.

## Geliştirme ortamı

Kurulum, test komutları ve proje yapısı: [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

Kısa özet:

```sh
git clone https://github.com/zoltrons/botgek.git
cd botgek
cp .env.example .env
cargo test --locked
```

## Pull request akışı

1. Değişikliğinizi açıklayan bir issue seçin veya açın.
2. `main` dalından kısa ömürlü bir dal oluşturun.
3. Değişikliği ve testleri ekleyin; davranış değişiyorsa ilgili belgeyi güncelleyin.
4. Aşağıdaki kontrolleri çalıştırın.
5. PR açıklamasında neyi, neden değiştirdiğinizi ve nasıl test ettiğinizi belirtin.

### Kontrol listesi

- [ ] `cargo fmt --all --check` geçiyor.
- [ ] `cargo clippy --all-targets --locked -- -D warnings` geçiyor.
- [ ] `cargo test --locked` geçiyor.
- [ ] Panel değiştiyse `node --check src/web/ui/app.js` ve `node --test tests/web-ui.test.cjs` geçiyor.
- [ ] Yeni davranış için test eklendi veya gerekçesi açıklandı.
- [ ] Kullanıcıya görünen değişiklikler README/docs içinde güncellendi.
- [ ] Token, çerez, webhook URL'si veya kişisel veri eklenmedi.
- [ ] Commit mesajı depo geleneğine uyuyor (`fix(...): …`, `feat(...): …`, `docs: …`).

## Kod beklentileri

- Biçimlendirme `rustfmt` ile, lint'ler Clippy `-D warnings` ile zorunludur.
- Ağ erişiminde zaman aşımı, yanıt boyutu sınırı ve hata davranışı tanımlanmalıdır.
- Kullanıcı girdisinden gelen URL'lerde public adres ve yönlendirme doğrulaması atlanmamalıdır.
- Yeni bağımlılık ekliyorsanız gerekçesini ve alternatiflerini PR'da belirtin.
- Testler ağdaki sosyal medya servislerine bağlanmamalıdır; loopback fixture'ları kullanın.
- Performans iddiaları ölçümle desteklenmelidir; ölçülmemiş yüzdeler belgelenmemelidir.

## Commit mesajları

Depo Conventional Commits benzeri bir düzen kullanır:

```text
fix(commands): /dönüştür komutunu DM'lerde görünür yap
feat(card): topluluk notunu karta ekle
docs: kurulum adımlarını sadeleştir
```

Kapsam (`commands`, `card`, `twitter`, `reddit`, `web`, `ci`, `docs`) değişikliğin dokunduğu alanı
belirtir.

## Lisans

Katkı göndererek, katkınızın [MIT](LICENSE) lisansı altında yayınlanmasına izin vermiş olursunuz.
