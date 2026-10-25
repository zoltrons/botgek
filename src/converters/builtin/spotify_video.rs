//! Spotify 30 sn önizlemesini (MP3) kapak görseliyle **sesli tek MP4**'e çevirir.
//!
//! Discord Components V2 kartında ses dosyası (`File` bileşeni) kartın içinde oynatıcı olarak
//! görünmez; video ise Media Gallery'de kartın içinde oynar. Projede **ffmpeg yok**; bu yüzden
//! video saf Rust ile üretilir:
//! - Kapak JPEG'i `zune-jpeg` ile çözülür, BT.601 YUV 4:2:0'a çevrilir.
//! - Görüntü `less-avc` ile **tek kayıpsız I-kare** (I_PCM, Baseline) olarak kodlanır; karenin
//!   süresi sesin tamamı kadardır (640×640 kapakta ~600 KB).
//! - Ses **AAC-LC**'ye çevrilir: MP3 `symphonia` ile çözülür, `rusty_aac` ile kodlanır. MP4
//!   içinde MP3 (`mp4a`, OTI `0x6B`) masaüstünde oynar ama **iOS Discord'da sessiz** kalır.
//!   Saf Rust AAC kodlayıcılar arasında en az CPU/bellek kullanan `rusty_aac` seçildi
//!   (30 sn stereo: ~1,2 sn CPU, ~13 MB ek bellek; bağımlılığı yok).
//! - Konteyner `shiguredo_mp4` ile, `moov` başta (faststart) olacak şekilde üretilir.
//!
//! Üretim **yalnızca kart gönderilirken** (`render::resolve_media`) ve video bütçesi içinde
//! yapılır; başarısız olursa kart videosuz (yalnızca kapak küçük görseliyle) gönderilir.

use std::io::Cursor;
use std::num::NonZeroU32;
use std::sync::{Arc, LazyLock};

use less_avc::LessEncoder;
use less_avc::ycbcr_image::{DataPlane, Planes, YCbCrImage};
use shiguredo_mp4::boxes::{
    AudioSampleEntryFields, Avc1Box, AvccBox, EsdsBox, Mp4aBox, SampleEntry,
    VisualSampleEntryFields,
};
use shiguredo_mp4::descriptors::{
    DecoderConfigDescriptor, DecoderSpecificInfo, EsDescriptor, SlConfigDescriptor,
};
use shiguredo_mp4::mux::{
    Mp4FileMuxer, Mp4FileMuxerOptions, Sample, estimate_maximum_moov_box_size,
};
use shiguredo_mp4::{FixedPointNumber, TrackKind, Uint};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use tracing::debug;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

use super::media::MediaFile;
use super::{MEDIA_TIMEOUT, USER_AGENT};

/// Önizleme sesi (~30 sn, 96 kbps ≈ 360 KB) için üst sınır.
const MAX_AUDIO_BYTES: u64 = 2 * 1024 * 1024;
/// Kapak görseli için üst sınır.
const MAX_COVER_BYTES: u64 = 2 * 1024 * 1024;
/// I_PCM kare boyutu piksel sayısıyla doğrusal büyür (~1,5 bayt/piksel); Spotify kapakları
/// 640×640'tır. Daha büyük görseller kodlanmaz.
const MAX_PIXELS: usize = 1024 * 1024;
const OUTPUT_FILENAME: &str = "spotify-preview.mp4";
/// AAC hedef bit hızı. Kaynak 96 kbps MP3'tür; yeniden kodlamada pay bırakılır (~470 KB / 30 sn).
const AAC_BITRATE: u32 = 128_000;
/// Önizlemenin üst süresi; daha uzun ses (beklenmeyen kaynak) kodlanmaz.
const MAX_AUDIO_SECONDS: usize = 60;
/// `rusty_aac` kareleri tüm çekirdeklere dağıtır: aynı anda en fazla iki üretim.
static ENCODE_SLOTS: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(2)));

/// Spotify önizleme CDN'i mi? (`https://p.scdn.co/mp3-preview/...`)
pub fn is_preview_url(url: &str) -> bool {
    url::Url::parse(url)
        .ok()
        .is_some_and(|url| url.scheme() == "https" && url.host_str() == Some("p.scdn.co"))
}

