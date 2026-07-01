use anyhow::Result;
use clap::{CommandFactory, Parser};
use std::{collections::HashSet, sync::Arc};
use tokio::sync::RwLock;

#[derive(Parser, Debug)]
#[command(name = "agents")]
#[command(about = "Track AI coding agent releases and changelogs")]
#[command(version)]
#[command(after_help = "\
\x1b[1;4mTool Commands:\x1b[0m
  agents <tool>                 Browse releases for a tool
  agents <tool> --latest        Show latest changelog directly
  agents <tool> --list, -l      List all versions
  agents <tool> --version <v>   Show changelog for a specific version
  agents <tool> --web, -w       Open releases page in browser

\x1b[1;4mExamples:\x1b[0m
  agents claude                 Browse Claude Code releases
  agents claude --latest        Latest Claude Code changelog
  agents cursor --list          All Cursor versions
  agents cursor --version 1.0.0 Show a specific Cursor version")]
pub struct AgentsCli {
    #[command(subcommand)]
    pub command: Option<AgentsCommand>,
}

#[derive(clap::Subcommand, Debug)]
pub enum AgentsCommand {
    /// Show status table for all tracked agents
    Status,
    /// Show releases from the last 24 hours
    Latest,
    /// List available agent sources
    ListSources,
    /// View changelog for a specific agent tool
    #[command(external_subcommand)]
    Tool(Vec<String>),
}

/// Parse tool-specific flags from the external subcommand args
#[derive(Debug)]
pub struct ToolArgs {
    pub tool: String,
    pub latest: bool,
    pub list: bool,
    pub version: Option<String>,
    pub web: bool,
}

impl ToolArgs {
    pub fn parse_from(args: Vec<String>) -> Result<Self> {
        if args.is_empty() {
            anyhow::bail!("No tool specified");
        }
        let tool = args[0].clone();
        let mut latest = false;
        let mut list = false;
        let mut version = None;
        let mut web = false;

        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--latest" => latest = true,
                "--list" | "-l" => list = true,
                "--web" | "-w" => web = true,
                "--version" => {
                    i += 1;
                    version = Some(
                        args.get(i)
                            .cloned()
                            .ok_or_else(|| anyhow::anyhow!("--version requires a value"))?,
                    );
                }
                other => anyhow::bail!("Unknown flag: {}", other),
            }
            i += 1;
        }

        // Mutual exclusivity
        let mode_count = [latest, list, version.is_some()]
            .iter()
            .filter(|&&v| v)
            .count();
        if mode_count > 1 {
            anyhow::bail!("--latest, --list, and --version are mutually exclusive");
        }

        Ok(Self {
            tool,
            latest,
            list,
            version,
            web,
        })
    }
}

#[derive(Clone)]
struct CatalogAgent {
    id: String,
    agent: crate::agents::data::Agent,
    tracked: bool,
}

enum ResolveTool {
    Single(Box<CatalogAgent>),
    Ambiguous(Vec<CatalogAgent>),
}

pub fn run() -> Result<()> {
    let cli = AgentsCli::parse();
    dispatch(cli.command)
}

pub fn run_with_command(command: Option<AgentsCommand>) -> Result<()> {
    dispatch(command)
}

fn dispatch(command: Option<AgentsCommand>) -> Result<()> {
    match command {
        Some(AgentsCommand::Status) => run_status(),
        Some(AgentsCommand::Latest) => run_latest(),
        Some(AgentsCommand::ListSources) => run_list_sources(),
        Some(AgentsCommand::Tool(args)) => {
            let tool_args = ToolArgs::parse_from(args)?;
            run_tool(tool_args)
        }
        None => {
            AgentsCli::command().print_long_help()?;
            println!();
            Ok(())
        }
    }
}

fn cached_github_data_for_repo(
    disk_cache: &crate::agents::cache::GitHubCache,
    repo: &str,
) -> Option<crate::agents::data::GitHubData> {
    disk_cache.get(repo).map(|c| c.data.to_github_data())
}

fn load_catalog(config: &crate::config::Config) -> Result<Vec<CatalogAgent>> {
    let agents_file = crate::agents::loader::load_agents()?;
    let mut entries: Vec<_> = agents_file
        .agents
        .iter()
        .map(|(id, agent)| CatalogAgent {
            id: id.clone(),
            agent: agent.clone(),
            tracked: config.is_tracked(id),
        })
        .collect();

    // Merge custom agents from config (same logic as TUI's AgentsApp::new)
    for custom in &config.agents.custom {
        let id = custom.name.to_lowercase().replace(' ', "-");
        if entries.iter().any(|e| e.id == id) {
            continue;
        }
        entries.push(CatalogAgent {
            id: id.clone(),
            agent: custom.to_agent(),
            tracked: config.is_tracked(&id),
        });
    }

    entries.sort_by(|a, b| a.agent.name.cmp(&b.agent.name));
    Ok(entries)
}

fn source_items(
    catalog: &[CatalogAgent],
    disk_cache: &crate::agents::cache::GitHubCache,
) -> Vec<crate::cli::agents_ui::AgentSourceItem> {
    catalog
        .iter()
        .map(|entry| {
            let github = cached_github_data_for_repo(disk_cache, &entry.agent.repo);
            let (stars, latest_version, latest_release_date, release_frequency) =
                if let Some(ref gh) = github {
                    let version = gh.latest_version().unwrap_or("\u{2014}").to_string();
                    let date = gh
                        .latest_release_date()
                        .map(|dt| {
                            let formatted = dt.format("%Y-%m-%d").to_string();
                            let relative = crate::agents::helpers::format_relative_time(&dt);
                            format!("{formatted} ({relative})")
                        })
                        .unwrap_or_else(|| "\u{2014}".to_string());
                    let freq = gh.release_frequency();
                    (gh.stars, version, date, freq)
                } else {
                    (
                        None,
                        "\u{2014}".to_string(),
                        "\u{2014}".to_string(),
                        "\u{2014}".to_string(),
                    )
                };

            crate::cli::agents_ui::AgentSourceItem {
                id: entry.id.clone(),
                name: entry.agent.name.clone(),
                repo: entry.agent.repo.clone(),
                cli_binary: entry
                    .agent
                    .cli_binary
                    .clone()
                    .unwrap_or_else(|| "\u{2014}".to_string()),
                categories: if entry.agent.categories.is_empty() {
                    "\u{2014}".to_string()
                } else {
                    entry.agent.categories.join(", ")
                },
                tracked: entry.tracked,
                open_source: entry.agent.open_source,
                supported_providers: if entry.agent.supported_providers.is_empty() {
                    "\u{2014}".to_string()
                } else {
                    entry.agent.supported_providers.join(", ")
                },
                platform_support: if entry.agent.platform_support.is_empty() {
                    "\u{2014}".to_string()
                } else {
                    entry.agent.platform_support.join(", ")
                },
                pricing: entry
                    .agent
                    .pricing
                    .as_ref()
                    .map(|p| {
                        let mut parts = vec![p.model.clone()];
                        if let Some(price) = p.subscription_price {
                            let period = p.subscription_period.as_deref().unwrap_or("month");
                            parts.push(format!("${price}/{period}"));
                        }
                        if p.free_tier {
                            parts.push("free tier".to_string());
                        }
                        parts.join(", ")
                    })
                    .unwrap_or_else(|| "\u{2014}".to_string()),
                homepage: entry
                    .agent
                    .homepage
                    .clone()
                    .unwrap_or_else(|| "\u{2014}".to_string()),
                docs: entry
                    .agent
                    .docs
                    .clone()
                    .unwrap_or_else(|| "\u{2014}".to_string()),
                stars,
                latest_version,
                latest_release_date,
                release_frequency,
            }
        })
        .collect()
}

fn browser_items_for_agent(
    _id: &str,
    agent: &crate::agents::data::Agent,
    github: &crate::agents::data::GitHubData,
) -> Vec<crate::cli::agents_ui::ReleaseBrowserItem> {
    github
        .releases
        .iter()
        .map(|release| {
            let date = format_release_date_ymd(release.date.as_deref())
                .unwrap_or_else(|| "\u{2014}".to_string());
            let ago = release
                .date
                .as_deref()
                .and_then(crate::agents::helpers::parse_date)
                .map(|d| crate::agents::helpers::format_relative_time(&d))
                .unwrap_or_else(|| "\u{2014}".to_string());
            crate::cli::agents_ui::ReleaseBrowserItem {
                agent_name: agent.name.clone(),
                version: release.version.clone(),
                released: date,
                ago,
                body: release.changelog.clone(),
                sort_key: release
                    .date
                    .as_deref()
                    .and_then(crate::agents::helpers::parse_date)
                    .map(|dt| dt.timestamp())
                    .unwrap_or(0),
                release: release.clone(),
            }
        })
        .collect()
}

fn apply_github_fetch_result(
    result: crate::agents::github::ConditionalFetchResult,
    repo: &str,
    disk_cache: &mut crate::agents::cache::GitHubCache,
    fresh_shared_cache: Option<crate::agents::cache::GitHubCache>,
) -> (&'static str, Option<crate::agents::data::GitHubData>) {
    match result {
        crate::agents::github::ConditionalFetchResult::Fresh(data, _etag) => {
            if let Some(shared_cache) = fresh_shared_cache {
                *disk_cache = shared_cache;
            }
            (" done.", Some(data))
        }
        crate::agents::github::ConditionalFetchResult::NotModified => (
            " up to date.",
            cached_github_data_for_repo(disk_cache, repo),
        ),
        crate::agents::github::ConditionalFetchResult::Error(_) => {
            (" failed.", cached_github_data_for_repo(disk_cache, repo))
        }
    }
}

fn format_release_date_ymd(date: Option<&str>) -> Option<String> {
    date.and_then(crate::agents::helpers::parse_date)
        .map(|d| d.format("%Y-%m-%d").to_string())
}

/// Fetch GitHub data for a single agent (used by `run_tool` for one-off fetches).
fn get_github_data(
    agent_name: &str,
    repo: &str,
    disk_cache: &mut crate::agents::cache::GitHubCache,
    runtime: &tokio::runtime::Runtime,
) -> Option<crate::agents::data::GitHubData> {
    eprint!("Fetching data for {}...", agent_name);
    let cache_arc = Arc::new(RwLock::new(disk_cache.clone()));
    let token = crate::agents::github::detect_github_token();
    let client =
        crate::agents::github::AsyncGitHubClient::with_disk_cache(token, cache_arc.clone());

    let result = runtime.block_on(client.fetch_conditional(repo));
    let fresh_shared_cache = if matches!(
        &result,
        crate::agents::github::ConditionalFetchResult::Fresh(_, _)
    ) {
        Some(runtime.block_on(cache_arc.read()).clone())
    } else {
        None
    };
    let (status, data) = apply_github_fetch_result(result, repo, disk_cache, fresh_shared_cache);
    eprintln!("{status}");
    data
}

/// Fetch GitHub releases and detect installed versions for multiple agents concurrently.
/// Uses releases-only endpoint (1 API call per agent, no repo metadata).
/// Returns a Vec of (agent_id, Option<GitHubData>, InstalledInfo).
fn get_github_data_batch(
    agents: &[(String, crate::agents::data::Agent)],
    disk_cache: &mut crate::agents::cache::GitHubCache,
    runtime: &tokio::runtime::Runtime,
) -> Vec<(
    String,
    Option<crate::agents::data::GitHubData>,
    crate::agents::data::InstalledInfo,
)> {
    let cache_arc = Arc::new(RwLock::new(disk_cache.clone()));
    let token = crate::agents::github::detect_github_token();

    eprint!("Fetching {} agents...", agents.len());

    let results: Vec<_> = runtime.block_on(async {
        let mut handles = Vec::new();
        for (id, agent) in agents {
            let client = crate::agents::github::AsyncGitHubClient::with_disk_cache(
                token.clone(),
                cache_arc.clone(),
            );
            let repo = agent.repo.clone();
            let id = id.clone();
            let agent = agent.clone();

            handles.push(tokio::spawn(async move {
                let (fetch_result, installed) = tokio::join!(
                    client.fetch_releases_only(&repo),
                    tokio::task::spawn_blocking(move || {
                        crate::agents::detect::detect_installed_cli(&agent)
                    })
                );
                let installed = installed.unwrap_or_default();
                (id, repo, fetch_result, installed)
            }));
        }
        let mut results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(r) => results.push(r),
                Err(e) => results.push((
                    String::new(),
                    String::new(),
                    crate::agents::github::ConditionalFetchResult::Error(e.to_string()),
                    crate::agents::data::InstalledInfo::default(),
                )),
            }
        }
        results
    });

    // Sync the shared cache back once
    *disk_cache = runtime.block_on(cache_arc.read()).clone();

    let output: Vec<_> = results
        .into_iter()
        .map(|(id, repo, result, installed)| {
            let data = match result {
                crate::agents::github::ConditionalFetchResult::Fresh(data, _etag) => Some(data),
                crate::agents::github::ConditionalFetchResult::NotModified => {
                    cached_github_data_for_repo(disk_cache, &repo)
                }
                crate::agents::github::ConditionalFetchResult::Error(_) => {
                    cached_github_data_for_repo(disk_cache, &repo)
                }
            };
            (id, data, installed)
        })
        .collect();

    eprintln!(" done.");
    output
}

