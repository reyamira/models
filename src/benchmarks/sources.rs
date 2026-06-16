//! Compile-time registry of benchmark data sources.
//!
//! Each [`SourceDescriptor`] describes one self-contained data lens: its CDN
//! data URL (a v2 [`super::schema::SourceFile`]), attribution link, and whether
//! the data is third-party verified.
//!
//! Phases 3-5 of the multi-source plan append `epoch` / `arena` / `llmstats`
//! entries here. This is a sanctioned deviation from plan section 4's "4
//! entries": entries land together with their committed data files so the UI
//! never renders a source whose data URL would 404.

/// Static description of one benchmark data source.
pub struct SourceDescriptor {
    /// Stable identifier (also the data filename stem under `data/v2/`).
    pub id: &'static str,
    /// Human-readable display name (shown in the source bar).
    pub name: &'static str,
    /// Attribution link to the source's website. Consumed by `model_url`'s
    /// unknown-source fallback arm; the four known sources deliberately
    /// hardcode their model-page hosts instead (see `model_url`), so changing
    /// this field does not repoint their opened pages.
    pub url: &'static str,
    /// Repo-relative path of the source's v2 `SourceFile` JSON (e.g.
    /// `data/v2/aa.json`). `fetch.rs` builds the multi-host fallback chain
    /// (jsDelivr CDN → Fastly edge → GitHub raw) from [`DATA_REPO`]/[`DATA_REF`]
    /// plus this path. Stored as coordinates rather than a full URL because the
    /// GitHub-raw tier has a different URL shape than the jsDelivr tiers (no
    /// `/gh/` segment, and `/{ref}` instead of `@{ref}`) that string-rewriting a
    /// CDN URL can't produce cleanly.
    pub data_path: &'static str,
    /// `true` when the data is third-party verified; `false` renders a
    /// "self-reported" badge.
    // TODO(phase-3+): verification is currently read from the data file's
    // `SourceMeta.verified`; this compile-time field is part of the binding
    // contract for sources whose verification is known before any data lands.
    #[allow(dead_code)]
    pub verified: bool,
}

impl SourceDescriptor {
    /// Per-source URL for a model's page, given the source's model id.
    ///
    /// The naive `{url}/models/{id}` form 404s on Epoch and Arena (verified live
    /// 2026-06-11), so each source gets a hand-tuned strategy:
    /// - `aa` / `llmstats` — straightforward `/models/{id}`.
    /// - `epoch` — the slug is the last path segment of the id, lowercased with
    ///   `.` → `-` (e.g. `zai-org/GLM-4-7` → `glm-4-7`). ~70% of Epoch ids
    ///   resolve this way (≈100% of frontier models); the caller falls back to
    ///   the model index page on a 404.
    /// - `arena` — no per-model pages exist (`/models/{id}`, `/model/{id}`,
    ///   `/models` all 404), so every model points at the text leaderboard.
    pub fn model_url(&self, model_id: &str) -> String {
        match self.id {
            "aa" => format!("https://artificialanalysis.ai/models/{model_id}"),
            "llmstats" => format!("https://llm-stats.com/models/{model_id}"),
            "epoch" => {
                let slug = model_id
                    .rsplit('/')
                    .next()
                    .unwrap_or(model_id)
                    .to_lowercase()
                    .replace('.', "-");
                format!("https://epoch.ai/models/{slug}")
            }
            "arena" => "https://arena.ai/leaderboard/text".to_string(),
            // Unknown source: fall back to the naive form against the attribution
            // URL so a future source still produces something openable.
            _ => format!("{}/models/{model_id}", self.url),
        }
    }
}

/// GitHub `owner/repo` the v2 data files live in. Compiled in (like the
/// `model_url` host hardcoding) so `fetch.rs` can build every fallback-tier URL.
pub const DATA_REPO: &str = "reyamira/models";
/// Git ref the v2 data files are served from — the benchmark bot commits fresh
/// data to this branch every ~30 min, so it must track the branch HEAD (not a
/// frozen tag).
pub const DATA_REF: &str = "main";

/// Compiled-in list of all known data sources. Order is display order.
pub const SOURCES: &[SourceDescriptor] = &[
    SourceDescriptor {
        id: "aa",
        name: "Artificial Analysis",
        url: "https://artificialanalysis.ai",
        data_path: "data/v2/aa.json",
        verified: true,
    },
    SourceDescriptor {
        id: "epoch",
        name: "Epoch AI",
        url: "https://epoch.ai",
        data_path: "data/v2/epoch.json",
        verified: true,
    },
    SourceDescriptor {
        id: "arena",
        name: "Arena",
        url: "https://arena.ai",
        data_path: "data/v2/arena.json",
        verified: true,
    },
    SourceDescriptor {
        id: "llmstats",
        name: "LLM Stats",
        url: "https://llm-stats.com",
        data_path: "data/v2/llmstats.json",
        // Aggregates third-party benchmark results; its methodology excludes
        // provider self-reported numbers from the ingested rankings, so it is
        // verified like the others (plan amendment 2026-06-11).
        verified: true,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn source(id: &str) -> &'static SourceDescriptor {
        SOURCES.iter().find(|s| s.id == id).expect("source present")
    }

    #[test]
    fn model_url_aa_uses_models_path() {
        assert_eq!(
            source("aa").model_url("gpt-5"),
            "https://artificialanalysis.ai/models/gpt-5"
        );
    }

    #[test]
    fn model_url_llmstats_uses_models_path() {
        assert_eq!(
            source("llmstats").model_url("claude-opus-4"),
            "https://llm-stats.com/models/claude-opus-4"
        );
    }

    #[test]
    fn model_url_arena_always_text_leaderboard() {
        // No per-model pages exist; every id collapses to the leaderboard.
        assert_eq!(
            source("arena").model_url("anything-at-all"),
            "https://arena.ai/leaderboard/text"
        );
    }

    #[test]
    fn model_url_epoch_strips_org_prefix() {
        // `org/Name` → last segment, lowercased.
        assert_eq!(
            source("epoch").model_url("zai-org/GLM-4-7"),
            "https://epoch.ai/models/glm-4-7"
        );
    }

    #[test]
    fn model_url_epoch_lowercases_and_dots_to_dashes() {
        assert_eq!(
            source("epoch").model_url("DeepSeek-V3.1"),
            "https://epoch.ai/models/deepseek-v3-1"
        );
    }

    #[test]
    fn model_url_epoch_plain_id_no_prefix() {
        assert_eq!(
            source("epoch").model_url("Claude-Opus-4"),
            "https://epoch.ai/models/claude-opus-4"
        );
    }
}
