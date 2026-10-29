# Handler

## Görev

Discord gateway olaylarını karşılar: mesajlar, etkileşimler, tepkiler. Link dönüştürmeyi kuyruğa verir, AI
sohbetini tetikler, 🗑️ tepkisiyle kart silmeyi yönetir.

## Dosyalar

`src/handler.rs`: `Handler` yapısı ve serenity `EventHandler` uygulaması.

## Girdi ve çıktı

- Girdi: serenity olayları (`ready`, `message`, `interaction_create`, `reaction_add`, `shard_stage_update`).
- Çıktı: `converters::enqueue_message` çağrısı, AI yanıtı mesajı, silinen kart mesajı.

`Handler` şunları taşır: paylaşılan `AppSettings` (`Arc<RwLock<_>>`), `Config`, paylaşılan `reqwest::Client`
ve `Http`.

## Nasıl çalışır

### `message`

1. Botun kendi mesajları, webhook mesajları ve `Regular`/`InlineReply` dışındaki türler atlanır.
2. Botun kendi kartı olan mesajlar (`builtin::is_card_message`) işlenmez.
3. Mesajda `://` yok, bot etiketlenmemiş ve yanıt değilse hiçbir akış tetiklenemez. Ayar çözümleme atlanır.
4. Etkin ayarlar `get_message_settings(guild, channel, thread_parent)` ile çözülür.
5. Yazar botsa: `respond_to_bots` açık, mesajda desteklenen girdi var ve `claim_bot_source` (5 dk içinde
   aynı mesaj bir kez) geçerse dönüşüm kuyruğa girer. Taklit zorla kapatılır ve AI hiç çalışmaz.
6. Normal kullanıcı: önce `enqueue_message`, sonra `ai_chat` modülü açıksa `reply_with_ai`.

### AI tetikleme

`should_trigger_ai_chat` kuralları sırayla:

1. Mesaj botun bir link dönüştürme mesajına yanıtsa tetiklenmez (etiket olsa bile).
2. Botun AI sohbet ya da başka bir mesajına yanıtsa tetiklenir.
3. Yanıt değilse ama bot etiketliyse tetiklenir.
4. Diğer durumlar tetiklemez.

Ucuz kontroller (rol, boş istem, kapasite yuvası) bekleme süresi ve bütçe tüketilmeden önce yapılır.
Sınırlar:

| Sınır | Değer |
| --- | --- |
| Kullanıcı başına bekleme | 3 sn |
| Bütçe penceresi | 10 sn |
| Pencere başına global istek | 20 |
| Pencere başına sunucu isteği | 5 |
| Süreç genelinde eşzamanlı AI isteği | 2 (`ai::has_capacity`) |

Model çıktısı gönderilmeden önce `sanitize_mentions` ile `@everyone`, `@here` ve rol etiketleri sıfır
genişlikli boşlukla etkisizleştirilir. Sağlayıcı hataları `ai_error_notice` ile sır içermeyen sabit
metinlere çevrilir.

### `reaction_add`

🗑️ tepkisi kartı siler. Yetkili olanlar: kartı paylaşan kullanıcı (`store` kaydından) ya da kanalda
mesaj yönetme izni olan üye. Thread izinleri üst kanaldan gelir; `can_manage_messages` thread'leri
`guild.threads` içinde de arar.

### `interaction_create`

Slash, mesaj komutu, bileşen ve modal etkileşimleri `commands` modülüne yönlendirilir. Bkz.
[commands.md](commands.md).

## Değişmezler

- Botun kendi kartları ve yanıtları AI sohbetini tetiklemez. `is_converted_message` bunu hem eski proxy
  biçimi (`<@id> [**Platform**](<url>)`) hem native kartlar için denetler.
- Yanıt referansı `soft_reply` ile kurulur: hedef mesaj silinmişse Discord isteği reddetmez, yanıt normal
  mesaj olarak gider.
- Bot kaynaklı dönüşümde taklit yoktur.
- `discord_thread_parent` thread ayarlarının üst kanaldan miras alınmasını sağlar. Kategori kapsamı yoktur.

## Değiştirirken

- Yeni gateway olayı: `EventHandler` uygulamasına ekleyin, gerekirse `main.rs` içindeki intent listesini
  genişletin.
- AI sınırlarını değiştirirken [ai.md](ai.md) içindeki istemci sınırlarıyla çakışmadığına bakın.
- Mesaj akışının ilk adımlarına pahalı iş koymayın. Sıradan sohbet mesajı için erken dönüş bilinçli bir
  performans kararıdır.

## Testler

`cargo test --locked handler`: AI bekleme süresi, mention temizleme, dönüşüm mesajı tanıma, hata
bildirimlerinin sızıntısız olması.
