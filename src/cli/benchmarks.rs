use std::time::Duration;

use anyhow::{bail, Result};
use clap::{CommandFactory, Parser, ValueEnum};
use comfy_table::{presets::UTF8_FULL_CONDENSED, Table as ComfyTable};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{
        Block, Borders, Cell as TuiCell, HighlightSpacing, Paragraph, Row as TuiRow,
        Table as TuiTable, TableState, Wrap,
    },
    Frame,
};
use serde::Serialize;

use super::picker::{self, PickerTerminal};

use crate::benchmarks::schema::{ModelRow, ReasoningStatus, SourceFile};
use crate::benchmarks::{fetch_source, multi::ReasoningFilter, sources::SOURCES};
use crate::formatting::{cmp_opt_f64, parse_date_to_numeric, truncate};

#[derive(Parser, Debug)]
#[command(name = "benchmarks")]
#[command(about = "Query benchmark data from the command line")]
#[command(after_help = "\
\x1b[1;4mExamples:\x1b[0m
  benchmarks list                     Open the interactive benchmark picker
  benchmarks list --sort speed --limit 10
  benchmarks list --creator openai --reasoning
  benchmarks list --json
  benchmarks show gpt-4o              Show benchmark details by slug
  benchmarks show \"Claude Sonnet 4\"   Show by display name
  benchmarks show gpt-4o --json       Output details as JSON")]
pub struct BenchmarksCli {
    #[command(subcommand)]
    pub command: Option<BenchmarksCommand>,
}

#[derive(clap::Subcommand, Debug)]
pub enum BenchmarksCommand {
    /// List benchmark entries with filtering and sorting
    List {
        /// Filter by model name, display name, slug, or creator
        #[arg(long)]
        search: Option<String>,
        /// Filter by creator slug or display name
        #[arg(long)]
        creator: Option<String>,
        /// Sort column
        #[arg(long, value_enum, default_value_t = BenchmarkSort::ReleaseDate)]
        sort: BenchmarkSort,
        /// Force ascending sort
        #[arg(long, conflicts_with = "desc")]
        asc: bool,
        /// Force descending sort
        #[arg(long, conflicts_with = "asc")]
        desc: bool,
        /// Only show open-weight models
        #[arg(long, conflicts_with = "closed")]
        open: bool,
        /// Only show closed-weight models
        #[arg(long, conflicts_with = "open")]
        closed: bool,
        /// Only show reasoning/adaptive reasoning models
        #[arg(long, conflicts_with = "non_reasoning")]
        reasoning: bool,
        /// Only show explicitly non-reasoning models
        #[arg(long, conflicts_with = "reasoning")]
        non_reasoning: bool,
        /// Limit rows in human-readable or JSON output
        #[arg(long)]
        limit: Option<usize>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show a single benchmark entry in detail
    #[command(after_help = "\
\x1b[1;4mExamples:\x1b[0m
  benchmarks show gpt-4o              Look up by slug
  benchmarks show \"Claude Sonnet 4.6\"  Look up by display name
  benchmarks show claude-sonnet-4-6   Exact slug match
  benchmarks show gpt-4o --json       Output as JSON")]
    Show {
        /// Benchmark model slug, exact display name, or unique partial match
        model: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BenchmarkSort {
    Intelligence,
    Coding,
    Math,
    Gpqa,
    #[value(name = "mmlu-pro")]
    MmluPro,
    Hle,
    #[value(name = "livecodebench")]
    LiveCodeBench,
    Scicode,
    Ifbench,
    Lcr,
    #[value(name = "terminalbench")]
    TerminalBench,
    Tau2,
    #[value(name = "speed")]
    Speed,
    Ttft,
    Ttfat,
    #[value(name = "price-input")]
    PriceInput,
    #[value(name = "price-output")]
    PriceOutput,
    #[value(name = "price-blended")]
    PriceBlended,
    Name,
    #[value(name = "release-date")]
    ReleaseDate,
}

impl BenchmarkSort {
    fn label(self) -> &'static str {
        match self {
            Self::Intelligence => "Intelligence",
            Self::Coding => "Coding",
            Self::Math => "Math",
            Self::Gpqa => "GPQA",
            Self::MmluPro => "MMLU-Pro",
            Self::Hle => "HLE",
            Self::LiveCodeBench => "LiveCodeBench",
            Self::Scicode => "SciCode",
            Self::Ifbench => "IFBench",
            Self::Lcr => "LCR",
            Self::TerminalBench => "TerminalBench",
            Self::Tau2 => "Tau2",
            Self::Speed => "Tok/s",
            Self::Ttft => "TTFT",
            Self::Ttfat => "TTFAT",
            Self::PriceInput => "Input $/M",
            Self::PriceOutput => "Output $/M",
            Self::PriceBlended => "Blended $/M",
            Self::Name => "Name",
            Self::ReleaseDate => "Release",
        }
    }

    fn default_descending(self) -> bool {
        !matches!(
            self,
            Self::Name
                | Self::Ttft
                | Self::Ttfat
                | Self::PriceInput
                | Self::PriceOutput
                | Self::PriceBlended
        )
    }

    /// The metric id this sort column reads from a model's score map, or `None`
    /// for the non-metric sorts (Name, ReleaseDate).
    fn metric_id(self) -> Option<&'static str> {
        Some(match self {
            Self::Intelligence => "intelligence_index",
            Self::Coding => "coding_index",
            Self::Math => "math_index",
            Self::Gpqa => "gpqa",
            Self::MmluPro => "mmlu_pro",
            Self::Hle => "hle",
            Self::LiveCodeBench => "livecodebench",
            Self::Scicode => "scicode",
            Self::Ifbench => "ifbench",
            Self::Lcr => "lcr",
            Self::TerminalBench => "terminalbench_hard",
            Self::Tau2 => "tau2",
            Self::Speed => "output_tps",
            Self::Ttft => "ttft",
            Self::Ttfat => "ttfat",
            Self::PriceInput => "price_input",
            Self::PriceOutput => "price_output",
            Self::PriceBlended => "price_blended",
            Self::Name | Self::ReleaseDate => return None,
        })
    }

    fn extract(self, row: &ModelRow) -> Option<f64> {
        match self {
            Self::Name => Some(0.0),
            Self::ReleaseDate => row.release_date.as_deref().and_then(parse_date_to_numeric),
            _ => self
                .metric_id()
                .and_then(|id| row.scores.get(id))
                .map(|cell| cell.value),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WeightsFilter {
    All,
    Open,
    Closed,
}

#[derive(Debug, Clone)]
struct ListOptions {
    search: Option<String>,
    creator: Option<String>,
    sort: BenchmarkSort,
    descending: bool,
    weights_filter: WeightsFilter,
    reasoning_filter: ReasoningFilter,
    limit: Option<usize>,
}

#[derive(Serialize)]
struct BenchmarkListItem<'a> {
    slug: &'a str,
    name: &'a str,
    display_name: &'a str,
    creator: &'a str,
    creator_name: &'a str,
    release_date: Option<&'a str>,
    sort: &'static str,
    sort_value: Option<f64>,
    open_weights: Option<bool>,
    reasoning: &'static str,
}

#[derive(Serialize)]
struct BenchmarkDetail<'a> {
    slug: &'a str,
    name: &'a str,
    display_name: &'a str,
    creator: &'a str,
    creator_name: &'a str,
    creator_id: &'a str,
    release_date: Option<&'a str>,
    open_weights: Option<bool>,
    reasoning: &'static str,
    effort_level: Option<&'a str>,
    variant_tag: Option<&'a str>,
    tool_call: Option<bool>,
    context_window: Option<u64>,
    max_output: Option<u64>,
    intelligence_index: Option<f64>,
    coding_index: Option<f64>,
    math_index: Option<f64>,
    mmlu_pro: Option<f64>,
    gpqa: Option<f64>,
    hle: Option<f64>,
    livecodebench: Option<f64>,
    scicode: Option<f64>,
    ifbench: Option<f64>,
    lcr: Option<f64>,
    terminalbench_hard: Option<f64>,
    tau2: Option<f64>,
    math_500: Option<f64>,
    aime: Option<f64>,
    aime_25: Option<f64>,
    output_tps: Option<f64>,
    ttft: Option<f64>,
    ttfat: Option<f64>,
    price_input: Option<f64>,
    price_output: Option<f64>,
    price_blended: Option<f64>,
}

enum ResolveEntry<'a> {
    Single(&'a ModelRow),
    Ambiguous(Vec<&'a ModelRow>),
}

const PICKER_SORTS: [BenchmarkSort; 9] = [
    BenchmarkSort::Intelligence,
    BenchmarkSort::Coding,
    BenchmarkSort::Math,
    BenchmarkSort::Gpqa,
    BenchmarkSort::Speed,
    BenchmarkSort::Ttft,
    BenchmarkSort::PriceBlended,
    BenchmarkSort::ReleaseDate,
    BenchmarkSort::Name,
];

/// Read a metric value (`scores[id].value`) from a model row.
fn metric(row: &ModelRow, id: &str) -> Option<f64> {
    row.scores.get(id).map(|cell| cell.value)
}

struct BenchmarkPicker<'a> {
    entries: Vec<&'a ModelRow>,
    visible_entries: Vec<&'a ModelRow>,
    sort: BenchmarkSort,
    descending: bool,
    title: String,
    query: String,
    filter_mode: bool,
    state: TableState,
}

impl<'a> BenchmarkPicker<'a> {
    fn new(
        entries: Vec<&'a ModelRow>,
        sort: BenchmarkSort,
        descending: bool,
        title: String,
    ) -> Self {
        let mut picker = Self {
            entries,
            visible_entries: Vec::new(),
            sort,
            descending,
            title,
            query: String::new(),
            filter_mode: false,
            state: TableState::default(),
        };
        picker.rebuild_visible_entries(None);
        picker
    }

    fn selected(&self) -> Option<&'a ModelRow> {
        self.state.selected().map(|idx| self.visible_entries[idx])
    }

    fn next(&mut self) {
        picker::nav_next(&mut self.state, self.visible_entries.len());
    }

    fn previous(&mut self) {
        picker::nav_previous(&mut self.state);
    }

    fn first(&mut self) {
        picker::nav_first(&mut self.state, self.visible_entries.len());
    }

    fn last(&mut self) {
        picker::nav_last(&mut self.state, self.visible_entries.len());
    }

    fn page_down(&mut self) {
        picker::nav_page_down(&mut self.state, self.visible_entries.len(), 10);
    }

    fn page_up(&mut self) {
        picker::nav_page_up(&mut self.state, 10);
    }

    fn cycle_sort(&mut self) {
        let current_idx = PICKER_SORTS
            .iter()
            .position(|&sort| sort == self.sort)
            .unwrap_or(0);
        self.sort = PICKER_SORTS[(current_idx + 1) % PICKER_SORTS.len()];
        self.descending = self.sort.default_descending();
        self.rebuild_visible_entries(self.selected().map(|row| row.id.as_str()));
    }

    fn toggle_descending(&mut self) {
        self.descending = !self.descending;
        self.rebuild_visible_entries(self.selected().map(|row| row.id.as_str()));
    }

    fn start_filter(&mut self) {
        self.filter_mode = true;
    }

    fn finish_filter(&mut self) {
        self.filter_mode = false;
    }

    fn clear_filter(&mut self) {
        self.query.clear();
        self.filter_mode = false;
        self.rebuild_visible_entries(None);
    }

    fn push_filter_char(&mut self, ch: char) {
        self.query.push(ch);
        self.rebuild_visible_entries(self.selected().map(|row| row.id.as_str()));
    }

    fn pop_filter_char(&mut self) {
        self.query.pop();
        self.rebuild_visible_entries(self.selected().map(|row| row.id.as_str()));
    }

    fn rebuild_visible_entries(&mut self, preserve_slug: Option<&str>) {
        self.visible_entries =
            filter_picker_entries(&self.entries, &self.query, self.sort, self.descending);
        let next_selected = preserve_slug
            .and_then(|slug| self.visible_entries.iter().position(|row| row.id == slug))
            .or_else(|| (!self.visible_entries.is_empty()).then_some(0));
        self.state.select(next_selected);
    }

    fn draw(&mut self, frame: &mut Frame<'_>) {
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(10), Constraint::Length(1)])
            .split(frame.area());
        let main = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(outer[0]);
        let rows = self.visible_entries.iter().map(|row| {
            use ratatui::text::Span;

            let reasoning_cell = match row.reasoning_status {
                ReasoningStatus::Reasoning => {
                    TuiCell::from(Span::styled("R", Style::default().fg(Color::Cyan)))
                }
                ReasoningStatus::Adaptive => {
                    TuiCell::from(Span::styled("A", Style::default().fg(Color::Cyan)))
                }
                _ => TuiCell::from(""),
            };
            let source_cell = match row.open_weights {
                Some(true) => TuiCell::from(Span::styled("O", Style::default().fg(Color::Green))),
                Some(false) => TuiCell::from(Span::styled("C", Style::default().fg(Color::Red))),
                None => TuiCell::from(""),
            };
            TuiRow::new(vec![
                TuiCell::from(truncate(&row.display_name, 28)),
                TuiCell::from(truncate(creator_label(row), 14)),
                TuiCell::from(
                    row.release_date
                        .clone()
                        .unwrap_or_else(|| "\u{2014}".to_string()),
                ),
                reasoning_cell,
                source_cell,
            ])
        });

        let table = TuiTable::new(
            rows,
            [
                Constraint::Percentage(40),
                Constraint::Percentage(22),
                Constraint::Percentage(20),
                Constraint::Length(3),
                Constraint::Length(3),
            ],
        )
        .header(
            TuiRow::new(vec!["Name", "Creator", "Release", "R", "S"]).style(picker::HEADER_STYLE),
        )
        .column_spacing(1)
        .highlight_symbol(picker::HIGHLIGHT_SYMBOL)
        .highlight_spacing(HighlightSpacing::Always)
        .row_highlight_style(picker::ROW_HIGHLIGHT_STYLE)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(picker::ACTIVE_BORDER_STYLE)
                .title(self.title_text()),
        );

        frame.render_stateful_widget(table, main[0], &mut self.state);

        let preview = Paragraph::new(self.preview_lines())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(picker::PREVIEW_BORDER_STYLE)
                    .title(" Preview "),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(preview, main[1]);

        let controls = Paragraph::new(self.status_line());
        frame.render_widget(controls, outer[1]);
    }

    fn title_text(&self) -> String {
        picker::picker_title(
            &self.title,
            self.visible_entries.len(),
            self.entries.len(),
            picker_sort_label(self.sort),
            self.descending,
            &self.query,
        )
    }

    fn preview_lines(&self) -> Vec<Line<'static>> {
        use ratatui::text::Span;

        let Some(row) = self.selected() else {
            return vec![
                Line::from("No matches"),
                Line::from(""),
                Line::from("Adjust the filter or clear it with Esc while filtering."),
            ];
        };
        let dim = Style::default().fg(Color::DarkGray);
        let label = |s: &str| -> Span<'static> { Span::styled(format!("{s}: "), dim) };
        let metric_span = |v: Option<f64>| -> Span<'static> {
            match v {
                Some(val) => Span::raw(format!("{val:.2}")),
                None => Span::styled("\u{2014}", dim),
            }
        };

        // Header
        let mut lines = vec![Line::from(vec![
            Span::styled(
                row.display_name.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(row.id.clone(), dim),
        ])];

        // Categorical row: creator + reasoning + source
        let reasoning_text = match row.reasoning_status {
            ReasoningStatus::Reasoning => ("R", Color::Cyan),
            ReasoningStatus::Adaptive => ("A", Color::Cyan),
            ReasoningStatus::NonReasoning => ("NR", Color::DarkGray),
            ReasoningStatus::None => ("?", Color::DarkGray),
        };
        let source_text = match row.open_weights {
            Some(true) => ("Open", Color::Green),
            Some(false) => ("Closed", Color::Red),
            None => ("\u{2014}", Color::DarkGray),
        };
        let mut meta_spans = vec![
            Span::styled(row.creator_name.clone(), dim),
            Span::raw("  "),
            Span::styled(
                reasoning_text.0.to_string(),
                Style::default().fg(reasoning_text.1),
            ),
            Span::raw("  "),
            Span::styled(
                source_text.0.to_string(),
                Style::default().fg(source_text.1),
            ),
        ];
        if let Some(ref date) = row.release_date {
            meta_spans.push(Span::raw("  "));
            meta_spans.push(Span::raw(date.clone()));
        }
        lines.push(Line::from(meta_spans));

        // Benchmarks
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            label("Intelligence"),
            metric_span(metric(row, "intelligence_index")),
            Span::raw("  "),
            label("Coding"),
            metric_span(metric(row, "coding_index")),
            Span::raw("  "),
            label("Math"),
            metric_span(metric(row, "math_index")),
        ]));
        lines.push(Line::from(vec![
            label("GPQA"),
            metric_span(metric(row, "gpqa")),
            Span::raw("  "),
            label("HLE"),
            metric_span(metric(row, "hle")),
            Span::raw("  "),
            label("MMLU-Pro"),
            metric_span(metric(row, "mmlu_pro")),
        ]));
        lines.push(Line::from(vec![
            label("LiveCode"),
            metric_span(metric(row, "livecodebench")),
            Span::raw("  "),
            label("SciCode"),
            metric_span(metric(row, "scicode")),
            Span::raw("  "),
            label("IFBench"),
            metric_span(metric(row, "ifbench")),
        ]));

        // Performance + pricing
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            label("Tok/s"),
            metric_span(metric(row, "output_tps")),
            Span::raw("  "),
            label("TTFT"),
            metric_span(metric(row, "ttft")),
            Span::raw("  "),
            label("Blended $/M"),
            metric_span(metric(row, "price_blended")),
        ]));
        lines.push(Line::from(vec![
            label("Input $/M"),
            metric_span(metric(row, "price_input")),
            Span::raw("  "),
            label("Output $/M"),
            metric_span(metric(row, "price_output")),
        ]));

        lines
    }

    fn status_line(&self) -> Line<'static> {
        if self.filter_mode {
            Line::from(format!(
                "Filter: {}_  Enter apply  Esc clear  Backspace delete",
                self.query
            ))
        } else {
            Line::from("Enter inspect   / filter   s sort   S reverse   q quit   ↑↓/j/k move")
        }
    }
}

