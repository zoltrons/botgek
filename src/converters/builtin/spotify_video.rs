//! Spotify 30 sn önizlemesini (MP3) kapak görseliyle **sesli tek MP4**'e çevirir.
//!
//! Discord Components V2 kartında ses dosyası (`File` bileşeni) kartın içinde oynatıcı olarak
//! görünmez; video ise Media Gallery'de kartın içinde oynar. Projede **ffmpeg yok**; bu yüzden
//! video saf Rust ile üretilir ve **ses yeniden kodlanmaz**:
//! - Kapak JPEG'i `zune-jpeg` ile çözülür, BT.601 YUV 4:2:0'a çevrilir.
//! - Görüntü `less-avc` ile **tek kayıpsız I-kare** (I_PCM, Baseline) olarak kodlanır; karenin
//!   süresi sesin tamamı kadardır (640×640 kapakta ~600 KB).
//! - MP3 kareleri olduğu gibi `mp4a` izine yazılır (OTI `0x6B`/`0x69`); konteyner
//!   `shiguredo_mp4` ile, `moov` başta (faststart) olacak şekilde üretilir.
//!
//! Üretim **yalnızca kart gönderilirken** (`render::resolve_media`) ve video bütçesi içinde
//! yapılır; başarısız olursa kart videosuz (yalnızca kapak küçük görseliyle) gönderilir.

use std::io::Cursor;
use std::num::NonZeroU32;

use less_avc::LessEncoder;
use less_avc::ycbcr_image::{DataPlane, Planes, YCbCrImage};
use shiguredo_mp4::boxes::{
    AudioSampleEntryFields, Avc1Box, AvccBox, EsdsBox, Mp4aBox, SampleEntry,
    VisualSampleEntryFields,
};
use shiguredo_mp4::descriptors::{DecoderConfigDescriptor, EsDescriptor, SlConfigDescriptor};
use shiguredo_mp4::mux::{
    Mp4FileMuxer, Mp4FileMuxerOptions, Sample, estimate_maximum_moov_box_size,
};
use shiguredo_mp4::{FixedPointNumber, TrackKind, Uint};
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
    let audio = parse_mp3(mp3)?;
    mux(&image, video, &audio, mp3)
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

/// MP3 dosyasının kare dizini.
#[derive(Debug, PartialEq, Eq)]
struct Mp3 {
    /// (başlangıç, uzunluk) — dosya içindeki kare aralıkları.
    frames: Vec<(usize, usize)>,
    sample_rate: u32,
    channels: u16,
    samples_per_frame: u32,
    /// MPEG-1 → `0x6B`, MPEG-2/2.5 (LSF) → `0x69`.
    object_type: u8,
    avg_bitrate: u32,
}

impl Mp3 {
    fn total_samples(&self) -> u64 {
        self.frames.len() as u64 * self.samples_per_frame as u64
    }
}

/// MP3'ü karelerine ayırır: baştaki ID3v2 etiketi atlanır, ilk senkrondan sonra geçerli
/// kare zinciri bozulduğunda (ID3v1/çöp) durulur.
fn parse_mp3(data: &[u8]) -> Result<Mp3, String> {
    let mut pos = 0;
    if data.len() >= 10 && &data[..3] == b"ID3" {
        let size = data[6..10]
            .iter()
            .fold(0usize, |acc, b| (acc << 7) | (*b as usize & 0x7f));
        let footer = if data[5] & 0x10 != 0 { 10 } else { 0 };
        pos = 10 + size + footer;
    }

    let mut frames = Vec::new();
    let mut format: Option<FrameHeader> = None;
    let mut total_kbps: u64 = 0;
    while pos + 4 <= data.len() {
        let word = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        let Some(header) = FrameHeader::parse(word) else {
            if frames.is_empty() {
                pos += 1;
                continue;
            }
            break;
        };
        if pos + header.len > data.len() {
            break;
        }
        // Akış ortasında biçim değişirse (bozuk veri) kesilir: tek örnek girdisi tek biçimdir.
        if format.is_some_and(|first| !first.same_format(&header)) {
            break;
        }
        format.get_or_insert(header);
        total_kbps += header.kbps as u64;
        frames.push((pos, header.len));
        pos += header.len;
    }
    let format = format.ok_or("MP3 karesi bulunamadı")?;
    Ok(Mp3 {
        avg_bitrate: (total_kbps * 1000 / frames.len() as u64) as u32,
        frames,
        sample_rate: format.sample_rate,
        channels: format.channels,
        samples_per_frame: format.samples_per_frame,
        object_type: format.object_type,
    })
}

/// MPEG audio Layer III kare başlığı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FrameHeader {
    len: usize,
    sample_rate: u32,
    channels: u16,
    samples_per_frame: u32,
    object_type: u8,
    kbps: u32,
}

