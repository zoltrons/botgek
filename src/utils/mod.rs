pub mod mentions;

/// Discord rejects any message whose content exceeds this many characters.
pub const DISCORD_CONTENT_LIMIT: usize = 2000;

/// Truncates to `max` characters (not bytes) on a `char` boundary and marks the
/// cut with an ellipsis.
pub fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
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
