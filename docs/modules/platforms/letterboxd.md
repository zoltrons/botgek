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

- Spoiler uyarılı incelemelerde metin `||...||` arkasındadır; tek paragrafa toplanır ve `SPOILER_LIMIT` (280)
  karakterde kesilip "Devamını oku" bağlantısı alır.
- İnceleme/günlük kartı tek bölümdür (Filmlinkd düzeni): `**Ad** izledi|inceledi`, bağlantılı film adı (`###`),
  kayda bağlı gün (bu yılsa yılsız) + yıldız + yeniden izleme + kalp; yanında poster. Metin ayraçla altta.
  Adın yanında rozet logosu (Patron tacı, Pro). "İncelemeyi/Kaydı aç" ve "Filmi aç" butonları kartın dışında,
  hemen altında (`CustomCard::buttons`).
- Kullanıcının seçtiği özel poster (`preferredAlternativePosterId`) kayıt sayfasından çözülür. Takip akışında RSS
  yalnızca varsayılan posteri verdiği için `DiaryEntry::post` gönderilecek kaydın sayfasını ayrıca okur.
- Film kartında künye tek blokta: özgün ad, yönetmen, oyuncular (`MAX_CAST` = 3), ardından tür/süre/ülke satırı.
- Açıklama (`SYNOPSIS_LIMIT`) 600 karakterle sınırlıdır.
- `boxd.it` kısa bağlantıları hedefe çözülür. Yanıt engellenmiş olsa bile varılan adres hedefi belirler.
- Cloudflare, profil ve numaralı inceleme sayfalarını sıradan User-Agent'lara 403 ile kapatır. Discord'un
  bağlantı botu tanıtıcısı bu kuraldan muaftır, bot da aynı tanıtıcıyı kullanır ve sonuna kendi adını ekler
  (`LETTERBOXD_USER_AGENT`).
- İnceleme sayfası yine okunamazsa kart, incelemenin kime ait olduğunu bağlantısıyla belirten film kartına
  düşer.
- Kart emojileri `Emoji` sabitleridir (yıldız, yarım yıldız, kalp, yeniden izleme, vb.).

## Takip akışı

`/letterboxd takip` ile bir kanal bir hesabı takip eder; yeni günlük kayıtları kanala inceleme kartı olarak
gelir. Dosyalar: `src/letterboxd_feed.rs` (yoklama), `src/commands/letterboxd.rs` (komut), `store.rs`
(`letterboxd_follows`).

- Kaynak `/{kullanıcı}/rss/` akışıdır (`diary`). Öğeler `letterboxd-review-{id}` ve `letterboxd-watch-{id}`
  guid'leriyle ayrılır; liste paylaşımları atlanır. Akış film adı, yıl, puan, beğeni, yeniden izleme, izleme
  günü, poster ve inceleme metnini taşır; ek sayfa istenmez. Kart `review_card`'dır: metinsiz (yalnızca
  izleme) kayıtta "Letterboxd günlüğü" ve "Kaydı aç" yazar.
- Yeni kayıt: kimliği kanalın `last_entry` değerinden büyük olan. Tur başına kanal başına en fazla 5 kayıt
  eskiden yeniye gönderilir.
- Hız: istekler tek tek, aralarında en az 1 sn; her hesap en sık 30 sn'de bir yoklanır. RSS `no-store`'dur ve
  hız sınırı başlığı yoktur; ölçümde ~2,3 istek/sn'de 429 görülmedi. 429/403/503'te aralık ikiye katlanır
  (en fazla 60 sn), başarıda yarıya iner.
- Gönderim: Discord reddederse (kanal silinmiş, yetki yok) bekleyen kayıtlar atlanır; geçici hatada kalanlar
  sonraki turda denenir; belirsiz sonuç yeniden gönderilmez. Kart `allowed_mentions: {parse: []}` ile gider.

## Değişmezler

- Kullanıcı User-Agent'ı ve bot adı birlikte gönderilir. Tanıtıcıyı değiştirirken 403 riskini göz önüne alın.
- Metinler kaçışlanır, spoiler gizlemesi korunur.
- Karta özgü emoji sabitleri değişirse Discord'da emojinin gerçekten var olduğunu doğrulayın.

## Değiştirirken

HTML ayrıştırması kırılgandır. Yeni alan eklerken gerçek sayfa fixture'larıyla test edin. Üç kart türü
farklı dallardır, birini değiştirirken diğerlerinin testlerini çalıştırın.

## Testler

`cargo test --locked letterboxd`. Dosya büyüktür (yaklaşık 2200 satır), testlerin çoğu aynı dosyadadır.