impl FrameHeader {
    fn parse(word: u32) -> Option<Self> {
        // 11 bit senkron + Layer III (`01`).
        if word >> 21 != 0x7ff || (word >> 17) & 3 != 1 {
            return None;
        }
        let version = (word >> 19) & 3; // 3 = MPEG-1, 2 = MPEG-2, 0 = MPEG-2.5, 1 = ayrılmış
        let bitrate_idx = ((word >> 12) & 0xf) as usize;
        let rate_idx = ((word >> 10) & 3) as usize;
        if version == 1 || bitrate_idx == 0 || bitrate_idx == 15 || rate_idx == 3 {
            return None;
        }
        const V1: [u32; 15] = [
            0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
        ];
        const V2: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
        let (kbps, sample_rate, samples_per_frame, object_type) = match version {
            3 => (V1[bitrate_idx], [44100, 48000, 32000][rate_idx], 1152, 0x6B),
            2 => (V2[bitrate_idx], [22050, 24000, 16000][rate_idx], 576, 0x69),
            _ => (V2[bitrate_idx], [11025, 12000, 8000][rate_idx], 576, 0x69),
        };
        let padding = ((word >> 9) & 1) as usize;
        let channels = if (word >> 6) & 3 == 3 { 1 } else { 2 };
        let len = (samples_per_frame / 8 * kbps * 1000 / sample_rate) as usize + padding;
        Some(Self {
            len,
            sample_rate,
            channels,
            samples_per_frame,
            object_type,
            kbps,
        })
    }

    fn same_format(&self, other: &Self) -> bool {
        self.sample_rate == other.sample_rate
            && self.channels == other.channels
            && self.object_type == other.object_type
    }
}

/// Tek video örneği (süresi = ses süresi) + MP3 örnekleri → faststart MP4.
fn mux(image: &Yuv420, video: StillFrame, audio: &Mp3, mp3: &[u8]) -> Result<Vec<u8>, String> {
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
                    object_type_indication: audio.object_type,
                    stream_type: DecoderConfigDescriptor::STREAM_TYPE_AUDIO,
                    up_stream: DecoderConfigDescriptor::UP_STREAM_FALSE,
                    buffer_size_db: Uint::new(0),
                    max_bitrate: audio.avg_bitrate,
                    avg_bitrate: audio.avg_bitrate,
                    dec_specific_info: None,
                },
                sl_config_descr: SlConfigDescriptor,
            },
        },
        unknown_boxes: vec![],
    });

    let total_ms = (audio.total_samples() * 1000 / audio.sample_rate as u64) as u32;
    let options = Mp4FileMuxerOptions {
        reserved_moov_box_size: estimate_maximum_moov_box_size(&[1, audio.frames.len()]),
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
    for (index, &(start, len)) in audio.frames.iter().enumerate() {
        muxer
            .append_sample(&Sample {
                track_kind: TrackKind::Audio,
                sample_entry: (index == 0).then(|| audio_entry.clone()),
                keyframe: true,
                timescale,
                duration: audio.samples_per_frame,
                composition_time_offset: None,
                data_offset: out.len() as u64,
                data_size: len,
            })
            .map_err(mux_error)?;
        out.extend_from_slice(&mp3[start..start + len]);
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

    /// MPEG-1 Layer III, 128 kbps, 44.1 kHz, joint stereo, padding yok → 417 bayt.
    const HEADER_128K: u32 = 0xFFFB_9064;

    fn mp3_frames(count: usize) -> Vec<u8> {
        let mut data = Vec::new();
        for _ in 0..count {
            let mut frame = vec![0u8; 417];
            frame[..4].copy_from_slice(&HEADER_128K.to_be_bytes());
            data.extend_from_slice(&frame);
        }
        data
    }

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
    fn mpeg1_layer3_header_is_parsed() {
        let header = FrameHeader::parse(HEADER_128K).expect("geçerli başlık");
        assert_eq!(header.len, 417);
        assert_eq!(header.sample_rate, 44100);
        assert_eq!(header.channels, 2);
        assert_eq!(header.samples_per_frame, 1152);
        assert_eq!(header.object_type, 0x6B);
        // Layer II ve geçersiz bitrate reddedilir.
        assert!(FrameHeader::parse(0xFFFD_9064).is_none());
        assert!(FrameHeader::parse(0xFFFB_F064).is_none());
    }

    #[test]
    fn id3_tag_and_trailing_garbage_are_skipped() {
        let mut data = b"ID3\x04\x00\x00\x00\x00\x00\x05hello".to_vec();
        data.extend(mp3_frames(3));
        data.extend_from_slice(b"TAGtrailing");

        let mp3 = parse_mp3(&data).expect("kareler bulunmalı");
        assert_eq!(mp3.frames, vec![(15, 417), (432, 417), (849, 417)]);
        assert_eq!(mp3.avg_bitrate, 128_000);
        assert_eq!(mp3.total_samples(), 3 * 1152);
        assert!(parse_mp3(b"not an mp3").is_err());
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
    fn muxed_file_is_faststart_mp4_with_both_tracks() {
        let image = solid_image(64, 64);
        let still = encode_still(&image).expect("kodlanmalı");
        let mp3 = mp3_frames(10);
        let audio = parse_mp3(&mp3).expect("kareler bulunmalı");

        let out = mux(&image, still, &audio, &mp3).expect("MP4 yazılmalı");
        assert_eq!(&out[4..8], b"ftyp");
        let moov = out.windows(4).position(|w| w == b"moov").expect("moov");
        let mdat = out.windows(4).position(|w| w == b"mdat").expect("mdat");
        assert!(moov < mdat, "moov başta olmalı (faststart)");
        assert!(out.windows(4).any(|w| w == b"avc1"));
        assert!(out.windows(4).any(|w| w == b"mp4a"));
        // Ses kareleri yeniden kodlanmadan aynen yazılır.
        assert!(out.windows(mp3.len()).any(|w| w == mp3.as_slice()));
    }
}
