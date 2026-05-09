//! Async HTTP client for fetching benchmark data from jsDelivr CDN.

use super::BenchmarkEntry;

const CDN_URL: &str = "https://cdn.jsdelivr.net/gh/reyamira/models@main/data/benchmarks.json";

/// Result of a fetch operation.
#[derive(Debug)]
pub enum BenchmarkFetchResult {
    /// New data fetched successfully.
    Fresh(Vec<BenchmarkEntry>),
    /// Fetch failed.
    Error,
}

pub struct BenchmarkFetcher {
    client: reqwest::Client,
}

impl BenchmarkFetcher {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent("models-tui")
            .build()
            .expect("Failed to build HTTP client");

        Self { client }
    }

    /// Fetch benchmark data from CDN.
    pub async fn fetch(&self) -> BenchmarkFetchResult {
        let response = match self.client.get(CDN_URL).send().await {
            Ok(resp) => resp,
            Err(_) => return BenchmarkFetchResult::Error,
        };

        if !response.status().is_success() {
            return BenchmarkFetchResult::Error;
        }

        let entries: Vec<BenchmarkEntry> = match response.json().await {
            Ok(e) => e,
            Err(_) => return BenchmarkFetchResult::Error,
        };

        BenchmarkFetchResult::Fresh(entries)
    }
}
