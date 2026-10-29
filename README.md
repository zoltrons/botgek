# Botgek

[![CI](https://github.com/zoltrons/botgek/actions/workflows/build-release.yml/badge.svg)](https://github.com/zoltrons/botgek/actions/workflows/build-release.yml)
[![Bağımlılık denetimi](https://github.com/zoltrons/botgek/actions/workflows/quality.yml/badge.svg)](https://github.com/zoltrons/botgek/actions/workflows/quality.yml)
[![Lisans: MIT](https://img.shields.io/badge/lisans-MIT-blue.svg)](LICENSE)

Discord'da paylaşılan sosyal medya bağlantılarını Components V2 kartlarına çeviren, kendi sunucunuzda
çalıştırdığınız bir bot. Rust ile yazıldı.

## Desteklenen platformlar

| Platform | Bağlantı türü |
| --- | --- |
| X (Twitter) | Gönderi, alıntı, medya |
| Instagram | Gönderi, Reels |
| Reddit | Gönderi, video |
| Bluesky | Gönderi |
| TikTok | Video, fotoğraf |
| Spotify | Parça, albüm, çalma listesi, sanatçı |
| Twitch | Klip |
| Pixiv | Çizim |
| Tumblr | Gönderi |
| Facebook | Gönderi, video, reel |
| YouTube | Topluluk gönderisi |
| Letterboxd | Film, inceleme, profil |

## Özellikler

- Bağlantıyı tek bir karta çevirir. Kart üretilemezse orijinal mesaj olduğu gibi kalır.
- Modüller, rol filtreleri ve çeviri ayarları sunucu ve kanal bazında web panelinden yönetilir.
- X gönderilerini istenen dile çevirir.
- Hassas içerik varsayılan olarak spoiler olur.
- OpenRouter ile çalışan isteğe bağlı bir AI sohbeti vardır.
- Geliştirme ve hata ayıklama için `/mcp` adresinde bir [MCP sunucusu](docs/MCP.md) sunar.

## Kurulum

Hazır imaj GitHub Container Registry'de yayınlanır (yalnızca `linux/arm64`):

```sh
docker pull ghcr.io/zoltrons/botgek:latest
```

`docker-compose.yml` içindeki `image:` satırının yorumunu kaldırıp `ghcr.io/zoltrons/botgek:latest` yazın,
sonra:

```sh
git clone https://github.com/zoltrons/botgek.git
cd botgek
cp .env.example .env
# .env içinde DISCORD_TOKEN ve ADMIN_TOKEN doldurun
docker compose up -d
```

Kaynaktan derlemek için `image:` satırını yorumda bırakıp `docker compose up -d --build` çalıştırın.
Docker olmadan `cargo run --locked` yeterlidir (Rust 1.88 veya üstü gerekir).

Panel `http://127.0.0.1:3000` adresinde açılır. Token üretmek için `openssl rand -hex 32` kullanabilirsiniz.

### Discord ayarları

1. [Discord Developer Portal](https://discord.com/developers/applications)'da bir uygulama ve bot oluşturun.
2. Bot sayfasında Message Content Intent'i açın.
3. Botu şu izinlerle davet edin: Send Messages, Embed Links, Read Message History, Add Reactions.
   Orijinal mesajları silecekse Manage Messages, webhook ile gönderecekse Manage Webhooks da ekleyin.
4. Davet bağlantısında `applications.commands` kapsamı olmalı.

Ortam değişkenleri ve ayrıntılar: [docs/CONFIGURATION.md](docs/CONFIGURATION.md).

## Komutlar

| Komut | Ne yapar |
| --- | --- |
| `/dönüştür link:<bağlantı>` | Bağlantıyı dönüştürür. DM'lerde de çalışır. |
| Mesaja sağ tık, Apps, Dönüştür | O mesajdaki bağlantıyı dönüştürür. |
| `/gif-yap link:<bağlantı>` | X gönderisindeki videoyu GIF'e çeviren bağlantıyı gönderir. |
| `/uyum kişi:<@biri>` | İki kişinin uyum kartı. |
| `/düello rakip:<@biri>` | Taş kağıt makas. |
| `/sayı-tahmini` | Sayı tahmin oyunu. |
| `/8top soru:<soru>` | Sihirli 8 top. |
| `/bomba rakip:<@biri>` | Kablo kesme oyunu. |
| `/refleks` | Reaksiyon hızı oyunu. |
| `/fal` | Kahve falı. |

Kartın altına 🗑️ tepkisi bırakarak kartı paylaşan kişi ya da Manage Messages yetkisi olan biri kartı silebilir.

## Teşekkürler

- [FxEmbed](https://github.com/FxEmbed/FxEmbed): X verisi FxTwitter API'sinden gelir, GIF dönüştürme FxEmbed servisini kullanır.
- [vxTwitter](https://github.com/dylanpdx/BetterTwitFix): FxTwitter'ın medyayı bozduğu gönderiler için yedek veri kaynağı.

## Belgeler

- [Yapılandırma](docs/CONFIGURATION.md)
- [Mimari](docs/ARCHITECTURE.md)
- [Modül belgeleri](docs/modules/README.md)
- [İşletim](docs/OPERATIONS.md)
- [Geliştirme](docs/DEVELOPMENT.md)
- [MCP sunucusu](docs/MCP.md)

## Katkı

Hata ve öneriler için GitHub Issues'ı kullanın. Pull request göndermeden önce
[CONTRIBUTING.md](CONTRIBUTING.md) dosyasına bakın. Güvenlik açıklarını issue olarak açmayın,
[SECURITY.md](SECURITY.md) içindeki yoldan bildirin.

## Lisans

[MIT](LICENSE)
