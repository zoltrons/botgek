# EG-04 — Her hedef dile çeviri

**Öncelik:** P1  
**Hedef:** Embedded'in “translate to any language” iddiasına karşılık, hedef dili yapılandırılabilir,
güvenli ve kota kontrollü ortak bir çeviri akışı sağlamak.

## Mevcut durum ve kanıt

`parser::Platform::supports_translation()` yalnızca Twitter ve Bluesky döndürüyor. `translation.rs`
bayrakları sınırlı bir sabit listeye çeviriyor. Native X akışı `TARGET_LANGUAGE_CODE` ile Türkçe'ye
odaklanıyor; WebUI'daki `auto_translate` de “Türkçe” olarak açıklanıyor. Proxy mesajlarında bayrak
reaksiyonu link domain'ini değiştiriyor; native kart reaksiyonu ise bilinçli olarak yok sayılıyor.

Bu nedenle mevcut özellik “çeviri yok” değildir; fakat “her platform/her hedef dil” sözleşmesini karşılamaz.

## Hedef sözleşme

```json
{
  "translation": {
    "enabled": true,
    "default_target": "tr",
    "allowed_targets": ["tr", "en", "de"],
    "mode": "reaction|automatic|both",
    "provider": "platform|llm|proxy"
  }
}
```

İlk sürümde öneri: `default_target` tek dil, reaction ile kullanıcı seçimi, platform-native translation
başarısızsa seçilmiş provider fallback'i. Dil kodu BCP-47 normalize edilmeli (`pt-BR` gibi bölgesel
kodlar kaybolmamalı).

## Uygulama planı

1. Sabit `emoji_to_lang_code()` listesini registry'ye taşıyın. Emoji→BCP-47 eşlemesi config'ten değil,
   kod içindeki güvenilir tablodan gelsin; kullanıcıdan gelen dil kodu allowlist ve uzunluk sınırından geçsin.
2. `TranslationRequest { source_text, source_lang?, target_lang, platform, message_id, guild_id }` ve
   `TranslationResult { text, detected_lang?, provider, cache_key }` tiplerini ekleyin.
3. Provider abstraction kurun. X'in native çevirisini yalnızca uygun target/endpoint varsa kullanın;
   herhangi bir LLM/HTTP provider için response body, timeout, mention sanitization ve maliyet limiti
   mevcut AI güvenlik kurallarıyla aynı olmalı.
4. Çeviriyi `BuiltinPost` render'ından önce değil, kartın kaynak ve çeviri metnini açıkça ayıran model
   alanıyla uygulayın. Kaynak metni varsayılan olarak kaybetmeyin; UI kuralı “çeviri üstte/kaynak spoiler
   veya alt satır” olarak seçilebilir olmalı.
5. Native kart translation reaction için mesaj edit yerine idempotent yeni kart/edit strategy belirleyin.
   Components V2 mesajlarında mevcut “native message ise return” guard'ını sessizce kaldırmayın.
6. Guild/channel bazlı enabled, default target, allowed targets ve quota'yı EG-02 ile resolve edin.
7. Cache key'e canonical URL + source hash + target + provider + model + prompt/schema sürümünü koyun.
8. Rate limit: kullanıcı, guild ve global bucket; eşzamanlı provider semaphore; karakter/token limiti;
   premium daha yüksek limit istiyorsa EG-07 quota katmanından okuyun.

## Kabul kriterleri

- Admin default hedef dili `en`, `de` veya BCP-47 uyumlu başka bir dile ayarlayabilir.
- Kullanıcı izin verilen flag/reaction ile desteklenen hedef dile çeviri başlatabilir.
- Desteklenmeyen hedef dil kartı değiştirmez; kullanıcıya güvenli kısa hata verir ve log'a secret yazılmaz.
- Kaynak dil bilinmiyorsa provider detection kullanılır veya açık fallback uygulanır.
- Çeviri sonucu @everyone/rol/user mention'ı tetikleyemez.
- Aynı kaynak/hedef için cache çalışır; provider timeout orijinal kartı bozmaz.
- Proxy ve native kartlarda ürün davranışı aynı dokümante edilmiş contract'a uyar.

## Test matrisi

BCP-47 normalization; emoji registry; Turkish/English/no-op; RTL metin; 4.000 char card limit; provider
429/timeout/invalid JSON; mention sanitization; quota exceeded; cache separation by target; native V2
translation update; proxy legacy behavior; WebUI round-trip.

## Karar gerektiren noktalar

“Any language” için kullanılacak provider ve maliyet sahibi belirtilmeden agent API key, vendor veya model
seçmemelidir. Platform-native endpoint yeterli değilse kullanıcıdan provider tercihi/secret yönetimi
kararı alınmalıdır.
