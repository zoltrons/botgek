//! Public Facebook post/video/reel previews; login walls leave the original message intact.
use super::{BuiltinPost, preview};
use crate::converters::parser::{ParsedPost, Platform};

pub async fn fetch(_client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    preview::fetch(parsed, Platform::Facebook).await
}
