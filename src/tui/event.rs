use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use super::app::{App, Message, Mode, Tab};
use super::models::Focus;

/// Shared navigation actions across all tabs
enum NavAction {
    Down,
    Up,
    First,
    Last,
    PageDown,
    PageUp,
    FocusLeft,
    FocusRight,
    FocusNext,
    Search,
    ClearEsc,
}

fn parse_nav_key(code: KeyCode, modifiers: KeyModifiers) -> Option<NavAction> {
    match code {
        KeyCode::Char('j') | KeyCode::Down => Some(NavAction::Down),
        KeyCode::Char('k') | KeyCode::Up => Some(NavAction::Up),
        KeyCode::Char('g') => Some(NavAction::First),
        KeyCode::Char('G') => Some(NavAction::Last),
        KeyCode::Char('d') if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(NavAction::PageDown)
        }
        KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => Some(NavAction::PageUp),
        KeyCode::PageDown => Some(NavAction::PageDown),
        KeyCode::PageUp => Some(NavAction::PageUp),
        KeyCode::Char('h') | KeyCode::Left => Some(NavAction::FocusLeft),
        KeyCode::Char('l') | KeyCode::Right => Some(NavAction::FocusRight),
        KeyCode::Tab | KeyCode::BackTab => Some(NavAction::FocusNext),
        KeyCode::Char('/') => Some(NavAction::Search),
        KeyCode::Esc => Some(NavAction::ClearEsc),
        _ => None,
    }
}

pub fn handle_events(app: &mut App) -> Result<Option<Message>> {
    if event::poll(Duration::from_millis(100))? {
        match event::read()? {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    return Ok(None);
                }

                // When help is showing, handle scroll and dismiss keys
                if app.show_help {
                    let msg = match key.code {
                        KeyCode::Char('?') | KeyCode::Esc => Some(Message::ToggleHelp),
                        KeyCode::Char('j') | KeyCode::Down => Some(Message::ScrollHelpDown),
                        KeyCode::Char('k') | KeyCode::Up => Some(Message::ScrollHelpUp),
                        _ => None,
                    };
                    return Ok(msg);
                }

                let msg = match app.mode {
                    Mode::Normal => handle_normal_mode(app, key.code, key.modifiers),
                    Mode::Search => handle_search_mode(key.code),
                };

                return Ok(msg);
            }
            Event::Mouse(ev) => return Ok(handle_mouse(app, ev)),
            _ => {}
        }
    }

    Ok(None)
}

/// True when a per-tab modal popup is open and should swallow mouse input so
/// clicks/scroll don't leak to the panels behind it. (The global help popup is
/// handled separately in `handle_mouse`.)
fn modal_popup_open(app: &App) -> bool {
    match app.current_tab {
        Tab::Agents => app
            .agents_app
            .as_ref()
            .is_some_and(|a| a.show_picker || a.show_add_form),
        Tab::Status => app.status_app.as_ref().is_some_and(|a| a.show_picker),
        Tab::Benchmarks => {
            app.benchmarks_app.show_sort_picker
                || app.benchmarks_app.show_glossary
                || app.benchmarks_app.show_column_picker
        }
        Tab::Models => false,
    }
}

/// Mouse handling while a per-tab modal popup is open: the wheel drives the
/// popup's own scroll/selection, a left-click selects/toggles the row under the
/// cursor, and everything else is swallowed so it can't reach the panels
/// behind. Mirrors the popup key handlers (`handle_picker_keys`,
/// `handle_sort_picker_keys`, `handle_glossary_keys`, `handle_column_picker_keys`).
fn handle_modal_popup_mouse(app: &mut App, ev: MouseEvent) -> Option<Message> {
    if let MouseEventKind::Down(MouseButton::Left) = ev.kind {
        return handle_modal_popup_click(app, ev);
    }
    // Wheel → the open popup's own scroll/navigation message.
    let (down, up) = match app.current_tab {
        // Add-agent form has no scrollable rows — swallow the wheel.
        Tab::Agents if app.agents_app.as_ref().is_some_and(|a| a.show_add_form) => return None,
        // Agents / Status provider-tracker checkbox modals.
        Tab::Agents | Tab::Status => (Message::PickerNext, Message::PickerPrev),
        Tab::Benchmarks if app.benchmarks_app.show_sort_picker => {
            (Message::SortPickerNext, Message::SortPickerPrev)
        }
        Tab::Benchmarks if app.benchmarks_app.show_glossary => {
            (Message::ScrollGlossaryDown, Message::ScrollGlossaryUp)
        }
        Tab::Benchmarks if app.benchmarks_app.show_column_picker => {
            (Message::ColumnPickerNext, Message::ColumnPickerPrev)
        }
        _ => return None,
    };
    match ev.kind {
        MouseEventKind::ScrollDown => Some(down),
        MouseEventKind::ScrollUp => Some(up),
        _ => None,
    }
}

