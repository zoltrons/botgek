//! Bot WebUI adresinde (`/mcp`) yayınlanan MCP sunucusu.
//!
//! Amaç, botun geliştirme/teşhis sürecinde ihtiyaç duyulan her şeye **aynı süreçten**
//! erişmektir: sürüm ve readiness, metrikler, son loglar, ayarlar, Discord cache'i,
//! kart veritabanı ve Discord'a göndermeden link önizleme. Endpoint WebUI ile aynı
//! portta olduğu için SSH tüneli/ters proxy dışında ek bir servis gerekmez.
//!
//! Taşıma: MCP Streamable HTTP (tek `POST` uç noktası, `application/json` yanıt).
//! Hem 2026-07-28 (`server/discover`, oturumsuz) hem de eski `initialize` akışı
//! desteklenir; DSH istemcisi hangi sürümü seçerse çalışır.
//!
//! Kimlik doğrulama: `Authorization: Bearer <MCP_TOKEN>`. Token yoksa endpoint kapalıdır
//! (503); WebUI girişi etkilenmez. Token sabit zamanlı karşılaştırılır ve yanıtlarda
//! asla yankılanmaz.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use subtle::ConstantTimeEq;
use tracing::{info, warn};

use crate::converters::builtin::{self, BuiltinPost};
use crate::converters::parser::{self, PlatformDomains};
use crate::settings::AppSettings;
use crate::store;
use crate::web::WebState;

/// `MCP_TOKEN` için üst sınır (`ADMIN_TOKEN` ile aynı politika).
const MAX_TOKEN_LEN: usize = 512;
/// Modern (oturumsuz) MCP revizyonu.
const MODERN_VERSION: &str = "2026-07-28";
/// Kabul edilen eski revizyonlar (`initialize` akışı).
const LEGACY_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
/// `server/discover` / `tools/*` yanıtlarında dönen önbellek ömrü.
const CACHE_TTL_MS: u64 = 600_000;

const INSTRUCTIONS: &str = "Botgek runtime debug server for the Discord link converter. \
Use bot_status and bot_logs first to orient yourself, bot_metrics for counters, \
settings_get/settings_patch for runtime settings, discord_overview for cached guilds, \
db_stats/db_query for the read-only card store, and convert_preview to fetch and inspect \
a link without posting to Discord. Mutations affect the running bot immediately.";

/// MCP Bearer tokenı. Token yalnızca bu yapıda tutulur; `Debug`/`Serialize` yoktur.
pub struct Auth {
    token: String,
}

impl Auth {
    pub fn new(token: String) -> Result<Arc<Self>, &'static str> {
        if !(32..=MAX_TOKEN_LEN).contains(&token.len()) || token.trim() != token {
            return Err(
                "MCP_TOKEN en az 32, en fazla 512 karakter olmalı ve baş/son boşluk içermemeli",
            );
        }
        Ok(Arc::new(Self { token }))
    }

    /// Uzunluk ve içerikten bağımsız sabit zamanlı karşılaştırma.
    fn verify(&self, candidate: &str) -> bool {
        fn padded(value: &[u8]) -> [u8; MAX_TOKEN_LEN] {
            let mut buffer = [0u8; MAX_TOKEN_LEN];
            let length = value.len().min(MAX_TOKEN_LEN);
            buffer[..length].copy_from_slice(&value[..length]);
            buffer
        }
        let candidate = candidate.as_bytes();
        let same_length = (self.token.len() as u64).ct_eq(&(candidate.len() as u64));
        let same_bytes = padded(self.token.as_bytes()).ct_eq(&padded(candidate));
        bool::from(same_length & same_bytes)
    }
}

