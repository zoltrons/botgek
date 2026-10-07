use axum::{
    extract::{DefaultBodyLimit, Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::{from_fn, Next},
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

use crate::config::Config;
use crate::settings::AppSettings;

pub type SharedSettings = Arc<RwLock<AppSettings>>;

/// WebUI'ın kendini tanıtması için gereken bilgiler: sürüm, port ve `.env` kaynaklı
/// yedek alan adları (WebUI'daki alan boş bırakıldığında geçerli olan değerler).
///
/// Panel bu bilgileri `/api/info`'dan alır: başlıkta sabit "PORT: 3000" yazmak yerine
/// gerçek port/sürüm gösterilir, alan adı kutularının ipucu da gerçek `.env` değerini yazar.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WebInfo {
    pub version: String,
    pub port: u16,
    pub env_domains: std::collections::BTreeMap<String, String>,
}

impl WebInfo {
    pub fn from_config(config: &Config) -> Self {
        let mut env_domains = std::collections::BTreeMap::new();
        for (key, value) in [
            ("twitter", &config.twitter_domain),
            ("tiktok", &config.tiktok_domain),
            ("instagram", &config.instagram_domain),
            ("reddit", &config.reddit_domain),
            ("bluesky", &config.bluesky_domain),
            ("spotify", &config.spotify_domain),
        ] {
            env_domains.insert(key.to_string(), value.clone());
        }

        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
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

pub async fn start_web_server(
    settings: SharedSettings,
    config: &Config,
    http: Arc<serenity::all::Http>,
    cache: Arc<serenity::all::Cache>,
) {
    let port = config.web_port;
    let state = WebState {
        settings,
        http,
        cache,
        info: WebInfo::from_config(config),
    };

    let app = build_router(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("WebUI başlatılıyor: http://localhost:{}", port);

    match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => {
            if let Err(e) = axum::serve(listener, app).await {
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
        .route("/app.css", get(app_css_handler))
        .route("/app.js", get(app_js_handler))
        .route("/api/settings", get(get_settings).post(save_settings))
        .route("/api/info", get(get_info))
        .route("/api/discord/roles", get(get_discord_roles))
        .route("/api/health", get(health_check))
        .layer(from_fn(add_security_headers))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(state)
}

async fn add_security_headers(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert("X-Content-Type-Options", HeaderValue::from_static("nosniff"));
    headers.insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    headers.insert("Referrer-Policy", HeaderValue::from_static("strict-origin-when-cross-origin"));
    headers.insert("Permissions-Policy", HeaderValue::from_static("camera=(), microphone=(), geolocation=()"));
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

async fn get_settings(State(state): State<WebState>) -> impl IntoResponse {
    let s = state.settings.read().await;
    Json((*s).clone())
}

/// Panelin başlık bilgileri: sürüm, gerçek WebUI portu ve `.env` kaynaklı alan adları.
async fn get_info(State(state): State<WebState>) -> impl IntoResponse {
    Json(state.info.clone())
}

async fn get_discord_roles(State(state): State<WebState>) -> impl IntoResponse {
    let mut result: Vec<DiscordGuildRolesDto> = Vec::new();
    let mut uncached_guild_ids = Vec::new();

    // 1. Önce cache'deki sunucuları ve rollerini topla (CacheRef senkron blokta düşürülür)
    {
        let guild_ids = state.cache.guilds();
        for guild_id in guild_ids {
            match state.cache.guild(guild_id) { Some(guild) => {
                let mut roles_list: Vec<DiscordRoleDto> = guild
                    .roles
                    .values()
                    .filter(|r| r.name != "@everyone")
                    .map(|r| {
                        let hex = format!("#{:06x}", r.colour.0);
                        DiscordRoleDto {
                            id: r.id.to_string(),
                            name: r.name.clone(),
                            color: if r.colour.0 == 0 { "#94a3b8".to_string() } else { hex },
                            position: r.position,
                        }
                    })
                    .collect();

                roles_list.sort_by_key(|a| std::cmp::Reverse(a.position));

                result.push(DiscordGuildRolesDto {
                    guild_id: guild.id.to_string(),
                    guild_name: guild.name.clone(),
                    roles: roles_list,
                });
            } _ => {
                uncached_guild_ids.push(guild_id);
            }}
        }
    }

    // 2. Cache'de bulunmayan sunucular için Discord HTTP API'ye danış
    for guild_id in uncached_guild_ids {
        if let Ok(roles) = state.http.get_guild_roles(guild_id).await {
            let mut roles_list: Vec<DiscordRoleDto> = roles
                .iter()
                .filter(|r| r.name != "@everyone")
                .map(|r| {
                    let hex = format!("#{:06x}", r.colour.0);
                    DiscordRoleDto {
                        id: r.id.to_string(),
                        name: r.name.clone(),
                        color: if r.colour.0 == 0 { "#94a3b8".to_string() } else { hex },
                        position: r.position,
                    }
                })
                .collect();

            roles_list.sort_by_key(|a| std::cmp::Reverse(a.position));

            result.push(DiscordGuildRolesDto {
                guild_id: guild_id.to_string(),
                guild_name: format!("Sunucu {}", guild_id),
                roles: roles_list,
            });
        }
    }

    Json(result)
}

async fn save_settings(
    State(state): State<WebState>,
    Json(mut new_settings): Json<AppSettings>,
) -> impl IntoResponse {
    // Güvenlik & Doğrulama: Tüm modüllerin rol ID'lerini kontrol et
    let sanitize_roles = |roles: &mut Vec<String>| {
        roles.retain(|r| {
            let trimmed = r.trim();
            !trimmed.is_empty() && trimmed.len() <= 25 && trimmed.chars().all(|c| c.is_ascii_digit())
        });
        if roles.len() > 200 {
            roles.truncate(200);
        }
    };

    sanitize_roles(&mut new_settings.role_filters.twitter.roles);
    sanitize_roles(&mut new_settings.role_filters.tiktok.roles);
    sanitize_roles(&mut new_settings.role_filters.instagram.roles);
    sanitize_roles(&mut new_settings.role_filters.reddit.roles);
    sanitize_roles(&mut new_settings.role_filters.bluesky.roles);
    sanitize_roles(&mut new_settings.role_filters.translation.roles);
    sanitize_roles(&mut new_settings.role_filters.ai_chat.roles);

    fn safe_truncate_str(s: &mut String, max_bytes: usize) {
        if s.len() > max_bytes {
            let mut end = max_bytes;
            while end > 0 && !s.is_char_boundary(end) {
                end -= 1;
            }
            s.truncate(end);
        }
    }

    safe_truncate_str(&mut new_settings.ai.system_prompt, 4000);
    safe_truncate_str(&mut new_settings.ai.model, 128);

    // Link dönüştürme motoru: alan adları temizlenir, sayısal aralıklar güvenli sınıra çekilir
    new_settings.engine.sanitize();

    // Sunucuya özel ayarları doğrula ve temizle
    new_settings.guilds.retain(|gid, _| {
        !gid.is_empty() && gid.len() <= 25 && gid.chars().all(|c| c.is_ascii_digit())
    });

    for guild_cfg in new_settings.guilds.values_mut() {
        sanitize_roles(&mut guild_cfg.role_filters.twitter.roles);
        sanitize_roles(&mut guild_cfg.role_filters.tiktok.roles);
        sanitize_roles(&mut guild_cfg.role_filters.instagram.roles);
        sanitize_roles(&mut guild_cfg.role_filters.reddit.roles);
        sanitize_roles(&mut guild_cfg.role_filters.bluesky.roles);
        sanitize_roles(&mut guild_cfg.role_filters.translation.roles);
        sanitize_roles(&mut guild_cfg.role_filters.ai_chat.roles);

        safe_truncate_str(&mut guild_cfg.ai.system_prompt, 4000);
        safe_truncate_str(&mut guild_cfg.ai.model, 128);

        guild_cfg.engine.sanitize();
    }

    if let Err(e) = new_settings.save() {
        error!("Ayarlar kaydedilirken hata: {:?}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "success": false, "error": e.to_string() })),
        );
    }

    let mut s = state.settings.write().await;
    *s = new_settings;
    info!("Ayarlar WebUI üzerinden güncellendi.");

    (
        StatusCode::OK,
        Json(serde_json::json!({ "success": true, "message": "Ayarlar kaydedildi" })),
    )
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
    ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], APP_JS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            discord_token: "test-token".to_string(),
            twitter_domain: "fixupx.com".to_string(),
            tiktok_domain: "tnktok.com".to_string(),
            instagram_domain: "ddinstagram.com".to_string(),
            reddit_domain: "vxreddit.com".to_string(),
            bluesky_domain: "fxbsky.app".to_string(),
            spotify_domain: "open.fixspotify.com".to_string(),
            delete_original: true,
            openrouter_api_key: None,
            web_port: 8123,
        }
    }

    /// Panelin başlık bilgileri gerçek yapılandırmadan üretilir: sabit "PORT: 3000" yok.
    #[test]
    fn info_payload_carries_version_port_and_env_domains() {
        let info = WebInfo::from_config(&config());

        assert_eq!(info.port, 8123);
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(info.env_domains.get("twitter").map(String::as_str), Some("fixupx.com"));
        assert_eq!(info.env_domains.get("bluesky").map(String::as_str), Some("fxbsky.app"));
        assert_eq!(
            info.env_domains.get("spotify").map(String::as_str),
            Some("open.fixspotify.com")
        );
        assert_eq!(info.env_domains.len(), 6, "altı platform alan adı taşınmalı");
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
        assert!(missing_ids.is_empty(), "panelde bulunmayan kimlikler: {missing_ids:?}");

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
        let routed: BTreeSet<String> =
            ["/api/settings", "/api/info", "/api/discord/roles", "/api/health"]
                .iter()
                .map(|path| (*path).to_string())
                .collect();
        let called = capture(APP_JS, r#"'(/api/[a-z/]*[a-z])'"#);
        assert!(!called.is_empty(), "panel API uçlarını çağırmalı");
        let unrouted: Vec<&String> = called.difference(&routed).collect();
        assert!(unrouted.is_empty(), "rotası olmayan API yolları: {unrouted:?}");
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
            "onclick=", "onchange=", "oninput=", "onsubmit=", "onkeyup=", "onkeydown=", "onblur=",
            "onfocus=", "onload=", "onerror=", "onmouseover=", "ondragstart=",
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
        let state = WebState {
            settings: Arc::new(RwLock::new(AppSettings::default())),
            http: Arc::new(serenity::all::Http::new("test-token")),
            cache: Arc::new(serenity::all::Cache::new()),
            info: WebInfo::from_config(&config()),
        };

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("test dinleyicisi");
        let addr = listener.local_addr().expect("adres");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, build_router(state)).await;
        });

        let client = reqwest::Client::new();
        let base = format!("http://{addr}");

        // 1) Panel (HTML) servis edilir: dinamik alanlar ve gömülü varlık bağlantıları.
        let page = client
            .get(format!("{base}/"))
            .send()
            .await
            .expect("panel isteği")
            .text()
            .await
            .expect("panel gövdesi");
        assert!(page.contains("id=\"app-version\""), "sürüm alanı panelde olmalı");
        assert!(page.contains("id=\"system-status\""), "durum alanı panelde olmalı");
        assert!(page.contains("/app.css"), "panel stil dosyasını istemeli");
        assert!(page.contains("/app.js"), "panel istemci mantığını istemeli");
        assert!(
            !page.contains("server-inheritance-notice"),
            "kaldırılan ölü öğe panelde olmamalı"
        );
        // Panelde satır içi olay yöneticisi ve satır içi stil yoktur: yanıt sonrası
        // sertleştirilen CSP (script-src 'self') bu yüzden yeterlidir.
        assert!(!page.contains("onclick="), "satır içi olay yöneticisi CSP ile uyumlu değil");
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
        assert!(css.contains("--z-dropdown: 200"), "açılır menü katmanı tanımlı olmalı");
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
        assert_eq!(info["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(info["env_domains"]["twitter"], "fixupx.com");

        // 5) /api/settings eski `role_filter` alanını döndürmez (tek gerçek: role_filters).
        let settings: serde_json::Value = client
            .get(format!("{base}/api/settings"))
            .send()
            .await
            .expect("ayar isteği")
            .json()
            .await
            .expect("ayar gövdesi");
        assert!(settings.get("role_filter").is_none(), "eski çift alan API'de görünmemeli");
        assert_eq!(settings["engine"]["card_style"], "modern");
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
            assert_eq!(header("cache-control"), Some("no-store"), "{path} önbelleğe alınmamalı");
            assert_eq!(header("x-content-type-options"), Some("nosniff"), "{path}");
            assert_eq!(header("x-frame-options"), Some("DENY"), "{path}");
            assert_eq!(header("referrer-policy"), Some("strict-origin-when-cross-origin"), "{path}");

            let csp = header("content-security-policy").expect("CSP başlığı olmalı");
            assert!(csp.contains("script-src 'self'"), "{path} CSP: {csp}");
            assert!(
                !csp.contains("script-src 'unsafe-inline'"),
                "{path} satır içi script izni verilmemeli: {csp}"
            );
        }

        server.abort();
    }
}