pub fn run() -> Result<()> {
    let cli = BenchmarksCli::parse();
    run_with_command(cli.command)
}

pub fn run_with_command(command: Option<BenchmarksCommand>) -> Result<()> {
    match command {
        Some(BenchmarksCommand::List {
            search,
            creator,
            sort,
            asc,
            desc,
            open,
            closed,
            reasoning,
            non_reasoning,
            limit,
            json,
        }) => run_list(
            ListOptions {
                search,
                creator,
                sort,
                descending: if asc {
                    false
                } else if desc {
                    true
                } else {
                    sort.default_descending()
                },
                weights_filter: if open {
                    WeightsFilter::Open
                } else if closed {
                    WeightsFilter::Closed
                } else {
                    WeightsFilter::All
                },
                reasoning_filter: if reasoning {
                    ReasoningFilter::Reasoning
                } else if non_reasoning {
                    ReasoningFilter::NonReasoning
                } else {
                    ReasoningFilter::All
                },
                limit,
            },
            json,
        ),
        Some(BenchmarksCommand::Show { model, json }) => run_show(&model, json),
        None => {
            BenchmarksCli::command().print_long_help()?;
            println!();
            Ok(())
        }
    }
}

fn run_list(options: ListOptions, json: bool) -> Result<()> {
    let file = load_benchmarks()?;
    let entries = filter_entries(&file.models, &options);

    if json {
        let items: Vec<_> = entries
            .iter()
            .map(|row| BenchmarkListItem {
                slug: row.id.as_str(),
                name: row.name.as_str(),
                display_name: row.display_name.as_str(),
                creator: row.creator.as_str(),
                creator_name: creator_label(row),
                release_date: row.release_date.as_deref(),
                sort: options.sort.label(),
                sort_value: options.sort.extract(row),
                open_weights: row.open_weights,
                reasoning: reasoning_label(row),
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    if entries.is_empty() {
        println!("No benchmark entries matched the current filters.");
        return Ok(());
    }

    if super::styles::is_tty() {
        let title = " Benchmark Picker ".to_string();
        if let Some(row) =
            pick_benchmark(entries, options.sort, options.descending, title.as_str())?
        {
            print_entry_detail(row, false)?;
        }
        return Ok(());
    }

    print_list_table(&entries, options.sort);
    Ok(())
}

fn print_list_table(entries: &[&ModelRow], sort: BenchmarkSort) {
    let mut table = ComfyTable::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(vec![
        "Slug",
        "Name",
        "Creator",
        sort.label(),
        "Source",
        "Reasoning",
        "Release",
    ]);

    for row in entries {
        table.add_row(vec![
            row.id.clone(),
            row.display_name.clone(),
            creator_label(row).to_string(),
            format_sort_value(sort, row),
            format_open_weights(row.open_weights),
            reasoning_label(row).to_string(),
            row.release_date
                .clone()
                .unwrap_or_else(|| "\u{2014}".to_string()),
        ]);
    }

    println!("{table}");
}

fn run_show(model: &str, json: bool) -> Result<()> {
    let file = load_benchmarks()?;
    match resolve_entry(&file.models, model)? {
        ResolveEntry::Single(row) => print_entry_detail(row, json)?,
        ResolveEntry::Ambiguous(rows) => {
            if json || !super::styles::is_tty() {
                bail!("{}", ambiguous_matches_message(model, &rows));
            }

            let title = format!(" Select Benchmark Match for \"{model}\" ");
            if let Some(row) = pick_benchmark(rows, BenchmarkSort::ReleaseDate, true, &title)? {
                print_entry_detail(row, false)?;
            }
        }
    }
    Ok(())
}

/// Fetch the AA source file via the v2 fetch lane and parse it.
///
/// AA is `SOURCES[0]`. The fetch runs on a one-shot blocking runtime so the CLI
/// stays synchronous.
//
// TODO(phase-3): `ModelRow.open_weights` is `None` for AA here — the trait-based
// openness enrichment is currently TUI-side only, so the `S` column renders an
// em-dash. A later phase may call
// `crate::benchmarks::apply_model_traits(&providers, &mut file.models)` here
// (after `crate::api::fetch_providers()`) to populate it, matching the TUI.
fn load_benchmarks() -> Result<SourceFile> {
    let runtime = tokio::runtime::Runtime::new()?;
    match runtime.block_on(fetch_source(&SOURCES[0])) {
        Some(file) => Ok(file),
        None => bail!("Failed to fetch benchmark data from the CDN"),
    }
}

fn filter_entries<'a>(models: &'a [ModelRow], options: &ListOptions) -> Vec<&'a ModelRow> {
    let search = options.search.as_ref().map(|s| s.to_lowercase());
    let creator = options.creator.as_ref().map(|s| s.to_lowercase());

    let mut filtered: Vec<_> = models
        .iter()
        .filter(|row| {
            if !matches_weights_filter(options.weights_filter, row) {
                return false;
            }

            if !options.reasoning_filter.matches(row) {
                return false;
            }

            if let Some(creator_filter) = &creator {
                let creator_name = creator_label(row).to_lowercase();
                if !row.creator.to_lowercase().contains(creator_filter)
                    && !creator_name.contains(creator_filter)
                {
                    return false;
                }
            }

            if let Some(search_query) = &search {
                let matches = row.id.to_lowercase().contains(search_query)
                    || row.name.to_lowercase().contains(search_query)
                    || row.display_name.to_lowercase().contains(search_query)
                    || row.creator.to_lowercase().contains(search_query)
                    || creator_label(row).to_lowercase().contains(search_query);
                if !matches {
                    return false;
                }
            }

            true
        })
        .collect();

    if !matches!(options.sort, BenchmarkSort::Name) {
        filtered.retain(|row| options.sort.extract(row).is_some());
    }

    filtered.sort_by(|a, b| {
        let order = match options.sort {
            BenchmarkSort::Name => a.display_name.cmp(&b.display_name),
            _ => cmp_opt_f64(options.sort.extract(a), options.sort.extract(b))
                .then_with(|| a.display_name.cmp(&b.display_name)),
        };

        if options.descending {
            order.reverse()
        } else {
            order
        }
    });

    if let Some(limit) = options.limit {
        filtered.truncate(limit);
    }

    filtered
}

fn matches_weights_filter(weights_filter: WeightsFilter, row: &ModelRow) -> bool {
    match weights_filter {
        WeightsFilter::All => true,
        WeightsFilter::Open => row.open_weights.unwrap_or(false),
        WeightsFilter::Closed => row.open_weights.map(|open| !open).unwrap_or(false),
    }
}

fn resolve_entry<'a>(models: &'a [ModelRow], query: &str) -> Result<ResolveEntry<'a>> {
    let query_lower = query.to_lowercase();

    if let Some(row) = models.iter().find(|row| row.id.eq_ignore_ascii_case(query)) {
        return Ok(ResolveEntry::Single(row));
    }

    let exact_matches = matching_entries(models, |row| {
        row.name.eq_ignore_ascii_case(query) || row.display_name.eq_ignore_ascii_case(query)
    });
    match exact_matches.as_slice() {
        [row] => return Ok(ResolveEntry::Single(row)),
        [] => {}
        many => return Ok(ResolveEntry::Ambiguous(many.to_vec())),
    }

    let matches = matching_entries(models, |row| {
        row.id.to_lowercase().contains(&query_lower)
            || row.name.to_lowercase().contains(&query_lower)
            || row.display_name.to_lowercase().contains(&query_lower)
    });

    match matches.as_slice() {
        [] => bail!("No benchmark entry matched '{query}'"),
        [row] => Ok(ResolveEntry::Single(row)),
        many => Ok(ResolveEntry::Ambiguous(many.to_vec())),
    }
}