/// `MCP_TOKEN` ortam değişkeninden kimlik doğrulamayı kurar.
///
/// Tanımsızsa `None` (endpoint kapalı). Tanımlı ama geçersizse uyarı loglanır ve
/// endpoint yine kapalı kalır: zayıf bir sırla yanlışlıkla açılmaz.
pub fn auth_from_env() -> Option<Arc<Auth>> {
    let raw = std::env::var("MCP_TOKEN").ok()?;
    if raw.trim().is_empty() {
        return None;
    }
    match Auth::new(raw) {
        Ok(auth) => {
            if std::env::var("ADMIN_TOKEN").ok().as_deref() == Some(auth.token.as_str()) {
                warn!(
                    "MCP_TOKEN ADMIN_TOKEN ile aynı: ayrı bir token kullanın (WebUI oturumu ile MCP erişimi ayrı sırlar olmalı)."
                );
            }
            Some(auth)
        }
        Err(reason) => {
            warn!("MCP endpoint disabled: {reason}");
            None
        }
    }
}

/// MCP endpoint'i (POST). Diğer metotlar [`method_not_allowed`] ile reddedilir.
pub async fn handle(State(state): State<WebState>, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(response) = authorize(&state, &headers) {
        return response;
    }
    if !origin_allowed(&headers) {
        return json_error_response(
            StatusCode::FORBIDDEN,
            Value::Null,
            -32600,
            "Origin doğrulanamadı (DNS rebinding koruması)",
        );
    }

    let payload: Value = match serde_json::from_slice(&body) {
        Ok(payload) => payload,
        Err(error) => {
            return json_error_response(
                StatusCode::BAD_REQUEST,
                Value::Null,
                -32700,
                &format!("JSON çözümlenemedi: {error}"),
            );
        }
    };
    let Some(object) = payload.as_object() else {
        return json_error_response(
            StatusCode::BAD_REQUEST,
            Value::Null,
            -32600,
            "Tek bir JSON-RPC nesnesi gönderin",
        );
    };

    let id = object.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return json_error_response(StatusCode::BAD_REQUEST, id, -32600, "method alanı eksik");
    };
    let params = object.get("params").cloned().unwrap_or(Value::Null);

    // Sürüm başlığı ile gövdedeki `_meta` alanı çelişmemeli (2026-07-28 sözleşmesi).
    let version_header = headers
        .get("mcp-protocol-version")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body_version = params
        .pointer("/_meta/io.modelcontextprotocol~1protocolVersion")
        .and_then(Value::as_str)
        .map(str::to_string);
    if let (Some(header_value), Some(body_value)) = (&version_header, &body_version)
        && header_value != body_value
    {
        return json_error_response(
            StatusCode::BAD_REQUEST,
            id,
            -32600,
            "MCP-Protocol-Version başlığı gövdedeki protocolVersion ile uyuşmuyor",
        );
    }
    if let Some(version) = version_header.as_deref()
        && version != MODERN_VERSION
        && !LEGACY_VERSIONS.contains(&version)
    {
        return json_error_response(
            StatusCode::BAD_REQUEST,
            id,
            -32001,
            &format!("Desteklenmeyen MCP sürümü: {version}"),
        );
    }
    let modern = version_header.as_deref() == Some(MODERN_VERSION)
        || body_version.as_deref() == Some(MODERN_VERSION);

    // Bildirimler yanıt taşımaz.
    if method.starts_with("notifications/") || object.get("id").is_none() {
        return StatusCode::ACCEPTED.into_response();
    }

    match method {
        "server/discover" => rpc_result(id, discover_result(), modern),
        "initialize" => rpc_result(id, initialize_result(&params), modern),
        "ping" => rpc_result(id, json!({}), modern),
        "tools/list" => rpc_result(id, tools_list_result(), modern),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let result = call_tool(&state, name, &arguments).await;
            rpc_result(id, tool_result(result), modern)
        }
        "logging/setLevel" => StatusCode::ACCEPTED.into_response(),
        other => json_error_response(
            if modern {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::OK
            },
            id,
            -32601,
            &format!("Bilinmeyen MCP metodu: {other}"),
        ),
    }
}

