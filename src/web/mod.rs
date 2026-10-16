mod auth;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{Next, from_fn, from_fn_with_state},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

use crate::config::Config;
use crate::settings::AppSettings;

pub type SharedSettings = Arc<RwLock<AppSettings>>;
type RolesCache = Arc<tokio::sync::Mutex<Option<(std::time::Instant, Vec<DiscordGuildRolesDto>)>>>;
type ChannelsCache =
    Arc<tokio::sync::Mutex<Option<(std::time::Instant, Vec<DiscordGuildChannelsDto>)>>>;

/// WebUI'ın kendini tanıtması için gereken bilgiler: sürüm, port ve `.env` kaynaklı
/// yedek alan adları (WebUI'daki alan boş bırakıldığında geçerli olan değerler).
///
/// Panel bu bilgileri `/api/info`'dan alır: başlıkta sabit "PORT: 3000" yazmak yerine
/// gerçek port/sürüm gösterilir, alan adı kutularının ipucu da gerçek `.env` değerini yazar.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WebInfo {
    /// Semver taban sürüm (ör. `0.2.0`) — **geriye dönük uyumlu** alan.
    pub version: String,
    /// `v` önekli tam sürüm (ör. `v0.2.0+5.g8fc2692`) — panel bunu gösterir.
    pub full_version: String,
    /// `v` öneksiz tam sürüm (ör. `0.2.0+5.g8fc2692`) — araçlar karşılaştırabilsin.
    pub display_version: String,
    /// Tam etiket isabeti + temiz ağaç (yayın sürümü) mı?
    pub is_release: bool,
    /// Derleme sırasında çalışma ağacı kirli miydi?
    pub is_dirty: bool,
    /// Kısaltılmış commit kimliği (`unknown` olabilir).
    pub git_sha: String,
    /// En yakın etiket (etiket yoksa `null`).
    pub git_tag: Option<String>,
    /// Etiketten bu yana commit sayısı (etiket yoksa `null`).
    pub commits_since_tag: Option<u32>,
    /// Derleme zamanı (UTC, ISO-8601; `SOURCE_DATE_EPOCH` ile tekrarlanabilir).
    pub build_date: String,
    /// Derlenen hedef üçlü (ör. `aarch64-unknown-linux-gnu`).
    pub target: String,
    pub port: u16,
    pub env_domains: std::collections::BTreeMap<String, String>,
}

impl WebInfo {
    pub fn from_config(config: &Config) -> Self {
        use crate::version;

        let mut env_domains = std::collections::BTreeMap::new();
        for (key, value) in [
            ("tiktok", &config.tiktok_domain),
            ("bluesky", &config.bluesky_domain),
            ("spotify", &config.spotify_domain),
        ] {
            env_domains.insert(key.to_string(), value.clone());
        }

        Self {
            version: version::SEMVER.to_string(),
            full_version: version::FULL.to_string(),
            display_version: version::DISPLAY.to_string(),
            is_release: version::IS_RELEASE,
            is_dirty: version::IS_DIRTY,
            git_sha: version::GIT_SHA.to_string(),
            git_tag: version::tag().map(str::to_string),
            commits_since_tag: version::commits_since_tag(),
            build_date: version::BUILD_DATE.to_string(),
            target: version::TARGET.to_string(),
            port: config.web_port,
            env_domains,
        }
    }
}

