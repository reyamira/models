use std::time::Duration;

use anyhow::{bail, Result};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use comfy_table::{presets::UTF8_FULL_CONDENSED, Table as ComfyTable};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell as TuiCell, HighlightSpacing, Paragraph, Row as TuiRow,
        Table as TuiTable, TableState, Wrap,
    },
    Frame,
};
use serde::Serialize;

use super::picker::{self, PickerTerminal};
use crate::formatting::truncate;
use crate::status::{
    registry::{status_seed_for_provider, STATUS_REGISTRY},
    ProviderHealth, ProviderStatus, StatusDetailAvailability, StatusDetailState, StatusFetchResult,
    StatusFetcher, StatusProvenance,
};

// ── CLI structure ───────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(name = "status")]
#[command(about = "Check AI provider service health")]
#[command(after_help = "\
\x1b[1;4mExamples:\x1b[0m
  status status                   Quick dashboard table
  status list                     Interactive provider health picker
  status list --json              All provider statuses as JSON
  status list --health degraded   Filter to degraded providers only
  status show openai              Detailed OpenAI status
  status show anthropic --json    Output as JSON")]
pub struct StatusCli {
    #[command(subcommand)]
    pub command: Option<StatusCommand>,
}

#[derive(Subcommand, Debug)]
pub enum StatusCommand {
    /// List all tracked provider health statuses
    #[command(after_help = "\
\x1b[1;4mExamples:\x1b[0m
  status list                     Interactive provider health picker
  status list --json              All provider statuses as JSON
  status list --health degraded   Filter to degraded providers")]
    List {
        /// Filter by health state
        #[arg(long, value_enum)]
        health: Option<HealthFilter>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show detailed status for a specific provider
    #[command(after_help = "\
\x1b[1;4mExamples:\x1b[0m
  status show openai              Detailed OpenAI status
  status show anthropic --json    Output as JSON")]
    Show {
        /// Provider slug (e.g., openai, anthropic, google)
        provider: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Print a quick status dashboard (always a table, no interaction)
    #[command(after_help = "\
\x1b[1;4mExamples:\x1b[0m
  status status                   Quick dashboard table
  status status --json            Dashboard as JSON")]
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Manage which providers are tracked
    #[command(after_help = "\
\x1b[1;4mExamples:\x1b[0m
  status sources                  Interactive source picker
  status sources --json           List all available sources as JSON")]
    Sources {
        /// Output as JSON (list all sources non-interactively)
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum HealthFilter {
    Operational,
    Degraded,
    Outage,
    Maintenance,
    Unknown,
}

// ── JSON serialization types ────────────────────────────────────────

#[derive(Serialize)]
struct StatusListItem<'a> {
    slug: &'a str,
    display_name: &'a str,
    health: &'static str,
    provenance: &'static str,
    source: Option<&'a str>,
    summary: Option<&'a str>,
    issues: usize,
    status_url: Option<&'a str>,
}

#[derive(Serialize)]
struct DetailStateJson {
    availability: &'static str,
}

impl DetailStateJson {
    fn from(state: &StatusDetailState) -> Self {
        let availability = match state.availability {
            StatusDetailAvailability::Available => "available",
            StatusDetailAvailability::NoneReported => "none_reported",
            StatusDetailAvailability::Unsupported => "unsupported",
            StatusDetailAvailability::FetchFailed => "fetch_failed",
            StatusDetailAvailability::NotAttempted => "not_attempted",
        };
        Self { availability }
    }
}

#[derive(Serialize)]
struct StatusDetailJson<'a> {
    slug: &'a str,
    display_name: &'a str,
    health: &'static str,
    provenance: &'static str,
    source: Option<&'a str>,
    source_method: Option<&'static str>,
    source_updated_at: Option<&'a str>,
    summary: Option<&'a str>,
    status_url: Option<&'a str>,
    issues: usize,
    components: Vec<ComponentJson<'a>>,
    components_state: DetailStateJson,
    incidents: Vec<IncidentJson<'a>>,
    incidents_state: DetailStateJson,
    scheduled_maintenances: Vec<MaintenanceJson<'a>>,
    scheduled_maintenances_state: DetailStateJson,
}

#[derive(Serialize)]
struct ComponentJson<'a> {
    name: &'a str,
    status: &'a str,
    group: Option<&'a str>,
}

#[derive(Serialize)]
struct IncidentJson<'a> {
    name: &'a str,
    status: &'a str,
    impact: &'a str,
    shortlink: Option<&'a str>,
    created_at: Option<&'a str>,
    updated_at: Option<&'a str>,
}

#[derive(Serialize)]
struct MaintenanceJson<'a> {
    name: &'a str,
    status: &'a str,
    scheduled_for: Option<&'a str>,
    scheduled_until: Option<&'a str>,
}

// ── Entry points ────────────────────────────────────────────────────

pub fn run() -> Result<()> {
    let cli = StatusCli::parse();
    dispatch(cli.command)
}

pub fn run_with_command(command: Option<StatusCommand>) -> Result<()> {
    dispatch(command)
}

fn dispatch(command: Option<StatusCommand>) -> Result<()> {
    match command {
        Some(StatusCommand::List { health, json }) => run_list(health, json),
        Some(StatusCommand::Show { provider, json }) => run_show(&provider, json),
        Some(StatusCommand::Status { json }) => run_status_table(json),
        Some(StatusCommand::Sources { json }) => run_sources(json),
        None => {
            StatusCli::command().print_long_help()?;
            println!();
            Ok(())
        }
    }
}

// ── Data fetching ───────────────────────────────────────────────────

fn fetch_all_statuses() -> Result<Vec<ProviderStatus>> {
    let seeds: Vec<_> = STATUS_REGISTRY
        .iter()
        .map(|e| status_seed_for_provider(e.slug))
        .collect();
    let client = reqwest::Client::builder()
        .user_agent("models-cli")
        .connect_timeout(Duration::from_secs(5))
        .build()?;
    let fetcher = StatusFetcher::with_client(client);
    let runtime = tokio::runtime::Runtime::new()?;
    let StatusFetchResult::Fresh(entries) = runtime.block_on(fetcher.fetch(&seeds));
    Ok(entries)
}

fn filter_by_tracked(entries: Vec<ProviderStatus>) -> Result<Vec<ProviderStatus>> {
    let config = crate::config::Config::load()?;
    Ok(entries
        .into_iter()
        .filter(|e| config.is_status_tracked(&e.slug))
        .collect())
}

fn filter_by_health(
    entries: Vec<ProviderStatus>,
    filter: Option<HealthFilter>,
) -> Vec<ProviderStatus> {
    let Some(filter) = filter else {
        return entries;
    };
    let target = match filter {
        HealthFilter::Operational => ProviderHealth::Operational,
        HealthFilter::Degraded => ProviderHealth::Degraded,
        HealthFilter::Outage => ProviderHealth::Outage,
        HealthFilter::Maintenance => ProviderHealth::Maintenance,
        HealthFilter::Unknown => ProviderHealth::Unknown,
    };
    entries.into_iter().filter(|e| e.health == target).collect()
}

// ── Helpers ─────────────────────────────────────────────────────────

fn health_label(health: ProviderHealth) -> &'static str {
    match health {
        ProviderHealth::Operational => "operational",
        ProviderHealth::Degraded => "degraded",
        ProviderHealth::Outage => "outage",
        ProviderHealth::Maintenance => "maintenance",
        ProviderHealth::Unknown => "unknown",
    }
}

fn provenance_label(provenance: StatusProvenance) -> &'static str {
    match provenance {
        StatusProvenance::Official => "official",
        StatusProvenance::Fallback => "fallback",
        StatusProvenance::Unavailable => "unavailable",
    }
}

fn health_icon_text(health: ProviderHealth) -> &'static str {
    match health {
        ProviderHealth::Operational => "\u{25CF} Ok",
        ProviderHealth::Degraded => "\u{25D0} Degraded",
        ProviderHealth::Outage => "\u{2717} Outage",
        ProviderHealth::Maintenance => "\u{25C6} Maint.",
        ProviderHealth::Unknown => "? Unknown",
    }
}

fn health_color(health: ProviderHealth) -> Color {
    match health {
        ProviderHealth::Operational => Color::Green,
        ProviderHealth::Degraded => Color::Yellow,
        ProviderHealth::Outage => Color::Red,
        ProviderHealth::Maintenance => Color::Blue,
        ProviderHealth::Unknown => Color::DarkGray,
    }
}

fn component_icon(status: &str) -> &'static str {
    let s = status.to_lowercase();
    if s.contains("operational") {
        "\u{25CF}"
    } else if s.contains("major_outage") || s.contains("outage") {
        "\u{2717}"
    } else if s.contains("degraded") || s.contains("partial") {
        "\u{25D0}"
    } else if s.contains("maint") {
        "\u{25C6}"
    } else {
        "?"
    }
}

fn component_color(status: &str) -> Color {
    let s = status.to_lowercase();
    if s.contains("operational") {
        Color::Green
    } else if s.contains("major_outage") || s.contains("outage") {
        Color::Red
    } else if s.contains("degraded") || s.contains("partial") {
        Color::Yellow
    } else if s.contains("maint") {
        Color::Blue
    } else {
        Color::DarkGray
    }
}

// ── list command ────────────────────────────────────────────────────

fn run_list(health: Option<HealthFilter>, json: bool) -> Result<()> {
    let entries = fetch_all_statuses()?;
    let entries = filter_by_tracked(entries)?;
    let entries = filter_by_health(entries, health);

    if json {
        let items: Vec<_> = entries
            .iter()
            .map(|e| StatusListItem {
                slug: &e.slug,
                display_name: &e.display_name,
                health: health_label(e.health),
                provenance: provenance_label(e.provenance),
                source: e.source_label.as_deref(),
                summary: e.provider_summary.as_deref(),
                issues: e.issue_count(),
                status_url: e.best_open_url(),
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    if entries.is_empty() {
        println!("No providers matched the current filters.");
        return Ok(());
    }

    if super::styles::is_tty() {
        run_picker(&entries)?;
        return Ok(());
    }

    print_list_table(&entries);
    Ok(())
}

fn print_list_table(entries: &[ProviderStatus]) {
    use super::styles;

    let mut table = ComfyTable::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    styles::arrange_table(&mut table);
    table.set_header(vec![
        styles::header_cell("Provider"),
        styles::header_cell("Health"),
        styles::header_cell("Issues"),
    ]);

    for entry in entries {
        let icon_text = health_icon_text(entry.health);
        let health_cell = match entry.health {
            ProviderHealth::Operational => styles::green_cell(icon_text),
            ProviderHealth::Degraded => styles::yellow_cell(icon_text),
            ProviderHealth::Outage => comfy_table::Cell::new(icon_text).fg(comfy_table::Color::Red),
            ProviderHealth::Maintenance => {
                comfy_table::Cell::new(icon_text).fg(comfy_table::Color::Blue)
            }
            ProviderHealth::Unknown => styles::dim_cell(icon_text),
        };

        let issues = entry.issue_count();
        let issues_cell = if issues > 0 {
            styles::yellow_cell(&issues.to_string())
        } else {
            comfy_table::Cell::new("0")
        };

        table.add_row(vec![
            styles::bold_cell(&entry.display_name),
            health_cell,
            issues_cell,
        ]);
    }

    println!("{table}");
}

// ── status table command ─────────────────────────────────────────────

fn run_status_table(json: bool) -> Result<()> {
    use super::styles;

    let entries = fetch_all_statuses()?;
    let entries = filter_by_tracked(entries)?;

    if json {
        let items: Vec<_> = entries
            .iter()
            .map(|e| StatusListItem {
                slug: &e.slug,
                display_name: &e.display_name,
                health: health_label(e.health),
                provenance: provenance_label(e.provenance),
                source: e.source_label.as_deref(),
                summary: e.provider_summary.as_deref(),
                issues: e.issue_count(),
                status_url: e.best_open_url(),
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    let mut table = ComfyTable::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    styles::arrange_table(&mut table);
    table.set_header(vec![
        styles::header_cell("Provider"),
        styles::header_cell("Health"),
        styles::header_cell("Issues"),
    ]);

    for entry in &entries {
        let icon_text = health_icon_text(entry.health);
        let health_cell = match entry.health {
            ProviderHealth::Operational => styles::green_cell(icon_text),
            ProviderHealth::Degraded => styles::yellow_cell(icon_text),
            ProviderHealth::Outage => comfy_table::Cell::new(icon_text).fg(comfy_table::Color::Red),
            ProviderHealth::Maintenance => {
                comfy_table::Cell::new(icon_text).fg(comfy_table::Color::Blue)
            }
            ProviderHealth::Unknown => styles::dim_cell(icon_text),
        };

        let issues = entry.issue_count();
        let issues_cell = if issues > 0 {
            styles::yellow_cell(&issues.to_string())
        } else {
            comfy_table::Cell::new("\u{2014}")
        };

        table.add_row(vec![
            styles::bold_cell(&entry.display_name),
            health_cell,
            issues_cell,
        ]);
    }

    println!("{table}");
    Ok(())
}

// ── show command ────────────────────────────────────────────────────

fn run_show(provider: &str, json: bool) -> Result<()> {
    let entries = fetch_all_statuses()?;
    let entry = resolve_provider(&entries, provider)?;

    if json {
        let detail = build_detail_json(entry);
        println!("{}", serde_json::to_string_pretty(&detail)?);
        return Ok(());
    }

    print_provider_detail(entry);
    Ok(())
}

fn resolve_provider<'a>(entries: &'a [ProviderStatus], query: &str) -> Result<&'a ProviderStatus> {
    let lower = query.to_lowercase();

    // Exact slug match
    if let Some(entry) = entries.iter().find(|e| e.slug == lower) {
        return Ok(entry);
    }

    // Case-insensitive display name match
    if let Some(entry) = entries
        .iter()
        .find(|e| e.display_name.to_lowercase() == lower)
    {
        return Ok(entry);
    }

    // Partial match
    let matches: Vec<_> = entries
        .iter()
        .filter(|e| e.slug.contains(&lower) || e.display_name.to_lowercase().contains(&lower))
        .collect();

    match matches.len() {
        0 => bail!(
            "Provider '{}' not found. Available: {}",
            query,
            entries
                .iter()
                .map(|e| e.slug.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        1 => Ok(matches[0]),
        _ => bail!(
            "Provider query '{}' was ambiguous. Matches: {}",
            query,
            matches
                .iter()
                .map(|e| e.slug.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn build_detail_json<'a>(entry: &'a ProviderStatus) -> StatusDetailJson<'a> {
    StatusDetailJson {
        slug: &entry.slug,
        display_name: &entry.display_name,
        health: health_label(entry.health),
        provenance: provenance_label(entry.provenance),
        source: entry.source_label.as_deref(),
        source_method: entry.source_method.map(|m| m.label()),
        source_updated_at: entry.source_updated_at.as_deref(),
        summary: entry.provider_summary.as_deref(),
        status_url: entry.best_open_url(),
        issues: entry.issue_count(),
        components: entry
            .components
            .iter()
            .map(|c| ComponentJson {
                name: &c.name,
                status: &c.status,
                group: c.group_name.as_deref(),
            })
            .collect(),
        components_state: DetailStateJson::from(&entry.components_state),
        incidents: entry
            .incidents
            .iter()
            .filter(|i| i.is_active())
            .map(|i| IncidentJson {
                name: &i.name,
                status: &i.status,
                impact: &i.impact,
                shortlink: i.shortlink.as_deref(),
                created_at: i.created_at.as_deref(),
                updated_at: i.updated_at.as_deref(),
            })
            .collect(),
        incidents_state: DetailStateJson::from(&entry.incidents_state),
        scheduled_maintenances: entry
            .scheduled_maintenances
            .iter()
            .map(|m| MaintenanceJson {
                name: &m.name,
                status: &m.status,
                scheduled_for: m.scheduled_for.as_deref(),
                scheduled_until: m.scheduled_until.as_deref(),
            })
            .collect(),
        scheduled_maintenances_state: DetailStateJson::from(&entry.scheduled_maintenances_state),
    }
}

fn print_provider_detail(entry: &ProviderStatus) {
    use super::styles;

    // Header
    println!(
        "{} \u{2014} {}",
        styles::agent_name(&entry.display_name),
        health_icon_text(entry.health)
    );
    println!("{}", styles::separator(40));

    // Current status — prominent display
    if let Some(summary) = &entry.provider_summary {
        println!();
        println!("  Current Status: {}", styles::key_value(summary));
    }

    // Metadata
    if let Some(url) = entry.best_open_url() {
        println!("  URL:            {}", styles::url(url));
    }
    if let Some(updated) = &entry.source_updated_at {
        println!("  Updated:        {updated}");
    }
    if let Some(note) = &entry.status_note {
        println!("  Note:           {}", styles::dim(note));
    }

    // Components — cap at 20 to handle Cloudflare-sized lists
    if entry.component_detail_available() && !entry.components.is_empty() {
        let total = entry.components.len();
        let cap = 20;
        println!();
        println!("  Components ({total}):");
        for comp in entry.components.iter().take(cap) {
            let icon = component_icon(&comp.status);
            let group_suffix = comp
                .group_name
                .as_deref()
                .map(|g| format!(" [{}]", g))
                .unwrap_or_default();
            println!("    {icon} {}{group_suffix}", comp.name);
        }
        if total > cap {
            println!("    {} ... and {} more", styles::dim("+"), total - cap);
        }
    } else if entry.confirmed_no_components() {
        println!();
        println!("  Components: {}", styles::dim("None reported"));
    }

    // Active incidents
    let active = entry.active_incidents();
    println!();
    if active.is_empty() {
        println!(
            "  Active Incidents: {}",
            styles::dim(if entry.incident_detail_available() {
                "None"
            } else {
                "Unavailable"
            })
        );
    } else {
        println!("  Active Incidents ({}):", active.len());
        for incident in &active {
            println!(
                "    \u{2022} {} [{}]",
                styles::key_value(&incident.name),
                incident.status
            );
            if let Some(link) = &incident.shortlink {
                println!("      {}", styles::url(link));
            }
        }
    }

    // Scheduled maintenance
    println!();
    if entry.scheduled_maintenances.is_empty() {
        println!(
            "  Scheduled Maintenance: {}",
            styles::dim(if entry.maintenance_detail_available() {
                "None"
            } else {
                "Unavailable"
            })
        );
    } else {
        println!(
            "  Scheduled Maintenance ({}):",
            entry.scheduled_maintenances.len()
        );
        for maint in &entry.scheduled_maintenances {
            let schedule = match (&maint.scheduled_for, &maint.scheduled_until) {
                (Some(from), Some(until)) => format!(" ({from} \u{2192} {until})"),
                (Some(from), None) => format!(" (from {from})"),
                _ => String::new(),
            };
            println!(
                "    \u{25C7} {} [{}]{schedule}",
                styles::key_value(&maint.name),
                maint.status
            );
        }
    }

    // Errors
    if let Some(err) = entry.error_summary() {
        println!();
        println!("  {}", styles::dim(&format!("Fetch errors: {err}")));
    }

    println!();
}

// ── sources command ─────────────────────────────────────────────────

#[derive(Serialize)]
struct SourceItem {
    slug: &'static str,
    display_name: &'static str,
    tracked: bool,
}

fn print_sources_table(items: &[SourceItem]) {
    use super::styles;

    let mut table = ComfyTable::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    styles::arrange_table(&mut table);
    table.set_header(vec![
        styles::header_cell("Slug"),
        styles::header_cell("Provider"),
        styles::header_cell("Tracked"),
    ]);

    for item in items {
        let tracked_cell = if item.tracked {
            styles::green_cell("yes")
        } else {
            styles::dim_cell("no")
        };
        table.add_row(vec![
            comfy_table::Cell::new(item.slug),
            styles::bold_cell(item.display_name),
            tracked_cell,
        ]);
    }

    println!("{table}");
}

fn run_sources(json: bool) -> Result<()> {
    let config = crate::config::Config::load()?;

    if json {
        let items: Vec<_> = STATUS_REGISTRY
            .iter()
            .map(|e| SourceItem {
                slug: e.slug,
                display_name: e.display_name,
                tracked: config.is_status_tracked(e.slug),
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    if !super::styles::is_tty() {
        let items: Vec<_> = STATUS_REGISTRY
            .iter()
            .map(|e| SourceItem {
                slug: e.slug,
                display_name: e.display_name,
                tracked: config.is_status_tracked(e.slug),
            })
            .collect();
        print_sources_table(&items);
        return Ok(());
    }

    // Fetch live status so the picker can show health info
    let statuses = fetch_all_statuses()?;

    let items: Vec<SourcePickerItem> = STATUS_REGISTRY
        .iter()
        .map(|e| {
            let status = statuses.iter().find(|s| s.slug == e.slug);
            SourcePickerItem {
                slug: e.slug,
                display_name: e.display_name,
                tracked: config.is_status_tracked(e.slug),
                health: status.map(|s| s.health).unwrap_or(ProviderHealth::Unknown),
                issues: status.map(|s| s.issue_count()).unwrap_or(0),
                summary: status.and_then(|s| s.provider_summary.clone()),
                status_url: status.and_then(|s| s.best_open_url().map(String::from)),
            }
        })
        .collect();

    if let Some(updated) = run_source_picker(&items)? {
        let mut config = config;
        for item in &updated {
            config.set_status_tracked(item.slug, item.tracked);
        }
        config.save()?;
        let tracked_count = updated.iter().filter(|i| i.tracked).count();
        println!(
            "Saved — tracking {tracked_count}/{} providers.",
            updated.len()
        );
    }

    Ok(())
}

#[derive(Clone)]
struct SourcePickerItem {
    slug: &'static str,
    display_name: &'static str,
    tracked: bool,
    health: ProviderHealth,
    issues: usize,
    summary: Option<String>,
    status_url: Option<String>,
}

struct StatusSourcePicker {
    items: Vec<SourcePickerItem>,
    state: TableState,
}

impl StatusSourcePicker {
    fn new(items: Vec<SourcePickerItem>) -> Self {
        let mut state = TableState::default();
        if !items.is_empty() {
            state.select(Some(0));
        }
        Self { items, state }
    }

    fn next(&mut self) {
        picker::nav_next(&mut self.state, self.items.len());
    }
    fn previous(&mut self) {
        picker::nav_previous(&mut self.state);
    }
    fn first(&mut self) {
        picker::nav_first(&mut self.state, self.items.len());
    }
    fn last(&mut self) {
        picker::nav_last(&mut self.state, self.items.len());
    }
    fn page_down(&mut self) {
        picker::nav_page_down(&mut self.state, self.items.len(), 10);
    }
    fn page_up(&mut self) {
        picker::nav_page_up(&mut self.state, 10);
    }

    fn toggle_current(&mut self) {
        if let Some(idx) = self.state.selected() {
            if let Some(item) = self.items.get_mut(idx) {
                item.tracked = !item.tracked;
            }
        }
    }

    fn selected(&self) -> Option<&SourcePickerItem> {
        self.state.selected().map(|idx| &self.items[idx])
    }
}

fn run_source_picker(items: &[SourcePickerItem]) -> Result<Option<Vec<SourcePickerItem>>> {
    let mut pt = PickerTerminal::new()?;
    let mut picker = StatusSourcePicker::new(items.to_vec());

    loop {
        pt.terminal.draw(|f| draw_source_picker(f, &mut picker))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(None),
            KeyCode::Char('j') | KeyCode::Down => picker.next(),
            KeyCode::Char('k') | KeyCode::Up => picker.previous(),
            KeyCode::Char('g') => picker.first(),
            KeyCode::Char('G') | KeyCode::End => picker.last(),
            KeyCode::PageDown => picker.page_down(),
            KeyCode::PageUp => picker.page_up(),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                picker.page_down();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                picker.page_up();
            }
            KeyCode::Char(' ') => picker.toggle_current(),
            KeyCode::Enter => return Ok(Some(picker.items)),
            _ => {}
        }
    }
}

fn draw_source_picker(f: &mut Frame, picker: &mut StatusSourcePicker) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(1)])
        .split(f.area());

    let inner = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(outer[0]);

    let tracked_count = picker.items.iter().filter(|i| i.tracked).count();
    let title = format!(
        " Sources ({}/{} tracked) ",
        tracked_count,
        picker.items.len()
    );

    let header = TuiRow::new(vec![
        TuiCell::from("Track"),
        TuiCell::from("Provider"),
        TuiCell::from("Health"),
    ])
    .style(picker::HEADER_STYLE);

    let rows: Vec<TuiRow> = picker
        .items
        .iter()
        .map(|item| {
            let checkbox = if item.tracked { "[x]" } else { "[ ]" };
            TuiRow::new(vec![
                TuiCell::from(checkbox),
                TuiCell::from(item.display_name),
                TuiCell::from(health_icon_text(item.health))
                    .style(Style::default().fg(health_color(item.health))),
            ])
        })
        .collect();

    let table = TuiTable::new(
        rows,
        [
            Constraint::Length(5),
            Constraint::Percentage(55),
            Constraint::Length(14),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(picker::ACTIVE_BORDER_STYLE)
            .title(title),
    )
    .row_highlight_style(picker::ROW_HIGHLIGHT_STYLE)
    .highlight_symbol(picker::HIGHLIGHT_SYMBOL)
    .highlight_spacing(HighlightSpacing::Always);

    f.render_stateful_widget(table, inner[0], &mut picker.state);

    // Preview: show selected provider health info
    let preview_lines = if let Some(item) = picker.selected() {
        let tracked_label = if item.tracked {
            "Tracked"
        } else {
            "Not tracked"
        };
        let tracked_color = if item.tracked {
            Color::Green
        } else {
            Color::DarkGray
        };

        let mut lines = vec![
            Line::from(vec![
                Span::styled(
                    item.display_name,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" \u{2014} "),
                Span::styled(
                    health_icon_text(item.health),
                    Style::default().fg(health_color(item.health)),
                ),
            ]),
            Line::from(vec![
                Span::styled("Slug: ", Style::default().fg(Color::Gray)),
                Span::raw(item.slug),
            ]),
            Line::from(Span::styled(
                tracked_label,
                Style::default().fg(tracked_color),
            )),
        ];

        // Current status summary
        if let Some(summary) = &item.summary {
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![
                Span::styled("Status: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    summary.as_str(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }

        // Issue count
        if item.issues > 0 {
            lines.push(Line::from(vec![
                Span::styled("Issues: ", Style::default().fg(Color::Gray)),
                Span::styled(item.issues.to_string(), Style::default().fg(Color::Yellow)),
            ]));
        }

        // URL
        if let Some(url) = &item.status_url {
            lines.push(Line::from(vec![
                Span::styled("URL:    ", Style::default().fg(Color::Gray)),
                Span::styled(url.as_str(), Style::default().fg(Color::Cyan)),
            ]));
        }

        lines
    } else {
        vec![Line::from(Span::styled(
            "No provider selected",
            Style::default().fg(Color::DarkGray),
        ))]
    };

    let preview = Paragraph::new(preview_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(picker::PREVIEW_BORDER_STYLE)
                .title(" Provider "),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(preview, inner[1]);

    // Status bar
    let status = Line::from(vec![
        Span::styled(" Space ", Style::default().fg(Color::Yellow)),
        Span::raw("toggle  "),
        Span::styled(" Enter ", Style::default().fg(Color::Yellow)),
        Span::raw("save  "),
        Span::styled(" q/Esc ", Style::default().fg(Color::Yellow)),
        Span::raw("cancel  "),
        Span::styled(" \u{2191}\u{2193}/j/k ", Style::default().fg(Color::Yellow)),
        Span::raw("move"),
    ]);
    f.render_widget(Paragraph::new(status), outer[1]);
}

// ── Interactive picker ──────────────────────────────────────────────

struct StatusPicker<'a> {
    entries: &'a [ProviderStatus],
    visible: Vec<usize>,
    query: String,
    filter_mode: bool,
    state: TableState,
}

impl<'a> StatusPicker<'a> {
    fn new(entries: &'a [ProviderStatus]) -> Self {
        let visible: Vec<usize> = (0..entries.len()).collect();
        let mut state = TableState::default();
        if !visible.is_empty() {
            state.select(Some(0));
        }
        Self {
            entries,
            visible,
            query: String::new(),
            filter_mode: false,
            state,
        }
    }

    fn rebuild_visible(&mut self) {
        let old_slug = self.selected().map(|e| e.slug.clone());

        if self.query.is_empty() {
            self.visible = (0..self.entries.len()).collect();
        } else {
            let lower = self.query.to_lowercase();
            self.visible = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.display_name.to_lowercase().contains(&lower) || e.slug.contains(&lower)
                })
                .map(|(i, _)| i)
                .collect();
        }

        // Restore selection if possible
        if let Some(slug) = old_slug {
            if let Some(pos) = self
                .visible
                .iter()
                .position(|&i| self.entries[i].slug == slug)
            {
                self.state.select(Some(pos));
                return;
            }
        }
        self.state.select(if self.visible.is_empty() {
            None
        } else {
            Some(0)
        });
    }

    fn selected(&self) -> Option<&'a ProviderStatus> {
        self.state
            .selected()
            .and_then(|i| self.visible.get(i))
            .map(|&idx| &self.entries[idx])
    }

    fn next(&mut self) {
        picker::nav_next(&mut self.state, self.visible.len());
    }
    fn previous(&mut self) {
        picker::nav_previous(&mut self.state);
    }
    fn first(&mut self) {
        picker::nav_first(&mut self.state, self.visible.len());
    }
    fn last(&mut self) {
        picker::nav_last(&mut self.state, self.visible.len());
    }
    fn page_down(&mut self) {
        picker::nav_page_down(&mut self.state, self.visible.len(), 10);
    }
    fn page_up(&mut self) {
        picker::nav_page_up(&mut self.state, 10);
    }
}

fn run_picker(entries: &[ProviderStatus]) -> Result<()> {
    let mut pt = PickerTerminal::new()?;
    let mut picker = StatusPicker::new(entries);

    loop {
        pt.terminal.draw(|f| draw_picker(f, &mut picker))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        if picker.filter_mode {
            match key.code {
                KeyCode::Esc => {
                    picker.filter_mode = false;
                    picker.query.clear();
                    picker.rebuild_visible();
                }
                KeyCode::Enter => {
                    picker.filter_mode = false;
                }
                KeyCode::Backspace => {
                    picker.query.pop();
                    picker.rebuild_visible();
                }
                KeyCode::Char(c) => {
                    picker.query.push(c);
                    picker.rebuild_visible();
                }
                _ => {}
            }
            continue;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => break,
            KeyCode::Char('j') | KeyCode::Down => picker.next(),
            KeyCode::Char('k') | KeyCode::Up => picker.previous(),
            KeyCode::Char('g') => picker.first(),
            KeyCode::Char('G') | KeyCode::End => picker.last(),
            KeyCode::PageDown => picker.page_down(),
            KeyCode::PageUp => picker.page_up(),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                picker.page_down();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                picker.page_up();
            }
            KeyCode::Char('/') => {
                picker.filter_mode = true;
            }
            KeyCode::Enter => {
                if let Some(entry) = picker.selected() {
                    if let Some(url) = entry.best_open_url() {
                        drop(pt);
                        let _ = open::that(url);
                        return Ok(());
                    }
                }
            }
            _ => {}
        }
    }

    Ok(())
}

fn draw_picker(f: &mut Frame, picker: &mut StatusPicker) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(1)])
        .split(f.area());

    let inner = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(outer[0]);

    draw_table(f, inner[0], picker);
    draw_preview(f, inner[1], picker);
    draw_status_bar(f, outer[1], picker);
}

fn draw_table(f: &mut Frame, area: ratatui::layout::Rect, picker: &mut StatusPicker) {
    let title = if picker.query.is_empty() {
        format!(" Status ({} providers) ", picker.visible.len())
    } else {
        format!(
            " Status ({} / {} providers) | / {} ",
            picker.visible.len(),
            picker.entries.len(),
            picker.query,
        )
    };

    let header = TuiRow::new(vec![
        TuiCell::from("Provider"),
        TuiCell::from("Health"),
        TuiCell::from("Issues"),
    ])
    .style(picker::HEADER_STYLE);

    let rows: Vec<TuiRow> = picker
        .visible
        .iter()
        .map(|&idx| {
            let entry = &picker.entries[idx];
            let issues = entry.issue_count();
            TuiRow::new(vec![
                TuiCell::from(entry.display_name.as_str())
                    .style(Style::default().add_modifier(Modifier::BOLD)),
                TuiCell::from(health_icon_text(entry.health))
                    .style(Style::default().fg(health_color(entry.health))),
                TuiCell::from(if issues > 0 {
                    issues.to_string()
                } else {
                    "\u{2014}".to_string()
                })
                .style(if issues > 0 {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default().fg(Color::DarkGray)
                }),
            ])
        })
        .collect();

    let table = TuiTable::new(
        rows,
        [
            Constraint::Percentage(45),
            Constraint::Length(14),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(picker::ACTIVE_BORDER_STYLE)
            .title(title),
    )
    .row_highlight_style(picker::ROW_HIGHLIGHT_STYLE)
    .highlight_symbol(picker::HIGHLIGHT_SYMBOL)
    .highlight_spacing(HighlightSpacing::Always);

    f.render_stateful_widget(table, area, &mut picker.state);
}

fn draw_preview(f: &mut Frame, area: ratatui::layout::Rect, picker: &StatusPicker) {
    let Some(entry) = picker.selected() else {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(picker::PREVIEW_BORDER_STYLE)
            .title(" Detail ");
        let para = Paragraph::new("No provider selected")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        f.render_widget(para, area);
        return;
    };

    let mut lines: Vec<Line> = Vec::new();

    // Header
    lines.push(Line::from(vec![
        Span::styled(
            &entry.display_name,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" \u{2014} "),
        Span::styled(
            health_icon_text(entry.health),
            Style::default().fg(health_color(entry.health)),
        ),
    ]));

    // Current status — prominent
    if let Some(summary) = &entry.provider_summary {
        lines.push(Line::from(vec![
            Span::styled("Status: ", Style::default().fg(Color::Gray)),
            Span::styled(
                summary.as_str(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    if let Some(url) = entry.best_open_url() {
        lines.push(Line::from(vec![
            Span::styled("URL:    ", Style::default().fg(Color::Gray)),
            Span::styled(url, Style::default().fg(Color::Cyan)),
        ]));
    }

    let active = entry.active_incidents();
    let has_issues = !active.is_empty()
        || matches!(
            entry.health,
            ProviderHealth::Degraded | ProviderHealth::Outage
        );

    // When there are active issues, show incidents prominently and skip components
    if has_issues {
        // Active incidents — the main focus
        if !active.is_empty() {
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                format!("Active Incidents ({})", active.len()),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
            for incident in active.iter().take(5) {
                lines.push(Line::from(vec![
                    Span::raw("  \u{2022} "),
                    Span::styled(
                        incident.name.as_str(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        format!("{} \u{2014} {}", incident.status, incident.impact),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
                if !incident.affected_components.is_empty() {
                    let affected = incident.affected_components.join(", ");
                    lines.push(Line::from(vec![
                        Span::styled("    Affected: ", Style::default().fg(Color::Gray)),
                        Span::raw(truncate(&affected, 40)),
                    ]));
                }
            }
        }

        // Degraded components (only those not operational)
        if entry.component_detail_available() {
            let degraded: Vec<_> = entry
                .components
                .iter()
                .filter(|c| {
                    let s = c.status.to_lowercase();
                    !s.contains("operational") && !s.is_empty()
                })
                .collect();
            if !degraded.is_empty() && active.is_empty() {
                lines.push(Line::raw(""));
                lines.push(Line::from(Span::styled(
                    format!("Affected Services ({})", degraded.len()),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )));
                for comp in degraded.iter().take(6) {
                    let icon = component_icon(&comp.status);
                    let color = component_color(&comp.status);
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(icon, Style::default().fg(color)),
                        Span::raw(format!(" {}", truncate(&comp.name, 30))),
                    ]));
                }
            }
        }
    } else {
        // Operational — brief component summary
        if entry.component_detail_available() && !entry.components.is_empty() {
            let total = entry.components.len();
            let operational = entry
                .components
                .iter()
                .filter(|c| c.status.to_lowercase().contains("operational"))
                .count();
            lines.push(Line::raw(""));
            if operational == total {
                lines.push(Line::from(Span::styled(
                    format!("{total} services \u{2014} all operational"),
                    Style::default().fg(Color::Green),
                )));
            } else {
                let other = total - operational;
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{total} services: "),
                        Style::default().fg(Color::Gray),
                    ),
                    Span::styled(
                        format!("{operational} operational"),
                        Style::default().fg(Color::Green),
                    ),
                    Span::raw(", "),
                    Span::styled(format!("{other} other"), Style::default().fg(Color::Yellow)),
                ]));
            }
        }
    }

    // Scheduled maintenance (always show if present)
    if !entry.scheduled_maintenances.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            format!("Maintenance ({})", entry.scheduled_maintenances.len()),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        for maint in entry.scheduled_maintenances.iter().take(3) {
            lines.push(Line::from(vec![
                Span::raw("  \u{25C7} "),
                Span::styled(
                    maint.name.as_str(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" [{}]", maint.status),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(picker::PREVIEW_BORDER_STYLE)
        .title(" Detail ");

    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(para, area);
}

fn draw_status_bar(f: &mut Frame, area: ratatui::layout::Rect, picker: &StatusPicker) {
    let line = if picker.filter_mode {
        Line::from(vec![
            Span::styled("Filter: ", Style::default().fg(Color::Cyan)),
            Span::raw(&picker.query),
            Span::styled("_", Style::default().add_modifier(Modifier::SLOW_BLINK)),
            Span::raw("  "),
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::raw(" apply  "),
            Span::styled("Esc", Style::default().fg(Color::Yellow)),
            Span::raw(" clear  "),
            Span::styled("Backspace", Style::default().fg(Color::Yellow)),
            Span::raw(" delete"),
        ])
    } else {
        Line::from(vec![
            Span::styled(" Enter ", Style::default().fg(Color::Yellow)),
            Span::raw("open  "),
            Span::styled(" / ", Style::default().fg(Color::Yellow)),
            Span::raw("filter  "),
            Span::styled(" q ", Style::default().fg(Color::Yellow)),
            Span::raw("quit  "),
            Span::styled(" \u{2191}\u{2193}/j/k ", Style::default().fg(Color::Yellow)),
            Span::raw("move"),
        ])
    };
    f.render_widget(Paragraph::new(line), area);
}
