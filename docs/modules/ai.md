# AI sohbeti

## Görev

Bot etiketlendiğinde ya da bir mesajına yanıt verildiğinde Groq (web aramalı) ve Gemini üzerinden kısa
bir yanıt üretir. Yalnızca `GROQ_API_KEY` ya da `GEMINI_API_KEY` tanımlıysa ve etkin ayarda `ai_chat`
modülü açıksa çalışır.

## Dosyalar

| Dosya | İçerik |
| --- | --- |
| `src/ai/mod.rs` | Groq/Gemini istemcisi: istek kurma, sağlayıcı sıralaması ve yedekleme, hata türleri. |
| `src/ai/discord.rs` | Discord'dan bağlam toplama: yanıtlanan mesaj, yanıt zinciri, görseller, sunucu emojileri, takma adlar. |
| `src/handler.rs` | Tetikleme kuralları ve bütçeler (bkz. [handler.md](handler.md)). |

## Girdi ve çıktı

- `ai::discord::gather(...)` bir `ChatPrompt` için bağlamı toplar.
- `ai::query(client, &Keys, &ChatPrompt) -> Result<String, AiError>`. `Keys` iki isteğe bağlı anahtar taşır.
- `ChatPrompt`: model, sistem istemi, kullanıcı istemi, konuşma bağlamı (geçmiş, görseller, emojiler).

## Nasıl çalışır

### İstek

- Sağlayıcılar: Groq `https://api.groq.com/openai/v1/chat/completions`, Gemini
  `https://generativelanguage.googleapis.com/v1beta/openai/chat/completions`. İkisi de OpenAI uyumludur.
  Gövde yalnızca bir `system` ve bir `user` mesajı içerir. Yanıtlanan mesaj ve yanıt zinciri `assistant`
  turu olarak değil, kullanıcı mesajının içine bağlam olarak yazılır. Bazı sohbet şablonları `assistant`
  ile başlayan turları reddeder.
- Sıra: görsel yoksa Groq, sonra Gemini; görsel varsa Gemini, sonra Groq (Groq'a görsel gönderilmez,
  modelleri görsel okuyamaz; istem görsellerin eklenemediğini söyler). Anahtarı olmayan sağlayıcı atlanır. Herhangi bir hatada (ağ, zaman aşımı, 401,
  404, 429, 5xx, boş yanıt) sıradaki sağlayıcı denenir. 401 ve 404 `error!` ile loglanır: yedek yanıt
  verdiği için bozuk anahtar ya da kaldırılmış model aksi halde fark edilmez.
- Groq isteği `tools: [{"type":"browser_search"}]` taşır: model güncel bilgi gerektiğinde kendisi arar.
  Arama turları dakikalık token sınırının (8000) büyük kısmını harcar; üst üste iki aramalı soru 429
  alıp Gemini'ye düşebilir. Yanıttaki atıf işaretleri (`【1†L4-L6】`) `strip_citations` ile silinir.
- Model adı (`ai.model`) yalnızca Groq içindir, 256 karakteri aşamaz, boşluk içeremez. Boş değer ve eski
  OpenRouter adları (`…:free`, `openrouter/…`) `DEFAULT_MODEL`'e çevrilir. Gemini modeli sabittir
  (`GEMINI_MODEL`). Groq model kaldırmayı sık yapar; kaldırılan model 404 verir ve Gemini'ye düşülür.
- Hata gövdesi Groq'ta `{"error":{...}}`, Gemini'de `[{"error":{...}}]` biçimindedir; ikisi de okunur.
- Groq için `reasoning_effort: "low"`, `include_reasoning: false`; `max_tokens` 2048, `temperature` 0.9. `finish_reason: "length"` ile boş içerik
  ya da yanıtın ortasında kopma durumunda yarım metin gönderilmez.
- Sağlayıcı başına zaman aşımı 20 sn; takılan ilk sağlayıcı yedeğe geçişi engellemez (yuva en fazla ~40 sn). Yanıt gövdesi 1 MiB ile, istem ve yanıt metni 8000 karakterle sınırlıdır.
- Süreç genelinde en fazla 2 eşzamanlı istek. Dolunca `AiError::Busy`.

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
- Groq istemi varsayılan olarak saklamaz (hata ayıklama için en fazla 30 gün kayıt tutabilir). Gemini'nin
  ücretsiz katmanı istemleri Google'ın ürün geliştirmesinde kullanabilir; görsel yalnızca Gemini
  anahtarı tanımlıysa gönderilir.

### Hatalar

`AiError` türleri: `Busy`, `InvalidModel`, `Network`, `Timeout`, `RateLimited`, `Auth`,
`Blocked`, `BadRequest`, `NoProvider` (model bulunamadı), `Unavailable` ve düşünmeye bütçe harcama durumu. Kullanıcıya
gösterilen metin türden `handler::ai_error_notice` ile seçilir, ayrıntı yalnızca loga yazılır.

## Değişmezler

- Kimlik bilgisi, gerçek kullanıcı adı ya da Discord CDN adresi sağlayıcıya gönderilmez.
- Model çıktısı `sanitize_mentions` olmadan Discord'a gitmez.
- Hata metinleri sağlayıcı ayrıntısı sızdırmaz (`ai_error_notice_never_leaks_provider_details`).
- Botun kendi kartlarına verilen yanıtlar AI'ı tetiklemez.

## Değiştirirken

- Yeni bağlam kaynağı: `discord.rs` içinde takma adlandırma ve kırpma sınırlarından geçirin, sınırı sabit
  yapın.
- Yeni hata türü: `AiError`, `from_code` ve `handler::ai_error_notice` birlikte güncellenir.
- Varsayılan model ve sistem istemi `AiSettings` içindedir, bkz. [settings.md](settings.md).

## Testler

`cargo test --locked ai`. Takma adlandırma, medya vekili URL'si ve hata ayrıştırma için birim testler var.