fn matching_entries<F>(models: &[ModelRow], predicate: F) -> Vec<&ModelRow>
where
    F: Fn(&ModelRow) -> bool,
{
    let mut matches: Vec<_> = models.iter().filter(|row| predicate(row)).collect();
    matches.sort_by(|a, b| {
        a.display_name
            .cmp(&b.display_name)
            .then_with(|| a.id.cmp(&b.id))
    });
    matches
}

fn filter_picker_entries<'a>(
    entries: &[&'a ModelRow],
    query: &str,
    sort: BenchmarkSort,
    descending: bool,
) -> Vec<&'a ModelRow> {
    let query = query.trim().to_lowercase();
    let mut visible: Vec<_> = entries
        .iter()
        .copied()
        .filter(|row| {
            query.is_empty()
                || row.id.to_lowercase().contains(&query)
                || row.name.to_lowercase().contains(&query)
                || row.display_name.to_lowercase().contains(&query)
                || row.creator.to_lowercase().contains(&query)
                || creator_label(row).to_lowercase().contains(&query)
        })
        .collect();

    if !matches!(sort, BenchmarkSort::Name) {
        visible.retain(|row| sort.extract(row).is_some());
    }

    visible.sort_by(|a, b| {
        let order = match sort {
            BenchmarkSort::Name => a.display_name.cmp(&b.display_name),
            _ => cmp_opt_f64(sort.extract(a), sort.extract(b))
                .then_with(|| a.display_name.cmp(&b.display_name)),
        };
        if descending {
            order.reverse()
        } else {
            order
        }
    });

    visible
}