/// `/mcp` üzerinde POST dışındaki metotlar (GET SSE akışı bu sunucuda yoktur).
pub async fn method_not_allowed(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Some(response) = authorize(&state, &headers) {
        return response;
    }
    let mut response = (
        StatusCode::METHOD_NOT_ALLOWED,
        Json(json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {"code": -32600, "message": "Yalnızca POST desteklenir"}
        })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static("POST"));
    response
}

/// Origin varsa isteğin kendi Host'uyla aynı olmalıdır; yoksa (yerel istemci) kabul.
fn origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return true;
    };
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    origin
        .split_once("://")
        .is_some_and(|(_, authority)| authority.eq_ignore_ascii_case(host))
}

fn authorize(state: &WebState, headers: &HeaderMap) -> Option<Response> {
    let Some(auth) = state.mcp.as_ref() else {
        return Some((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {"code": -32002, "message": "MCP endpoint kapalı: MCP_TOKEN yapılandırılmadı"}
            })),
        )
            .into_response());
    };
    let candidate = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim);
    if candidate.is_some_and(|token| auth.verify(token)) {
        return None;
    }
    Some(
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer realm=\"botgek-mcp\"")],
            Json(json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {"code": -32001, "message": "Yetkisiz: geçerli Bearer token gerekli"}
            })),
        )
            .into_response(),
    )
}

fn rpc_result(id: Value, mut result: Value, modern: bool) -> Response {
    if modern && let Some(object) = result.as_object_mut() {
        object
            .entry("resultType")
            .or_insert_with(|| Value::String("complete".to_string()));
    }
    Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
}

fn json_error_response(status: StatusCode, id: Value, code: i64, message: &str) -> Response {
    (
        status,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": code, "message": message}
        })),
    )
        .into_response()
}

fn discover_result() -> Value {
    json!({
        "supportedVersions": [MODERN_VERSION],
        "capabilities": {"tools": {"listChanged": false}},
        "_meta": {
            "io.modelcontextprotocol/serverInfo": {
                "name": "botgek",
                "version": crate::version::SEMVER,
                "title": "Botgek runtime debug server"
            }
        },
        "instructions": INSTRUCTIONS,
        "ttlMs": CACHE_TTL_MS,
        "cacheScope": "private"
    })
}

fn initialize_result(params: &Value) -> Value {
    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or("2025-06-18");
    let version = if LEGACY_VERSIONS.contains(&requested) {
        requested
    } else {
        "2025-06-18"
    };
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {
            "name": "botgek",
            "version": crate::version::SEMVER,
            "title": "Botgek runtime debug server"
        },
        "instructions": INSTRUCTIONS
    })
}

fn tools_list_result() -> Value {
    json!({"tools": tool_definitions()})
}

fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "bot_status",
            "title": "Bot status",
            "description": "Sürüm, çalışma süresi, Discord readiness, veritabanı/settings durumu ve sır içermeyen yapılandırma özeti.",
            "inputSchema": {"type": "object", "additionalProperties": false}
        }),
        json!({
            "name": "bot_metrics",
            "title": "Bot metrics",
            "description": "Süreç içi telemetri: dönüşüm kuyruğu, platform bazlı başarı/hata ve gecikme kovaları.",
            "inputSchema": {"type": "object", "additionalProperties": false}
        }),
        json!({
            "name": "bot_diagnostics",
            "title": "Detailed diagnostics",
            "description": "Ayrıntılı telemetri: Discord REST rota/status/429/retry, dönüşüm fazları, upstream HTTP hostları, önbellekler, SQLite işlemleri, gateway ve süreç/kaynak metrikleri.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "section": {
                        "type": "string",
                        "enum": ["all", "rest", "pipeline", "http", "cache", "db", "process", "gateway", "conversion"],
                        "default": "all"
                    }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "bot_logs",
            "title": "Recent logs",
            "description": "Bellekteki son tracing loglarını döner (en yeniden eskiye, süreç ömrüyle sınırlı).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "minimum": 1, "maximum": 512, "default": 120},
                    "min_level": {"type": "string", "enum": ["trace", "debug", "info", "warn", "error"], "default": "info"}
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "settings_get",
            "title": "Get settings",
            "description": "Etkin ayar dosyasını ve içerik revizyonunu döner (WebUI ile aynı model).",
            "inputSchema": {"type": "object", "additionalProperties": false}
        }),
        json!({
            "name": "settings_patch",
            "title": "Patch settings",
            "description": "Ayarlara JSON merge patch uygular, aynı normalizasyonla diske yazar ve belleği günceller. Değişiklik botu anında etkiler.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "patch": {"type": "object", "description": "Birleştirilecek JSON nesnesi; null bir alanı siler."},
                    "expected_revision": {"type": "string", "description": "Verilirse mevcut revizyon eşleşmezse yazma reddedilir."}
                },
                "required": ["patch"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "discord_overview",
            "title": "Discord overview",
            "description": "Serenity cache'inde bulunan sunucuları, kanalları ve rolleri döner (Discord'a istek atmaz).",
            "inputSchema": {"type": "object", "additionalProperties": false}
        }),
        json!({
            "name": "db_stats",
            "title": "Database stats",
            "description": "Kart kayıt veritabanının durumu: yol, kayıt sayısı ve dosya boyutu.",
            "inputSchema": {"type": "object", "additionalProperties": false}
        }),
        json!({
            "name": "db_query",
            "title": "Read-only SQL",
            "description": "Kart kayıt veritabanında salt okunur tek bir SELECT/WITH çalıştırır.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "sql": {"type": "string", "description": "Tek bir salt okunur SQLite sorgusu."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                },
                "required": ["sql"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "convert_preview",
            "title": "Preview conversion",
            "description": "Bir bağlantıyı çözümleyip verisini çeker ve render planını döner; Discord'a hiçbir mesaj göndermez.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": {"type": "string", "description": "Dönüştürülecek desteklenen bağlantı."},
                    "spoiler": {"type": "boolean", "default": false, "description": "Kullanıcı ||spoiler|| işaretlemesi gibi davran."},
                    "channel_nsfw": {"type": "boolean", "description": "Kanal NSFW mi (hassas içerik görünürlüğü için)."}
                },
                "required": ["url"],
                "additionalProperties": false
            }
        }),
    ]
}

async fn call_tool(state: &WebState, name: &str, args: &Value) -> Result<Value, String> {
    info!(tool = name, "MCP tool call");
    match name {
        "bot_status" => Ok(tool_bot_status(state).await),
        "bot_metrics" => Ok(crate::telemetry::snapshot()),
        "bot_diagnostics" => {
            let section = args.get("section").and_then(Value::as_str).unwrap_or("all");
            diagnostics_snapshot(section)
        }
        "bot_logs" => {
            let limit = integer_arg(args, "limit").unwrap_or(120) as usize;
            let min_level = args
                .get("min_level")
                .and_then(Value::as_str)
                .unwrap_or("info");
            Ok(json!({
                "min_level": min_level,
                "capacity": crate::logs::CAPACITY,
                "stored": crate::logs::len(),
                "entries": crate::logs::recent(limit, min_level)
            }))
        }
        "settings_get" => {
            let settings = state.settings.read().await;
            Ok(json!({
                "revision": crate::web::settings_revision(&settings),
                "path": state.settings_path.display().to_string(),
                "settings": *settings
            }))
        }
        "settings_patch" => tool_settings_patch(state, args).await,
        "discord_overview" => Ok(discord_overview(state)),
        "db_stats" => Ok(db_stats()),
        "db_query" => {
            let sql = string_arg(args, "sql")?;
            let limit = integer_arg(args, "limit").unwrap_or(50).clamp(1, 200) as usize;
            let result = store::query_readonly_blocking(sql, limit).await?;
            Ok(serde_json::to_value(result).map_err(|error| error.to_string())?)
        }
        "convert_preview" => convert_preview(state, args).await,
        other => Err(format!("Bilinmeyen araç: {other}")),
    }
}