#[derive(Clone)]
pub struct WebState {
    pub settings: SharedSettings,
    pub http: Arc<serenity::all::Http>,
    pub cache: Arc<serenity::all::Cache>,
    pub info: WebInfo,
    pub auth: Arc<auth::Auth>,
    pub save_lock: Arc<tokio::sync::Mutex<()>>,
    pub settings_path: Arc<std::path::PathBuf>,
    pub roles_cache: RolesCache,
    pub channels_cache: ChannelsCache,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscordRoleDto {
    pub id: String,
    pub name: String,
    pub color: String,
    pub position: u16,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscordGuildRolesDto {
    pub guild_id: String,
    pub guild_name: String,
    pub roles: Vec<DiscordRoleDto>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscordChannelDto {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub parent_id: Option<String>,
    pub nsfw: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscordGuildChannelsDto {
    pub guild_id: String,
    pub guild_name: String,
    pub channels: Vec<DiscordChannelDto>,
}

pub async fn start_web_server(
    settings: SharedSettings,
    config: &Config,
    http: Arc<serenity::all::Http>,
    cache: Arc<serenity::all::Cache>,
) {
    let port = config.web_port;
    let secure = crate::config::env_bool("WEB_SECURE_COOKIE", false);
    let auth = match auth::Auth::new(std::env::var("ADMIN_TOKEN").unwrap_or_default(), secure) {
        Ok(auth) => auth,
        Err(reason) => {
            error!("WebUI disabled: {reason}");
            return;
        }
    };
    let bind = match std::env::var("WEB_BIND")
        .unwrap_or_else(|_| "127.0.0.1".into())
        .parse::<std::net::IpAddr>()
    {
        Ok(bind) => bind,
        Err(_) => {
            error!("WebUI disabled: invalid WEB_BIND");
            return;
        }
    };
    let state = WebState {
        settings,
        http,
        cache,
        info: WebInfo::from_config(config),
        auth,
        save_lock: Arc::new(tokio::sync::Mutex::new(())),
        settings_path: Arc::new(crate::settings::resolve_settings_path()),
        roles_cache: Arc::new(tokio::sync::Mutex::new(None)),
        channels_cache: Arc::new(tokio::sync::Mutex::new(None)),
    };

    let app = build_router(state);

    let addr = SocketAddr::from((bind, port));
    info!("WebUI başlatılıyor: http://localhost:{}", port);

    match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => {
            // Peer addresses key the login failure limiter (`auth::login`).
            let service = app.into_make_service_with_connect_info::<SocketAddr>();
            if let Err(e) = axum::serve(listener, service).await {
                error!("WebUI sunucusunda hata oluştu: {:?}", e);
            }
        }
        Err(e) => {
            error!("WebUI {} portuna bağlanamadı: {:?}", port, e);
        }
    }
}

/// Panelin rotalarını kurar (sunucu ve testler aynı yapıyı kullanır).
fn build_router(state: WebState) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/app.js", get(app_js_handler))
        .route("/api/settings", get(get_settings).post(save_settings))
        .route("/api/info", get(get_info))
        .route("/api/discord/roles", get(get_discord_roles))
        .route("/api/discord/channels", get(get_discord_channels))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/ready", get(readiness))
        .route("/api/metrics", get(metrics))
        .route_layer(from_fn_with_state(state.auth.clone(), auth::require_login))
        .route("/login", get(auth::page))
        .route("/login.js", get(auth::script))
        .route("/app.css", get(app_css_handler))
        .route("/api/auth/login", post(auth::login))
        .route("/api/health", get(health_check))
        .layer(from_fn(auth::csrf))
        .layer(from_fn(add_security_headers))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(state)
}

async fn add_security_headers(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(
        "X-Content-Type-Options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    headers.insert(
        "Referrer-Policy",
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        "Permissions-Policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    // Yönetim paneli (rol ID'leri, sistem istemi) önbelleğe alınmamalı
    headers.insert("Cache-Control", HeaderValue::from_static("no-store"));
    // Panel kendi varlıklarını (/app.css, /app.js) servis eder ve **satır içi olay
    // yöneticisi kullanmaz**: bu yüzden script-src yalnızca 'self' olabilir, enjekte
    // edilen bir metin çalıştırılamaz. style-src'teki 'unsafe-inline' yalnızca
    // CSSOM ile yapılan dinamik stil atamaları ve Google Fonts için bırakıldı.
    headers.insert(
        "Content-Security-Policy",
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; \
             font-src 'self' https://fonts.gstatic.com; img-src 'self' data:; connect-src 'self'; \
             base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
        ),
    );
    response
}

async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

async fn readiness() -> impl IntoResponse {
    let discord = crate::telemetry::DISCORD_READY.load(std::sync::atomic::Ordering::Relaxed);
    let database = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        crate::store::blocking(crate::store::healthy),
    )
    .await
    .unwrap_or(false);
    (
        if discord && database {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(serde_json::json!({"discord": discord, "persistent_database": database})),
    )
}

async fn metrics() -> impl IntoResponse {
    Json(crate::telemetry::snapshot())
}

async fn get_settings(State(state): State<WebState>) -> impl IntoResponse {
    let s = state.settings.read().await;
    Json((*s).clone())
}

/// Panelin başlık bilgileri: sürüm, gerçek WebUI portu ve `.env` kaynaklı alan adları.
async fn get_info(State(state): State<WebState>) -> impl IntoResponse {
    Json(state.info.clone())
}

async fn get_discord_roles(State(state): State<WebState>) -> impl IntoResponse {
    // One refresh at a time; concurrent panels reuse the same short-lived snapshot.
    let mut cached = state.roles_cache.lock().await;
    if let Some((created, roles)) = cached.as_ref()
        && created.elapsed() < std::time::Duration::from_secs(30)
    {
        return Json(roles.clone());
    }
    let mut result: Vec<DiscordGuildRolesDto> = Vec::new();
    let mut uncached_guild_ids = Vec::new();

    // 1. Önce cache'deki sunucuları ve rollerini topla (CacheRef senkron blokta düşürülür)
    {
        let guild_ids = state.cache.guilds();
        for guild_id in guild_ids {
            match state.cache.guild(guild_id) {
                Some(guild) => {
                    let mut roles_list: Vec<DiscordRoleDto> = guild
                        .roles
                        .values()
                        .filter(|r| r.name != "@everyone")
                        .map(role_dto)
                        .collect();

                    roles_list.sort_by_key(|a| std::cmp::Reverse(a.position));

                    result.push(DiscordGuildRolesDto {
                        guild_id: guild.id.to_string(),
                        guild_name: guild.name.clone(),
                        roles: roles_list,
                    });
                }
                _ => {
                    uncached_guild_ids.push(guild_id);
                }
            }
        }
    }

    // At most four cache misses at once; a slow guild cannot wait indefinitely.
    for chunk in uncached_guild_ids.chunks(4) {
        let mut jobs = tokio::task::JoinSet::new();
        for &guild_id in chunk {
            let http = state.http.clone();
            jobs.spawn(async move {
                let roles = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    http.get_guild_roles(guild_id),
                )
                .await
                .ok()?
                .ok()?;
                let mut roles: Vec<_> = roles
                    .iter()
                    .filter(|r| r.name != "@everyone")
                    .map(role_dto)
                    .collect();
                roles.sort_by_key(|r| std::cmp::Reverse(r.position));
                Some(DiscordGuildRolesDto {
                    guild_id: guild_id.to_string(),
                    guild_name: format!("Sunucu {guild_id}"),
                    roles,
                })
            });
        }
        while let Some(result_item) = jobs.join_next().await {
            if let Ok(Some(guild)) = result_item {
                result.push(guild);
            }
        }
    }
    result.sort_by(|a, b| a.guild_id.cmp(&b.guild_id));
    *cached = Some((std::time::Instant::now(), result.clone()));
    Json(result)
}

fn supported_channel_kind(kind: serenity::all::ChannelType) -> bool {
    matches!(
        kind,
        serenity::all::ChannelType::Text
            | serenity::all::ChannelType::News
            | serenity::all::ChannelType::Forum
            | serenity::all::ChannelType::NewsThread
            | serenity::all::ChannelType::PublicThread
            | serenity::all::ChannelType::PrivateThread
    )
}

fn channel_dto(channel: &serenity::all::GuildChannel) -> Option<DiscordChannelDto> {
    if !supported_channel_kind(channel.kind) {
        return None;
    }
    let kind = match channel.kind {
        serenity::all::ChannelType::Text => "text",
        serenity::all::ChannelType::News => "news",
        serenity::all::ChannelType::Forum => "forum",
        serenity::all::ChannelType::NewsThread => "news_thread",
        serenity::all::ChannelType::PublicThread => "public_thread",
        serenity::all::ChannelType::PrivateThread => "private_thread",
        _ => return None,
    };
    Some(DiscordChannelDto {
        id: channel.id.to_string(),
        name: channel.name.clone(),
        kind: kind.to_string(),
        parent_id: channel.parent_id.map(|id| id.to_string()),
        nsfw: channel.nsfw,
    })
}

async fn get_discord_channels(State(state): State<WebState>) -> impl IntoResponse {
    let mut cached = state.channels_cache.lock().await;
    if let Some((created, channels)) = cached.as_ref()
        && created.elapsed() < std::time::Duration::from_secs(30)
    {
        return Json(channels.clone());
    }
    let mut result = Vec::new();
    for guild_id in state.cache.guilds() {
        let Some(guild) = state.cache.guild(guild_id) else {
            continue;
        };
        let mut channels: Vec<_> = guild.channels.values().filter_map(channel_dto).collect();
        channels.extend(guild.threads.iter().filter_map(channel_dto));
        channels.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        result.push(DiscordGuildChannelsDto {
            guild_id: guild.id.to_string(),
            guild_name: guild.name.clone(),
            channels,
        });
    }
    result.sort_by(|a, b| a.guild_id.cmp(&b.guild_id));
    *cached = Some((std::time::Instant::now(), result.clone()));
    Json(result)
}

fn role_dto(role: &serenity::all::Role) -> DiscordRoleDto {
    DiscordRoleDto {
        id: role.id.to_string(),
        name: role.name.clone(),
        color: if role.colour.0 == 0 {
            "#94a3b8".into()
        } else {
            format!("#{:06x}", role.colour.0)
        },
        position: role.position,
    }
}

async fn save_settings(
    State(state): State<WebState>,
    Json(mut new_settings): Json<AppSettings>,
) -> impl IntoResponse {
    new_settings.role_filters.sanitize();

    // Sınır **karakter** sayısıdır: panel sayacı (`value.length`) ve kanal ayarlarının
    // `sanitize`'ı da karakter sayar; bayt sınırı Türkçe istemleri sessizce kırpıyordu.
    fn safe_truncate_str(s: &mut String, max_chars: usize) {
        if let Some((end, _)) = s.char_indices().nth(max_chars) {
            s.truncate(end);
        }
    }

    safe_truncate_str(&mut new_settings.ai.system_prompt, 4000);
    safe_truncate_str(&mut new_settings.ai.model, 128);

    // Link dönüştürme motoru: alan adları temizlenir, sayısal aralıklar güvenli sınıra çekilir
    new_settings.engine.sanitize();
    new_settings.sanitize_channels();

    // Sunucuya özel (sparse) ayarları doğrula ve temizle. Kayıtta **yalnızca sabitlenmiş
    // bölümler** bulunur; boş kalan kayıt saklanmaz (o sunucu genel ayarları miras alır).
    new_settings
        .guilds
        .retain(|gid, _| gid.parse::<u64>().is_ok_and(|id| id > 0));

    for guild_cfg in new_settings.guilds.values_mut() {
        if let Some(filters) = guild_cfg.role_filters.as_mut() {
            filters.sanitize();
        }

        if let Some(ai) = guild_cfg.ai.as_mut() {
            safe_truncate_str(&mut ai.system_prompt, 4000);
            safe_truncate_str(&mut ai.model, 128);
        }

        if let Some(engine) = guild_cfg.engine.as_mut() {
            engine.sanitize();
        }
    }

    // "Miras" durumundaki (hiçbir bölümü sabitlenmemiş) kayıtlar saklanmaz.
    new_settings.guilds.retain(|_, over| !over.is_empty());

    // Şema sürümü panelden gelmez; sunucudaki değer **korunur**. Aksi hâlde sürüm 0'a düşer
    // ve sunucu ayarları göçü her açılışta yeniden çalışarak bilerek sabitlenen bölümleri
    // sessizce mirasa döndürebilirdi.
    // The owned task completes disk + memory publication even if the HTTP caller disconnects.
    let update = tokio::spawn(async move {
        let _writer = state.save_lock.lock_owned().await;
        new_settings.schema_version = state.settings.read().await.schema_version;
        let persisted = new_settings.clone();
        let settings_path = state.settings_path.clone();
        tokio::task::spawn_blocking(move || persisted.save_to(&settings_path))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        *state.settings.write().await = new_settings.clone();
        Ok::<_, String>(new_settings)
    })
    .await;
    match update {
        Ok(Ok(settings)) => (
            StatusCode::OK,
            Json(serde_json::json!({"success": true, "settings": settings})),
        ),
        result => {
            error!("Settings transaction failed: {result:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::json!({"success": false, "error": "Ayarlar kaydedilemedi; sunucu logunu kontrol edin"}),
                ),
            )
        }
    }
}

