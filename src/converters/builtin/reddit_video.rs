//! Reddit (v.redd.it) videolarını **sesli tek MP4**'e birleştirir.
//!
//! Reddit video ve sesi ayrı akışlarda tutar: `CMAF_{h}.m3u8/mp4` **video-only**, `CMAF_AUDIO_*`
//! **ses-only**. Projede **ffmpeg yok**; bu yüzden birleştirme, saf Rust **`hls-transmux`** ile
//! **yeniden kodlama yapılmadan** (örnekler opak kalır, yalnızca konteyner yeniden yazılır)
//! yapılır: master playlist çözülür, video ve ses medya playlist'leri **iki ayrı girdi** olarak
//! (`MultiTrackInputs::with_audio`) tek oturuma verilir ve çıktı tek MP4 olur.
//!
//! Kaynaklar (master playlist + parçalar) v.redd.it CDN'inden **kimliksiz** erişilebilir; ek
//! proxy gerekmez. HTTP, kendi `reqwest` istemcimizle sağlanır (`Source` uygulaması): böylece
//! crate'in opsiyonel `default-source` özelliği kapalı kalır ve ikinci bir `reqwest` sürümü
//! indirilmez.
//!
//! Birleştirme **yalnızca video ek olarak gönderilecekken** çağrılır (`attach_videos`); sığmazsa
//! ya da hata olursa çağıran taraf sessiz kart videosuna düşer.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::LazyLock;
use std::time::Duration;

use hls_transmux::crypto::key::{
    KeyClock, KeyFuture, KeyProvider, KeyRequest, KeyResolution, KeySession, KeySessionOptions,
};
use hls_transmux::playlist::{InputId, PlaylistContext, parse_playlist_snapshot};
use hls_transmux::{
    ByteRange, ContinuousError, ContinuousInput, ContinuousOptions, ContinuousWait, EmbeddedAudio,
    Error as HlsError, MultiTrackInputs, MultiTrackSession, OutputFormat, Source, SourceLocation,
    TextResource, TrackMetadata,
};
use tracing::{debug, warn};
use url::Url;

use super::media::MediaFile;
use super::{REQUEST_TIMEOUT, USER_AGENT};

/// Tek bir parça (segment) okuması için üst sınır: 16 MB.
///
/// HLS parçaları genelde birkaç yüz KB'dir; BYTERANGE kullanılmayan listelerde parça tek dosyanın
/// tamamı olabilir — bu yüzden kart medya sınırından (`MAX_BODY_BYTES` = 2 MB) geniş tutulur.
const MAX_PART_BYTES: usize = 16 * 1024 * 1024;
static MERGE_SLOTS: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(2)));

/// `v.redd.it/{id}/...` bağlantısından video kimliğini çıkarır (`4ilyyn689buh1` gibi).
fn video_id_from_url(url: &str) -> Option<String> {
    let rest = url.split("v.redd.it/").nth(1)?;
    let id = rest.split(['/', '?', '#']).next()?;
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())).then(|| id.to_string())
}

/// Video kimliğinden HLS master playlist bağlantısını üretir.
fn master_playlist_url(video_id: &str) -> String {
    format!("https://v.redd.it/{video_id}/HLSPlaylist.m3u8")
}

/// Master playlist'ten seçilebilir bir varyant.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Variant {
    /// Bildirilen bant genişliği (bps) — yüksekten düşüğe sıralama için kullanılır.
    bandwidth: u64,
    /// Video medya playlist adresi (master'a göre göreli olabilir).
    video: String,
    /// Bu varyantın `AUDIO="grup"` eşleşmesinden bulunan ses medya playlist adresi.
    audio: Option<String>,
}

/// `KEY=VALUE,KEY="VALUE"` biçimindeki HLS özniteliklerini çözer.
fn parse_attributes(input: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut key = String::new();
    let mut value = String::new();
    let mut in_quotes = false;
    let mut reading_key = true;

    for ch in input.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                if !key.trim().is_empty() {
                    map.insert(key.trim().to_string(), value.trim().to_string());
                }
                key.clear();
                value.clear();
                reading_key = true;
            }
            '=' if reading_key && !in_quotes => reading_key = false,
            _ if reading_key => key.push(ch),
            _ => value.push(ch),
        }
    }
    if !key.trim().is_empty() {
        map.insert(key.trim().to_string(), value.trim().to_string());
    }

    map
}

