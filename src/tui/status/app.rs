use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::widgets::ListState;

use crate::config::Config;
use crate::status::{
    status_seed_for_provider, ProviderHealth, ProviderStatus, ScheduledMaintenance,
    StatusLoadState, StatusProvenance, StatusProviderSeed, STATUS_REGISTRY,
};
use crate::tui::app::{App, Message};
use crate::tui::mouse::{hit, row_at};
use crate::tui::widgets::ScrollOffset;

const PAGE_SIZE: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusFocus {
    #[default]
    List,
    Details,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverallPanelFocus {
    #[default]
    Incidents,
    Degradation,
    Maintenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DetailPanelFocus {
    Services,
    #[default]
    Incidents,
    Maintenance,
}

pub struct StatusApp {
    pub entries: Vec<ProviderStatus>,
    pub filtered_entries: Vec<usize>,
    pub selected: usize,
    pub list_state: ListState,
    pub focus: StatusFocus,
    pub overall_panel_focus: OverallPanelFocus,
    pub detail_panel_focus: DetailPanelFocus,
    pub search_query: String,
    pub detail_scroll: ScrollOffset,
    pub overall_incidents_scroll: ScrollOffset,
    pub overall_degradation_scroll: ScrollOffset,
    pub overall_maintenance_scroll: ScrollOffset,
    pub services_scroll: ScrollOffset,
    pub maintenance_scroll: ScrollOffset,
    pub loading: bool,
    pub last_refreshed: Option<Instant>,
    pub last_error: Option<String>,
    /// Slugs of providers to track (fetch status for)
    pub tracked: HashSet<String>,
    // Picker modal state
    pub show_picker: bool,
    pub picker_selected: usize,
    pub picker_changes: HashMap<String, bool>,
    /// Panel rects cached at render time for mouse hit-testing (see
    /// `crate::tui::mouse`). `provider_list_area` is the bare list-item region
    /// (block inner, no border), so `row_at` uses `top_skip = 0`; item index
    /// maps straight to the display index (0 = Overall). The overall/detail
    /// sub-panel rects are the *outer* SoftCard panel rects (used with `hit()`
    /// only — those panels have no per-row selection). They are reset to `None`
    /// each render so a sub-panel that vanishes (e.g. maintenance) can't keep a
    /// stale rect, and only one of the two sets is live at a time (overall vs
    /// provider-detail view).
    pub provider_list_area: Option<Rect>,
    pub overall_incidents_area: Option<Rect>,
    pub overall_degradation_area: Option<Rect>,
    pub overall_maintenance_area: Option<Rect>,
    pub detail_services_area: Option<Rect>,
    pub detail_incidents_area: Option<Rect>,
    pub detail_maintenance_area: Option<Rect>,
    /// Inner list rect of the provider-tracker modal (borders excluded), cached
    /// for click hit-testing. `Cell` so the `&App` render path can write it.
    pub picker_area: std::cell::Cell<Option<Rect>>,
}

/// Sub-panel rects produced by `draw_overall_dashboard`, assigned back onto
/// `StatusApp` by the caller (which holds the `&mut`).
#[derive(Default)]
pub struct OverallPanelRects {
    pub incidents: Option<Rect>,
    pub degradation: Option<Rect>,
    pub maintenance: Option<Rect>,
}

/// Sub-panel rects produced by `draw_provider_status_detail`.
#[derive(Default)]
pub struct DetailPanelRects {
    pub services: Option<Rect>,
    pub incidents: Option<Rect>,
    pub maintenance: Option<Rect>,
}

impl StatusApp {
    pub fn new(config: &Config) -> Self {
        let tracked = config.status.tracked.clone();

        let mut by_slug: BTreeMap<String, StatusProviderSeed> = BTreeMap::new();

        for entry in STATUS_REGISTRY {
            by_slug.insert(
                entry.slug.to_string(),
                StatusProviderSeed {
                    slug: entry.slug.to_string(),
                    display_name: entry.display_name.to_string(),
                    source_slug: entry.source_slug.to_string(),
                    strategy: entry.strategy,
                    support_tier: entry.support_tier,
                },
            );
        }

        let entries: Vec<_> = by_slug.values().map(ProviderStatus::placeholder).collect();

        let mut list_state = ListState::default();
        list_state.select(Some(0));

        let mut app = Self {
            entries,
            filtered_entries: Vec::new(),
            selected: 0,
            list_state,
            focus: StatusFocus::default(),
            overall_panel_focus: OverallPanelFocus::default(),
            detail_panel_focus: DetailPanelFocus::default(),
            search_query: String::new(),
            detail_scroll: ScrollOffset::default(),
            overall_incidents_scroll: ScrollOffset::default(),
            overall_degradation_scroll: ScrollOffset::default(),
            overall_maintenance_scroll: ScrollOffset::default(),
            services_scroll: ScrollOffset::default(),
            maintenance_scroll: ScrollOffset::default(),
            loading: true,
            last_refreshed: None,
            last_error: None,
            tracked,
            show_picker: false,
            picker_selected: 0,
            picker_changes: HashMap::new(),
            provider_list_area: None,
            overall_incidents_area: None,
            overall_degradation_area: None,
            overall_maintenance_area: None,
            detail_services_area: None,
            detail_incidents_area: None,
            detail_maintenance_area: None,
            picker_area: std::cell::Cell::new(None),
        };
        app.update_filtered();
        app
    }

    pub fn fetch_seeds(&self) -> Vec<StatusProviderSeed> {
        self.entries
            .iter()
            .filter(|entry| self.tracked.contains(&entry.slug))
            .map(|entry| status_seed_for_provider(&entry.slug))
            .collect()
    }

    pub fn apply_fetch(&mut self, fetched: Vec<ProviderStatus>) {
        if fetched.is_empty() {
            return;
        }
        // Merge by slug: update fetched entries, reset untracked to placeholder
        let fetched_map: HashMap<String, ProviderStatus> =
            fetched.into_iter().map(|e| (e.slug.clone(), e)).collect();

        for entry in &mut self.entries {
            if let Some(fetched_entry) = fetched_map.get(&entry.slug) {
                *entry = fetched_entry.clone();
            } else if !self.tracked.contains(&entry.slug) {
                *entry = ProviderStatus::placeholder(&status_seed_for_provider(&entry.slug));
            }
        }

        // Preserve selected provider across re-sort
        let selected_slug = self.current_entry().map(|e| e.slug.clone());

        self.entries.sort_by(|a, b| {
            a.health
                .sort_rank()
                .cmp(&b.health.sort_rank())
                .then_with(|| a.support_tier.sort_rank().cmp(&b.support_tier.sort_rank()))
                .then_with(|| a.provenance.sort_rank().cmp(&b.provenance.sort_rank()))
                .then_with(|| a.display_name.cmp(&b.display_name))
        });
        self.loading = false;
        self.last_refreshed = Some(Instant::now());
        self.last_error = None;
        self.normalize_overall_panel_focus();
        self.update_filtered();

        // Restore selection to the same provider after re-sort
        if let Some(slug) = selected_slug {
            if let Some(pos) = self
                .filtered_entries
                .iter()
                .position(|&idx| self.entries[idx].slug == slug)
            {
                // +1 because selected=0 is "Overall"
                self.selected = pos + 1;
                self.list_state.select(Some(self.selected));
            }
        }
    }

    // ── Picker modal methods ───────────────────────────────────

    pub fn open_picker(&mut self) {
        self.show_picker = true;
        self.picker_selected = 0;
        self.picker_changes.clear();
        // Initialize with current tracked states
        for entry in STATUS_REGISTRY {
            let is_tracked = self.tracked.contains(entry.slug);
            self.picker_changes
                .insert(entry.slug.to_string(), is_tracked);
        }
    }

    pub fn close_picker(&mut self) {
        self.show_picker = false;
        self.picker_changes.clear();
    }

    pub fn picker_toggle_current(&mut self) {
        let slugs: Vec<&str> = STATUS_REGISTRY.iter().map(|e| e.slug).collect();
        if let Some(&slug) = slugs.get(self.picker_selected) {
            let current = self
                .picker_changes
                .get(slug)
                .copied()
                .unwrap_or_else(|| self.tracked.contains(slug));
            self.picker_changes.insert(slug.to_string(), !current);
        }
    }

    pub fn picker_next(&mut self) {
        let max = STATUS_REGISTRY.len().saturating_sub(1);
        if self.picker_selected < max {
            self.picker_selected += 1;
        }
    }

    pub fn picker_prev(&mut self) {
        if self.picker_selected > 0 {
            self.picker_selected -= 1;
        }
    }

    /// Save picker changes, update tracked set, save config. Returns newly-tracked slugs.
    pub fn picker_save(&mut self, config: &mut Config) -> Result<Vec<String>, String> {
        let mut newly_tracked = Vec::new();

        for (slug, &tracked) in &self.picker_changes {
            let was_tracked = config.is_status_tracked(slug);
            config.set_status_tracked(slug, tracked);
            if tracked && !was_tracked {
                newly_tracked.push(slug.clone());
            }
            if tracked {
                self.tracked.insert(slug.clone());
            } else {
                self.tracked.remove(slug.as_str());
            }
        }

        if let Err(e) = config.save() {
            self.close_picker();
            return Err(format!("Failed to save config: {}", e));
        }

        // Reset untracked entries to placeholder health
        for entry in &mut self.entries {
            if !self.tracked.contains(&entry.slug) {
                entry.health = ProviderHealth::Unknown;
                entry.load_state = StatusLoadState::Placeholder;
            }
        }

        self.close_picker();
        self.update_filtered();
        Ok(newly_tracked)
    }

    /// `selected` is a display index: 0 = Overall, 1+ = provider at `filtered_entries[selected - 1]`.
    pub fn update_filtered(&mut self) {
        let query = self.search_query.to_lowercase();
        self.filtered_entries = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                // Only show tracked providers
                if !self.tracked.contains(&entry.slug) {
                    return false;
                }
                query.is_empty()
                    || entry.display_name.to_lowercase().contains(&query)
                    || entry.slug.to_lowercase().contains(&query)
                    || entry
                        .source_label
                        .as_ref()
                        .is_some_and(|name| name.to_lowercase().contains(&query))
                    || entry
                        .provider_summary_text()
                        .is_some_and(|summary| summary.to_lowercase().contains(&query))
                    || entry
                        .status_note_text()
                        .is_some_and(|note| note.to_lowercase().contains(&query))
            })
            .map(|(idx, _)| idx)
            .collect();

        self.normalize_overall_panel_focus();

        // If current provider selection is out of range, reset to Overall
        if self.selected > self.filtered_entries.len() {
            self.selected = 0;
        }
        self.list_state.select(Some(self.selected));
    }

    pub fn is_overall_selected(&self) -> bool {
        self.selected == 0
    }

    /// Returns the selected provider, or `None` when Overall (index 0) is selected.
    pub fn current_entry(&self) -> Option<&ProviderStatus> {
        if self.selected == 0 {
            return None;
        }
        self.filtered_entries
            .get(self.selected - 1)
            .and_then(|&idx| self.entries.get(idx))
    }

    fn reset_detail_scrolls(&mut self) {
        self.detail_scroll.jump_top();
        self.services_scroll.jump_top();
        self.maintenance_scroll.jump_top();
        self.normalize_detail_panel_focus();
    }

    pub fn next(&mut self) {
        if self.filtered_entries.is_empty() {
            return;
        }
        self.selected = (self.selected + 1).min(self.filtered_entries.len());
        self.list_state.select(Some(self.selected));
        self.reset_detail_scrolls();
    }

    pub fn prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.list_state.select(Some(self.selected));
        self.reset_detail_scrolls();
    }

    /// Select a provider by its display index (0 = Overall, 1.. = providers).
    /// Used by mouse clicks. Out-of-range indices are ignored.
    pub fn select_at_index(&mut self, index: usize) {
        if index <= self.filtered_entries.len() && index != self.selected {
            self.selected = index;
            self.list_state.select(Some(self.selected));
            self.reset_detail_scrolls();
        }
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
        self.list_state.select(Some(0));
        self.reset_detail_scrolls();
    }

    pub fn select_last(&mut self) {
        self.selected = self.filtered_entries.len(); // last provider (0 = Overall)
        self.list_state.select(Some(self.selected));
        self.reset_detail_scrolls();
    }

    pub fn page_down(&mut self) {
        self.selected = (self.selected + PAGE_SIZE).min(self.filtered_entries.len());
        self.list_state.select(Some(self.selected));
        self.reset_detail_scrolls();
    }

    pub fn page_up(&mut self) {
        self.selected = self.selected.saturating_sub(PAGE_SIZE);
        self.list_state.select(Some(self.selected));
        self.reset_detail_scrolls();
    }

    pub fn health_counts(&self) -> (usize, usize, usize, usize) {
        let mut op = 0;
        let mut deg = 0;
        let mut out = 0;
        let mut other = 0;
        for entry in self
            .entries
            .iter()
            .filter(|e| self.tracked.contains(&e.slug))
        {
            match entry.health {
                ProviderHealth::Operational => op += 1,
                ProviderHealth::Degraded => deg += 1,
                ProviderHealth::Outage => out += 1,
                _ => other += 1,
            }
        }
        (op, deg, out, other)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn provenance_counts(&self) -> (usize, usize, usize) {
        let mut official = 0;
        let mut fallback = 0;
        let mut unavailable = 0;
        for entry in &self.entries {
            match entry.provenance {
                StatusProvenance::Official => official += 1,
                StatusProvenance::Fallback => fallback += 1,
                StatusProvenance::Unavailable => unavailable += 1,
            }
        }
        (official, fallback, unavailable)
    }

    /// All scheduled maintenances across all providers, as (display_name, maintenance) pairs.
    pub fn all_maintenances(&self) -> Vec<(&str, &ScheduledMaintenance)> {
        self.entries
            .iter()
            .filter(|entry| self.tracked.contains(&entry.slug))
            .flat_map(|entry| {
                entry
                    .scheduled_maintenances
                    .iter()
                    .map(move |m| (entry.display_name.as_str(), m))
            })
            .collect()
    }

    pub fn switch_focus(&mut self) {
        self.focus = match self.focus {
            StatusFocus::List => StatusFocus::Details,
            StatusFocus::Details => StatusFocus::List,
        };
    }

    fn visible_overall_panels(&self) -> [OverallPanelFocus; 3] {
        [
            OverallPanelFocus::Incidents,
            OverallPanelFocus::Degradation,
            OverallPanelFocus::Maintenance,
        ]
    }

    pub fn maintenance_panel_visible(&self) -> bool {
        !self.all_maintenances().is_empty()
    }

    pub fn normalize_overall_panel_focus(&mut self) {
        if self.overall_panel_focus == OverallPanelFocus::Maintenance
            && !self.maintenance_panel_visible()
        {
            self.overall_panel_focus = OverallPanelFocus::Incidents;
        }
    }

    pub fn select_prev_overall_panel(&mut self) {
        let panels = self.visible_overall_panels();
        let visible_count = if self.maintenance_panel_visible() {
            3
        } else {
            2
        };
        let current = panels[..visible_count]
            .iter()
            .position(|panel| *panel == self.overall_panel_focus)
            .unwrap_or(0);
        let prev = if current == 0 {
            visible_count - 1
        } else {
            current - 1
        };
        self.overall_panel_focus = panels[prev];
    }

    pub fn select_next_overall_panel(&mut self) {
        let panels = self.visible_overall_panels();
        let visible_count = if self.maintenance_panel_visible() {
            3
        } else {
            2
        };
        let current = panels[..visible_count]
            .iter()
            .position(|panel| *panel == self.overall_panel_focus)
            .unwrap_or(0);
        self.overall_panel_focus = panels[(current + 1) % visible_count];
    }

    pub fn active_overall_scroll(&self) -> &ScrollOffset {
        match self.overall_panel_focus {
            OverallPanelFocus::Incidents => &self.overall_incidents_scroll,
            OverallPanelFocus::Degradation => &self.overall_degradation_scroll,
            OverallPanelFocus::Maintenance => &self.overall_maintenance_scroll,
        }
    }

    pub fn scroll_active_overall_panel_up(&self) {
        self.active_overall_scroll().decrement(1);
    }

    pub fn scroll_active_overall_panel_down(&self) {
        self.active_overall_scroll().increment(1);
    }

    pub fn scroll_active_overall_panel_top(&self) {
        self.active_overall_scroll().jump_top();
    }

    pub fn scroll_active_overall_panel_bottom(&self) {
        self.active_overall_scroll().jump_bottom();
    }

    pub fn page_scroll_active_overall_panel_up(&self) {
        self.active_overall_scroll().decrement(PAGE_SIZE as u16);
    }

    pub fn page_scroll_active_overall_panel_down(&self) {
        self.active_overall_scroll().increment(PAGE_SIZE as u16);
    }

    // ── Detail panel focus (individual provider view) ─────────

    fn detail_has_services(&self) -> bool {
        self.current_entry()
            .is_some_and(|entry| entry.component_detail_available() || !entry.components.is_empty())
    }

    fn detail_has_maintenance(&self) -> bool {
        self.current_entry()
            .is_some_and(|entry| !entry.scheduled_maintenances.is_empty())
    }

    fn visible_detail_panels(&self) -> Vec<DetailPanelFocus> {
        let mut panels = Vec::new();
        if self.detail_has_services() {
            panels.push(DetailPanelFocus::Services);
        }
        panels.push(DetailPanelFocus::Incidents);
        if self.detail_has_maintenance() {
            panels.push(DetailPanelFocus::Maintenance);
        }
        panels
    }

    pub fn normalize_detail_panel_focus(&mut self) {
        let panels = self.visible_detail_panels();
        if !panels.contains(&self.detail_panel_focus) {
            self.detail_panel_focus = DetailPanelFocus::Incidents;
        }
    }

    pub fn select_prev_detail_panel(&mut self) {
        let panels = self.visible_detail_panels();
        if panels.is_empty() {
            return;
        }
        let current = panels
            .iter()
            .position(|p| *p == self.detail_panel_focus)
            .unwrap_or(0);
        let prev = if current == 0 {
            panels.len() - 1
        } else {
            current - 1
        };
        self.detail_panel_focus = panels[prev];
    }

    pub fn select_next_detail_panel(&mut self) {
        let panels = self.visible_detail_panels();
        if panels.is_empty() {
            return;
        }
        let current = panels
            .iter()
            .position(|p| *p == self.detail_panel_focus)
            .unwrap_or(0);
        self.detail_panel_focus = panels[(current + 1) % panels.len()];
    }

    pub fn active_detail_scroll(&self) -> &ScrollOffset {
        match self.detail_panel_focus {
            DetailPanelFocus::Services => &self.services_scroll,
            DetailPanelFocus::Incidents => &self.detail_scroll,
            DetailPanelFocus::Maintenance => &self.maintenance_scroll,
        }
    }

    pub fn scroll_active_detail_panel_up(&self) {
        self.active_detail_scroll().decrement(1);
    }

    pub fn scroll_active_detail_panel_down(&self) {
        self.active_detail_scroll().increment(1);
    }

    pub fn scroll_active_detail_panel_top(&self) {
        self.active_detail_scroll().jump_top();
    }

    pub fn scroll_active_detail_panel_bottom(&self) {
        self.active_detail_scroll().jump_bottom();
    }

    pub fn page_scroll_active_detail_panel_up(&self) {
        self.active_detail_scroll().decrement(PAGE_SIZE as u16);
    }

    pub fn page_scroll_active_detail_panel_down(&self) {
        self.active_detail_scroll().increment(PAGE_SIZE as u16);
    }
}

