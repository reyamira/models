use std::time::Duration;

use anyhow::{bail, Result};
use comfy_table::{presets::UTF8_FULL_CONDENSED, Table as ComfyTable};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell as TuiCell, HighlightSpacing, Paragraph, Row as TuiRow,
        Table as TuiTable, TableState,
    },
    Frame,
};
use serde::Serialize;

use crate::formatting::{cmp_opt_f64, parse_date_to_numeric, truncate};
use crate::{
    api,
    data::{CostTier, Model as ApiModel, ReasoningOption},
};

use super::picker::{self, PickerTerminal};
const PICKER_SORTS: [ModelSort; 6] = [
    ModelSort::ReleaseDate,
    ModelSort::Name,
    ModelSort::Provider,
    ModelSort::Context,
    ModelSort::InputPrice,
    ModelSort::OutputPrice,
];

#[derive(Debug, Clone, Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub models_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelRow {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub provider_name: String,
    pub display_id: String,
    pub context: String,
    pub output: String,
    pub cost: String,
    pub capabilities: String,
    pub modalities: String,
    pub family: Option<String>,
    pub input_cost: Option<f64>,
    pub output_cost: Option<f64>,
    pub cache_read_cost: Option<f64>,
    pub cache_write_cost: Option<f64>,
    pub reasoning_cost: Option<f64>,
    pub input_audio_cost: Option<f64>,
    pub output_audio_cost: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tiers: Vec<CostTier>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reasoning_options: Vec<ReasoningOption>,
    pub reasoning: bool,
    pub tool_call: bool,
    pub attachment: bool,
    pub structured_output: Option<bool>,
    pub description: Option<String>,
    pub release_date: Option<String>,
    pub last_updated: Option<String>,
    pub knowledge_cutoff: Option<String>,
    pub open_weights: bool,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelDetail {
    pub id: String,
    pub name: String,
    pub provider_id: String,
    pub provider_name: String,
    pub family: Option<String>,
    pub context: String,
    pub output: String,
    pub input_cost: Option<f64>,
    pub output_cost: Option<f64>,
    pub cache_read_cost: Option<f64>,
    pub cache_write_cost: Option<f64>,
    pub reasoning_cost: Option<f64>,
    pub input_audio_cost: Option<f64>,
    pub output_audio_cost: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tiers: Vec<CostTier>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reasoning_options: Vec<ReasoningOption>,
    pub reasoning: bool,
    pub tool_call: bool,
    pub attachment: bool,
    pub structured_output: Option<bool>,
    pub description: Option<String>,
    pub modalities: String,
    pub release_date: Option<String>,
    pub last_updated: Option<String>,
    pub knowledge_cutoff: Option<String>,
    pub open_weights: bool,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSort {
    Name,
    Provider,
    Context,
    InputPrice,
    OutputPrice,
    ReleaseDate,
}

impl ModelSort {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Provider => "Provider",
            Self::Context => "Context",
            Self::InputPrice => "Input $/M",
            Self::OutputPrice => "Output $/M",
            Self::ReleaseDate => "Release",
        }
    }

    pub fn default_descending(self) -> bool {
        matches!(self, Self::Context | Self::ReleaseDate)
    }

    fn extract(self, row: &ModelRow) -> Option<f64> {
        match self {
            Self::Name | Self::Provider => Some(0.0),
            Self::Context => parse_token_count(&row.context),
            Self::InputPrice => row.input_cost,
            Self::OutputPrice => row.output_cost,
            Self::ReleaseDate => row.release_date.as_deref().and_then(parse_date_to_numeric),
        }
    }
}

pub enum ResolveModel {
    Single(Box<ModelRow>),
    Ambiguous(Vec<ModelRow>),
}

struct ModelPicker {
    entries: Vec<ModelRow>,
    visible_entries: Vec<ModelRow>,
    sort: ModelSort,
    descending: bool,
    title: String,
    query: String,
    filter_mode: bool,
    state: TableState,
    copied_at: Option<std::time::Instant>,
}

