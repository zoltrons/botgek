//! Manual, offline baseline. Run the same command on the deployment host for comparisons.
use crate::{
    converters::{
        builtin::{
            Author, BuiltinPost,
            render::{RenderOptions, ResolvedMedia},
            v2::{V2Card, build_payload},
        },
        parser::{Platform, parse_message},
    },
    settings::{EmojiSettings, EngineSettings},
};
use std::{hint::black_box, time::Instant};

#[test]
#[ignore = "manual CPU baseline; cargo test --release offline_card_baseline -- --ignored --nocapture"]
fn offline_card_baseline() {
    let mut post = BuiltinPost::new(
        Platform::Twitter,
        "https://x.com/example/status/1",
        Author::new("Example", "example"),
    );
    post.text = "Representative Unicode body: merhaba dünya. ".repeat(20);
    let options = RenderOptions::from_engine(&EngineSettings::default(), "twitter");
    let emojis = EmojiSettings::default();
    let resolved = ResolvedMedia::default();
    let card = V2Card {
        post: &post,
        options: &options,
        resolved: &resolved,
        emojis: &emojis,
        spoiler: false,
        sender: None,
        user_comment: None,
        impersonation: None,
        allowed_mentions: None,
    };
    for _ in 0..100 {
        black_box(build_payload(&card));
    }
    let mut timings = Vec::with_capacity(10_000);
    for _ in 0..10_000 {
        let started = Instant::now();
        black_box(parse_message("https://x.com/example/status/1"));
        black_box(build_payload(&card));
        timings.push(started.elapsed().as_nanos());
    }
    timings.sort_unstable();
    println!(
        "offline parse+render; samples=10000; p50={}ns; p95={}ns; no network/RSS measured; {}",
        timings[5000],
        timings[9500],
        crate::version::summary()
    );
}
