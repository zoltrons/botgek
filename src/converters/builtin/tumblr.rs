//! Tumblr previews, with an fxtumblr renderer fallback for NPF posts and reblog/media fidelity.
use super::{BuiltinPost, preview, safe_http};
use crate::converters::parser::{ParsedPost, Platform};
use url::Url;

const DEFAULT_FXTUMBLR_DOMAIN: &str = "tpmblr.com";

fn fxtumblr_domain() -> Option<String> {
    let raw =
        std::env::var("FXTUMBLR_DOMAIN").unwrap_or_else(|_| DEFAULT_FXTUMBLR_DOMAIN.to_string());
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let candidate = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let url = Url::parse(&candidate).ok()?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    (host.contains('.')
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-')))
    .then_some(host)
}

fn tumblr_post_parts(source: &str) -> Option<(String, String)> {
    let url = Url::parse(source).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let parts: Vec<_> = url
        .path()
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let (blog, id) = if host.ends_with(".tumblr.com") && host != "www.tumblr.com" {
        let blog = host.strip_suffix(".tumblr.com")?;
        let id = if parts.first() == Some(&"post") {
            parts.get(1)?
        } else {
            return None;
        };
        (blog, *id)
    } else if matches!(host.as_str(), "tumblr.com" | "www.tumblr.com") {
        let blog = parts.first()?;
        let id = if parts.get(1) == Some(&"post") {
            parts.get(2)?
        } else {
            parts.get(1)?
        };
        (*blog, *id)
    } else {
        return None;
    };
    if blog.is_empty()
        || !blog
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        || id.is_empty()
        || !id.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    Some((blog.to_string(), id.to_string()))
}

async fn fetch_fxtumblr(parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    let domain = fxtumblr_domain().ok_or("invalid FXTUMBLR_DOMAIN")?;
    let (blog, id) = tumblr_post_parts(&parsed.original_url).ok_or("unsupported Tumblr URL")?;
    let endpoint = format!("https://{domain}/{blog}/{id}");
    let (final_url, html) = safe_http::preview(&endpoint).await?;
    let final_host = Url::parse(&final_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .ok_or("fxtumblr final host missing")?;
    if final_host != domain {
        return Err("fxtumblr redirected outside its configured host".into());
    }
    preview::from_external_html(&parsed.original_url, Platform::Tumblr, &html)
}

pub async fn fetch(_client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    // fxtumblr renders Tumblr's NPF/reblog/media structure to one safe preview image. Keep the
    // original public Tumblr URL in the card and fall back to native OG metadata if the renderer
    // is unavailable or a post is private/missing.
    match fetch_fxtumblr(parsed).await {
        Ok(post) => Ok(post),
        Err(renderer_error) => {
            tracing::debug!(
                "fxtumblr preview unavailable ({}); falling back to Tumblr OG metadata",
                renderer_error
            );
            preview::fetch(parsed, Platform::Tumblr).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_fxtumblr_route_from_supported_tumblr_urls() {
        assert_eq!(
            tumblr_post_parts("https://www.tumblr.com/ballots/829790623215321088"),
            Some(("ballots".into(), "829790623215321088".into()))
        );
        assert_eq!(
            tumblr_post_parts("https://artist.tumblr.com/post/123456789/slug"),
            Some(("artist".into(), "123456789".into()))
        );
        assert!(tumblr_post_parts("https://artist.tumblr.com/about").is_none());
        assert!(tumblr_post_parts("https://example.com/artist/123").is_none());
    }

    #[test]
    fn fxtumblr_domain_rejects_urls_and_credentials() {
        assert!(fxtumblr_domain().is_some());
    }
}
