use axum::{
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderValue, StatusCode},
    middleware::{from_fn, Next},
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

use crate::settings::AppSettings;

pub type SharedSettings = Arc<RwLock<AppSettings>>;

#[derive(Clone)]
pub struct WebState {
    pub settings: SharedSettings,
    pub http: Arc<serenity::all::Http>,
    pub cache: Arc<serenity::all::Cache>,
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
    port: u16,
    http: Arc<serenity::all::Http>,
    cache: Arc<serenity::all::Cache>,
) {
    let state = WebState {
        settings,
        http,
        cache,
    };

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/api/settings", get(get_settings).post(save_settings))
        .route("/api/discord/roles", get(get_discord_roles))
        .route("/api/health", get(health_check))
        .layer(from_fn(add_security_headers))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(state);

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

async fn add_security_headers(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert("X-Content-Type-Options", HeaderValue::from_static("nosniff"));
    headers.insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    headers.insert("Referrer-Policy", HeaderValue::from_static("strict-origin-when-cross-origin"));
    headers.insert("Permissions-Policy", HeaderValue::from_static("camera=(), microphone=(), geolocation=()"));
    // Yönetim paneli (rol ID'leri, sistem istemi) önbelleğe alınmamalı
    headers.insert("Cache-Control", HeaderValue::from_static("no-store"));
    // Panel tek dosyalık gömülü HTML: script/style 'unsafe-inline' gerekir,
    // ancak default-src/connect-src 'self' ile dışarı veri sızıntısı ve harici script engellenir.
    headers.insert(
        "Content-Security-Policy",
        HeaderValue::from_static(
            "default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline' https://fonts.googleapis.com; \
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

async fn get_discord_roles(State(state): State<WebState>) -> impl IntoResponse {
    let mut result: Vec<DiscordGuildRolesDto> = Vec::new();
    let mut uncached_guild_ids = Vec::new();

    // 1. Önce cache'deki sunucuları ve rollerini topla (CacheRef senkron blokta düşürülür)
    {
        let guild_ids = state.cache.guilds();
        for guild_id in guild_ids {
            if let Some(guild) = state.cache.guild(guild_id) {
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
            } else {
                uncached_guild_ids.push(guild_id);
            }
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

const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="tr">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>BOTGEK // Cyber Control Center</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;500;700&family=Plus+Jakarta+Sans:wght@400;500;600;700;800&display=swap" rel="stylesheet">
    <style>
        :root {
            --bg: #06080d;
            --surface: #0c0f17;
            --surface-elevated: #131722;
            --surface-hover: #191e2c;
            --border: rgba(255, 255, 255, 0.08);
            --border-glow: rgba(99, 102, 241, 0.3);
            --primary: #6366f1;
            --primary-glow: rgba(99, 102, 241, 0.4);
            --cyan: #06b6d4;
            --emerald: #10b981;
            --rose: #f43f5e;
            --amber: #f59e0b;
            --text: #f8fafc;
            --text-secondary: #94a3b8;
            --text-muted: #64748b;
            --radius-lg: 16px;
            --radius-md: 12px;
            --radius-sm: 8px;
        }

        * { box-sizing: border-box; margin: 0; padding: 0; }

        body {
            font-family: 'Plus Jakarta Sans', -apple-system, sans-serif;
            background-color: var(--bg);
            background-image: 
                radial-gradient(circle at 50% 0%, rgba(99, 102, 241, 0.12) 0%, transparent 60%),
                radial-gradient(circle at 100% 50%, rgba(6, 182, 212, 0.06) 0%, transparent 50%);
            color: var(--text);
            min-height: 100vh;
            padding: 32px 16px 120px 16px;
            display: flex;
            justify-content: center;
        }

        .mono { font-family: 'JetBrains Mono', monospace; }

        .shell {
            width: 100%;
            max-width: 860px;
        }

        /* Top Brand Header */
        .masthead {
            display: flex;
            align-items: center;
            justify-content: space-between;
            margin-bottom: 32px;
            padding-bottom: 24px;
            border-bottom: 1px solid var(--border);
        }

        .brand {
            display: flex;
            align-items: center;
            gap: 16px;
        }

        .brand-icon {
            width: 48px;
            height: 48px;
            border-radius: var(--radius-md);
            background: linear-gradient(135deg, var(--primary), var(--cyan));
            display: flex;
            align-items: center;
            justify-content: center;
            font-size: 24px;
            box-shadow: 0 0 24px var(--primary-glow);
            position: relative;
        }

        .brand-titles h1 {
            font-size: 22px;
            font-weight: 800;
            letter-spacing: -0.5px;
            display: flex;
            align-items: center;
            gap: 8px;
        }

        .brand-titles h1 span.tag {
            font-size: 11px;
            background: rgba(99, 102, 241, 0.2);
            color: #818cf8;
            padding: 2px 8px;
            border-radius: 6px;
            border: 1px solid rgba(99, 102, 241, 0.3);
            font-weight: 600;
        }

        .brand-titles p {
            font-size: 13px;
            color: var(--text-secondary);
            margin-top: 2px;
        }

        .status-pill {
            display: flex;
            align-items: center;
            gap: 8px;
            background: rgba(16, 185, 129, 0.08);
            border: 1px solid rgba(16, 185, 129, 0.25);
            padding: 6px 14px;
            border-radius: 9999px;
            font-size: 12px;
            font-weight: 600;
            color: var(--emerald);
        }

        .pulse-dot {
            width: 8px;
            height: 8px;
            border-radius: 50%;
            background: var(--emerald);
            box-shadow: 0 0 10px var(--emerald);
            animation: pulse 2s infinite;
        }

        @keyframes pulse {
            0%, 100% { opacity: 1; transform: scale(1); }
            50% { opacity: 0.4; transform: scale(0.85); }
        }

        /* Card Panels */
        .panel {
            background: var(--surface);
            border: 1px solid var(--border);
            border-radius: var(--radius-lg);
            padding: 24px;
            margin-bottom: 24px;
            backdrop-filter: blur(16px);
            transition: border-color 0.25s, box-shadow 0.25s;
        }

        .panel:hover {
            border-color: rgba(255, 255, 255, 0.14);
        }

        .panel-title-bar {
            display: flex;
            align-items: center;
            justify-content: space-between;
            margin-bottom: 20px;
        }

        .panel-heading {
            display: flex;
            align-items: center;
            gap: 10px;
            font-size: 16px;
            font-weight: 700;
            color: #fff;
        }

        .panel-heading svg {
            width: 20px;
            height: 20px;
            color: var(--primary);
        }

        .panel-sub {
            font-size: 12px;
            color: var(--text-muted);
            margin-top: -14px;
            margin-bottom: 20px;
        }

        /* Modules Grid */
        .modules-grid {
            display: grid;
            grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
            gap: 12px;
        }

        .module-tile {
            background: var(--surface-elevated);
            border: 1px solid var(--border);
            border-radius: var(--radius-md);
            padding: 16px;
            display: flex;
            align-items: center;
            justify-content: space-between;
            transition: all 0.2s;
            cursor: pointer;
            user-select: none;
        }

        .module-tile:hover {
            background: var(--surface-hover);
            border-color: var(--border-glow);
            transform: translateY(-1px);
        }

        .module-info {
            display: flex;
            align-items: center;
            gap: 12px;
        }

        .module-icon {
            width: 38px;
            height: 38px;
            border-radius: var(--radius-sm);
            background: rgba(255, 255, 255, 0.04);
            border: 1px solid var(--border);
            display: flex;
            align-items: center;
            justify-content: center;
            font-size: 18px;
        }

        .module-meta .name {
            font-size: 14px;
            font-weight: 600;
            color: var(--text);
        }

        .module-meta .status-label {
            font-size: 11px;
            color: var(--text-muted);
            margin-top: 2px;
            font-weight: 500;
        }

        /* Modern Toggle */
        .toggle-switch {
            position: relative;
            width: 44px;
            height: 24px;
        }

        .toggle-switch input { opacity: 0; width: 0; height: 0; }

        .toggle-track {
            position: absolute;
            top: 0; left: 0; right: 0; bottom: 0;
            background: #232736;
            border-radius: 9999px;
            cursor: pointer;
            transition: all 0.25s cubic-bezier(0.4, 0, 0.2, 1);
            border: 1px solid rgba(255, 255, 255, 0.06);
        }

        .toggle-track:before {
            position: absolute;
            content: "";
            height: 18px;
            width: 18px;
            left: 2px;
            bottom: 2px;
            background: #cbd5e1;
            border-radius: 50%;
            transition: all 0.25s cubic-bezier(0.4, 0, 0.2, 1);
            box-shadow: 0 2px 4px rgba(0,0,0,0.3);
        }

        input:checked + .toggle-track {
            background: var(--primary);
            border-color: var(--primary);
            box-shadow: 0 0 12px var(--primary-glow);
        }

        input:checked + .toggle-track:before {
            transform: translateX(20px);
            background: #fff;
        }

        /* Segmented Mode Selector */
        .segmented-tabs {
            display: grid;
            grid-template-columns: repeat(3, 1fr);
            background: var(--surface-elevated);
            border: 1px solid var(--border);
            padding: 4px;
            border-radius: var(--radius-md);
            gap: 4px;
            margin-bottom: 20px;
        }

        .tab-btn {
            background: transparent;
            border: none;
            color: var(--text-secondary);
            font-family: inherit;
            font-size: 13px;
            font-weight: 600;
            padding: 10px 14px;
            border-radius: var(--radius-sm);
            cursor: pointer;
            transition: all 0.2s;
            display: flex;
            align-items: center;
            justify-content: center;
            gap: 8px;
        }

        .tab-btn:hover {
            color: var(--text);
            background: rgba(255, 255, 255, 0.03);
        }

        .tab-btn.active {
            background: var(--primary);
            color: #fff;
            box-shadow: 0 2px 8px var(--primary-glow);
        }

        /* Module Selector Tabs in Role Studio */
        .module-selector {
            display: flex;
            gap: 8px;
            overflow-x: auto;
            padding-bottom: 12px;
            margin-bottom: 16px;
            border-bottom: 1px solid var(--border);
        }

        .mod-tab-btn {
            background: var(--surface-elevated);
            border: 1px solid var(--border);
            color: var(--text-secondary);
            padding: 8px 14px;
            border-radius: var(--radius-sm);
            cursor: pointer;
            font-size: 13px;
            font-weight: 600;
            display: flex;
            align-items: center;
            gap: 8px;
            white-space: nowrap;
            transition: all 0.2s;
        }

        .mod-tab-btn:hover {
            border-color: rgba(255, 255, 255, 0.2);
            color: var(--text);
        }

        .mod-tab-btn.active {
            background: rgba(99, 102, 241, 0.15);
            border-color: var(--primary);
            color: #fff;
            box-shadow: 0 0 12px rgba(99, 102, 241, 0.25);
        }

        .mod-tab-status {
            font-family: 'JetBrains Mono', monospace;
            font-size: 10px;
            padding: 2px 6px;
            border-radius: 4px;
            background: rgba(255, 255, 255, 0.06);
        }

        /* Tag Chip Manager */
        .roles-studio {
            background: var(--surface-elevated);
            border: 1px solid var(--border);
            border-radius: var(--radius-md);
            padding: 16px;
        }

        .role-input-row {
            display: flex;
            gap: 8px;
            margin-bottom: 14px;
            flex-wrap: wrap;
        }

        /* Discord Role Picker Dropdown */
        .role-picker-wrapper {
            position: relative;
            display: inline-block;
        }

        .btn-pick-role {
            background: rgba(6, 182, 212, 0.12);
            color: #67e8f9;
            border: 1px solid rgba(6, 182, 212, 0.3);
            border-radius: var(--radius-sm);
            padding: 0 14px;
            height: 40px;
            font-size: 13px;
            font-weight: 600;
            font-family: inherit;
            cursor: pointer;
            transition: all 0.2s;
            display: flex;
            align-items: center;
            gap: 6px;
            white-space: nowrap;
        }

        .btn-pick-role:hover {
            background: var(--cyan);
            color: #000;
        }

        .role-picker-dropdown {
            position: absolute;
            top: 46px;
            left: 0;
            width: 330px;
            max-height: 380px;
            background: #0f131d;
            border: 1px solid rgba(255, 255, 255, 0.15);
            border-radius: var(--radius-md);
            box-shadow: 0 14px 36px rgba(0, 0, 0, 0.7);
            z-index: 100;
            display: none;
            flex-direction: column;
            overflow: hidden;
        }

        .role-picker-dropdown.open {
            display: flex;
        }

        .role-search-box {
            padding: 10px;
            border-bottom: 1px solid var(--border);
            background: #090c13;
        }

        .role-search-box input {
            width: 100%;
            background: #141824;
            border: 1px solid var(--border);
            border-radius: 6px;
            padding: 8px 12px;
            color: var(--text);
            font-size: 12px;
            outline: none;
        }

        .role-list-scroll {
            overflow-y: auto;
            max-height: 320px;
            padding: 6px 0;
        }

        .role-dropdown-item {
            padding: 8px 14px;
            display: flex;
            align-items: center;
            justify-content: space-between;
            cursor: pointer;
            transition: background 0.15s;
            font-size: 13px;
        }

        .role-dropdown-item:hover {
            background: rgba(255, 255, 255, 0.06);
        }

        .role-item-left {
            display: flex;
            align-items: center;
            gap: 10px;
            overflow: hidden;
        }

        .role-color-dot {
            width: 12px;
            height: 12px;
            border-radius: 50%;
            flex-shrink: 0;
        }

        .role-name-text {
            font-weight: 600;
            white-space: nowrap;
            overflow: hidden;
            text-overflow: ellipsis;
        }

        .role-id-badge {
            font-family: 'JetBrains Mono', monospace;
            font-size: 10px;
            color: var(--text-muted);
        }

        .role-check-icon {
            color: var(--emerald);
            font-weight: bold;
        }


        .role-input-row input {
            flex: 1;
            background: #090c13;
            border: 1px solid var(--border);
            border-radius: var(--radius-sm);
            padding: 10px 14px;
            color: var(--text);
            font-family: 'JetBrains Mono', monospace;
            font-size: 13px;
            outline: none;
            transition: border-color 0.2s;
        }

        .role-input-row input:focus {
            border-color: var(--primary);
        }

        .btn-add-role {
            background: rgba(99, 102, 241, 0.15);
            color: #a5b4fc;
            border: 1px solid rgba(99, 102, 241, 0.3);
            border-radius: var(--radius-sm);
            padding: 0 16px;
            font-size: 13px;
            font-weight: 600;
            font-family: inherit;
            cursor: pointer;
            transition: all 0.2s;
        }

        .btn-add-role:hover {
            background: var(--primary);
            color: #fff;
        }

        .role-chips-wrapper {
            display: flex;
            flex-wrap: wrap;
            gap: 8px;
            min-height: 38px;
            align-items: center;
        }

        .role-chip {
            background: #090c13;
            border: 1px solid rgba(255, 255, 255, 0.1);
            color: #e2e8f0;
            font-family: 'JetBrains Mono', monospace;
            font-size: 12px;
            padding: 6px 10px 6px 12px;
            border-radius: 6px;
            display: inline-flex;
            align-items: center;
            gap: 8px;
            transition: border-color 0.2s;
        }

        .role-chip:hover {
            border-color: var(--rose);
        }

        .role-chip-remove {
            cursor: pointer;
            color: var(--text-muted);
            font-weight: 700;
            font-size: 14px;
            transition: color 0.15s;
        }

        .role-chip-remove:hover {
            color: var(--rose);
        }

        .empty-chips-hint {
            font-size: 12px;
            color: var(--text-muted);
            font-style: italic;
        }

        /* AI Prompt Studio */
        .ai-field-group {
            margin-bottom: 16px;
        }

        .ai-label {
            display: flex;
            align-items: center;
            justify-content: space-between;
            font-size: 12px;
            font-weight: 700;
            color: var(--text-secondary);
            text-transform: uppercase;
            letter-spacing: 0.5px;
            margin-bottom: 8px;
        }

        .model-presets {
            display: flex;
            gap: 6px;
            margin-top: 8px;
            flex-wrap: wrap;
        }

        .preset-badge {
            background: rgba(255, 255, 255, 0.04);
            border: 1px solid var(--border);
            color: var(--text-secondary);
            font-size: 11px;
            font-family: 'JetBrains Mono', monospace;
            padding: 4px 10px;
            border-radius: 6px;
            cursor: pointer;
            transition: all 0.2s;
        }

        .preset-badge:hover {
            background: rgba(99, 102, 241, 0.15);
            color: #c7d2fe;
            border-color: var(--primary);
        }

        .text-input, .textarea-input {
            width: 100%;
            background: #090c13;
            border: 1px solid var(--border);
            border-radius: var(--radius-sm);
            padding: 12px 14px;
            color: var(--text);
            font-family: inherit;
            font-size: 13px;
            outline: none;
            transition: border-color 0.2s;
        }

        .textarea-input {
            min-height: 90px;
            line-height: 1.6;
            resize: vertical;
        }

        .text-input:focus, .textarea-input:focus {
            border-color: var(--primary);
        }

        /* Link Motoru / Embed Studio */
        .engine-row {
            display: grid;
            grid-template-columns: minmax(180px, 1fr) 170px minmax(180px, 1fr);
            gap: 10px;
            align-items: center;
            background: var(--surface-elevated);
            border: 1px solid var(--border);
            border-radius: var(--radius-md);
            padding: 12px 14px;
            margin-bottom: 10px;
        }

        .engine-platform {
            display: flex;
            align-items: center;
            gap: 10px;
        }

        .engine-platform .name {
            font-size: 13px;
            font-weight: 600;
            color: var(--text);
        }

        .engine-note {
            font-size: 10px;
            font-weight: 600;
            font-family: 'JetBrains Mono', monospace;
            color: var(--text-muted);
            margin-top: 2px;
            letter-spacing: 0.3px;
        }

        .engine-note.on { color: var(--emerald); }
        .engine-note.off { color: var(--text-muted); }

        .option-grid {
            display: grid;
            grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
            gap: 10px;
        }

        .option-tile {
            background: var(--surface-elevated);
            border: 1px solid var(--border);
            border-radius: var(--radius-md);
            padding: 12px 14px;
            display: flex;
            align-items: center;
            justify-content: space-between;
            gap: 10px;
            cursor: pointer;
            user-select: none;
            transition: all 0.2s;
        }

        .option-tile:hover {
            background: var(--surface-hover);
            border-color: var(--border-glow);
        }

        .option-tile .name {
            font-size: 12px;
            font-weight: 600;
            color: var(--text);
        }

        .option-tile .hint {
            font-size: 10px;
            color: var(--text-muted);
            margin-top: 2px;
        }

        .number-row {
            display: grid;
            grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
            gap: 10px;
            margin-top: 10px;
        }

        /* Modern kart emoji seti: kompakt metin alanları */
        .emoji-grid {
            display: grid;
            grid-template-columns: repeat(auto-fill, minmax(210px, 1fr));
            gap: 10px;
            margin-top: 10px;
        }

        .emoji-grid .text-input {
            padding: 8px 10px;
            font-size: 12px;
        }

        .engine-range {
            width: 100%;
            accent-color: var(--primary);
            cursor: pointer;
        }

        @media (max-width: 700px) {
            .engine-row { grid-template-columns: 1fr; }
        }

        /* Floating Command Bar */
        .command-bar {
            position: fixed;
            bottom: 24px;
            left: 50%;
            transform: translateX(-50%);
            width: calc(100% - 32px);
            max-width: 860px;
            background: rgba(12, 15, 23, 0.85);
            backdrop-filter: blur(20px);
            border: 1px solid rgba(255, 255, 255, 0.12);
            border-radius: var(--radius-lg);
            padding: 14px 20px;
            display: flex;
            align-items: center;
            justify-content: space-between;
            box-shadow: 0 20px 40px rgba(0, 0, 0, 0.6);
            z-index: 100;
        }

        .shortcut-hint {
            font-size: 12px;
            color: var(--text-muted);
            display: flex;
            align-items: center;
            gap: 6px;
        }

        .kbd {
            background: rgba(255, 255, 255, 0.08);
            border: 1px solid var(--border);
            padding: 2px 6px;
            border-radius: 4px;
            font-family: 'JetBrains Mono', monospace;
            font-size: 11px;
            color: var(--text-secondary);
        }

        .btn-deploy {
            background: linear-gradient(135deg, var(--primary), #4f46e5);
            color: white;
            border: none;
            padding: 10px 24px;
            border-radius: var(--radius-sm);
            font-family: inherit;
            font-weight: 700;
            font-size: 13px;
            letter-spacing: 0.2px;
            cursor: pointer;
            box-shadow: 0 4px 14px var(--primary-glow);
            transition: all 0.2s;
            display: flex;
            align-items: center;
            gap: 8px;
        }

        .btn-deploy:hover {
            filter: brightness(1.15);
            transform: translateY(-1px);
        }

        .btn-deploy:active {
            transform: translateY(1px);
        }

        /* Toast Feedback */
        .toast-notification {
            position: fixed;
            top: 24px;
            right: 24px;
            background: #0f172a;
            border: 1px solid var(--emerald);
            color: #fff;
            padding: 12px 20px;
            border-radius: var(--radius-md);
            font-size: 13px;
            font-weight: 600;
            box-shadow: 0 10px 30px rgba(0, 0, 0, 0.5);
            display: flex;
            align-items: center;
            gap: 10px;
            opacity: 0;
            transform: translateY(-12px);
            pointer-events: none;
            transition: all 0.3s cubic-bezier(0.16, 1, 0.3, 1);
            z-index: 999;
        }

        .toast-notification.visible {
            opacity: 1;
            transform: translateY(0);
        }

        @media (max-width: 640px) {
            .modules-grid { grid-template-columns: 1fr; }
            .segmented-tabs { grid-template-columns: 1fr; }
            .masthead { flex-direction: column; align-items: flex-start; gap: 16px; }
            .command-bar { flex-direction: column; gap: 12px; }
            .btn-deploy { width: 100%; justify-content: center; }
        }
    </style>
</head>
<body>
    <div class="shell">
        <!-- Brand Header -->
        <header class="masthead">
            <div class="brand">
                <div class="brand-icon">⚡</div>
                <div class="brand-titles">
                    <h1>BOTGEK <span class="tag mono">v0.1.0</span></h1>
                    <p>Discord Social Embed Converter & Smart AI Companion</p>
                </div>
            </div>
            <div class="status-pill">
                <div class="pulse-dot"></div>
                <span class="mono">SYSTEM LIVE // PORT: 3000</span>
            </div>
        </header>

        <!-- 0. SUNUCU SEÇİCİ (SERVER SWITCHER) -->
        <section class="panel server-panel" style="border-color: rgba(99, 102, 241, 0.4); background: linear-gradient(180deg, rgba(99, 102, 241, 0.08) 0%, var(--surface) 100%); margin-bottom: 24px;">
            <div class="panel-title-bar">
                <div class="panel-heading">
                    <span>🌐 Yapılandırılan Discord Sunucusu</span>
                </div>
                <span class="mono" id="server-custom-badge" style="font-size: 11px; padding: 3px 10px; border-radius: 4px; background: rgba(16, 185, 129, 0.15); color: var(--emerald); font-weight: 700;">GENEL VARSAYILAN</span>
            </div>
            <p class="panel-sub">Ayarları tüm sunucular için genel varsayılan olarak belirleyebilir veya sunucu seçerek o sunucuya özel modüller, rol kuralları ve AI prompt'u tanımlayabilirsiniz.</p>

            <div style="display: flex; gap: 12px; align-items: center; flex-wrap: wrap; margin-top: 16px;">
                <div style="flex: 1; min-width: 260px;">
                    <select id="server-select" class="text-input" style="font-size: 14px; font-weight: 600; cursor: pointer;" onchange="onServerChange()">
                        <option value="global">🌐 Genel / Varsayılan Ayarlar (Tüm Sunucular)</option>
                    </select>
                </div>
                <div id="server-custom-controls" style="display: none; align-items: center; gap: 10px;">
                    <button type="button" id="btn-toggle-custom" onclick="toggleServerCustomSettings()" style="background: rgba(99, 102, 241, 0.15); border: 1px solid var(--border-glow); color: #fff; padding: 10px 16px; border-radius: var(--radius-sm); font-size: 12px; font-weight: 700; cursor: pointer; transition: all 0.2s;">
                        ✨ Bu Sunucuya Özel Ayarla
                    </button>
                </div>
            </div>

            <div id="server-inheritance-notice" style="display: none; margin-top: 14px; padding: 10px 14px; border-radius: var(--radius-sm); background: rgba(6, 182, 212, 0.08); border: 1px solid rgba(6, 182, 212, 0.2); font-size: 12px; color: var(--cyan);">
                ℹ️ Bu sunucu şu anda <strong>Genel Varsayılan</strong> ayarları miras alıyor. Ayarları değiştirdiğinizde veya "Özel Ayarla" butonuna tıkladığınızda bu sunucuya özgü kaydedilecektir.
            </div>
        </section>

        <!-- 1. MODÜL SEÇİCİ -->
        <section class="panel">
            <div class="panel-title-bar">
                <div class="panel-heading">
                    <span>📦 Platform ve Dönüştürme Modülleri</span>
                </div>
                <span class="mono" id="active-modules-count" style="font-size: 12px; color: var(--cyan);">7/7 AKTİF</span>
            </div>
            <p class="panel-sub">İstemediğiniz platformların bağlantı dönüştürmesini buradan anında devre dışı bırakabilirsiniz.</p>

            <div class="modules-grid">
                <div class="module-tile" onclick="toggleSwitch('mod-twitter')">
                    <div class="module-info">
                        <div class="module-icon">🐦</div>
                        <div class="module-meta">
                            <div class="name">Twitter / X</div>
                            <div class="status-label mono" id="status-twitter">AKTİF</div>
                        </div>
                    </div>
                    <label class="toggle-switch" onclick="event.stopPropagation()">
                        <input type="checkbox" id="mod-twitter" onchange="onConfigModified(); updateTileMeta()">
                        <span class="toggle-track"></span>
                    </label>
                </div>

                <div class="module-tile" onclick="toggleSwitch('mod-tiktok')">
                    <div class="module-info">
                        <div class="module-icon">🎵</div>
                        <div class="module-meta">
                            <div class="name">TikTok (fxTikTok)</div>
                            <div class="status-label mono" id="status-tiktok">AKTİF</div>
                        </div>
                    </div>
                    <label class="toggle-switch" onclick="event.stopPropagation()">
                        <input type="checkbox" id="mod-tiktok" onchange="onConfigModified(); updateTileMeta()">
                        <span class="toggle-track"></span>
                    </label>
                </div>

                <div class="module-tile" onclick="toggleSwitch('mod-instagram')">
                    <div class="module-info">
                        <div class="module-icon">📷</div>
                        <div class="module-meta">
                            <div class="name">Instagram</div>
                            <div class="status-label mono" id="status-instagram">AKTİF</div>
                        </div>
                    </div>
                    <label class="toggle-switch" onclick="event.stopPropagation()">
                        <input type="checkbox" id="mod-instagram" onchange="onConfigModified(); updateTileMeta()">
                        <span class="toggle-track"></span>
                    </label>
                </div>

                <div class="module-tile" onclick="toggleSwitch('mod-reddit')">
                    <div class="module-info">
                        <div class="module-icon">🤖</div>
                        <div class="module-meta">
                            <div class="name">Reddit</div>
                            <div class="status-label mono" id="status-reddit">AKTİF</div>
                        </div>
                    </div>
                    <label class="toggle-switch" onclick="event.stopPropagation()">
                        <input type="checkbox" id="mod-reddit" onchange="onConfigModified(); updateTileMeta()">
                        <span class="toggle-track"></span>
                    </label>
                </div>

                <div class="module-tile" onclick="toggleSwitch('mod-bluesky')">
                    <div class="module-info">
                        <div class="module-icon">🦋</div>
                        <div class="module-meta">
                            <div class="name">Bluesky</div>
                            <div class="status-label mono" id="status-bluesky">AKTİF</div>
                        </div>
                    </div>
                    <label class="toggle-switch" onclick="event.stopPropagation()">
                        <input type="checkbox" id="mod-bluesky" onchange="onConfigModified(); updateTileMeta()">
                        <span class="toggle-track"></span>
                    </label>
                </div>

                <div class="module-tile" onclick="toggleSwitch('mod-translation')">
                    <div class="module-info">
                        <div class="module-icon">🇹🇷</div>
                        <div class="module-meta">
                            <div class="name">Bayrak Çevirisi</div>
                            <div class="status-label mono" id="status-translation">AKTİF</div>
                        </div>
                    </div>
                    <label class="toggle-switch" onclick="event.stopPropagation()">
                        <input type="checkbox" id="mod-translation" onchange="onConfigModified(); updateTileMeta()">
                        <span class="toggle-track"></span>
                    </label>
                </div>

                <div class="module-tile" onclick="toggleSwitch('mod-ai_chat')">
                    <div class="module-info">
                        <div class="module-icon">💬</div>
                        <div class="module-meta">
                            <div class="name">Sarcastic AI Chat</div>
                            <div class="status-label mono" id="status-ai_chat">AKTİF</div>
                        </div>
                    </div>
                    <label class="toggle-switch" onclick="event.stopPropagation()">
                        <input type="checkbox" id="mod-ai_chat" onchange="onConfigModified(); updateTileMeta()">
                        <span class="toggle-track"></span>
                    </label>
                </div>
            </div>
        </section>

        <!-- 2. ROL FİLTRESİ STUDIO -->
        <section class="panel">
            <div class="panel-title-bar">
                <div class="panel-heading">
                    <span>🛡️ Modül Bazlı Rol Filtresi Stüdyosu</span>
                </div>
                <span class="mono" id="filter-mode-badge" style="font-size: 11px; padding: 2px 8px; border-radius: 4px; background: rgba(255,255,255,0.06); color: var(--text-secondary);">DEVRE DIŞI</span>
            </div>
            <p class="panel-sub">Her platform için bağımsız kara liste (Blacklist) veya beyaz liste (Allowlist) yapılandırın.</p>

            <!-- Modül Seçim Sekmeleri -->
            <div class="module-selector">
                <button class="mod-tab-btn active" id="modtab-twitter" onclick="selectRoleModule('twitter')">
                    <span>🐦 Twitter</span>
                    <span class="mod-tab-status" id="badge-twitter">KAPALI</span>
                </button>
                <button class="mod-tab-btn" id="modtab-tiktok" onclick="selectRoleModule('tiktok')">
                    <span>🎵 TikTok</span>
                    <span class="mod-tab-status" id="badge-tiktok">KAPALI</span>
                </button>
                <button class="mod-tab-btn" id="modtab-instagram" onclick="selectRoleModule('instagram')">
                    <span>📷 Instagram</span>
                    <span class="mod-tab-status" id="badge-instagram">KAPALI</span>
                </button>
                <button class="mod-tab-btn" id="modtab-reddit" onclick="selectRoleModule('reddit')">
                    <span>🤖 Reddit</span>
                    <span class="mod-tab-status" id="badge-reddit">KAPALI</span>
                </button>
                <button class="mod-tab-btn" id="modtab-bluesky" onclick="selectRoleModule('bluesky')">
                    <span>🦋 Bluesky</span>
                    <span class="mod-tab-status" id="badge-bluesky">KAPALI</span>
                </button>
                <button class="mod-tab-btn" id="modtab-translation" onclick="selectRoleModule('translation')">
                    <span>🇹🇷 Çeviri</span>
                    <span class="mod-tab-status" id="badge-translation">KAPALI</span>
                </button>
                <button class="mod-tab-btn" id="modtab-ai_chat" onclick="selectRoleModule('ai_chat')">
                    <span>💬 AI Chat</span>
                    <span class="mod-tab-status" id="badge-ai_chat">KAPALI</span>
                </button>
            </div>

            <div class="segmented-tabs">
                <button class="tab-btn active" id="tab-disabled" onclick="setFilterMode('disabled')">
                    <span>🚫 Devre Dışı</span>
                </button>
                <button class="tab-btn" id="tab-blacklist" onclick="setFilterMode('blacklist')">
                    <span>⛔ Kara Liste (Blacklist)</span>
                </button>
                <button class="tab-btn" id="tab-allowlist" onclick="setFilterMode('allowlist')">
                    <span>✅ Beyaz Liste (Allowlist)</span>
                </button>
            </div>

            <div class="roles-studio">
                <div class="role-input-row">
                    <!-- Sunucudan Otomatik Rol Seçici Dropdown -->
                    <div class="role-picker-wrapper">
                        <button class="btn-pick-role" id="btn-toggle-picker" onclick="toggleRolePicker(event)">
                            <span>🎭 Sunucudan Rol Seç</span>
                            <span style="font-size: 10px;">▾</span>
                        </button>
                        <div class="role-picker-dropdown" id="role-picker-dropdown">
                            <div class="role-search-box">
                                <input type="text" id="role-search-input" placeholder="Rol ara..." oninput="filterRolePickerList()" onclick="event.stopPropagation()">
                            </div>
                            <div class="role-list-scroll" id="role-picker-list">
                                <div style="padding: 16px; text-align: center; color: var(--text-muted); font-size: 12px;">Sunucu rolleri yükleniyor...</div>
                            </div>
                        </div>
                    </div>

                    <input type="text" id="role-input-box" placeholder="veya manuel Rol ID girin..." onkeydown="handleRoleInputKey(event)">
                    <button class="btn-add-role" onclick="addRoleFromInput()">+ ID Ekle</button>
                </div>

                <div class="role-chips-wrapper" id="chips-container">
                    <span class="empty-chips-hint">Bu modül için eklenmiş bir rol yok.</span>
                </div>
            </div>
        </section>

        <!-- 3. AI ASİSTAN & PERSONA -->
        <section class="panel">
            <div class="panel-title-bar">
                <div class="panel-heading">
                    <span>🤖 OpenRouter AI & Sarcastic Persona</span>
                </div>
                <span class="mono" style="font-size: 11px; color: var(--cyan);">@botgekv2 REPLIES</span>
            </div>
            <p class="panel-sub">Bot etiketlendiğinde veya ona cevap verildiğinde çalışacak model ve hazırcevap karakter komutları.</p>

            <div class="ai-field-group">
                <div class="ai-label">
                    <span>Model Seçimi (OpenRouter)</span>
                </div>
                <input type="text" class="text-input mono" id="ai-model-box" oninput="onConfigModified()" placeholder="google/gemma-4-31b-it:free">
                <div class="model-presets">
                    <span class="preset-badge" onclick="setModelPreset('google/gemma-4-31b-it:free')">⚡ Gemma 4 31B (Önerilen)</span>
                    <span class="preset-badge" onclick="setModelPreset('openrouter/free')">Auto Free Router</span>
                    <span class="preset-badge" onclick="setModelPreset('meta-llama/llama-3.3-70b-instruct:free')">Llama 3.3 70B</span>
                    <span class="preset-badge" onclick="setModelPreset('mistralai/mistral-7b-instruct:free')">Mistral 7B</span>
                </div>
            </div>

            <div class="ai-field-group">
                <div class="ai-label">
                    <span>Sistem Ön Komutu (Bot Kişiliği)</span>
                    <span class="mono" id="char-counter" style="font-size: 11px; color: var(--text-muted);">0 karakter</span>
                </div>
                <textarea class="textarea-input" id="ai-prompt-box" oninput="onConfigModified(); updateCharCount()"></textarea>
            </div>
        </section>

        <!-- 4. LİNK DÖNÜŞTÜRME MOTORU (NATIVE EMBED / PROXY) -->
        <section class="panel">
            <div class="panel-title-bar">
                <div class="panel-heading">
                    <span>🔗 Link Dönüştürme Motoru (Native Embed)</span>
                </div>
                <span class="mono" id="engine-summary" style="font-size: 11px; padding: 2px 8px; border-radius: 4px; background: rgba(6, 182, 212, 0.12); color: var(--cyan);">-</span>
            </div>
            <p class="panel-sub">
                Her platform için <strong>Proxy</strong> (harici önizleme servisi) veya <strong>Native Embed</strong> (bot bağlantıyı kendi çözer, Discord embed'ini kendisi üretir) motorunu seçin.
                Alan adı boş bırakılırsa .env değeri kullanılır. Native embed başarısız olursa proxy linkine düşülür.
                <strong>Kart stili</strong> gönderimin görünümünü belirler:
                <span class="mono">modern</span> tek parça Components V2 kartıdır (video/görseller kartın içinde oynar),
                <span class="mono">otomatik</span> video içeren gönderilerde bu kartı, diğerlerinde klasik embed'i kullanır,
                <span class="mono">fxembed</span> ise her zaman klasik embed gönderir.
                Kartta <strong>profil resmi gösterilmez</strong>; yalnızca ad ve (varsa) doğrulama işareti yer alır.
                <strong>Taklit</strong> açıkken mesaj, paylaşan kullanıcının adı ve avatarıyla webhook üzerinden gönderilir;
                böylece mesajda link/mention hilelerine gerek kalmaz.
            </p>

            <div class="ai-label" style="margin-top: 4px;">
                <span>Platform Motorları</span>
                <span class="mono" style="font-size: 10px; color: var(--text-muted);">MOTOR / YEDEK ALAN ADI</span>
            </div>
            <div id="engine-rows"></div>

            <div class="ai-field-group" style="margin-top: 16px;">
                <div class="ai-label"><span>Twitter / X Veri Kaynağı</span></div>
                <select class="text-input" id="engine-twitter-source" onchange="onConfigModified(); updateEngineSummary()" style="cursor: pointer;">
                    <option value="syndication">X Syndication (üçüncü taraf yok, ücretsiz)</option>
                    <option value="fxtwitter">FxTwitter (görüntülenme, repost, topluluk notu)</option>
                    <option value="hybrid">Hibrit (önce syndication, olmazsa fxtwitter)</option>
                </select>
            </div>
            <div class="ai-label" style="margin-top: 16px;"><span>Embed İçeriği</span></div>
            <div class="option-grid">
                <div class="option-tile" onclick="toggleEngineOption('engine-auto-translate')">
                    <div>
                        <div class="name">Otomatik Türkçe çeviri</div>
                        <div class="hint">Türkçe/İngilizce dışı X gönderileri (X'in çevirisi)</div>
                    </div>
                    <label class="toggle-switch" onclick="event.stopPropagation()">
                        <input type="checkbox" id="engine-auto-translate" onchange="onConfigModified(); updateEngineSummary()">
                        <span class="toggle-track"></span>
                    </label>
                </div>
                <div class="option-tile" onclick="toggleEngineOption('engine-show-metrics')">
                    <div>
                        <div class="name">Etkileşim metrikleri</div>
                        <div class="hint">Beğeni, repost, yanıt, görüntülenme</div>
                    </div>
                    <label class="toggle-switch" onclick="event.stopPropagation()">
                        <input type="checkbox" id="engine-show-metrics" onchange="onConfigModified(); updateEngineSummary()">
                        <span class="toggle-track"></span>
                    </label>
                </div>

                <div class="option-tile" onclick="toggleEngineOption('engine-show-timestamp')">
                    <div>
                        <div class="name">Gönderi tarihi</div>
                        <div class="hint">Discord'un göreli zaman gösterimi</div>
                    </div>
                    <label class="toggle-switch" onclick="event.stopPropagation()">
                        <input type="checkbox" id="engine-show-timestamp" onchange="onConfigModified(); updateEngineSummary()">
                        <span class="toggle-track"></span>
                    </label>
                </div>

                <div class="option-tile" onclick="toggleEngineOption('engine-show-note')">
                    <div>
                        <div class="name">Topluluk notu</div>
                        <div class="hint">Varsa X notu ayrı alan olarak</div>
                    </div>
                    <label class="toggle-switch" onclick="event.stopPropagation()">
                        <input type="checkbox" id="engine-show-note" onchange="onConfigModified(); updateEngineSummary()">
                        <span class="toggle-track"></span>
                    </label>
                </div>

                <div class="option-tile" onclick="toggleEngineOption('engine-show-quote')">
                    <div>
                        <div class="name">Alıntılanan gönderi</div>
                        <div class="hint">İkinci embed olarak göster</div>
                    </div>
                    <label class="toggle-switch" onclick="event.stopPropagation()">
                        <input type="checkbox" id="engine-show-quote" onchange="onConfigModified(); updateEngineSummary()">
                        <span class="toggle-track"></span>
                    </label>
                </div>
            </div>
            <div class="ai-label" style="margin-top: 16px;"><span>Gönderim Biçimi</span></div>
            <div class="option-grid">
                <div class="option-tile">
                    <div>
                        <div class="name">Kart stili</div>
                        <div class="hint">modern: tek kart (video kartta oynar) • otomatik: video varsa modern • fxembed: klasik embed</div>
                    </div>
                    <select class="text-input" id="engine-card-style" onchange="onConfigModified(); updateEngineSummary()" style="cursor: pointer; max-width: 200px;">
                        <option value="modern">Modern kart (V2)</option>
                        <option value="auto">Otomatik (video → modern)</option>
                        <option value="fxembed">fxembed (klasik embed)</option>
                    </select>
                </div>

                <div class="option-tile" onclick="toggleEngineOption('engine-impersonate')">
                    <div>
                        <div class="name">Kullanıcıyı taklit et</div>
                        <div class="hint">Mesaj kullanıcının adı/avatarıyla webhook olarak gider (Manage Webhooks izni gerekir)</div>
                    </div>
                    <label class="toggle-switch" onclick="event.stopPropagation()">
                        <input type="checkbox" id="engine-impersonate" onchange="onConfigModified(); updateEngineSummary()">
                        <span class="toggle-track"></span>
                    </label>
                </div>

                <div class="option-tile" onclick="toggleEngineOption('engine-impersonate-skip-replies')">
                    <div>
                        <div class="name">Yanıtlarda yanıtı koru</div>
                        <div class="hint">Webhook mesajları Discord'da yanıt olamaz: yanıt olarak atılan linkler bot adına gönderilir (yanıt bağlantısı korunur)</div>
                    </div>
                    <label class="toggle-switch" onclick="event.stopPropagation()">
                        <input type="checkbox" id="engine-impersonate-skip-replies" onchange="onConfigModified(); updateEngineSummary()">
                        <span class="toggle-track"></span>
                    </label>
                </div>
            </div>
            <div class="ai-label" style="margin-top: 16px;"><span>Medya Yükleme</span></div>
            <div class="option-grid">
                <div class="option-tile" onclick="toggleEngineOption('engine-attach-images')">
                    <div>
                        <div class="name">Görselleri dosya olarak ekle</div>
                        <div class="hint">Tam çözünürlük, CDN bağımsız</div>
                    </div>
                    <label class="toggle-switch" onclick="event.stopPropagation()">
                        <input type="checkbox" id="engine-attach-images" onchange="onConfigModified(); updateEngineSummary()">
                        <span class="toggle-track"></span>
                    </label>
                </div>

                <div class="option-tile" onclick="toggleEngineOption('engine-attach-videos')">
                    <div>
                        <div class="name">Videoları dosya olarak ekle</div>
                        <div class="hint">Limit altındaysa indirilir</div>
                    </div>
                    <label class="toggle-switch" onclick="event.stopPropagation()">
                        <input type="checkbox" id="engine-attach-videos" onchange="onConfigModified(); updateEngineSummary()">
                        <span class="toggle-track"></span>
                    </label>
                </div>

                <div class="option-tile" onclick="toggleEngineOption('engine-fallback')">
                    <div>
                        <div class="name">Proxy yedeği</div>
                        <div class="hint">Native embed olmazsa proxy linki</div>
                    </div>
                    <label class="toggle-switch" onclick="event.stopPropagation()">
                        <input type="checkbox" id="engine-fallback" onchange="onConfigModified(); updateEngineSummary()">
                        <span class="toggle-track"></span>
                    </label>
                </div>
            </div>

            <div class="number-row">
                <div class="ai-field-group">
                    <div class="ai-label">
                        <span>Maks. Görsel Sayısı</span>
                        <span class="mono" id="engine-max-images-value" style="font-size: 11px; color: var(--cyan);">4 adet</span>
                    </div>
                    <input type="range" class="engine-range" id="engine-max-images" min="1" max="10" step="1" oninput="onConfigModified(); updateEngineSummary()">
                </div>

                <div class="ai-field-group">
                    <div class="ai-label">
                        <span>Maks. Video Boyutu</span>
                        <span class="mono" id="engine-max-video-value" style="font-size: 11px; color: var(--cyan);">8 MB</span>
                    </div>
                    <input type="range" class="engine-range" id="engine-max-video" min="1" max="50" step="1" oninput="onConfigModified(); updateEngineSummary()">
                </div>
            </div>

            <div class="ai-label" style="margin-top: 16px;">
                <span>Modern Kart Emoji Seti</span>
                <span class="mono" style="font-size: 10px; color: var(--text-muted);">BOŞ = UNICODE</span>
            </div>
            <p class="panel-sub" style="margin-top: 6px;">
                Sunucunuzdaki özel emojileri <span class="mono">&lt;:ad:id&gt;</span> biçiminde yazın (boş bırakılan alan Unicode emoji kullanır).
                Bu emojiler yalnızca <strong>Modern kart (Components V2)</strong> açıkken görünür.
            </p>
            <div class="emoji-grid" id="engine-emoji-grid"></div>
        </section>
        <!-- Floating Command Bar -->
        <div class="command-bar">
            <div class="shortcut-hint">
                <span>Kaydetmek için</span>
                <span class="kbd">Ctrl</span> + <span class="kbd">S</span>
                <span>veya butona tıklayın</span>
            </div>
            <button class="btn-deploy" onclick="saveSettingsToServer()">
                <span>⚡ Ayarları Güncelle</span>
            </button>
        </div>
    </div>

    <!-- Toast Notification -->
    <div class="toast-notification" id="toast-box">
        <span style="font-size: 16px;">✅</span>
        <span id="toast-msg">Ayarlar başarıyla kaydedildi!</span>
    </div>

    <script>
        let currentModule = 'twitter';
        let activeServer = 'global'; 
        let rawGuildsList = []; 
        let discordRoles = []; 

        // Link dönüştürme motoru sabitleri (Rust tarafındaki EngineSettings varsayılanlarıyla aynı)
        const ENGINE_PLATFORMS = [
            { key: 'twitter', name: 'Twitter / X', icon: '🐦' },
            { key: 'tiktok', name: 'TikTok', icon: '🎵' },
            { key: 'instagram', name: 'Instagram', icon: '📷' },
            { key: 'reddit', name: 'Reddit', icon: '🤖' },
            { key: 'bluesky', name: 'Bluesky', icon: '🦋' },
        ];

        const ENGINE_DEFAULTS = {
            modes: { twitter: 'builtin', tiktok: 'proxy', instagram: 'proxy', reddit: 'builtin', bluesky: 'builtin' },
            domains: { twitter: '', tiktok: '', instagram: '', reddit: '', bluesky: '' },
            twitter_source: 'syndication',
            attach_images: true,
            attach_videos: true,
            max_images: 4,
            max_video_mb: 8,
            show_metrics: true,
            show_timestamp: true,
            show_community_note: true,
            show_quote: true,
            auto_translate: true,
            fallback_to_proxy: true,
            impersonate_user: true,
            impersonate_skip_replies: true,
            card_style: 'modern',
            emojis: {
                verified: '<:verified:1553709836859084841>',
                replies: '<:comment:1553712370302451782>',
                reposts: '<:rt:1553712344939495424>',
                likes: '<:like:1553712395057365102>',
                views: '<:metric:1553712416242798712>',
                translation: '<:translation:1476614397249061056>',
                twitter: '<:twitter:1476642012072920355>',
                quote: '<:qutoe:1553728214998847569>',
            },
        };

        // Modern kartta kullanılan özel emoji alanları (Rust tarafındaki EmojiSettings ile aynı)
        const ENGINE_EMOJI_FIELDS = [
            { key: 'verified', label: 'Doğrulama işareti' },
            { key: 'replies', label: 'Yanıt' },
            { key: 'reposts', label: 'Repost' },
            { key: 'likes', label: 'Beğeni' },
            { key: 'views', label: 'Görüntülenme' },
            { key: 'translation', label: 'Çeviri notu' },
            { key: 'twitter', label: 'X / Twitter logosu' },
            { key: 'quote', label: 'Alıntı göstergesi' },
        ];

        let appSettings = {
            modules: {
                twitter: true,
                tiktok: true,
                instagram: true,
                reddit: true,
                bluesky: true,
                translation: true,
                ai_chat: true,
            },
            role_filters: {
                twitter: { mode: "disabled", roles: [] },
                tiktok: { mode: "disabled", roles: [] },
                instagram: { mode: "disabled", roles: [] },
                reddit: { mode: "disabled", roles: [] },
                bluesky: { mode: "disabled", roles: [] },
                translation: { mode: "disabled", roles: [] },
                ai_chat: { mode: "disabled", roles: [] },
            },
            ai: {
                model: "google/gemma-4-31b-it:free",
                system_prompt: ""
            },
            engine: JSON.parse(JSON.stringify(ENGINE_DEFAULTS)),
            guilds: {}
        };

        function getActiveConfig() {
            if (activeServer === 'global') return appSettings;
            return appSettings.guilds[activeServer] || appSettings;
        }

        // Eksik motor alanları (eski kayıtlar) varsayılanlarla doldurulur.
        function getEngineConfig(cfg) {
            cfg.engine = cfg.engine || {};
            const engine = cfg.engine;
            if (!engine.modes) engine.modes = Object.assign({}, ENGINE_DEFAULTS.modes);
            if (!engine.domains) engine.domains = Object.assign({}, ENGINE_DEFAULTS.domains);
            if (!engine.emojis) engine.emojis = Object.assign({}, ENGINE_DEFAULTS.emojis);
            Object.keys(ENGINE_DEFAULTS.emojis).forEach(key => {
                if (engine.emojis[key] === undefined || engine.emojis[key] === null) {
                    engine.emojis[key] = ENGINE_DEFAULTS.emojis[key];
                }
            });
            ['twitter_source', 'attach_images', 'attach_videos', 'max_images', 'max_video_mb',
             'show_metrics', 'show_timestamp', 'show_community_note', 'show_quote', 'auto_translate', 'fallback_to_proxy',
             'impersonate_user', 'impersonate_skip_replies', 'card_style']
                .forEach(field => {
                    if (engine[field] === undefined || engine[field] === null) {
                        engine[field] = ENGINE_DEFAULTS[field];
                    }
                });
            // Eski kayıtlardaki `components_v2` alanı kart stiline dönüştürüldü.
            if (engine.components_v2 === false) engine.card_style = 'fxembed';
            delete engine.components_v2;
            return engine;
        }

        function isServerCustomized(guildId) {
            return !!(appSettings.guilds && appSettings.guilds[guildId]);
        }

        function ensureActiveConfigCustomized() {
            if (activeServer === 'global') return;
            if (!appSettings.guilds) appSettings.guilds = {};
            if (!appSettings.guilds[activeServer]) {
                appSettings.guilds[activeServer] = {
                    modules: JSON.parse(JSON.stringify(appSettings.modules)),
                    role_filters: JSON.parse(JSON.stringify(appSettings.role_filters)),
                    ai: JSON.parse(JSON.stringify(appSettings.ai)),
                    engine: JSON.parse(JSON.stringify(getEngineConfig(appSettings)))
                };
            }
        }

        function onConfigModified() {
            if (activeServer !== 'global' && !isServerCustomized(activeServer)) {
                ensureActiveConfigCustomized();
                updateServerSelectDropdown();
                updateServerHeaderUI();
            }
        }

        function commitDomToActiveConfig() {
            const cfg = getActiveConfig();
            cfg.modules = cfg.modules || {};
            cfg.modules.twitter = document.getElementById('mod-twitter').checked;
            cfg.modules.tiktok = document.getElementById('mod-tiktok').checked;
            cfg.modules.instagram = document.getElementById('mod-instagram').checked;
            cfg.modules.reddit = document.getElementById('mod-reddit').checked;
            cfg.modules.bluesky = document.getElementById('mod-bluesky').checked;
            cfg.modules.translation = document.getElementById('mod-translation').checked;
            cfg.modules.ai_chat = document.getElementById('mod-ai_chat').checked;

            cfg.ai = cfg.ai || {};
            cfg.ai.model = document.getElementById('ai-model-box').value.trim() || 'google/gemma-4-31b-it:free';
            cfg.ai.system_prompt = document.getElementById('ai-prompt-box').value.trim();

            commitEngineDomToConfig(cfg);
        }

        // Link motoru alanlarını DOM'dan aktif config'e yazar (değerler güvenli aralığa çekilir).
        function commitEngineDomToConfig(cfg) {
            const engine = getEngineConfig(cfg);

            ENGINE_PLATFORMS.forEach(p => {
                const modeEl = document.getElementById('engine-mode-' + p.key);
                const domainEl = document.getElementById('engine-domain-' + p.key);
                if (modeEl && modeEl.value) engine.modes[p.key] = modeEl.value;
                if (domainEl) engine.domains[p.key] = domainEl.value.trim();
            });

            const sourceEl = document.getElementById('engine-twitter-source');
            if (sourceEl) engine.twitter_source = sourceEl.value;

            const isChecked = (id) => {
                const el = document.getElementById(id);
                return el ? el.checked : false;
            };
            const rangedNumber = (id, fallback, min, max) => {
                const el = document.getElementById(id);
                const value = el ? parseInt(el.value, 10) : NaN;
                if (Number.isNaN(value)) return fallback;
                return Math.min(max, Math.max(min, value));
            };

            engine.attach_images = isChecked('engine-attach-images');
            engine.attach_videos = isChecked('engine-attach-videos');
            engine.fallback_to_proxy = isChecked('engine-fallback');
            engine.auto_translate = isChecked('engine-auto-translate');
            engine.impersonate_user = isChecked('engine-impersonate');
            engine.impersonate_skip_replies = isChecked('engine-impersonate-skip-replies');

            const cardStyleEl = document.getElementById('engine-card-style');
            if (cardStyleEl) engine.card_style = cardStyleEl.value;

            ENGINE_EMOJI_FIELDS.forEach(field => {
                const el = document.getElementById('engine-emoji-' + field.key);
                if (el) engine.emojis[field.key] = el.value.trim();
            });
            engine.show_metrics = isChecked('engine-show-metrics');
            engine.show_timestamp = isChecked('engine-show-timestamp');
            engine.show_community_note = isChecked('engine-show-note');
            engine.show_quote = isChecked('engine-show-quote');
            engine.max_images = rangedNumber('engine-max-images', ENGINE_DEFAULTS.max_images, 1, 10);
            engine.max_video_mb = rangedNumber('engine-max-video', ENGINE_DEFAULTS.max_video_mb, 1, 50);
        }

        function onServerChange() {
            commitDomToActiveConfig();
            activeServer = document.getElementById('server-select').value;
            updateServerHeaderUI();
            filterDiscordRolesForActiveServer();
            renderUI();
        }

        function toggleServerCustomSettings() {
            if (activeServer === 'global') return;
            if (isServerCustomized(activeServer)) {
                if (confirm("Bu sunucuya özel ayarları silip Genel/Varsayılan ayarlara dönmek istediğinize emin misiniz?")) {
                    delete appSettings.guilds[activeServer];
                    updateServerSelectDropdown();
                    updateServerHeaderUI();
                    renderUI();
                    showToast("Sunucu ayarları genel varsayılana sıfırlandı!");
                }
            } else {
                ensureActiveConfigCustomized();
                updateServerSelectDropdown();
                updateServerHeaderUI();
                renderUI();
                showToast("Bu sunucu için özel ayarlar oluşturuldu!");
            }
        }

        function updateServerHeaderUI() {
            const badge = document.getElementById('server-custom-badge');
            const btnToggle = document.getElementById('btn-toggle-custom');
            const controls = document.getElementById('server-custom-controls');
            if (activeServer === 'global') {
                if(badge) badge.textContent = "GENEL VARSAYILAN";
                if(controls) controls.style.display = "none";
            } else {
                if(controls) controls.style.display = "flex";
                if (isServerCustomized(activeServer)) {
                    if(badge) badge.textContent = "ÖZEL AYAR AKTİF";
                    if(btnToggle) btnToggle.textContent = "🔄 Varsayılana Sıfırla";
                } else {
                    if(badge) badge.textContent = "MİRAS ALIYOR (GENEL)";
                    if(btnToggle) btnToggle.textContent = "✨ Bu Sunucuya Özel Ayarla";
                }
            }
        }

        function updateServerSelectDropdown() {
            const select = document.getElementById('server-select');
            if (!select) return;
            const prevVal = activeServer;
            select.innerHTML = '<option value="global">🌐 Genel / Varsayılan Ayarlar</option>';
            rawGuildsList.forEach(g => {
                const opt = document.createElement('option');
                opt.value = g.guild_id;
                opt.textContent = `🏠 ${g.guild_name} ${isServerCustomized(g.guild_id) ? '(Özel)' : ''}`;
                select.appendChild(opt);
            });
            select.value = prevVal;
        }

        function filterDiscordRolesForActiveServer() {
            discordRoles = [];
            rawGuildsList.forEach(g => {
                if (activeServer === 'global' || g.guild_id === activeServer) {
                    g.roles.forEach(r => discordRoles.push({...r, guild_name: g.guild_name}));
                }
            });
            renderRolePickerList();
            renderRoleChips();
        }

        async function fetchSettings() {
            try {
                const res = await fetch('/api/settings');
                const data = await res.json();
                appSettings.modules = data.modules || appSettings.modules;
                appSettings.ai = data.ai || appSettings.ai;
                appSettings.role_filters = data.role_filters || appSettings.role_filters;
                appSettings.engine = data.engine || appSettings.engine;
                appSettings.guilds = data.guilds || {};
                updateServerSelectDropdown();
                renderUI();
            } catch (err) { console.error(err); }
        }

        async function fetchDiscordRoles() {
            try {
                const res = await fetch('/api/discord/roles');
                if (res.ok) {
                    rawGuildsList = await res.json();
                    updateServerSelectDropdown();
                    filterDiscordRolesForActiveServer();
                }
            } catch (err) { console.warn(err); }
        }

        function renderUI() {
            const cfg = getActiveConfig();
            document.getElementById('mod-twitter').checked = !!cfg.modules.twitter;
            document.getElementById('mod-tiktok').checked = !!cfg.modules.tiktok;
            document.getElementById('mod-instagram').checked = !!cfg.modules.instagram;
            document.getElementById('mod-reddit').checked = !!cfg.modules.reddit;
            document.getElementById('mod-bluesky').checked = !!cfg.modules.bluesky;
            document.getElementById('mod-translation').checked = !!cfg.modules.translation;
            document.getElementById('mod-ai_chat').checked = !!cfg.modules.ai_chat;
            document.getElementById('ai-model-box').value = cfg.ai.model;
            document.getElementById('ai-prompt-box').value = cfg.ai.system_prompt;
            renderEngineUI(cfg);
            updateTileMeta();
            updateCharCount();
            selectRoleModule(currentModule, false);
        }

        // Platform motor satırlarını (mod + yedek alan adı) yeniden çizer.
        function renderEngineUI(cfg) {
            const container = document.getElementById('engine-rows');
            if (!container) return;
            const engine = getEngineConfig(cfg);

            container.innerHTML = ENGINE_PLATFORMS.map(p => `
                <div class="engine-row">
                    <div class="engine-platform">
                        <span class="module-icon" style="width: 32px; height: 32px; font-size: 15px;">${p.icon}</span>
                        <div>
                            <div class="name">${p.name}</div>
                            <div class="engine-note" id="engine-note-${p.key}">-</div>
                        </div>
                    </div>
                    <select class="text-input" id="engine-mode-${p.key}" onchange="onConfigModified(); updateEngineNote('${p.key}'); updateEngineSummary()" style="cursor: pointer;">
                        <option value="proxy">Proxy</option>
                        <option value="builtin">Native Embed</option>
                    </select>
                    <input type="text" class="text-input mono" id="engine-domain-${p.key}" placeholder="yedek alan adı (boş = .env)" oninput="onConfigModified()">
                </div>
            `).join('');

            ENGINE_PLATFORMS.forEach(p => {
                const modeEl = document.getElementById('engine-mode-' + p.key);
                const domainEl = document.getElementById('engine-domain-' + p.key);
                if (modeEl) modeEl.value = engine.modes[p.key] || 'proxy';
                if (domainEl) domainEl.value = engine.domains[p.key] || '';
                updateEngineNote(p.key);
            });

            renderEngineEmojiInputs(engine);

            const sourceEl = document.getElementById('engine-twitter-source');
            if (sourceEl) sourceEl.value = engine.twitter_source;

            const cardStyleEl = document.getElementById('engine-card-style');

            const setChecked = (id, value) => {
                const el = document.getElementById(id);
                if (el) el.checked = !!value;
            };
            setChecked('engine-attach-images', engine.attach_images);
            setChecked('engine-attach-videos', engine.attach_videos);
            setChecked('engine-fallback', engine.fallback_to_proxy);
            setChecked('engine-auto-translate', engine.auto_translate);
            setChecked('engine-impersonate', engine.impersonate_user);
            setChecked('engine-impersonate-skip-replies', engine.impersonate_skip_replies);

            if (cardStyleEl) cardStyleEl.value = engine.card_style || 'modern';
            setChecked('engine-show-metrics', engine.show_metrics);
            setChecked('engine-show-timestamp', engine.show_timestamp);
            setChecked('engine-show-note', engine.show_community_note);
            setChecked('engine-show-quote', engine.show_quote);

            const imagesEl = document.getElementById('engine-max-images');
            const videoEl = document.getElementById('engine-max-video');
            if (imagesEl) imagesEl.value = engine.max_images;
            if (videoEl) videoEl.value = engine.max_video_mb;

            updateEngineSummary();
        }

        // Modern kart emoji alanlarını (bir kez) oluşturur ve mevcut değerleri yazar.
        function renderEngineEmojiInputs(engine) {
            const container = document.getElementById('engine-emoji-grid');
            if (!container) return;
            const emojis = engine.emojis || ENGINE_DEFAULTS.emojis;

            container.innerHTML = ENGINE_EMOJI_FIELDS.map(field => `
                <div class="ai-field-group">
                    <div class="ai-label"><span>${field.label}</span></div>
                    <input type="text" class="text-input mono" id="engine-emoji-${field.key}"
                           placeholder="${ENGINE_DEFAULTS.emojis[field.key]}" oninput="onConfigModified()">
                </div>
            `).join('');

            ENGINE_EMOJI_FIELDS.forEach(field => {
                const el = document.getElementById('engine-emoji-' + field.key);
                if (el) el.value = emojis[field.key] || '';
            });
        }

        function updateEngineNote(platformKey) {
            const modeEl = document.getElementById('engine-mode-' + platformKey);
            const noteEl = document.getElementById('engine-note-' + platformKey);
            if (!modeEl || !noteEl) return;
            const builtin = modeEl.value === 'builtin';
            noteEl.textContent = builtin ? 'NATIVE EMBED' : 'PROXY';
            noteEl.classList.toggle('on', builtin);
            noteEl.classList.toggle('off', !builtin);
        }

        function toggleEngineOption(id) {
            const el = document.getElementById(id);
            if (!el) return;
            el.checked = !el.checked;
            onConfigModified();
            updateEngineSummary();
        }

        function updateEngineSummary() {
            const cfg = getActiveConfig();
            const engine = getEngineConfig(cfg);
            const modes = engine.modes || ENGINE_DEFAULTS.modes;
            const builtinCount = ENGINE_PLATFORMS.filter(p => modes[p.key] === 'builtin').length;

            const imagesEl = document.getElementById('engine-max-images');
            const videoEl = document.getElementById('engine-max-video');
            const imagesLabel = document.getElementById('engine-max-images-value');
            const videoLabel = document.getElementById('engine-max-video-value');
            if (imagesLabel && imagesEl) imagesLabel.textContent = imagesEl.value + ' adet';
            if (videoLabel && videoEl) videoLabel.textContent = videoEl.value + ' MB';

            const summary = document.getElementById('engine-summary');
            if (summary) {
                const extras = [];
                if (engine.card_style === 'modern') extras.push('V2 KART');
                else if (engine.card_style === 'auto') extras.push('V2 (VİDEO)');
                else extras.push('FXEMBED');
                if (engine.impersonate_user) extras.push('TAKLİT');
                summary.textContent = builtinCount + '/' + ENGINE_PLATFORMS.length + ' NATIVE EMBED'
                    + (extras.length ? ' • ' + extras.join(' • ') : '');
            }
        }

        function updateTileMeta() {
            const list = ['twitter', 'tiktok', 'instagram', 'reddit', 'bluesky', 'translation', 'ai_chat'];
            let activeCount = 0;
            list.forEach(id => {
                const el = document.getElementById('mod-' + id);
                const isChecked = el ? el.checked : false;
                const label = document.getElementById('status-' + id);
                if (isChecked) {
                    activeCount++;
                    if (label) { label.textContent = "AKTİF"; label.style.color = "var(--emerald)"; }
                } else if (label) { label.textContent = "DEVRE DIŞI"; label.style.color = "var(--text-muted)"; }
            });
            const countEl = document.getElementById('active-modules-count');
            if (countEl) countEl.textContent = `${activeCount}/7 AKTİF`;
        }

        function toggleSwitch(id) {
            const el = document.getElementById(id);
            if (el) { el.checked = !el.checked; onConfigModified(); updateTileMeta(); }
        }

        function selectRoleModule(modKey) {
            currentModule = modKey;
            ['twitter', 'tiktok', 'instagram', 'reddit', 'bluesky', 'translation', 'ai_chat'].forEach(m => {
                const btn = document.getElementById('modtab-' + m);
                if (btn) btn.classList.toggle('active', m === modKey);
            });
            const filter = getCurFilter();
            setFilterModeUI(filter.mode);
            renderRoleChips();
            updateModuleBadges();
            renderRolePickerList();
        }

        function getCurFilter() {
            const cfg = getActiveConfig();
            if (!cfg.role_filters) cfg.role_filters = {};
            if (!cfg.role_filters[currentModule]) cfg.role_filters[currentModule] = { mode: "disabled", roles: [] };
            return cfg.role_filters[currentModule];
        }

        function setFilterMode(mode) {
            onConfigModified();
            getCurFilter().mode = mode;
            setFilterModeUI(mode);
            updateModuleBadges();
        }

        function setFilterModeUI(mode) {
            ['disabled', 'blacklist', 'allowlist'].forEach(m => {
                const btn = document.getElementById('tab-' + m);
                if (btn) btn.classList.toggle('active', m === mode);
            });
            const badge = document.getElementById('filter-mode-badge');
            if (badge) {
                badge.textContent = mode === 'blacklist' ? "KARA LİSTE" : (mode === 'allowlist' ? "BEYAZ LİSTE" : "DEVRE DIŞI");
            }
        }

        function updateModuleBadges() {
            const filters = getActiveConfig().role_filters || {};
            ['twitter', 'tiktok', 'instagram', 'reddit', 'bluesky', 'translation', 'ai_chat'].forEach(m => {
                const badge = document.getElementById('badge-' + m);
                if (!badge) return;
                const f = filters[m];
                badge.textContent = (!f || f.mode === 'disabled') ? "KAPALI" : (f.mode === 'blacklist' ? `⛔ ${f.roles.length}` : `✅ ${f.roles.length}`);
            });
        }

        function renderRoleChips() {
            const container = document.getElementById('chips-container');
            container.innerHTML = '';
            const filter = getCurFilter();

            if (!filter.roles || filter.roles.length === 0) {
                const hint = document.createElement('span');
                hint.className = 'empty-chips-hint';
                hint.textContent = "Bu modül için eklenmiş bir rol yok.";
                container.appendChild(hint);
                return;
            }

            filter.roles.forEach((roleId, index) => {
                const chip = document.createElement('div');
                chip.className = 'role-chip';

                const matched = discordRoles.find(r => r.id === roleId);

                if (matched) {
                    const dot = document.createElement('span');
                    dot.className = 'role-color-dot';
                    dot.style.backgroundColor = matched.color;
                    chip.appendChild(dot);

                    const nameLabel = document.createElement('span');
                    nameLabel.textContent = '@' + matched.name;
                    nameLabel.style.fontWeight = '600';
                    chip.appendChild(nameLabel);

                    const idSub = document.createElement('span');
                    idSub.textContent = `(${roleId})`;
                    idSub.style.fontSize = '10px';
                    idSub.style.color = 'var(--text-muted)';
                    chip.appendChild(idSub);
                } else {
                    const label = document.createElement('span');
                    label.textContent = '#' + roleId;
                    chip.appendChild(label);
                }

                const removeBtn = document.createElement('span');
                removeBtn.className = 'role-chip-remove';
                removeBtn.textContent = '×';
                removeBtn.onclick = () => removeRole(index);

                chip.appendChild(removeBtn);
                container.appendChild(chip);
            });
        }

        function toggleRole(roleId) {
            onConfigModified();
            const filter = getCurFilter();
            const idx = filter.roles.indexOf(roleId);
            if (idx > -1) {
                filter.roles.splice(idx, 1);
            } else {
                filter.roles.push(roleId);
            }
            renderRoleChips();
            renderRolePickerList();
            updateModuleBadges();
        }

        function filterRolePickerList() {
            const q = (document.getElementById('role-search-input').value || '').toLowerCase().trim();
            renderRolePickerList(q);
        }

        function renderRolePickerList(query = '') {
            const list = document.getElementById('role-picker-list');
            list.innerHTML = '';

            if (discordRoles.length === 0) {
                list.innerHTML = '<div style="padding: 16px; text-align: center; color: var(--text-muted); font-size: 12px;">Bu sunucu için rol bulunamadı.</div>';
                return;
            }

            const filter = getCurFilter();
            const filtered = discordRoles.filter(r => r.name.toLowerCase().includes(query) || r.id.includes(query));

            if (filtered.length === 0) {
                list.innerHTML = '<div style="padding: 16px; text-align: center; color: var(--text-muted); font-size: 12px;">Eşleşen rol bulunamadı.</div>';
                return;
            }

            filtered.forEach(role => {
                const item = document.createElement('div');
                item.className = 'role-picker-item';
                item.onclick = (e) => {
                    e.stopPropagation();
                    toggleRole(role.id);
                };

                const left = document.createElement('div');
                left.className = 'role-item-left';

                const dot = document.createElement('div');
                dot.className = 'role-color-dot';
                dot.style.backgroundColor = role.color;

                const nameWrap = document.createElement('div');
                nameWrap.style.display = 'flex';
                nameWrap.style.flexDirection = 'column';

                const nameText = document.createElement('span');
                nameText.className = 'role-name-text';
                nameText.textContent = '@' + role.name;

                const idBadge = document.createElement('span');
                idBadge.className = 'role-id-badge';
                idBadge.textContent = role.id;

                nameWrap.appendChild(nameText);
                nameWrap.appendChild(idBadge);

                left.appendChild(dot);
                left.appendChild(nameWrap);
                item.appendChild(left);

                if (filter.roles.includes(role.id)) {
                    const check = document.createElement('span');
                    check.className = 'role-check-icon';
                    check.textContent = '✓ Seçili';
                    item.appendChild(check);
                }

                list.appendChild(item);
            });
        }

        function toggleRolePicker(e) {
            if (e) e.stopPropagation();
            const dropdown = document.getElementById('role-picker-dropdown');
            dropdown.classList.toggle('open');
            if (dropdown.classList.contains('open')) {
                const search = document.getElementById('role-search-input');
                search.value = '';
                renderRolePickerList('');
                setTimeout(() => search.focus(), 50);
            }
        }

        window.addEventListener('click', (e) => {
            const dropdown = document.getElementById('role-picker-dropdown');
            if (dropdown && dropdown.classList.contains('open')) {
                if (!dropdown.contains(e.target) && !e.target.closest('#btn-toggle-picker')) {
                    dropdown.classList.remove('open');
                }
            }
        });

        function handleRoleInputKey(e) {
            if (e.key === 'Enter') {
                e.preventDefault();
                addRoleFromInput();
            }
        }

        function addRoleFromInput() { 
            const input = document.getElementById('role-input-box'); 
            const v = input.value.trim(); 
            if(v) { 
                const ids = v.split(/[\s,]+/).filter(x => x.length > 0);
                const filter = getCurFilter();
                ids.forEach(id => {
                    if (/^\d+$/.test(id) && !filter.roles.includes(id)) {
                        onConfigModified();
                        filter.roles.push(id);
                    }
                });
                input.value = ''; 
                renderRoleChips(); 
                renderRolePickerList();
                updateModuleBadges(); 
            }
        }

        function removeRole(i) { 
            onConfigModified(); 
            getCurFilter().roles.splice(i, 1); 
            renderRoleChips(); 
            renderRolePickerList();
            updateModuleBadges(); 
        }

        function setModelPreset(m) { 
            onConfigModified(); 
            document.getElementById('ai-model-box').value = m; 
        }

        function updateCharCount() { 
            document.getElementById('char-counter').textContent = document.getElementById('ai-prompt-box').value.length + " karakter"; 
        }

        async function saveSettingsToServer() {
            commitDomToActiveConfig();

            try {
                const res = await fetch('/api/settings', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify(appSettings)
                });
                if (res.ok) {
                    showToast("Tüm ayarlar başarıyla güncellendi!");
                    updateServerSelectDropdown();
                    updateServerHeaderUI();
                } else {
                    alert("Ayarlar kaydedilirken hata meydana geldi.");
                }
            } catch (err) {
                alert("Sunucuya bağlanılamadı: " + err);
            }
        }

        function showToast(msg) {
            const toast = document.getElementById('toast-box');
            document.getElementById('toast-msg').textContent = msg;
            toast.classList.add('visible');
            setTimeout(() => {
                toast.classList.remove('visible');
            }, 3200);
        }

        window.addEventListener('keydown', (e) => {
            if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
                e.preventDefault();
                saveSettingsToServer();
            }
        });

        window.onload = () => {
            fetchSettings();
            fetchDiscordRoles();
        };
    </script>
</body>
</html>
"#;