/// Önizleme ve kapağı indirip MP4 üretir. Herhangi bir adım başarısızsa ya da sonuç
/// `max_bytes`'ı aşarsa `None` döner.
pub async fn build(preview_url: &str, cover_url: &str, max_bytes: u64) -> Option<MediaFile> {
    let (audio, cover) = tokio::join!(
        download(preview_url, MAX_AUDIO_BYTES),
        download(cover_url, MAX_COVER_BYTES)
    );
    let (audio, cover) = (audio?, cover?);

    // JPEG çözme + kodlama + mux CPU işidir: async çalışanları bloklamasın.
    let _slot = ENCODE_SLOTS.clone().acquire_owned().await.ok()?;
    let bytes = match tokio::task::spawn_blocking(move || encode(&cover, &audio)).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(error)) => {
            debug!("Spotify önizleme videosu üretilemedi ({preview_url}): {error}");
            return None;
        }
        Err(_) => return None,
    };
    if bytes.len() as u64 > max_bytes {
        return None;
    }
    Some(MediaFile {
        bytes,
        filename: OUTPUT_FILENAME.to_string(),
        mime: "video/mp4".to_string(),
    })
}

/// Güvenli HTTP yolu ile boyutu sınırlı indirme (adresler embed verisinden gelir).
async fn download(url: &str, max_bytes: u64) -> Option<Vec<u8>> {
    if !url.starts_with("https://") {
        return None;
    }
    let (_, mut response) = super::safe_http::public_get(url, MEDIA_TIMEOUT, USER_AGENT)
        .await
        .ok()?;
    if response.content_length().is_some_and(|len| len > max_bytes) {
        return None;
    }
    let mut bytes = Vec::with_capacity(64 * 1024);
    while let Some(chunk) = response.chunk().await.ok()? {
        if bytes.len() as u64 + chunk.len() as u64 > max_bytes {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    (!bytes.is_empty()).then_some(bytes)
}

/// Kapak JPEG'i + MP3 → MP4 baytları.
fn encode(cover: &[u8], mp3: &[u8]) -> Result<Vec<u8>, String> {
    let image = jpeg_to_yuv420(cover)?;
    let video = encode_still(&image)?;
    let (pcm, sample_rate, channels) = decode_mp3(mp3)?;
    let audio = encode_aac(&pcm, sample_rate, channels)?;
    mux(&image, video, &audio)
}

/// 16'nın katına kırpılmış BT.601 (limited range) YUV 4:2:0 görüntü.
struct Yuv420 {
    width: u32,
    height: u32,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

fn jpeg_to_yuv420(jpeg: &[u8]) -> Result<Yuv420, String> {
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = JpegDecoder::new_with_options(Cursor::new(jpeg), options);
    decoder
        .decode_headers()
        .map_err(|e| format!("JPEG başlığı çözülemedi: {e:?}"))?;
    let (src_w, src_h) = decoder.dimensions().ok_or("JPEG boyutu yok")?;
    if src_w * src_h > MAX_PIXELS {
        return Err(format!("kapak çok büyük: {src_w}x{src_h}"));
    }
    let rgb = decoder
        .decode()
        .map_err(|e| format!("JPEG çözülemedi: {e:?}"))?;
    rgb_to_yuv420(&rgb, src_w, src_h)
}

/// RGB'yi ortadan 16'nın katına kırpar ve YUV 4:2:0'a çevirir (less-avc makroblok hizası ister).
fn rgb_to_yuv420(rgb: &[u8], src_w: usize, src_h: usize) -> Result<Yuv420, String> {
    if rgb.len() < src_w * src_h * 3 {
        return Err("RGB verisi eksik".into());
    }
    let (w, h) = (src_w / 16 * 16, src_h / 16 * 16);
    if w == 0 || h == 0 {
        return Err("kapak 16 pikselden küçük".into());
    }
    let (ox, oy) = ((src_w - w) / 2, (src_h - h) / 2);
    let px = |x: usize, y: usize| {
        let i = ((oy + y) * src_w + ox + x) * 3;
        (rgb[i] as f32, rgb[i + 1] as f32, rgb[i + 2] as f32)
    };
    let to_u8 = |value: f32| value.round().clamp(0.0, 255.0) as u8;

    let mut y_plane = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = px(x, y);
            y_plane[y * w + x] = to_u8(16.0 + 0.257 * r + 0.504 * g + 0.098 * b);
        }
    }
    let (cw, ch) = (w / 2, h / 2);
    let mut u_plane = vec![0u8; cw * ch];
    let mut v_plane = vec![0u8; cw * ch];
    for y in 0..ch {
        for x in 0..cw {
            let (mut r, mut g, mut b) = (0.0, 0.0, 0.0);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (pr, pg, pb) = px(x * 2 + dx, y * 2 + dy);
                r += pr / 4.0;
                g += pg / 4.0;
                b += pb / 4.0;
            }
            u_plane[y * cw + x] = to_u8(128.0 - 0.148 * r - 0.291 * g + 0.439 * b);
            v_plane[y * cw + x] = to_u8(128.0 + 0.439 * r - 0.368 * g - 0.071 * b);
        }
    }
    Ok(Yuv420 {
        width: w as u32,
        height: h as u32,
        y: y_plane,
        u: u_plane,
        v: v_plane,
    })
}