async fn tool_bot_status(state: &WebState) -> Value {
    let settings = state.settings.read().await;
    json!({
        "version": {
            "semver": crate::version::SEMVER,
            "full": crate::version::FULL,
            "display": crate::version::DISPLAY,
            "git_sha": crate::version::GIT_SHA,
            "git_tag": crate::version::tag(),
            "commits_since_tag": crate::version::commits_since_tag(),
            "build_date": crate::version::BUILD_DATE,
            "target": crate::version::TARGET,
            "is_release": crate::version::IS_RELEASE,
            "is_dirty": crate::version::IS_DIRTY
        },
        "uptime_seconds": state.started.elapsed().as_secs(),
        "web": {"port": state.info.port},
        "discord": {
            "ready": crate::telemetry::DISCORD_READY.load(std::sync::atomic::Ordering::Relaxed)
        },
        "database": {
            "healthy": store::healthy(),
            "path": store::path(),
            "records": store::record_count(),
            "bytes": store::file_bytes(),
            "files": store::file_sizes(),
            "max_records": 200_000
        },
        "process": crate::telemetry::process_snapshot(),
        "gateway": crate::telemetry::gateway_snapshot(),
        "settings": {
            "path": state.settings_path.display().to_string(),
            "revision": crate::web::settings_revision(&settings)
        },
        "config": {
            "delete_original": state.config.delete_original,
            "tiktok_domain": state.config.tiktok_domain,
            "bluesky_domain": state.config.bluesky_domain,
            "spotify_domain": state.config.spotify_domain,
            "ai_configured": state.config.openrouter_api_key.is_some(),
            "instagram_sessionid_set": state.config.instagram_sessionid.is_some(),
            "reddit_client_id": state.config.reddit_client_id
        },
        "mcp": {
            "enabled": state.mcp.is_some(),
            "transport": "streamable-http",
            "endpoint": "/mcp"
        }
    })
}

async fn tool_settings_patch(state: &WebState, args: &Value) -> Result<Value, String> {
    let patch = args
        .get("patch")
        .ok_or_else(|| "patch alanı gerekli".to_string())?;
    if !patch.is_object() {
        return Err("patch bir JSON nesnesi olmalı".to_string());
    }
    let expected_revision = args.get("expected_revision").and_then(Value::as_str);

    let _writer = state.save_lock.lock().await;
    let current = state.settings.read().await.clone();
    let current_revision = crate::web::settings_revision(&current);
    if let Some(expected) = expected_revision
        && expected.trim_matches('"') != current_revision.trim_matches('"')
    {
        return Err(format!(
            "revizyon çakışması: beklenen {expected}, güncel {current_revision}"
        ));
    }

    let mut merged = serde_json::to_value(&current).map_err(|error| error.to_string())?;
    merge_json(&mut merged, patch);
    let mut next: AppSettings = serde_json::from_value(merged)
        .map_err(|error| format!("geçersiz ayar gövdesi: {error}"))?;
    // Şema sürümü panelden/MCP'den değiştirilemez; göçü yeniden tetiklememeli.
    next.schema_version = current.schema_version;
    next.sanitize_for_save();

    let path = state.settings_path.clone();
    let persisted = next.clone();
    tokio::task::spawn_blocking(move || persisted.save_to(&path))
        .await
        .map_err(|error| format!("ayar yazma görevi düştü: {error}"))?
        .map_err(|error| format!("ayarlar yazılamadı: {error}"))?;

    let revision = crate::web::settings_revision(&next);
    *state.settings.write().await = next.clone();
    Ok(json!({
        "revision": revision,
        "path": state.settings_path.display().to_string(),
        "settings": next
    }))
}