/// Left-click inside a modal popup: map the click to the row under the cursor
/// (via the popup's cached inner rect + `mouse::popup_row_at`), set that row as
/// the popup's selection, and return its "act on the current row" message —
/// confirm for the sort picker (click-to-apply), toggle for the checkbox
/// pickers. Clicks outside the rows are swallowed; the glossary has no rows.
fn handle_modal_popup_click(app: &mut App, ev: MouseEvent) -> Option<Message> {
    use super::mouse::popup_row_at;
    match app.current_tab {
        Tab::Benchmarks if app.benchmarks_app.show_sort_picker => {
            let count = app
                .multi_store
                .file(app.benchmarks_app.active_source)
                .map(|f| super::benchmarks::BenchmarksApp::sort_options(f).len())
                .unwrap_or(0);
            let rect = app.benchmarks_app.sort_picker_area.get()?;
            let item = popup_row_at(rect, app.benchmarks_app.sort_picker_selected, count, ev.row)?;
            app.benchmarks_app.sort_picker_selected = item;
            Some(Message::SortPickerConfirm)
        }
        Tab::Benchmarks if app.benchmarks_app.show_column_picker => {
            let count = app
                .multi_store
                .file(app.benchmarks_app.active_source)
                .map(|f| f.metrics.len())
                .unwrap_or(0);
            let rect = app.benchmarks_app.column_picker_area.get()?;
            let item = popup_row_at(
                rect,
                app.benchmarks_app.column_picker_selected,
                count,
                ev.row,
            )?;
            app.benchmarks_app.column_picker_selected = item;
            Some(Message::ColumnPickerToggle)
        }
        // Glossary popup has no selectable rows — swallow the click.
        Tab::Benchmarks => None,
        Tab::Agents => {
            let a = app.agents_app.as_mut()?;
            // Add-agent form has no selectable rows — swallow the click.
            if a.show_add_form {
                return None;
            }
            let count = a.entries.len();
            let rect = a.picker_area.get()?;
            let item = popup_row_at(rect, a.picker_selected, count, ev.row)?;
            a.picker_selected = item;
            Some(Message::PickerToggle)
        }
        Tab::Status => {
            let s = app.status_app.as_mut()?;
            let count = crate::status::STATUS_REGISTRY.len();
            let rect = s.picker_area.get()?;
            let item = popup_row_at(rect, s.picker_selected, count, ev.row)?;
            s.picker_selected = item;
            Some(Message::PickerToggle)
        }
        Tab::Models => None,
    }
}

/// Dispatch a mouse event. High-frequency `Moved`/`Drag` events are dropped
/// (mouse capture enables any-motion tracking, which would otherwise flood the
/// loop). Popups take precedence over panels; the header tab bar is clickable;
/// otherwise the active tab's handler runs. Per-tab handlers apply focus,
/// selection, and scroll directly to `app`.
fn handle_mouse(app: &mut App, ev: MouseEvent) -> Option<Message> {
    if matches!(ev.kind, MouseEventKind::Moved | MouseEventKind::Drag(_)) {
        return None;
    }

    // Global help popup: scroll or click-to-close; never leaks to panels behind.
    if app.show_help {
        return match ev.kind {
            MouseEventKind::ScrollDown => Some(Message::ScrollHelpDown),
            MouseEventKind::ScrollUp => Some(Message::ScrollHelpUp),
            MouseEventKind::Down(MouseButton::Left) => Some(Message::ToggleHelp),
            _ => None,
        };
    }

    // Per-tab modal popups: route the wheel to the popup's own scroll/selection
    // and swallow clicks so nothing leaks to the panels behind.
    if modal_popup_open(app) {
        return handle_modal_popup_mouse(app, ev);
    }

    // Header tab bar: left-click a label to switch tabs.
    if let MouseEventKind::Down(MouseButton::Left) = ev.kind {
        if let Some(tab) = super::ui::tab_at(ev.column, ev.row) {
            app.current_tab = tab;
            return None;
        }
    }

    match app.current_tab {
        Tab::Models => super::models::handle_models_mouse(app, ev),
        Tab::Agents => super::agents::handle_agents_mouse(app, ev),
        Tab::Benchmarks => super::benchmarks::handle_benchmarks_mouse(app, ev),
        Tab::Status => super::status::handle_status_mouse(app, ev),
    }
}

