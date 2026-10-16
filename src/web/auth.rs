//! Environment-backed administrator login. The master token never leaves this module.
use axum::{
    Json,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::sync::Mutex;

const SESSION_TTL: Duration = Duration::from_secs(8 * 3600);
const COOKIE: &str = "botgek_session";
const MAX_SESSIONS: usize = 64;
const MAX_TOKEN_LEN: usize = 512;
const FAILURE_WINDOW: Duration = Duration::from_secs(60);
const FAILURES_PER_WINDOW: u32 = 20;
/// Bounds the per-address failure table so many sources cannot grow it without limit.
const MAX_TRACKED_ADDRESSES: usize = 1024;

pub struct Auth {
    token: String,
    secure: bool,
    sessions: Mutex<HashMap<String, Instant>>,
    /// Failed logins per client address: only failures count, so neither successful logins nor another address can lock the administrator out.
    failures: Mutex<HashMap<IpAddr, (Instant, u32)>>,
}

impl Auth {
    pub fn new(token: String, secure: bool) -> Result<Arc<Self>, &'static str> {
        if !(32..=MAX_TOKEN_LEN).contains(&token.len()) || token.trim() != token {
            return Err(
                "ADMIN_TOKEN must contain 32–512 characters without surrounding whitespace",
            );
        }
        Ok(Arc::new(Self {
            token,
            secure,
            sessions: Mutex::new(HashMap::new()),
            failures: Mutex::new(HashMap::new()),
        }))
    }

    async fn valid(&self, headers: &HeaderMap) -> bool {
        let Some(session) = session_cookie(headers) else {
            return false;
        };
        let mut sessions = self.sessions.lock().await;
        sessions.retain(|_, created| created.elapsed() < SESSION_TTL);
        sessions.contains_key(session)
    }

    /// Compares in time independent of both the content and the length of the secret.
    fn token_matches(&self, candidate: &str) -> bool {
        fn padded(value: &[u8]) -> [u8; MAX_TOKEN_LEN] {
            let mut buf = [0u8; MAX_TOKEN_LEN];
            let len = value.len().min(MAX_TOKEN_LEN);
            buf[..len].copy_from_slice(&value[..len]);
            buf
        }
        let candidate = candidate.as_bytes();
        let same_len = (self.token.len() as u64).ct_eq(&(candidate.len() as u64));
        let same_bytes = padded(self.token.as_bytes()).ct_eq(&padded(candidate));
        bool::from(same_len & same_bytes)
    }

    async fn locked_out(&self, ip: IpAddr) -> bool {
        let mut failures = self.failures.lock().await;
        failures.retain(|_, (started, _)| started.elapsed() < FAILURE_WINDOW);
        failures
            .get(&ip)
            .is_some_and(|(_, count)| *count >= FAILURES_PER_WINDOW)
    }

    async fn record_failure(&self, ip: IpAddr) {
        let mut failures = self.failures.lock().await;
        if failures.len() >= MAX_TRACKED_ADDRESSES && !failures.contains_key(&ip) {
            return;
        }
        let entry = failures.entry(ip).or_insert((Instant::now(), 0));
        entry.1 = entry.1.saturating_add(1);
    }

    fn cookie(&self, value: &str, age: u64) -> String {
        format!(
            "{COOKIE}={value}; HttpOnly; SameSite=Strict; Path=/; Max-Age={age}{}",
            if self.secure { "; Secure" } else { "" }
        )
    }
}

fn session_cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| {
            let (name, value) = part.trim().split_once('=')?;
            (name == COOKIE && value.len() == 64).then_some(value)
        })
}

/// No CORS is enabled. A non-simple header makes cross-origin mutations fail preflight.
pub async fn csrf(req: Request, next: Next) -> Response {
    if !matches!(
        *req.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    ) && req
        .headers()
        .get("x-botgek-request")
        .and_then(|v| v.to_str().ok())
        != Some("1")
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(req).await
}