fn ambiguous_matches_message(query: &str, rows: &[&ModelRow]) -> String {
    let suggestions = rows
        .iter()
        .take(5)
        .map(|row| row.id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!("Benchmark query '{query}' was ambiguous; try a slug. Matches: {suggestions}")
}

fn creator_label(row: &ModelRow) -> &str {
    if row.creator_name.is_empty() {
        &row.creator
    } else {
        &row.creator_name
    }
}

fn reasoning_label(row: &ModelRow) -> &'static str {
    match row.reasoning_status {
        ReasoningStatus::Adaptive => "Adaptive",
        ReasoningStatus::Reasoning => "Reasoning",
        ReasoningStatus::NonReasoning => "Non-reasoning",
        ReasoningStatus::None => "Unknown",
    }
}

fn format_sort_value(sort: BenchmarkSort, row: &ModelRow) -> String {
    match sort {
        BenchmarkSort::Name => row.display_name.clone(),
        BenchmarkSort::ReleaseDate => row
            .release_date
            .clone()
            .unwrap_or_else(|| "\u{2014}".to_string()),
        _ => format_metric(sort.extract(row)),
    }
}

fn picker_sort_label(sort: BenchmarkSort) -> &'static str {
    match sort {
        BenchmarkSort::Name => "Slug",
        _ => sort.label(),
    }
}