fn handle_normal_mode(app: &App, code: KeyCode, modifiers: KeyModifiers) -> Option<Message> {
    // Check for modal popups (intercept before global keys to prevent e.g. 'q' quitting)
    if app.current_tab == super::app::Tab::Agents {
        if let Some(ref agents_app) = app.agents_app {
            if agents_app.show_picker {
                return handle_picker_keys(code);
            }
            if agents_app.show_add_form {
                return handle_add_agent_keys(code);
            }
        }
    }
    if app.current_tab == super::app::Tab::Status
        && app
            .status_app
            .as_ref()
            .map(|a| a.show_picker)
            .unwrap_or(false)
    {
        return handle_picker_keys(code);
    }
    if app.current_tab == super::app::Tab::Benchmarks && app.benchmarks_app.show_sort_picker {
        return handle_sort_picker_keys(code);
    }
    // Glossary popup intercepts keys before the global handler so 'q' scrolls/
    // closes rather than quitting the app.
    if app.current_tab == super::app::Tab::Benchmarks && app.benchmarks_app.show_glossary {
        return handle_glossary_keys(code);
    }
    // Column picker intercepts all keys while open (browse mode only).
    if app.current_tab == super::app::Tab::Benchmarks && app.benchmarks_app.show_column_picker {
        return handle_column_picker_keys(code, modifiers);
    }

    // Global keys (work on any tab)
    match code {
        KeyCode::Char('q') => return Some(Message::Quit),
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
            return Some(Message::Quit)
        }
        KeyCode::Char('[') => return Some(Message::PrevTab),
        KeyCode::Char(']') => return Some(Message::NextTab),
        KeyCode::Char('?') => return Some(Message::ToggleHelp),
        _ => {}
    }

    // Tab-specific keys
    match app.current_tab {
        super::app::Tab::Models => handle_models_keys(app, code, modifiers),
        super::app::Tab::Agents => handle_agents_keys(app, code, modifiers),
        super::app::Tab::Benchmarks => handle_benchmarks_keys(app, code, modifiers),
        super::app::Tab::Status => handle_status_keys(app, code, modifiers),
    }
}

fn resolve_models_nav(app: &App, action: NavAction) -> Option<Message> {
    match action {
        NavAction::Down => match app.models_app.focus {
            Focus::Providers => Some(Message::NextProvider),
            Focus::Models => Some(Message::NextModel),
            Focus::Details => Some(Message::ScrollModelDetailDown),
        },
        NavAction::Up => match app.models_app.focus {
            Focus::Providers => Some(Message::PrevProvider),
            Focus::Models => Some(Message::PrevModel),
            Focus::Details => Some(Message::ScrollModelDetailUp),
        },
        NavAction::First => match app.models_app.focus {
            Focus::Providers => Some(Message::SelectFirstProvider),
            Focus::Models => Some(Message::SelectFirstModel),
            Focus::Details => Some(Message::ScrollModelDetailTop),
        },
        NavAction::Last => match app.models_app.focus {
            Focus::Providers => Some(Message::SelectLastProvider),
            Focus::Models => Some(Message::SelectLastModel),
            Focus::Details => Some(Message::ScrollModelDetailBottom),
        },
        NavAction::PageDown => match app.models_app.focus {
            Focus::Providers => Some(Message::PageDownProvider),
            Focus::Models => Some(Message::PageDownModel),
            Focus::Details => Some(Message::PageScrollModelDetailDown),
        },
        NavAction::PageUp => match app.models_app.focus {
            Focus::Providers => Some(Message::PageUpProvider),
            Focus::Models => Some(Message::PageUpModel),
            Focus::Details => Some(Message::PageScrollModelDetailUp),
        },
        NavAction::FocusLeft => Some(Message::FocusModelLeft),
        NavAction::FocusRight | NavAction::FocusNext => Some(Message::FocusModelRight),
        NavAction::Search => Some(Message::EnterSearch),
        NavAction::ClearEsc => Some(Message::ClearSearch),
    }
}