pub async fn require_login(State(auth): State<Arc<Auth>>, req: Request, next: Next) -> Response {
    if !auth.valid(req.headers()).await {
        return if req.uri().path() == "/" {
            Redirect::to("/login").into_response()
        } else {
            StatusCode::UNAUTHORIZED.into_response()
        };
    }
    next.run(req).await
}

#[derive(Deserialize)]
pub struct Login {
    token: String,
}

pub async fn login(
    State(state): State<super::WebState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    Json(input): Json<Login>,
) -> Response {
    let auth = &state.auth;
    let ip = peer.map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |ConnectInfo(addr)| {
        addr.ip()
    });
    if auth.locked_out(ip).await {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    if !auth.token_matches(&input.token) {
        auth.record_failure(ip).await;
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut random = [0u8; 32];
    if getrandom::fill(&mut random).is_err() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let session: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let mut sessions = auth.sessions.lock().await;
    sessions.retain(|_, created| created.elapsed() < SESSION_TTL);
    if sessions.len() >= MAX_SESSIONS {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    sessions.insert(session.clone(), Instant::now());
    (
        [(
            header::SET_COOKIE,
            auth.cookie(&session, SESSION_TTL.as_secs()),
        )],
        StatusCode::NO_CONTENT,
    )
        .into_response()
}

pub async fn logout(State(state): State<super::WebState>, headers: HeaderMap) -> Response {
    if let Some(session) = session_cookie(&headers) {
        state.auth.sessions.lock().await.remove(session);
    }
    (
        [(header::SET_COOKIE, state.auth.cookie("", 0))],
        StatusCode::NO_CONTENT,
    )
        .into_response()
}

pub async fn page() -> Html<&'static str> {
    Html(include_str!("ui/login.html"))
}
pub async fn script() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("ui/login.js"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn sessions_expire_and_missing_credentials_fail_closed() {
        assert!(Auth::new(String::new(), false).is_err());
        let auth = Auth::new("a".repeat(32), true).unwrap();
        assert!(!auth.valid(&HeaderMap::new()).await);
        let session = "b".repeat(64);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("{COOKIE}={session}").parse().unwrap(),
        );
        auth.sessions
            .lock()
            .await
            .insert(session.clone(), Instant::now());
        assert!(auth.valid(&headers).await);
        auth.sessions
            .lock()
            .await
            .insert(session, Instant::now() - SESSION_TTL);
        assert!(!auth.valid(&headers).await);
        let cookie = auth.cookie("test", 1);
        assert!(
            cookie.contains("HttpOnly")
                && cookie.contains("SameSite=Strict")
                && cookie.contains("Secure")
        );
    }

    #[test]
    fn token_comparison_requires_exact_match() {
        let token = "a".repeat(32);
        let auth = Auth::new(token.clone(), false).unwrap();
        assert!(auth.token_matches(&token));
        assert!(!auth.token_matches(&"a".repeat(31)));
        assert!(!auth.token_matches(&"a".repeat(33)));
        assert!(!auth.token_matches(&format!("{}b", "a".repeat(31))));
        assert!(!auth.token_matches(&"a".repeat(MAX_TOKEN_LEN * 2)));
        assert!(!auth.token_matches(""));
    }

    /// Yalnızca başarısız denemeler sayılır ve kilit adres başınadır.
    #[tokio::test]
    async fn failures_lock_out_only_the_offending_address() {
        let auth = Auth::new("a".repeat(32), false).unwrap();
        let attacker: IpAddr = "203.0.113.7".parse().unwrap();
        let admin: IpAddr = "127.0.0.1".parse().unwrap();
        for _ in 0..FAILURES_PER_WINDOW {
            assert!(!auth.locked_out(attacker).await);
            auth.record_failure(attacker).await;
        }
        assert!(auth.locked_out(attacker).await);
        assert!(!auth.locked_out(admin).await);
    }
}
