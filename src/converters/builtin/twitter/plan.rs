//! Pure, content-aware enrichment decisions for the posts visible in the X card.

use super::super::{BuiltinPost, translate::needs_translation};
use crate::settings::EngineSettings;

/// What the source knows about the first quote's missing nested quote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NestedHint {
    Unknown,
    Absent,
    Present(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum QuoteFetch {
    /// Fetch the known third post directly.
    Direct(String),
    /// Fetch the first quote to discover whether it quotes another post.
    Inspect(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FetchPlan {
    pub translation_urls: Vec<String>,
    pub quote: Option<QuoteFetch>,
    pub preview_url: Option<String>,
    /// At least one visible post already has a nonblank media source URL.
    /// A text-only card is valid when this is false.
    pub media_ready: bool,
}

/// Plan only enrichment that can affect the visible card, without fetching or mutating data.
pub(super) fn decide(
    post: &BuiltinPost,
    engine: &EngineSettings,
    nested_hint: &NestedHint,
) -> FetchPlan {
    let first_quote = if engine.show_quote {
        post.quote.as_deref()
    } else {
        None
    };
    // V2 still shows the third post's text when the first quote contains media. Only
    // media on the root limits the visible chain to two posts.
    let third = if post.media.is_empty() {
        first_quote.and_then(|quote| quote.quote.as_deref())
    } else {
        None
    };

    let mut translation_urls = Vec::new();
    let mut media_ready = false;
    for visible in [Some(post), first_quote, third].into_iter().flatten() {
        media_ready |= visible
            .media
            .iter()
            .any(|media| !media.url.trim().is_empty());
        if engine.auto_translate
            && !visible.text.trim().is_empty()
            && visible.translation.is_none()
            && needs_translation(
                visible.language.as_deref(),
                &engine.translation_target,
                engine.translate_english,
            )
            && !translation_urls.contains(&visible.url)
        {
            translation_urls.push(visible.url.clone());
        }
    }

    let quote = if post.media.is_empty() {
        first_quote
            .filter(|quote| quote.quote.is_none())
            .and_then(|quote| match nested_hint {
                NestedHint::Unknown => Some(QuoteFetch::Inspect(quote.url.clone())),
                NestedHint::Absent => None,
                NestedHint::Present(url) => Some(QuoteFetch::Direct(url.clone())),
            })
    } else {
        None
    };

    let preview_url = if post.media.is_empty()
        && post.quote.is_none()
        && post
            .link_preview
            .as_ref()
            .is_none_or(|preview| preview.is_empty())
    {
        super::external_link(&post.text)
    } else {
        None
    };

    FetchPlan {
        translation_urls,
        quote,
        preview_url,
        media_ready,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::converters::builtin::{Author, LinkPreview, Media, MediaKind, Translation};
    use crate::converters::parser::Platform;

    fn post(id: u64, text: &str, language: Option<&str>) -> BuiltinPost {
        let mut post = BuiltinPost::new(
            Platform::Twitter,
            format!("https://x.com/test/status/{id}"),
            Author::new("Test", "test"),
        );
        post.text = text.to_string();
        post.language = language.map(str::to_string);
        post
    }

    fn media(kind: MediaKind, url: &str) -> Media {
        Media {
            kind,
            url: url.to_string(),
            poster: None,
            width: None,
            height: None,
            duration: None,
        }
    }

    #[test]
    fn english_and_turkish_text_need_no_enrichment_for_the_default_target() {
        let mut root = post(1, "Hello world", Some("en-US"));
        root.quote = Some(Box::new(post(2, "Merhaba dünya", Some("TR-tr"))));

        assert_eq!(
            decide(&root, &EngineSettings::default(), &NestedHint::Absent),
            FetchPlan {
                translation_urls: Vec::new(),
                quote: None,
                preview_url: None,
                media_ready: false,
            }
        );
    }

    #[test]
    fn custom_target_uses_the_shared_language_policy() {
        let engine = EngineSettings {
            translation_target: "de-DE".to_string(),
            ..EngineSettings::default()
        };
        let mut root = post(1, "Hello world", Some("en_US"));
        let mut first = post(2, "Merhaba dünya", Some("tr"));
        first.quote = Some(Box::new(post(3, "Guten Morgen", Some("de-AT"))));
        root.quote = Some(Box::new(first));

        assert_eq!(
            decide(&root, &engine, &NestedHint::Unknown).translation_urls,
            vec![root.url.clone(), root.quote.as_ref().unwrap().url.clone()]
        );
    }

    #[test]
    fn unknown_source_languages_never_schedule_translation() {
        let engine = EngineSettings {
            translation_target: "fr".to_string(),
            ..EngineSettings::default()
        };
        for language in [None, Some(""), Some("und"), Some("qme"), Some("12")] {
            let root = post(1, "Unclassified text", language);
            assert!(
                decide(&root, &engine, &NestedHint::Unknown)
                    .translation_urls
                    .is_empty(),
                "unknown language {language:?} must not trigger a request"
            );
        }
    }

    #[test]
    fn japanese_quote_is_translated_even_with_a_media_root() {
        let engine = EngineSettings::default();
        let mut root = post(1, "Hello world", Some("en"));
        let first = post(2, "おはようございます", Some("ja"));
        let quote_url = first.url.clone();
        root.quote = Some(Box::new(first));

        assert_eq!(
            decide(&root, &engine, &NestedHint::Absent).translation_urls,
            vec![quote_url.clone()]
        );

        root.media.push(media(
            MediaKind::Image,
            "https://pbs.twimg.com/media/root.jpg",
        ));
        let plan = decide(&root, &engine, &NestedHint::Unknown);
        assert_eq!(plan.translation_urls, vec![quote_url]);
        assert_eq!(plan.quote, None);
        assert!(plan.media_ready);
    }

    #[test]
    fn english_translation_can_be_forced_for_interactions() {
        let root = post(1, "Hello world", Some("en"));
        let default = EngineSettings::default();
        assert!(
            decide(&root, &default, &NestedHint::Absent)
                .translation_urls
                .is_empty(),
            "normal akışta İngilizce varsayılan olarak çevrilmez"
        );

        let forced = EngineSettings {
            translate_english: true,
            ..EngineSettings::default()
        };
        assert_eq!(
            decide(&root, &forced, &NestedHint::Absent).translation_urls,
            vec![root.url.clone()],
            "etkileşim dönüşümü İngilizce için de çeviri istemeli"
        );
    }

    #[test]
    fn translation_requires_enabled_nonempty_untranslated_text() {
        let mut root = post(1, "おはようございます", Some("ja"));
        let engine = EngineSettings {
            auto_translate: false,
            ..EngineSettings::default()
        };
        assert!(
            decide(&root, &engine, &NestedHint::Absent)
                .translation_urls
                .is_empty()
        );

        let engine = EngineSettings::default();
        root.translation = Some(Translation {
            source: Some("ja".to_string()),
        });
        assert!(
            decide(&root, &engine, &NestedHint::Absent)
                .translation_urls
                .is_empty()
        );

        root.translation = None;
        for text in ["", " \n\t "] {
            root.text = text.to_string();
            assert!(
                decide(&root, &engine, &NestedHint::Absent)
                    .translation_urls
                    .is_empty()
            );
        }
    }

    #[test]
    fn translation_urls_are_deduplicated_in_visible_order() {
        let mut root = post(1, "一番", Some("ja"));
        let mut first = post(1, "同じ投稿", Some("ja"));
        let third = post(3, "三番", Some("ja"));
        let expected = vec![root.url.clone(), third.url.clone()];
        first.quote = Some(Box::new(third));
        root.quote = Some(Box::new(first));

        assert_eq!(
            decide(&root, &EngineSettings::default(), &NestedHint::Unknown).translation_urls,
            expected
        );
    }

    #[test]
    fn video_and_gif_sources_are_ready_without_translation_or_preview_work() {
        for (kind, url) in [
            (
                MediaKind::Video,
                "https://video.twimg.com/ext_tw_video/1/vid/movie.mp4",
            ),
            (
                MediaKind::Gif,
                "https://video.twimg.com/tweet_video/animation.mp4",
            ),
        ] {
            let mut root = post(1, "Watch https://example.com/article", Some("en"));
            root.media.push(media(kind, url));
            let plan = decide(&root, &EngineSettings::default(), &NestedHint::Unknown);

            assert!(plan.media_ready, "{kind:?} should be usable immediately");
            assert!(plan.translation_urls.is_empty());
            assert_eq!(plan.quote, None);
            assert_eq!(plan.preview_url, None);
        }
    }

    #[test]
    fn blank_media_sources_and_posters_alone_are_not_ready() {
        let mut root = post(1, "Hello world", Some("en"));
        for kind in [MediaKind::Image, MediaKind::Video, MediaKind::Gif] {
            let mut item = media(kind, " \t ");
            item.poster = Some("https://pbs.twimg.com/media/poster.jpg".to_string());
            root.media.push(item);
        }

        assert!(!decide(&root, &EngineSettings::default(), &NestedHint::Absent).media_ready);
    }

    #[test]
    fn nested_hints_distinguish_absence_inspection_and_a_direct_third_post() {
        let third_url = "https://x.com/test/status/3".to_string();
        for kind in [
            None,
            Some(MediaKind::Image),
            Some(MediaKind::Video),
            Some(MediaKind::Gif),
        ] {
            let mut root = post(1, "Hello world", Some("en"));
            let mut first = post(2, "Quoted text", Some("en"));
            if let Some(kind) = kind {
                first
                    .media
                    .push(media(kind, "https://cdn.example.com/media"));
            }
            let first_url = first.url.clone();
            root.quote = Some(Box::new(first));
            let engine = EngineSettings::default();

            assert_eq!(
                decide(&root, &engine, &NestedHint::Absent).quote,
                None,
                "a known absence must not be inspected ({kind:?})"
            );
            assert_eq!(
                decide(&root, &engine, &NestedHint::Unknown).quote,
                Some(QuoteFetch::Inspect(first_url)),
                "an unknown nested quote must be inspected ({kind:?})"
            );
            assert_eq!(
                decide(&root, &engine, &NestedHint::Present(third_url.clone())).quote,
                Some(QuoteFetch::Direct(third_url.clone())),
                "first-quote media must not hide the known third post ({kind:?})"
            );
        }
    }

    #[test]
    fn nested_fetch_needs_a_first_quote_and_a_media_free_root() {
        let mut root = post(1, "Hello world", Some("en"));
        let engine = EngineSettings::default();
        let hints = [
            NestedHint::Unknown,
            NestedHint::Absent,
            NestedHint::Present("https://x.com/test/status/3".to_string()),
        ];
        for hint in &hints {
            assert_eq!(decide(&root, &engine, hint).quote, None);
        }

        root.quote = Some(Box::new(post(2, "Quoted text", Some("en"))));
        root.media.push(media(
            MediaKind::Image,
            "https://pbs.twimg.com/media/root.jpg",
        ));
        for hint in &hints {
            assert_eq!(decide(&root, &engine, hint).quote, None);
        }
    }

    #[test]
    fn hidden_quotes_do_not_translate_fetch_or_supply_media() {
        let engine = EngineSettings {
            show_quote: false,
            ..EngineSettings::default()
        };
        let mut root = post(1, "日本語 https://example.com/article", Some("ja"));
        let mut first = post(2, "引用", Some("ja"));
        first.media.push(media(
            MediaKind::Video,
            "https://video.twimg.com/ext_tw_video/1/vid/movie.mp4",
        ));
        first.quote = Some(Box::new(post(3, "元の投稿", Some("ja"))));
        root.quote = Some(Box::new(first));

        for hint in [
            NestedHint::Unknown,
            NestedHint::Absent,
            NestedHint::Present("https://x.com/test/status/3".to_string()),
        ] {
            assert_eq!(
                decide(&root, &engine, &hint),
                FetchPlan {
                    translation_urls: vec![root.url.clone()],
                    quote: None,
                    preview_url: None,
                    media_ready: false,
                }
            );
        }

        // The same restriction applies when the hidden third post is still missing.
        root.quote.as_deref_mut().unwrap().quote = None;
        assert_eq!(decide(&root, &engine, &NestedHint::Unknown).quote, None);
    }

    #[test]
    fn an_existing_triple_is_complete_and_only_three_posts_are_classified() {
        let mut root = post(1, "一番", Some("ja"));
        let mut first = post(2, "二番", Some("ja"));
        let mut third = post(3, "三番", Some("ja"));
        let expected = vec![root.url.clone(), first.url.clone(), third.url.clone()];
        let mut fourth = post(4, "네 번째", Some("ko"));
        fourth.media.push(media(
            MediaKind::Image,
            "https://pbs.twimg.com/media/hidden-fourth.jpg",
        ));
        third.quote = Some(Box::new(fourth));
        first.quote = Some(Box::new(third));
        root.quote = Some(Box::new(first));

        for hint in [
            NestedHint::Unknown,
            NestedHint::Absent,
            NestedHint::Present("https://x.com/test/status/99".to_string()),
        ] {
            let plan = decide(&root, &EngineSettings::default(), &hint);
            assert_eq!(plan.translation_urls, expected);
            assert_eq!(plan.quote, None, "existing data overrides the hint");
            assert!(!plan.media_ready, "the fourth post is outside the card");
        }
    }

    #[test]
    fn mixed_media_triple_keeps_the_third_text_visible() {
        let mut root = post(1, "Hello world", Some("en"));
        let mut first = post(2, "Quoted text", Some("en"));
        first.media.push(media(
            MediaKind::Video,
            "https://video.twimg.com/ext_tw_video/1/vid/movie.mp4",
        ));
        let third = post(3, "元の投稿", Some("ja"));
        let third_url = third.url.clone();
        first.quote = Some(Box::new(third));
        root.quote = Some(Box::new(first));

        let plan = decide(&root, &EngineSettings::default(), &NestedHint::Unknown);
        assert_eq!(plan.translation_urls, vec![third_url]);
        assert_eq!(plan.quote, None);
        assert!(plan.media_ready);

        // Moving the media to the root hides only the third post; the first stays visible.
        let first = root.quote.as_deref_mut().unwrap();
        first.language = Some("ja".to_string());
        root.media = std::mem::take(&mut first.media);
        let plan = decide(&root, &EngineSettings::default(), &NestedHint::Unknown);
        assert_eq!(
            plan.translation_urls,
            vec![root.quote.as_ref().unwrap().url.clone()]
        );
        assert_eq!(plan.quote, None);
        assert!(plan.media_ready);
    }

    #[test]
    fn the_third_post_can_be_the_only_ready_media_source() {
        let mut root = post(1, "Hello world", Some("en"));
        let mut first = post(2, "Quoted text", Some("en"));
        let mut third = post(3, "Original text", Some("en"));
        third.media.push(media(
            MediaKind::Gif,
            "https://video.twimg.com/tweet_video/animation.mp4",
        ));
        first.quote = Some(Box::new(third));
        root.quote = Some(Box::new(first));

        let plan = decide(&root, &EngineSettings::default(), &NestedHint::Unknown);
        assert!(plan.media_ready);
        assert!(plan.translation_urls.is_empty());
        assert_eq!(plan.quote, None);
    }

    #[test]
    fn preview_uses_the_parent_external_link_policy_and_preserves_existing_data() {
        let mut root = post(
            1,
            "https://t.co/short https://x.com/test/status/2 https://example.com/article.",
            Some("en"),
        );
        let engine = EngineSettings::default();
        assert_eq!(
            decide(&root, &engine, &NestedHint::Unknown)
                .preview_url
                .as_deref(),
            Some("https://example.com/article")
        );

        root.link_preview = Some(LinkPreview::default());
        assert_eq!(
            decide(&root, &engine, &NestedHint::Unknown)
                .preview_url
                .as_deref(),
            Some("https://example.com/article")
        );

        for preview in [
            LinkPreview {
                title: Some("Existing title".to_string()),
                ..LinkPreview::default()
            },
            LinkPreview {
                description: Some("Existing description".to_string()),
                ..LinkPreview::default()
            },
            LinkPreview {
                image: Some("https://example.com/image.jpg".to_string()),
                ..LinkPreview::default()
            },
        ] {
            root.link_preview = Some(preview);
            assert_eq!(
                decide(&root, &engine, &NestedHint::Unknown).preview_url,
                None
            );
        }

        root.link_preview = None;
        root.quote = Some(Box::new(post(2, "Quoted text", Some("en"))));
        assert_eq!(
            decide(&root, &engine, &NestedHint::Absent).preview_url,
            None
        );

        root.quote = None;
        root.text = "Only https://x.com/test/status/2 and https://t.co/short".to_string();
        assert_eq!(
            decide(&root, &engine, &NestedHint::Unknown).preview_url,
            None
        );
    }
}