fn handle_models_keys(app: &App, code: KeyCode, modifiers: KeyModifiers) -> Option<Message> {
    if let Some(action) = parse_nav_key(code, modifiers) {
        return resolve_models_nav(app, action);
    }
    match code {
        KeyCode::Char('c') => Some(Message::CopyFull),
        KeyCode::Char('C') => Some(Message::CopyModelId),
        KeyCode::Char('D') => Some(Message::CopyProviderDoc),
        KeyCode::Char('A') => Some(Message::CopyProviderApi),
        KeyCode::Char('o') => Some(Message::OpenProviderDoc),
        KeyCode::Char('r') => Some(Message::RefreshModels),
        KeyCode::Char('s') => Some(Message::CycleSort),
        KeyCode::Char('S') => Some(Message::ToggleSortDir),
        KeyCode::Char('1') => Some(Message::ToggleReasoning),
        KeyCode::Char('2') => Some(Message::ToggleTools),
        KeyCode::Char('3') => Some(Message::ToggleOpenWeights),
        KeyCode::Char('4') => Some(Message::ToggleFree),
        KeyCode::Char('5') => Some(Message::CycleProviderCategory),
        KeyCode::Char('6') => Some(Message::ToggleGrouping),
        _ => None,
    }
}

fn resolve_agents_nav(app: &App, action: NavAction) -> Option<Message> {
    use super::agents::AgentFocus;
    let focus = app
        .agents_app
        .as_ref()
        .map(|a| a.focus)
        .unwrap_or(AgentFocus::List);

    match action {
        NavAction::Down => {
            if focus == AgentFocus::List {
                Some(Message::NextAgent)
            } else {
                Some(Message::ScrollDetailDown)
            }
        }
        NavAction::Up => {
            if focus == AgentFocus::List {
                Some(Message::PrevAgent)
            } else {
                Some(Message::ScrollDetailUp)
            }
        }
        NavAction::First => {
            if focus == AgentFocus::List {
                Some(Message::SelectFirstAgent)
            } else {
                Some(Message::ScrollDetailTop)
            }
        }
        NavAction::Last => {
            if focus == AgentFocus::List {
                Some(Message::SelectLastAgent)
            } else {
                Some(Message::ScrollDetailBottom)
            }
        }
        NavAction::PageDown => {
            if focus == AgentFocus::List {
                Some(Message::PageDownAgent)
            } else {
                Some(Message::PageScrollDetailDown)
            }
        }
        NavAction::PageUp => {
            if focus == AgentFocus::List {
                Some(Message::PageUpAgent)
            } else {
                Some(Message::PageScrollDetailUp)
            }
        }
        NavAction::FocusLeft | NavAction::FocusRight | NavAction::FocusNext => {
            Some(Message::SwitchAgentFocus)
        }
        NavAction::Search => Some(Message::EnterSearch),
        NavAction::ClearEsc => Some(Message::ClearSearch),
    }
}

fn handle_agents_keys(app: &App, code: KeyCode, modifiers: KeyModifiers) -> Option<Message> {
    if let Some(action) = parse_nav_key(code, modifiers) {
        return resolve_agents_nav(app, action);
    }
    match code {
        KeyCode::Char('o') => Some(Message::OpenAgentDocs),
        KeyCode::Char('r') => Some(Message::OpenAgentRepo),
        KeyCode::Char('R') => Some(Message::RefreshAgents),
        KeyCode::Char('c') => Some(Message::CopyAgentName),
        KeyCode::Char('1') => Some(Message::ToggleInstalledFilter),
        KeyCode::Char('2') => Some(Message::ToggleCliFilter),
        KeyCode::Char('3') => Some(Message::ToggleOpenSourceFilter),
        KeyCode::Char('a') => Some(Message::OpenPicker),
        KeyCode::Char('A') => Some(Message::OpenAddAgent),
        KeyCode::Char('n') => Some(Message::NextSearchMatch),
        KeyCode::Char('N') => Some(Message::PrevSearchMatch),
        KeyCode::Char('s') => Some(Message::CycleAgentSort),
        _ => None,
    }
}

