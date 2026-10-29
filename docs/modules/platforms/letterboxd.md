# Letterboxd

## Görev

Letterboxd film, kullanıcı incelemesi ve kullanıcı profili bağlantılarını Components V2 kartlarına çevirir.
Üç kart türü, üç vurgu rengi vardır.

## Dosyalar

`builtin/letterboxd.rs`.

## Girdi ve çıktı

`letterboxd::fetch(client, &ParsedPost) -> Result<BuiltinPost, String>`: `custom_card` doludur.

## Nasıl çalışır

Veri anahtar gerektirmeyen herkese açık sayfalardan okunur:

| Kart | Sayfa | Kaynak | Renk |
| --- | --- | --- | --- |
| Film | `/film/{slug}/` | JSON-LD `Movie` (başlık, poster, yönetmen, oyuncular, tür, ülke, süre, açıklama, `aggregateRating`). HTML'den slogan, özgün ad, yıl, fragman, IMDb ve TMDB bağlantıları. | Yeşil `0x00E054` |
| İnceleme | `/{kullanıcı}/film/{slug}/` | JSON-LD `Review` (yazar, puan, metin, tarih). HTML'den beğeni sayısı, kalp, yeniden izleme, izleme tarihi, spoiler uyarısı. | Turuncu `0xFF8000` |
| Profil | `/{kullanıcı}/` | JSON-LD yok. HTML'den görünen ad, avatar, rozet (Pro/Patron/HQ), bio, istatistikler, konum, harici bağlantılar, favori filmler, son etkinlik. | Mavi `0x40BCF4` |

- Spoiler uyarılı incelemelerde metin `||...||` arkasındadır.
- Açıklama (`SYNOPSIS_LIMIT`) 600 karakterle sınırlıdır.
- `boxd.it` kısa bağlantıları hedefe çözülür. Yanıt engellenmiş olsa bile varılan adres hedefi belirler.
- Cloudflare, profil ve numaralı inceleme sayfalarını sıradan User-Agent'lara 403 ile kapatır. Discord'un
  bağlantı botu tanıtıcısı bu kuraldan muaftır, bot da aynı tanıtıcıyı kullanır ve sonuna kendi adını ekler
  (`LETTERBOXD_USER_AGENT`).
- İnceleme sayfası yine okunamazsa kart, incelemenin kime ait olduğunu bağlantısıyla belirten film kartına
  düşer.
- Kart emojileri `Emoji` sabitleridir (yıldız, yarım yıldız, kalp, yeniden izleme, vb.).

## Değişmezler

- Kullanıcı User-Agent'ı ve bot adı birlikte gönderilir. Tanıtıcıyı değiştirirken 403 riskini göz önüne alın.
- Metinler kaçışlanır, spoiler gizlemesi korunur.
- Karta özgü emoji sabitleri değişirse Discord'da emojinin gerçekten var olduğunu doğrulayın.

## Değiştirirken

HTML ayrıştırması kırılgandır. Yeni alan eklerken gerçek sayfa fixture'larıyla test edin. Üç kart türü
farklı dallardır, birini değiştirirken diğerlerinin testlerini çalıştırın.

## Testler

`cargo test --locked letterboxd`. Dosya büyüktür (yaklaşık 2200 satır), testlerin çoğu aynı dosyadadır.