impl ModelPicker {
    fn new(entries: Vec<ModelRow>, sort: ModelSort, descending: bool, title: String) -> Self {
        let mut picker = Self {
            entries,
            visible_entries: Vec::new(),
            sort,
            descending,
            title,
            query: String::new(),
            filter_mode: false,
            state: TableState::default(),
            copied_at: None,
        };
        picker.rebuild_visible_entries(None);
        picker
    }

    fn selected(&self) -> Option<&ModelRow> {
        self.state.selected().map(|idx| &self.visible_entries[idx])
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
        let preserve = self.selected().map(|row| row.display_id.clone());
        let current_idx = PICKER_SORTS
            .iter()
            .position(|&sort| sort == self.sort)
            .unwrap_or(0);
        self.sort = PICKER_SORTS[(current_idx + 1) % PICKER_SORTS.len()];
        self.descending = self.sort.default_descending();
        self.rebuild_visible_entries(preserve.as_deref());
    }

    fn toggle_descending(&mut self) {
        let preserve = self.selected().map(|row| row.display_id.clone());
        self.descending = !self.descending;
        self.rebuild_visible_entries(preserve.as_deref());
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
        let preserve = self.selected().map(|row| row.display_id.clone());
        self.query.push(ch);
        self.rebuild_visible_entries(preserve.as_deref());
    }

    fn pop_filter_char(&mut self) {
        let preserve = self.selected().map(|row| row.display_id.clone());
        self.query.pop();
        self.rebuild_visible_entries(preserve.as_deref());
    }

    fn rebuild_visible_entries(&mut self, preserve_id: Option<&str>) {
        self.visible_entries =
            filter_picker_entries(&self.entries, &self.query, self.sort, self.descending);
        let next_selected = preserve_id
            .and_then(|id| {
                self.visible_entries
                    .iter()
                    .position(|entry| entry.display_id == id)
            })
            .or_else(|| (!self.visible_entries.is_empty()).then_some(0));
        self.state.select(next_selected);
    }

    fn draw(&mut self, frame: &mut Frame<'_>) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(7),
                Constraint::Length(5),
                Constraint::Length(1),
            ])
            .split(frame.area());

        let rows = self.visible_entries.iter().map(|entry| {
            TuiRow::new(vec![
                TuiCell::from(truncate(&entry.name, 26)),
                TuiCell::from(truncate(&entry.provider_name, 14)),
                TuiCell::from(truncate(&format_picker_sort_value(self.sort, entry), 12)),
                TuiCell::from(truncate(&entry.cost, 14)),
                TuiCell::from(truncate(&entry.capabilities, 18)),
                TuiCell::from(
                    entry
                        .release_date
                        .clone()
                        .unwrap_or_else(|| "\u{2014}".to_string()),
                ),
            ])
        });

        let table = TuiTable::new(
            rows,
            [
                Constraint::Percentage(28),
                Constraint::Percentage(15),
                Constraint::Percentage(12),
                Constraint::Percentage(15),
                Constraint::Percentage(18),
                Constraint::Percentage(12),
            ],
        )
        .header(
            TuiRow::new(vec![
                "Name",
                "Provider",
                picker_sort_label(self.sort),
                "Cost",
                "Capabilities",
                "Release",
            ])
            .style(picker::HEADER_STYLE),
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

        frame.render_stateful_widget(table, chunks[0], &mut self.state);
        frame.render_widget(
            Paragraph::new(self.preview_lines()).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(picker::PREVIEW_BORDER_STYLE)
                    .title(" Preview "),
            ),
            chunks[1],
        );
        frame.render_widget(Paragraph::new(self.status_line()), chunks[2]);
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
        let Some(entry) = self.selected() else {
            return vec![
                Line::from("No matches"),
                Line::from(""),
                Line::from("Adjust the filter or clear it with Esc while filtering."),
            ];
        };
        vec![
            Line::from(format!(
                "id: {}   provider: {}",
                truncate(&entry.display_id, 36),
                entry.provider_name
            )),
            Line::from(format!(
                "context: {}   output: {}   open: {}",
                entry.context,
                entry.output,
                if entry.open_weights { "yes" } else { "no" }
            )),
            Line::from(format!(
                "input: {}   output: {}   reasoning: {}   tools: {}",
                format_optional_price(entry.input_cost),
                format_optional_price(entry.output_cost),
                yes_no(entry.reasoning),
                yes_no(entry.tool_call),
            )),
            Line::from(format!(
                "files: {}   modalities: {}",
                yes_no(entry.attachment),
                truncate(&entry.modalities, 44)
            )),
        ]
    }

    fn status_line(&self) -> Line<'static> {
        if self.filter_mode {
            Line::from(format!(
                "Filter: {}_  Enter apply  Esc clear  Backspace delete",
                self.query
            ))
        } else if self
            .copied_at
            .is_some_and(|t| t.elapsed().as_millis() < 1500)
        {
            Line::from(Span::styled(
                "Copied to clipboard!",
                Style::default().fg(Color::Green),
            ))
        } else {
            Line::from(
                "Enter inspect   / filter   s sort   S reverse   c copy   q quit   ↑↓/j/k move",
            )
        }
    }
}