/// HLS **master** playlist'ini çözümler.
///
/// Reddit sesi ayrı rendition olarak verir (`#EXT-X-MEDIA:TYPE=AUDIO`); bu yüzden
/// `EXT-X-STREAM-INF` satırındaki `AUDIO="grup"` değeri ilgili ses playlist'ine eşlenir.
fn parse_master_playlist(text: &str) -> Vec<Variant> {
    let mut audio_groups: HashMap<String, String> = HashMap::new();
    let mut variants: Vec<Variant> = Vec::new();
    let mut pending: Option<(u64, Option<String>)> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(attributes) = line.strip_prefix("#EXT-X-MEDIA:") {
            let parsed = parse_attributes(attributes);
            if parsed.get("TYPE").map(String::as_str) == Some("AUDIO")
                && let (Some(group), Some(uri)) = (parsed.get("GROUP-ID"), parsed.get("URI"))
            {
                audio_groups.insert(group.clone(), uri.clone());
            }
            continue;
        }

        if let Some(attributes) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            let parsed = parse_attributes(attributes);
            let bandwidth = parsed
                .get("BANDWIDTH")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            pending = Some((bandwidth, parsed.get("AUDIO").cloned()));
            continue;
        }

        if line.starts_with('#') {
            continue;
        }

        // Varyant URI'si `EXT-X-STREAM-INF` satırından sonraki ilk yorum olmayan satırdır.
        if let Some((bandwidth, audio_group)) = pending.take() {
            let audio = audio_group.and_then(|group| audio_groups.get(&group).cloned());
            variants.push(Variant {
                bandwidth,
                video: line.to_string(),
                audio,
            });
        }
    }

    variants
}

/// Denenecek varyant sırası: en yüksek, orta ve en düşük kalite (yinelenen adresler atılır).
///
/// Böylece yüksek kalite Discord ek limitini aşarsa doğrudan en düşüğe düşmek yerine ara
/// çözünürlük denenir.
fn variant_ladder(variants: &[Variant]) -> Vec<&Variant> {
    let mut sorted: Vec<&Variant> = variants.iter().collect();
    sorted.sort_by_key(|a| std::cmp::Reverse(a.bandwidth));
    sorted.dedup_by(|a, b| a.video == b.video);
    if sorted.is_empty() {
        return sorted;
    }

    let mut ladder: Vec<&Variant> = Vec::new();
    for index in [0, sorted.len() / 2, sorted.len() - 1] {
        let candidate = sorted[index];
        if !ladder
            .iter()
            .any(|existing| existing.video == candidate.video)
        {
            ladder.push(candidate);
        }
    }
    ladder
}

/// Konumdan URL üretir (yerel dosya konumları desteklenmez).
fn location_url(location: &SourceLocation) -> Result<Url, HlsError> {
    match location {
        SourceLocation::Url(url)
            if url.scheme() == "https"
                && url.host_str() == Some("v.redd.it")
                && url.username().is_empty()
                && url.password().is_none()
                && url.port_or_known_default() == Some(443) =>
        {
            Ok(url.clone())
        }
        SourceLocation::Url(_) => Err(HlsError::InvalidInput(
            "Reddit media must stay on v.redd.it".into(),
        )),
        SourceLocation::File(path) => Err(HlsError::InvalidInput(format!(
            "yerel dosya konumu desteklenmiyor: {}",
            path.display()
        ))),
    }
}

/// Botun kendi `reqwest` istemcisini kullanan `Source` uygulaması.
#[derive(Debug, Clone)]
struct ReqwestSource {
    client: reqwest::Client,
}

impl fmt::Display for ReqwestSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("reqwest")
    }
}