fn discord_overview(state: &WebState) -> Value {
    let mut guilds = Vec::new();
    for guild_id in state.cache.guilds() {
        let Some(guild) = state.cache.guild(guild_id) else {
            guilds.push(json!({"id": guild_id.to_string(), "cached": false}));
            continue;
        };
        let mut channels: Vec<Value> = guild
            .channels
            .values()
            .map(|channel| {
                json!({
                    "id": channel.id.to_string(),
                    "name": channel.name,
                    "kind": format!("{:?}", channel.kind),
                    "parent_id": channel.parent_id.map(|id| id.to_string()),
                    "nsfw": channel.nsfw
                })
            })
            .collect();
        channels.extend(guild.threads.iter().map(|thread| {
            json!({
                "id": thread.id.to_string(),
                "name": thread.name,
                "kind": format!("{:?}", thread.kind),
                "parent_id": thread.parent_id.map(|id| id.to_string()),
                "nsfw": thread.nsfw
            })
        }));
        channels.sort_by(|left, right| {
            left["name"]
                .as_str()
                .cmp(&right["name"].as_str())
                .then(left["id"].as_str().cmp(&right["id"].as_str()))
        });
        let mut roles: Vec<Value> = guild
            .roles
            .values()
            .map(|role| {
                json!({
                    "id": role.id.to_string(),
                    "name": role.name,
                    "position": role.position,
                    "color": format!("#{:06x}", role.colour.0),
                    "managed": role.managed
                })
            })
            .collect();
        roles.sort_by_key(|role| std::cmp::Reverse(role["position"].as_u64().unwrap_or(0)));
        guilds.push(json!({
            "id": guild.id.to_string(),
            "name": guild.name,
            "owner_id": guild.owner_id.to_string(),
            "member_count": guild.member_count,
            "cached_members": guild.members.len(),
            "channels": channels,
            "roles": roles
        }));
    }
    guilds.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    json!({"guilds": guilds, "guild_count": guilds.len()})
}

fn db_stats() -> Value {
    json!({
        "healthy": store::healthy(),
        "path": store::path(),
        "records": store::record_count(),
        "bytes": store::file_bytes(),
        "files": store::file_sizes(),
        "max_records": 200_000,
        "note": "SQLite WAL; kayıtlar mesaj → paylaşan kullanıcı eşlemesidir."
    })
}

/// `bot_diagnostics` bölüm filtresi. Tam anlık görüntü her çağrıda yeniden üretilir;
/// çağrı sıklığı düşük olduğu için bu maliyet kabul edilebilir.
fn diagnostics_snapshot(section: &str) -> Result<Value, String> {
    let full = crate::telemetry::detailed_snapshot();
    let selected = match section {
        "all" => return Ok(full),
        "rest" => json!({"rest": full["rest"]}),
        "pipeline" | "phases" => json!({"phases": full["phases"]}),
        "http" => json!({"http": full["http"]}),
        "cache" | "caches" => json!({"caches": full["caches"]}),
        "db" => json!({"db": full["db"]}),
        "process" => json!({"process": full["process"]}),
        "gateway" => json!({"gateway": full["gateway"]}),
        "conversion" => json!({
            "cache_hits": full["cache_hits"],
            "overloads": full["overloads"],
            "conversion_queue": full["conversion_queue"],
            "native_conversions": full["native_conversions"]
        }),
        other => {
            return Err(format!(
                "bilinmeyen bölüm: {other}; all/rest/pipeline/http/cache/db/process/gateway/conversion"
            ));
        }
    };
    Ok(selected)
}