fn format_metric(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.2}"))
        .unwrap_or_else(|| "\u{2014}".to_string())
}

fn format_open_weights(open_weights: Option<bool>) -> String {
    match open_weights {
        Some(true) => "Open".to_string(),
        Some(false) => "Closed".to_string(),
        None => "\u{2014}".to_string(),
    }
}

fn print_detail(detail: &BenchmarkDetail<'_>) {
    println!("{}", detail.display_name);
    println!("{}", "=".repeat(detail.display_name.len()));
    println!();
    println!("Slug:         {}", detail.slug);
    println!("Name:         {}", detail.name);
    println!("Creator:      {} ({})", detail.creator_name, detail.creator);
    if !detail.creator_id.is_empty() {
        println!("Creator ID:   {}", detail.creator_id);
    }
    if let Some(release_date) = detail.release_date {
        println!("Released:     {}", release_date);
    }
    println!(
        "Open Weights: {}",
        match detail.open_weights {
            Some(true) => "Yes",
            Some(false) => "No",
            None => "Unknown",
        }
    );
    println!("Reasoning:    {}", detail.reasoning);
    if let Some(effort_level) = detail.effort_level {
        println!("Effort:       {}", effort_level);
    }
    if let Some(variant_tag) = detail.variant_tag {
        println!("Variant:      {}", variant_tag);
    }
    println!();

    println!("Indexes");
    println!("-------");
    println!("Intelligence: {}", format_metric(detail.intelligence_index));
    println!("Coding:       {}", format_metric(detail.coding_index));
    println!("Math:         {}", format_metric(detail.math_index));
    println!();

    println!("Benchmarks");
    println!("----------");
    println!("GPQA:         {}", format_metric(detail.gpqa));
    println!("MMLU-Pro:     {}", format_metric(detail.mmlu_pro));
    println!("HLE:          {}", format_metric(detail.hle));
    println!("LiveCodeBench: {}", format_metric(detail.livecodebench));
    println!("SciCode:      {}", format_metric(detail.scicode));
    println!("IFBench:      {}", format_metric(detail.ifbench));
    println!("LCR:          {}", format_metric(detail.lcr));
    println!(
        "TerminalBench: {}",
        format_metric(detail.terminalbench_hard)
    );
    println!("Tau2:         {}", format_metric(detail.tau2));
    println!("Math-500:     {}", format_metric(detail.math_500));
    println!("AIME:         {}", format_metric(detail.aime));
    println!("AIME 2025:    {}", format_metric(detail.aime_25));
    println!();

    println!("Performance");
    println!("-----------");
    println!("Output tok/s:  {}", format_metric(detail.output_tps));
    println!("TTFT:         {}", format_metric(detail.ttft));
    println!("TTFAT:        {}", format_metric(detail.ttfat));
    println!(
        "Tool Use:     {}",
        match detail.tool_call {
            Some(true) => "Yes",
            Some(false) => "No",
            None => "Unknown",
        }
    );
    if let Some(context_window) = detail.context_window {
        println!("Context:      {} tokens", context_window);
    }
    if let Some(max_output) = detail.max_output {
        println!("Max Output:   {} tokens", max_output);
    }
    println!();

    println!("Pricing");
    println!("-------");
    println!("Input $/M:    {}", format_metric(detail.price_input));
    println!("Output $/M:   {}", format_metric(detail.price_output));
    println!("Blended $/M:  {}", format_metric(detail.price_blended));
}

