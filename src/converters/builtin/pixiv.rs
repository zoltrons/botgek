//! Public Pixiv artwork previews; title text is never interpreted as a user ID.
use super::{BuiltinPost, preview};
use crate::converters::parser::{ParsedPost, Platform};

pub async fn fetch(_client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    preview::fetch(parsed, Platform::Pixiv).await
}
