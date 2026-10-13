# EG-07 — Premium, branding ve entitlement parity'si

**Öncelik:** P2 / ürün kararı gerekli  
**Hedef:** Embedded'in core özelliklerinden ayrı olarak public Premium sayfasında listelenen ücretli
özellikleri Botgek'e eklemek veya bilinçli biçimde kapsam dışı bırakmak.

> Bu rapor teknik bir “hemen uygula” talimatı değildir. Ödeme sağlayıcısı, Discord App Subscription,
> fiyatlandırma, vergi, iade, guild ownership ve veri saklama politikası seçilmeden agent kod yazmamalıdır.

## Embedded public premium iddiaları

Ana sayfadaki Premium bölümü sunucu başına aylık $1.99 karşılığında şunları listeliyor:

- Custom Bot Avatar
- No Watermark on Impersonation
- Colors on Containers
- Respond to Bots
- Higher Translation Limit

Core özelliklerin ücretsiz, premium'un optional olduğu ayrıca belirtiliyor.

## Botgek mevcut durumu

- Impersonation zaten mevcut; kullanıcı adı/avatar webhook kimliğiyle değişiyor. Ancak premium entitlement
  ve watermark ayrımı yok.
- Container accent color `CustomCard` içinde teknik olarak mevcut; kullanıcı/guild ayarı ve genel kart
  renk seçimi yok.
- Bot avatarı için kalıcı guild avatar config/upload akışı yok.
- Bot mesajlarına yanıt default guard ile kapalı; EG-06 bunu güvenli bir feature flag'e ayırır.
- Çeviri için guild/premium quota modeli yok.

## Hedef mimari

```text
EntitlementProvider
  -> Entitlements { guild_id, plan, expires_at, features, limits, checked_at }
  -> FeatureGate::allows(guild_id, feature)
  -> resolved policy / renderer / quota
```

Entitlement kalıcı ve imzalı/sağlayıcıdan doğrulanabilir olmalı; client WebUI'ın gönderdiği `premium: true`
alanına güvenilmemeli. Provider ulaşılmazsa son doğrulanmış kısa TTL cache veya güvenli free plan seçilmeli.

## Uygulama planı

1. Önce ürün kararlarını yazılı hale getirin: provider, plan ID, fiyat/currency, guild owner doğrulaması,
   grace period, iptal/expiry, test/sandbox, veri bölgesi ve webhook imza doğrulaması.
2. `EntitlementProvider` trait ve no-op free provider ekleyin. Üretim provider'ı secret'ı yalnızca env/
   secret store'dan almalı.
3. Guild/channel settings resolution sonrası feature gate uygulayın. Free kullanıcıya gizli premium
   davranış açmayın; WebUI kaydetme sırasında açık validation ve kullanıcı dostu hata döndürün.
4. **Custom avatar:** Discord webhook avatar URL veya bot guild avatar API'sini seçin. Upload boyutu,
   MIME, image dimensions, SSRF ve cache invalidation kuralları yazılmadan URL kabul etmeyin.
5. **Watermark:** Watermark'ın tam olarak hangi visible marker olduğu belirlenmeli. Mevcut Botgek kart
   düzenindeki güvenilir identity/attribution bilgisi güvenlik için silinmemeli; yalnızca ürünün görünür
   watermark metni premium ile kaldırılmalı.
6. **Container colors:** `engine.container_accent_color` için allowlisted RGB/hex ve accessibility preview
   ekleyin. Platform özel `CustomCard.accent_color` ile kullanıcı rengi arasındaki precedence yazılmalı.
7. **Respond to bots:** EG-06'nın güvenli feature gate'ine bağlayın; yalnızca entitlement kontrolü ile
   handler guard'ını kaldırmayın.
8. **Higher translation limit:** Normal/premium karakter, request/day ve concurrent limitleri merkezi
   quota store'da tutun. Restart ve çoklu instance davranışı açıkça seçilmeli.
9. Audit: entitlement değişikliği, premium feature denial ve quota tüketimi kişisel içerik/token içermeden
   ölçülsün. `/api/metrics` secret veya kullanıcı metni döndürmemeli.

## Kabul kriterleri

- Entitlement yok/expired/provider hatası free davranışına güvenle döner.
- Kullanıcı WebUI JSON'una premium alanı yazarak özellik açamaz.
- Her premium özellik bağımsız gate ve test içerir.
- Subscription webhook imzası doğrulanmadan entitlement değişmez.
- Avatar upload/URL dış kaynaklara SSRF açmaz.
- Container color kart metni kontrastını ve mevcut platform accent önceliğini bozmaz.
- Translation quota kullanıcı/guild/global sınırlarını ve 429 davranışını korur.
- İptal sonrası grace/expiry davranışı ve rollback belgelenmiştir.

## Önerilen kapsam kararı

Ürün henüz ödeme almayacaksa bu raporun teknik çıktısı yalnızca şu olmalıdır: `FeatureGate`/no-op
abstraction, free defaults, EG-06 opt-in flag ve renk ayarı için geleceğe uyumlu schema. Gerçek ödeme ve
avatar upload kodu kullanıcı onayı olmadan eklenmemelidir.
