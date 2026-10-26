//! Fetch untrusted preview URLs without trusting DNS or redirect targets.
use reqwest::{Url, redirect::Policy};
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || a == 0
        || a >= 240
        || (a == 100 && (64..=127).contains(&b))
        // Only 192.0.0.0/24 is reserved here; 192.0.77.0/24 is a public
        // Tumblr/WordPress VIP range and must not be rejected as if it were
        // the whole 192.0.0.0/16.
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 88 && c == 99)
        || (a == 198 && (b == 18 || b == 19))
        || ip.is_documentation())
}

/// Adres herkese açık (global unicast) mı? Önizleme sınırının tek kaynağıdır: hem buradaki
/// istek yolu hem de [`super::preview`] medya denetimi bu kuralı kullanır.
pub(super) fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_v4(ip),
        IpAddr::V6(ip) => {
            let s = ip.segments();
            // Global unicast only; reject transition and documentation networks as well.
            s[0] & 0xe000 == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}

/// Yalnızca kimlik bilgisi taşımayan, standart portlu HTTP(S) adreslerine izin verilir.
pub(super) fn allowed_endpoint(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(url.port_or_known_default(), Some(80 | 443))
}

fn validate_url(url: &Url) -> Result<(), String> {
    if !allowed_endpoint(url) {
        return Err("preview URL is not an allowed public HTTP endpoint".into());
    }
    Ok(())
}

/// `host`'u çözer; sonuç boşsa, 16 adresten fazlaysa ya da herkese açık olmayan bir adres
/// içeriyorsa reddeder. Dönen adresler bağlantıyı sabitlemek içindir: ikinci bir DNS sorgusu
/// rebinding'e kapı açardı.
pub(super) async fn resolve_public(host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
    let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host.trim_matches(['[', ']']), port))
        .await
        .map_err(|_| "preview DNS failed")?
        .take(17)
        .collect();
    if addresses.is_empty() || addresses.len() > 16 || addresses.iter().any(|a| !public_ip(a.ip()))
    {
        return Err("preview target is not public".into());
    }
    Ok(addresses)
}

/// Sabitlenmiş istemcinin yeniden kullanılabileceği süre (bağlantı havuzunun boşta kalma
/// süresiyle aynı).
const PINNED_CLIENT_TTL: Duration = Duration::from_secs(90);
/// Aynı anda tutulan en fazla sabitlenmiş istemci.
const PINNED_CLIENT_LIMIT: usize = 32;

type PinnedKey = (String, Vec<SocketAddr>);

/// Doğrulanmış adreslere sabitlenmiş istemciler.
///
/// Anahtar, host ve **o istekte** doğrulanmış adres kümesidir: DNS her istekte yeniden çözülüp
/// doğrulanır, sonuç değişirse yeni istemci kurulur. Böylece istemci yalnızca az önce herkese
/// açık olduğu doğrulanan adreslere bağlanabilir (rebinding koruması aynen kalır). Kazanç:
/// aynı CDN'e giden yönlendirme adımları ve ardışık indirmeler TLS bağlantısını yeniden kullanır
/// ve istemci her istekte yeniden kurulmaz (kök sertifikalar dahil ~10-20 ms).
static PINNED_CLIENTS: LazyLock<Mutex<HashMap<PinnedKey, (Instant, reqwest::Client)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn pinned_key(host: &str, addresses: &[SocketAddr]) -> PinnedKey {
    let mut addresses = addresses.to_vec();
    addresses.sort_unstable();
    addresses.dedup();
    (host.to_ascii_lowercase(), addresses)
}

fn pinned_client(host: &str, addresses: &[SocketAddr]) -> Result<reqwest::Client, String> {
    let key = pinned_key(host, addresses);
    {
        let mut clients = PINNED_CLIENTS.lock().unwrap_or_else(|err| err.into_inner());
        clients.retain(|_, (created, _)| created.elapsed() < PINNED_CLIENT_TTL);
        if let Some((_, client)) = clients.get(&key) {
            return Ok(client.clone());
        }
    }
    // İstemci kilit dışında kurulur: kurulum diğer isteklerin önbellek okumasını bekletmesin.
    let client = reqwest::Client::builder()
        .tls_backend_rustls()
        .no_proxy()
        .redirect(Policy::none())
        .resolve_to_addrs(host, addresses)
        .pool_idle_timeout(PINNED_CLIENT_TTL)
        .tcp_nodelay(true)
        .build()
        .map_err(|_| "preview client failed")?;
    let mut clients = PINNED_CLIENTS.lock().unwrap_or_else(|err| err.into_inner());
    if clients.len() >= PINNED_CLIENT_LIMIT
        && let Some(oldest) = clients
            .iter()
            .min_by_key(|(_, (created, _))| *created)
            .map(|(key, _)| key.clone())
    {
        clients.remove(&oldest);
    }
    clients.insert(key, (Instant::now(), client.clone()));
    Ok(client)
}

