# Tumblr

## Görev

Tumblr gönderilerini kartlara çevirir. NPF yapısı, reblog zinciri ve çoklu medya tek render görselinde
korunur.

## Dosyalar

`builtin/tumblr.rs`.

## Girdi ve çıktı

`tumblr::fetch(client, &ParsedPost) -> Result<BuiltinPost, String>`.

## Nasıl çalışır

1. Gönderi adresinden blog ve gönderi kimliği çıkarılır (`tumblr_post_parts`).
2. fxtumblr uyumlu bir renderer'dan sayfa alınır. Alan adı `FXTUMBLR_DOMAIN`, varsayılan `tpmblr.com`
   (`fxtumblr_domain`). İstek `safe_http::preview` ile gider ve renderer başka bir ana makineye yönlendirirse
   reddedilir. Gelen HTML `preview::from_external_html` ile kartlaştırılır.
3. Renderer erişilemezse ya da sonuç vermezse herkese açık OG önizlemesine düşülür (`preview::fetch`).

Platforma özgü API ya da kimlik doğrulama yoktur.

## Değişmezler

- Renderer alan adı yapılandırılabilir ama `safe_http` kurallarından geçer. Özel adresli bir alan adı
  reddedilir.
- Medya indirilmez, render görseli bağlantıyla gösterilir.

## Değiştirirken

Başka bir renderer alan adı için yalnızca `FXTUMBLR_DOMAIN` değişir. Yeni yol eklerken `tumblr_post_parts` testlerini
genişletin.

## Testler

`cargo test --locked tumblr`.