fn build_detail(row: &ModelRow) -> BenchmarkDetail<'_> {
    BenchmarkDetail {
        slug: row.id.as_str(),
        name: row.name.as_str(),
        display_name: row.display_name.as_str(),
        creator: row.creator.as_str(),
        creator_name: creator_label(row),
        // Creator IDs are not carried in the v2 schema; the previous data file
        // had an empty `creator_id` for every entry, so the Creator ID line was
        // already suppressed in practice.
        creator_id: "",
        release_date: row.release_date.as_deref(),
        open_weights: row.open_weights,
        reasoning: reasoning_label(row),
        effort_level: row.effort_level.as_deref(),
        variant_tag: row.variant_tag.as_deref(),
        // tool_call / max_output are not part of the v2 schema; only the TUI
        // path consumed them and they were always absent in the CLI's detail
        // output unless populated by traits matching.
        tool_call: None,
        context_window: row.context_window,
        max_output: None,
        intelligence_index: metric(row, "intelligence_index"),
        coding_index: metric(row, "coding_index"),
        math_index: metric(row, "math_index"),
        mmlu_pro: metric(row, "mmlu_pro"),
        gpqa: metric(row, "gpqa"),
        hle: metric(row, "hle"),
        livecodebench: metric(row, "livecodebench"),
        scicode: metric(row, "scicode"),
        ifbench: metric(row, "ifbench"),
        lcr: metric(row, "lcr"),
        terminalbench_hard: metric(row, "terminalbench_hard"),
        tau2: metric(row, "tau2"),
        math_500: metric(row, "math_500"),
        aime: metric(row, "aime"),
        aime_25: metric(row, "aime_25"),
        output_tps: metric(row, "output_tps"),
        ttft: metric(row, "ttft"),
        ttfat: metric(row, "ttfat"),
        price_input: metric(row, "price_input"),
        price_output: metric(row, "price_output"),
        price_blended: metric(row, "price_blended"),
    }
}

fn print_entry_detail(row: &ModelRow, json: bool) -> Result<()> {
    let detail = build_detail(row);
    if json {
        println!("{}", serde_json::to_string_pretty(&detail)?);
    } else {
        print_detail(&detail);
    }
    Ok(())
}