/// Tek I-karenin NAL birimleri (başlık baytıyla, start code'suz).
struct StillFrame {
    sps: Vec<u8>,
    pps: Vec<u8>,
    frame: Vec<u8>,
}

/// less-avc SPS'e sabit `level_idc = 10` (176×144'e kadar) yazar; 640×640'ta katı çözücüler
/// (mobil donanım) akışı reddedebilir. Makroblok sayısına göre 3.1 ya da 4.0 seçilir.
fn level_for(width: u32, height: u32) -> u8 {
    let macroblocks = (width / 16) * (height / 16);
    if macroblocks <= 3600 { 31 } else { 40 }
}

fn encode_still(image: &Yuv420) -> Result<StillFrame, String> {
    let plane = |data, stride| DataPlane {
        data,
        stride,
        bit_depth: less_avc::BitDepth::Depth8,
    };
    let chroma_stride = image.width as usize / 2;
    let ycbcr = YCbCrImage {
        planes: Planes::YCbCr((
            plane(&image.y, image.width as usize),
            plane(&image.u, chroma_stride),
            plane(&image.v, chroma_stride),
        )),
        width: image.width,
        height: image.height,
    };
    let (initial, _encoder) =
        LessEncoder::new(&ycbcr).map_err(|e| format!("H.264 kodlanamadı: {e:?}"))?;
    let mut sps = initial.sps.to_nal_unit();
    // [NAL başlığı, profile_idc, constraint bayrakları, level_idc, ...]
    if sps.len() < 4 {
        return Err("SPS kısa".into());
    }
    sps[3] = level_for(image.width, image.height);
    Ok(StillFrame {
        sps,
        pps: initial.pps.to_nal_unit(),
        frame: initial.frame.to_nal_unit(),
    })
}