impl Source for ReqwestSource {
    fn read_text<'a>(
        &'a self,
        location: &'a SourceLocation,
    ) -> Pin<Box<dyn Future<Output = Result<TextResource, HlsError>> + Send + 'a>> {
        Box::pin(async move {
            let url = location_url(location)?;
            let content = super::get_text(&self.client, url.as_str())
                .await
                .map_err(HlsError::Http)?;
            Ok(TextResource {
                content,
                location: SourceLocation::Url(url),
            })
        })
    }

    fn read_bytes<'a>(
        &'a self,
        location: &'a SourceLocation,
        range: Option<&'a ByteRange>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, HlsError>> + Send + 'a>> {
        Box::pin(async move {
            let url = location_url(location)?;
            let mut request = self
                .client
                .get(url.clone())
                .header("user-agent", USER_AGENT)
                .header("accept", "*/*")
                .timeout(REQUEST_TIMEOUT);
            if let Some(range) = range {
                request = request.header(
                    "range",
                    format!(
                        "bytes={}-{}",
                        range.offset,
                        range.offset + range.length.saturating_sub(1)
                    ),
                );
            }

            let response = request
                .send()
                .await
                .map_err(|err| HlsError::Http(format!("{url}: {err}")))?;
            if !response.status().is_success() {
                return Err(HlsError::Http(format!("{url}: {}", response.status())));
            }

            let bytes = read_limited(response, MAX_PART_BYTES).await?;
            // Reddit'in AAC parçalarındaki `sbgp`(roll) kutuları motorun "resource-only" profiline
            // takılır; kutu çıkarılırken örnek verisi hiç değişmez (bkz. `strip_sample_group_boxes`).
            Ok(strip_sample_group_boxes(&bytes).unwrap_or(bytes))
        })
    }
}

/// fMP4 kutusu başlığını okur: (boyut, tip). Geçersiz/taşan kutuda `None`.
fn read_box(data: &[u8], offset: usize) -> Option<(usize, [u8; 4])> {
    let header = data.get(offset..offset.checked_add(8)?)?;
    let size = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
    if size < 8 || offset.checked_add(size)? > data.len() {
        return None;
    }
    Some((size, [header[4], header[5], header[6], header[7]]))
}

/// `traf` gövdesindeki `trun` kutularının `data_offset` alanlarını `delta` kadar kaydırır.
///
/// `tfhd.default-base-is-moof` kullanıldığında offset, `moof` başına göre görelidir; `moof`
/// küçüldüğünde `mdat` başa yaklaşır, bu yüzden tüm `trun` offset'leri azaltılmalıdır.
fn adjust_trun_offsets(payload: &mut [u8], delta: i32) -> Option<()> {
    let mut offset = 0;
    while offset + 8 <= payload.len() {
        let (size, kind) = read_box(payload, offset)?;
        if &kind == b"trun" {
            let fullbox = payload.get(offset + 8..offset + size)?;
            let flags = u32::from_be_bytes(fullbox.get(..4)?.try_into().ok()?);
            if flags & 1 != 0 {
                if size < 20 {
                    return None;
                }
                let at = offset + 16; // box header + version/flags + sample_count
                let current = i32::from_be_bytes(payload.get(at..at + 4)?.try_into().ok()?);
                payload
                    .get_mut(at..at + 4)?
                    .copy_from_slice(&current.checked_sub(delta)?.to_be_bytes());
            }
        }
        offset += size;
    }
    (offset == payload.len()).then_some(())
}

/// Parçadaki `sbgp`/`sgpd` (örnek grubu) kutularını çıkarır.
///
/// `hls-transmux` "resource-only" profilinde bu kutuları kabul etmez; Reddit'in **AAC** parçaları
/// `sbgp`(roll) taşır (video parçalarında yoktur), bu yüzden doğrulama başarısız olur. Roll grubu
/// yalnızca kodek primerleme ipucudur: kutular çıkarılırken **örnek verisi hiç değişmez**, yalnızca
/// `moof`/`traf` boyutları ve `trun` göreli offsetleri tutarlı biçimde güncellenir.
///
/// Değişiklik gerekmiyorsa ya da yapı beklenmedikse `None` döner (çağıran özgün baytları kullanır).
fn strip_sample_group_boxes(data: &[u8]) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(data.len());
    let mut cursor = 0;
    let mut changed = false;
    while cursor < data.len() {
        let (size, kind) = read_box(data, cursor)?;
        let current = data.get(cursor..cursor + size)?;
        if kind == *b"moof" {
            if let Some(rewritten) = strip_moof(current) {
                output.extend_from_slice(&rewritten);
                changed = true;
            } else {
                output.extend_from_slice(current);
            }
        } else {
            output.extend_from_slice(current);
        }
        cursor += size;
    }
    changed.then_some(output)
}

