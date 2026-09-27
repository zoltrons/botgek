//! X (Twitter) gönderilerinin otomatik Türkçe çevirisi için dil politikası.
//!
//! Çeviri, **X'in kendi çeviri servisi** ile yapılır: FxTwitter'ın dil ekli ucu
//! (`api.fxtwitter.com/i/status/{id}/{dil}`) yanıtta `translation` alanını döndürür
//! (sağlayıcı X'in çevirisi, ör. `provider: "grok"`). Bu modül yalnızca "hangi gönderi
//! çevrilir" kararını verir; ağ isteği `twitter` modülündedir.
//!
//! Türkçe (`tr`) ve İngilizce (`en`) gönderiler olduğu gibi bırakılır.

/// X'in çeviri servisinden istenen hedef dil kodu.
pub const TARGET_LANGUAGE_CODE: &str = "tr";
/// Bu dillerdeki gönderiler çevrilmez: metin zaten Türkçe veya İngilizce.
pub const SKIP_LANGUAGE_CODES: [&str; 2] = ["tr", "en"];
/// Dilin belirlenemediği/çeviriye uygun olmadığı kodlar (X'in sözde dilleri dahil).
const UNKNOWN_LANGUAGE_CODES: [&str; 7] = ["und", "zxx", "mul", "qme", "qam", "qht", "art"];

/// Dil kodunu sadeleştirir: `tr-TR` / `en_US` → `tr` / `en`; belirsiz kodlar → `None`.
pub fn normalize_language(language: Option<&str>) -> Option<String> {
    let raw = language?.trim();
    if raw.is_empty() {
        return None;
    }
    let primary = raw
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if primary.is_empty()
        || !primary.chars().all(|c| c.is_ascii_alphabetic())
        || UNKNOWN_LANGUAGE_CODES.contains(&primary.as_str())
    {
        return None;
    }
    Some(primary)
}

/// Gönderi Türkçeye çevrilmeli mi? Türkçe/İngilizce ve dili belirsiz gönderiler için `false`.
pub fn needs_turkish_translation(language: Option<&str>) -> bool {
    match normalize_language(language) {
        Some(lang) => !SKIP_LANGUAGE_CODES.contains(&lang.as_str()),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_codes_are_normalized() {
        assert_eq!(normalize_language(Some("TR")).as_deref(), Some("tr"));
        assert_eq!(normalize_language(Some("de-DE")).as_deref(), Some("de"));
        assert_eq!(normalize_language(Some("en_US")).as_deref(), Some("en"));
        assert_eq!(normalize_language(Some("  fr  ")).as_deref(), Some("fr"));
        assert_eq!(normalize_language(Some("ja")).as_deref(), Some("ja"));
    }

    #[test]
    fn undetermined_languages_are_treated_as_unknown() {
        assert_eq!(normalize_language(Some("und")), None);
        assert_eq!(normalize_language(Some("qme")), None);
        assert_eq!(normalize_language(Some("")), None);
        assert_eq!(normalize_language(None), None);
        assert_eq!(normalize_language(Some("12")), None);
    }

    #[test]
    fn only_foreign_languages_need_translation() {
        // Türkçe ve İngilizce gönderiler olduğu gibi bırakılır
        assert!(!needs_turkish_translation(Some("tr")));
        assert!(!needs_turkish_translation(Some("en-US")));
        assert!(!needs_turkish_translation(Some("TR")));
        assert!(!needs_turkish_translation(Some("und")));
        assert!(!needs_turkish_translation(None));
        // Diğer diller çevrilir
        assert!(needs_turkish_translation(Some("de")));
        assert!(needs_turkish_translation(Some("ja")));
        assert!(needs_turkish_translation(Some("ru")));
        assert!(needs_turkish_translation(Some("ar-SA")));
    }
}