pub fn providers(json: bool) -> Result<()> {
    let providers = api::fetch_providers()?;
    let mut infos: Vec<ProviderInfo> = providers
        .values()
        .map(|provider| ProviderInfo {
            id: provider.id.clone(),
            name: provider.name.clone(),
            models_count: provider.models.len(),
        })
        .collect();
    infos.sort_by(|a, b| a.id.cmp(&b.id));

    if json {
        println!("{}", serde_json::to_string_pretty(&infos)?);
        return Ok(());
    }

    let mut table = ComfyTable::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(vec!["ID", "Name", "Models"]);
    for info in infos {
        table.add_row(vec![info.id, info.name, info.models_count.to_string()]);
    }
    println!("{table}");
    Ok(())
}

pub fn list(provider: Option<&str>, json: bool) -> Result<()> {
    let rows = load_model_rows(provider)?;
    if rows.is_empty() {
        bail!("No models found");
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    if super::styles::is_tty() {
        let title = " Model Picker ".to_string();
        if let Some(row) = pick_model(rows, ModelSort::ReleaseDate, true, &title)? {
            print_model_detail(&row, false)?;
        }
        return Ok(());
    }

    print_model_table(&rows, ModelSort::ReleaseDate);
    Ok(())
}

pub fn show(query: &str, json: bool) -> Result<()> {
    match resolve_model(query)? {
        ResolveModel::Single(row) => print_model_detail(&row, json),
        ResolveModel::Ambiguous(rows) => {
            if json || !super::styles::is_tty() {
                bail!("{}", ambiguous_model_matches_message(query, &rows));
            }
            let title = format!(" Select Model Match for \"{query}\" ");
            if let Some(row) = pick_model(rows, ModelSort::ReleaseDate, true, &title)? {
                print_model_detail(&row, false)?;
            }
            Ok(())
        }
    }
}

pub fn search(query: &str, json: bool) -> Result<()> {
    let rows = load_model_rows(None)?;
    let filtered = filter_picker_entries(&rows, query, ModelSort::ReleaseDate, true);
    if filtered.is_empty() {
        println!("No models found matching '{}'", query);
        return Ok(());
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&filtered)?);
        return Ok(());
    }

    if super::styles::is_tty() {
        let title = " Model Search ".to_string();
        if let Some(row) =
            pick_model_with_query(filtered, ModelSort::ReleaseDate, true, &title, query)?
        {
            print_model_detail(&row, false)?;
        }
        return Ok(());
    }

    print_model_table(&filtered, ModelSort::ReleaseDate);
    Ok(())
}