fn strip_moof(data: &[u8]) -> Option<Vec<u8>> {
    let mut cursor = 0;
    let mut moof = None;
    while let Some((size, kind)) = read_box(data, cursor) {
        if &kind == b"moof" {
            moof = Some((cursor, size));
            break;
        }
        cursor += size;
    }
    let (moof_offset, moof_size) = moof?;
    let moof_end = moof_offset + moof_size;

    let mut removed_total = 0usize;
    let mut removed_at: Vec<usize> = Vec::new();
    let mut traf_shrinks: Vec<(usize, usize)> = Vec::new();

    let mut pos = moof_offset + 8;
    while pos < moof_end {
        let (size, kind) = read_box(data, pos)?;
        if pos + size > moof_end {
            return None;
        }
        if &kind == b"sbgp" || &kind == b"sgpd" {
            removed_at.push(pos);
            removed_total += size;
        } else if &kind == b"traf" {
            let traf_end = pos + size;
            let mut inner = pos + 8;
            let mut shrink = 0usize;
            while inner < traf_end {
                let (inner_size, inner_kind) = read_box(data, inner)?;
                if inner + inner_size > traf_end {
                    return None;
                }
                if &inner_kind == b"sbgp" || &inner_kind == b"sgpd" {
                    removed_at.push(inner);
                    shrink += inner_size;
                }
                inner += inner_size;
            }
            if shrink > 0 {
                traf_shrinks.push((pos, shrink));
                removed_total += shrink;
            }
        }
        pos += size;
    }

    if removed_total == 0 {
        return None;
    }

    let mut output: Vec<u8> = Vec::with_capacity(data.len() - removed_total);
    output.extend_from_slice(data.get(..moof_offset)?);
    output.extend_from_slice(&((moof_size - removed_total) as u32).to_be_bytes());
    output.extend_from_slice(b"moof");

    let mut pos = moof_offset + 8;
    while pos < moof_end {
        let (size, kind) = read_box(data, pos)?;
        if removed_at.contains(&pos) {
            pos += size;
            continue;
        }
        let shrink = traf_shrinks
            .iter()
            .find(|(at, _)| *at == pos)
            .map(|(_, shrink)| *shrink)
            .unwrap_or(0);
        let mut payload = Vec::new();
        if &kind == b"traf" {
            let mut inner = pos + 8;
            while inner < pos + size {
                let (child_size, _) = read_box(data, inner)?;
                if inner + child_size > pos + size {
                    return None;
                }
                if !removed_at.contains(&inner) {
                    payload.extend_from_slice(data.get(inner..inner + child_size)?);
                }
                inner += child_size;
            }
            // Every traf points into the same mdat, including tracks without removed boxes.
            adjust_trun_offsets(&mut payload, i32::try_from(removed_total).ok()?)?;
        } else {
            payload.extend_from_slice(data.get(pos + 8..pos + size)?);
        }
        output.extend_from_slice(&((size - shrink) as u32).to_be_bytes());
        output.extend_from_slice(&kind);
        output.extend_from_slice(&payload);
        pos += size;
    }
    output.extend_from_slice(data.get(moof_end..)?);

    Some(output)
}

