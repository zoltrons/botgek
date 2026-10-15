//! X (Twitter) gönderilerinin otomatik Türkçe çevirisi için dil politikası.
//!
//! Çeviri, **X'in kendi çeviri servisi** ile yapılır: FxTwitter'ın dil ekli ucu
//! (`api.fxtwitter.com/i/status/{id}/{dil}`) yanıtta `translation` alanını döndürür
//! (sağlayıcı X'in çevirisi, ör. `provider: "grok"`). Bu modül yalnızca "hangi gönderi
//! çevrilir" kararını verir; ağ isteği `twitter` modülündedir.
//!
//! Türkçe (`tr`) ve İngilizce (`en`) gönderiler olduğu gibi bırakılır.

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
/// Gönderi dili hedef dilden farklıysa X'in çeviri uç noktasına gidilmelidir.
pub fn needs_translation(language: Option<&str>, target: &str) -> bool {
    let target = normalize_language(Some(target));
    match normalize_language(language) {
        Some(lang) => {
            target.as_deref() != Some(lang.as_str())
                // Preserve the existing default policy: Turkish auto-translation does not
                // rewrite English posts. A different configured target may still translate
                // English (for example, English -> German).
                && !(target.as_deref() == Some("tr") && lang == "en")
        }
        None => false,
    }
}

/// Yaygın dil kodları → Türkçe dil adı (kart alt bilgisindeki "…'dan çevrildi" notu için).
///
/// X'in `source_lang` alanı BCP-47 birincil kodu verir (`ja`, `de`, `zh-Hans` → `zh`).
/// Tabloda olmayan kod için ad **uydurulmaz**: çağıran ham kodu gösterir.
const TURKISH_LANGUAGE_NAMES: &[(&str, &str)] = &[
    ("ar", "Arapça"),
    ("az", "Azerice"),
    ("be", "Belarusça"),
    ("bg", "Bulgarca"),
    ("bn", "Bengalce"),
    ("bs", "Boşnakça"),
    ("ca", "Katalanca"),
    ("cs", "Çekçe"),
    ("da", "Danca"),
    ("de", "Almanca"),
    ("el", "Yunanca"),
    ("en", "İngilizce"),
    ("es", "İspanyolca"),
    ("et", "Estonca"),
    ("eu", "Baskça"),
    ("fa", "Farsça"),
    ("fi", "Fince"),
    ("fr", "Fransızca"),
    ("ga", "İrlandaca"),
    ("gl", "Galiçyaca"),
    ("he", "İbranice"),
    ("hi", "Hintçe"),
    ("hr", "Hırvatça"),
    ("hu", "Macarca"),
    ("hy", "Ermenice"),
    ("id", "Endonezce"),
    ("is", "İzlandaca"),
    ("it", "İtalyanca"),
    ("ja", "Japonca"),
    ("ka", "Gürcüce"),
    ("kk", "Kazakça"),
    ("km", "Kmerce"),
    ("ko", "Korece"),
    ("ku", "Kürtçe"),
    ("ky", "Kırgızca"),
    ("lo", "Laoca"),
    ("lt", "Litvanca"),
    ("lv", "Letonca"),
    ("mk", "Makedonca"),
    ("ml", "Malayalamca"),
    ("mn", "Moğolca"),
    ("mr", "Marathice"),
    ("ms", "Malayca"),
    ("mt", "Maltaca"),
    ("my", "Birmanca"),
    ("ne", "Nepalce"),
    ("nl", "Felemenkçe"),
    ("no", "Norveççe"),
    ("pa", "Pencapça"),
    ("pl", "Lehçe"),
    ("ps", "Peştuca"),
    ("pt", "Portekizce"),
    ("ro", "Rumence"),
    ("ru", "Rusça"),
    ("si", "Seylanca"),
    ("sk", "Slovakça"),
    ("sl", "Slovence"),
    ("so", "Somalice"),
    ("sq", "Arnavutça"),
    ("sr", "Sırpça"),
    ("sv", "İsveççe"),
    ("sw", "Svahili"),
    ("ta", "Tamilce"),
    ("te", "Teluguca"),
    ("tg", "Tacikçe"),
    ("th", "Tayca"),
    ("tl", "Tagalogca"),
    ("tk", "Türkmence"),
    ("tr", "Türkçe"),
    ("uk", "Ukraynaca"),
    ("ur", "Urduca"),
    ("uz", "Özbekçe"),
    ("vi", "Vietnamca"),
    ("yi", "Yidiş"),
    ("zh", "Çince"),
];

/// Dil kodunu Türkçe adına çevirir (`ja` → `Japonca`); kod tanınmıyorsa `None`.
pub fn turkish_language_name(language: &str) -> Option<&'static str> {
    let code = normalize_language(Some(language))?;
    TURKISH_LANGUAGE_NAMES
        .iter()
        .find(|(candidate, _)| *candidate == code)
        .map(|(_, name)| *name)
}