fn handle_sort_picker_keys(code: KeyCode) -> Option<Message> {
    match code {
        KeyCode::Char('j') | KeyCode::Down => Some(Message::SortPickerNext),
        KeyCode::Char('k') | KeyCode::Up => Some(Message::SortPickerPrev),
        KeyCode::Enter => Some(Message::SortPickerConfirm),
        KeyCode::Esc | KeyCode::Char('s') => Some(Message::CloseSortPicker),
        _ => None,
    }
}

/// Glossary popup keys: `i`/`Esc` close, arrows/`j`/`k` scroll. All other keys
/// are swallowed so the popup is modal (e.g. `q` does not quit).
fn handle_glossary_keys(code: KeyCode) -> Option<Message> {
    match code {
        KeyCode::Char('i') | KeyCode::Esc => Some(Message::ToggleGlossary),
        KeyCode::Char('j') | KeyCode::Down => Some(Message::ScrollGlossaryDown),
        KeyCode::Char('k') | KeyCode::Up => Some(Message::ScrollGlossaryUp),
        _ => None,
    }
}

/// Column picker popup keys. Intercepts all keys so `q` / `?` / etc. don't pass
/// through to the global handler.
fn handle_column_picker_keys(code: KeyCode, modifiers: KeyModifiers) -> Option<Message> {
    match code {
        KeyCode::Char('j') | KeyCode::Down => Some(Message::ColumnPickerNext),
        KeyCode::Char('k') | KeyCode::Up => Some(Message::ColumnPickerPrev),
        KeyCode::Char('g') => Some(Message::ColumnPickerFirst),
        KeyCode::Char('G') => Some(Message::ColumnPickerLast),
        KeyCode::Char('d') if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Message::ColumnPickerLast)
        }
        KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Message::ColumnPickerFirst)
        }
        KeyCode::PageDown => Some(Message::ColumnPickerLast),
        KeyCode::PageUp => Some(Message::ColumnPickerFirst),
        KeyCode::Char(' ') => Some(Message::ColumnPickerToggle),
        KeyCode::Enter => Some(Message::ColumnPickerSave),
        KeyCode::Esc => Some(Message::ColumnPickerCancel),
        _ => None,
    }
}

