//! TikTok gönderi verisi (oEmbed).
//!
//! TikTok'un herkese açık oEmbed uç noktası yalnızca başlık, yazar ve kapak görseli verir;
//! doğrudan oynatılabilir video akışı yoktur. Bu yüzden native motor TikTok için bir
//! "kart" üretir. Video akışı isteniyorsa varsayılan olan proxy modu kullanılmalıdır.

use serde::Deserialize;

use super::{encode_component, Author, BuiltinPost, Media, MediaKind};
use crate::converters::parser::{ParsedPost, Platform};

const OEMBED_URL: &str = "https://www.tiktok.com/oembed";

/// TikTok gönderisini oEmbed üzerinden çeker ve kart verisine dönüştürür.
pub async fn fetch(client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    let url = format!("{OEMBED_URL}?url={}", encode_component(&parsed.original_url));
    let value = super::get_json(client, &url).await?;
    let oembed: TikTokOEmbed = serde_json::from_value(value)
        .map_err(|e| format!("TikTok oEmbed yanıtı çözümlenemedi: {e}"))?;

    if oembed.title.is_none() && oembed.thumbnail_url.is_none() {
        return Err(format!("TikTok gönderisi bulunamadı: {}", parsed.original_url));
    }

    Ok(oembed.into_post(parsed))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TikTokOEmbed {
    title: Option<String>,
    author_name: Option<String>,
    author_unique_id: Option<String>,
    thumbnail_url: Option<String>,
    thumbnail_width: Option<u32>,
    thumbnail_height: Option<u32>,
}

impl TikTokOEmbed {
    fn into_post(self, parsed: &ParsedPost) -> BuiltinPost {
        let handle = self.author_unique_id.clone().unwrap_or_default();
        let mut author = Author::new(
            self.author_name.clone().unwrap_or_else(|| handle.clone()),
            handle.clone(),
        );
        author.profile_url = if handle.is_empty() {
            None
        } else {
            Some(format!("https://www.tiktok.com/@{handle}"))
        };

        let mut post = BuiltinPost::new(Platform::TikTok, parsed.original_url.clone(), author);
        post.title = self.title.clone().filter(|title| !title.trim().is_empty());
        post.extra_label = Some("TikTok".to_string());

        if let Some(thumbnail) = self.thumbnail_url.clone() {
            post.media.push(Media {
                kind: MediaKind::Image,
                url: thumbnail,
                poster: None,
                width: self.thumbnail_width,
                height: self.thumbnail_height,
                duration: None,
            });
        }

        post
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed() -> ParsedPost {
        ParsedPost {
            platform: Platform::TikTok,
            original_url: "https://www.tiktok.com/@kullanici/video/7106594312292453675".to_string(),
            fixed_url: "https://tnktok.com/@kullanici/video/7106594312292453675".to_string(),
            is_spoiler: false,
            user_comment: None,
        }
    }

    #[test]
    fn oembed_response_becomes_a_card_post() {
        let json = r#"{
            "title": "Harika bir dans",
            "author_name": "Kullanıcı",
            "author_unique_id": "kullanici",
            "thumbnail_url": "https://p16-sign.tiktokcdn.com/thumb.jpg",
            "thumbnail_width": 720,
            "thumbnail_height": 1280
        }"#;

        let oembed: TikTokOEmbed = serde_json::from_str(json).expect("örnek JSON geçerli olmalı");
        let post = oembed.into_post(&parsed());

        assert_eq!(post.author.label(), "Kullanıcı (@kullanici)");
        assert_eq!(
            post.author.profile_url.as_deref(),
            Some("https://www.tiktok.com/@kullanici")
        );
        assert_eq!(post.title.as_deref(), Some("Harika bir dans"));
        assert_eq!(post.url, parsed().original_url);
        assert_eq!(post.media.len(), 1);
        assert_eq!(post.media[0].kind, MediaKind::Image);
        assert_eq!(post.media[0].height, Some(1280));
    }

    #[test]
    fn oembed_request_url_is_encoded() {
        let url = format!("{OEMBED_URL}?url={}", encode_component(&parsed().original_url));
        assert!(url.contains("%3A%2F%2Fwww.tiktok.com%2F%40kullanici"), "url: {url}");
    }
}