/// Kart alt bilgisindeki Türkçe çeviri notu: `Japonca'dan çevrildi`.
///
/// Kaynak dili bilinmiyorsa yalnızca `Çevrildi` yazılır; kod tanınmıyorsa ham kod korunur
/// (ad uydurulmaz) ve ek yine ünlü uyumuna göre seçilir.
pub fn translated_from_note(language: Option<&str>) -> String {
    let Some(raw) = language.map(str::trim).filter(|value| !value.is_empty()) else {
        return "Çevrildi".to_string();
    };
    // Belirsiz/geçersiz kod (`und`, `zxx`, `12`) için ad uydurulmaz: yalnızca "Çevrildi".
    let Some(normalized) = normalize_language(Some(raw)) else {
        return "Çevrildi".to_string();
    };
    let label = turkish_language_name(&normalized).unwrap_or(normalized.as_str());
    format!("{label}{} çevrildi", ablative_suffix(label))
}

/// Türkçe ünlü uyumuna göre ablatif eki (`-dan/-den/-don/-dön`, kesme işaretli).
///
/// Dil adları `…ca`/`…ce` bittiği için pratikte `'dan`/`'den` olur; fonksiyon yine de son
/// ünlüye göre karar verir, böylece tanınmayan ham kodlar da doğru eki alır.
fn ablative_suffix(word: &str) -> &'static str {
    match word.chars().rev().find_map(turkish_vowel) {
        Some('a') | Some('ı') => "'dan",
        Some('e') | Some('i') => "'den",
        Some('o') | Some('u') => "'don",
        Some('ö') | Some('ü') => "'dön",
        _ => "'den",
    }
}

/// Türkçe ünlüyü küçük harfe indirger (`İ`/`I` dahil); ünsüzde `None`.
fn turkish_vowel(ch: char) -> Option<char> {
    match ch.to_lowercase().next()? {
        'a' => Some('a'),
        'e' => Some('e'),
        'ı' => Some('ı'),
        'i' => Some('i'),
        'o' => Some('o'),
        'ö' => Some('ö'),
        'u' => Some('u'),
        'ü' => Some('ü'),
        _ => None,
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
        assert!(!needs_translation(Some("tr"), "tr"));
        assert!(!needs_translation(Some("en-US"), "en"));
        assert!(!needs_translation(Some("TR"), "tr"));
        assert!(!needs_translation(Some("und"), "tr"));
        assert!(!needs_translation(None, "tr"));
        // Diğer diller çevrilir
        assert!(needs_translation(Some("de"), "tr"));
        assert!(needs_translation(Some("ja"), "tr"));
        assert!(needs_translation(Some("ru"), "tr"));
        assert!(needs_translation(Some("ar-SA"), "tr"));
    }

    #[test]
    fn common_language_codes_have_turkish_names() {
        assert_eq!(turkish_language_name("ja"), Some("Japonca"));
        assert_eq!(turkish_language_name("de-DE"), Some("Almanca"));
        assert_eq!(turkish_language_name("PT"), Some("Portekizce"));
        assert_eq!(turkish_language_name("zh-Hans"), Some("Çince"));
        // Bilinmeyen/belirsiz kodda ad uydurulmaz.
        assert_eq!(turkish_language_name("xx"), None);
        assert_eq!(turkish_language_name("und"), None);
    }

    #[test]
    fn translation_note_is_turkish_with_vowel_harmony() {
        assert_eq!(translated_from_note(Some("ja")), "Japonca'dan çevrildi");
        assert_eq!(translated_from_note(Some("de")), "Almanca'dan çevrildi");
        assert_eq!(translated_from_note(Some("pt")), "Portekizce'den çevrildi");
        assert_eq!(translated_from_note(Some("en")), "İngilizce'den çevrildi");
        assert_eq!(translated_from_note(Some("ru")), "Rusça'dan çevrildi");
        assert_eq!(translated_from_note(Some("zh")), "Çince'den çevrildi");
        assert_eq!(translated_from_note(Some("sw")), "Svahili'den çevrildi");
        // Tanınmayan ama geçerli kod: ham kod korunur, ek ünlü uyumuna göre seçilir.
        assert_eq!(translated_from_note(Some("xyz")), "xyz'den çevrildi");
        // Belirsiz/boş kaynak: yalnızca "Çevrildi".
        assert_eq!(translated_from_note(Some("und")), "Çevrildi");
        assert_eq!(translated_from_note(Some("   ")), "Çevrildi");
        assert_eq!(translated_from_note(None), "Çevrildi");
    }
}
