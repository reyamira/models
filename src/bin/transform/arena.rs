//! `transform arena` — oolong-tea Arena leaderboard snapshot -> v2 `SourceFile`.
//!
//! Input is a directory holding one JSON per board, conforming to the upstream
//! `schemas/leaderboard.json`: `meta{leaderboard, source_url, fetched_at,
//! last_updated, model_count}` + `models[]{rank, model, vendor, license, score,
//! ci, votes}` (score/ci/votes/vendor/last_updated all nullable; license is
//! "proprietary" | "open" | null).
//!
//! Only the 6 LLM-relevant boards are ingested (media-gen boards are a plan
//! non-goal). Each board contributes one Elo metric; models are merged across
//! boards by EXACT model-name match (same scraper, consistent naming within a
//! snapshot). Mirrors `aa.rs` for structure, error handling, and the
//! if-changed (skip-write-when-only-`fetched_at`-differs) semantics.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::schema::{
    MetricDef, MetricKind, ModelRow, ReasoningStatus, ScoreCell, SourceFile, SourceMeta,
};

// ---------------------------------------------------------------------------
// Board registry (binding) — (filename stem, metric id, metric label).
// Display order = this order. All boards map to a single "Arena Elo" group:
// 6 axes = exactly one clean radar preset (>= 3 higher_is_better metrics).
// ---------------------------------------------------------------------------

/// One board: (file stem, metric id, metric label, description).
type BoardEntry = (&'static str, &'static str, &'static str, &'static str);

const BOARDS: &[BoardEntry] = &[
    (
        "text",
        "elo_text",
        "Text",
        "Arena's general text-chat board: humans pick the better of two blind side-by-side \
         responses across conversation, writing, and instruction-following. Score is an Elo \
         rating from those votes; higher is better.",
    ),
    (
        "vision",
        "elo_vision",
        "Vision",
        "Human-preference ranking for image and visual understanding: voters compare two \
         models answering image-based queries. Score is an Elo rating from those votes; \
         higher is better.",
    ),
    (
        "code",
        "elo_code",
        "Code",
        "Human-preference ranking for code generation and web development: voters compare two \
         models' programming outputs side by side. Score is an Elo rating from those votes; \
         higher is better.",
    ),
    (
        "agent",
        "elo_agent",
        "Agent",
        "Human-preference ranking for agentic, autonomous task completion. Score is an Elo \
         rating from blind side-by-side votes; higher is better.",
    ),
    (
        "search",
        "elo_search",
        "Search",
        "Human-preference ranking for search-augmented answers that combine retrieval with \
         language understanding. Score is an Elo rating from blind side-by-side votes; higher \
         is better.",
    ),
    (
        "document",
        "elo_document",
        "Document",
        "Human-preference ranking for document and PDF understanding. Score is an Elo rating \
         from blind side-by-side votes; higher is better.",
    ),
];

/// Single group name — all 6 Elo metrics live here, forming one radar preset.
const GROUP: &str = "Arena Elo";

// ---------------------------------------------------------------------------
// Raw board shape (typed serde; #[serde(default)] tolerates nullable fields).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawBoard {
    #[serde(default)]
    meta: RawMeta,
    #[serde(default)]
    models: Vec<RawModel>,
}