fn run_status() -> Result<()> {
    use super::styles;

    let config = crate::config::Config::load()?;
    let mut disk_cache = crate::agents::cache::GitHubCache::load();

    let entries: Vec<_> = load_catalog(&config)?
        .into_iter()
        .filter(|entry| entry.tracked)
        .collect();

    // Fetch all agents and detect installed versions concurrently
    let batch_input: Vec<_> = entries
        .iter()
        .map(|entry| (entry.id.clone(), entry.agent.clone()))
        .collect();

    let runtime = tokio::runtime::Runtime::new()?;
    let batch_results = get_github_data_batch(&batch_input, &mut disk_cache, &runtime);

    // Fetch service health for mapped agents
    let status_entries: Vec<crate::status::ProviderStatus> = {
        use crate::agents::health::AGENT_SERVICE_MAPPINGS;
        use crate::status::registry::status_seed_for_provider;
        let tracked_ids: std::collections::HashSet<&str> =
            entries.iter().map(|e| e.id.as_str()).collect();
        let slugs: std::collections::HashSet<&str> = AGENT_SERVICE_MAPPINGS
            .iter()
            .filter(|m| tracked_ids.contains(m.agent_id))
            .map(|m| m.provider_slug)
            .collect();
        let seeds: Vec<_> = slugs.iter().map(|s| status_seed_for_provider(s)).collect();
        let client = reqwest::Client::builder()
            .user_agent("models-cli")
            .connect_timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("Failed to build HTTP client");
        let fetcher = crate::status::StatusFetcher::with_client(client);
        let crate::status::StatusFetchResult::Fresh(entries) =
            runtime.block_on(fetcher.fetch(&seeds));
        entries
    };

    let mut table = comfy_table::Table::new();
    table.load_preset(comfy_table::presets::UTF8_FULL);
    table.set_header(vec![
        styles::header_cell("Tool"),
        styles::header_cell("24h"),
        styles::header_cell("Installed"),
        styles::header_cell("Latest"),
        styles::header_cell("Updated"),
        styles::header_cell("Freq."),
        styles::header_cell("Status"),
    ]);

    // Sort by most recently updated (newest first), with missing dates last
    let mut rows: Vec<_> = entries.iter().zip(batch_results.iter()).collect();
    rows.sort_by(|(_, (_, g_a, _)), (_, (_, g_b, _))| {
        let date_a = g_a
            .as_ref()
            .and_then(|g| g.latest_release())
            .and_then(|r| r.date.as_deref());
        let date_b = g_b
            .as_ref()
            .and_then(|g| g.latest_release())
            .and_then(|r| r.date.as_deref());
        match (date_b, date_a) {
            (Some(b), Some(a)) => b.cmp(a),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    });

    for (entry, (_, github, installed)) in rows {
        let latest_version = github
            .as_ref()
            .and_then(|g| g.latest_version())
            .unwrap_or("\u{2014}");

        let latest_date = github
            .as_ref()
            .and_then(|g| g.latest_release())
            .and_then(|r| r.date.as_deref())
            .and_then(crate::agents::helpers::parse_date);

        let is_24h = latest_date
            .map(|d| crate::agents::helpers::is_within_24h(&d))
            .unwrap_or(false);
        let updated = latest_date
            .map(|d| crate::agents::helpers::format_relative_time(&d))
            .unwrap_or_else(|| "\u{2014}".to_string());

        let release_dates: Vec<_> = github
            .as_ref()
            .map(|g| {
                g.releases
                    .iter()
                    .filter_map(|r| r.date.as_deref())
                    .filter_map(crate::agents::helpers::parse_date)
                    .collect()
            })
            .unwrap_or_default();

        let freq = crate::agents::helpers::calculate_release_frequency(&release_dates);

        let installed_str = installed
            .version
            .clone()
            .unwrap_or_else(|| "\u{2014}".to_string());

        let installed_cell = if installed_str == "\u{2014}" {
            styles::dim_cell(&installed_str)
        } else if installed_str == latest_version {
            styles::green_cell(&installed_str)
        } else {
            styles::yellow_cell(&installed_str)
        };

        let service_cell = {
            use crate::agents::health::resolve_agent_service_health;
            use crate::status::ProviderHealth;
            match resolve_agent_service_health(&entry.id, &status_entries) {
                Some(resolved) => {
                    let (icon, label) = match resolved.health {
                        ProviderHealth::Operational => ("\u{25CF}", "Ok"),
                        ProviderHealth::Degraded => ("\u{25D0}", "Degraded"),
                        ProviderHealth::Outage => ("\u{2717}", "Outage"),
                        ProviderHealth::Maintenance => ("\u{25C6}", "Maint."),
                        ProviderHealth::Unknown => ("?", "Unknown"),
                    };
                    let text = format!("{} {}", icon, label);
                    match resolved.health {
                        ProviderHealth::Operational => styles::green_cell(&text),
                        ProviderHealth::Degraded => styles::yellow_cell(&text),
                        ProviderHealth::Outage => {
                            comfy_table::Cell::new(&text).fg(comfy_table::Color::Red)
                        }
                        ProviderHealth::Maintenance => {
                            comfy_table::Cell::new(&text).fg(comfy_table::Color::Blue)
                        }
                        ProviderHealth::Unknown => styles::dim_cell(&text),
                    }
                }
                None => styles::dim_cell("\u{2014}"),
            }
        };

        table.add_row(vec![
            styles::bold_cell(&entry.agent.name),
            if is_24h {
                styles::green_cell("\u{2713}")
            } else {
                comfy_table::Cell::new("")
            },
            installed_cell,
            styles::bold_cell(latest_version),
            comfy_table::Cell::new(&updated),
            comfy_table::Cell::new(&freq),
            service_cell,
        ]);
    }

    disk_cache.save().ok();
    println!("{table}");
    Ok(())
}

fn run_latest() -> Result<()> {
    use super::styles;

    let config = crate::config::Config::load()?;
    let mut disk_cache = crate::agents::cache::GitHubCache::load();

    let tracked: Vec<_> = load_catalog(&config)?
        .into_iter()
        .filter(|entry| entry.tracked)
        .collect();

    let batch_input: Vec<_> = tracked
        .iter()
        .map(|entry| (entry.id.clone(), entry.agent.clone()))
        .collect();

    let runtime = tokio::runtime::Runtime::new()?;
    let batch_results = get_github_data_batch(&batch_input, &mut disk_cache, &runtime);

    let mut recent: Vec<crate::cli::agents_ui::ReleaseBrowserItem> = Vec::new();

    for (entry, (_, github, _installed)) in tracked.iter().zip(batch_results.iter()) {
        if let Some(github) = github {
            for release in &github.releases {
                if let Some(date) = release
                    .date
                    .as_deref()
                    .and_then(crate::agents::helpers::parse_date)
                {
                    if crate::agents::helpers::is_within_24h(&date) {
                        recent.push(crate::cli::agents_ui::ReleaseBrowserItem {
                            agent_name: entry.agent.name.clone(),
                            version: release.version.clone(),
                            released: format_release_date_ymd(release.date.as_deref())
                                .unwrap_or_else(|| "\u{2014}".to_string()),
                            ago: crate::agents::helpers::format_relative_time(&date),
                            body: release.changelog.clone(),
                            sort_key: date.timestamp(),
                            release: release.clone(),
                        });
                    }
                }
            }
        }
    }

    disk_cache.save().ok();

    if recent.is_empty() {
        println!("No releases in the last 24 hours.");
        return Ok(());
    }

    sort_recent_release_items(&mut recent);

    if super::styles::is_tty() {
        if let Some(selected) =
            crate::cli::agents_ui::browse_releases(recent, " Recent Agent Releases ", true)?
        {
            println!();
            print_release(&selected.agent_name, &selected.release);
        }
        return Ok(());
    }

    for item in &recent {
        println!(
            "\n{} {} ({})",
            styles::agent_name(&item.agent_name),
            styles::key_value(&item.version),
            styles::dim(&item.ago)
        );
        println!("{}", styles::separator(40));
        if has_changelog_body(item.body.as_deref()) {
            let body = item.body.as_deref().unwrap_or_default();
            print_changelog_body(body);
        } else {
            println!("(no changelog)");
        }
    }

    Ok(())
}

fn run_list_sources() -> Result<()> {
    use super::styles;

    let mut config = crate::config::Config::load()?;
    let catalog = load_catalog(&config)?;
    let disk_cache = crate::agents::cache::GitHubCache::load();

    if super::styles::is_tty() {
        let items = source_items(&catalog, &disk_cache);
        if let Some(updated) =
            crate::cli::agents_ui::manage_agent_sources(items, " Agent Sources ")?
        {
            let tracked_ids: HashSet<_> = updated
                .iter()
                .filter(|item| item.tracked)
                .map(|item| item.id.clone())
                .collect();
            let all_ids: Vec<_> = updated.iter().map(|item| item.id.clone()).collect();
            for id in all_ids {
                config.set_tracked(&id, tracked_ids.contains(&id));
            }
            config.save()?;
            println!("Saved tracked agents.");
        }
        return Ok(());
    }

    let mut table = comfy_table::Table::new();
    table.load_preset(comfy_table::presets::UTF8_FULL);
    table.set_header(vec![
        styles::header_cell("ID"),
        styles::header_cell("Name"),
        styles::header_cell("Repo"),
        styles::header_cell("CLI Binary"),
        styles::header_cell("Tracked"),
    ]);

    for entry in catalog {
        let tracked = if entry.tracked {
            styles::green_cell("\u{2713}")
        } else {
            comfy_table::Cell::new("")
        };
        let cli = entry.agent.cli_binary.as_deref().unwrap_or("\u{2014}");
        table.add_row(vec![
            comfy_table::Cell::new(entry.id.as_str()),
            styles::bold_cell(&entry.agent.name),
            styles::dim_cell(&entry.agent.repo),
            comfy_table::Cell::new(cli),
            tracked,
        ]);
    }

    println!("{table}");
    Ok(())
}

fn run_tool(args: ToolArgs) -> Result<()> {
    use super::styles;

    let config = crate::config::Config::load()?;
    let catalog = load_catalog(&config)?;
    let mut disk_cache = crate::agents::cache::GitHubCache::load();

    let entry = match resolve_tool(&args.tool, &catalog)? {
        ResolveTool::Single(entry) => *entry,
        ResolveTool::Ambiguous(matches) => {
            if super::styles::is_tty() {
                let title = format!(" Select Agent for \"{}\" ", args.tool);
                let items = source_items(&matches, &disk_cache);
                let Some(selected) = crate::cli::agents_ui::pick_agent(items, &title)? else {
                    return Ok(());
                };
                matches
                    .into_iter()
                    .find(|entry| entry.id == selected.id)
                    .ok_or_else(|| anyhow::anyhow!("Selected agent disappeared"))?
            } else {
                let names: Vec<_> = matches
                    .iter()
                    .map(|entry| styles::code_ref(&entry.id))
                    .collect();
                anyhow::bail!(
                    "{} Ambiguous tool {}. Matches: {}",
                    styles::error_prefix(),
                    styles::input_badge(&args.tool),
                    names.join(", ")
                );
            }
        }
    };

    if args.web {
        let url = format!("https://github.com/{}/releases", entry.agent.repo);
        open::that(&url)?;
        println!("Opened {}", styles::url(&url));
        return Ok(());
    }

    let runtime = tokio::runtime::Runtime::new()?;
    let github = get_github_data(
        &entry.agent.name,
        &entry.agent.repo,
        &mut disk_cache,
        &runtime,
    )
    .unwrap_or_default();

    disk_cache.save().ok();

    if args.list {
        return run_version_list(&entry.agent, &github);
    }

    if args.latest {
        return print_specific_or_latest_release(&entry.agent.name, &github, None);
    }

    if let Some(ref ver) = args.version {
        return print_specific_or_latest_release(&entry.agent.name, &github, Some(ver));
    }

    if super::styles::is_tty() {
        return run_release_browser(&entry, &github);
    }

    let release = github.latest_release();

    match release {
        Some(r) => print_release(&entry.agent.name, r),
        None => {
            println!(
                "{} No release found for {} ({})",
                styles::error_prefix(),
                styles::agent_name(&entry.agent.name),
                styles::input_badge("latest")
            );
        }
    }

    Ok(())
}

fn resolve_tool(tool: &str, catalog: &[CatalogAgent]) -> Result<ResolveTool> {
    if let Some(entry) = catalog
        .iter()
        .find(|entry| entry.id.eq_ignore_ascii_case(tool))
        .cloned()
    {
        return Ok(ResolveTool::Single(Box::new(entry)));
    }

    let cli_matches: Vec<_> = catalog
        .iter()
        .filter(|entry| {
            entry.agent.cli_binary.as_deref() == Some(tool)
                || entry
                    .agent
                    .alt_binaries
                    .iter()
                    .any(|binary| binary.eq_ignore_ascii_case(tool))
        })
        .cloned()
        .collect();
    match cli_matches.as_slice() {
        [entry] => return Ok(ResolveTool::Single(Box::new(entry.clone()))),
        [] => {}
        many => return Ok(ResolveTool::Ambiguous(many.to_vec())),
    }

    let lower = tool.to_lowercase();
    let matches: Vec<_> = catalog
        .iter()
        .filter(|entry| {
            entry.id.to_lowercase().contains(&lower)
                || entry.agent.name.to_lowercase().contains(&lower)
        })
        .cloned()
        .collect();
    match matches.len() {
        1 => Ok(ResolveTool::Single(Box::new(matches[0].clone()))),
        n if n > 1 => Ok(ResolveTool::Ambiguous(matches)),
        _ => anyhow::bail!(
            "Unknown agent '{}'. Run agents list-sources to see available agents.",
            tool
        ),
    }
}

fn print_specific_or_latest_release(
    name: &str,
    github: &crate::agents::data::GitHubData,
    version: Option<&str>,
) -> Result<()> {
    let release = if let Some(ver) = version {
        github.releases.iter().find(|r| r.version == ver)
    } else {
        github.latest_release()
    };

    match release {
        Some(r) => print_release(name, r),
        None => {
            println!(
                "No release found for {} ({})",
                name,
                version.unwrap_or("latest")
            );
        }
    }

    Ok(())
}

fn run_release_browser(
    entry: &CatalogAgent,
    github: &crate::agents::data::GitHubData,
) -> Result<()> {
    if github.releases.is_empty() {
        println!("No releases found for {}", entry.agent.name);
        return Ok(());
    }

    let items = browser_items_for_agent(&entry.id, &entry.agent, github);
    if let Some(selected) = crate::cli::agents_ui::browse_releases(
        items,
        &format!(" {} Releases ", entry.agent.name),
        false,
    )? {
        println!();
        print_release(&entry.agent.name, &selected.release);
    }

    Ok(())
}

fn print_release(name: &str, release: &crate::agents::data::Release) {
    use super::styles;

    let version = &release.version;
    let date = format_release_date_ymd(release.date.as_deref())
        .unwrap_or_else(|| "unknown date".to_string());
    println!(
        "{} {} ({})",
        styles::agent_name(name),
        styles::key_value(version),
        styles::dim(&date)
    );
    println!("{}", styles::separator(40));
    if has_changelog_body(release.changelog.as_deref()) {
        let body = release.changelog.as_deref().unwrap_or_default();
        print_changelog_body(body);
    } else {
        println!("(no changelog body)");
    }
}

fn has_changelog_body(body: Option<&str>) -> bool {
    body.is_some_and(|body| !body.trim().is_empty())
}

fn sort_recent_release_items(items: &mut [crate::cli::agents_ui::ReleaseBrowserItem]) {
    items.sort_by(|a, b| {
        b.sort_key
            .cmp(&a.sort_key)
            .then_with(|| b.version.cmp(&a.version))
    });
}

fn print_changelog_body(body: &str) {
    if super::styles::is_tty() {
        let skin = super::styles::changelog_skin();
        let rendered = skin.term_text(body).to_string();
        let styled = super::styles::style_urls(&rendered);
        print!("{}", styled);
    } else {
        // Plain text when piped
        use crate::agents::changelog_parser::{parse_changelog, ChangelogBlock};
        let changelog = parse_changelog(body);
        for block in &changelog.blocks {
            match block {
                ChangelogBlock::Heading(text) => println!("\n[{}]", text),
                ChangelogBlock::Bullet(text) => println!("  - {}", text),
                ChangelogBlock::Paragraph(text) => println!("{}", text),
            }
        }
    }
}

fn run_version_list(
    agent: &crate::agents::data::Agent,
    github: &crate::agents::data::GitHubData,
) -> Result<()> {
    use super::styles;

    let count = github.releases.len().to_string();
    println!(
        "{} \u{2014} {} releases\n",
        styles::agent_name(&agent.name),
        styles::dim(&count)
    );

    let mut table = comfy_table::Table::new();
    table.load_preset(comfy_table::presets::UTF8_FULL);
    table.set_header(vec![
        styles::header_cell("Version"),
        styles::header_cell("Released"),
        styles::header_cell("Ago"),
    ]);

    for release in &github.releases {
        let parsed = release
            .date
            .as_deref()
            .and_then(crate::agents::helpers::parse_date);
        let date_str = format_release_date_ymd(release.date.as_deref())
            .unwrap_or_else(|| "\u{2014}".to_string());
        let ago = parsed
            .map(|d| crate::agents::helpers::format_relative_time(&d))
            .unwrap_or_else(|| "\u{2014}".to_string());
        table.add_row(vec![
            styles::bold_cell(&release.version),
            comfy_table::Cell::new(&date_str),
            styles::dim_cell(&ago),
        ]);
    }

    println!("{table}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_agent(
        name: &str,
        repo: &str,
        cli_binary: Option<&str>,
    ) -> crate::agents::data::Agent {
        crate::agents::data::Agent {
            name: name.to_string(),
            repo: repo.to_string(),
            categories: vec!["cli".to_string()],
            installation_method: None,
            pricing: None,
            supported_providers: vec![],
            platform_support: vec![],
            open_source: true,
            cli_binary: cli_binary.map(str::to_string),
            alt_binaries: vec![],
            version_command: vec![],
            update_command: vec![],
            version_regex: None,
            config_files: vec![],
            homepage: None,
            docs: None,
        }
    }

    fn catalog_entry(id: &str, name: &str, cli_binary: Option<&str>) -> CatalogAgent {
        CatalogAgent {
            id: id.to_string(),
            agent: sample_agent(name, &format!("owner/{id}"), cli_binary),
            tracked: true,
        }
    }

    fn sample_github_data(version: &str) -> crate::agents::data::GitHubData {
        crate::agents::data::GitHubData {
            releases: vec![crate::agents::data::Release {
                version: version.to_string(),
                date: Some("2024-06-15".to_string()),
                changelog: Some("changelog".to_string()),
            }],
            ..Default::default()
        }
    }

    fn cached_entry(version: &str) -> crate::agents::cache::CachedGitHubData {
        crate::agents::cache::CachedGitHubData {
            data: crate::agents::cache::SerializableGitHubData::from(&sample_github_data(version)),
            etag: Some("etag-1".to_string()),
            fetched_at: 123,
        }
    }

    #[test]
    fn get_github_data_fresh_branch_syncs_local_cache_from_shared_cache() {
        let repo = "owner/repo";
        let mut local_cache = crate::agents::cache::GitHubCache::new();
        let mut shared_cache = crate::agents::cache::GitHubCache::new();
        shared_cache.insert(repo.to_string(), cached_entry("2.0.0"));

        let result = crate::agents::github::ConditionalFetchResult::Fresh(
            sample_github_data("2.0.0"),
            Some("etag-2".to_string()),
        );

        let (_status, data) =
            apply_github_fetch_result(result, repo, &mut local_cache, Some(shared_cache.clone()));

        assert_eq!(data.unwrap().latest_version(), Some("2.0.0"));
        assert_eq!(
            local_cache.get(repo).and_then(|entry| entry
                .data
                .to_github_data()
                .latest_version()
                .map(str::to_string)),
            Some("2.0.0".to_string())
        );
        assert!(local_cache.get("Different Agent Name").is_none());
    }

    #[test]
    fn get_github_data_not_modified_falls_back_to_cached_repo_key_not_agent_name() {
        let repo = "owner/repo";
        let mut local_cache = crate::agents::cache::GitHubCache::new();
        local_cache.insert(repo.to_string(), cached_entry("1.2.3"));
        local_cache.insert("Agent Name".to_string(), cached_entry("9.9.9"));

        let (_status, data) = apply_github_fetch_result(
            crate::agents::github::ConditionalFetchResult::NotModified,
            repo,
            &mut local_cache,
            None,
        );

        assert_eq!(data.unwrap().latest_version(), Some("1.2.3"));
    }

    #[test]
    fn get_github_data_error_falls_back_to_cached_repo_key_not_agent_name() {
        let repo = "owner/repo";
        let mut local_cache = crate::agents::cache::GitHubCache::new();
        local_cache.insert(repo.to_string(), cached_entry("3.4.5"));
        local_cache.insert("Agent Name".to_string(), cached_entry("0.0.1"));

        let (_status, data) = apply_github_fetch_result(
            crate::agents::github::ConditionalFetchResult::Error("network down".to_string()),
            repo,
            &mut local_cache,
            None,
        );

        assert_eq!(data.unwrap().latest_version(), Some("3.4.5"));
    }

    #[test]
    fn format_release_date_ymd_formats_plain_iso_date() {
        assert_eq!(
            format_release_date_ymd(Some("2024-06-15")),
            Some("2024-06-15".to_string())
        );
    }

    #[test]
    fn format_release_date_ymd_accepts_rfc3339_offset_input() {
        assert_eq!(
            format_release_date_ymd(Some("2024-06-15T23:30:00-02:00")),
            Some("2024-06-16".to_string())
        );
    }

    #[test]
    fn tool_args_parses_latest_flag() {
        let parsed =
            ToolArgs::parse_from(vec!["claude".to_string(), "--latest".to_string()]).unwrap();
        assert_eq!(parsed.tool, "claude");
        assert!(parsed.latest);
        assert!(!parsed.list);
        assert!(parsed.version.is_none());
    }

    #[test]
    fn tool_args_rejects_conflicting_latest_and_list() {
        let err = ToolArgs::parse_from(vec![
            "claude".to_string(),
            "--latest".to_string(),
            "--list".to_string(),
        ])
        .unwrap_err()
        .to_string();
        assert!(err.contains("mutually exclusive"));
    }

    #[test]
    fn resolve_tool_returns_ambiguous_matches_for_partial_name() {
        let catalog = vec![
            catalog_entry("claude-code", "Claude Code", Some("claude")),
            catalog_entry("codex", "OpenAI Codex", Some("codex")),
            catalog_entry("opencode", "OpenCode", Some("opencode")),
        ];

        match resolve_tool("code", &catalog).unwrap() {
            ResolveTool::Single(_) => panic!("expected ambiguous result"),
            ResolveTool::Ambiguous(matches) => {
                let ids: Vec<_> = matches.iter().map(|entry| entry.id.as_str()).collect();
                assert!(ids.contains(&"claude-code"));
                assert!(ids.contains(&"opencode"));
            }
        }
    }

    #[test]
    fn has_changelog_body_rejects_blank_strings() {
        assert!(!has_changelog_body(None));
        assert!(!has_changelog_body(Some("")));
        assert!(!has_changelog_body(Some("   ")));
        assert!(has_changelog_body(Some("fixed a bug")));
    }

    #[test]
    fn sort_recent_release_items_uses_timestamp_not_rendered_date() {
        let mut items = vec![
            crate::cli::agents_ui::ReleaseBrowserItem {
                agent_name: "Alpha".to_string(),
                version: "1.0.0".to_string(),
                released: "2026-03-11".to_string(),
                ago: "1h ago".to_string(),
                body: Some("a".to_string()),
                sort_key: 100,
                release: crate::agents::data::Release {
                    version: "1.0.0".to_string(),
                    date: Some("2026-03-11T10:00:00Z".to_string()),
                    changelog: Some("a".to_string()),
                },
            },
            crate::cli::agents_ui::ReleaseBrowserItem {
                agent_name: "Beta".to_string(),
                version: "2.0.0".to_string(),
                released: "2026-03-11".to_string(),
                ago: "10m ago".to_string(),
                body: Some("b".to_string()),
                sort_key: 200,
                release: crate::agents::data::Release {
                    version: "2.0.0".to_string(),
                    date: Some("2026-03-11T11:00:00Z".to_string()),
                    changelog: Some("b".to_string()),
                },
            },
        ];

        sort_recent_release_items(&mut items);
        assert_eq!(items[0].agent_name, "Beta");
        assert_eq!(items[1].agent_name, "Alpha");
    }
}