pub fn load_model_rows(provider: Option<&str>) -> Result<Vec<ModelRow>> {
    let providers = api::fetch_providers()?;
    let provider = provider.map(str::to_lowercase);
    let mut rows = Vec::new();

    for provider_data in providers.values() {
        if let Some(filter) = &provider {
            let id_matches = provider_data.id.to_lowercase() == *filter;
            let name_matches = provider_data.name.to_lowercase() == *filter;
            if !id_matches && !name_matches {
                continue;
            }
        }

        for model in provider_data.models.values() {
            rows.push(flatten_model_row(
                &provider_data.id,
                &provider_data.name,
                model,
            ));
        }
    }

    rows.sort_by(|a, b| {
        a.provider
            .cmp(&b.provider)
            .then_with(|| a.id.cmp(&b.id))
            .then_with(|| a.name.cmp(&b.name))
    });

    if let Some(filter) = provider {
        if rows.is_empty() {
            bail!("Provider '{}' not found", filter);
        }
    }

    Ok(rows)
}

fn flatten_model_row(provider_id: &str, provider_name: &str, model: &ApiModel) -> ModelRow {
    ModelRow {
        id: model.id.clone(),
        name: model.name.clone(),
        provider: provider_id.to_string(),
        provider_name: provider_name.to_string(),
        display_id: format!("{provider_id}/{}", model.id),
        context: model.context_str(),
        output: model.output_str(),
        cost: model.cost_str(),
        capabilities: model.capabilities_str(),
        modalities: model.modalities_str(),
        family: model.family.clone(),
        input_cost: model.cost.as_ref().and_then(|c| c.input),
        output_cost: model.cost.as_ref().and_then(|c| c.output),
        cache_read_cost: model.cost.as_ref().and_then(|c| c.cache_read),
        cache_write_cost: model.cost.as_ref().and_then(|c| c.cache_write),
        reasoning_cost: model.cost.as_ref().and_then(|c| c.reasoning),
        input_audio_cost: model.cost.as_ref().and_then(|c| c.input_audio),
        output_audio_cost: model.cost.as_ref().and_then(|c| c.output_audio),
        tiers: model
            .cost
            .as_ref()
            .map(|c| c.tiers.clone())
            .unwrap_or_default(),
        reasoning_options: model.reasoning_options.clone(),
        reasoning: model.reasoning,
        tool_call: model.tool_call,
        attachment: model.attachment,
        structured_output: model.structured_output,
        description: model.description.clone(),
        release_date: model.release_date.clone(),
        last_updated: model.last_updated.clone(),
        knowledge_cutoff: model.knowledge.clone(),
        open_weights: model.open_weights,
        status: model.status.clone(),
    }
}

fn resolve_model(query: &str) -> Result<ResolveModel> {
    let rows = load_model_rows(None)?;
    let query_lower = query.to_lowercase();

    if let Some(row) = rows
        .iter()
        .find(|row| row.display_id.eq_ignore_ascii_case(query))
        .cloned()
    {
        return Ok(ResolveModel::Single(Box::new(row)));
    }

    let exact_id_matches = matching_model_rows(&rows, |row| row.id.eq_ignore_ascii_case(query));
    match exact_id_matches.as_slice() {
        [row] => return Ok(ResolveModel::Single(Box::new((*row).clone()))),
        [] => {}
        many => {
            return Ok(ResolveModel::Ambiguous(
                many.iter().map(|row| (*row).clone()).collect(),
            ))
        }
    }

    let exact_name_matches = matching_model_rows(&rows, |row| row.name.eq_ignore_ascii_case(query));
    match exact_name_matches.as_slice() {
        [row] => return Ok(ResolveModel::Single(Box::new((*row).clone()))),
        [] => {}
        many => {
            return Ok(ResolveModel::Ambiguous(
                many.iter().map(|row| (*row).clone()).collect(),
            ))
        }
    }

    let partial_matches = matching_model_rows(&rows, |row| {
        row.display_id.to_lowercase().contains(&query_lower)
            || row.id.to_lowercase().contains(&query_lower)
            || row.name.to_lowercase().contains(&query_lower)
            || row.provider.to_lowercase().contains(&query_lower)
            || row.provider_name.to_lowercase().contains(&query_lower)
    });

    match partial_matches.as_slice() {
        [] => bail!("Model '{}' not found", query),
        [row] => Ok(ResolveModel::Single(Box::new((*row).clone()))),
        many => Ok(ResolveModel::Ambiguous(
            many.iter().map(|row| (*row).clone()).collect(),
        )),
    }
}

