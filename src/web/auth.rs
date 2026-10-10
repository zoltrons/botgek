//! Environment-backed administrator login. The master token never leaves this module.
use axum::{
    Json,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::sync::Mutex;

const SESSION_TTL: Duration = Duration::from_secs(8 * 3600);
const COOKIE: &str = "botgek_session";

pub struct Auth {
    token: String,
    secure: bool,
    sessions: Mutex<HashMap<String, Instant>>,
    attempts: Mutex<(Instant, u32)>,
}

impl Auth {
    pub fn new(token: String, secure: bool) -> Result<Arc<Self>, &'static str> {
        if !(32..=512).contains(&token.len()) || token.trim() != token {
            return Err(
                "ADMIN_TOKEN must contain 32–512 characters without surrounding whitespace",
            );
        }
        Ok(Arc::new(Self {
            token,
            secure,
            sessions: Mutex::new(HashMap::new()),
            attempts: Mutex::new((Instant::now(), 0)),
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

pub async fn login(State(state): State<super::WebState>, Json(input): Json<Login>) -> Response {
    let auth = &state.auth;
    {
        let mut attempts = auth.attempts.lock().await;
        if attempts.0.elapsed() >= Duration::from_secs(60) {
            *attempts = (Instant::now(), 0);
        }
        if attempts.1 >= 20 {
            return StatusCode::TOO_MANY_REQUESTS.into_response();
        }
        attempts.1 += 1;
    }
    if !bool::from(auth.token.as_bytes().ct_eq(input.token.as_bytes())) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut random = [0u8; 32];
    if getrandom::fill(&mut random).is_err() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let session: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let mut sessions = auth.sessions.lock().await;
    sessions.retain(|_, created| created.elapsed() < SESSION_TTL);
    if sessions.len() >= 64 {
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
}