fn resolve_benchmarks_nav(app: &App, action: NavAction) -> Option<Message> {
    use super::benchmarks::BenchmarkFocus;
    let focus = app.benchmarks_app.focus;
    let is_h2h_compare = focus == BenchmarkFocus::Compare
        && app.benchmarks_app.bottom_view == super::benchmarks::BottomView::H2H;

    match action {
        NavAction::Down => match focus {
            BenchmarkFocus::Creators => Some(Message::NextBenchmarkCreator),
            BenchmarkFocus::List => Some(Message::NextBenchmark),
            BenchmarkFocus::Details => Some(Message::ScrollBenchmarkDetailDown),
            BenchmarkFocus::Compare if is_h2h_compare => Some(Message::ScrollH2HDown),
            BenchmarkFocus::Compare => None,
        },
        NavAction::Up => match focus {
            BenchmarkFocus::Creators => Some(Message::PrevBenchmarkCreator),
            BenchmarkFocus::List => Some(Message::PrevBenchmark),
            BenchmarkFocus::Details => Some(Message::ScrollBenchmarkDetailUp),
            BenchmarkFocus::Compare if is_h2h_compare => Some(Message::ScrollH2HUp),
            BenchmarkFocus::Compare => None,
        },
        NavAction::First => match focus {
            BenchmarkFocus::Creators => Some(Message::SelectFirstBenchmarkCreator),
            BenchmarkFocus::List => Some(Message::SelectFirstBenchmark),
            BenchmarkFocus::Details => Some(Message::ScrollBenchmarkDetailTop),
            BenchmarkFocus::Compare if is_h2h_compare => Some(Message::ScrollH2HTop),
            BenchmarkFocus::Compare => None,
        },
        NavAction::Last => match focus {
            BenchmarkFocus::Creators => Some(Message::SelectLastBenchmarkCreator),
            BenchmarkFocus::List => Some(Message::SelectLastBenchmark),
            BenchmarkFocus::Details => Some(Message::ScrollBenchmarkDetailBottom),
            BenchmarkFocus::Compare => None,
        },
        NavAction::PageDown => match focus {
            BenchmarkFocus::Creators => Some(Message::PageDownBenchmarkCreator),
            BenchmarkFocus::List => Some(Message::PageDownBenchmark),
            BenchmarkFocus::Details => Some(Message::PageScrollBenchmarkDetailDown),
            BenchmarkFocus::Compare if is_h2h_compare => Some(Message::ScrollH2HPageDown),
            BenchmarkFocus::Compare => None,
        },
        NavAction::PageUp => match focus {
            BenchmarkFocus::Creators => Some(Message::PageUpBenchmarkCreator),
            BenchmarkFocus::List => Some(Message::PageUpBenchmark),
            BenchmarkFocus::Details => Some(Message::PageScrollBenchmarkDetailUp),
            BenchmarkFocus::Compare if is_h2h_compare => Some(Message::ScrollH2HPageUp),
            BenchmarkFocus::Compare => None,
        },
        NavAction::FocusLeft => Some(Message::FocusBenchmarkLeft),
        NavAction::FocusRight | NavAction::FocusNext => Some(Message::FocusBenchmarkRight),
        NavAction::Search => Some(Message::EnterSearch),
        NavAction::ClearEsc => {
            if app.benchmarks_app.show_detail_overlay {
                Some(Message::CloseDetailOverlay)
            } else {
                Some(Message::ClearSearch)
            }
        }
    }
}

fn handle_benchmarks_keys(app: &App, code: KeyCode, modifiers: KeyModifiers) -> Option<Message> {
    if let Some(action) = parse_nav_key(code, modifiers) {
        return resolve_benchmarks_nav(app, action);
    }
    match code {
        // Number row: 1/2 = creator grouping, 3 = reasoning filter, 4 = weights
        // filter. There are no quick-sort number keys — the `s` sort picker and
        // `S` direction toggle cover sorting.
        KeyCode::Char('1') => Some(Message::ToggleRegionGrouping),
        KeyCode::Char('2') => Some(Message::ToggleTypeGrouping),
        // `3` cycles the reasoning filter — no-op (and footer-hidden) when no
        // model in the active source carries a reasoning status.
        KeyCode::Char('3')
            if super::benchmarks::BenchmarksApp::reasoning_filter_available(
                app.multi_store.file(app.benchmarks_app.active_source),
            ) =>
        {
            Some(Message::CycleReasoningFilter)
        }
        KeyCode::Char('4') => Some(Message::CycleBenchmarkSource),
        // `{` / `}` cycle data source prev/next (tab-local; `[` / `]` stay global).
        KeyCode::Char('{') => Some(Message::CycleDataSourcePrev),
        KeyCode::Char('}') => Some(Message::CycleDataSourceNext),
        // `r` re-fetches the active source (stale-while-revalidate).
        KeyCode::Char('r') => Some(Message::RefreshBenchmarkSource),
        KeyCode::Char('s') => Some(Message::OpenSortPicker),
        KeyCode::Char('S') => Some(Message::ToggleBenchmarkSortDir),
        KeyCode::Char('i') => Some(Message::ToggleGlossary),
        KeyCode::Char('c') if !app.selections.is_empty() => Some(Message::ClearBenchmarkSelections),
        // `C` opens the column picker — browse mode only (< 2 selections).
        KeyCode::Char('C') if app.selections.len() < 2 => Some(Message::OpenColumnPicker),
        KeyCode::Char('o') => Some(Message::OpenBenchmarkUrl),
        KeyCode::Char(' ') => Some(Message::ToggleBenchmarkSelection),
        KeyCode::Char('v') if app.selections.len() >= 2 => Some(Message::CycleBenchmarkView),
        KeyCode::Char('x')
            if app.benchmarks_app.bottom_view == super::benchmarks::BottomView::Scatter =>
        {
            Some(Message::CycleScatterX)
        }
        KeyCode::Char('y')
            if app.benchmarks_app.bottom_view == super::benchmarks::BottomView::Scatter =>
        {
            Some(Message::CycleScatterY)
        }
        KeyCode::Char('a')
            if app.benchmarks_app.bottom_view == super::benchmarks::BottomView::Radar =>
        {
            Some(Message::CycleRadarPreset)
        }
        // Browse mode (< 2 selections): `a` cycles the detail comparator column.
        // The radar-preset `a` above is guarded to compare mode (Radar view), so
        // the two never collide.
        KeyCode::Char('a') if app.selections.len() < 2 => Some(Message::CycleComparator),
        KeyCode::Char('d') if app.selections.len() >= 2 => Some(Message::ToggleDetailOverlay),
        KeyCode::Char('t') if app.selections.len() >= 2 => Some(Message::ToggleComparePanel),
        _ => None,
    }
}