async fn index_handler() -> Html<&'static str> {
    Html(INDEX_HTML)
}

/// Panelin gömülü varlıkları. Derleme sırasında ikiliye gömülür: çalışma anında
/// `src/web/ui` dizinine ihtiyaç yoktur (systemd/Docker dağıtımı değişmez), görünüm
/// katmanı ise Rust metni yerine gerçek HTML/CSS/JS dosyalarında yaşar.
const INDEX_HTML: &str = include_str!("ui/index.html");
const APP_CSS: &str = include_str!("ui/app.css");
const APP_JS: &str = include_str!("ui/app.js");

/// Panelin stil dosyası (gömülü). Önbellek başlığı ara katmandan (`no-store`) gelir.
async fn app_css_handler() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], APP_CSS)
}

/// Panelin istemci mantığı (gömülü). Panelde satır içi olay yöneticisi
/// kullanılmadığı için CSP `script-src 'self'` ile sıkı tutulabilir.
async fn app_js_handler() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        APP_JS,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            discord_token: "test-token".to_string(),
            tiktok_domain: "tnktok.com".to_string(),
            bluesky_domain: "fxbsky.app".to_string(),
            spotify_domain: "open.fixspotify.com".to_string(),
            delete_original: true,
            openrouter_api_key: None,
            reddit_client_id: "test-id".to_string(),
            instagram_sessionid: None,
            web_port: 8123,
        }
    }

    /// Panelin başlık bilgileri gerçek yapılandırmadan üretilir: sabit "PORT: 3000" yok.
    #[test]
    fn info_payload_carries_version_port_and_env_domains() {
        let info = WebInfo::from_config(&config());

        assert_eq!(info.port, 8123);
        assert_eq!(info.version, crate::version::SEMVER);
        // Sürüm alanları derleme zamanında gömülen değerlerle tutarlı olmalı: panel ve
        // `/api/info` hangi ikilinin çalıştığını kesin olarak gösterebilsin.
        assert_eq!(info.full_version, crate::version::FULL);
        assert_eq!(info.display_version, crate::version::DISPLAY);
        assert_eq!(info.is_release, crate::version::IS_RELEASE);
        assert_eq!(info.git_sha, crate::version::GIT_SHA);
        assert_eq!(info.git_tag.as_deref(), crate::version::tag());
        assert_eq!(info.commits_since_tag, crate::version::commits_since_tag());
        assert_eq!(info.build_date, crate::version::BUILD_DATE);
        assert_eq!(info.target, crate::version::TARGET);
        assert!(
            info.full_version.starts_with('v'),
            "panel biçimi: {}",
            info.full_version
        );
        assert_eq!(
            info.env_domains.get("tiktok").map(String::as_str),
            Some("tnktok.com")
        );
        assert_eq!(
            info.env_domains.get("bluesky").map(String::as_str),
            Some("fxbsky.app")
        );
        assert_eq!(
            info.env_domains.get("spotify").map(String::as_str),
            Some("open.fixspotify.com")
        );
        assert_eq!(
            info.env_domains.len(),
            3,
            "yalnızca proxy motoru kullanılabilen platformların yedek alan adı taşınır"
        );
        assert!(
            !info.env_domains.contains_key("twitter"),
            "Twitter/X native-only: yedek alan adı yok"
        );
    }

    /// **Varlık sözleşmesi:** görünüm katmanı ayrı dosyalara taşındığı için
    /// (`ui/index.html` / `ui/app.css` / `ui/app.js`) panel ile ayar modeli arasındaki bağ
    /// artık derleyicinin göremediği bir sözleşmedir. Bu test kopuk bağları yakalar:
    /// HTML'de bulunmayan bir kimliğe erişen istemci, stilini kaybetmiş bir sınıf, panelde
    /// anahtarı olmayan bir ayar alanı ya da sunucuda rotası olmayan bir API isteği.
    #[test]
    fn panel_assets_agree_on_ids_classes_controls_and_routes() {
        use std::collections::BTreeSet;

        /// Desenin birinci grubunu küme olarak toplar (yinelenenler tekilleşir).
        fn capture(haystack: &str, pattern: &str) -> BTreeSet<String> {
            regex::Regex::new(pattern)
                .expect("desen geçerli olmalı")
                .captures_iter(haystack)
                .map(|captures| captures[1].to_string())
                .collect()
        }

        let page_ids = capture(INDEX_HTML, r#"id="([^"]+)""#);
        assert!(page_ids.len() > 50, "panel alanları eksik görünüyor");

        // 1) İstemci yalnızca sayfada var olan kimliklere erişir.
        let looked_up = capture(APP_JS, r#"\$\('([^']+)'\)"#);
        assert!(!looked_up.is_empty(), "istemci mantığı alanları bulmalı");
        let missing_ids: Vec<&String> = looked_up.difference(&page_ids).collect();
        assert!(
            missing_ids.is_empty(),
            "panelde bulunmayan kimlikler: {missing_ids:?}"
        );

        // 2) İstemcinin değiştirdiği her sınıfın stili vardır (bölme sırasında stil kaybı
        //    sessizce bozuk görünüme yol açıyordu).
        let toggled = capture(APP_JS, r#"classList\.(?:add|remove|toggle)\(\s*'([\w-]+)'"#);
        let styled = capture(APP_CSS, r#"\.([A-Za-z][\w-]*)"#);
        let unstyled: Vec<&String> = toggled.difference(&styled).collect();
        assert!(unstyled.is_empty(), "stili olmayan sınıflar: {unstyled:?}");

        // 3) Panel anahtarları gerçek ayar şemasıyla **iki yönlü** örtüşür: eksik anahtar
        //    düzenlenemeyen bir ayar, fazla anahtar sessizce kaybolan bir değer demektir.
        let engine = serde_json::to_value(crate::settings::EngineSettings::default())
            .expect("motor ayarları serileştirilmeli");
        let engine = engine.as_object().expect("motor ayarları nesne olmalı");

        let booleans = |value: &serde_json::Value| value.is_boolean();
        let expected_switches: BTreeSet<String> = engine
            .iter()
            .filter(|(_, value)| booleans(value))
            .map(|(key, _)| key.clone())
            .collect();
        assert_eq!(
            capture(INDEX_HTML, r#"data-engine-switch="([a-z_]+)""#),
            expected_switches,
            "her mantıksal motor ayarının panelde bir anahtarı olmalı"
        );

        let expected_ranges: BTreeSet<String> = engine
            .iter()
            .filter(|(_, value)| value.is_number())
            .map(|(key, _)| key.clone())
            .collect();
        assert_eq!(
            capture(INDEX_HTML, r#"data-engine-range="([a-z_]+)""#),
            expected_ranges,
            "sayısal motor ayarlarının panelde kaydırıcısı olmalı"
        );

        let expected_emojis: BTreeSet<String> = engine["emojis"]
            .as_object()
            .expect("emoji seti nesne olmalı")
            .keys()
            .cloned()
            .collect();
        assert_eq!(
            capture(INDEX_HTML, r#"data-emoji-key="([a-z_]+)""#),
            expected_emojis,
            "emoji setindeki her alanın kutusu olmalı"
        );

        let platforms: BTreeSet<String> = engine["domains"]
            .as_object()
            .expect("alan adları nesne olmalı")
            .keys()
            .cloned()
            .collect();
        assert_eq!(
            capture(INDEX_HTML, r#"data-domain-key="([a-z_]+)""#),
            platforms,
            "her platform için yedek alan adı kutusu olmalı"
        );
        assert_eq!(
            capture(INDEX_HTML, r#"name="engine-mode-([a-z_]+)""#),
            platforms,
            "her platform için motor (proxy/native) seçimi olmalı"
        );

        let modules = serde_json::to_value(crate::settings::AppSettings::default().modules)
            .expect("modüller serileştirilmeli");
        let module_keys: BTreeSet<String> = modules
            .as_object()
            .expect("modüller nesne olmalı")
            .keys()
            .cloned()
            .collect();
        assert_eq!(
            capture(INDEX_HTML, r#"data-module-key="([a-z_]+)""#),
            module_keys,
            "her modülün panelde bir anahtarı olmalı"
        );

        // 4) İstemci yalnızca sunucuda tanımlı API yollarını çağırır (bkz. `build_router`).
        let routed: BTreeSet<String> = [
            "/api/settings",
            "/api/info",
            "/api/discord/roles",
            "/api/discord/channels",
            "/api/health",
            "/api/auth/logout",
            "/api/ready",
            "/api/metrics",
        ]
        .iter()
        .map(|path| (*path).to_string())
        .collect();
        let called = capture(APP_JS, r#"'(/api/[a-z/]*[a-z])'"#);
        assert!(!called.is_empty(), "panel API uçlarını çağırmalı");
        let unrouted: Vec<&String> = called.difference(&routed).collect();

        // 5) **Tek çözümleme kaynağı:** panel, sunucu ayarını bölüm bazlı sparse çözümlemeli
        //    (Rust `resolve_settings` ile aynı semantik) ve sunucuya yalnızca sabitlenen
        //    bölümleri göndermelidir. Aksi hâlde sunucu kaydı yeniden tam kopyaya dönerdi.
        for marker in [
            "function resolveSettings",
            "function buildGuildsPayload",
            "function resetBlockToGlobal",
        ] {
            assert!(
                APP_JS.contains(marker),
                "panelde bölüm bazlı sparse çözümleme izi bulunamadı: {marker}"
            );
        }
        assert!(
            unrouted.is_empty(),
            "rotası olmayan API yolları: {unrouted:?}"
        );
    }

    /// Panel HTML'i **dengeli** ve **beklenen şekilde iç içe** olmalıdır.
    ///
    /// Kapanmayan tek bir `<div>`, tarayıcının hata düzeltmesiyle izleyen TÜM bölümü yanlış bir
    /// kapsayıcıya hapseder: "medya" bölümündeki dağılma tam buydu — kaldırılan "medya yöntemi"
    /// seçiminden artakalan `<div class="tile tile--static">` + `<div class="tile-body">`
    /// sarmalayıcıları kapanmadan kalmış, ızgaranın geri kalanı ve emoji alanları
    /// `tile--static`in (satır yönlü) çocuğu olarak dar sütunlara sıkışmıştı.
    #[test]
    fn panel_markup_is_balanced_and_nested_as_expected() {
        // Yorumlar ayıklanır: içlerindeki `<...>` dizileri etiket sanılmasın.
        let mut html = INDEX_HTML.to_string();
        while let (Some(start), Some(end)) = (html.find("<!--"), html.find("-->")) {
            if end < start {
                break;
            }
            html.replace_range(start..end + 3, "");
        }

        const VOID: [&str; 14] = [
            "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
            "source", "track", "wbr",
        ];
        let tag =
            regex::Regex::new(r"<(/?)([a-zA-Z][a-zA-Z0-9-]*)([^>]*)>").expect("desen geçerli");

        // (etiket adı, ham açılış etiketi) yığını: ham etiket, sınıf denetimi için tutulur.
        let mut stack: Vec<(String, String)> = Vec::new();
        let mut fallback_ancestors: Option<Vec<String>> = None;
        // Izgaraların ataları: bir `tile` içine hapsolurlarsa alanlar dar sütunlara sıkışır.
        let mut gridd_ancestors: Vec<(String, Vec<String>)> = Vec::new();
        const GRIDS: [&str; 3] = ["tile-grid", "range-grid", "emoji-grid"];

        for captures in tag.captures_iter(&html) {
            let closing = &captures[1] == "/";
            let name = captures[2].to_lowercase();
            let raw = captures[0].to_string();
            let rest = captures[3].trim();

            for grid in GRIDS {
                if raw.contains(&format!("class=\"{grid}\"")) {
                    gridd_ancestors.push((
                        grid.to_string(),
                        stack.iter().map(|(_, raw)| raw.clone()).collect(),
                    ));
                }
            }

            if raw.contains("data-engine-switch=\"fallback_to_proxy\"") {
                // Bu anahtarın **en yakın `div` atası** doğrudan medya ızgarası olmalı.
                fallback_ancestors = Some(
                    stack
                        .iter()
                        .rev()
                        .filter(|(tag, _)| tag == "div")
                        .map(|(_, raw)| raw.clone())
                        .collect(),
                );
            }

            // Boşluk (void) elemanlar ve kendini kapatan etiketler yığına girmez.
            if VOID.contains(&name.as_str()) || rest.ends_with('/') {
                continue;
            }

            if closing {
                let (opened, _) = stack
                    .pop()
                    .unwrap_or_else(|| panic!("fazladan kapanış etiketi: </{name}>"));
                assert_eq!(
                    opened, name,
                    "yanlış kapanış: <{opened}> beklenirken </{name}>"
                );
            } else {
                stack.push((name, raw));
            }
        }

        assert!(stack.is_empty(), "kapanmayan etiketler: {stack:?}");
        assert!(
            !html.contains("tile--static"),
            "kaldırılan sarmalayıcı sınıfı geri gelmemeli: dengesizlik kaynağıydı"
        );

        let ancestors = fallback_ancestors.expect("proxy yedeği anahtarı panelde bulunamadı");
        let nearest_div = ancestors.first().expect("anahtar bir div içinde olmalı");
        assert!(
            nearest_div.contains("tile-grid"),
            "proxy yedeği anahtarı doğrudan ızgarada olmalı; en yakın div: {nearest_div}"
        );

        // Izgaralar başka bir `tile`ın çocuğu olmamalı: ekran görüntüsündeki sıkışma tam
        // olarak buydu (ızgara içeriği bir `tile--static`in satır yönlü flex'ine düşmüştü).
        for grid in GRIDS {
            assert!(
                gridd_ancestors.iter().any(|(name, _)| name == grid),
                "panelde `{grid}` bulunamadı"
            );
        }
        for (grid, ancestors) in &gridd_ancestors {
            assert!(
                !ancestors.iter().any(|tag| tag.contains("tile")),
                "`{grid}` bir `tile` içine hapsolmuş: {ancestors:?}"
            );
        }
    }

    /// CSP `script-src 'self'` ile sıkı tutulduğu için panelde **satır içi** kod ve olay
    /// yöneticisi bulunmamalıdır; aksi hâlde sertleştirme paneli sessizce kırardı.
    #[test]
    fn panel_has_no_inline_scripts_or_handlers() {
        assert_eq!(
            INDEX_HTML.matches("<script").count(),
            1,
            "istemci mantığı yalnızca bir dış dosyadan yüklenmeli"
        );
        assert!(
            INDEX_HTML.contains(r#"<script src="/app.js""#),
            "script etiketi dış dosyayı göstermeli"
        );

        for attribute in [
            "onclick=",
            "onchange=",
            "oninput=",
            "onsubmit=",
            "onkeyup=",
            "onkeydown=",
            "onblur=",
            "onfocus=",
            "onload=",
            "onerror=",
            "onmouseover=",
            "ondragstart=",
        ] {
            assert!(
                !INDEX_HTML.contains(attribute),
                "satır içi olay yöneticisi CSP ile uyumlu değil: {attribute}"
            );
        }
        assert!(
            !INDEX_HTML.contains("style=\""),
            "satır içi stil kullanılmamalı (CSSOM ile atama serbest)"
        );
        assert!(
            !APP_JS.contains("new Function(") && !APP_JS.contains("eval("),
            "CSP eval/Function kullanımını engeller"
        );
    }

    /// Panel, gömülü varlıkları ve API uçları gerçek HTTP isteğiyle doğrulanır.
    #[tokio::test]
    async fn web_api_serves_the_panel_and_info_payload() {
        let settings_path = std::env::temp_dir().join(format!(
            "botgek-web-test-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = WebState {
            settings: Arc::new(RwLock::new(AppSettings::default())),
            http: Arc::new(serenity::all::Http::new("test-token")),
            cache: Arc::new(serenity::all::Cache::new()),
            info: WebInfo::from_config(&config()),
            auth: auth::Auth::new("test-administrator-token-32-characters".into(), false).unwrap(),
            save_lock: Arc::new(tokio::sync::Mutex::new(())),
            settings_path: Arc::new(settings_path.clone()),
            roles_cache: Arc::new(tokio::sync::Mutex::new(None)),
            channels_cache: Arc::new(tokio::sync::Mutex::new(None)),
        };

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("test dinleyicisi");
        let addr = listener.local_addr().expect("adres");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, build_router(state)).await;
        });

        let base = format!("http://{addr}");
        let anonymous = reqwest::Client::new();
        assert_eq!(
            anonymous
                .get(format!("{base}/api/settings"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            anonymous
                .post(format!("{base}/api/auth/login"))
                .json(&serde_json::json!({"token":"incorrect"}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            anonymous
                .post(format!("{base}/api/auth/login"))
                .header("x-botgek-request", "1")
                .json(&serde_json::json!({"token":"incorrect"}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let login = reqwest::Client::new()
            .post(format!("{base}/api/auth/login"))
            .header("x-botgek-request", "1")
            .json(&serde_json::json!({"token": "test-administrator-token-32-characters"}))
            .send()
            .await
            .unwrap();
        assert_eq!(login.status(), StatusCode::NO_CONTENT);
        let cookie = login.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::COOKIE, cookie.parse().unwrap());
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .unwrap();

        // 1) Panel (HTML) servis edilir: dinamik alanlar ve gömülü varlık bağlantıları.
        let page = client
            .get(format!("{base}/"))
            .send()
            .await
            .expect("panel isteği")
            .text()
            .await
            .expect("panel gövdesi");
        assert!(
            page.contains("id=\"app-version\""),
            "sürüm alanı panelde olmalı"
        );
        assert!(
            page.contains("id=\"system-status\""),
            "durum alanı panelde olmalı"
        );
        assert!(page.contains("/app.css"), "panel stil dosyasını istemeli");
        assert!(page.contains("/app.js"), "panel istemci mantığını istemeli");
        assert!(
            !page.contains("server-inheritance-notice"),
            "kaldırılan ölü öğe panelde olmamalı"
        );
        // Panelde satır içi olay yöneticisi ve satır içi stil yoktur: yanıt sonrası
        // sertleştirilen CSP (script-src 'self') bu yüzden yeterlidir.
        assert!(
            !page.contains("onclick="),
            "satır içi olay yöneticisi CSP ile uyumlu değil"
        );
        assert!(!page.contains("style=\""), "satır içi stil kullanılmamalı");

        // 2) Stil dosyası: doğru içerik türü + katman sırası sözleşmesi.
        let css_response = client
            .get(format!("{base}/app.css"))
            .send()
            .await
            .expect("css isteği");
        assert_eq!(css_response.status(), reqwest::StatusCode::OK);
        assert!(
            css_response.headers()[reqwest::header::CONTENT_TYPE]
                .to_str()
                .expect("css içerik türü")
                .starts_with("text/css"),
            "css doğru içerik türüyle servis edilmeli"
        );
        let css = css_response.text().await.expect("css gövdesi");
        // Katman hatası (menünün sonraki panelin arkasında kalması) tekrar etmesin:
        // panellerde stacking context yaratan backdrop-filter bulunmamalı ve açılır menü
        // katmanı komut çubuğundan yüksek olmalı.
        assert!(
            !css.contains("backdrop-filter:"),
            "backdrop-filter açılır menüyü sonraki panelin arkasında bırakıyordu"
        );
        assert!(
            css.contains("--z-dropdown: 200"),
            "açılır menü katmanı tanımlı olmalı"
        );
        assert!(
            css.contains("z-index: var(--z-dropdown)"),
            "açılır menü komut çubuğunun üzerinde olmalı"
        );

        // 3) İstemci mantığı servis edilir: /api/info çağrısı ve dinamik alan yazımı.
        let js_response = client
            .get(format!("{base}/app.js"))
            .send()
            .await
            .expect("js isteği");
        assert_eq!(js_response.status(), reqwest::StatusCode::OK);
        let js = js_response.text().await.expect("js gövdesi");
        assert!(js.contains("fetchInfo()"), "panel /api/info'yu çağırmalı");
        assert!(js.contains("'app-version'"), "sürüm alanı doldurulmalı");
        assert!(js.contains("'system-status'"), "durum alanı doldurulmalı");

        // 4) /api/info gerçek sürüm/port/.env alan adlarını döner.
        let info: serde_json::Value = client
            .get(format!("{base}/api/info"))
            .send()
            .await
            .expect("info isteği")
            .json()
            .await
            .expect("info gövdesi");
        assert_eq!(info["port"], 8123);
        assert_eq!(info["version"], crate::version::SEMVER);
        assert_eq!(info["full_version"], crate::version::FULL);
        assert_eq!(info["is_release"], crate::version::IS_RELEASE);
        assert_eq!(info["git_sha"], crate::version::GIT_SHA);
        assert_eq!(info["build_date"], crate::version::BUILD_DATE);
        assert_eq!(info["target"], crate::version::TARGET);
        assert_eq!(info["env_domains"]["tiktok"], "tnktok.com");

        // 5) /api/settings eski `role_filter` alanını döndürmez (tek gerçek: role_filters).
        let settings: serde_json::Value = client
            .get(format!("{base}/api/settings"))
            .send()
            .await
            .expect("ayar isteği")
            .json()
            .await
            .expect("ayar gövdesi");
        assert!(
            settings.get("role_filter").is_none(),
            "eski çift alan API'de görünmemeli"
        );
        assert_eq!(settings["modules"]["ai_chat"], true);

        // 6) Güvenlik başlıkları varlıklarda da geçerli; sertleştirilmiş CSP satır içi
        //    script'e izin vermez (panel bu yüzden /app.js'ten yüklenir).
        let health = client
            .get(format!("{base}/api/health"))
            .send()
            .await
            .expect("sağlık isteği");
        assert_eq!(health.status(), reqwest::StatusCode::OK);
        assert_eq!(health.text().await.expect("sağlık gövdesi"), "OK");

        for (path, content_type) in [
            ("/", "text/html"),
            ("/app.css", "text/css"),
            ("/app.js", "text/javascript"),
        ] {
            let response = client
                .get(format!("{base}{path}"))
                .send()
                .await
                .expect("varlık isteği");
            assert_eq!(response.status(), reqwest::StatusCode::OK, "{path}");
            let headers = response.headers().clone();

            let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
            assert!(
                header("content-type").is_some_and(|value| value.starts_with(content_type)),
                "{path} içerik türü: {:?}",
                header("content-type")
            );
            assert_eq!(
                header("cache-control"),
                Some("no-store"),
                "{path} önbelleğe alınmamalı"
            );
            assert_eq!(header("x-content-type-options"), Some("nosniff"), "{path}");
            assert_eq!(header("x-frame-options"), Some("DENY"), "{path}");
            assert_eq!(
                header("referrer-policy"),
                Some("strict-origin-when-cross-origin"),
                "{path}"
            );

            let csp = header("content-security-policy").expect("CSP başlığı olmalı");
            assert!(csp.contains("script-src 'self'"), "{path} CSP: {csp}");
            assert!(
                !csp.contains("script-src 'unsafe-inline'"),
                "{path} satır içi script izni verilmemeli: {csp}"
            );
        }

        assert_eq!(
            client
                .post(format!("{base}/api/settings"))
                .json(&serde_json::json!({}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        let saved = client.post(format!("{base}/api/settings"))
            .header("x-botgek-request", "1")
            .json(&serde_json::json!({"schema_version":999,"role_filters":{"spotify":{"mode":"allowlist","roles":[" 42 ", "bad", "42"]}}}))
            .send().await.unwrap();
        assert_eq!(saved.status(), StatusCode::OK);
        let saved: serde_json::Value = saved.json().await.unwrap();
        assert_eq!(
            saved["settings"]["schema_version"],
            crate::settings::SETTINGS_SCHEMA_VERSION
        );
        assert_eq!(
            saved["settings"]["role_filters"]["spotify"]["roles"],
            serde_json::json!(["42"])
        );
        let disk: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert_eq!(disk, saved["settings"]);
        assert_eq!(
            client
                .get(format!("{base}/api/settings"))
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap(),
            disk
        );
        assert_eq!(
            client
                .get(format!("{base}/api/ready"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            client
                .get(format!("{base}/api/metrics"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        let first = client
            .post(format!("{base}/api/settings"))
            .header("x-botgek-request", "1")
            .json(&serde_json::json!({"modules":{"twitter":false}}));
        let second = client
            .post(format!("{base}/api/settings"))
            .header("x-botgek-request", "1")
            .json(&serde_json::json!({"modules":{"twitter":true}}));
        let (first, second) = tokio::join!(first.send(), second.send());
        assert_eq!(first.unwrap().status(), StatusCode::OK);
        assert_eq!(second.unwrap().status(), StatusCode::OK);
        let published: serde_json::Value = client
            .get(format!("{base}/api/settings"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let persisted: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert_eq!(
            published, persisted,
            "concurrent writes publish the same state to memory and disk"
        );
        std::fs::remove_file(&settings_path).unwrap();
        std::fs::create_dir(&settings_path).unwrap();
        let failed = client
            .post(format!("{base}/api/settings"))
            .header("x-botgek-request", "1")
            .json(&serde_json::json!({"ai":{"model":"must-not-publish"}}))
            .send()
            .await
            .unwrap();
        assert_eq!(failed.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            client
                .get(format!("{base}/api/settings"))
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap(),
            published
        );
        std::fs::remove_dir(&settings_path).unwrap();
        assert_eq!(
            client
                .post(format!("{base}/api/auth/logout"))
                .header("x-botgek-request", "1")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            client
                .get(format!("{base}/api/settings"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        server.abort();
    }
}