#[derive(Debug, Default, Deserialize)]
struct RawMeta {
    #[serde(default)]
    source_url: Option<String>,
    #[serde(default)]
    fetched_at: Option<String>,
    #[serde(default)]
    last_updated: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawModel {
    #[serde(default)]
    model: String,
    #[serde(default)]
    vendor: Option<String>,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    score: Option<f64>,
    #[serde(default)]
    ci: Option<f64>,
}

// ---------------------------------------------------------------------------
// Date + slug helpers.
// ---------------------------------------------------------------------------

/// Reduce a board's `last_updated`/`fetched_at` to a `YYYY-MM-DD` date string.
///
/// Upstream `last_updated` is human-readable (e.g. `"Jun 5, 2026"`) and may be
/// null; `fetched_at` is an RFC3339 timestamp. Resolution order:
/// 1. Parse `last_updated` as `%b %d, %Y` (e.g. `Jun 5, 2026`).
/// 2. Fall back to the date part of `fetched_at` (everything before the `T`).
/// 3. Empty string if neither is usable (defensive; real data always has one).
fn board_date(meta: &RawMeta) -> String {
    if let Some(raw) = &meta.last_updated {
        let raw = raw.trim();
        if let Ok(d) = chrono::NaiveDate::parse_from_str(raw, "%b %d, %Y") {
            return d.format("%Y-%m-%d").to_string();
        }
    }
    if let Some(fetched) = &meta.fetched_at {
        // RFC3339 -> date part (split on 'T'); robust without parsing the tz.
        return fetched.split('T').next().unwrap_or("").to_string();
    }
    String::new()
}

/// Slugify a name: lowercase, spaces and slashes -> '-'. Per spec, only spaces
/// and slashes are collapsed (dots etc. are preserved, e.g. `Z.ai` -> `z.ai`).
fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c == ' ' || c == '/' { '-' } else { c })
        .collect()
}

