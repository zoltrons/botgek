use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::get,
    Json, Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

use crate::settings::AppSettings;

pub type SharedSettings = Arc<RwLock<AppSettings>>;

pub async fn start_web_server(settings: SharedSettings, port: u16) {
    let app = Router::new()
        .route("/", get(index_handler))
        .route("/api/settings", get(get_settings).post(save_settings))
        .route("/api/health", get(health_check))
        .with_state(settings);

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

async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

async fn get_settings(State(settings): State<SharedSettings>) -> impl IntoResponse {
    let s = settings.read().await;
    Json((*s).clone())
}

async fn save_settings(
    State(settings): State<SharedSettings>,
    Json(new_settings): Json<AppSettings>,
) -> impl IntoResponse {
    if let Err(e) = new_settings.save() {
        error!("Ayarlar kaydedilirken hata: {:?}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "success": false, "error": e.to_string() })),
        );
    }

    let mut s = settings.write().await;
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
                        <input type="checkbox" id="mod-twitter" onchange="updateTileMeta()">
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
                        <input type="checkbox" id="mod-tiktok" onchange="updateTileMeta()">
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
                        <input type="checkbox" id="mod-instagram" onchange="updateTileMeta()">
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
                        <input type="checkbox" id="mod-reddit" onchange="updateTileMeta()">
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
                        <input type="checkbox" id="mod-bluesky" onchange="updateTileMeta()">
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
                        <input type="checkbox" id="mod-translation" onchange="updateTileMeta()">
                        <span class="toggle-track"></span>
                    </label>
                </div>

                <div class="module-tile" onclick="toggleSwitch('mod-ai-chat')">
                    <div class="module-info">
                        <div class="module-icon">💬</div>
                        <div class="module-meta">
                            <div class="name">Sarcastic AI Chat</div>
                            <div class="status-label mono" id="status-ai-chat">AKTİF</div>
                        </div>
                    </div>
                    <label class="toggle-switch" onclick="event.stopPropagation()">
                        <input type="checkbox" id="mod-ai-chat" onchange="updateTileMeta()">
                        <span class="toggle-track"></span>
                    </label>
                </div>
            </div>
        </section>

        <!-- 2. ROL FİLTRESİ STUDIO -->
        <section class="panel">
            <div class="panel-title-bar">
                <div class="panel-heading">
                    <span>🛡️ Rol Tabanlı Allowlist / Blacklist Stüdyosu</span>
                </div>
                <span class="mono" id="filter-mode-badge" style="font-size: 11px; padding: 2px 8px; border-radius: 4px; background: rgba(255,255,255,0.06); color: var(--text-secondary);">DEVRE DIŞI</span>
            </div>
            <p class="panel-sub">Botun link dönüştürme sistemini sunucudaki belirli rollere kısıtlayın veya hariç tutun.</p>

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
                    <input type="text" id="role-input-box" placeholder="Discord Rol ID girin (örn: 123456789012345678) ve Enter'a basın..." onkeydown="handleRoleInputKey(event)">
                    <button class="btn-add-role" onclick="addRoleFromInput()">+ Rol Ekle</button>
                </div>

                <div class="role-chips-wrapper" id="chips-container">
                    <span class="empty-chips-hint">Henüz eklenmiş bir rol ID'si yok. Yukarıdaki kutudan ekleyebilirsiniz.</span>
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
                <input type="text" class="text-input mono" id="ai-model-box" placeholder="google/gemma-4-31b-it:free">
                <div class="model-presets">
                    <span class="preset-badge" onclick="setModelPreset('google/gemma-4-31b-it:free')">⚡ Gemma 4 31B (Önerilen)</span>
                    <span class="preset-badge" onclick="setModelPreset('google/gemma-4-26b-a4b-it:free')">Gemma 4 26B</span>
                    <span class="preset-badge" onclick="setModelPreset('nvidia/nemotron-3.5-lightning:free')">Nemotron 3.5</span>
                    <span class="preset-badge" onclick="setModelPreset('openrouter/free')">Auto Free Router</span>
                </div>
            </div>

            <div class="ai-field-group">
                <div class="ai-label">
                    <span>Sistem Ön Komutu (Bot Kişiliği)</span>
                    <span class="mono" id="char-counter" style="font-size: 11px; color: var(--text-muted);">0 karakter</span>
                </div>
                <textarea class="textarea-input" id="ai-prompt-box" oninput="updateCharCount()"></textarea>
            </div>
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
        let state = {
            modules: {
                twitter: true,
                tiktok: true,
                instagram: true,
                reddit: true,
                bluesky: true,
                translation: true,
                ai_chat: true,
            },
            role_filter: {
                mode: "disabled",
                roles: []
            },
            ai: {
                model: "google/gemma-4-31b-it:free",
                system_prompt: ""
            }
        };

        async function fetchSettings() {
            try {
                const res = await fetch('/api/settings');
                state = await res.json();
                renderUI();
            } catch (err) {
                console.error("Ayarlar çekilemedi:", err);
            }
        }

        function renderUI() {
            // Modüller
            document.getElementById('mod-twitter').checked = state.modules.twitter;
            document.getElementById('mod-tiktok').checked = state.modules.tiktok;
            document.getElementById('mod-instagram').checked = state.modules.instagram;
            document.getElementById('mod-reddit').checked = state.modules.reddit;
            document.getElementById('mod-bluesky').checked = state.modules.bluesky;
            document.getElementById('mod-translation').checked = state.modules.translation;
            document.getElementById('mod-ai-chat').checked = state.modules.ai_chat;

            updateTileMeta();

            // Rol Filtresi Modu
            setFilterMode(state.role_filter.mode, false);

            // Rol Çipleri
            renderRoleChips();

            // AI
            document.getElementById('ai-model-box').value = state.ai.model;
            document.getElementById('ai-prompt-box').value = state.ai.system_prompt;
            updateCharCount();
        }

        function updateTileMeta() {
            const list = ['twitter', 'tiktok', 'instagram', 'reddit', 'bluesky', 'translation', 'ai-chat'];
            let activeCount = 0;
            list.forEach(id => {
                const checked = document.getElementById('mod-' + id).checked;
                const label = document.getElementById('status-' + id);
                if (checked) {
                    activeCount++;
                    label.textContent = "AKTİF";
                    label.style.color = "var(--emerald)";
                } else {
                    label.textContent = "DEVRE DIŞI";
                    label.style.color = "var(--text-muted)";
                }
            });
            document.getElementById('active-modules-count').textContent = `${activeCount}/7 AKTİF`;
        }

        function toggleSwitch(id) {
            const el = document.getElementById(id);
            el.checked = !el.checked;
            updateTileMeta();
        }

        function setFilterMode(mode, markDirty = true) {
            state.role_filter.mode = mode;
            ['disabled', 'blacklist', 'allowlist'].forEach(m => {
                const btn = document.getElementById('tab-' + m);
                if (m === mode) {
                    btn.classList.add('active');
                } else {
                    btn.classList.remove('active');
                }
            });

            const badge = document.getElementById('filter-mode-badge');
            if (mode === 'blacklist') {
                badge.textContent = "KARA LİSTE AKTİF";
                badge.style.color = "var(--rose)";
                badge.style.background = "rgba(244, 63, 94, 0.12)";
            } else if (mode === 'allowlist') {
                badge.textContent = "BEYAZ LİSTE AKTİF";
                badge.style.color = "var(--emerald)";
                badge.style.background = "rgba(16, 185, 129, 0.12)";
            } else {
                badge.textContent = "DEVRE DIŞI";
                badge.style.color = "var(--text-secondary)";
                badge.style.background = "rgba(255, 255, 255, 0.06)";
            }
        }

        function renderRoleChips() {
            const container = document.getElementById('chips-container');
            container.innerHTML = '';
            if (!state.role_filter.roles || state.role_filter.roles.length === 0) {
                container.innerHTML = '<span class="empty-chips-hint">Henüz eklenmiş bir rol ID\'si yok. Yukarıdaki kutudan ekleyebilirsiniz.</span>';
                return;
            }

            state.role_filter.roles.forEach((roleId, index) => {
                const chip = document.createElement('div');
                chip.className = 'role-chip';
                chip.innerHTML = `<span>#${roleId}</span><span class="role-chip-remove" onclick="removeRole(${index})">×</span>`;
                container.appendChild(chip);
            });
        }

        function handleRoleInputKey(e) {
            if (e.key === 'Enter') {
                e.preventDefault();
                addRoleFromInput();
            }
        }

        function addRoleFromInput() {
            const input = document.getElementById('role-input-box');
            const val = input.value.trim();
            if (val) {
                const ids = val.split(/[\s,]+/).filter(x => x.length > 0);
                ids.forEach(id => {
                    if (!state.role_filter.roles.includes(id)) {
                        state.role_filter.roles.push(id);
                    }
                });
                input.value = '';
                renderRoleChips();
            }
        }

        function removeRole(index) {
            state.role_filter.roles.splice(index, 1);
            renderRoleChips();
        }

        function setModelPreset(modelName) {
            document.getElementById('ai-model-box').value = modelName;
        }

        function updateCharCount() {
            const len = document.getElementById('ai-prompt-box').value.length;
            document.getElementById('char-counter').textContent = `${len} karakter`;
        }

        async function saveSettingsToServer() {
            // State'i formdan topla
            state.modules.twitter = document.getElementById('mod-twitter').checked;
            state.modules.tiktok = document.getElementById('mod-tiktok').checked;
            state.modules.instagram = document.getElementById('mod-instagram').checked;
            state.modules.reddit = document.getElementById('mod-reddit').checked;
            state.modules.bluesky = document.getElementById('mod-bluesky').checked;
            state.modules.translation = document.getElementById('mod-translation').checked;
            state.modules.ai_chat = document.getElementById('mod-ai-chat').checked;

            state.ai.model = document.getElementById('ai-model-box').value.trim() || 'google/gemma-4-31b-it:free';
            state.ai.system_prompt = document.getElementById('ai-prompt-box').value.trim();

            try {
                const res = await fetch('/api/settings', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify(state)
                });
                if (res.ok) {
                    showToast("Ayarlar başarıyla güncellendi!");
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

        // Ctrl + S Kısayolu
        window.addEventListener('keydown', (e) => {
            if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
                e.preventDefault();
                saveSettingsToServer();
            }
        });

        window.onload = fetchSettings;
    </script>
</body>
</html>
"#;

