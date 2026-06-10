//! Async HTTP client for fetching per-source benchmark data from jsDelivr CDN.
//!
//! `fetch_source` pulls a single source's v2 [`SourceFile`] for the benchmarks
//! tab's data-source switcher.

use super::schema::SourceFile;
use super::sources::SourceDescriptor;

/// Resolve the fetch URL for a source descriptor.
///
/// Uses `desc.data_url` unless `MODELS_DATA_BASE_URL` is set, in which case the
/// URL is `{base}/{id}.json` — a sanctioned dev override for serving data files
/// from a local directory or staging host.
fn source_url(desc: &SourceDescriptor) -> String {
    match std::env::var("MODELS_DATA_BASE_URL") {
        Ok(base) if !base.is_empty() => {
            let base = base.trim_end_matches('/');
            format!("{base}/{}.json", desc.id)
        }
        _ => desc.data_url.to_string(),
    }
}

/// Fetch and deserialize one source's v2 `SourceFile`.
///
/// Returns `None` on any error (network failure, non-2xx status, or parse
/// error). No error payload is carried — keeping the failure path data-free
/// avoids the clippy unused-field lint and matches the `MultiStore::set_failed`
/// contract.
pub async fn fetch_source(desc: &SourceDescriptor) -> Option<SourceFile> {
    let client = reqwest::Client::builder()
        .user_agent("models-tui")
        .build()
        .ok()?;

    let url = source_url(desc);
    let response = client.get(&url).send().await.ok()?;

    if !response.status().is_success() {
        return None;
    }

    response.json::<SourceFile>().await.ok()
}
