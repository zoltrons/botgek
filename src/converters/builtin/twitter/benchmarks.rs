//! Read-only network comparison; no credentials or Discord messages are used.
use super::*;

#[tokio::test]
#[ignore = "manual network comparison; run twitter::benchmarks -- --ignored --nocapture"]
async fn live_fetch_scenarios() {
    let client = reqwest::Client::builder()
        .tls_backend_rustls()
        .timeout(Duration::from_secs(6))
        .tcp_nodelay(true)
        .build()
        .unwrap();
    let engine = EngineSettings {
        twitter_source: TwitterSource::Vxtwitter,
        auto_translate: true,
        show_quote: true,
        ..EngineSettings::default()
    };
    let scenarios = [
        ("video", "https://x.com/calif_io/status/2111080240978223545"),
        (
            "image+quote",
            "https://x.com/testingcatalog/status/2111380259295653976",
        ),
        (
            "quoted-ja-video",
            "https://x.com/Tyrielwood/status/2111361252207583299",
        ),
        (
            "triple-image",
            "https://x.com/IncroyableSauce/status/2111307452495073706",
        ),
    ];
    println!("Twitter fetch comparison; {}", crate::version::summary());
    for round in 0..2 {
        for (scenario, url) in scenarios {
            let parsed = ParsedPost {
                platform: Platform::Twitter,
                original_url: url.to_string(),
                fixed_url: url.to_string(),
                is_spoiler: false,
                user_comment: None,
            };
            let started = Instant::now();
            match tokio::time::timeout(Duration::from_secs(8), fetch(&client, &parsed, &engine))
                .await
            {
                Ok(Ok(post)) => {
                    let quote = post.quote.as_deref();
                    let nested = quote.and_then(|quote| quote.quote.as_deref());
                    println!(
                        "round={round} scenario={scenario} ms={} media={} quote={} nested={} translated={} quote_translated={}",
                        started.elapsed().as_millis(),
                        post.media.len(),
                        quote.is_some(),
                        nested.is_some(),
                        post.translation.is_some(),
                        quote.is_some_and(|quote| quote.translation.is_some()),
                    );
                }
                Ok(Err(reason)) => println!("round={round} scenario={scenario} failed={reason}"),
                Err(_) => println!("round={round} scenario={scenario} cutoff_ms=8000"),
            }
        }
    }
}