pub async fn preview(raw: &str) -> Result<(String, String), String> {
    let (url, mut response) = public_get(raw, Duration::from_secs(2), super::USER_AGENT).await?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "preview body failed")? {
        if bytes.len() + chunk.len() > super::MAX_BODY_BYTES {
            return Err("preview body too large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes)
        .map(|text| (url.to_string(), text))
        .map_err(|_| "preview is not UTF-8".into())
}

/// Güvenilmeyen bir adrese GET atar: her hop'ta adres doğrulanır, DNS sonucu yalnızca herkese
/// açık IP'lerden oluşmalıdır ve bağlantı tam bu adreslere sabitlenir. En fazla üç yönlendirme
/// elle izlenir. Başarılı (2xx) yanıt, varılan son adresle birlikte döner; gövde okunmamıştır.
pub async fn public_get(
    raw: &str,
    timeout: Duration,
    user_agent: &str,
) -> Result<(Url, reqwest::Response), String> {
    public_get_with(raw, timeout, user_agent, &[]).await
}

/// [`public_get`] ile aynı, her hop'ta ek başlıklar (ör. TikTok çerezi ve Referer) gönderir.
pub async fn public_get_with(
    raw: &str,
    timeout: Duration,
    user_agent: &str,
    headers: &[(&str, String)],
) -> Result<(Url, reqwest::Response), String> {
    let mut url = Url::parse(raw).map_err(|_| "invalid preview URL")?;
    for _ in 0..=3 {
        validate_url(&url)?;
        let host = url.host_str().ok_or("preview host missing")?;
        let port = url.port_or_known_default().ok_or("preview port missing")?;
        let addresses = resolve_public(host, port).await?;
        // Pin exactly the addresses checked above: a second DNS lookup would allow rebinding.
        let client = pinned_client(host, &addresses)?;
        let mut request = client
            .get(url.clone())
            .timeout(timeout)
            .header("user-agent", user_agent);
        for (name, value) in headers {
            request = request.header(*name, value);
        }
        let response = request.send().await.map_err(|_| "preview request failed")?;
        if response.status().is_redirection() {
            let target = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or("preview redirect missing")?;
            url = url.join(target).map_err(|_| "invalid preview redirect")?;
            continue;
        }
        if !response.status().is_success() {
            return Err("preview response rejected".into());
        }
        return Ok((url, response));
    }
    Err("too many preview redirects".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_reserved_and_transition_addresses_are_denied() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "192.0.2.1",
            "192.0.0.1",
            "198.18.0.1",
            "240.0.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "2002:7f00:1::",
            "2001:db8::1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "8.8.8.8", "192.0.77.40", "2606:4700:4700::1111"] {
            assert!(public_ip(ip.parse().unwrap()));
        }
        for url in [
            "file:///etc/passwd",
            "http://user:pass@example.com/",
            "http://example.com:8080/",
        ] {
            assert!(validate_url(&Url::parse(url).unwrap()).is_err());
        }
    }
    #[test]
    fn pinned_clients_are_reused_only_for_the_same_verified_addresses() {
        let host = "pinned-reuse.example";
        let first: SocketAddr = "1.1.1.1:443".parse().unwrap();
        let second: SocketAddr = "8.8.8.8:443".parse().unwrap();
        let cached = |addresses: &[SocketAddr]| {
            PINNED_CLIENTS
                .lock()
                .unwrap()
                .contains_key(&pinned_key(host, addresses))
        };

        pinned_client(host, &[second, first]).unwrap();
        // Sıra fark etmez: aynı küme aynı istemciyi kullanır.
        assert!(cached(&[first, second]));
        // Farklı (yeniden çözülmüş) bir küme eski istemciyi asla kullanmaz.
        assert!(!cached(&[first]));
        pinned_client(host, &[first]).unwrap();
        assert!(cached(&[first]));
    }

    #[tokio::test]
    async fn loopback_is_rejected_before_connecting() {
        assert!(
            preview("http://127.0.0.1/")
                .await
                .unwrap_err()
                .contains("not public")
        );
    }
}
