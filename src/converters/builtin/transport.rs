//! Discord transport: bounded responses, route/global cooldowns, no blind POST retries.
use std::{
    collections::HashMap,
    sync::LazyLock,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

#[derive(Debug)]
pub enum Error {
    Rejected(u16),
    RateLimited(Duration),
    Ambiguous,
    InvalidPayload,
    Unavailable,
}

impl Error {
    pub fn safe_fallback(&self) -> bool {
        matches!(
            self,
            Self::Rejected(_) | Self::InvalidPayload | Self::Unavailable
        )
    }
    pub fn expired_webhook(&self) -> bool {
        matches!(self, Self::Rejected(401 | 404))
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(status) => write!(f, "Discord {status} rejected the request"),
            Self::RateLimited(wait) => write!(f, "Discord rate limited; retry after {wait:?}"),
            Self::Ambiguous => f.write_str("Discord outcome unknown; automatic resend suppressed"),
            Self::InvalidPayload => f.write_str("Invalid Discord payload"),
            Self::Unavailable => f.write_str("Webhook unavailable"),
        }
    }
}
impl std::error::Error for Error {}

pub fn retry_after(headers: &reqwest::header::HeaderMap, body: &str) -> Duration {
    fn duration(value: f64) -> Option<Duration> {
        if !value.is_finite() || value < 0.0 {
            return None;
        }
        Duration::try_from_secs_f64(value.max(0.05)).ok()
    }
    headers
        .get("retry-after")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse().ok())
        .and_then(duration)
        .or_else(|| {
            serde_json::from_str::<serde_json::Value>(body)
                .ok()?
                .get("retry_after")?
                .as_f64()
                .and_then(duration)
        })
        .unwrap_or(Duration::from_millis(500))
}