/// Add-agent form keys. Intercepts all keys (returning `None` for unhandled
/// ones) so the modal is exclusive — `q`/`?`/etc. don't pass through to the
/// global handler while typing into a field.
fn handle_add_agent_keys(code: KeyCode) -> Option<Message> {
    match code {
        KeyCode::Esc => Some(Message::CloseAddAgent),
        KeyCode::Enter => Some(Message::AddAgentSave),
        KeyCode::Tab | KeyCode::Down | KeyCode::Up => Some(Message::AddAgentToggleField),
        KeyCode::Backspace => Some(Message::AddAgentBackspace),
        KeyCode::Char(c) => Some(Message::AddAgentInput(c)),
        _ => None,
    }
}

fn handle_picker_keys(code: KeyCode) -> Option<Message> {
    match code {
        KeyCode::Char('j') | KeyCode::Down => Some(Message::PickerNext),
        KeyCode::Char('k') | KeyCode::Up => Some(Message::PickerPrev),
        KeyCode::Char(' ') => Some(Message::PickerToggle),
        KeyCode::Enter => Some(Message::PickerSave),
        KeyCode::Esc => Some(Message::ClosePicker),
        _ => None,
    }
}

fn resolve_status_nav(app: &App, action: NavAction) -> Option<Message> {
    use super::status::StatusFocus;
    let focus = app
        .status_app
        .as_ref()
        .map(|a| a.focus)
        .unwrap_or(StatusFocus::List);
    match action {
        NavAction::Down => {
            if focus == StatusFocus::List {
                Some(Message::NextStatusProvider)
            } else {
                Some(Message::ScrollStatusDetailDown)
            }
        }
        NavAction::Up => {
            if focus == StatusFocus::List {
                Some(Message::PrevStatusProvider)
            } else {
                Some(Message::ScrollStatusDetailUp)
            }
        }
        NavAction::First => {
            if focus == StatusFocus::List {
                Some(Message::SelectFirstStatusProvider)
            } else {
                Some(Message::ScrollStatusDetailTop)
            }
        }
        NavAction::Last => {
            if focus == StatusFocus::List {
                Some(Message::SelectLastStatusProvider)
            } else {
                Some(Message::ScrollStatusDetailBottom)
            }
        }
        NavAction::PageDown => {
            if focus == StatusFocus::List {
                Some(Message::PageDownStatusProvider)
            } else {
                Some(Message::PageScrollStatusDetailDown)
            }
        }
        NavAction::PageUp => {
            if focus == StatusFocus::List {
                Some(Message::PageUpStatusProvider)
            } else {
                Some(Message::PageScrollStatusDetailUp)
            }
        }
        NavAction::FocusLeft => {
            if focus == StatusFocus::Details {
                Some(Message::PrevOverallStatusPanel)
            } else {
                Some(Message::SwitchStatusFocus)
            }
        }
        NavAction::FocusRight => {
            if focus == StatusFocus::Details {
                Some(Message::NextOverallStatusPanel)
            } else {
                Some(Message::SwitchStatusFocus)
            }
        }
        NavAction::FocusNext => Some(Message::SwitchStatusFocus),
        NavAction::Search => Some(Message::EnterSearch),
        NavAction::ClearEsc => Some(Message::ClearSearch),
    }
}

