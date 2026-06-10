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
    /// Attribution link to the source's website.
    // TODO(phase-3+): the UI currently reads attribution from the data file's
    // `SourceMeta.url`; this compile-time field is part of the binding contract
    // and is consumed once source-level (non-data-derived) links are surfaced.
    #[allow(dead_code)]
    pub url: &'static str,
    /// jsDelivr CDN URL of the source's v2 `SourceFile` JSON.
    pub data_url: &'static str,
    /// `true` when the data is third-party verified; `false` renders a
    /// "self-reported" badge.
    // TODO(phase-3+): verification is currently read from the data file's
    // `SourceMeta.verified`; this compile-time field is part of the binding
    // contract for sources whose verification is known before any data lands.
    #[allow(dead_code)]
    pub verified: bool,
}

/// Compiled-in list of all known data sources. Order is display order.
pub const SOURCES: &[SourceDescriptor] = &[
    SourceDescriptor {
        id: "aa",
        name: "Artificial Analysis",
        url: "https://artificialanalysis.ai",
        data_url: "https://cdn.jsdelivr.net/gh/reyamira/models@main/data/v2/aa.json",
        verified: true,
    },
    SourceDescriptor {
        id: "epoch",
        name: "Epoch AI",
        url: "https://epoch.ai",
        data_url: "https://cdn.jsdelivr.net/gh/reyamira/models@main/data/v2/epoch.json",
        verified: true,
    },
    SourceDescriptor {
        id: "arena",
        name: "Arena",
        url: "https://arena.ai",
        data_url: "https://cdn.jsdelivr.net/gh/reyamira/models@main/data/v2/arena.json",
        verified: true,
    },
    SourceDescriptor {
        id: "llmstats",
        name: "LLM Stats",
        url: "https://llm-stats.com",
        data_url: "https://cdn.jsdelivr.net/gh/reyamira/models@main/data/v2/llmstats.json",
        verified: false,
    },
];