static COOLDOWNS: LazyLock<Mutex<HashMap<String, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub async fn send(
    client: &reqwest::Client,
    request: reqwest::RequestBuilder,
    route: String,
) -> Result<String, Error> {
    use crate::telemetry::{self, RouteClass};
    use tracing::warn;

    let class = RouteClass::classify(&route);
    telemetry::rest_request(class);
    let total_started = Instant::now();
    let request = match request.build() {
        Ok(request) => request,
        Err(_) => {
            telemetry::rest_invalid(class);
            telemetry::rest_total(class, total_started.elapsed());
            return Err(Error::InvalidPayload);
        }
    };
    let _route = super::coordination::lock(format!("rest:{route}")).await;
    let outcome = tokio::time::timeout(Duration::from_secs(90), async {
        for attempt in 0..3 {
            let wait = {
                let mut cooldowns = COOLDOWNS.lock().await;
                cooldowns.retain(|_, until| *until > Instant::now());
                cooldowns
                    .get(&route)
                    .into_iter()
                    .chain(cooldowns.get("global"))
                    .max()
                    .map(|until| until.saturating_duration_since(Instant::now()))
                    .unwrap_or_default()
            };
            if wait > Duration::from_secs(90) {
                return Err(Error::RateLimited(wait));
            }
            if !wait.is_zero() {
                telemetry::rest_rate_limit_wait(class, wait);
            }
            tokio::time::sleep(wait).await;
            let attempt_started = Instant::now();
            let mut response = match client
                .execute(request.try_clone().ok_or(Error::InvalidPayload)?)
                .await
            {
                Ok(response) => response,
                Err(_) => {
                    telemetry::rest_attempt(class, attempt_started.elapsed(), None);
                    return Err(Error::Ambiguous);
                }
            };
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let mut bytes = Vec::new();
            loop {
                match response.chunk().await {
                    Ok(Some(chunk)) => {
                        if bytes.len() + chunk.len() > super::MAX_BODY_BYTES {
                            telemetry::rest_attempt(class, attempt_started.elapsed(), None);
                            return Err(Error::Ambiguous);
                        }
                        bytes.extend_from_slice(&chunk);
                    }
                    Ok(None) => break,
                    Err(_) => {
                        telemetry::rest_attempt(class, attempt_started.elapsed(), None);
                        return Err(Error::Ambiguous);
                    }
                }
            }
            let body = match String::from_utf8(bytes) {
                Ok(body) => body,
                Err(_) => {
                    telemetry::rest_attempt(class, attempt_started.elapsed(), None);
                    return Err(Error::Ambiguous);
                }
            };
            telemetry::rest_attempt(class, attempt_started.elapsed(), Some(status));
            if status == 429 {
                let wait = retry_after(&headers, &body);
                let global = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .is_some_and(|v| v["global"] == true);
                let key = if global {
                    "global".into()
                } else {
                    route.clone()
                };
                if let Some(until) = Instant::now().checked_add(wait) {
                    let mut cooldowns = COOLDOWNS.lock().await;
                    cooldowns
                        .entry(key)
                        .and_modify(|old| *old = (*old).max(until))
                        .or_insert(until);
                }
                telemetry::rest_rate_limit_wait(class, wait);
                warn!(
                    route = class.name(),
                    attempt = attempt + 1,
                    wait_ms = wait.as_millis().min(u64::MAX as u128) as u64,
                    global,
                    "Discord 429 rate limit"
                );
                if attempt == 2 {
                    return Err(Error::RateLimited(wait));
                }
                telemetry::rest_retry(class);
                continue;
            }
            if (200..300).contains(&status) {
                return Ok(body);
            }
            if (400..500).contains(&status) && status != 408 {
                if matches!(status, 401 | 403) {
                    warn!(route = class.name(), status, "Discord rejected the request");
                }
                return Err(Error::Rejected(status));
            }
            warn!(route = class.name(), status, "Discord server error");
            return Err(Error::Ambiguous);
        }
        Err(Error::Ambiguous)
    })
    .await;
    let result = match outcome {
        Ok(result) => result,
        Err(_) => {
            telemetry::rest_timeout(class);
            Err(Error::Ambiguous)
        }
    };
    telemetry::rest_total(class, total_started.elapsed());
    match &result {
        Err(Error::Ambiguous) => telemetry::rest_ambiguous(class),
        Err(Error::InvalidPayload) => telemetry::rest_invalid(class),
        _ => {}
    }
    if total_started.elapsed() >= Duration::from_secs(2) {
        telemetry::rest_slow(class);
        warn!(
            route = class.name(),
            elapsed_ms = total_started.elapsed().as_millis().min(u64::MAX as u128) as u64,
            "Slow Discord REST call"
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn ambiguous_post_is_not_retried_and_429_is_retried_after_wait() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let ambiguous_calls = Arc::new(AtomicUsize::new(0));
        let ambiguous_counter = ambiguous_calls.clone();
        let app = axum::Router::new()
            .route(
                "/ambiguous",
                axum::routing::post(move || {
                    let count = ambiguous_counter.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR
                    }
                }),
            )
            .route(
                "/rate",
                axum::routing::post(move || {
                    let calls = counted.clone();
                    async move {
                        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                            (
                                axum::http::StatusCode::TOO_MANY_REQUESTS,
                                "{\"retry_after\":0.05}",
                            )
                        } else {
                            (axum::http::StatusCode::OK, "{\"id\":\"42\"}")
                        }
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = reqwest::Client::new();
        assert!(matches!(
            send(
                &client,
                client.post(format!("http://{address}/ambiguous")),
                "test:ambiguous".into()
            )
            .await,
            Err(Error::Ambiguous)
        ));
        assert_eq!(ambiguous_calls.load(Ordering::SeqCst), 1);
        let started = Instant::now();
        assert_eq!(
            send(
                &client,
                client.post(format!("http://{address}/rate")),
                "test:rate".into()
            )
            .await
            .unwrap(),
            "{\"id\":\"42\"}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(started.elapsed() >= Duration::from_millis(50));
        server.abort();
    }
    #[test]
    fn server_wait_is_not_shortened_and_nonfinite_values_are_rejected() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("retry-after", "20".parse().unwrap());
        assert_eq!(retry_after(&headers, "{}"), Duration::from_secs(20));
        headers.insert("retry-after", "NaN".parse().unwrap());
        assert_eq!(
            retry_after(&headers, r#"{"retry_after":12}"#),
            Duration::from_secs(12)
        );
        assert!(!Error::Ambiguous.safe_fallback());
        assert!(!Error::RateLimited(Duration::ZERO).safe_fallback());
        assert!(Error::Rejected(403).safe_fallback());
    }
}