async fn convert_preview(state: &WebState, args: &Value) -> Result<Value, String> {
    let url = string_arg(args, "url")?;
    let spoiler = args
        .get("spoiler")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let channel_nsfw = args.get("channel_nsfw").and_then(Value::as_bool);
    let _preview = crate::telemetry::PhaseTimer::new(crate::telemetry::Phase::Preview);

    let settings = state.settings.read().await.clone();
    let engine = settings.get_guild_settings(None).engine;
    let domains = PlatformDomains {
        tiktok: engine.domain_for("tiktok", &state.config),
        bluesky: engine.domain_for("bluesky", &state.config),
        spotify: engine.domain_for("spotify", &state.config),
    };
    let parsed = parser::parse_message_with_domains(&url, &domains)
        .ok_or_else(|| "desteklenen bir bağlantı bulunamadı (X, TikTok, Instagram, Reddit, Bluesky, Spotify, Twitch, Pixiv, Tumblr, Facebook, YouTube)".to_string())?;

    let started = Instant::now();
    let fetched = tokio::time::timeout(
        Duration::from_secs(20),
        builtin::fetch(&state.client, &parsed, &engine, &state.config),
    )
    .await
    .map_err(|_| "veri çekme 20 saniyede tamamlanmadı".to_string())?
    .map_err(|error| format!("veri alınamadı: {error}"))?;
    let fetch_ms = started.elapsed().as_millis();

    let options = builtin::render::RenderOptions::from_engine(
        &engine,
        builtin::platform_key(parsed.platform),
    );
    let visible =
        builtin::render::apply_visibility(&fetched, spoiler, engine.sensitive_mode, channel_nsfw);
    let plan = builtin::render::plan_media(&visible, &options);

    Ok(json!({
        "parsed": {
            "platform": builtin::platform_key(parsed.platform),
            "original_url": parsed.original_url,
            "fixed_url": parsed.fixed_url,
            "is_spoiler": parsed.is_spoiler,
            "user_comment": parsed.user_comment
        },
        "fetch_ms": fetch_ms,
        "render": {
            "video_requires_download": options.video_requires_download,
            "max_images": options.max_images,
            "max_video_mb": options.max_video_mb,
            "plan": {
                "remote_images": plan.remote_images.len(),
                "gifs": plan.gifs.len(),
                "video": plan.video.is_some()
            }
        },
        "effective_hidden": visible.sensitive,
        "post": post_json(&visible, 0),
        "note": "Discord'a hiçbir mesaj gönderilmedi; yalnızca veri çekme ve render planı."
    }))
}

/// `BuiltinPost` için JSON görünümü (kaynak modelde serde türevi yok).
fn post_json(post: &BuiltinPost, depth: usize) -> Value {
    let media: Vec<Value> = post
        .media
        .iter()
        .map(|media| {
            json!({
                "kind": match media.kind {
                    builtin::MediaKind::Image => "image",
                    builtin::MediaKind::Video => "video",
                    builtin::MediaKind::Gif => "gif",
                },
                "url": media.url,
                "poster": media.poster,
                "width": media.width,
                "height": media.height,
                "duration": media.duration
            })
        })
        .collect();
    json!({
        "platform": builtin::platform_key(post.platform),
        "url": post.url,
        "author": {
            "name": post.author.name,
            "handle": post.author.handle,
            "verified": post.author.verified,
            "avatar": post.author.avatar,
            "profile_url": post.author.profile_url
        },
        "title": post.title,
        "text": post.text,
        "created_at": post.created_at,
        "stats": {
            "replies": post.stats.replies,
            "reposts": post.stats.reposts,
            "likes": post.stats.likes,
            "quotes": post.stats.quotes,
            "views": post.stats.views
        },
        "media": media,
        "quote": post.quote.as_deref().filter(|_| depth < 2).map(|quote| post_json(quote, depth + 1)),
        "community_note": post.community_note,
        "source_spoiler": post.source_spoiler,
        "sensitive": post.sensitive,
        "extra_label": post.extra_label,
        "language": post.language,
        "translation": post.translation.as_ref().map(|translation| json!({"source": translation.source})),
        "custom_card": post.custom_card.as_ref().map(|card| json!({
            "component_count": card.components.len(),
            "accent_color": card.accent_color
        })),
        "link_preview": post.link_preview.as_ref().map(|preview| json!({
            "url": preview.url,
            "title": preview.title,
            "description": preview.description,
            "image": preview.image,
            "site_name": preview.site_name
        }))
    })
}

