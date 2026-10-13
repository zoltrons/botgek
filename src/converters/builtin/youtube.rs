//! Public YouTube Community previews; player and HLS URLs are not direct-file media.
use super::{BuiltinPost, preview};
use crate::converters::parser::{ParsedPost, Platform};

pub async fn fetch(_client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    preview::fetch(parsed, Platform::YouTube).await
}