fn matching_model_rows<F>(rows: &[ModelRow], predicate: F) -> Vec<&ModelRow>
where
    F: Fn(&ModelRow) -> bool,
{
    let mut matches: Vec<_> = rows.iter().filter(|row| predicate(row)).collect();
    matches.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.provider.cmp(&b.provider))
            .then_with(|| a.id.cmp(&b.id))
    });
    matches
}

fn pick_model(
    entries: Vec<ModelRow>,
    sort: ModelSort,
    descending: bool,
    title: &str,
) -> Result<Option<ModelRow>> {
    pick_model_with_query(entries, sort, descending, title, "")
}

fn pick_model_with_query(
    entries: Vec<ModelRow>,
    sort: ModelSort,
    descending: bool,
    title: &str,
    query: &str,
) -> Result<Option<ModelRow>> {
    let mut picker = ModelPicker::new(entries, sort, descending, title.to_string());
    if !query.is_empty() {
        picker.query = query.to_string();
        picker.rebuild_visible_entries(None);
    }
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
                    KeyCode::Char('c') => {
                        if let Some(row) = picker.selected() {
                            let text = row.id.clone();
                            std::thread::spawn(move || {
                                if let Ok(mut cb) = arboard::Clipboard::new() {
                                    let _ = cb.set_text(&text);
                                    std::thread::sleep(Duration::from_secs(2));
                                }
                            });
                            picker.copied_at = Some(std::time::Instant::now());
                        }
                    }
                    KeyCode::Char('s') => picker.cycle_sort(),
                    KeyCode::Char('S') => picker.toggle_descending(),
                    KeyCode::Enter => return Ok(picker.selected().cloned()),
                    KeyCode::Esc | KeyCode::Char('q') => return Ok(None),
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn filter_picker_entries(
    entries: &[ModelRow],
    query: &str,
    sort: ModelSort,
    descending: bool,
) -> Vec<ModelRow> {
    let query = query.trim().to_lowercase();
    let mut visible: Vec<_> = entries
        .iter()
        .filter(|row| {
            query.is_empty()
                || row.display_id.to_lowercase().contains(&query)
                || row.id.to_lowercase().contains(&query)
                || row.name.to_lowercase().contains(&query)
                || row.provider.to_lowercase().contains(&query)
                || row.provider_name.to_lowercase().contains(&query)
        })
        .cloned()
        .collect();

    if !matches!(sort, ModelSort::Name | ModelSort::Provider) {
        visible.retain(|row| sort.extract(row).is_some());
    }

    visible.sort_by(|a, b| {
        let order = match sort {
            ModelSort::Name => a.name.cmp(&b.name),
            ModelSort::Provider => a
                .provider_name
                .cmp(&b.provider_name)
                .then_with(|| a.name.cmp(&b.name)),
            _ => cmp_opt_f64(sort.extract(a), sort.extract(b))
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.provider.cmp(&b.provider)),
        };
        if descending {
            order.reverse()
        } else {
            order
        }
    });

    visible
}

fn print_model_table(rows: &[ModelRow], sort: ModelSort) {
    let mut table = ComfyTable::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(vec![
        "ID",
        "Name",
        "Provider",
        match sort {
            ModelSort::Name => "Context",
            _ => sort.label(),
        },
        "Cost",
        "Capabilities",
    ]);

    for row in rows {
        table.add_row(vec![
            row.display_id.clone(),
            row.name.clone(),
            row.provider_name.clone(),
            format_picker_sort_value(sort, row),
            row.cost.clone(),
            row.capabilities.clone(),
        ]);
    }

    println!("{table}");
}

