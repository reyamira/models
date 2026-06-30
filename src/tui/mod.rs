use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub mod agents;
pub mod app;
pub mod benchmarks;
pub mod event;
pub mod markdown;
pub mod models;
pub mod mouse;
pub mod status;
pub mod ui;
pub mod widgets;

use crate::agents::{
    load_agents, AsyncGitHubClient, ConditionalFetchResult, GitHubCache, GitHubData,
};
use crate::benchmarks::fetch_source;
use crate::benchmarks::schema::SourceFile;
use crate::benchmarks::sources::SOURCES;
use crate::config::Config;
use crate::data::ProvidersMap;
use crate::status::{StatusFetchResult, StatusFetcher};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Copy text to clipboard, keeping it alive on Linux.
/// On Linux, the clipboard is selection-based and needs the source app to stay alive.
/// We spawn a thread to hold the clipboard for a few seconds.
fn copy_to_clipboard(text: String) {
    std::thread::spawn(move || {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let _ = clipboard.set_text(&text);
            // Keep clipboard alive for other apps to read on Linux
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
}

/// Peek the label of the metric that a scatter axis will advance to, used for
/// the status-bar message before `app.update` runs the cycle.
fn peek_next_metric_label(app: &app::App, current: usize) -> Option<String> {
    let file = app.active_benchmark_file()?;
    let n = file.metrics.len();
    if n == 0 {
        return None;
    }
    let next = (current + 1) % n;
    Some(file.metrics[next].label.clone())
}

/// Peek the name of the radar group the preset will advance to.
fn peek_next_radar_group_label(app: &app::App) -> Option<String> {
    let file = app.active_benchmark_file()?;
    let groups = crate::benchmarks::multi::radar_groups(file);
    if groups.is_empty() {
        return None;
    }
    let next = (app.benchmarks_app.radar_group + 1) % groups.len();
    Some(groups[next].clone())
}

/// Result of a GitHub fetch operation for an agent.
#[derive(Debug)]
pub enum FetchResult {
    /// Successful fetch: (agent_id, github_data)
    Success(String, GitHubData),
    /// Failed fetch: (agent_id, error_message)
    Failure(String, String),
}

/// Progress/result of a background agent self-update subprocess.
#[derive(Debug)]
pub enum UpdateEvent {
    /// A captured stdout/stderr line: (agent_id, line).
    Output(String, String),
    /// Terminal result: (agent_id, success, summary_message).
    Finished(String, bool, String),
    /// Version re-detected after a successful update: (agent_id, installed).
    Redetected(String, crate::agents::InstalledInfo),
}

struct StatusRuntime {
    rx: mpsc::Receiver<(u64, StatusFetchResult)>,
    tx: mpsc::Sender<(u64, StatusFetchResult)>,
    client: reqwest::Client,
    last_fetch_time: Option<Instant>,
    fetch_generation: u64,
}

struct RuntimeHandles {
    github_rx: mpsc::Receiver<FetchResult>,
    github_tx: mpsc::Sender<FetchResult>,
    /// Background agent-update progress/results.
    update_rx: mpsc::Receiver<UpdateEvent>,
    update_tx: mpsc::Sender<UpdateEvent>,
    client: AsyncGitHubClient,
    disk_cache: Arc<RwLock<GitHubCache>>,
    /// One message per source fetch: `(source_idx, Option<SourceFile>)`.
    bench_rx: mpsc::Receiver<(usize, Option<SourceFile>)>,
    /// `r`-triggered active-source refetch results: `(source_idx, Option<SourceFile>)`.
    refresh_rx: mpsc::Receiver<(usize, Option<SourceFile>)>,
    refresh_tx: mpsc::Sender<(usize, Option<SourceFile>)>,
    /// `r`-triggered models.dev refetch result.
    models_refresh_rx: mpsc::Receiver<Option<crate::data::ProvidersMap>>,
    models_refresh_tx: mpsc::Sender<Option<crate::data::ProvidersMap>>,
    /// Final URL opened by an async benchmark-url task (Epoch 404-fallback path).
    url_rx: mpsc::Receiver<String>,
    url_tx: mpsc::Sender<String>,
    status: StatusRuntime,
}
pub async fn run(providers: ProvidersMap) -> Result<()> {
    // Load remaining data
    let agents_file = load_agents().ok();
    let config = Config::load().ok();

    // Benchmark data fetched from CDN in background; the multi-store starts with
    // every source in the Loading state until each fetch lands.

    // Load disk cache for GitHub data (load before wrapping to avoid blocking in async)
    let disk_cache = GitHubCache::load();

    // Create app BEFORE entering alternate screen
    let mut app = app::App::new(providers, agents_file.as_ref(), config);

    // Install panic hook to restore terminal on crash
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // Restore terminal before printing panic message
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original_hook(panic_info);
    }));

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Pre-populate agent entries from disk cache for instant display, but leave
    // the fetch_status at the constructor's `Loading` (set for tracked entries
    // in AgentsApp::new). `spawn_agent_fetches` only dispatches for `Loading |
    // NotStarted` entries, so marking these `Loaded` here would skip the startup
    // background fetch entirely — leaving stale cached changelogs on screen until
    // the user pressed `R`. Seeding the cached data gives instant display; the
    // background fetch then revalidates and flips the status to Loaded on
    // success (stale-while-revalidate).
    if let Some(ref mut agents_app) = app.agents_app {
        for entry in &mut agents_app.entries {
            if entry.tracked {
                // Look up cached data by repo (cache keys are repos)
                if let Some(cached) = disk_cache.get(&entry.agent.repo) {
                    entry.github = cached.data.clone().into();
                }
            }
        }
        // Re-apply sorting after populating cache data (in case sorted by stars/updated)
        agents_app.apply_sort();
    }

    // Now wrap cache in Arc<RwLock> for async sharing
    let disk_cache = Arc::new(RwLock::new(disk_cache));

    // Create GitHub client and channel for fetch results
    let token = crate::agents::github::detect_github_token();
    let client = AsyncGitHubClient::with_disk_cache(token, disk_cache.clone());
    let (tx, rx) = mpsc::channel(100);

    // Spawn background GitHub fetches for agents (non-blocking)
    // Uses conditional fetches with ETag to avoid re-downloading unchanged data.
    // Uses the shared `spawn_agent_fetches` helper so the refresh path (`R`) is
    // identical.
    let fetch_handles = if let Some(ref agents_app) = app.agents_app {
        spawn_agent_fetches(
            &agents_app.entries,
            tx.clone(),
            client.clone(),
            disk_cache.clone(),
        )
    } else {
        Vec::new()
    };

    // Spawn one background fetch per compiled-in data source. Each posts its
    // source index alongside the result so the main loop can route it.
    let (bench_tx, bench_rx) = mpsc::channel(SOURCES.len().max(1));
    for (idx, descriptor) in SOURCES.iter().enumerate() {
        let bench_tx = bench_tx.clone();
        tokio::spawn(async move {
            let result = fetch_source(descriptor).await;
            let _ = bench_tx.send((idx, result)).await;
        });
    }

    let (status_tx, status_rx) = mpsc::channel(4);
    let status_client = reqwest::Client::builder()
        .user_agent("models-tui")
        .connect_timeout(Duration::from_secs(5))
        .build()
        .expect("Failed to build HTTP client");
    if let Some(ref status_app) = app.status_app {
        let seeds = status_app.fetch_seeds();
        let tx = status_tx.clone();
        let fetcher = StatusFetcher::with_client(status_client.clone());
        tokio::spawn(async move {
            let result = fetcher.fetch(&seeds).await;
            let _ = tx.send((0, result)).await;
        });
    }

    let status_runtime = StatusRuntime {
        rx: status_rx,
        tx: status_tx,
        client: status_client,
        last_fetch_time: None,
        fetch_generation: 0,
    };
    let (url_tx, url_rx) = mpsc::channel(8);
    let (refresh_tx, refresh_rx) = mpsc::channel(SOURCES.len().max(1));
    let (models_refresh_tx, models_refresh_rx) = mpsc::channel(2);
    let (update_tx, update_rx) = mpsc::channel(256);
    let runtime_handles = RuntimeHandles {
        github_rx: rx,
        github_tx: tx,
        update_rx,
        update_tx,
        client,
        disk_cache: disk_cache.clone(),
        bench_rx,
        refresh_rx,
        refresh_tx,
        models_refresh_rx,
        models_refresh_tx,
        url_rx,
        url_tx,
        status: status_runtime,
    };
    let result = run_app(&mut terminal, &mut app, runtime_handles);

    // Abort any remaining fetch tasks to allow clean shutdown
    for handle in fetch_handles {
        handle.abort();
    }

    // Save cache to disk before exiting (best-effort, don't crash on failure)
    // Use try_read() to avoid blocking in async context
    if let Ok(cache_guard) = disk_cache.try_read() {
        // Ignore save errors - cache is not critical and we don't want to crash on exit
        let _ = cache_guard.save();
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

/// Spawn conditional GitHub fetches for tracked agents — used both at startup
/// and when `R` is pressed on the Agents tab. Only spawns for entries that are
/// `tracked` and in a `Loading` or `NotStarted` fetch state. Each fetch posts
/// its result to `tx`. Returns the spawned task handles so the caller can abort
/// them on shutdown if needed.
fn spawn_agent_fetches(
    entries: &[crate::agents::AgentEntry],
    tx: mpsc::Sender<FetchResult>,
    client: AsyncGitHubClient,
    cache: Arc<RwLock<GitHubCache>>,
) -> Vec<tokio::task::JoinHandle<()>> {
    use crate::agents::FetchStatus;
    let mut handles = Vec::new();
    for entry in entries.iter().filter(|e| {
        e.tracked
            && matches!(
                e.fetch_status,
                FetchStatus::Loading | FetchStatus::NotStarted
            )
    }) {
        let tx = tx.clone();
        let client = client.clone();
        let id = entry.id.clone();
        let repo = entry.agent.repo.clone();
        let cache = cache.clone();

        let handle = tokio::spawn(async move {
            let result = match client.fetch_conditional(&repo).await {
                crate::agents::ConditionalFetchResult::Fresh(data, _etag) => {
                    FetchResult::Success(id, data)
                }
                crate::agents::ConditionalFetchResult::NotModified => {
                    let cache_guard = cache.read().await;
                    if let Some(cached) = cache_guard.get(&repo) {
                        FetchResult::Success(id, cached.data.clone().into())
                    } else {
                        FetchResult::Failure(id, "Cache miss on NotModified".to_string())
                    }
                }
                crate::agents::ConditionalFetchResult::Error(e) => FetchResult::Failure(id, e),
            };
            let _ = tx.send(result).await;
        });
        handles.push(handle);
    }
    handles
}

/// Run one agent's verified self-update as a background subprocess, streaming its
/// stdout/stderr line-by-line over `tx` (no TTY — interactive prompts will hang
/// and hit the 5-minute timeout). On clean exit, re-detect the installed version
/// off the runtime so the status dot can flip without a restart. All output is
/// flushed before `Finished` so the log reads in order.
fn spawn_agent_update(
    id: String,
    agent: crate::agents::Agent,
    command: Vec<String>,
    tx: mpsc::Sender<UpdateEvent>,
) {
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command;

    tokio::spawn(async move {
        if command.is_empty() {
            let _ = tx
                .send(UpdateEvent::Finished(
                    id,
                    false,
                    "empty update command".to_string(),
                ))
                .await;
            return;
        }

        let mut cmd = Command::new(&command[0]);
        cmd.args(&command[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Put the child in its own process group so it can't read the TUI's
        // controlling terminal. A tool that opens /dev/tty for a prompt (sudo)
        // is then a background-group reader → SIGTTIN-stopped (caught by the
        // timeout) rather than stealing keystrokes or corrupting the screen.
        #[cfg(unix)]
        cmd.process_group(0);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = tx
                    .send(UpdateEvent::Finished(
                        id,
                        false,
                        format!("✗ failed to start `{}`: {e}", command[0]),
                    ))
                    .await;
                return;
            }
        };

        // Forward each stream's lines concurrently; keep the handles so we can
        // drain both before reporting the result.
        let mut readers = Vec::new();
        if let Some(out) = child.stdout.take() {
            let tx = tx.clone();
            let id = id.clone();
            readers.push(tokio::spawn(async move {
                let mut lines = BufReader::new(out).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if tx
                        .send(UpdateEvent::Output(id.clone(), line))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }
        if let Some(err) = child.stderr.take() {
            let tx = tx.clone();
            let id = id.clone();
            readers.push(tokio::spawn(async move {
                let mut lines = BufReader::new(err).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if tx
                        .send(UpdateEvent::Output(id.clone(), line))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }

        // Bound the run so a hung or prompt-waiting updater can't run forever.
        let wait = tokio::time::timeout(Duration::from_secs(300), child.wait()).await;
        let (success, message) = match wait {
            Ok(Ok(status)) if status.success() => (true, "✓ update completed".to_string()),
            Ok(Ok(status)) => (false, format!("✗ updater exited with {status}")),
            Ok(Err(e)) => (false, format!("✗ updater error: {e}")),
            Err(_) => {
                let _ = child.start_kill();
                (
                    false,
                    "✗ update timed out after 5m (needs an interactive prompt?)".to_string(),
                )
            }
        };

        // Flush all captured output before the terminal message.
        for r in readers {
            let _ = r.await;
        }

        if success {
            let agent = agent.clone();
            if let Ok(installed) =
                tokio::task::spawn_blocking(move || crate::agents::detect_installed(&agent)).await
            {
                let _ = tx
                    .send(UpdateEvent::Redetected(id.clone(), installed))
                    .await;
            }
        }

        let _ = tx.send(UpdateEvent::Finished(id, success, message)).await;
    });
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut app::App,
    mut runtime: RuntimeHandles,
) -> Result<()> {
    let mut last_status_time: Option<std::time::Instant> = None;

    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        // Clear status after 2 seconds
        if let Some(time) = last_status_time {
            if time.elapsed() > std::time::Duration::from_secs(2) {
                app.clear_status();
                last_status_time = None;
            }
        }

        // Spawn fetches for newly tracked agents
        if !app.pending_fetches.is_empty() {
            let fetches = std::mem::take(&mut app.pending_fetches);
            for (agent_id, repo) in fetches {
                let tx = runtime.github_tx.clone();
                let client = runtime.client.clone();
                let cache = runtime.disk_cache.clone();

                tokio::spawn(async move {
                    let result = match client.fetch_conditional(&repo).await {
                        ConditionalFetchResult::Fresh(data, _etag) => {
                            FetchResult::Success(agent_id, data)
                        }
                        ConditionalFetchResult::NotModified => {
                            let cache_guard = cache.read().await;
                            if let Some(cached) = cache_guard.get(&repo) {
                                FetchResult::Success(agent_id, cached.data.clone().into())
                            } else {
                                FetchResult::Failure(
                                    agent_id,
                                    "Cache miss on NotModified".to_string(),
                                )
                            }
                        }
                        ConditionalFetchResult::Error(e) => FetchResult::Failure(agent_id, e),
                    };
                    let _ = tx.send(result).await;
                });
            }
        }

        // Spawn confirmed agent self-updates as background subprocesses. Look up
        // each agent (for the post-update version re-detect) at drain time.
        if !app.pending_updates.is_empty() {
            let updates = std::mem::take(&mut app.pending_updates);
            for (agent_id, command) in updates {
                let agent = app
                    .agents_app
                    .as_ref()
                    .and_then(|a| a.entries.iter().find(|e| e.id == agent_id))
                    .map(|e| e.agent.clone());
                if let Some(agent) = agent {
                    spawn_agent_update(agent_id, agent, command, runtime.update_tx.clone());
                }
            }
        }

        // Check for GitHub updates (non-blocking)
        while let Ok(result) = runtime.github_rx.try_recv() {
            match result {
                FetchResult::Success(id, data) => {
                    app.update(app::Message::GitHubDataReceived(id, data));
                }
                FetchResult::Failure(id, error) => {
                    app.update(app::Message::GitHubFetchFailed(id, error));
                }
            }
        }

        // Drain agent-update progress/results (non-blocking).
        while let Ok(event) = runtime.update_rx.try_recv() {
            let mut finished_status: Option<String> = None;
            if let Some(ref mut agents_app) = app.agents_app {
                match event {
                    UpdateEvent::Output(id, line) => agents_app.push_update_output(&id, line),
                    UpdateEvent::Redetected(id, installed) => {
                        agents_app.apply_redetected(&id, installed)
                    }
                    UpdateEvent::Finished(id, success, message) => {
                        let name = agents_app
                            .entries
                            .iter()
                            .find(|e| e.id == id)
                            .map(|e| e.agent.name.clone())
                            .unwrap_or_else(|| id.clone());
                        agents_app.finish_update(&id, success, message);
                        finished_status = Some(if success {
                            format!("{} updated", name)
                        } else {
                            format!("{} update failed — see detail panel", name)
                        });
                    }
                }
            }
            // Set status after the agents_app borrow ends to avoid aliasing app.
            if let Some(s) = finished_status {
                app.set_status(s);
                last_status_time = Some(std::time::Instant::now());
            }
        }

        // Check for benchmark source updates (non-blocking) — drain all that
        // have landed so multiple sources are absorbed in one tick.
        while let Ok((idx, result)) = runtime.bench_rx.try_recv() {
            app.update(app::Message::DataSourceLoaded(idx, result));
        }

        // Drain `r`-triggered refresh results — the handler sets its own status
        // (Refreshed / Failed to refresh).
        while let Ok((idx, result)) = runtime.refresh_rx.try_recv() {
            app.update(app::Message::DataSourceRefreshed(idx, result));
            last_status_time = Some(std::time::Instant::now());
        }

        // Drain `r`-triggered models.dev refresh results.
        while let Ok(result) = runtime.models_refresh_rx.try_recv() {
            app.update(app::Message::ProvidersRefreshed(result));
            last_status_time = Some(std::time::Instant::now());
        }

        // Drain async benchmark-url opens (Epoch 404-fallback path) — the final
        // opened URL arrives as a `BenchmarkUrlOpened` message reported to the
        // status bar.
        while let Ok(url) = runtime.url_rx.try_recv() {
            let msg = app::Message::BenchmarkUrlOpened(url);
            if let app::Message::BenchmarkUrlOpened(url) = &msg {
                app.set_status(format!("Opened: {url}"));
                last_status_time = Some(std::time::Instant::now());
            }
            app.update(msg);
        }

        if app.pending_status_refresh {
            app.pending_status_refresh = false;
            let force = app.force_status_refresh;
            app.force_status_refresh = false;
            let stale = runtime
                .status
                .last_fetch_time
                .is_none_or(|t| t.elapsed() > Duration::from_secs(60));
            let recent = runtime
                .status
                .last_fetch_time
                .is_some_and(|t| t.elapsed() < Duration::from_secs(2));
            if force || (stale && !recent) {
                if let Some(ref status_app) = app.status_app {
                    runtime.status.fetch_generation += 1;
                    let gen = runtime.status.fetch_generation;
                    runtime.status.last_fetch_time = Some(Instant::now());
                    let seeds = status_app.fetch_seeds();
                    let tx = runtime.status.tx.clone();
                    let fetcher = StatusFetcher::with_client(runtime.status.client.clone());
                    tokio::spawn(async move {
                        let result = fetcher.fetch(&seeds).await;
                        let _ = tx.send((gen, result)).await;
                    });
                }
            } else if let Some(ref mut status_app) = app.status_app {
                status_app.loading = false;
            }
        }

        if let Ok((gen, result)) = runtime.status.rx.try_recv() {
            if gen >= runtime.status.fetch_generation {
                let StatusFetchResult::Fresh(entries) = result;
                app.update(app::Message::StatusDataReceived(entries));
            }
        }

        if let Some(msg) = event::handle_events(app)? {
            // Set when RefreshAgents is seen — spawn must run AFTER app.update()
            // marks tracked entries Loading (so the filter in spawn_agent_fetches
            // matches them). See ordering note below.
            let mut need_refresh_agents = false;

            // Handle clipboard operations and set status with timer
            match &msg {
                app::Message::CopyFull => {
                    if let Some(text) = app.get_copy_full() {
                        copy_to_clipboard(text.clone());
                        app.set_status(format!("Copied: {}", text));
                        last_status_time = Some(std::time::Instant::now());
                    }
                }
                app::Message::CopyModelId => {
                    if let Some(text) = app.get_copy_model_id() {
                        copy_to_clipboard(text.clone());
                        app.set_status(format!("Copied: {}", text));
                        last_status_time = Some(std::time::Instant::now());
                    }
                }
                app::Message::CopyProviderDoc => {
                    if let Some(text) = app.get_provider_doc() {
                        copy_to_clipboard(text.clone());
                        app.set_status(format!("Copied: {}", text));
                        last_status_time = Some(std::time::Instant::now());
                    }
                }
                app::Message::CopyProviderApi => {
                    if let Some(text) = app.get_provider_api() {
                        copy_to_clipboard(text.clone());
                        app.set_status(format!("Copied: {}", text));
                        last_status_time = Some(std::time::Instant::now());
                    }
                }
                app::Message::OpenProviderDoc => {
                    if let Some(url) = app.get_provider_doc() {
                        let _ = open::that_in_background(&url);
                        app.set_status(format!("Opened: {}", url));
                        last_status_time = Some(std::time::Instant::now());
                    }
                }
                app::Message::OpenAgentDocs => {
                    if let Some(ref agents_app) = app.agents_app {
                        if let Some(entry) = agents_app.current_entry() {
                            if let Some(ref url) = entry.agent.docs {
                                let _ = open::that_in_background(url);
                                app.set_status(format!("Opened: {}", url));
                                last_status_time = Some(std::time::Instant::now());
                            } else if let Some(ref url) = entry.agent.homepage {
                                let _ = open::that_in_background(url);
                                app.set_status(format!("Opened: {}", url));
                                last_status_time = Some(std::time::Instant::now());
                            }
                        }
                    }
                }
                app::Message::OpenAgentRepo => {
                    if let Some(ref agents_app) = app.agents_app {
                        if let Some(entry) = agents_app.current_entry() {
                            let url = format!("https://github.com/{}", entry.agent.repo);
                            let _ = open::that_in_background(&url);
                            app.set_status(format!("Opened: {}", url));
                            last_status_time = Some(std::time::Instant::now());
                        }
                    }
                }
                app::Message::CopyAgentName => {
                    if let Some(ref agents_app) = app.agents_app {
                        if let Some(entry) = agents_app.current_entry() {
                            copy_to_clipboard(entry.agent.name.clone());
                            app.set_status(format!("Copied: {}", entry.agent.name));
                            last_status_time = Some(std::time::Instant::now());
                        }
                    }
                }
                app::Message::CopyBenchmarkName => {
                    if let Some(file) = app.active_benchmark_file() {
                        if let Some(model) = app.benchmarks_app.current_model(file) {
                            copy_to_clipboard(model.display_name.clone());
                            app.set_status(format!("Copied: {}", model.display_name));
                            last_status_time = Some(std::time::Instant::now());
                        }
                    }
                }
                app::Message::OpenBenchmarkUrl => {
                    // Per-source URL strategy (sources.rs `model_url`). Epoch's
                    // per-model pages only resolve for ~70% of ids, so it gets a
                    // 200-probe with a fallback to the model index page; the
                    // other sources open synchronously.
                    let active = app.benchmarks_app.active_source;
                    let descriptor = crate::tui::benchmarks::BenchmarksApp::active_descriptor(
                        &app.multi_store,
                        active,
                    );
                    let model_id = app
                        .active_benchmark_file()
                        .and_then(|file| app.benchmarks_app.current_model(file))
                        .map(|model| model.id.clone());
                    if let (Some(descriptor), Some(model_id)) = (descriptor, model_id) {
                        let url = descriptor.model_url(&model_id);
                        if descriptor.id == "epoch" {
                            // Probe the model page; open it on 200, else the
                            // model index. Final URL reported via url_tx.
                            let url_tx = runtime.url_tx.clone();
                            tokio::spawn(async move {
                                const FALLBACK: &str = "https://epoch.ai/data/ai-models";
                                let client = reqwest::Client::builder()
                                    .user_agent("models-tui")
                                    .timeout(Duration::from_secs(3))
                                    .build();
                                let resolved = match client {
                                    Ok(client) => match client.head(&url).send().await {
                                        Ok(resp) if resp.status().is_success() => url,
                                        _ => FALLBACK.to_string(),
                                    },
                                    Err(_) => FALLBACK.to_string(),
                                };
                                let _ = open::that_in_background(&resolved);
                                let _ = url_tx.send(resolved).await;
                            });
                            app.set_status("Opening Epoch model page…".to_string());
                        } else {
                            let _ = open::that_in_background(&url);
                            app.set_status(format!("Opened: {}", url));
                        }
                        last_status_time = Some(std::time::Instant::now());
                    }
                }
                app::Message::OpenStatusPage => {
                    if let Some(entry) = app.status_app.as_ref().and_then(|a| a.current_entry()) {
                        if let Some(url) = entry.best_open_url() {
                            let _ = open::that_in_background(url);
                            app.set_status(format!("Opened: {}", url));
                            last_status_time = Some(std::time::Instant::now());
                        }
                    }
                }
                app::Message::RefreshStatus => {
                    app.set_status("Refreshing provider status…".to_string());
                    last_status_time = Some(std::time::Instant::now());
                }
                app::Message::PickerSave => {
                    // Picker save sets its own status message via app.update
                    last_status_time = Some(std::time::Instant::now());
                }
                app::Message::AddAgentSave => {
                    // Add-agent save sets its own status message via app.update;
                    // the new agent's GitHub fetch is dispatched from the
                    // pending_fetches drain at the top of the loop.
                    last_status_time = Some(std::time::Instant::now());
                }
                app::Message::RequestUpdateAgent
                | app::Message::RequestUpdateAll
                | app::Message::ConfirmUpdate
                | app::Message::CancelUpdate => {
                    // These set their own transient status via app.update (errors
                    // like "No updater for X", or "Updating N…"); arm the auto-clear
                    // so the message doesn't linger. Update completion sets a fresh
                    // status from the update_rx drain above.
                    last_status_time = Some(std::time::Instant::now());
                }
                app::Message::ColumnPickerSave => {
                    // Column persistence sets its own status via app.update
                    last_status_time = Some(std::time::Instant::now());
                }
                app::Message::ToggleBenchmarkSelection => {
                    // Look up the model name for the status message
                    if let Some(&store_idx) = app
                        .benchmarks_app
                        .filtered_indices
                        .get(app.benchmarks_app.selected)
                    {
                        let name = app
                            .active_benchmark_file()
                            .and_then(|f| f.models.get(store_idx))
                            .map(|m| m.display_name.as_str())
                            .unwrap_or("?");
                        let is_already_selected = app.selections.contains(&store_idx);
                        if is_already_selected {
                            let count = app.selections.len() - 1;
                            app.set_status(format!(
                                "Removed {} ({}/{})",
                                name,
                                count,
                                app::MAX_SELECTIONS
                            ));
                        } else if app.selections.len() < app::MAX_SELECTIONS {
                            let count = app.selections.len() + 1;
                            app.set_status(format!(
                                "Added {} ({}/{})",
                                name,
                                count,
                                app::MAX_SELECTIONS
                            ));
                        }
                        last_status_time = Some(std::time::Instant::now());
                    }
                }
                app::Message::ClearBenchmarkSelections => {
                    let count = app.selections.len();
                    if count > 0 {
                        app.set_status(format!(
                            "Cleared {} selection{}",
                            count,
                            if count == 1 { "" } else { "s" }
                        ));
                        last_status_time = Some(std::time::Instant::now());
                    }
                }
                app::Message::CycleBenchmarkView => {
                    // Show status after the update processes the cycle
                    // We need to peek at what the NEXT view will be
                    let next_view = match app.benchmarks_app.bottom_view {
                        crate::tui::benchmarks::BottomView::H2H => "Scatter",
                        crate::tui::benchmarks::BottomView::Scatter => "Radar",
                        crate::tui::benchmarks::BottomView::Radar => "H2H",
                        crate::tui::benchmarks::BottomView::Detail => "H2H",
                    };
                    app.set_status(format!("View: {}", next_view));
                    last_status_time = Some(std::time::Instant::now());
                }
                app::Message::CycleScatterX => {
                    if let Some(label) = peek_next_metric_label(app, app.benchmarks_app.scatter_x) {
                        app.set_status(format!("X-axis: {}", label));
                        last_status_time = Some(std::time::Instant::now());
                    }
                }
                app::Message::CycleScatterY => {
                    if let Some(label) = peek_next_metric_label(app, app.benchmarks_app.scatter_y) {
                        app.set_status(format!("Y-axis: {}", label));
                        last_status_time = Some(std::time::Instant::now());
                    }
                }
                app::Message::CycleRadarPreset => {
                    if let Some(label) = peek_next_radar_group_label(app) {
                        app.set_status(format!("Radar: {}", label));
                        last_status_time = Some(std::time::Instant::now());
                    }
                }
                app::Message::RefreshBenchmarkSource => {
                    // Re-fetch the active source without flipping it to Loading
                    // (stale-while-revalidate): the current data keeps rendering
                    // while the fetch runs. The result routes to
                    // `DataSourceRefreshed` so a failure keeps the old file.
                    let idx = app.benchmarks_app.active_source;
                    if let Some(descriptor) = SOURCES.get(idx) {
                        let name = descriptor.name;
                        let refresh_tx = runtime.refresh_tx.clone();
                        tokio::spawn(async move {
                            let result = fetch_source(descriptor).await;
                            let _ = refresh_tx.send((idx, result)).await;
                        });
                        app.set_status(format!("Refreshing {name}…"));
                        last_status_time = Some(std::time::Instant::now());
                    }
                }
                app::Message::RefreshModels => {
                    // Spawn an async models.dev refetch. Runs the blocking
                    // reqwest call in a `spawn_blocking` wrapper so the event
                    // loop is never blocked. Result arrives via `ProvidersRefreshed`.
                    let models_refresh_tx = runtime.models_refresh_tx.clone();
                    tokio::spawn(async move {
                        let result =
                            tokio::task::spawn_blocking(|| crate::api::fetch_providers().ok())
                                .await
                                .unwrap_or(None);
                        let _ = models_refresh_tx.send(result).await;
                    });
                    app.set_status("Refreshing models.dev…".to_string());
                    last_status_time = Some(std::time::Instant::now());
                }
                app::Message::RefreshAgents => {
                    // app.update() (below) must run first — it marks tracked entries
                    // Loading so spawn_agent_fetches' filter can match them. Spawning
                    // here, before update(), would race against Loaded entries and
                    // dispatch zero fetches. Set the flag; spawn after update().
                    need_refresh_agents = true;
                    last_status_time = Some(std::time::Instant::now());
                }
                _ => {}
            }

            if !app.update(msg) {
                return Ok(());
            }

            // Post-update: spawn fetches now that app.update(RefreshAgents) has
            // flipped tracked entries to Loading. spawn_agent_fetches filters on
            // Loading | NotStarted, so this ordering is required for the refresh
            // to actually dispatch network requests after the initial load.
            if need_refresh_agents {
                if let Some(ref agents_app) = app.agents_app {
                    spawn_agent_fetches(
                        &agents_app.entries,
                        runtime.github_tx.clone(),
                        runtime.client.clone(),
                        runtime.disk_cache.clone(),
                    );
                }
            }

            // Interactive update (suspend-and-run): a confirmed single-agent
            // update that hands the terminal to the updater so the user can
            // answer prompts. Runs synchronously on this (the terminal-owning)
            // thread; the TUI is restored before the next draw.
            if let Some((id, command)) = app.pending_interactive_update.take() {
                let agent = app
                    .agents_app
                    .as_ref()
                    .and_then(|a| a.entries.iter().find(|e| e.id == id))
                    .map(|e| e.agent.clone());
                let (success, message) = run_interactive_update(terminal, &command);
                if let Some(ref mut agents_app) = app.agents_app {
                    if success {
                        if let Some(agent) = agent {
                            agents_app
                                .apply_redetected(&id, crate::agents::detect_installed(&agent));
                        }
                    }
                    let name = agents_app
                        .entries
                        .iter()
                        .find(|e| e.id == id)
                        .map(|e| e.agent.name.clone())
                        .unwrap_or_else(|| id.clone());
                    agents_app.finish_update(&id, success, message);
                    app.set_status(if success {
                        format!("{} updated", name)
                    } else {
                        format!("{} update failed — see detail panel", name)
                    });
                    last_status_time = Some(std::time::Instant::now());
                }
            }
        }
    }
}

/// Suspend the TUI, run an updater attached to the real terminal so the user can
/// answer prompts interactively, then restore the TUI. Single-agent only.
///
/// Restore is **unconditional** — nothing between teardown and restore uses `?`,
/// so a child error can't leave the terminal wedged (and `run()`'s end-cleanup +
/// the idempotent panic hook are additional safety nets). On re-entry we mirror
/// `run()`'s setup order and `clear()` so ratatui repaints against a fresh buffer.
fn run_interactive_update(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    command: &[String],
) -> (bool, String) {
    use std::io::Write;

    if command.is_empty() {
        return (false, "empty update command".to_string());
    }

    // Tear down the TUI so the child fully owns the terminal (cooked mode, normal
    // screen, mouse capture off).
    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    );

    println!("\nRunning: {}\n", command.join(" "));
    let _ = io::stdout().flush();

    // Inherited stdio → fully interactive (prompts, sudo, selection menus).
    let status = std::process::Command::new(&command[0])
        .args(&command[1..])
        .status();
    let (success, message) = match status {
        Ok(s) if s.success() => (true, "✓ update completed (ran interactively)".to_string()),
        Ok(s) => (false, format!("✗ updater exited with {s}")),
        Err(e) => (false, format!("✗ failed to start `{}`: {e}", command[0])),
    };

    // Keep the output visible until the user is ready to return.
    print!("\n{message}\nPress Enter to return to models… ");
    let _ = io::stdout().flush();
    let mut buf = String::new();
    let _ = io::stdin().read_line(&mut buf);

    // Restore the TUI (unconditional).
    let _ = enable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        EnterAlternateScreen,
        EnableMouseCapture
    );
    let _ = terminal.clear();

    (success, message)
}

#[cfg(test)]
mod update_exec_tests {
    use super::*;
    use crate::agents::Agent;

    fn dummy_agent() -> Agent {
        Agent {
            name: "X".to_string(),
            repo: "o/x".to_string(),
            categories: vec![],
            installation_method: None,
            pricing: None,
            supported_providers: vec![],
            platform_support: vec![],
            open_source: false,
            cli_binary: None,
            alt_binaries: vec![],
            version_command: vec![],
            update_command: vec![],
            version_regex: None,
            config_files: vec![],
            homepage: None,
            docs: None,
        }
    }

    /// A missing updater binary must degrade gracefully to a failed result with a
    /// helpful message — never panic or hang. Portable (no real binary involved).
    #[tokio::test]
    async fn nonexistent_binary_reports_failure() {
        let (tx, mut rx) = mpsc::channel(16);
        spawn_agent_update(
            "x".to_string(),
            dummy_agent(),
            vec!["definitely-not-a-real-binary-zzz".to_string()],
            tx,
        );
        let mut finished = None;
        while let Some(ev) = rx.recv().await {
            if let UpdateEvent::Finished(id, ok, msg) = ev {
                finished = Some((id, ok, msg));
                break;
            }
        }
        let (id, ok, msg) = finished.expect("a Finished event");
        assert_eq!(id, "x");
        assert!(!ok);
        assert!(msg.contains("failed to start"), "got: {msg}");
    }

    /// A real, quick command streams its output and reports success, with all
    /// output flushed before the terminal event.
    #[cfg(unix)]
    #[tokio::test]
    async fn echo_streams_output_then_succeeds() {
        let (tx, mut rx) = mpsc::channel(16);
        spawn_agent_update(
            "x".to_string(),
            dummy_agent(),
            vec!["echo".to_string(), "hello-update".to_string()],
            tx,
        );
        let mut outputs = Vec::new();
        let mut success = None;
        while let Some(ev) = rx.recv().await {
            match ev {
                UpdateEvent::Output(_, line) => outputs.push(line),
                UpdateEvent::Finished(_, ok, _) => {
                    success = Some(ok);
                    break;
                }
                UpdateEvent::Redetected(..) => {}
            }
        }
        assert_eq!(success, Some(true));
        assert!(
            outputs.iter().any(|l| l.contains("hello-update")),
            "expected streamed output, got: {outputs:?}"
        );
    }
}