/// MP3'ü interleaved f32 PCM'e çözer: (örnekler, örnekleme hızı, kanal sayısı).
fn decode_mp3(mp3: &[u8]) -> Result<(Vec<f32>, u32, u16), String> {
    let source = MediaSourceStream::new(Box::new(Cursor::new(mp3.to_vec())), Default::default());
    let mut format = symphonia::default::get_probe()
        .probe(
            Hint::new().with_extension("mp3"),
            source,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| format!("MP3 açılamadı: {e}"))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or("MP3 ses izi yok")?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .ok_or("MP3 ses parametreleri yok")?
        .clone();
    let sample_rate = params.sample_rate.ok_or("örnekleme hızı yok")?;
    let channels = params
        .channels
        .as_ref()
        .map(|channels| channels.count())
        .filter(|count| (1..=2).contains(count))
        .ok_or("desteklenmeyen kanal sayısı")?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| format!("MP3 çözücü yok: {e}"))?;

    let max_samples = sample_rate as usize * channels * MAX_AUDIO_SECONDS;
    let mut pcm: Vec<f32> = Vec::with_capacity(sample_rate as usize * channels * 31);
    let mut frame: Vec<f32> = Vec::new();
    while let Some(packet) = format
        .next_packet()
        .map_err(|e| format!("MP3 okunamadı: {e}"))?
    {
        if packet.track_id != track_id {
            continue;
        }
        // Bozuk tek kare tüm önizlemeyi düşürmesin (symphonia örneğiyle aynı tutum).
        let Ok(audio) = decoder.decode(&packet) else {
            continue;
        };
        frame.resize(audio.samples_interleaved(), 0.0);
        audio.copy_to_slice_interleaved(&mut frame);
        pcm.extend_from_slice(&frame);
        if pcm.len() > max_samples {
            return Err("önizleme beklenenden uzun".into());
        }
    }
    if pcm.is_empty() {
        return Err("MP3 karesi çözülemedi".into());
    }
    Ok((pcm, sample_rate, channels as u16))
}

/// AAC-LC ses izi: ham erişim birimleri (ADTS'siz) + `esds` için AudioSpecificConfig.
struct AacTrack {
    packets: Vec<Vec<u8>>,
    sample_rate: u32,
    channels: u16,
    audio_specific_config: Vec<u8>,
    /// Kodlayıcı önceliği (priming) hariç gerçek ses süresi (örnek/kanal).
    pcm_frames: u64,
}

/// Interleaved PCM'i AAC-LC'ye kodlar.
fn encode_aac(pcm: &[f32], sample_rate: u32, channels: u16) -> Result<AacTrack, String> {
    let mut encoder = rusty_aac::AacEncoder::new(rusty_aac::AacEncoderConfig {
        bitrate_bps: AAC_BITRATE,
        ..Default::default()
    });
    encoder
        .push_pcm(pcm, channels, sample_rate)
        .map_err(|e| format!("AAC kodlanamadı: {e}"))?;
    encoder.finish();
    let mut packets = Vec::new();
    while let Ok(packet) = encoder.next_packet() {
        packets.push(packet.data);
    }
    if packets.is_empty() {
        return Err("AAC çıktısı boş".into());
    }
    Ok(AacTrack {
        packets,
        sample_rate,
        channels,
        audio_specific_config: rusty_aac::audio_specific_config_bytes(sample_rate, channels),
        pcm_frames: (pcm.len() / channels as usize) as u64,
    })
}

