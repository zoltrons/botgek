mod ai;
mod config;
mod converters;
mod handler;
mod settings;
mod utils;
mod web;

use std::sync::Arc;
use tokio::sync::RwLock;

use config::Config;
use handler::Handler;
use serenity::all::{Client, GatewayIntents};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    // .env dosyasını yükle
    dotenvy::dotenv().ok();

    // Loglama sistemini başlat
    let default_filter = "botgek=info,serenity=warn";
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_filter));

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .init();

    info!("Botgek başlatılıyor...");

    // Konfigürasyonu oku
    let config = match Config::from_env() {
        Ok(c) => c,
        Err(err) => {
            error!("{}", err);
            std::process::exit(1);
        }
    };

    // Dinamik Ayarları Yükle (settings.json)
    let settings = Arc::new(RwLock::new(settings::AppSettings::load()));

    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    // Bot Gateway Intent'leri
    // NOT: Discord Developer Portal'dan "MESSAGE CONTENT INTENT" mutlaka aktif edilmelidir!
    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::GUILD_MESSAGE_REACTIONS
        | GatewayIntents::DIRECT_MESSAGES;

    let handler = Handler {
        config: config.clone(),
        settings: settings.clone(),
        http_client,
    };

    let mut client = match Client::builder(&config.discord_token, intents)
        .event_handler(handler)
        .await
    {
        Ok(c) => c,
        Err(err) => {
            error!("Serenity Client oluşturulamadı: {:?}", err);
            std::process::exit(1);
        }
    };

    // Minimal WebUI Sunucusunu Başlat (Arka planda - Discord Http ve Cache ile)
    let web_settings = settings.clone();
    let web_port = config.web_port;
    let web_http = client.http.clone();
    let web_cache = client.cache.clone();
    tokio::spawn(async move {
        web::start_web_server(web_settings, web_port, web_http, web_cache).await;
    });


    // Güvenli durdurma sinyali dinleyicisi (CTRL+C ve Unix SIGTERM)
    let shard_manager = client.shard_manager.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM dinleyicisi başlatılamadı");
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = sigterm.recv() => {},
            }
        }
        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c()
                .await
                .expect("Sinyal dinleyicisi başlatılamadı");
        }
        info!("Kapatma sinyali alındı. Shard yöneticisi durduruluyor...");
        shard_manager.shutdown_all().await;
    });

    if let Err(err) = client.start().await {
        error!("Bot çalışırken kritik hata oluştu: {:?}", err);
    }
}