/// License -> open_weights: "open" => Some(true), "proprietary" => Some(false),
/// anything else / null => None.
fn license_to_open_weights(license: Option<&str>) -> Option<bool> {
    match license {
        Some("open") => Some(true),
        Some("proprietary") => Some(false),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Merge state — one row per distinct model NAME, accumulated across boards.
// ---------------------------------------------------------------------------

struct MergeRow {
    name: String,
    vendor: Option<String>,
    license: Option<String>,
    scores: BTreeMap<String, ScoreCell>,
    /// First-seen ordering tiebreaker (board iteration order); only used as a
    /// stable secondary key — the final output is sorted by name (see below).
    first_seen: usize,
}

/// Build a `SourceFile` from the per-board raw inputs (in `BOARDS` order; any
/// missing board is simply absent from `boards`). The caller has already warned
/// about and skipped missing board files.
fn build_source_file(boards: Vec<(BoardEntry, RawBoard)>) -> SourceFile {
    // Derive attribution URL + fetched_at from the first board with a source_url
    // (all boards share the same arena.ai domain within a snapshot).
    let attribution_url = boards
        .iter()
        .find_map(|(_, b)| b.meta.source_url.as_deref())
        .map(domain_root)
        .unwrap_or_else(|| "https://arena.ai".to_string());

    // Metric defs: one per PRESENT board, in BOARDS display order. Each carries
    // that board's resolved date as `last_updated`.
    let mut metrics: Vec<MetricDef> = Vec::new();
    for ((_stem, id, label, description), board) in &boards {
        let date = board_date(&board.meta);
        metrics.push(MetricDef {
            id: (*id).to_string(),
            label: (*label).to_string(),
            kind: MetricKind::Elo,
            group: GROUP.to_string(),
            higher_is_better: true,
            last_updated: if date.is_empty() { None } else { Some(date) },
            description: Some((*description).to_string()),
        });
    }

    // Merge models across boards by SLUGIFIED-name identity. Boards are not
    // naming-consistent: the agent board uses Title-Case display names
    // ("GLM 5.1") where the other boards use slug-style names ("glm-5.1").
    // Both forms slugify to the same id, so keying the merge by slug folds
    // them into one row instead of emitting id-colliding duplicates.
    let mut rows: BTreeMap<String, MergeRow> = BTreeMap::new();
    let mut order = 0usize;
    for ((_stem, metric_id, _label, _description), board) in &boards {
        let date = board_date(&board.meta);
        for raw in &board.models {
            let entry = rows.entry(slugify(&raw.model)).or_insert_with(|| {
                let seen = order;
                order += 1;
                MergeRow {
                    name: raw.model.clone(),
                    vendor: raw.vendor.clone(),
                    license: raw.license.clone(),
                    scores: BTreeMap::new(),
                    first_seen: seen,
                }
            });
            // First non-null vendor/license wins (boards are consistent, but be
            // defensive about a board that left a field null).
            if entry.vendor.is_none() {
                entry.vendor = raw.vendor.clone();
            }
            if entry.license.is_none() {
                entry.license = raw.license.clone();
            }
            // Skip null scores (agent board ships all-null; other boards may
            // carry the odd null). A present-but-null score contributes nothing.
            if let Some(value) = raw.score {
                entry.scores.insert(
                    (*metric_id).to_string(),
                    ScoreCell {
                        value,
                        date: if date.is_empty() {
                            None
                        } else {
                            Some(date.clone())
                        },
                        ci: raw.ci,
                    },
                );
            }
        }
    }

    // Deterministic output ordering: by model name (BTreeMap iteration is
    // already name-sorted). `first_seen` is retained only as documentation of
    // board insertion order; name is the sole, stable ordering key.
    let mut merge_rows: Vec<MergeRow> = rows.into_values().collect();
    // Drop rows that ended up with no scores at all (the agent board currently
    // ships all-null scores, so agent-only models would otherwise render as
    // all-em-dash phantom rows under Name sort and in the compare list).
    merge_rows.retain(|r| !r.scores.is_empty());
    merge_rows.sort_by(|a, b| a.name.cmp(&b.name).then(a.first_seen.cmp(&b.first_seen)));

    let models = merge_rows.into_iter().map(merge_to_row).collect();

    SourceFile {
        source: SourceMeta {
            id: "arena".to_string(),
            name: "Arena".to_string(),
            url: attribution_url,
            fetched_at: chrono::Utc::now().to_rfc3339(),
            verified: true,
        },
        metrics,
        models,
    }
}

/// Derive `https://<host>` from a board `source_url`. Falls back to the trimmed
/// input on any parse trouble, then to `https://arena.ai` for empty input.
fn domain_root(source_url: &str) -> String {
    // source_url looks like "https://arena.ai/leaderboard/text".
    let (scheme, rest) = match source_url.split_once("://") {
        Some((s, r)) => (s, r),
        None => return "https://arena.ai".to_string(),
    };
    let host = rest.split('/').next().unwrap_or("").trim();
    if host.is_empty() {
        return "https://arena.ai".to_string();
    }
    format!("{scheme}://{host}")
}

/// Convert one merged row into a `ModelRow`. Arena names rarely carry
/// parentheticals, but `parse_name_metadata` is run anyway (per spec) so any
/// `(Thinking)` / `(High)` suffix on e.g. agent-board names is cleaned from
/// `display_name` and folded into reasoning/effort/variant metadata.
fn merge_to_row(row: MergeRow) -> ModelRow {
    let parsed = crate::schema::parse_name_metadata(&row.name, ReasoningStatus::None);

    let creator = row.vendor.as_deref().map(slugify).unwrap_or_default();
    let creator_name = row.vendor.unwrap_or_default();

    ModelRow {
        id: slugify(&row.name),
        name: row.name,
        display_name: parsed.display_name,
        creator,
        creator_name,
        // Arena boards carry no release dates.
        release_date: None,
        reasoning_status: parsed.reasoning_status,
        effort_level: parsed.effort_level,
        variant_tag: parsed.variant_tag,
        open_weights: license_to_open_weights(row.license.as_deref()),
        // Arena boards carry no context window.
        context_window: None,
        scores: row.scores,
    }
}

/// Read + parse the present board files from `input_dir`. A missing board file
/// is skipped with a stderr warning (not an error). Returns boards in `BOARDS`
/// display order.
fn read_boards(input_dir: &Path) -> Result<Vec<(BoardEntry, RawBoard)>, String> {
    let mut boards = Vec::new();
    for &entry in BOARDS {
        let (stem, _id, _label, _description) = entry;
        let path = input_dir.join(format!("{stem}.json"));
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => {
                eprintln!("warning: board file {} missing — skipping", path.display());
                continue;
            }
        };
        let board: RawBoard = serde_json::from_str(&text)
            .map_err(|e| format!("failed to parse board {}: {e}", path.display()))?;
        boards.push((entry, board));
    }
    Ok(boards)
}

/// Two `SourceFile`s are "the same" for commit-if-changed purposes if they are
/// equal after normalizing `fetched_at` out (the timestamp changes every run).
fn unchanged(old: &SourceFile, new: &SourceFile) -> bool {
    let mut old_norm = old.clone();
    old_norm.source.fetched_at = new.source.fetched_at.clone();
    &old_norm == new
}

pub fn run(input_dir: &Path, output: &Path) -> Result<(), String> {
    let boards = read_boards(input_dir)?;
    if boards.is_empty() {
        return Err(format!(
            "no Arena board files found in {} (expected text/vision/code/agent/search/document .json)",
            input_dir.display()
        ));
    }

    let source = build_source_file(boards);

    // If-changed: skip the write (and leave mtime/content untouched) when the
    // only difference from the existing output is the fetched_at timestamp.
    if let Ok(existing_text) = std::fs::read_to_string(output) {
        if let Ok(existing) = serde_json::from_str::<SourceFile>(&existing_text) {
            if unchanged(&existing, &source) {
                println!("unchanged");
                return Ok(());
            }
        }
    }

    let pretty =
        serde_json::to_string_pretty(&source).map_err(|e| format!("failed to serialize: {e}"))?;
    std::fs::write(output, pretty)
        .map_err(|e| format!("failed to write {}: {e}", output.display()))?;

    println!(
        "wrote {} ({} models, {} metrics)",
        output.display(),
        source.models.len(),
        source.metrics.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/arena")
    }

    fn parse_fixture() -> SourceFile {
        let boards = read_boards(&fixture_dir()).expect("fixture boards read");
        build_source_file(boards)
    }

    fn row<'a>(sf: &'a SourceFile, id: &str) -> &'a ModelRow {
        sf.models
            .iter()
            .find(|m| m.id == id)
            .unwrap_or_else(|| panic!("model {id} present"))
    }

    // --- date helper -------------------------------------------------------

    #[test]
    fn board_date_parses_human_readable() {
        let meta = RawMeta {
            source_url: None,
            fetched_at: Some("2026-06-10T06:04:20.630968+00:00".into()),
            last_updated: Some("Jun 5, 2026".into()),
        };
        assert_eq!(board_date(&meta), "2026-06-05");
    }

    #[test]
    fn board_date_falls_back_to_fetched_at_when_last_updated_null() {
        let meta = RawMeta {
            source_url: None,
            fetched_at: Some("2026-06-10T06:04:20.630968+00:00".into()),
            last_updated: None,
        };
        assert_eq!(board_date(&meta), "2026-06-10");
    }

    #[test]
    fn board_date_falls_back_when_last_updated_unparseable() {
        let meta = RawMeta {
            source_url: None,
            fetched_at: Some("2026-06-10T06:04:20+00:00".into()),
            last_updated: Some("sometime recently".into()),
        };
        assert_eq!(board_date(&meta), "2026-06-10");
    }

    // --- slug + domain helpers --------------------------------------------

    #[test]
    fn slugify_lowercases_and_replaces_spaces_and_slashes() {
        assert_eq!(slugify("GPT 5.5 (High)"), "gpt-5.5-(high)");
        assert_eq!(slugify("Perplexity AI"), "perplexity-ai");
        assert_eq!(slugify("a/b model"), "a-b-model");
        assert_eq!(slugify("Z.ai"), "z.ai");
    }

    #[test]
    fn domain_root_extracts_host() {
        assert_eq!(
            domain_root("https://arena.ai/leaderboard/text"),
            "https://arena.ai"
        );
        assert_eq!(domain_root("http://example.com/x/y"), "http://example.com");
        assert_eq!(domain_root("not-a-url"), "https://arena.ai");
    }

    // --- metric set / order / group ---------------------------------------

    #[test]
    fn metric_set_order_and_group() {
        let sf = parse_fixture();
        // All 6 boards present in the fixture -> 6 metrics in BOARDS order.
        let ids: Vec<&str> = sf.metrics.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "elo_text",
                "elo_vision",
                "elo_code",
                "elo_agent",
                "elo_search",
                "elo_document"
            ]
        );
        let labels: Vec<&str> = sf.metrics.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Text", "Vision", "Code", "Agent", "Search", "Document"]
        );
        for m in &sf.metrics {
            assert_eq!(m.kind, MetricKind::Elo);
            assert_eq!(m.group, "Arena Elo");
            assert!(m.higher_is_better);
        }
    }

    #[test]
    fn every_metric_has_a_nonempty_description() {
        let sf = parse_fixture();
        for m in &sf.metrics {
            let d = m
                .description
                .as_deref()
                .unwrap_or_else(|| panic!("metric {} has no description", m.id));
            assert!(d.len() > 20, "metric {} description too short", m.id);
        }
    }

    #[test]
    fn single_group_yields_six_axis_radar() {
        let sf = parse_fixture();
        // One group with 6 higher_is_better Elo metrics -> exactly one radar
        // preset (the >= 3 higher_is_better rule in multi.rs::radar_groups).
        let hib: Vec<_> = sf
            .metrics
            .iter()
            .filter(|m| m.group == "Arena Elo" && m.higher_is_better)
            .collect();
        assert_eq!(hib.len(), 6);
    }

    #[test]
    fn metric_last_updated_resolved_per_board() {
        let sf = parse_fixture();
        let by_id = |id: &str| {
            sf.metrics
                .iter()
                .find(|m| m.id == id)
                .unwrap()
                .last_updated
                .clone()
        };
        // text/vision: "Jun 5, 2026" -> 2026-06-05
        assert_eq!(by_id("elo_text").as_deref(), Some("2026-06-05"));
        assert_eq!(by_id("elo_vision").as_deref(), Some("2026-06-05"));
        // agent: "Jun 9, 2026" -> 2026-06-09
        assert_eq!(by_id("elo_agent").as_deref(), Some("2026-06-09"));
        // code/search/document: last_updated null -> fetched_at date 2026-06-10
        assert_eq!(by_id("elo_code").as_deref(), Some("2026-06-10"));
        assert_eq!(by_id("elo_search").as_deref(), Some("2026-06-10"));
        assert_eq!(by_id("elo_document").as_deref(), Some("2026-06-10"));
    }

    // --- source meta -------------------------------------------------------

    #[test]
    fn source_meta_is_arena() {
        let sf = parse_fixture();
        assert_eq!(sf.source.id, "arena");
        assert_eq!(sf.source.name, "Arena");
        assert_eq!(sf.source.url, "https://arena.ai");
        assert!(sf.source.verified);
        assert!(!sf.source.fetched_at.is_empty());
    }

    // --- cross-board merge -------------------------------------------------

    #[test]
    fn cross_board_merge_by_slug_identity() {
        let sf = parse_fixture();
        // claude-opus-4-7 appears on text/vision/code/search/document (NOT the
        // agent board, which uses the parenthesized "Claude Opus 4.7 (Thinking)"
        // name — a DIFFERENT slug, so it does not merge here). 5 Elo scores
        // merged onto one row.
        let m = row(&sf, "claude-opus-4-7");
        assert_eq!(m.scores.len(), 5);
        assert_eq!(m.scores["elo_text"].value, 1493.0);
        assert_eq!(m.scores["elo_vision"].value, 1300.0);
        assert_eq!(m.scores["elo_code"].value, 1557.0);
        assert_eq!(m.scores["elo_search"].value, 1237.0);
        assert_eq!(m.scores["elo_document"].value, 1499.0);
        assert!(!m.scores.contains_key("elo_agent"));
        assert_eq!(m.creator, "anthropic");
        assert_eq!(m.creator_name, "Anthropic");
    }

    #[test]
    fn second_model_merges_two_boards() {
        let sf = parse_fixture();
        // gemini-3-pro: vision + document only.
        let m = row(&sf, "gemini-3-pro");
        assert_eq!(m.scores.len(), 2);
        assert_eq!(m.scores["elo_vision"].value, 1388.0);
        assert_eq!(m.scores["elo_document"].value, 1466.0);
    }

    // --- license mapping ---------------------------------------------------

    #[test]
    fn license_proprietary_maps_to_closed() {
        let sf = parse_fixture();
        assert_eq!(row(&sf, "claude-opus-4-7").open_weights, Some(false));
    }

    #[test]
    fn license_open_maps_to_open() {
        let sf = parse_fixture();
        // glm-5.1 (Z.ai): license null on the text board, "open" on the merged
        // Title-Case agent row.
        assert_eq!(row(&sf, "glm-5.1").open_weights, Some(true));
    }

    #[test]
    fn license_null_maps_to_none() {
        let sf = parse_fixture();
        // nemo-mystery has license null.
        assert_eq!(row(&sf, "nemo-mystery").open_weights, None);
    }

    // --- null score skipped ------------------------------------------------

    #[test]
    fn scoreless_agent_only_rows_dropped() {
        let sf = parse_fixture();
        // The agent board ships only null scores. "Claude Opus 4.7 (Thinking)"
        // exists ONLY there (distinct slug from claude-opus-4-7), so it ends up
        // with zero scores and is dropped entirely — no phantom rows.
        assert!(sf
            .models
            .iter()
            .all(|m| m.id != "claude-opus-4.7-(thinking)"));
        // No model carries an elo_agent score (whole board is null).
        assert!(sf
            .models
            .iter()
            .all(|m| !m.scores.contains_key("elo_agent")));
    }

    #[test]
    fn titlecase_agent_row_merges_into_slug_row() {
        let sf = parse_fixture();
        // Agent board's Title-Case "GLM 5.1" slugifies to "glm-5.1" and merges
        // into the text board's slug-style row instead of duplicating the id.
        assert_eq!(
            sf.models.iter().filter(|m| m.id == "glm-5.1").count(),
            1,
            "title-case agent row must not create a duplicate id"
        );
        let m = row(&sf, "glm-5.1");
        // First-seen (text board) name wins for display.
        assert_eq!(m.name, "glm-5.1");
        // The text board left license null; the agent row supplies "open".
        assert_eq!(m.open_weights, Some(true));
    }

    #[test]
    fn scoreless_non_agent_row_dropped() {
        let sf = parse_fixture();
        // search-null-score's only board entry is a null score -> zero scores
        // overall -> dropped from the output.
        assert!(sf.models.iter().all(|m| m.id != "search-null-score"));
    }

    #[test]
    fn all_models_carry_scores() {
        let sf = parse_fixture();
        assert!(sf.models.iter().all(|m| !m.scores.is_empty()));
    }

    // --- ci captured -------------------------------------------------------

    #[test]
    fn ci_captured_when_present() {
        let sf = parse_fixture();
        let m = row(&sf, "claude-opus-4-7");
        assert_eq!(m.scores["elo_code"].ci, Some(9.0));
        assert_eq!(m.scores["elo_text"].ci, Some(5.0));
    }

    #[test]
    fn score_cell_date_resolved() {
        let sf = parse_fixture();
        let m = row(&sf, "claude-opus-4-7");
        // text board -> "Jun 5, 2026"
        assert_eq!(m.scores["elo_text"].date.as_deref(), Some("2026-06-05"));
        // code board -> fetched_at fallback
        assert_eq!(m.scores["elo_code"].date.as_deref(), Some("2026-06-10"));
    }

    // --- model field mapping ----------------------------------------------

    #[test]
    fn null_vendor_yields_empty_creator() {
        let sf = parse_fixture();
        let m = row(&sf, "nemo-mystery");
        assert_eq!(m.creator, "");
        assert_eq!(m.creator_name, "");
    }

    #[test]
    fn vendor_with_space_slugified() {
        let sf = parse_fixture();
        // perplexity-sonar, vendor "Perplexity AI".
        let m = row(&sf, "perplexity-sonar");
        assert_eq!(m.creator, "perplexity-ai");
        assert_eq!(m.creator_name, "Perplexity AI");
    }

    #[test]
    fn id_is_slugified_name_and_display_name_parsed() {
        let sf = parse_fixture();
        // Scored search-board row with a parenthetical name: cleaned in
        // display_name, effort folded into metadata.
        let m = row(&sf, "sonar-pro-(high)");
        assert_eq!(m.name, "Sonar Pro (High)");
        assert_eq!(m.display_name, "Sonar Pro");
        // "(High)" is a pure-effort keyword -> implies reasoning.
        assert_eq!(m.effort_level.as_deref(), Some("high"));
        assert_eq!(m.reasoning_status, ReasoningStatus::Reasoning);
    }

    #[test]
    fn release_date_and_context_window_always_none() {
        let sf = parse_fixture();
        for m in &sf.models {
            assert!(m.release_date.is_none());
            assert!(m.context_window.is_none());
        }
    }

    // --- deterministic ordering -------------------------------------------

    #[test]
    fn models_sorted_by_name_deterministically() {
        let sf = parse_fixture();
        // Ordering key is the RAW model name (documented in build_source_file).
        let names: Vec<&str> = sf.models.iter().map(|m| m.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "models must be sorted by raw name");
        // 6 surviving scored rows: "Sonar Pro (High)", claude-opus-4-7,
        // gemini-3-pro, glm-5.1 (Title-Case agent row merged in), nemo-mystery,
        // perplexity-sonar. The scoreless agent-only row and search-null-score
        // are dropped.
        assert_eq!(sf.models.len(), 6);
        // Capitalized name sorts first (ASCII upper-case < lower-case).
        assert_eq!(sf.models[0].name, "Sonar Pro (High)");
    }

    // --- missing board skipped --------------------------------------------

    #[test]
    fn missing_board_is_skipped_not_fatal() {
        let dir = std::env::temp_dir().join(format!(
            "transform_arena_missing_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        // Copy only text + code (omit vision/agent/search/document).
        for b in ["text", "code"] {
            std::fs::copy(
                fixture_dir().join(format!("{b}.json")),
                dir.join(format!("{b}.json")),
            )
            .expect("copy board");
        }

        let boards = read_boards(&dir).expect("read partial boards");
        let sf = build_source_file(boards);
        // Only the two present boards yield metrics.
        let ids: Vec<&str> = sf.metrics.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["elo_text", "elo_code"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- round-trip + if-changed ------------------------------------------

    #[test]
    fn serialize_deserialize_round_trip() {
        let sf = parse_fixture();
        let json = serde_json::to_string_pretty(&sf).expect("serialize");
        let back: SourceFile = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(sf, back);
    }

    #[test]
    fn unchanged_ignores_fetched_at() {
        let mut a = parse_fixture();
        let mut b = a.clone();
        a.source.fetched_at = "2026-01-01T00:00:00+00:00".to_string();
        b.source.fetched_at = "2099-12-31T23:59:59+00:00".to_string();
        assert!(unchanged(&a, &b));

        // A real difference is detected.
        b.models[0].name = "Different Name".to_string();
        assert!(!unchanged(&a, &b));
    }

    #[test]
    fn if_changed_second_run_leaves_file_untouched() {
        let output_dir = std::env::temp_dir().join(format!(
            "transform_arena_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&output_dir).expect("create temp dir");
        let output = output_dir.join("arena.json");

        // First run writes the file.
        run(&fixture_dir(), &output).expect("first run ok");
        let first_bytes = std::fs::read(&output).expect("read output");
        let first_mtime = std::fs::metadata(&output).unwrap().modified().unwrap();

        // Second run: identical input -> only fetched_at would differ ->
        // detected as unchanged -> file is NOT rewritten.
        run(&fixture_dir(), &output).expect("second run ok");
        let second_bytes = std::fs::read(&output).expect("read output again");
        let second_mtime = std::fs::metadata(&output).unwrap().modified().unwrap();

        assert_eq!(first_bytes, second_bytes, "content unchanged");
        assert_eq!(first_mtime, second_mtime, "mtime unchanged (no rewrite)");

        let _ = std::fs::remove_dir_all(&output_dir);
    }

    #[test]
    fn if_changed_detects_real_change_and_rewrites() {
        let output_dir = std::env::temp_dir().join(format!(
            "transform_arena_change_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&output_dir).expect("create temp dir");
        let output = output_dir.join("arena.json");

        // Write an output that differs in a real (non-timestamp) field.
        let mut stale = parse_fixture();
        stale.models[0].name = "Stale Name".to_string();
        std::fs::write(
            &output,
            serde_json::to_string_pretty(&stale).expect("serialize stale"),
        )
        .expect("write stale output");

        run(&fixture_dir(), &output).expect("run ok");
        let rewritten: SourceFile =
            serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).expect("parse output");
        // First model by sorted RAW name is the capitalized search-board entry.
        assert_eq!(rewritten.models[0].name, "Sonar Pro (High)");

        let _ = std::fs::remove_dir_all(&output_dir);
    }
}
