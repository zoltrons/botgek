# EG-02 — Kanal bazlı ayar ve politika override'ları

**Öncelik:** P0  
**Hedef:** Guild genel ayarını bozmadan Discord text/news/forum/thread kanallarında Embedded benzeri
kanal-spesifik dönüşüm ve politika ayarları sağlamak.

## Mevcut durum ve kanıt

`AppSettings` ve `GuildSettings` global + sparse guild override kullanıyor. Override bölümleri yalnızca
`modules`, `role_filters`, `ai`, `engine` (`src/settings.rs`, `src/web/ui/app.js`).
`handle_message()` ayarı `msg.guild_id` üzerinden alıyor; `msg.channel_id` politika seçiminde yok.
WebUI yalnızca sunucu seçiyor ve `/api/discord/roles` rol verisi döndürüyor; kanal listesi endpoint'i yok.

## Hedef veri modeli

Mevcut sparse semantiği koruyun; yeni top-level blok ekleyin:

```json
{
  "guilds": {
    "123": {
      "channels": {
        "456": {
          "modules": {"twitter": true},
          "policy": {"ignore": false, "sensitive_mode": "hide"}
        }
      }
    }
  }
}
```

Gerçek JSON şeması Rust serde tipleriyle belirlenir. Kanal override “tam kopya” değil, aynı mevcut
blok-bazlı sparse kuralla yalnızca sabitlenen blokları yazmalıdır. Bilinmeyen guild/channel ID'leri
ve geçersiz bloklar kaydedilmemelidir.

Önerilen çözümleme sırası:

```text
global → guild override → channel override
```

Thread/forum child channel için parent channel politikası açıkça tanımlanmalıdır; varsayılan öneri:
önce thread override, yoksa parent override, yoksa guild/global.

## Uygulama planı

1. `ChannelOverride`, `GuildOverride.channels` ve schema version migration ekleyin. Eski settings dosyası
   migration olmadan aynı sonucu vermelidir.
2. Rust `resolve_settings(global, guild, channel)` fonksiyonunu tek kaynak yapın. `get_guild_settings`
   yerine mesaj context'i alan `resolve_message_settings(guild_id, channel_id, parent_id)` ekleyin.
3. `handle_message`, AI trigger ve reaction translation akışlarını aynı resolved policy'den besleyin;
   yalnızca link conversion'a kanal override uygulayıp AI/translation'ı unutmayın.
4. WebUI'ya authenticated `GET /api/discord/channels` endpoint'i ekleyin. Yalnızca botun erişebildiği
   guild'leri, channel type/name/parent bilgisini döndürün; 30 saniyelik cache ve bounded concurrency
   kullanın.
5. UI scope selector'ı `global / guild / channel` olarak genişletin. Parent/child ve override badge'i
   gösterin; “genele sıfırla” davranışını kanal için de uygulayın.
6. JSON save sanitization: ID digit/length, en fazla kanal sayısı, string limitleri ve duplicate kayıtları
   sınırlandırın. Atomic write mevcut davranışını koruyun.
7. Thread/forum inheritance ve DM davranışını test edin. DM'de guild/channel override aramayın.

## Önerilen dosya sınırı

`src/settings.rs`, `src/converters/mod.rs`, `src/handler.rs`, `src/web/mod.rs`, `src/web/ui/app.js`,
`src/web/ui/index.html`, `tests/fixtures/settings-resolution.json` ve yeni channel fixture'ları.

## Kabul kriterleri

- Aynı guild'de A kanalı dönüşümü açık, B kanalı kapalıyken link yalnızca A'da işlenir.
- Guild ayarı değişince override edilmemiş kanallar yeni değeri miras alır.
- Kanalda sabitlenen blok, guild'de aynı değer olsa bile “override” olarak kalır.
- Forum thread'i için seçilen inheritance semantiği belgelenmiş ve testlidir.
- WebUI botun göremediği kanal/guild'leri seçilebilir göstermemelidir.
- Eski `schema_version` dosyaları kayıpsız okunur; round-trip testleri geçer.

## Güvenlik ve rollback

Kanal override'ı bir permission grant değildir; role policy hâlâ uygulanmalıdır. Yetkili panel kullanıcısı
olmayan kişi API'den kanal ID göndererek ayar yazamamalıdır. Migration başarısızsa eski dosyayı varsayılan
erişim politikasıyla çalıştırmak yerine mevcut fail-safe davranış korunmalıdır.
