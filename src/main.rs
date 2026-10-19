mod ai;
#[cfg(test)]
mod benchmarks;
mod config;
mod converters;
mod handler;
mod policy;
mod settings;
mod store;
mod telemetry;
mod utils;
mod version;
mod web;

use std::sync::Arc;
use tokio::sync::RwLock;

use config::Config;
use handler::Handler;
use serenity::all::{Client, GatewayIntents};
use tracing::error;
use tracing::info;
use tracing_subscriber::EnvFilter;

/// Komut satırı kullanımı (`botgek --help`).
const USAGE: &str = "\
botgek — Discord link dönüştürücü + AI bot

Kullanım:
  botgek            botu başlatır (.env + settings.json)
  botgek --version  sürüm künyesini basar (sürüm, sha, tarih, hedef, etiket)
  botgek --help     bu metni basar

Yapılandırma: .env (bkz. .env.example) ve settings.json (WebUI /api/settings).";

/// Komut satırı bayrağının yorumu.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CliAction {
    /// Botu normal başlat.
    Run,
    /// `--version` / `-V`: sürüm künyesini bas.
    Version,
    /// `--help` / `-h`: kullanımı bas.
    Help,
    /// Tanınmayan bayrak (kullanım hatası → çıkış kodu 2).
    Unknown(String),
}

/// İlk argümanı yorumlar. Saf fonksiyondur: yan etkisi yoktur, testler doğrudan çağırır.
fn parse_cli_arg(arg: Option<&str>) -> CliAction {
    match arg {
        None => CliAction::Run,
        Some("--version") | Some("-V") => CliAction::Version,
        Some("--help") | Some("-h") => CliAction::Help,
        Some(other) => CliAction::Unknown(other.to_string()),
    }
}

/// Kart gönderiminde/veri çekiminde kullanılan uçlar. Her biri için DNS + TCP + TLS
/// el sıkışması bir kez ödenir; bağlantı havuzu atıl kalırsa **her dönüşüm** bu maliyeti
/// yeniden öder (ölçülen: Discord POST'unda ~150-250 ms).
const KEEPALIVE_TARGETS: [(&str, &str); 5] = [
    // X verisi (FxTwitter API) — açıkça FxTwitter seçildiyse kullanılan yol
    (
        "https://api.fxtwitter.com/i/status/20",
        converters::builtin::USER_AGENT,
    ),
    // X verisi (vxTwitter API) — varsayılan çeviri kaynağı
    (
        "https://api.vxtwitter.com/i/status/20",
        converters::builtin::USER_AGENT,
    ),
    // X'in kendi syndication ucu — hızlı kart verisi ve hedge yedeği
    (
        "https://cdn.syndication.twimg.com/",
        converters::builtin::USER_AGENT,
    ),
    // Discord REST — kart gönderimi (serenity'den ayrı istemci kullanılır)
    (
        "https://discord.com/api/v10/gateway",
        converters::builtin::DISCORD_USER_AGENT,
    ),
    // Medya CDN'i — ışık hızı modunda kart bu bağlantıları taşır
    (
        "https://pbs.twimg.com/media/",
        converters::builtin::USER_AGENT,
    ),
];

/// Bağlantı havuzunu sıcak tutmak için yoklama aralığı. İstemcinin `pool_idle_timeout`
/// değerinden (90 sn) **küçük** olmalı; aksi hâlde bağlantı iki yoklama arasında atıl kalır.
const KEEPALIVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(45);

/// Tek bir uca **arka planda** ısıtma isteği yapar (yanıt içeriği önemsiz: amaç DNS +
/// TCP + TLS + HTTP/2 el sıkışmasını kurup bağlantı havuzunu sıcak tutmak).
async fn warm(client: &reqwest::Client, url: &str, user_agent: &str) {
    let Ok(response) = client
        .get(url)
        .header("user-agent", user_agent)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    else {
        return;
    };
    // Gövdeyi tüketmeden Response'u bırakmak HTTP/2 akışını sıfırlayıp bağlantının
    // sonraki kart gönderiminde yeniden kullanılmasını engelleyebilir.
    let _ = response.bytes().await;
}

/// Tüm uçları ısıtır; her istek kendi görevinde yürür, biri yavaşsa diğerlerini beklemez.
/// Hatalar sessizce yutulur (ısıtma başarısız olsa da dönüşüm çalışır).
fn warm_all(client: &reqwest::Client) {
    for (url, user_agent) in KEEPALIVE_TARGETS {
        let client = client.clone();
        tokio::spawn(async move {
            warm(&client, url, user_agent).await;
        });
    }
}

/// İlk dönüşümde DNS + TLS el sıkışması beklenmesin diye uçları **açılışta** ısıtır ve
/// dönüşümler seyrek olduğu için bağlantı havuzunu düzenli aralıklarla sıcak tutar.
fn prewarm_connections(client: &reqwest::Client) {
    let client = client.clone();
    tokio::spawn(async move {
        warm_all(&client);
        let mut ticker = tokio::time::interval(KEEPALIVE_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // `interval` ilk tıkı hemen verir; açılış ısıtması zaten yapıldı.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            warm_all(&client);
        }
    });
}