fn handle_status_keys(app: &App, code: KeyCode, modifiers: KeyModifiers) -> Option<Message> {
    if let Some(action) = parse_nav_key(code, modifiers) {
        return resolve_status_nav(app, action);
    }
    match code {
        KeyCode::Char('o') => Some(Message::OpenStatusPage),
        KeyCode::Char('r') => Some(Message::RefreshStatus),
        KeyCode::Char('a') => Some(Message::OpenStatusPicker),
        _ => None,
    }
}

fn handle_search_mode(code: KeyCode) -> Option<Message> {
    match code {
        KeyCode::Esc | KeyCode::Enter => Some(Message::ExitSearch),
        KeyCode::Backspace => Some(Message::SearchBackspace),
        KeyCode::Char(c) => Some(Message::SearchInput(c)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    fn app() -> App {
        App::new(Default::default(), None, None)
    }

    fn click_at(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn wheel(down: bool) -> MouseEvent {
        MouseEvent {
            kind: if down {
                MouseEventKind::ScrollDown
            } else {
                MouseEventKind::ScrollUp
            },
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn left_click() -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        }
    }

    // `Message` does not derive `PartialEq`, so assert via `matches!`.

    #[test]
    fn glossary_popup_wheel_scrolls_glossary() {
        let mut a = app();
        a.current_tab = Tab::Benchmarks;
        a.benchmarks_app.show_glossary = true;
        // Full dispatch path: handle_mouse → modal_popup_open → popup scroll.
        assert!(matches!(
            handle_mouse(&mut a, wheel(true)),
            Some(Message::ScrollGlossaryDown)
        ));
        assert!(matches!(
            handle_mouse(&mut a, wheel(false)),
            Some(Message::ScrollGlossaryUp)
        ));
        // Clicks are swallowed so they don't leak to the panel behind the popup.
        assert!(handle_mouse(&mut a, left_click()).is_none());
    }

    #[test]
    fn sort_picker_wheel_moves_selection() {
        let mut a = app();
        a.current_tab = Tab::Benchmarks;
        a.benchmarks_app.show_sort_picker = true;
        assert!(matches!(
            handle_mouse(&mut a, wheel(true)),
            Some(Message::SortPickerNext)
        ));
        assert!(matches!(
            handle_mouse(&mut a, wheel(false)),
            Some(Message::SortPickerPrev)
        ));
    }

    #[test]
    fn column_picker_wheel_moves_selection() {
        let mut a = app();
        a.current_tab = Tab::Benchmarks;
        a.benchmarks_app.show_column_picker = true;
        assert!(matches!(
            handle_mouse(&mut a, wheel(true)),
            Some(Message::ColumnPickerNext)
        ));
    }

    #[test]
    fn help_popup_wheel_scrolls_and_click_closes() {
        let mut a = app();
        a.show_help = true;
        assert!(matches!(
            handle_mouse(&mut a, wheel(true)),
            Some(Message::ScrollHelpDown)
        ));
        assert!(matches!(
            handle_mouse(&mut a, left_click()),
            Some(Message::ToggleHelp)
        ));
    }

    #[test]
    fn tracker_modal_click_selects_and_toggles_row() {
        let mut a = app();
        a.current_tab = Tab::Status;
        {
            let s = a.status_app.as_mut().unwrap();
            s.show_picker = true;
            // Inner list rect: rows 5..25, tall enough that the registry fits
            // (offset 0). Click the 5th row (y = 5 + 4) → item index 4.
            s.picker_area.set(Some(Rect::new(0, 5, 50, 20)));
        }
        assert!(matches!(
            handle_mouse(&mut a, click_at(10, 9)),
            Some(Message::PickerToggle)
        ));
        assert_eq!(a.status_app.as_ref().unwrap().picker_selected, 4);

        // A click below the item rows is swallowed and changes nothing.
        assert!(handle_mouse(&mut a, click_at(10, 200)).is_none());
        assert_eq!(a.status_app.as_ref().unwrap().picker_selected, 4);
    }

    #[test]
    fn moved_events_are_ignored() {
        let mut a = app();
        let moved = MouseEvent {
            kind: MouseEventKind::Moved,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        assert!(handle_mouse(&mut a, moved).is_none());
    }
}
