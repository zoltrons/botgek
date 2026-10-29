# AI sohbeti

## Görev

Bot etiketlendiğinde ya da bir mesajına yanıt verildiğinde OpenRouter üzerinden kısa bir yanıt üretir.
Yalnızca `OPENROUTER_API_KEY` tanımlıysa ve etkin ayarda `ai_chat` modülü açıksa çalışır.

## Dosyalar

| Dosya | İçerik |
| --- | --- |
| `src/ai/mod.rs` | OpenRouter istemcisi: istek kurma, model normalizasyonu, yedek zinciri, hata türleri. |
| `src/ai/discord.rs` | Discord'dan bağlam toplama: yanıtlanan mesaj, yanıt zinciri, görseller, sunucu emojileri, takma adlar. |
| `src/handler.rs` | Tetikleme kuralları ve bütçeler (bkz. [handler.md](handler.md)). |

## Girdi ve çıktı

- `ai::discord::gather(...)` bir `ChatPrompt` için bağlamı toplar.
- `ai::query_openrouter(client, api_key, &ChatPrompt) -> Result<String, AiError>`.
- `ChatPrompt`: model, sistem istemi, kullanıcı istemi, konuşma bağlamı (geçmiş, görseller, emojiler).

## Nasıl çalışır

### İstek

- Tek `POST https://openrouter.ai/api/v1/chat/completions`. Gövde yalnızca bir `system` ve bir `user`
  mesajı içerir. Yanıtlanan mesaj ve yanıt zinciri `assistant` turu olarak değil, kullanıcı mesajının
  içine bağlam olarak yazılır. Bazı sohbet şablonları `assistant` ile başlayan turları reddeder.
- Görsel varsa `user` içeriği metin ve `image_url` parçalarına dönüşür.
- Model adı normalize edilir (`openrouter:` öneki ve model sayfası adresi kabul edilir), 256 karakteri
  aşamaz, boşluk içeremez.
- Yedek: `models: [model, "openrouter/free"]`. Birincil model hata verirse OpenRouter aynı istekte ücretsiz
  yönlendiriciye geçer. Model zaten `openrouter/free` ise yedek listesi gönderilmez.
- Geçici hatalarda (ağ, zaman aşımı, 429, 5xx, boş yanıt) istemci süre bütçesi izin verirse ücretsiz
  yönlendiriciyle bir kez daha dener. 400, 401, 402 ve 403 tekrar denenmez.
- `reasoning.exclude: true`, `max_tokens` 2048, `temperature` 0.9. `finish_reason: "length"` ile boş içerik
  ya da yanıtın ortasında kopma durumunda yarım metin gönderilmez.
- Sağlayıcı zaman aşımı 20 sn. Yanıt gövdesi 1 MiB ile, istem ve yanıt metni 8000 karakterle sınırlıdır.
- Süreç genelinde en fazla 2 eşzamanlı istek. Dolunca `AiError::Busy`.
- İstek `HTTP-Referer` ve `X-OpenRouter-Title` ile uygulama künyesini taşır.

### Bağlam toplama (`discord.rs`)

- Yanıtlanan mesaj en fazla 1500 karakter (embed metinleri dahil, en fazla 3 embed, her biri 800 karakter).
  Ondan geriye en fazla 4 mesajlık yanıt zinciri, her biri 600 karakter.
- Görseller: en fazla 3, her biri 3 MiB. Yalnızca Discord alan adlarından, medya vekilinden en uzun kenarı
  1024 px'e küçültülmüş WebP olarak indirilir ve `data:` adresi olarak gömülür. Sağlayıcı Discord
  bağlantısını görmez. İndirme zaman aşımı 8 sn.
- Sunucunun bot tarafından kullanılabilen etkin emojilerinden en fazla 60'ı sistem istemine `:ad:` olarak
  yazılır. Model çıktısındaki `:ad:` kodları gönderimden önce `<:ad:id>` biçimine çevrilir, bir yanıtta
  ikiden fazla özel emoji varsa fazlası atılır.

### Gizlilik

- İstekte `user` alanı yoktur. Kullanıcı kimliği, adı ve takma adı sağlayıcıya gitmez. Konuşanlar ve
  metindeki etiketler tutarlı takma adlara çevrilir (`Kullanıcı`, `Kişi A`, ...). Rol ve kanal etiketleri
  `@rol` ve `#kanal` olur. Dosya adları gönderilmez, yalnızca ek sayısı söylenir.
- `provider.data_collection: "deny"`: istem yalnızca saklamayan ve eğitimde kullanmayan sağlayıcılara gider.
  Böyle sağlayıcı yoksa OpenRouter 404 döner (`AiError::NoProvider`) ve kullanıcıya ayrı bildirim çıkar.

### Hatalar

`AiError` türleri: `Busy`, `InvalidModel`, `Network`, `Timeout`, `RateLimited`, `Auth`, `Credits`,
`Blocked`, `BadRequest`, `NoProvider`, `Unavailable` ve düşünmeye bütçe harcama durumu. Kullanıcıya
gösterilen metin türden `handler::ai_error_notice` ile seçilir, ayrıntı yalnızca loga yazılır.

## Değişmezler

- Kimlik bilgisi, gerçek kullanıcı adı ya da Discord CDN adresi sağlayıcıya gönderilmez.
- Model çıktısı `sanitize_mentions` olmadan Discord'a gitmez.
- Hata metinleri sağlayıcı ayrıntısı sızdırmaz (`ai_error_notice_never_leaks_provider_details`).
- Botun kendi kartlarına verilen yanıtlar AI'ı tetiklemez.

## Değiştirirken

- Yeni bağlam kaynağı: `discord.rs` içinde takma adlandırma ve kırpma sınırlarından geçirin, sınırı sabit
  yapın.
- Yeni hata türü: `AiError`, `from_code`, `retryable` ve `handler::ai_error_notice` birlikte güncellenir.
- Varsayılan model ve sistem istemi `AiSettings` içindedir, bkz. [settings.md](settings.md).

## Testler

`cargo test --locked ai`. Takma adlandırma, medya vekili URL'si ve hata ayrıştırma için birim testler var.