/// Handle a mouse event while the Status tab is active.
///
/// All state changes (focus, selection, scroll) are applied directly to the
/// `StatusApp`, so this returns `None`; the main loop redraws after every event.
///
/// Hit-testing distinguishes the two detail views by `is_overall_selected()`:
/// when Overall (display index 0) is selected the right side renders the overall
/// dashboard (incidents / degradation / maintenance SoftCard panels), otherwise
/// it renders the selected provider's detail (services / incidents / maintenance
/// panels). Only the matching rect set is consulted, and `render.rs` resets all
/// six sub-panel rects to `None` every frame so a vanished panel never matches.
pub fn handle_status_mouse(app: &mut App, ev: MouseEvent) -> Option<Message> {
    let s = app.status_app.as_mut()?;
    match ev.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if hit(s.provider_list_area, &ev) {
                s.focus = StatusFocus::List;
                if let Some(area) = s.provider_list_area {
                    // item index == display index (0 = Overall, a valid target).
                    if let Some(idx) = row_at(
                        area,
                        s.list_state.offset(),
                        0,
                        s.filtered_entries.len() + 1,
                        ev.row,
                    ) {
                        s.select_at_index(idx);
                    }
                }
            } else if s.is_overall_selected() {
                if hit(s.overall_incidents_area, &ev) {
                    s.focus = StatusFocus::Details;
                    s.overall_panel_focus = OverallPanelFocus::Incidents;
                } else if hit(s.overall_degradation_area, &ev) {
                    s.focus = StatusFocus::Details;
                    s.overall_panel_focus = OverallPanelFocus::Degradation;
                } else if hit(s.overall_maintenance_area, &ev) {
                    s.focus = StatusFocus::Details;
                    s.overall_panel_focus = OverallPanelFocus::Maintenance;
                }
            } else if hit(s.detail_services_area, &ev) {
                s.focus = StatusFocus::Details;
                s.detail_panel_focus = DetailPanelFocus::Services;
            } else if hit(s.detail_incidents_area, &ev) {
                s.focus = StatusFocus::Details;
                s.detail_panel_focus = DetailPanelFocus::Incidents;
            } else if hit(s.detail_maintenance_area, &ev) {
                s.focus = StatusFocus::Details;
                s.detail_panel_focus = DetailPanelFocus::Maintenance;
            }
        }
        // Wheel: focus the panel under the cursor, then scroll it (reusing the
        // same per-panel actions the arrow keys drive).
        MouseEventKind::ScrollDown => {
            if hit(s.provider_list_area, &ev) {
                s.focus = StatusFocus::List;
                s.next();
            } else if s.is_overall_selected() {
                if let Some(panel) = hit_overall_panel(s, &ev) {
                    s.focus = StatusFocus::Details;
                    s.overall_panel_focus = panel;
                    s.scroll_active_overall_panel_down();
                }
            } else if let Some(panel) = hit_detail_panel(s, &ev) {
                s.focus = StatusFocus::Details;
                s.detail_panel_focus = panel;
                s.scroll_active_detail_panel_down();
            }
        }
        MouseEventKind::ScrollUp => {
            if hit(s.provider_list_area, &ev) {
                s.focus = StatusFocus::List;
                s.prev();
            } else if s.is_overall_selected() {
                if let Some(panel) = hit_overall_panel(s, &ev) {
                    s.focus = StatusFocus::Details;
                    s.overall_panel_focus = panel;
                    s.scroll_active_overall_panel_up();
                }
            } else if let Some(panel) = hit_detail_panel(s, &ev) {
                s.focus = StatusFocus::Details;
                s.detail_panel_focus = panel;
                s.scroll_active_detail_panel_up();
            }
        }
        _ => {}
    }
    None
}