pub fn print_model_detail(row: &ModelRow, json: bool) -> Result<()> {
    let detail = ModelDetail {
        id: row.id.clone(),
        name: row.name.clone(),
        provider_id: row.provider.clone(),
        provider_name: row.provider_name.clone(),
        family: row.family.clone(),
        context: row.context.clone(),
        output: row.output.clone(),
        input_cost: row.input_cost,
        output_cost: row.output_cost,
        cache_read_cost: row.cache_read_cost,
        cache_write_cost: row.cache_write_cost,
        reasoning_cost: row.reasoning_cost,
        input_audio_cost: row.input_audio_cost,
        output_audio_cost: row.output_audio_cost,
        tiers: row.tiers.clone(),
        reasoning_options: row.reasoning_options.clone(),
        reasoning: row.reasoning,
        tool_call: row.tool_call,
        attachment: row.attachment,
        structured_output: row.structured_output,
        description: row.description.clone(),
        modalities: row.modalities.clone(),
        release_date: row.release_date.clone(),
        last_updated: row.last_updated.clone(),
        knowledge_cutoff: row.knowledge_cutoff.clone(),
        open_weights: row.open_weights,
        status: row.status.clone(),
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&detail)?);
    } else {
        print_detail(&detail);
    }
    Ok(())
}

fn print_detail(d: &ModelDetail) {
    println!("{}", d.name);
    println!("{}", "=".repeat(d.name.len()));
    println!();
    println!("ID:          {}", d.id);
    println!("Provider:    {} ({})", d.provider_name, d.provider_id);
    if let Some(family) = &d.family {
        println!("Family:      {}", family);
    }
    if let Some(desc) = d.description.as_deref().filter(|s| !s.is_empty()) {
        println!("Description: {}", desc);
    }
    println!();

    println!("Limits");
    println!("------");
    println!("Context:     {} tokens", d.context);
    println!("Max Output:  {} tokens", d.output);
    println!();

    println!("Pricing (per million tokens)");
    println!("----------------------------");
    if let Some(input) = d.input_cost {
        println!("Input:       ${:.2}", input);
    }
    if let Some(output) = d.output_cost {
        println!("Output:      ${:.2}", output);
    }
    if let Some(cache_read) = d.cache_read_cost {
        println!("Cache Read:  ${:.2}", cache_read);
    }
    if let Some(cache_write) = d.cache_write_cost {
        println!("Cache Write: ${:.2}", cache_write);
    }
    if let Some(reasoning) = d.reasoning_cost {
        println!("Thinking:    ${:.2}", reasoning);
    }
    if let Some(audio_in) = d.input_audio_cost {
        println!("Audio In:    ${:.2}", audio_in);
    }
    if let Some(audio_out) = d.output_audio_cost {
        println!("Audio Out:   ${:.2}", audio_out);
    }
    for t in &d.tiers {
        let threshold = t
            .tier
            .as_ref()
            .and_then(|ts| ts.size)
            .map(|s| format!("Over {}", crate::formatting::format_tokens(s)))
            .unwrap_or_else(|| "Tier".to_string());
        let fmt = |v: Option<f64>| {
            v.map(|x| format!("${:.2}", x))
                .unwrap_or_else(|| "—".into())
        };
        println!(
            "{:<12} {} / {}",
            format!("{}:", threshold),
            fmt(t.input),
            fmt(t.output)
        );
    }
    println!();

    println!("Capabilities");
    println!("------------");
    println!("Reasoning:   {}", yes_no(d.reasoning));
    println!("Tool Use:    {}", yes_no(d.tool_call));
    println!("Attachments: {}", yes_no(d.attachment));
    println!("Structured:  {}", yes_no_opt(d.structured_output));
    for (label, value) in crate::data::reasoning_controls(&d.reasoning_options) {
        println!("{:<13}{}", format!("{label}:"), value);
    }
    println!("Modalities:  {}", d.modalities);
    println!();

    println!("Metadata");
    println!("--------");
    if let Some(date) = &d.release_date {
        println!("Released:    {}", date);
    }
    if let Some(date) = &d.last_updated {
        println!("Updated:     {}", date);
    }
    if let Some(date) = &d.knowledge_cutoff {
        println!("Knowledge:   {}", date);
    }
    println!("Open Weights: {}", yes_no(d.open_weights));
    if let Some(status) = &d.status {
        println!("Status:      {}", status);
    }
}

