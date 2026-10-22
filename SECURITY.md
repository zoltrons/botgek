# Güvenlik Politikası

## Desteklenen sürümler

Güvenlik düzeltmeleri `main` dalına ve en son sürüm etiketine uygulanır. Eski sürümler için geriye dönük
yama taahhüdü yoktur; lütfen güncel sürümü kullanın.

## Açık bildirimi

**Güvenlik açıklarını herkese açık issue, tartışma veya PR olarak açmayın.**

Bildirim için deponun **Security** sekmesindeki **Report a vulnerability** formunu kullanın. Bu form
kapalıysa `@zoltrons` ile GitHub profili üzerinden özel olarak iletişime geçin.

Bildirime şunları ekleyin:

- Etkilenen sürüm veya commit,
- Sorunun türü (kimlik doğrulama atlatma, SSRF, XSS, komut enjeksiyonu, hizmet reddi, sır sızıntısı …),
- Yeniden üretme adımları veya kavram kanıtı,
- Olası etki ve önerdiğiniz düzeltme.

Tokenları, çerezleri ve kişisel verileri maskeleyin. Yanıt süresi en iyi çaba esaslıdır; düzeltme
yayınlanana kadar ayrıntıları gizli tutun.

## Kapsam

Özellikle şu alanlar inceleme kapsamındadır:

- Yönetim paneli kimlik doğrulaması, oturum ve CSRF kontrolleri,
- Kullanıcı girdisinden gelen URL'lerde SSRF ve DNS rebinding koruması,
- Kart render yolunda XSS, markdown ve mention enjeksiyonu,
- Ayar dosyası ve göç mantığında yetkisiz erişim veya veri kaybı,
- Discord webhook impersonation ve silme yetkisi kontrolleri,
- Hizmet reddine yol açabilecek sınırsız istek, bellek veya eşzamanlılık.

## Dağıtım sertleştirme listesi

- Paneli internete doğrudan açmayın; `127.0.0.1` veya SSH tüneli/HTTPS ters proxy kullanın.
- HTTPS arkasında `WEB_SECURE_COOKIE=true` ayarlayın. Uygulama TLS sonlandırmaz.
- `ADMIN_TOKEN` için en az 32 karakterlik rastgele bir sır üretin (`openssl rand -hex 32`) ve düzenli
  olarak döndürün.
- Botu en az yetkiyle davet edin. **Manage Webhooks** yalnızca impersonation gerekiyorsa verilmelidir.
- `.env` dosyasını Git dışında tutun ve dosya izinlerini kısıtlayın.
- `docker compose` kullanıyorsanız non-root kullanıcı, salt okunur kök dosya sistemi ve `cap_drop: ALL`
  ayarlarını koruyun.
- Logları özel tutun; dönüşüm logları gönderi URL'leri içerebilir.
- Bağımlılık denetimini (`cargo audit` / `quality.yml`) takip edin; RUSTSEC uyarılarını sessizce
  susturmayın.
- SQLite ve ayar dosyalarını düzenli olarak yedekleyin; yedeği public bir yere koymayın.
