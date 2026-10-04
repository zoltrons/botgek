use std::sync::LazyLock;
use regex::Regex;
use serenity::all::ReactionType;

static TRANSLATE_URL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[:\]\((https?://[^/\s]+/(?:[^/\s]+/)?(?:[^/\s]+)/(?:status|post)/[a-zA-Z0-9]+)(?:/([a-z]{2}))?\)")
        .expect("Geçersiz çeviri regex deseni")
});

/// Emoji bayrağını 2 harfli ISO 639-1 dil koduna dönüştürür.
/// Örn: 🇹🇷 -> "tr", 🇬🇧/🇺🇸 -> "en", 🇩🇪 -> "de", vb.
pub fn emoji_to_lang_code(emoji: &ReactionType) -> Option<&'static str> {
    match emoji {
        ReactionType::Unicode(s) => match s.as_str() {
            "🇹🇷" => Some("tr"),
            "🇬🇧" | "🇺🇸" => Some("en"),
            "🇩🇪" => Some("de"),
            "🇫🇷" => Some("fr"),
            "🇪🇸" => Some("es"),
            "🇯🇵" => Some("ja"),
            "🇷🇺" => Some("ru"),
            "🇮🇹" => Some("it"),
            "🇦🇿" => Some("az"),
            "🇸🇦" => Some("ar"),
            "🇰🇷" => Some("ko"),
            "🇵🇹" | "🇧🇷" => Some("pt"),
            "🇺🇦" => Some("uk"),
            "🇨🇳" => Some("zh"),
            "🇳🇱" => Some("nl"),
            "🇵🇱" => Some("pl"),
            "🇸🇪" => Some("sv"),
            "🇬🇷" => Some("el"),
            "🇮🇳" => Some("hi"),
            _ => None,
        },
        ReactionType::Custom { name: Some(name), .. } => match name.as_str() {
            "flag_tr" | "tr" => Some("tr"),
            "flag_us" | "flag_gb" | "en" => Some("en"),
            "flag_de" => Some("de"),
            "flag_fr" => Some("fr"),
            "flag_es" => Some("es"),
            "flag_jp" => Some("ja"),
            "flag_ru" => Some("ru"),
            "flag_az" => Some("az"),
            "flag_it" => Some("it"),
            "flag_sa" | "flag_ar" => Some("ar"),
            "flag_kr" => Some("ko"),
            "flag_pt" | "flag_br" => Some("pt"),
            "flag_ua" | "flag_uk" => Some("uk"),
            "flag_cn" => Some("zh"),
            "flag_nl" => Some("nl"),
            "flag_pl" => Some("pl"),
            "flag_se" => Some("sv"),
            "flag_gr" => Some("el"),
            "flag_in" => Some("hi"),
            _ => None,
        },
        _ => None,
    }
}

/// Mesaj içeriğindeki FxTwitter / FixupX / FxBluesky bağlantısına çeviri dil kodunu ekler veya kaldırır.
/// target_lang: Some("tr") ise /tr ekler, None ise kaldırıp orijinal linke döner.
pub fn update_link_translation(content: &str, target_lang: Option<&str>) -> Option<String> {
    if let Some(caps) = TRANSLATE_URL_REGEX.captures(content) {
        let full_match = caps.get(0)?.as_str();
        let base_url = caps.get(1)?.as_str();

        let new_chunk = match target_lang {
            Some(lang) => format!("[:]({}/{})", base_url, lang),
            None => format!("[:]({})", base_url),
        };

        if full_match != new_chunk {
            return Some(content.replacen(full_match, &new_chunk, 1));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_turkish_translation() {
        let content = "<@123>[:](https://fixupx.com/user/status/456) [**Twitter**](<https://x.com/user/status/456>)";
        let updated = update_link_translation(content, Some("tr")).unwrap();
        assert_eq!(
            updated,
            "<@123>[:](https://fixupx.com/user/status/456/tr) [**Twitter**](<https://x.com/user/status/456>)"
        );
    }

    #[test]
    fn test_switch_translation() {
        let content = "<@123>[:](https://fixupx.com/user/status/456/en) [**Twitter**](<https://x.com/user/status/456>)";
        let updated = update_link_translation(content, Some("tr")).unwrap();
        assert_eq!(
            updated,
            "<@123>[:](https://fixupx.com/user/status/456/tr) [**Twitter**](<https://x.com/user/status/456>)"
        );
    }

    #[test]
    fn test_remove_translation() {
        let content = "<@123>[:](https://fixupx.com/user/status/456/tr) [**Twitter**](<https://x.com/user/status/456>)";
        let updated = update_link_translation(content, None).unwrap();
        assert_eq!(
            updated,
            "<@123>[:](https://fixupx.com/user/status/456) [**Twitter**](<https://x.com/user/status/456>)"
        );
    }

    #[test]
    fn test_spoiler_translation() {
        let content = "<@123>||[:](https://fixupx.com/user/status/456)|| [**Twitter**](<https://x.com/user/status/456>)";
        let updated = update_link_translation(content, Some("tr")).unwrap();
        assert_eq!(
            updated,
            "<@123>||[:](https://fixupx.com/user/status/456/tr)|| [**Twitter**](<https://x.com/user/status/456>)"
        );
    }

    #[test]
    fn test_bluesky_translation() {
        let content = "<@123>[:](https://fxbsky.app/profile/alice.bsky.social/post/3k6abc) [**Bluesky**](<https://bsky.app/profile/alice.bsky.social/post/3k6abc>)";
        let updated = update_link_translation(content, Some("tr")).unwrap();
        assert_eq!(
            updated,
            "<@123>[:](https://fxbsky.app/profile/alice.bsky.social/post/3k6abc/tr) [**Bluesky**](<https://bsky.app/profile/alice.bsky.social/post/3k6abc>)"
        );
    }
}

