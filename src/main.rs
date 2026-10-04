mod ai;
mod config;
mod converters;
mod handler;
mod settings;
mod store;
mod utils;
mod web;

use std::sync::Arc;
use tokio::sync::RwLock;

use config::Config;
use handler::Handler;
use serenity::all::{Client, GatewayIntents};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

/// İlk dönüşümde DNS çözümlemesi + TLS el sıkışması beklenmesin diye ilgili uçlara
/// **arka planda** ısıtma isteği yapar. Bot her açılışta bir kez çalışır; ilk kartın
/// ~200-400 ms daha erken gelmesini sağlar (yanıt içeriği önemsiz, bağlantı havuzu ısınır).
fn prewarm_connections(client: &reqwest::Client) {
    /// Yanıtı yok sayar: amaç bağlantıyı kurmak (DNS + TLS + HTTP/2 el sıkışması).
    async fn warm(client: &reqwest::Client, url: &str) {
        let _ = client
            .get(url)
            .header("user-agent", converters::builtin::DISCORD_USER_AGENT)
            .send()
            .await;
    }

    let client = client.clone();
    tokio::spawn(async move {
        tokio::join!(
            // X verisi (FxEmbed/FxTwitter API) — hızlı yol
            warm(&client, "https://api.fxtwitter.com/i/status/20"),
            // Discord REST — kart gönderimi (serenity'den ayrı istemci kullanılır)
            warm(&client, "https://discord.com/api/v10/gateway"),
            // Medya CDN'i — ışık hızı modunda kart bu bağlantıları taşır
            warm(&client, "https://pbs.twimg.com/media/"),
        );
    });
}

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

    // Kalıcı kayıt deposu (SQLite): kart kayıtları, 🗑️ silme izni ve AI koruması.
    // Bot yeniden başladığında/güncellendiğinde hiçbir kayıt kaybolmaz.
    store::init();

    // HTTP/2 + bağlantı havuzu: paralel istekler tek bağlantıda çoğullanır, TLS el sıkışması
    // tekrarlanmaz → dönüşüm gecikmesi düşer.
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(8))
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .pool_max_idle_per_host(8)
        .tcp_nodelay(true)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    // Işık hızı: ilk dönüşümde DNS + TLS el sıkışması beklemesin diye bağlantılar ısıtılır.
    prewarm_connections(&http_client);

    // Bot Gateway Intent'leri
    // NOT: Discord Developer Portal'dan "MESSAGE CONTENT INTENT" mutlaka aktif edilmelidir!
    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::GUILD_MESSAGE_REACTIONS
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::DIRECT_MESSAGE_REACTIONS;

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
    let web_config = config.clone();
    let web_http = client.http.clone();
    let web_cache = client.cache.clone();
    tokio::spawn(async move {
        web::start_web_server(web_settings, &web_config, web_http, web_cache).await;
    });


    // Güvenli durdurma sinyali dinleyicisi (CTRL+C ve Unix SIGTERM)
    let shard_manager = client.shard_manager.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            // panic=abort profiline karşı: SIGTERM kaydı başarısız olsa da süreç
            // abort edilmemeli, en azından Ctrl+C ile temiz kapanabilmeli
            match signal(SignalKind::terminate()) {
                Ok(mut sigterm) => {
                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => {}
                        _ = sigterm.recv() => {}
                    }
                }
                Err(err) => {
                    warn!("SIGTERM dinleyicisi kaydedilemedi: {err:?}; yalnızca Ctrl+C dinleniyor");
                    let _ = tokio::signal::ctrl_c().await;
                }
            }
        }
        #[cfg(not(unix))]
        {
            if let Err(err) = tokio::signal::ctrl_c().await {
                error!("Sinyal dinleyicisi başlatılamadı: {err:?}");
                return;
            }
        }
        info!("Kapatma sinyali alındı. Shard yöneticisi durduruluyor...");
        shard_manager.shutdown_all().await;
    });

    if let Err(err) = client.start().await {
        error!("Bot çalışırken kritik hata oluştu: {:?}", err);
    }
}
