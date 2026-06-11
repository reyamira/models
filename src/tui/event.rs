use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use super::app::{App, Message, Mode};
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

pub fn handle_events(app: &App) -> Result<Option<Message>> {
    if event::poll(Duration::from_millis(100))? {
        if let Event::Key(key) = event::read()? {
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
    }

    Ok(None)
}

fn handle_normal_mode(app: &App, code: KeyCode, modifiers: KeyModifiers) -> Option<Message> {
    // Check for modal popups (intercept before global keys to prevent e.g. 'q' quitting)
    if app.current_tab == super::app::Tab::Agents {
        if let Some(ref agents_app) = app.agents_app {
            if agents_app.show_picker {
                return handle_picker_keys(code);
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
        KeyCode::Char('c') => Some(Message::CopyAgentName),
        KeyCode::Char('1') => Some(Message::ToggleInstalledFilter),
        KeyCode::Char('2') => Some(Message::ToggleCliFilter),
        KeyCode::Char('3') => Some(Message::ToggleOpenSourceFilter),
        KeyCode::Char('a') => Some(Message::OpenPicker),
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
        // Quick sorts live on the back half of the number row (8/9/0): their
        // targets are per-source (first metric / date / speed-if-present), so
        // a stable-shaped footer can't honestly hint them — they're documented
        // in the help popup instead. 1-3 are deliberately unbound here.
        KeyCode::Char('8') => Some(Message::QuickSortIntelligence),
        KeyCode::Char('9') => Some(Message::QuickSortDate),
        KeyCode::Char('0') => Some(Message::QuickSortSpeed),
        KeyCode::Char('4') => Some(Message::CycleBenchmarkSource),
        KeyCode::Char('5') => Some(Message::ToggleRegionGrouping),
        KeyCode::Char('6') => Some(Message::ToggleTypeGrouping),
        // `7` cycles the reasoning filter — no-op (and footer-hidden) when no
        // model in the active source carries a reasoning status.
        KeyCode::Char('7')
            if super::benchmarks::BenchmarksApp::reasoning_filter_available(
                app.multi_store.file(app.benchmarks_app.active_source),
            ) =>
        {
            Some(Message::CycleReasoningFilter)
        }
        // `{` / `}` cycle data source prev/next (tab-local; `[` / `]` stay global).
        KeyCode::Char('{') => Some(Message::CycleDataSourcePrev),
        KeyCode::Char('}') => Some(Message::CycleDataSourceNext),
        // `r` re-fetches the active source (stale-while-revalidate).
        KeyCode::Char('r') => Some(Message::RefreshBenchmarkSource),
        KeyCode::Char('s') => Some(Message::OpenSortPicker),
        KeyCode::Char('S') => Some(Message::ToggleBenchmarkSortDir),
        KeyCode::Char('i') => Some(Message::ToggleGlossary),
        KeyCode::Char('c') if !app.selections.is_empty() => Some(Message::ClearBenchmarkSelections),
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
