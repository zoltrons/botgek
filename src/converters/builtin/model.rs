//! Platform adapters share these data types; no I/O, settings or Discord client lives here.
use crate::converters::parser::Platform;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Author {
    pub name: String,
    /// Kullanıcı adı, @ işareti olmadan.
    pub handle: String,
    pub avatar: Option<String>,
    pub verified: bool,
    pub profile_url: Option<String>,
}

impl Author {
    pub fn new(name: impl Into<String>, handle: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            handle: handle.into(),
            avatar: None,
            verified: false,
            profile_url: None,
        }
    }
    #[cfg(test)]
    pub fn label(&self) -> String {
        if self.handle.is_empty() {
            self.name.clone()
        } else {
            format!("{} (@{})", self.name, self.handle)
        }
    }
}

/// Etkileşim metrikleri. Reddit'in skoru mevcut `likes` alanında taşınır.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub replies: u64,
    pub reposts: u64,
    pub likes: u64,
    pub quotes: u64,
    pub views: Option<u64>,
}
impl Stats {
    pub fn is_empty(&self) -> bool {
        self.replies == 0
            && self.reposts == 0
            && self.likes == 0
            && self.quotes == 0
            && self.views.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
    Gif,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Media {
    pub kind: MediaKind,
    /// Doğrudan görsel/mp4/HLS adresi.
    pub url: String,
    pub poster: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Saniye.
    pub duration: Option<u64>,
}
impl Media {
    pub fn is_video(&self) -> bool {
        matches!(self.kind, MediaKind::Video | MediaKind::Gif)
    }
    #[cfg(test)]
    pub fn is_image(&self) -> bool {
        matches!(self.kind, MediaKind::Image)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Translation {
    /// Kaynak dil kodu; platform bildirmediyse None.
    pub source: Option<String>,
}

/// Medyasız gönderide public hedef sayfanın önizlemesi. Görsel indirilmez.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkPreview {
    /// DNS ve yönlendirme doğrulamasından geçen son URL; og:url güvenilmez.
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
    pub site_name: Option<String>,
}
impl LinkPreview {
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.description.is_none() && self.image.is_none()
    }
}

/// Spotify gibi platformların hazır V2 container içeriği.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomCard {
    pub components: Vec<serde_json::Value>,
    pub accent_color: Option<u32>,
    /// Gönderimde video eki üretilebildiğinde `components` yerine kullanılan düzen (ör. Spotify:
    /// kapak videoda göründüğü için küçük görselsiz, kompakt başlık). Video ilk bileşenin
    /// altına eklenir. `None`: video olsa da `components` kullanılır.
    pub video_components: Option<Vec<serde_json::Value>>,
}

/// Render katmanının platformdan bağımsız girdisi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinPost {
    pub platform: Platform,
    pub url: String,
    pub author: Author,
    pub title: Option<String>,
    pub text: String,
    /// Unix saniye.
    pub created_at: Option<i64>,
    pub stats: Stats,
    pub media: Vec<Media>,
    /// Adapter tarafından sınırlandırılan alıntı zinciri.
    pub quote: Option<Box<BuiltinPost>>,
    pub community_note: Option<String>,
    /// Kaynağın açıkça spoiler olarak işaretlediği içerik (Reddit `spoiler`).
    /// NSFW kanalda bile hiçbir zaman otomatik olarak açılmaz.
    pub source_spoiler: bool,
    pub sensitive: bool,
    pub extra_label: Option<String>,
    pub language: Option<String>,
    pub translation: Option<Translation>,
    pub custom_card: Option<CustomCard>,
    pub link_preview: Option<LinkPreview>,
}

impl BuiltinPost {
    pub fn new(platform: Platform, url: impl Into<String>, author: Author) -> Self {
        Self {
            platform,
            url: url.into(),
            author,
            title: None,
            text: String::new(),
            created_at: None,
            stats: Stats::default(),
            media: Vec::new(),
            quote: None,
            community_note: None,
            source_spoiler: false,
            sensitive: false,
            extra_label: None,
            language: None,
            translation: None,
            custom_card: None,
            link_preview: None,
        }
    }
}
