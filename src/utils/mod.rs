pub mod mentions;

/// Discord rejects any message whose content exceeds this many characters.
pub const DISCORD_CONTENT_LIMIT: usize = 2000;

/// Gizli değerlerin (`ADMIN_TOKEN`, `MCP_TOKEN`) en büyük uzunluğu.
pub const MAX_SECRET_LEN: usize = 512;

/// `secret` ile `candidate`'i hem içerikten hem uzunluktan bağımsız sabit sürede karşılaştırır.
///
/// Her iki değer de [`MAX_SECRET_LEN`] baytlık tampona doldurulur; uzunluk ayrıca sabit zamanlı
/// karşılaştırılır, böylece tamponu aşan bir aday da eşleşmez.
pub fn secret_matches(secret: &str, candidate: &str) -> bool {
    use subtle::ConstantTimeEq;
    fn padded(value: &[u8]) -> [u8; MAX_SECRET_LEN] {
        let mut buffer = [0u8; MAX_SECRET_LEN];
        let length = value.len().min(MAX_SECRET_LEN);
        buffer[..length].copy_from_slice(&value[..length]);
        buffer
    }
    let (secret, candidate) = (secret.as_bytes(), candidate.as_bytes());
    let same_length = (secret.len() as u64).ct_eq(&(candidate.len() as u64));
    let same_bytes = padded(secret).ct_eq(&padded(candidate));
    bool::from(same_length & same_bytes)
}

/// Standart (dolgulu) base64 kodlayıcı; ek bağımlılık gerektirmez.
pub fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0] as u32;
        let second = chunk.get(1).copied().unwrap_or(0) as u32;
        let third = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (first << 16) | (second << 8) | third;
        out.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((triple >> 6) & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(triple & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Truncates to `max` characters (not bytes) on a `char` boundary and marks the
/// cut with an ellipsis.
/// Metni `max` **karaktere** kısaltır; kısaltıldıysa sonuna `…` ekler.
///
/// Hızlı yol: bayt uzunluğu sınırı aşmıyorsa hiç tarama yapılmaz (karakter sayısı bayt
/// sayısından büyük olamaz) — kart bileşenleri her gönderimde bu fonksiyondan geçtiği için
/// asıl kazanç buradadır. Kısaltma yolunda metin **tek geçişte** (`char_indices`) kesilir.
pub fn truncate_chars(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if text.len() <= max || text.chars().count() <= max {
        return text.to_string();
    }
    let cut = text
        .char_indices()
        .nth(max - 1)
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    let mut out = text[..cut].to_string();
    // Sondaki boşlukları at, kesildiğini `…` ile belli et (toplam uzunluk `max`'ı aşmaz).
    out.truncate(out.trim_end().len());
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_is_untouched() {
        assert_eq!(truncate_chars("merhaba", 2000), "merhaba");
        assert_eq!(truncate_chars("", 10), "");
    }

    #[test]
    fn long_text_is_truncated_to_limit() {
        let result = truncate_chars(&"a".repeat(2500), 2000);
        assert_eq!(result.chars().count(), 2000);
        assert!(result.ends_with('…'));
    }

    #[test]
    fn truncation_never_splits_a_multibyte_char() {
        // "é" is two bytes; cutting by bytes here would panic or emit invalid UTF-8
        let result = truncate_chars(&"é".repeat(3000), 2000);
        assert_eq!(result.chars().count(), 2000);
        assert!(result.starts_with("ééé"));
    }

    #[test]
    fn truncation_does_not_end_on_trailing_whitespace() {
        assert_eq!(truncate_chars("abc  def", 5), "abc…");
    }
}