fn ambiguous_model_matches_message(query: &str, rows: &[ModelRow]) -> String {
    let suggestions = rows
        .iter()
        .take(5)
        .map(|row| row.display_id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!("Model query '{query}' was ambiguous; try provider/model. Matches: {suggestions}")
}

fn format_picker_sort_value(sort: ModelSort, row: &ModelRow) -> String {
    match sort {
        ModelSort::Name => row.context.clone(),
        ModelSort::Provider => row.provider_name.clone(),
        ModelSort::Context => row.context.clone(),
        ModelSort::InputPrice => format_optional_price(row.input_cost),
        ModelSort::OutputPrice => format_optional_price(row.output_cost),
        ModelSort::ReleaseDate => row
            .release_date
            .clone()
            .unwrap_or_else(|| "\u{2014}".to_string()),
    }
}

fn picker_sort_label(sort: ModelSort) -> &'static str {
    match sort {
        ModelSort::Name => "Context",
        _ => sort.label(),
    }
}

fn format_optional_price(value: Option<f64>) -> String {
    value
        .map(|v| ApiModel::cost_short(Some(v)))
        .unwrap_or_else(|| "\u{2014}".to_string())
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "Yes"
    } else {
        "No"
    }
}

/// Three-state variant for `Option<bool>` capability flags — an em-dash marks
/// "unknown" (field absent upstream), distinct from an explicit `No`.
fn yes_no_opt(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Yes",
        Some(false) => "No",
        None => "—",
    }
}

fn parse_token_count(text: &str) -> Option<f64> {
    if text == "-" || text == "\u{2014}" {
        return None;
    }
    let lower = text.to_lowercase();
    if let Some(raw) = lower.strip_suffix('m') {
        return raw.parse::<f64>().ok().map(|v| v * 1_000_000.0);
    }
    if let Some(raw) = lower.strip_suffix('k') {
        return raw.parse::<f64>().ok().map(|v| v * 1_000.0);
    }
    lower.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        id: &str,
        provider: &str,
        name: &str,
        context: &str,
        input_cost: Option<f64>,
    ) -> ModelRow {
        ModelRow {
            id: id.to_string(),
            name: name.to_string(),
            provider: provider.to_string(),
            provider_name: provider.to_string(),
            display_id: format!("{provider}/{id}"),
            context: context.to_string(),
            output: "8k".to_string(),
            cost: "-/-".to_string(),
            capabilities: "reasoning, tools".to_string(),
            modalities: "text -> text".to_string(),
            family: None,
            input_cost,
            output_cost: input_cost.map(|v| v * 2.0),
            cache_read_cost: None,
            cache_write_cost: None,
            reasoning_cost: None,
            input_audio_cost: None,
            output_audio_cost: None,
            tiers: Vec::new(),
            reasoning_options: Vec::new(),
            reasoning: true,
            tool_call: true,
            attachment: false,
            structured_output: None,
            description: None,
            release_date: Some("2025-01-01".to_string()),
            last_updated: None,
            knowledge_cutoff: None,
            open_weights: false,
            status: None,
        }
    }

    #[test]
    fn filter_picker_entries_applies_query() {
        let rows = vec![
            row("gpt-4o", "openai", "GPT-4o", "128k", Some(2.0)),
            row("claude", "anthropic", "Claude Sonnet", "200k", Some(3.0)),
        ];
        let filtered = filter_picker_entries(&rows, "claude", ModelSort::Name, false);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "claude");
    }

    #[test]
    fn filter_picker_entries_sorts_by_context_desc() {
        let rows = vec![
            row("small", "openai", "Small", "32k", Some(2.0)),
            row("large", "openai", "Large", "128k", Some(2.0)),
        ];
        let filtered = filter_picker_entries(&rows, "", ModelSort::Context, true);
        assert_eq!(filtered[0].id, "large");
    }

    #[test]
    fn ambiguous_model_matches_message_uses_display_ids() {
        let rows = vec![
            row("gpt-4o", "openai", "GPT-4o", "128k", Some(2.0)),
            row("gpt-4o", "azure", "GPT-4o", "128k", Some(2.0)),
        ];
        let message = ambiguous_model_matches_message("gpt-4o", &rows);
        assert!(message.contains("openai/gpt-4o"));
        assert!(message.contains("azure/gpt-4o"));
    }
}