fn tool_result(result: Result<Value, String>) -> Value {
    match result {
        Ok(payload) => {
            let text =
                serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
            json!({
                "content": [{"type": "text", "text": text}],
                "isError": false
            })
        }
        Err(message) => json!({
            "content": [{"type": "text", "text": message}],
            "isError": true
        }),
    }
}

fn string_arg(args: &Value, name: &str) -> Result<String, String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{name} alanı gerekli"))
}

fn integer_arg(args: &Value, name: &str) -> Option<u64> {
    args.get(name).and_then(Value::as_u64)
}

/// RFC 7386 benzeri JSON merge patch: nesneler özyinelemeli birleşir, `null` anahtarı siler.
fn merge_json(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(base_map), Value::Object(patch_map)) => {
            for (key, value) in patch_map {
                if value.is_null() {
                    base_map.remove(key);
                } else {
                    merge_json(base_map.entry(key.clone()).or_insert(Value::Null), value);
                }
            }
        }
        (base, patch) => *base = patch.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_policy_requires_a_strong_trimmed_secret() {
        assert!(Auth::new("a".repeat(31)).is_err());
        assert!(Auth::new(format!(" {} ", "a".repeat(32))).is_err());
        assert!(Auth::new("a".repeat(32)).is_ok());
        assert!(Auth::new("a".repeat(MAX_TOKEN_LEN + 1)).is_err());
    }

    #[test]
    fn token_comparison_is_exact() {
        let token = "a".repeat(32);
        let auth = Auth::new(token.clone()).expect("geçerli token");
        assert!(auth.verify(&token));
        assert!(!auth.verify(&"a".repeat(31)));
        assert!(!auth.verify(&format!("{}b", "a".repeat(31))));
        assert!(!auth.verify(""));
    }

    #[test]
    fn merge_patch_recurses_and_deletes_with_null() {
        let mut base = json!({"a": 1, "nested": {"x": 1, "y": 2}});
        merge_json(&mut base, &json!({"a": 3, "nested": {"y": null, "z": 4}}));
        assert_eq!(base, json!({"a": 3, "nested": {"x": 1, "z": 4}}));
    }

    #[test]
    fn protocol_version_shapes_are_supported() {
        let discover = discover_result();
        assert_eq!(discover["supportedVersions"][0], MODERN_VERSION);
        assert_eq!(
            discover["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            "botgek"
        );
        let legacy = initialize_result(&json!({"protocolVersion": "2024-11-05"}));
        assert_eq!(legacy["protocolVersion"], "2024-11-05");
        let unknown = initialize_result(&json!({"protocolVersion": "gelecek"}));
        assert_eq!(unknown["protocolVersion"], "2025-06-18");
    }

    #[test]
    fn tool_definitions_are_unique_with_object_schemas() {
        let tools = tool_definitions();
        let mut names = std::collections::BTreeSet::new();
        for tool in &tools {
            let name = tool["name"].as_str().expect("araç adı");
            assert!(names.insert(name.to_string()), "yinelenen araç: {name}");
            assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
            assert!(
                !tool["description"].as_str().unwrap_or("").is_empty(),
                "{name} açıklaması boş"
            );
        }
        assert!(names.contains("convert_preview"));
        assert!(names.contains("db_query"));
        assert!(names.contains("bot_diagnostics"));
        assert!(diagnostics_snapshot("rest").unwrap()["rest"].is_object());
        assert!(diagnostics_snapshot("process").unwrap()["process"].is_object());
        assert!(diagnostics_snapshot("bilinmeyen").is_err());
    }
}