/// Yanıt gövdesini sınırlı okuyup ham baytları döner.
async fn read_limited(mut response: reqwest::Response, max: usize) -> Result<Vec<u8>, HlsError> {
    if let Some(len) = response.content_length()
        && len > max as u64
    {
        return Err(HlsError::InvalidInput(format!(
            "yanıt çok büyük: {len} bayt"
        )));
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| HlsError::Http(err.to_string()))?
    {
        if body.len() + chunk.len() > max {
            return Err(HlsError::InvalidInput("parça boyutu sınırı aşıldı".into()));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Crate hatalarını tek biçime indirger (yalnızca loglanır; akışı bozmaz).
fn hls_err<E: fmt::Debug>(err: E) -> HlsError {
    HlsError::InvalidInput(format!("{err:?}"))
}

/// Motor hatalarını, tanılamayı **kaybetmeden** (kaynak/örnek nedeni dahil) çevirir.
fn engine_err(err: ContinuousError) -> HlsError {
    match err.raw_cause() {
        Some(cause) => HlsError::InvalidInput(format!("{err:?} | neden: {cause:?}")),
        None => hls_err(err),
    }
}

/// Şifreleme yok: Reddit akışları şifresizdir, anahtar hiçbir zaman sağlanmaz.
struct NoKeys;

impl KeyProvider for NoKeys {
    fn resolve(&self, _request: KeyRequest) -> KeyFuture<KeyResolution> {
        Box::pin(async { KeyResolution::Unavailable })
    }
}

/// Anahtar ömrü hesabı için saat (şifresiz içerikte kullanılmaz).
struct Clock;

impl KeyClock for Clock {
    fn now(&self) -> u64 {
        0
    }
}

/// Motorun beklemeleri için basit uyutucu (çok-izli oturumda **zorunludur**, aksi halde motor
/// `InvalidOptions` döner).
struct Waiter;

impl ContinuousWait for Waiter {
    fn wait(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(tokio::time::sleep(duration))
    }
}

/// Reddit videosunu **sesli** tek MP4'e birleştirir.
///
/// `hls-transmux`'ın future'ları `Send` **değildir**; bu yüzden birleştirme, kendi current-thread
/// Tokio runtime'ına sahip ayrı bir iş parçacığında yürütülür ve sonuç kanalla geri taşınır —
/// böylece çağıran taraf (serenity olay döngüsü) `Send` kalır.
///
/// Master playlist çözülür; en yüksek → orta → en düşük kalite sırasıyla denenir ve Discord ek
/// limitine (`max_bytes`) sığan ilk sonuç döner. Herhangi bir hata `None` döner: çağıran taraf
/// sessiz kart videosuna (Reddit'in video-only mp4'ü) düşer, akış bozulmaz.
pub async fn merge(client: &reqwest::Client, video_url: &str, max_bytes: u64) -> Option<MediaFile> {
    let video_id = video_id_from_url(video_url)?;
    let _ = client;
    // Reject overload rather than creating an unbounded queue of native threads.
    let permit = MERGE_SLOTS.clone().try_acquire_owned().ok()?;
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 3
                || attempt.url().host_str() != Some("v.redd.it")
                || attempt.url().scheme() != "https"
            {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .timeout(REQUEST_TIMEOUT)
        .build()
        .ok()?;

    let (sender, receiver) = tokio::sync::oneshot::channel();
    let (cancel, cancelled) = tokio::sync::oneshot::channel::<()>();
    let _cancel_on_drop = cancel;
    std::thread::Builder::new()
        .name("reddit-merge".into())
        .spawn(move || {
            let _permit = permit;
            let merged = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()
                .and_then(|runtime| {
                    runtime.block_on(cancellable_merge(
                        cancelled,
                        super::render::VIDEO_MERGE_BUDGET,
                        merge_inner(&client, &video_id, max_bytes),
                    ))
                });
            let _ = sender.send(merged);
        })
        .ok()?;

    receiver.await.ok().flatten()
}

async fn cancellable_merge(
    cancelled: tokio::sync::oneshot::Receiver<()>,
    budget: Duration,
    work: impl Future<Output = Option<MediaFile>>,
) -> Option<MediaFile> {
    tokio::select! {
        _ = cancelled => None,
        result = tokio::time::timeout(budget, work) => result.ok().flatten(),
    }
}

/// Birleştirmenin asıl akışı (ayrı iş parçacığında, current-thread runtime üzerinde çalışır).
async fn merge_inner(
    client: &reqwest::Client,
    video_id: &str,
    max_bytes: u64,
) -> Option<MediaFile> {
    let master_url = Url::parse(&master_playlist_url(video_id)).ok()?;
    let master_text = super::get_text(client, master_url.as_str()).await.ok()?;

    let variants = parse_master_playlist(&master_text);
    let ladder = variant_ladder(&variants);
    if ladder.is_empty() {
        debug!("Reddit master playlist'inde varyant bulunamadı: {video_id}");
        return None;
    }

    let source = Arc::new(ReqwestSource {
        client: client.clone(),
    });

    for variant in ladder {
        let video_uri = variant.video.clone();
        let audio_uri = variant.audio.clone();
        match render_variant(
            &master_url,
            &video_uri,
            audio_uri.as_deref(),
            source.clone(),
            max_bytes,
        )
        .await
        {
            Ok(bytes) if bytes.len() as u64 <= max_bytes => {
                debug!(
                    "Reddit videosu sesli MP4 olarak birleştirildi ({} bayt): {video_id}",
                    bytes.len()
                );
                return Some(MediaFile {
                    bytes,
                    filename: format!("{video_id}.mp4"),
                    mime: "video/mp4".to_string(),
                });
            }
            Ok(bytes) => debug!(
                "Reddit videosu ek limitini aşıyor ({} bayt > {max_bytes}), daha küçük varyant deneniyor: {video_id}",
                bytes.len()
            ),
            Err(err) => warn!("Reddit videosu birleştirilemedi ({video_id}): {err:?}"),
        }
    }

    None
}

/// Tek bir varyantı (video medya playlist'i + varsa ses medya playlist'i) tek MP4'e birleştirir.
async fn render_variant(
    master_url: &Url,
    video_uri: &str,
    audio_uri: Option<&str>,
    source: Arc<ReqwestSource>,
    max_bytes: u64,
) -> Result<Vec<u8>, HlsError> {
    let video_url = master_url.join(video_uri).map_err(hls_err)?;
    let video_text = source
        .read_text(&SourceLocation::Url(video_url.clone()))
        .await?
        .content;

    let video_id_input = InputId::new("video").map_err(hls_err)?;
    let video_snapshot = parse_playlist_snapshot(
        &TextResource {
            location: SourceLocation::Url(video_url),
            content: video_text,
        },
        PlaylistContext::new(video_id_input.clone(), 0),
    )
    .map_err(hls_err)?;

    let mut inputs = MultiTrackInputs::new(
        ContinuousInput::new(video_id_input.clone(), source.clone()),
        // Video akışı ses içermez; ses ayrı bir girdi olarak eklenir.
        EmbeddedAudio::Exclude,
    );
    let mut audio_track = None;

    if let Some(audio_uri) = audio_uri {
        let audio_url = master_url.join(audio_uri).map_err(hls_err)?;
        let audio_text = source
            .read_text(&SourceLocation::Url(audio_url.clone()))
            .await?
            .content;
        let audio_id_input = InputId::new("audio").map_err(hls_err)?;
        let audio_snapshot = parse_playlist_snapshot(
            &TextResource {
                location: SourceLocation::Url(audio_url),
                content: audio_text,
            },
            PlaylistContext::new(audio_id_input.clone(), 0),
        )
        .map_err(hls_err)?;
        inputs = inputs.with_audio(
            ContinuousInput::new(audio_id_input.clone(), source.clone()),
            TrackMetadata::new("und", "audio"),
        );
        audio_track = Some((audio_id_input, audio_snapshot));
    }

    let keys = KeySession::new(
        "botgek",
        "reddit",
        Arc::new(NoKeys),
        Arc::new(Clock),
        KeySessionOptions::default(),
    )
    .map_err(hls_err)?;
    // Çok-izli oturumda `waiter` zorunludur (aksi halde motor `InvalidOptions` döner).
    let options =
        ContinuousOptions::default().with_waiter(Arc::new(Waiter), Duration::from_secs(30));
    let session = MultiTrackSession::new(inputs, keys, options).map_err(engine_err)?;

    session
        .handle()
        .accept_snapshot(&video_id_input, &video_snapshot)
        .map_err(engine_err)?;
    // Sonlu (VOD) girdi: snapshot kabul edildikten sonra EOF bildirilir; aksi halde oturum
    // hiçbir zaman tamamlanmaz ve çıktı üretilmez.
    session
        .handle()
        .end_input(&video_id_input)
        .map_err(engine_err)?;

    if let Some((audio_id_input, audio_snapshot)) = &audio_track {
        session
            .handle()
            .accept_snapshot(audio_id_input, audio_snapshot)
            .map_err(engine_err)?;
        session
            .handle()
            .end_input(audio_id_input)
            .map_err(engine_err)?;
    }

    let (bytes, report) = session
        .into_bytes(max_bytes as usize, OutputFormat::FragmentedMp4)
        .await
        .map_err(engine_err)?;
    debug!(
        "Reddit birleştirme raporu: {} bayt, {} iz",
        bytes.len(),
        report.tracks().len()
    );

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn mp4_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut bytes = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn sample_groups_are_removed_and_all_track_offsets_are_rebased() {
        let mut trun = 1u32.to_be_bytes().to_vec(); // data-offset-present
        trun.extend_from_slice(&7u32.to_be_bytes()); // sample_count must remain unchanged
        trun.extend_from_slice(&200i32.to_be_bytes());
        let trun = mp4_box(b"trun", &trun);
        let group = mp4_box(b"sbgp", &[0; 4]);
        let track = mp4_box(b"traf", &[trun.clone(), group.clone()].concat());
        let other = mp4_box(b"traf", &trun);
        let moof = mp4_box(b"moof", &[track, other].concat());
        let data = [moof.clone(), mp4_box(b"mdat", &[42; 8])].concat();
        let result = strip_sample_group_boxes(&data).unwrap();
        assert_eq!(result.len(), data.len() - group.len());
        assert_eq!(read_box(&result, 0).unwrap().0, moof.len() - group.len());
        assert!(!result.windows(4).any(|s| s == b"sbgp"));
        for pos in [8, 36] {
            assert_eq!(read_box(&result, pos).unwrap(), (28, *b"traf"));
            assert_eq!(&result[pos + 20..pos + 24], &7u32.to_be_bytes());
            assert_eq!(&result[pos + 24..pos + 28], &188i32.to_be_bytes());
        }
        assert_eq!(&result[result.len() - 8..], &[42; 8]);
    }

    #[test]
    fn malformed_boxes_and_missing_data_offsets_are_safe() {
        assert!(strip_sample_group_boxes(&[0; 7]).is_none());
        assert!(read_box(&[0; 8], usize::MAX).is_none());
        let mut no_offset = mp4_box(b"trun", &[0; 8]);
        let before = no_offset.clone();
        assert_eq!(adjust_trun_offsets(&mut no_offset, 12), Some(()));
        assert_eq!(no_offset, before);
        let mut truncated = mp4_box(b"trun", &1u32.to_be_bytes());
        assert_eq!(adjust_trun_offsets(&mut truncated, 12), None);
    }

    #[tokio::test]
    async fn cancelling_caller_drops_work_and_overload_does_not_start_downloads() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct Dropped(Arc<AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = Dropped(dropped.clone());
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        let (started, running) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(cancellable_merge(
            cancelled,
            Duration::from_secs(10),
            async move {
                let _guard = guard;
                let _ = started.send(());
                std::future::pending::<Option<MediaFile>>().await
            },
        ));
        running.await.unwrap();
        drop(cancel);
        assert!(
            tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap()
                .is_none()
        );
        assert!(dropped.load(Ordering::SeqCst));
        let _full = MERGE_SLOTS.clone().acquire_many_owned(2).await.unwrap();
        assert!(
            merge(
                &reqwest::Client::new(),
                "https://v.redd.it/test/CMAF_720.mp4",
                1024
            )
            .await
            .is_none()
        );
    }

    /// Reddit'in gerçek master playlist'i (kısaltılmış): iki ses rendition'ı (`GROUP-ID` 5/6) ve
    /// bu gruplara `AUDIO=` ile bağlanan video varyantları.
    const MASTER: &str = r#"#EXTM3U
#EXT-X-VERSION:4
#EXT-X-MEDIA:URI="CMAF_AUDIO_64.m3u8",TYPE=AUDIO,GROUP-ID="5",NAME="audio 0",DEFAULT=YES,AUTOSELECT=YES
#EXT-X-MEDIA:URI="CMAF_AUDIO_128.m3u8",TYPE=AUDIO,GROUP-ID="6",NAME="audio 0",DEFAULT=YES,AUTOSELECT=YES
#EXT-X-STREAM-INF:PROGRAM-ID=0,CLOSED-CAPTIONS=NONE,BANDWIDTH=1401379,RESOLUTION=480x854,CODECS="avc1.4d401f,mp4a.40.2",AUDIO="6"
CMAF_480.m3u8
#EXT-X-STREAM-INF:PROGRAM-ID=0,CLOSED-CAPTIONS=NONE,BANDWIDTH=550641,RESOLUTION=270x480,CODECS="avc1.4d401e,mp4a.40.2",AUDIO="5"
CMAF_270.m3u8
#EXT-X-STREAM-INF:PROGRAM-ID=0,CLOSED-CAPTIONS=NONE,BANDWIDTH=2387007,RESOLUTION=720x1280,CODECS="avc1.4d401f,mp4a.40.2",AUDIO="5"
CMAF_720.m3u8"#;

    #[test]
    fn video_id_is_extracted_from_url() {
        assert_eq!(
            video_id_from_url("https://v.redd.it/4ilyyn689buh1/CMAF_720.mp4?source=fallback")
                .as_deref(),
            Some("4ilyyn689buh1")
        );
        assert_eq!(
            video_id_from_url("https://v.redd.it/abc123/DASH_480.mp4").as_deref(),
            Some("abc123")
        );
        assert_eq!(video_id_from_url("https://example.com/x/y.mp4"), None);
        assert_eq!(video_id_from_url("https://v.redd.it/"), None);
        assert_eq!(
            video_id_from_url("https://v.redd.it/kötü-id/x.mp4"),
            None,
            "alfabe dışı karakter reddedilir"
        );
    }

    #[test]
    fn master_playlist_url_is_built() {
        assert_eq!(
            master_playlist_url("4ilyyn689buh1"),
            "https://v.redd.it/4ilyyn689buh1/HLSPlaylist.m3u8"
        );
    }

    #[test]
    fn master_playlist_parses_variants_and_audio_groups() {
        let variants = parse_master_playlist(MASTER);
        assert_eq!(variants.len(), 3, "üç video varyantı olmalı");

        let v480 = variants
            .iter()
            .find(|v| v.video == "CMAF_480.m3u8")
            .unwrap();
        assert_eq!(v480.bandwidth, 1_401_379);
        assert_eq!(
            v480.audio.as_deref(),
            Some("CMAF_AUDIO_128.m3u8"),
            "AUDIO=6 grubu 128k ses rendition'ına eşlenmeli"
        );

        let v720 = variants
            .iter()
            .find(|v| v.video == "CMAF_720.m3u8")
            .unwrap();
        assert_eq!(v720.bandwidth, 2_387_007);
        assert_eq!(v720.audio.as_deref(), Some("CMAF_AUDIO_64.m3u8"));

        // Öznitelikler tırnaklı değerlerle birlikte doğru çözülür.
        let attributes = parse_attributes(r#"A=1,B="x,y",C="z""#);
        assert_eq!(attributes.get("A").map(String::as_str), Some("1"));
        assert_eq!(attributes.get("B").map(String::as_str), Some("x,y"));
        assert_eq!(attributes.get("C").map(String::as_str), Some("z"));
    }

    #[test]
    fn variant_ladder_is_high_to_low_without_duplicates() {
        let variants = parse_master_playlist(MASTER);
        let ladder = variant_ladder(&variants);
        let uris: Vec<&str> = ladder.iter().map(|v| v.video.as_str()).collect();
        assert_eq!(
            uris,
            vec!["CMAF_720.m3u8", "CMAF_480.m3u8", "CMAF_270.m3u8"]
        );
    }

    /// Canlı liste üzerinden video gönderisi bulup kimliğini döner (yoksa `None`).
    async fn live_video_id(client: &reqwest::Client) -> Option<String> {
        let token =
            super::super::reddit::oauth_token(client, super::super::reddit::DEFAULT_CLIENT_ID)
                .await?;
        let listing = super::super::reddit::get_json_with_bearer(
            client,
            "https://oauth.reddit.com/r/nextfuckinglevel/hot?limit=25&raw_json=1",
            &token,
        )
        .await
        .ok()?;
        listing["data"]["children"]
            .as_array()?
            .iter()
            .find_map(|child| {
                let url = child["data"]["media"]["reddit_video"]["fallback_url"].as_str()?;
                video_id_from_url(url)
            })
    }

    /// Canlı test: Reddit videosu **ses izi içeren** tek MP4 olarak birleştirilmeli.
    ///
    /// Ağ/upstream erişilemezse (ör. kısıtlı CI) test sessizce atlanır.
    #[tokio::test]
    #[ignore = "requires live Reddit video access; run explicitly"]
    async fn live_reddit_video_is_merged_with_audio() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();

        let video_id = live_video_id(&client)
            .await
            .expect("live Reddit video lookup failed");

        let video_url = format!("https://v.redd.it/{video_id}/CMAF_720.mp4");
        let merged = merge(&client, &video_url, 10 * 1024 * 1024)
            .await
            .expect("live video merge failed");
        println!(
            "{video_id}: sesli MP4 birleştirildi — {} bayt",
            merged.bytes.len()
        );

        assert_eq!(merged.mime, "video/mp4");
        assert_eq!(merged.filename, format!("{video_id}.mp4"));
        assert!(merged.bytes.len() > 64 * 1024, "makul boyutta MP4 beklenir");
        assert_eq!(&merged.bytes[4..8], b"ftyp", "MP4 konteyneri (ftyp) olmalı");

        // Ses gerçekten eklenmiş mi? AAC kodek kutusu (`mp4a`) ve ses izi işleyicisi (`soun`).
        let has = |needle: &[u8]| {
            merged
                .bytes
                .windows(needle.len())
                .any(|window| window == needle)
        };
        assert!(
            has(b"mp4a"),
            "AAC ses kodek kutusu bulunmalı (ses birleştirildi)"
        );
        assert!(has(b"soun"), "ses izi işleyicisi (soun) bulunmalı");
    }
}