/// Tek video örneği (süresi = ses süresi) + AAC örnekleri → faststart MP4.
fn mux(image: &Yuv420, video: StillFrame, audio: &AacTrack) -> Result<Vec<u8>, String> {
    let mux_error = |e: shiguredo_mp4::mux::MuxError| format!("MP4 yazılamadı: {e}");

    let video_entry = SampleEntry::Avc1(Avc1Box {
        visual: VisualSampleEntryFields {
            data_reference_index: VisualSampleEntryFields::DEFAULT_DATA_REFERENCE_INDEX,
            width: image.width as u16,
            height: image.height as u16,
            horizresolution: VisualSampleEntryFields::DEFAULT_HORIZRESOLUTION,
            vertresolution: VisualSampleEntryFields::DEFAULT_VERTRESOLUTION,
            frame_count: VisualSampleEntryFields::DEFAULT_FRAME_COUNT,
            compressorname: VisualSampleEntryFields::NULL_COMPRESSORNAME,
            depth: VisualSampleEntryFields::DEFAULT_DEPTH,
        },
        avcc_box: AvccBox {
            avc_profile_indication: video.sps[1],
            profile_compatibility: video.sps[2],
            avc_level_indication: video.sps[3],
            length_size_minus_one: Uint::new(3),
            sps_list: vec![video.sps],
            pps_list: vec![video.pps],
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext_list: vec![],
        },
        unknown_boxes: vec![],
    });
    let audio_entry = SampleEntry::Mp4a(Mp4aBox {
        audio: AudioSampleEntryFields {
            data_reference_index: AudioSampleEntryFields::DEFAULT_DATA_REFERENCE_INDEX,
            channelcount: audio.channels,
            samplesize: AudioSampleEntryFields::DEFAULT_SAMPLESIZE,
            samplerate: FixedPointNumber::new(audio.sample_rate as u16, 0),
        },
        esds_box: EsdsBox {
            es: EsDescriptor {
                es_id: EsDescriptor::MIN_ES_ID,
                stream_priority: EsDescriptor::LOWEST_STREAM_PRIORITY,
                depends_on_es_id: None,
                url_string: None,
                ocr_es_id: None,
                dec_config_descr: DecoderConfigDescriptor {
                    object_type_indication:
                        DecoderConfigDescriptor::OBJECT_TYPE_INDICATION_AUDIO_ISO_IEC_14496_3,
                    stream_type: DecoderConfigDescriptor::STREAM_TYPE_AUDIO,
                    up_stream: DecoderConfigDescriptor::UP_STREAM_FALSE,
                    buffer_size_db: Uint::new(0),
                    max_bitrate: AAC_BITRATE,
                    avg_bitrate: AAC_BITRATE,
                    dec_specific_info: Some(DecoderSpecificInfo {
                        payload: audio.audio_specific_config.clone(),
                    }),
                },
                sl_config_descr: SlConfigDescriptor,
            },
        },
        unknown_boxes: vec![],
    });

    let total_ms = (audio.pcm_frames * 1000 / audio.sample_rate as u64) as u32;
    let options = Mp4FileMuxerOptions {
        reserved_moov_box_size: estimate_maximum_moov_box_size(&[1, audio.packets.len()]),
        ..Default::default()
    };
    let mut muxer = Mp4FileMuxer::with_options(options).map_err(mux_error)?;
    let mut out = muxer.initial_boxes_bytes().to_vec();

    // AVCC: 4 bayt uzunluk önekli NAL.
    let frame_len = u32::try_from(video.frame.len()).map_err(|_| "kare çok büyük")?;
    muxer
        .append_sample(&Sample {
            track_kind: TrackKind::Video,
            sample_entry: Some(video_entry),
            keyframe: true,
            timescale: NonZeroU32::new(1000).expect("sıfır değil"),
            duration: total_ms,
            composition_time_offset: None,
            data_offset: out.len() as u64,
            data_size: video.frame.len() + 4,
        })
        .map_err(mux_error)?;
    out.extend_from_slice(&frame_len.to_be_bytes());
    out.extend_from_slice(&video.frame);

    let timescale = NonZeroU32::new(audio.sample_rate).ok_or("örnekleme hızı sıfır")?;
    for (index, packet) in audio.packets.iter().enumerate() {
        muxer
            .append_sample(&Sample {
                track_kind: TrackKind::Audio,
                sample_entry: (index == 0).then(|| audio_entry.clone()),
                keyframe: true,
                timescale,
                duration: 1024,
                composition_time_offset: None,
                data_offset: out.len() as u64,
                data_size: packet.len(),
            })
            .map_err(mux_error)?;
        out.extend_from_slice(packet);
    }

    let finalized = muxer.finalize().map_err(mux_error)?;
    for (offset, bytes) in finalized.offset_and_bytes_pairs() {
        let start = offset as usize;
        let end = start + bytes.len();
        if out.len() < end {
            out.resize(end, 0);
        }
        out[start..end].copy_from_slice(bytes);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0,5 sn, 440 Hz, 44.1 kHz stereo, 64 kbps MP3 (ffmpeg/LAME ile üretildi).
    const SINE_MP3: &[u8] = include_bytes!("../../../tests/fixtures/sine-440hz-0.5s.mp3");

    fn solid_image(width: usize, height: usize) -> Yuv420 {
        let rgb: Vec<u8> = [30u8, 215, 96].repeat(width * height);
        rgb_to_yuv420(&rgb, width, height).expect("görüntü çevrilmeli")
    }

    #[test]
    fn preview_url_is_limited_to_spotify_cdn() {
        assert!(is_preview_url("https://p.scdn.co/mp3-preview/abc?cid=1"));
        assert!(!is_preview_url("http://p.scdn.co/mp3-preview/abc"));
        assert!(!is_preview_url("https://v.redd.it/abc/HLSPlaylist.m3u8"));
        assert!(!is_preview_url("https://p.scdn.co.evil.example/x"));
    }

    #[test]
    fn mp3_is_decoded_to_interleaved_pcm() {
        let (pcm, rate, channels) = decode_mp3(SINE_MP3).expect("çözülmeli");
        assert_eq!((rate, channels), (44100, 2));
        let frames = pcm.len() / 2;
        // LAME gecikme/dolgu kareleriyle 0,5 sn'ye yakın.
        assert!((22_050..=24_500).contains(&frames), "{frames}");
        let peak = pcm.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
        // ffmpeg `volumedetect` aynı dosyada tepe seviyeyi -21,4 dBFS (≈0,085) ölçer.
        assert!((0.075..0.095).contains(&peak), "tepe seviye: {peak}");
        assert!(decode_mp3(b"not an mp3").is_err());
    }

    #[test]
    fn pcm_is_encoded_to_aac_lc_access_units() {
        let pcm: Vec<f32> = (0..44_100)
            .flat_map(|i| {
                let s = (i as f32 * 440.0 * std::f32::consts::TAU / 44_100.0).sin() * 0.5;
                [s, s]
            })
            .collect();
        let aac = encode_aac(&pcm, 44_100, 2).expect("kodlanmalı");
        assert_eq!(aac.pcm_frames, 44_100);
        // 1 sn / 1024 örnek ≈ 43 kare (+ öncelik/kuyruk).
        assert!(
            (43..=46).contains(&aac.packets.len()),
            "{}",
            aac.packets.len()
        );
        // AudioSpecificConfig: AOT 2 (LC), 44.1 kHz (indeks 4), 2 kanal.
        assert_eq!(aac.audio_specific_config, vec![0x12, 0x10]);
    }

    #[test]
    fn image_is_cropped_to_macroblock_multiple() {
        let image = solid_image(40, 33);
        assert_eq!((image.width, image.height), (32, 32));
        assert_eq!(image.y.len(), 32 * 32);
        assert_eq!(image.u.len(), 16 * 16);
        assert!(rgb_to_yuv420(&[0; 3 * 15 * 15], 15, 15).is_err());
    }

    #[test]
    fn sps_level_is_raised_for_cover_sized_frames() {
        let still = encode_still(&solid_image(640, 640)).expect("kodlanmalı");
        assert_eq!(still.sps[1], 66, "Baseline profil");
        assert_eq!(still.sps[3], 31);
        assert_eq!(level_for(1920, 1088), 40);
    }

    #[test]
    fn muxed_file_is_faststart_mp4_with_avc_and_aac() {
        let image = solid_image(64, 64);
        let still = encode_still(&image).expect("kodlanmalı");
        let (pcm, rate, channels) = decode_mp3(SINE_MP3).expect("çözülmeli");
        let audio = encode_aac(&pcm, rate, channels).expect("kodlanmalı");

        let out = mux(&image, still, &audio).expect("MP4 yazılmalı");
        assert_eq!(&out[4..8], b"ftyp");
        let moov = out.windows(4).position(|w| w == b"moov").expect("moov");
        let mdat = out.windows(4).position(|w| w == b"mdat").expect("mdat");
        assert!(moov < mdat, "moov başta olmalı (faststart)");
        assert!(out.windows(4).any(|w| w == b"avc1"));
        assert!(out.windows(4).any(|w| w == b"mp4a"));
        // esds: DecoderConfigDescriptor (etiket 4) OTI 0x40 = MPEG-4 Audio (AAC), MP3 değil.
        let esds = out.windows(4).position(|w| w == b"esds").expect("esds");
        let decoder_config = out[esds..]
            .iter()
            .position(|&b| b == 0x04)
            .map(|offset| esds + offset)
            .expect("DecoderConfigDescriptor");
        assert!(out[decoder_config..decoder_config + 8].contains(&0x40));
        assert!(!out[esds..esds + 64].contains(&0x6B));
    }
}