/// Which overall-dashboard sub-panel (if any) the event falls inside.
fn hit_overall_panel(s: &StatusApp, ev: &MouseEvent) -> Option<OverallPanelFocus> {
    if hit(s.overall_incidents_area, ev) {
        Some(OverallPanelFocus::Incidents)
    } else if hit(s.overall_degradation_area, ev) {
        Some(OverallPanelFocus::Degradation)
    } else if hit(s.overall_maintenance_area, ev) {
        Some(OverallPanelFocus::Maintenance)
    } else {
        None
    }
}

/// Which provider-detail sub-panel (if any) the event falls inside.
fn hit_detail_panel(s: &StatusApp, ev: &MouseEvent) -> Option<DetailPanelFocus> {
    if hit(s.detail_services_area, ev) {
        Some(DetailPanelFocus::Services)
    } else if hit(s.detail_incidents_area, ev) {
        Some(DetailPanelFocus::Incidents)
    } else if hit(s.detail_maintenance_area, ev) {
        Some(DetailPanelFocus::Maintenance)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn builds_unique_provider_entries_from_catalog() {
        let app = StatusApp::new(&Config::default());

        let slugs: Vec<_> = app
            .entries
            .iter()
            .map(|entry| entry.slug.as_str())
            .collect();
        assert!(slugs.contains(&"google"));
        assert!(slugs.contains(&"openai"));
        assert!(slugs.contains(&"openrouter"));
        assert!(slugs.contains(&"cursor"));
        assert_eq!(
            app.entries
                .iter()
                .find(|entry| entry.slug == "google")
                .map(|entry| entry.source_slug.as_str()),
            Some("gemini")
        );
        // All providers tracked by default, so fetch_seeds returns google
        assert_eq!(
            app.fetch_seeds()
                .iter()
                .find(|seed| seed.slug == "google")
                .map(|seed| seed.source_slug.as_str()),
            Some("gemini")
        );
    }

    #[test]
    fn health_counts_tallies_all_entries() {
        let app = StatusApp::new(&Config::default());

        // All entries start as Unknown health (from placeholders)
        let (op, deg, out, other) = app.health_counts();
        assert_eq!(op, 0);
        assert_eq!(deg, 0);
        assert_eq!(out, 0);
        assert!(other > 0); // all Unknown = other
    }

    #[test]
    fn provenance_counts_tallies_all_entries() {
        let app = StatusApp::new(&Config::default());

        let (official, fallback, unavailable) = app.provenance_counts();
        assert_eq!(official, 0);
        assert_eq!(fallback, 0);
        assert!(unavailable > 0);
    }

    #[test]
    fn overall_panel_focus_skips_maintenance_when_hidden() {
        let mut app = StatusApp::new(&Config::default());

        app.overall_panel_focus = OverallPanelFocus::Incidents;
        app.select_next_overall_panel();
        assert_eq!(app.overall_panel_focus, OverallPanelFocus::Degradation);

        app.select_next_overall_panel();
        assert_eq!(app.overall_panel_focus, OverallPanelFocus::Incidents);
    }

    #[test]
    fn overall_panel_focus_includes_maintenance_when_visible() {
        let mut app = StatusApp::new(&Config::default());

        if let Some(entry) = app.entries.first_mut() {
            entry.scheduled_maintenances.push(ScheduledMaintenance {
                name: "DB maintenance".to_string(),
                status: "scheduled".to_string(),
                impact: "none".to_string(),
                shortlink: None,
                scheduled_for: Some("2026-03-18T12:00:00Z".to_string()),
                scheduled_until: None,
                affected_components: vec!["API".to_string()],
            });
        }

        app.overall_panel_focus = OverallPanelFocus::Incidents;
        app.select_next_overall_panel();
        assert_eq!(app.overall_panel_focus, OverallPanelFocus::Degradation);

        app.select_next_overall_panel();
        assert_eq!(app.overall_panel_focus, OverallPanelFocus::Maintenance);

        app.select_next_overall_panel();
        assert_eq!(app.overall_panel_focus, OverallPanelFocus::Incidents);
    }

    #[test]
    fn fetch_seeds_respects_tracked() {
        let mut config = Config::default();
        // Only track openai
        config.status.tracked.clear();
        config.status.tracked.insert("openai".to_string());

        let app = StatusApp::new(&config);
        let seeds = app.fetch_seeds();
        assert_eq!(seeds.len(), 1);
        assert_eq!(seeds[0].slug, "openai");
    }

    #[test]
    fn fetch_seeds_all_tracked_by_default() {
        let app = StatusApp::new(&Config::default());
        let seeds = app.fetch_seeds();
        assert_eq!(seeds.len(), STATUS_REGISTRY.len());
    }
}
