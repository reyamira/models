use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::widgets::ListState;

use crate::data::{Model, Provider};
use crate::provider_category::{provider_category, ProviderCategory};
use crate::tui::app::{App, Message};
use crate::tui::mouse::{hit, row_at};
use crate::tui::widgets::scroll_offset::ScrollOffset;

/// Page size for page up/down navigation
const PAGE_SIZE: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Providers,
    Models,
    Details,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    Default,
    #[default]
    ReleaseDate,
    Cost,
    Context,
}

impl SortOrder {
    pub fn next(self) -> Self {
        match self {
            SortOrder::Default => SortOrder::ReleaseDate,
            SortOrder::ReleaseDate => SortOrder::Cost,
            SortOrder::Cost => SortOrder::Context,
            SortOrder::Context => SortOrder::Default,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Filters {
    pub reasoning: bool,
    pub tools: bool,
    pub open_weights: bool,
    pub free: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderListItem {
    All,
    CategoryHeader(ProviderCategory),
    Provider(usize, usize), // (index into providers, match count)
}

#[derive(Debug, Clone)]
pub struct ModelEntry {
    pub id: String,
    pub model: Model,
    pub provider_id: String,
}

pub struct ModelsApp {
    pub selected_provider: usize,
    pub selected_model: usize,
    pub provider_list_state: ListState,
    pub model_list_state: ListState,
    pub focus: Focus,
    pub sort_order: SortOrder,
    pub sort_ascending: bool,
    pub filters: Filters,
    pub search_query: String,
    pub provider_category_filter: ProviderCategory,
    pub group_by_category: bool,
    pub provider_list_items: Vec<ProviderListItem>,
    filtered_models: Vec<ModelEntry>,
    pub detail_scroll: ScrollOffset,
    /// Glossary popup (`i`) explaining the capability/pricing fields.
    pub show_glossary: bool,
    pub glossary_scroll: ScrollOffset,
    /// Panel rects cached at render time for mouse hit-testing (see
    /// `crate::tui::mouse`). The stored areas are the exact rects the list /
    /// detail widgets render into — `provider_list_area`/`model_list_area` are
    /// the bare item regions (no border, no filter row), so `row_at` uses
    /// `top_skip = 0`.
    pub provider_list_area: Option<Rect>,
    pub model_list_area: Option<Rect>,
    pub provider_card_area: Option<Rect>,
    pub model_detail_area: Option<Rect>,
}

impl ModelsApp {
    pub fn new(providers: &[(String, Provider)]) -> Self {
        let mut provider_list_state = ListState::default();
        provider_list_state.select(Some(0));
        let mut model_list_state = ListState::default();
        model_list_state.select(Some(1)); // +1 for header row

        let mut app = Self {
            selected_provider: 0, // Start with "All"
            selected_model: 0,
            provider_list_state,
            model_list_state,
            focus: Focus::Providers,
            sort_order: SortOrder::ReleaseDate,
            sort_ascending: false,
            filters: Filters::default(),
            search_query: String::new(),
            provider_category_filter: ProviderCategory::All,
            group_by_category: false,
            provider_list_items: Vec::new(),
            filtered_models: Vec::new(),
            detail_scroll: ScrollOffset::default(),
            show_glossary: false,
            glossary_scroll: ScrollOffset::default(),
            provider_list_area: None,
            model_list_area: None,
            provider_card_area: None,
            model_detail_area: None,
        };

        app.update_provider_list(providers);
        app.update_filtered_models(providers);
        app
    }

    pub fn is_all_selected(&self) -> bool {
        matches!(
            self.provider_list_items.get(self.selected_provider),
            Some(ProviderListItem::All)
        )
    }

    pub fn provider_list_len(&self) -> usize {
        self.provider_list_items.len()
    }

    pub fn selected_provider_data<'a>(
        &self,
        providers: &'a [(String, Provider)],
    ) -> Option<&'a (String, Provider)> {
        match self.provider_list_items.get(self.selected_provider) {
            Some(ProviderListItem::Provider(idx, _)) => providers.get(*idx),
            _ => None,
        }
    }

    fn has_active_filters(&self) -> bool {
        !self.search_query.is_empty()
            || self.filters.reasoning
            || self.filters.tools
            || self.filters.open_weights
            || self.filters.free
    }

    fn provider_match_count(&self, provider_id: &str, provider: &Provider) -> usize {
        let query_lower = self.search_query.to_lowercase();
        provider
            .models
            .iter()
            .filter(|(model_id, model)| {
                let search_matches = query_lower.is_empty()
                    || model_id.to_lowercase().contains(&query_lower)
                    || model.name.to_lowercase().contains(&query_lower)
                    || provider_id.to_lowercase().contains(&query_lower);
                search_matches && self.passes_filters(model)
            })
            .count()
    }

    pub fn update_provider_list(&mut self, providers: &[(String, Provider)]) {
        self.provider_list_items.clear();
        self.provider_list_items.push(ProviderListItem::All);

        let filtering = self.has_active_filters();

        if self.group_by_category {
            let categories = [
                ProviderCategory::Origin,
                ProviderCategory::Cloud,
                ProviderCategory::Inference,
                ProviderCategory::Gateway,
                ProviderCategory::Tool,
            ];

            for cat in &categories {
                if self.provider_category_filter != ProviderCategory::All
                    && self.provider_category_filter != *cat
                {
                    continue;
                }

                let mut items: Vec<(usize, usize)> = providers
                    .iter()
                    .enumerate()
                    .filter(|(_, (id, _))| provider_category(id) == *cat)
                    .filter_map(|(idx, (id, provider))| {
                        let count = if filtering {
                            let c = self.provider_match_count(id, provider);
                            if c == 0 {
                                return None;
                            }
                            c
                        } else {
                            provider.models.len()
                        };
                        Some((idx, count))
                    })
                    .collect();

                if items.is_empty() {
                    continue;
                }

                items.sort_by(|a, b| providers[a.0].0.cmp(&providers[b.0].0));

                self.provider_list_items
                    .push(ProviderListItem::CategoryHeader(*cat));
                for (idx, count) in items {
                    self.provider_list_items
                        .push(ProviderListItem::Provider(idx, count));
                }
            }
        } else {
            for (idx, (id, provider)) in providers.iter().enumerate() {
                if self.provider_category_filter != ProviderCategory::All
                    && provider_category(id) != self.provider_category_filter
                {
                    continue;
                }
                let count = if filtering {
                    let c = self.provider_match_count(id, provider);
                    if c == 0 {
                        continue;
                    }
                    c
                } else {
                    provider.models.len()
                };
                self.provider_list_items
                    .push(ProviderListItem::Provider(idx, count));
            }
        }
    }

    pub fn find_selectable_index(&self, from: usize, forward: bool) -> usize {
        let len = self.provider_list_items.len();
        if len == 0 {
            return 0;
        }

        let mut idx = from;
        loop {
            if !matches!(
                self.provider_list_items.get(idx),
                Some(ProviderListItem::CategoryHeader(_))
            ) {
                return idx;
            }
            if forward {
                if idx >= len - 1 {
                    return self.find_selectable_index(from.saturating_sub(1), false);
                }
                idx += 1;
            } else {
                if idx == 0 {
                    return 0;
                }
                idx -= 1;
            }
        }
    }

    fn passes_filters(&self, model: &Model) -> bool {
        if self.filters.reasoning && !model.reasoning {
            return false;
        }
        if self.filters.tools && !model.tool_call {
            return false;
        }
        if self.filters.open_weights && !model.open_weights {
            return false;
        }
        if self.filters.free && !model.is_free() {
            return false;
        }
        true
    }

    pub fn update_filtered_models(&mut self, providers: &[(String, Provider)]) {
        let query_lower = self.search_query.to_lowercase();
        let cat_filter = self.provider_category_filter;

        self.filtered_models = if self.is_all_selected() {
            let mut entries: Vec<ModelEntry> = providers
                .iter()
                .filter(|(id, _)| {
                    cat_filter == ProviderCategory::All || provider_category(id) == cat_filter
                })
                .flat_map(|(provider_id, provider)| {
                    provider.models.iter().filter_map(|(model_id, model)| {
                        let search_matches = query_lower.is_empty()
                            || model_id.to_lowercase().contains(&query_lower)
                            || model.name.to_lowercase().contains(&query_lower)
                            || provider_id.to_lowercase().contains(&query_lower);

                        if search_matches && self.passes_filters(model) {
                            Some(ModelEntry {
                                id: model_id.clone(),
                                model: model.clone(),
                                provider_id: provider_id.clone(),
                            })
                        } else {
                            None
                        }
                    })
                })
                .collect();

            self.sort_entries(&mut entries);
            entries
        } else {
            let provider_data = self.selected_provider_data(providers).cloned();
            if let Some((provider_id, provider)) = provider_data {
                let mut entries: Vec<ModelEntry> = provider
                    .models
                    .iter()
                    .filter_map(|(model_id, model)| {
                        let search_matches = query_lower.is_empty()
                            || model_id.to_lowercase().contains(&query_lower)
                            || model.name.to_lowercase().contains(&query_lower);

                        if search_matches && self.passes_filters(model) {
                            Some(ModelEntry {
                                id: model_id.clone(),
                                model: model.clone(),
                                provider_id: provider_id.clone(),
                            })
                        } else {
                            None
                        }
                    })
                    .collect();

                self.sort_entries(&mut entries);
                entries
            } else {
                Vec::new()
            }
        };
    }

    fn sort_entries(&self, entries: &mut [ModelEntry]) {
        match self.sort_order {
            SortOrder::Default => {
                entries.sort_by(|a, b| a.provider_id.cmp(&b.provider_id).then(a.id.cmp(&b.id)));
            }
            SortOrder::ReleaseDate => {
                entries.sort_by(
                    |a, b| match (&b.model.release_date, &a.model.release_date) {
                        (Some(b_date), Some(a_date)) => {
                            if self.sort_ascending {
                                a_date.cmp(b_date)
                            } else {
                                b_date.cmp(a_date)
                            }
                        }
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => a.id.cmp(&b.id),
                    },
                );
            }
            SortOrder::Cost => {
                entries.sort_by(|a, b| {
                    let a_cost = a.model.cost.as_ref().and_then(|c| c.input);
                    let b_cost = b.model.cost.as_ref().and_then(|c| c.input);
                    match (a_cost, b_cost) {
                        (Some(a_val), Some(b_val)) => {
                            let cmp = a_val
                                .partial_cmp(&b_val)
                                .unwrap_or(std::cmp::Ordering::Equal);
                            if self.sort_ascending {
                                cmp.reverse()
                            } else {
                                cmp
                            }
                        }
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => a.id.cmp(&b.id),
                    }
                });
            }
            SortOrder::Context => {
                entries.sort_by(|a, b| {
                    let a_ctx = a.model.limit.as_ref().and_then(|l| l.context);
                    let b_ctx = b.model.limit.as_ref().and_then(|l| l.context);
                    match (b_ctx, a_ctx) {
                        (Some(b_val), Some(a_val)) => {
                            if self.sort_ascending {
                                a_val.cmp(&b_val)
                            } else {
                                b_val.cmp(&a_val)
                            }
                        }
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => a.id.cmp(&b.id),
                    }
                });
            }
        }
    }

    pub fn select_provider_at_index(&mut self, index: usize, providers: &[(String, Provider)]) {
        self.selected_provider = index;
        self.selected_model = 0;
        self.provider_list_state
            .select(Some(self.selected_provider));
        self.update_filtered_models(providers);
        self.model_list_state.select(Some(self.selected_model + 1));
        // +1 for header
        self.reset_detail_scroll();
    }

    pub fn current_model(&self) -> Option<&ModelEntry> {
        self.filtered_models.get(self.selected_model)
    }

    pub fn filtered_models(&self) -> &[ModelEntry] {
        &self.filtered_models
    }

    pub fn filtered_model_count(&self) -> usize {
        self.provider_list_items
            .iter()
            .filter_map(|item| match item {
                ProviderListItem::Provider(_, count) => Some(count),
                _ => None,
            })
            .sum()
    }

    pub fn get_copy_full(&self) -> Option<String> {
        self.current_model()
            .map(|entry| format!("{}/{}", entry.provider_id, entry.id))
    }

    pub fn get_copy_model_id(&self) -> Option<String> {
        self.current_model().map(|entry| entry.id.clone())
    }

    pub fn get_provider_doc(&self, providers: &[(String, Provider)]) -> Option<String> {
        self.current_model().and_then(|entry| {
            providers
                .iter()
                .find(|(id, _)| id == &entry.provider_id)
                .and_then(|(_, provider)| provider.doc.clone())
        })
    }

    pub fn get_provider_api(&self, providers: &[(String, Provider)]) -> Option<String> {
        self.current_model().and_then(|entry| {
            providers
                .iter()
                .find(|(id, _)| id == &entry.provider_id)
                .and_then(|(_, provider)| provider.api.clone())
        })
    }

    // --- Navigation handlers called from App::update ---

    pub fn next_provider(&mut self, providers: &[(String, Provider)]) {
        if self.selected_provider < self.provider_list_len().saturating_sub(1) {
            let next = self.find_selectable_index(self.selected_provider + 1, true);
            if next != self.selected_provider {
                self.select_provider_at_index(next, providers);
            }
        }
    }

    pub fn prev_provider(&mut self, providers: &[(String, Provider)]) {
        if self.selected_provider > 0 {
            let prev = self.find_selectable_index(self.selected_provider - 1, false);
            if prev != self.selected_provider {
                self.select_provider_at_index(prev, providers);
            }
        }
    }

    pub fn select_first_provider(&mut self, providers: &[(String, Provider)]) {
        let first = self.find_selectable_index(0, true);
        if first != self.selected_provider {
            self.select_provider_at_index(first, providers);
        }
    }

    pub fn select_last_provider(&mut self, providers: &[(String, Provider)]) {
        let last_raw = self.provider_list_len().saturating_sub(1);
        let last = self.find_selectable_index(last_raw, false);
        if last != self.selected_provider {
            self.select_provider_at_index(last, providers);
        }
    }

    pub fn page_down_provider(&mut self, providers: &[(String, Provider)]) {
        let last_index = self.provider_list_len().saturating_sub(1);
        let raw = (self.selected_provider + PAGE_SIZE).min(last_index);
        let next = self.find_selectable_index(raw, true);
        if next != self.selected_provider {
            self.select_provider_at_index(next, providers);
        }
    }

    pub fn page_up_provider(&mut self, providers: &[(String, Provider)]) {
        let raw = self.selected_provider.saturating_sub(PAGE_SIZE);
        let next = self.find_selectable_index(raw, false);
        if next != self.selected_provider {
            self.select_provider_at_index(next, providers);
        }
    }

    pub fn next_model(&mut self) {
        if self.selected_model < self.filtered_models.len().saturating_sub(1) {
            self.selected_model += 1;
            self.model_list_state.select(Some(self.selected_model + 1));
            // +1 for header
            self.reset_detail_scroll();
        }
    }

    pub fn prev_model(&mut self) {
        if self.selected_model > 0 {
            self.selected_model -= 1;
            self.model_list_state.select(Some(self.selected_model + 1));
            // +1 for header
            self.reset_detail_scroll();
        }
    }

    /// Select a model by its index into `filtered_models` (used by mouse clicks).
    pub fn select_model_at_index(&mut self, index: usize) {
        if index < self.filtered_models.len() && index != self.selected_model {
            self.selected_model = index;
            self.model_list_state.select(Some(self.selected_model + 1));
            // +1 for header
            self.reset_detail_scroll();
        }
    }

    pub fn select_first_model(&mut self) {
        if self.selected_model > 0 {
            self.selected_model = 0;
            self.model_list_state.select(Some(self.selected_model + 1));
            self.reset_detail_scroll();
        }
    }

    pub fn select_last_model(&mut self) {
        if self.selected_model < self.filtered_models.len().saturating_sub(1) {
            self.selected_model = self.filtered_models.len().saturating_sub(1);
            self.model_list_state.select(Some(self.selected_model + 1));
            self.reset_detail_scroll();
        }
    }

    pub fn page_down_model(&mut self) {
        let last_index = self.filtered_models.len().saturating_sub(1);
        let next = (self.selected_model + PAGE_SIZE).min(last_index);
        if next != self.selected_model {
            self.selected_model = next;
            self.model_list_state.select(Some(self.selected_model + 1));
            self.reset_detail_scroll();
        }
    }

    pub fn page_up_model(&mut self) {
        let next = self.selected_model.saturating_sub(PAGE_SIZE);
        if next != self.selected_model {
            self.selected_model = next;
            self.model_list_state.select(Some(self.selected_model + 1));
            self.reset_detail_scroll();
        }
    }

    pub fn focus_right(&mut self) {
        self.focus = match self.focus {
            Focus::Providers => Focus::Models,
            Focus::Models => Focus::Details,
            Focus::Details => Focus::Providers,
        };
    }

    pub fn focus_left(&mut self) {
        self.focus = match self.focus {
            Focus::Providers => Focus::Details,
            Focus::Models => Focus::Providers,
            Focus::Details => Focus::Models,
        };
    }

    pub fn reset_detail_scroll(&self) {
        self.detail_scroll.jump_top();
    }

    pub fn toggle_glossary(&mut self) {
        self.show_glossary = !self.show_glossary;
        if self.show_glossary {
            self.glossary_scroll.jump_top();
        }
    }

    pub fn scroll_glossary_down(&self) {
        self.glossary_scroll.increment(1);
    }

    pub fn scroll_glossary_up(&self) {
        self.glossary_scroll.decrement(1);
    }

    pub fn cycle_sort(&mut self, providers: &[(String, Provider)]) {
        self.sort_order = self.sort_order.next();
        self.sort_ascending = false;
        self.selected_model = 0;
        self.update_filtered_models(providers);
        self.model_list_state.select(Some(self.selected_model + 1));
        self.reset_detail_scroll();
    }

    pub fn toggle_sort_dir(&mut self, providers: &[(String, Provider)]) {
        if self.sort_order != SortOrder::Default {
            self.sort_ascending = !self.sort_ascending;
            self.selected_model = 0;
            self.update_filtered_models(providers);
            self.model_list_state.select(Some(self.selected_model + 1));
            self.reset_detail_scroll();
        }
    }

    pub fn toggle_reasoning(&mut self, providers: &[(String, Provider)]) {
        self.filters.reasoning = !self.filters.reasoning;
        self.rebuild_after_filter_change(providers);
    }

    pub fn toggle_tools(&mut self, providers: &[(String, Provider)]) {
        self.filters.tools = !self.filters.tools;
        self.rebuild_after_filter_change(providers);
    }

    pub fn toggle_open_weights(&mut self, providers: &[(String, Provider)]) {
        self.filters.open_weights = !self.filters.open_weights;
        self.rebuild_after_filter_change(providers);
    }

    pub fn toggle_free(&mut self, providers: &[(String, Provider)]) {
        self.filters.free = !self.filters.free;
        self.rebuild_after_filter_change(providers);
    }

    pub fn cycle_provider_category(&mut self, providers: &[(String, Provider)]) {
        self.provider_category_filter = self.provider_category_filter.next();
        self.update_provider_list(providers);
        self.selected_provider = self.find_selectable_index(0, true);
        self.provider_list_state
            .select(Some(self.selected_provider));
        self.selected_model = 0;
        self.update_filtered_models(providers);
        self.model_list_state.select(Some(self.selected_model + 1));
        self.reset_detail_scroll();
    }

    pub fn toggle_grouping(&mut self, providers: &[(String, Provider)]) {
        self.group_by_category = !self.group_by_category;
        self.update_provider_list(providers);
        self.selected_provider = self.find_selectable_index(0, true);
        self.provider_list_state
            .select(Some(self.selected_provider));
        self.selected_model = 0;
        self.update_filtered_models(providers);
        self.model_list_state.select(Some(self.selected_model + 1));
        self.reset_detail_scroll();
    }

    pub fn search_input(&mut self, c: char, providers: &[(String, Provider)]) {
        self.search_query.push(c);
        self.rebuild_after_filter_change(providers);
    }

    pub fn search_backspace(&mut self, providers: &[(String, Provider)]) {
        self.search_query.pop();
        self.rebuild_after_filter_change(providers);
    }

    pub fn clear_search(&mut self, providers: &[(String, Provider)]) {
        self.search_query.clear();
        self.rebuild_after_filter_change(providers);
    }

    /// Rebuild provider list and model list after any search/filter change.
    /// Preserves the selected provider if it's still visible, otherwise falls back to "All".
    fn rebuild_after_filter_change(&mut self, providers: &[(String, Provider)]) {
        // Remember which provider was selected (by index into providers slice)
        let prev_provider_idx = match self.provider_list_items.get(self.selected_provider) {
            Some(ProviderListItem::Provider(idx, _)) => Some(*idx),
            _ => None, // All or CategoryHeader
        };

        self.update_provider_list(providers);

        // Try to find the previously selected provider in the new list
        let new_pos = prev_provider_idx.and_then(|prev_idx| {
            self.provider_list_items.iter().position(
                |item| matches!(item, ProviderListItem::Provider(idx, _) if *idx == prev_idx),
            )
        });

        self.selected_provider = new_pos.unwrap_or(0);
        self.provider_list_state
            .select(Some(self.selected_provider));
        self.selected_model = 0;
        self.update_filtered_models(providers);
        self.model_list_state.select(Some(self.selected_model + 1));
        self.reset_detail_scroll();
    }
}

/// Handle a mouse event while the Models tab is active.
///
/// All state changes (focus, selection, scroll) are applied directly to `app`,
/// so the function returns `None`; the main loop redraws after every event. The
/// `Option<Message>` return keeps the per-tab handler signature uniform with the
/// other tabs' dispatchers.
pub fn handle_models_mouse(app: &mut App, ev: MouseEvent) -> Option<Message> {
    match ev.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if hit(app.models_app.provider_list_area, &ev) {
                app.models_app.focus = Focus::Providers;
                if let Some(area) = app.models_app.provider_list_area {
                    if let Some(idx) = row_at(
                        area,
                        app.models_app.provider_list_state.offset(),
                        0,
                        app.models_app.provider_list_items.len(),
                        ev.row,
                    ) {
                        // Skip non-selectable category headers.
                        if !matches!(
                            app.models_app.provider_list_items.get(idx),
                            Some(ProviderListItem::CategoryHeader(_))
                        ) {
                            app.models_app.select_provider_at_index(idx, &app.providers);
                        }
                    }
                }
            } else if hit(app.models_app.model_list_area, &ev) {
                app.models_app.focus = Focus::Models;
                if let Some(area) = app.models_app.model_list_area {
                    // Item 0 is the column header; models occupy items 1..=N.
                    if let Some(idx) = row_at(
                        area,
                        app.models_app.model_list_state.offset(),
                        0,
                        app.models_app.filtered_models.len() + 1,
                        ev.row,
                    ) {
                        if let Some(model_idx) = idx.checked_sub(1) {
                            app.models_app.select_model_at_index(model_idx);
                        }
                    }
                }
            } else if hit(app.models_app.model_detail_area, &ev)
                || hit(app.models_app.provider_card_area, &ev)
            {
                app.models_app.focus = Focus::Details;
            }
        }
        // Wheel: focus the panel under the cursor, then scroll it (reusing the
        // same per-panel nav the arrow keys drive).
        MouseEventKind::ScrollDown => {
            if hit(app.models_app.provider_list_area, &ev) {
                app.models_app.focus = Focus::Providers;
                app.models_app.next_provider(&app.providers);
            } else if hit(app.models_app.model_list_area, &ev) {
                app.models_app.focus = Focus::Models;
                app.models_app.next_model();
            } else if hit(app.models_app.model_detail_area, &ev)
                || hit(app.models_app.provider_card_area, &ev)
            {
                app.models_app.focus = Focus::Details;
                app.models_app.detail_scroll.increment(1);
            }
        }
        MouseEventKind::ScrollUp => {
            if hit(app.models_app.provider_list_area, &ev) {
                app.models_app.focus = Focus::Providers;
                app.models_app.prev_provider(&app.providers);
            } else if hit(app.models_app.model_list_area, &ev) {
                app.models_app.focus = Focus::Models;
                app.models_app.prev_model();
            } else if hit(app.models_app.model_detail_area, &ev)
                || hit(app.models_app.provider_card_area, &ev)
            {
                app.models_app.focus = Focus::Details;
                app.models_app.detail_scroll.decrement(1);
            }
        }
        _ => {}
    }
    None
}
