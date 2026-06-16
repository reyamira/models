//! Async HTTP client for fetching per-source benchmark data, with a multi-host
//! fallback chain.
//!
//! `fetch_source` pulls a single source's v2 [`SourceFile`] for the benchmarks
//! tab's data-source switcher, trying a chain of hosts so a single jsDelivr edge
//! or branch-resolution hiccup can't take the whole tab down (see the
//! 2026-06-16 jsDelivr outage that motivated this).

use std::time::Duration;

use super::schema::SourceFile;
use super::sources::{SourceDescriptor, DATA_REF, DATA_REPO};

/// Ordered list of candidate URLs to try for a source, most- to least-preferred.
///
/// When `MODELS_DATA_BASE_URL` is set and non-empty, the chain is bypassed
/// entirely and a single `{base}/{id}.json` URL is returned — a sanctioned dev
/// override for serving data files from a local directory or staging host.
///
/// Otherwise the chain is:
/// 1. `cdn.jsdelivr.net@{ref}` — primary (fast, multi-CDN).
/// 2. `fastly.jsdelivr.net@{ref}` — a warm-cache shortcut on jsDelivr's Fastly
///    edge. NOT an independent source: every jsDelivr edge sits in front of the
///    same branch resolver, so this only rescues *short* outages where a Fastly
///    cache copy outlives the resolver failure. Cheap to try, so it stays.
/// 3. `raw.githubusercontent.com/{ref}` — the real backstop: bypasses jsDelivr's
///    resolution layer entirely and reads the branch HEAD straight from GitHub.
///    Different URL shape (no `/gh/`, `/{ref}` not `@{ref}`).
fn candidate_urls(desc: &SourceDescriptor) -> Vec<String> {
    if let Ok(base) = std::env::var("MODELS_DATA_BASE_URL") {
        if !base.is_empty() {
            let base = base.trim_end_matches('/');
            return vec![format!("{base}/{}.json", desc.id)];
        }
    }

    let path = desc.data_path;
    vec![
        format!("https://cdn.jsdelivr.net/gh/{DATA_REPO}@{DATA_REF}/{path}"),
        format!("https://fastly.jsdelivr.net/gh/{DATA_REPO}@{DATA_REF}/{path}"),
        format!("https://raw.githubusercontent.com/{DATA_REPO}/{DATA_REF}/{path}"),
    ]
}

/// Fetch and deserialize one source's v2 `SourceFile`, trying each candidate
/// host in order and returning the first success.
///
/// Returns `None` only when every candidate fails (network failure, non-2xx
/// status, or parse error). No error payload is carried — keeping the failure
/// path data-free avoids the clippy unused-field lint and matches the
/// `MultiStore::set_failed` contract.
///
/// The client caps redirects low (`limited(2)`) so a jsDelivr 301-redirect loop
/// fast-fails into the next candidate instead of burning the default 10 hops;
/// healthy jsDelivr serves `200` with no redirect. A request timeout fast-fails
/// a hung connection the same way.
pub async fn fetch_source(desc: &SourceDescriptor) -> Option<SourceFile> {
    let client = reqwest::Client::builder()
        .user_agent("models-tui")
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(2))
        .build()
        .ok()?;

    for url in candidate_urls(desc) {
        let Ok(response) = client.get(&url).send().await else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        if let Ok(file) = response.json::<SourceFile>().await {
            return Some(file);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmarks::sources::SOURCES;

    fn source(id: &str) -> &'static SourceDescriptor {
        SOURCES.iter().find(|s| s.id == id).expect("source present")
    }

    // Both assertions live in one test on purpose: `MODELS_DATA_BASE_URL` is
    // process-global and cargo runs tests in parallel threads, so splitting the
    // "no override" and "override" cases into separate tests would let them race
    // on the env var. Keeping them sequential here is deterministic.
    #[test]
    fn candidate_urls_chain_and_override() {
        let prev = std::env::var("MODELS_DATA_BASE_URL").ok();

        // No override: cdn → fastly → raw (raw has a distinct shape — no `/gh/`,
        // `/main` not `@main`).
        std::env::remove_var("MODELS_DATA_BASE_URL");
        assert_eq!(
            candidate_urls(source("aa")),
            vec![
                "https://cdn.jsdelivr.net/gh/reyamira/models@main/data/v2/aa.json".to_string(),
                "https://fastly.jsdelivr.net/gh/reyamira/models@main/data/v2/aa.json".to_string(),
                "https://raw.githubusercontent.com/reyamira/models/main/data/v2/aa.json"
                    .to_string(),
            ]
        );

        // Override set: single URL, chain bypassed.
        std::env::set_var("MODELS_DATA_BASE_URL", "http://localhost:8080/data/");
        assert_eq!(
            candidate_urls(source("epoch")),
            vec!["http://localhost:8080/data/epoch.json".to_string()]
        );

        match prev {
            Some(v) => std::env::set_var("MODELS_DATA_BASE_URL", v),
            None => std::env::remove_var("MODELS_DATA_BASE_URL"),
        }
    }

    /// Live integration probe — hits the real network, so it is `#[ignore]`d and
    /// never runs in CI / `mise run test`. Run manually with
    /// `cargo test --bin models -- --ignored fetch_source_falls_through` to
    /// confirm the fallback chain delivers data even when the primary CDN tier
    /// is failing (it falls through to Fastly / GitHub raw).
    #[tokio::test]
    #[ignore]
    async fn fetch_source_falls_through_to_a_working_host() {
        std::env::remove_var("MODELS_DATA_BASE_URL");
        let file = fetch_source(source("aa")).await;
        assert!(
            file.is_some(),
            "expected the fallback chain to deliver aa.json from some host"
        );
    }
}