#[tokio::main]
async fn main() {
    // CLI: sürüm/kullanım bayrakları bot yapılandırmasından **önce** işlenir; böylece
    // `botgek --version` .env olmadan da çalışır (systemd/CI/Docker teşhisi).
    match parse_cli_arg(std::env::args().nth(1).as_deref()) {
        CliAction::Run => {}
        CliAction::Version => {
            println!("botgek {}", version::summary());
            return;
        }
        CliAction::Help => {
            println!("{USAGE}");
            return;
        }
        CliAction::Unknown(flag) => {
            eprintln!("bilinmeyen argüman: {flag}\n");
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }

    // .env dosyasını yükle
    dotenvy::dotenv().ok();

    // Loglama sistemini başlat
    let default_filter = "botgek=info,serenity=warn";
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));

    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    info!("Botgek başlatılıyor...");
    // Sürüm, derleme zamanında git etiketlerinden gömülür (`build.rs`): hangi ikilinin
    // çalıştığı, hangi commit'ten ve hangi hedeften geldiği tek satırda görünür.
    info!("Sürüm: {}", version::summary());

    // Konfigürasyonu oku
    let config = match Config::from_env() {
        Ok(c) => c,
        Err(err) => {
            error!("{}", err);
            std::process::exit(1);
        }
    };

    // Dinamik Ayarları Yükle (settings.json)
    let loaded = match settings::AppSettings::load() {
        Ok(settings) => settings,
        Err(reason) => {
            error!("{reason}");
            std::process::exit(1);
        }
    };
    let settings = Arc::new(RwLock::new(loaded));

    // Kalıcı kayıt deposu (SQLite): kart kayıtları, 🗑️ silme izni ve AI koruması.
    // Bot yeniden başladığında/güncellendiğinde hiçbir kayıt kaybolmaz.
    if let Err(reason) = store::init() {
        error!("Kalıcı kayıt deposu hazır değil: {reason}");
        std::process::exit(1);
    }

    // HTTP/2 + bağlantı havuzu: paralel istekler tek bağlantıda çoğullanır, TLS el sıkışması
    // tekrarlanmaz → dönüşüm gecikmesi düşer.
    let http_client = reqwest::Client::builder()
        .tls_backend_rustls()
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(8))
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .pool_max_idle_per_host(8)
        // Discord bağlantısını boşta da canlı tut: ilk karttan sonra oluşan TLS/HTTP2
        // yeniden el sıkışması, gönderim fazının kuyruğa takılmasına neden olmasın.
        .http2_keep_alive_interval(std::time::Duration::from_secs(30))
        .http2_keep_alive_timeout(std::time::Duration::from_secs(10))
        .http2_keep_alive_while_idle(true)
        .tcp_nodelay(true)
        .build();
    // Zaman aşımsız bir yedek istemci, takılan her isteği sonsuza dek bekletirdi.
    let http_client = match http_client {
        Ok(client) => client,
        Err(err) => {
            error!("HTTP istemcisi kurulamadı: {err}");
            std::process::exit(1);
        }
    };

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
            use tokio::signal::unix::{SignalKind, signal};
            // SIGTERM kaydı başarısız olsa da süreç en azından Ctrl+C ile temiz kapanabilmeli
            match signal(SignalKind::terminate()) {
                Ok(mut sigterm) => {
                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => {}
                        _ = sigterm.recv() => {}
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        "SIGTERM dinleyicisi kaydedilemedi: {err:?}; yalnızca Ctrl+C dinleniyor"
                    );
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
        // Sıfır olmayan çıkış: systemd/Docker yeniden başlatma politikası devreye girsin.
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Isıtma aralığı bağlantı havuzunun atıl kalma süresinden **kısa** olmalı; aksi hâlde
    /// bağlantı her yoklama arasında kapanır ve kazanç kaybolur.
    #[test]
    fn keepalive_interval_is_shorter_than_the_idle_pool_timeout() {
        assert!(KEEPALIVE_INTERVAL < std::time::Duration::from_secs(90));
        assert!(
            KEEPALIVE_TARGETS
                .iter()
                .any(|(url, _)| url.contains("discord.com/api"))
        );
        assert!(
            KEEPALIVE_TARGETS
                .iter()
                .any(|(url, _)| url.contains("vxtwitter"))
        );
        assert!(
            KEEPALIVE_TARGETS
                .iter()
                .any(|(url, _)| url.contains("syndication"))
        );
    }

    /// CLI bayrakları: bayrak yoksa bot çalışır; sürüm/kullanım bayrakları çıkışa yol açar.
    #[test]
    fn cli_flags_are_parsed() {
        assert_eq!(parse_cli_arg(None), CliAction::Run);
        assert_eq!(parse_cli_arg(Some("--version")), CliAction::Version);
        assert_eq!(parse_cli_arg(Some("-V")), CliAction::Version);
        assert_eq!(parse_cli_arg(Some("--help")), CliAction::Help);
        assert_eq!(parse_cli_arg(Some("-h")), CliAction::Help);
        assert_eq!(
            parse_cli_arg(Some("--yanlis")),
            CliAction::Unknown("--yanlis".to_string())
        );
    }

    /// Kullanım metni bayrakları ve yapılandırma kaynaklarını belgelemeli.
    #[test]
    fn usage_text_documents_flags() {
        assert!(USAGE.contains("--version"));
        assert!(USAGE.contains("--help"));
        assert!(USAGE.contains(".env"));
    }
}