fn pick_benchmark<'a>(
    entries: Vec<&'a ModelRow>,
    sort: BenchmarkSort,
    descending: bool,
    title: &str,
) -> Result<Option<&'a ModelRow>> {
    let mut picker = BenchmarkPicker::new(entries, sort, descending, title.to_string());
    let mut terminal = PickerTerminal::new()?;

    loop {
        terminal.terminal.draw(|frame| picker.draw(frame))?;

        if !event::poll(Duration::from_millis(250))? {
            continue;
        }

        match event::read()? {
            Event::Resize(_, _) => terminal.terminal.autoresize()?,
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if picker.filter_mode {
                    match key.code {
                        KeyCode::Enter => picker.finish_filter(),
                        KeyCode::Esc => picker.clear_filter(),
                        KeyCode::Backspace => picker.pop_filter_char(),
                        KeyCode::Char(ch) => picker.push_filter_char(ch),
                        _ => {}
                    }
                    continue;
                }

                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => picker.previous(),
                    KeyCode::Down | KeyCode::Char('j') => picker.next(),
                    KeyCode::PageUp => picker.page_up(),
                    KeyCode::PageDown => picker.page_down(),
                    KeyCode::Home | KeyCode::Char('g') => picker.first(),
                    KeyCode::End | KeyCode::Char('G') => picker.last(),
                    KeyCode::Char('/') => picker.start_filter(),
                    KeyCode::Char('s') => picker.cycle_sort(),
                    KeyCode::Char('S') => picker.toggle_descending(),
                    KeyCode::Enter => return Ok(picker.selected()),
                    KeyCode::Esc | KeyCode::Char('q') => return Ok(None),
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmarks::schema::ScoreCell;
    use std::collections::BTreeMap;

    fn cell(value: f64) -> ScoreCell {
        ScoreCell {
            value,
            date: None,
            ci: None,
            votes: None,
        }
    }

    fn make_row(
        id: &str,
        display_name: &str,
        creator: &str,
        creator_name: &str,
        intelligence_index: Option<f64>,
    ) -> ModelRow {
        let mut scores: BTreeMap<String, ScoreCell> = BTreeMap::new();
        if let Some(v) = intelligence_index {
            scores.insert("intelligence_index".to_string(), cell(v));
        }
        scores.insert("coding_index".to_string(), cell(50.0));
        scores.insert("math_index".to_string(), cell(55.0));
        scores.insert("mmlu_pro".to_string(), cell(60.0));
        scores.insert("gpqa".to_string(), cell(61.0));
        scores.insert("hle".to_string(), cell(62.0));
        scores.insert("livecodebench".to_string(), cell(63.0));
        scores.insert("scicode".to_string(), cell(64.0));
        scores.insert("ifbench".to_string(), cell(65.0));
        scores.insert("lcr".to_string(), cell(66.0));
        scores.insert("terminalbench_hard".to_string(), cell(67.0));
        scores.insert("tau2".to_string(), cell(68.0));
        scores.insert("math_500".to_string(), cell(69.0));
        scores.insert("aime".to_string(), cell(70.0));
        scores.insert("aime_25".to_string(), cell(71.0));
        scores.insert("output_tps".to_string(), cell(72.0));
        scores.insert("ttft".to_string(), cell(1.5));
        scores.insert("ttfat".to_string(), cell(2.5));
        scores.insert("price_input".to_string(), cell(3.5));
        scores.insert("price_output".to_string(), cell(4.5));
        scores.insert("price_blended".to_string(), cell(5.5));

        ModelRow {
            id: id.to_string(),
            name: display_name.to_string(),
            display_name: display_name.to_string(),
            creator: creator.to_string(),
            creator_name: creator_name.to_string(),
            release_date: Some("2025-01-01".to_string()),
            reasoning_status: ReasoningStatus::None,
            effort_level: None,
            variant_tag: None,
            open_weights: None,
            context_window: Some(200_000),
            supports_tools: None,
            max_output: None,
            scores,
        }
    }

    #[test]
    fn filter_entries_applies_sort_filters_and_limit() {
        let mut alpha = make_row("alpha", "Alpha", "openai", "OpenAI", Some(90.0));
        alpha.reasoning_status = ReasoningStatus::Reasoning;
        alpha.open_weights = Some(false);

        let mut beta = make_row("beta", "Beta", "meta", "Meta", Some(80.0));
        beta.reasoning_status = ReasoningStatus::NonReasoning;
        beta.open_weights = Some(true);

        let gamma = make_row("gamma", "Gamma", "openai", "OpenAI", None);

        let models = vec![beta.clone(), gamma, alpha.clone()];

        let filtered = filter_entries(
            &models,
            &ListOptions {
                search: Some("a".to_string()),
                creator: Some("openai".to_string()),
                sort: BenchmarkSort::Intelligence,
                descending: true,
                weights_filter: WeightsFilter::Closed,
                reasoning_filter: ReasoningFilter::Reasoning,
                limit: Some(5),
            },
        );

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "alpha");
    }

    #[test]
    fn filter_entries_sorts_name_ascending() {
        let models = vec![
            make_row("beta", "Beta", "meta", "Meta", Some(80.0)),
            make_row("alpha", "Alpha", "openai", "OpenAI", Some(90.0)),
        ];

        let filtered = filter_entries(
            &models,
            &ListOptions {
                search: None,
                creator: None,
                sort: BenchmarkSort::Name,
                descending: false,
                weights_filter: WeightsFilter::All,
                reasoning_filter: ReasoningFilter::All,
                limit: None,
            },
        );

        assert_eq!(filtered[0].display_name, "Alpha");
        assert_eq!(filtered[1].display_name, "Beta");
    }

    #[test]
    fn resolve_entry_prefers_exact_slug_then_unique_partial() {
        let models = vec![
            make_row("gpt-4o", "GPT-4o", "openai", "OpenAI", Some(90.0)),
            make_row(
                "claude-sonnet-4",
                "Claude Sonnet 4",
                "anthropic",
                "Anthropic",
                Some(88.0),
            ),
        ];

        match resolve_entry(&models, "gpt-4o").unwrap() {
            ResolveEntry::Single(row) => assert_eq!(row.display_name, "GPT-4o"),
            ResolveEntry::Ambiguous(_) => panic!("expected exact slug to resolve to a single row"),
        }
        match resolve_entry(&models, "Sonnet").unwrap() {
            ResolveEntry::Single(row) => assert_eq!(row.display_name, "Claude Sonnet 4"),
            ResolveEntry::Ambiguous(_) => {
                panic!("expected unique partial match to resolve to a single row")
            }
        }
    }

    #[test]
    fn resolve_entry_returns_ambiguous_partial_matches() {
        let models = vec![
            make_row(
                "claude-sonnet-4",
                "Claude Sonnet 4",
                "anthropic",
                "Anthropic",
                Some(88.0),
            ),
            make_row(
                "claude-opus-4",
                "Claude Opus 4",
                "anthropic",
                "Anthropic",
                Some(89.0),
            ),
        ];

        match resolve_entry(&models, "Claude").unwrap() {
            ResolveEntry::Single(_) => panic!("expected ambiguous partial query"),
            ResolveEntry::Ambiguous(matches) => {
                assert_eq!(matches.len(), 2);
                assert_eq!(matches[0].id, "claude-opus-4");
                assert_eq!(matches[1].id, "claude-sonnet-4");
            }
        }
    }

    #[test]
    fn resolve_entry_returns_ambiguous_exact_display_matches() {
        let models = vec![
            make_row(
                "claude-sonnet-4-6-adaptive",
                "Claude Sonnet 4.6",
                "anthropic",
                "Anthropic",
                Some(88.0),
            ),
            make_row(
                "claude-sonnet-4-6-non-reasoning",
                "Claude Sonnet 4.6",
                "anthropic",
                "Anthropic",
                Some(82.0),
            ),
        ];

        match resolve_entry(&models, "Claude Sonnet 4.6").unwrap() {
            ResolveEntry::Single(_) => panic!("expected ambiguous exact display-name query"),
            ResolveEntry::Ambiguous(matches) => {
                assert_eq!(matches.len(), 2);
                assert_eq!(matches[0].id, "claude-sonnet-4-6-adaptive");
                assert_eq!(matches[1].id, "claude-sonnet-4-6-non-reasoning");
            }
        }
    }

    #[test]
    fn ambiguous_matches_message_lists_candidate_slugs() {
        let models = [
            make_row("alpha", "Alpha", "openai", "OpenAI", Some(90.0)),
            make_row("beta", "Beta", "openai", "OpenAI", Some(80.0)),
        ];
        let matches = [&models[0], &models[1]];

        let message = ambiguous_matches_message("a", &matches);
        assert!(message.contains("ambiguous"));
        assert!(message.contains("alpha"));
        assert!(message.contains("beta"));
    }

    #[test]
    fn filter_picker_entries_applies_live_query() {
        let models = [
            make_row(
                "claude-opus",
                "Claude Opus",
                "anthropic",
                "Anthropic",
                Some(90.0),
            ),
            make_row(
                "gpt-5-3-codex",
                "GPT-5.3 Codex",
                "openai",
                "OpenAI",
                Some(88.0),
            ),
        ];
        let selected = models.iter().collect::<Vec<_>>();

        let filtered = filter_picker_entries(&selected, "claude", BenchmarkSort::Name, false);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "claude-opus");
    }

    #[test]
    fn filter_picker_entries_resorts_by_requested_metric() {
        let models = [
            make_row("alpha", "Alpha", "openai", "OpenAI", Some(80.0)),
            make_row("beta", "Beta", "openai", "OpenAI", Some(90.0)),
        ];
        let selected = models.iter().collect::<Vec<_>>();

        let filtered = filter_picker_entries(&selected, "", BenchmarkSort::Intelligence, true);
        assert_eq!(filtered[0].id, "beta");
        assert_eq!(filtered[1].id, "alpha");
    }

    #[test]
    fn extract_reads_metric_from_score_map() {
        let row = make_row("x", "X", "openai", "OpenAI", Some(42.0));
        assert_eq!(BenchmarkSort::Intelligence.extract(&row), Some(42.0));
        assert_eq!(BenchmarkSort::Speed.extract(&row), Some(72.0));
        assert_eq!(BenchmarkSort::PriceBlended.extract(&row), Some(5.5));
        assert_eq!(BenchmarkSort::ReleaseDate.extract(&row), Some(20_250_101.0));
        assert_eq!(BenchmarkSort::Name.extract(&row), Some(0.0));

        let mut bare = make_row("y", "Y", "openai", "OpenAI", None);
        bare.scores.clear();
        assert_eq!(BenchmarkSort::Intelligence.extract(&bare), None);
    }

    #[test]
    fn print_detail_includes_key_sections() {
        let detail = BenchmarkDetail {
            slug: "gpt-4o",
            name: "GPT-4o",
            display_name: "GPT-4o",
            creator: "openai",
            creator_name: "OpenAI",
            creator_id: "",
            release_date: Some("2025-01-01"),
            open_weights: Some(false),
            reasoning: "Reasoning",
            effort_level: Some("high"),
            variant_tag: None,
            tool_call: Some(true),
            context_window: Some(200_000),
            max_output: Some(8_000),
            intelligence_index: Some(90.0),
            coding_index: Some(88.0),
            math_index: Some(87.0),
            mmlu_pro: Some(86.0),
            gpqa: Some(85.0),
            hle: Some(84.0),
            livecodebench: Some(83.0),
            scicode: Some(82.0),
            ifbench: Some(81.0),
            lcr: Some(80.0),
            terminalbench_hard: Some(79.0),
            tau2: Some(78.0),
            math_500: Some(77.0),
            aime: Some(76.0),
            aime_25: Some(75.0),
            output_tps: Some(74.0),
            ttft: Some(1.2),
            ttfat: Some(2.3),
            price_input: Some(3.4),
            price_output: Some(4.5),
            price_blended: Some(5.6),
        };

        let mut output = Vec::new();
        {
            use std::io::Write;

            writeln!(&mut output, "{}", detail.display_name).unwrap();
        }
        assert_eq!(String::from_utf8(output).unwrap().trim(), "GPT-4o");
        assert_eq!(format_open_weights(Some(false)), "Closed");
        assert_eq!(format_metric(Some(74.0)), "74.00");
    }
}
