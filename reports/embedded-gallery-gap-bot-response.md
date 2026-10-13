# EG-06 — Bot mesajlarına yanıt verebilme

**Öncelik:** P1  
**Hedef:** Embedded Premium sayfasındaki “Respond to Bots” davranışını, Botgek'in mevcut self-loop ve
kart tekrarını engelleyen güvenlikleri bozmadan seçilebilir hale getirmek.

## Mevcut durum ve kanıt

`Handler::message()` ilk adımda `if msg.author.bot { return; }` ile tüm bot mesajlarını eliyor. Bu,
Botgek kartlarının tekrar işlenmesini de doğal olarak engelliyor; guard'ı doğrudan kaldırmak sonsuz
dönüşüm döngüsü ve webhook echo riski yaratır. `is_card_message()`/kalıcı kart kaydı native kartların
tanınması için zaten var, fakat “başka botları işle” için allow policy yok.

## Hedef sözleşme

```text
respond_to_bots = false (default)

if true:
  process only non-Botgek bot messages
  ignore Botgek bot user id
  ignore Botgek webhook id/name/marker
  require supported link and module/channel/role policy
  enforce per-message idempotency
```

Premium aktif değilse ayar kabul edilmemeli veya feature gate tarafından no-op yapılmalıdır; bu karar
EG-07 ile ortaklaştırılmalı.

## Uygulama planı

1. `EngineSettings` veya ayrı `PolicySettings` içine `respond_to_bots: bool` ekleyin; default false.
2. Botgek kimliğini READY sonrasında state/config içinde tutun. Botgek webhook'larını yalnızca name'e
   göre değil, kayıtlı webhook ID'leri ve güvenilir card marker/SQLite message ID ile tanıyın.
3. `handler.rs` erken return'ünü şu güvenli karar fonksiyonuna değiştirin:
   `should_process_message(author, is_botgek, respond_to_bots, is_system/webhook) -> bool`.
   Discord system/webhook ve bilinmeyen bot ayrımı açıkça test edilmelidir.
4. İşlenen message ID'lerini bounded TTL store'a yazın. Restart sonrasında kritik idempotency gerekiyorsa
   SQLite'a yazın; belirsiz Discord POST sonucunda kör retry yapmayın.
5. Bot mesajındaki mention/reply ile AI akışının yanlış tetiklenmesini engelleyin. Önce link conversion,
   sonra AI ayrımı mevcut semantiği koruyacak şekilde tekrar gözden geçirin.
6. Native kart üretiminde Botgek marker'ını daraltın (Components V2 + marker + kayıtlı webhook/message).
   Genel “empty content + no embed” imzasına güvenmeyin.
7. WebUI'da risk açıklaması, default off, guild/channel override ve premium gate gösterin.

## Kabul kriterleri

- Default olarak başka botun desteklenen linki işlenmez; mevcut davranış korunur.
- Opt-in açıkken başka botun linki bir kez dönüştürülür.
- Botgek bot mesajı, Botgek webhook kartı ve aynı kartın edit/reaction event'i tekrar işlenmez.
- İki bot birbirinin ürettiği kartı karşılıklı dönüştürerek loop oluşturamaz.
- Message ID duplicate event'i tek kart üretir.
- Channel/guild role policy, cooldown ve module disable hâlâ uygulanır.

## Test matrisi

Bot user false/true; Botgek bot user; Botgek webhook; unknown webhook; Components V2 empty message;
duplicate message ID; two bot loop fixture; bot reply + AI mention; opt-in guild/channel resolution;
restart/idempotency store; concurrent duplicate events.

## Rollback

Production config flag'i false'a çekmek eski güvenli davranışı geri getirmelidir. Guard kaldırılmadan önce
marker ve idempotency testleri zorunludur.
