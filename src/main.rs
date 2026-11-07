mod chat;
mod config;
mod desktop;
mod duration;
mod render;
mod review;
mod snapshot;
mod tools;
mod trust;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;

use chat::ChatClient;
use duration::TimeDuration;
use review::Reviewer;
use snapshot::Snapshot;

/// An AI sidekick that watches you code and suggests improvements!
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Directory to watch.
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Config file to read [default: ~/.config/sidekick/config.toml].
    #[arg(long, env = "SIDEKICK_CONFIG")]
    config: Option<PathBuf>,

    /// Model to review with (any model your provider serves) [default: anthropic/claude-opus-5.5].
    #[arg(long, env = "SIDEKICK_MODEL")]
    model: Option<String>,

    /// OpenAI-compatible API base URL (OpenRouter, OpenAI, Ollama, LM Studio, ...)
    /// [default: https://openrouter.ai/api/v1].
    #[arg(long, env = "SIDEKICK_BASE_URL")]
    base_url: Option<String>,

    /// Reasoning effort to request (low, medium, high), if the model supports it.
    #[arg(long, env = "SIDEKICK_EFFORT")]
    effort: Option<String>,

    /// Review once you've stopped saving for this long (e.g. 10s, 1m). Saves
    /// within the window are batched into one review to save API calls [default: 10s].
    #[arg(long, env = "SIDEKICK_IDLE")]
    idle: Option<TimeDuration>,

    /// Never hold a batch longer than this, even if you keep saving [default: 1m].
    #[arg(long, env = "SIDEKICK_MAX_WAIT")]
    max_wait: Option<TimeDuration>,

    /// How often to print the session's API cost (0 to disable) [default: 3m].
    #[arg(long, env = "SIDEKICK_COST_EVERY")]
    cost_every: Option<TimeDuration>,

    /// Send a desktop notification when a review has suggestions.
    #[arg(long, env = "SIDEKICK_NOTIFY")]
    notify: bool,
}

/// Settings after merging flags, env vars, the config file and defaults.
struct Settings {
    api_key: String,
    model: String,
    base_url: String,
    effort: Option<String>,
    idle: TimeDuration,
    max_wait: TimeDuration,
    cost_every: TimeDuration,
    notify: bool,
}

impl Settings {
    fn resolve(args: &Args) -> Result<Self> {
        let config = config::Config::load(args.config.as_deref())?;
        let duration = |s: &str| s.parse::<TimeDuration>().expect("valid default");
        let api_key = std::env::var("SIDEKICK_API_KEY")
            .or_else(|_| std::env::var("OPENROUTER_API_KEY"))
            .ok()
            .or(config.api_key)
            .unwrap_or_default(); // local servers like Ollama don't need one
        Ok(Self {
            api_key,
            model: args
                .model
                .clone()
                .or(config.model)
                .unwrap_or_else(|| "anthropic/claude-opus-5.5".into()),
            base_url: args
                .base_url
                .clone()
                .or(config.base_url)
                .unwrap_or_else(|| "https://openrouter.ai/api/v1".into()),
            effort: args.effort.clone().or(config.effort),
            idle: args.idle.or(config.idle).unwrap_or(duration("10s")),
            max_wait: args.max_wait.or(config.max_wait).unwrap_or(duration("1m")),
            cost_every: args
                .cost_every
                .or(config.cost_every)
                .unwrap_or(duration("3m")),
            notify: args.notify || config.notify.unwrap_or(false),
        })
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let settings = Settings::resolve(&args)?;
    let root = args
        .path
        .canonicalize()
        .context("watch path does not exist")?;
    trust::ensure_allowed(&root)?;

    if settings.api_key.is_empty() && settings.base_url.contains("openrouter.ai") {
        anyhow::bail!(
            "set OPENROUTER_API_KEY (or SIDEKICK_API_KEY), or `api_key` in {}, to your OpenRouter key",
            config::default_path()?.display()
        );
    }

    let mut snapshot = Snapshot::new(&root);
    let client = ChatClient::new(
        settings.base_url,
        settings.api_key,
        settings.model.clone(),
        settings.effort,
    )?;
    let mut reviewer = Reviewer::new(client);

    let (tx, rx) = mpsc::channel();
    // Only collapses the events of a single save; batching happens in `wait_for_batch`.
    let mut debouncer = new_debouncer(Duration::from_millis(300), tx)?;
    debouncer.watcher().watch(&root, RecursiveMode::Recursive)?;

    render::status(&format!(
        "sidekick watching {} ({} files) with {} — go write some code!",
        root.display(),
        snapshot.tracked_count(),
        settings.model
    ));
    let guidelines = review::guideline_files(&snapshot);
    if !guidelines.is_empty() {
        render::status(&format!(
            "using project guidelines from {}",
            guidelines.join(", ")
        ));
    }

    let mut cost_timer = CostTimer::new(settings.cost_every);

    loop {
        let paths = wait_for_batch(
            &rx,
            &snapshot,
            settings.idle.get(),
            settings.max_wait.get(),
            &mut || cost_timer.tick(reviewer.usage()),
        )?;

        let mut changes = Vec::new();
        for path in paths {
            changes.extend(snapshot.update(&path));
        }
        if changes.is_empty() {
            render::status("no net changes; skipping review");
            continue;
        }

        let labelled: Vec<_> = changes
            .iter()
            .map(|c| (snapshot.relative(&c.path).display().to_string(), c))
            .collect();
        let names: Vec<_> = labelled.iter().map(|(n, _)| n.clone()).collect();
        render::status(&format!("reviewing {}...", names.join(", ")));

        match reviewer.review(&snapshot, &labelled) {
            Ok(review) if review.suggestions.is_empty() => render::all_good(&names),
            Ok(review) => {
                review.suggestions.iter().for_each(render::suggestion);
                if settings.notify {
                    desktop::notify_review(&review.suggestions);
                }
            }
            Err(e) => render::error(&format!("{e:#}")),
        }
        cost_timer.tick(reviewer.usage());
    }
}

type WatchEvents = notify_debouncer_mini::DebounceEventResult;

/// Wait for a change, then keep collecting until the user has been idle for
/// `idle` (capped at `max_wait`).
fn wait_for_batch(
    rx: &mpsc::Receiver<WatchEvents>,
    snapshot: &Snapshot,
    idle: Duration,
    max_wait: Duration,
    on_idle: &mut dyn FnMut(),
) -> Result<BTreeSet<PathBuf>> {
    let mut paths = BTreeSet::new();
    loop {
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(events) => {
                if collect(events, snapshot, &mut paths) {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => on_idle(),
            Err(mpsc::RecvTimeoutError::Disconnected) => anyhow::bail!("file watcher stopped"),
        }
    }

    let names: Vec<_> = paths
        .iter()
        .map(|p| snapshot.relative(p).display().to_string())
        .collect();
    render::status(&format!(
        "noticed {}; reviewing after {}s without saves",
        names.join(", "),
        idle.as_secs()
    ));

    let first = Instant::now();
    let mut last = first;
    loop {
        let deadline = (last + idle).min(first + max_wait);
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        match rx.recv_timeout(remaining) {
            Ok(events) => {
                if collect(events, snapshot, &mut paths) {
                    last = Instant::now();
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => anyhow::bail!("file watcher stopped"),
        }
    }
    Ok(paths)
}

/// Returns whether any path was relevant, so ignored files don't delay reviews.
fn collect(events: WatchEvents, snapshot: &Snapshot, paths: &mut BTreeSet<PathBuf>) -> bool {
    match events {
        Ok(events) => {
            let mut relevant = false;
            for e in events {
                // Gone and never tracked: a temp file written and deleted mid-save.
                let transient = !e.path.exists() && !snapshot.is_tracked(&e.path);
                if !transient && !snapshot.is_ignored(&e.path) {
                    relevant = true;
                    paths.insert(e.path);
                }
            }
            relevant
        }
        Err(e) => {
            render::error(&format!("watch error: {e}"));
            false
        }
    }
}

struct CostTimer {
    interval: Option<Duration>,
    last: Instant,
}

impl CostTimer {
    fn new(every: TimeDuration) -> Self {
        let interval = Some(every.get()).filter(|d| !d.is_zero());
        Self {
            interval,
            last: Instant::now(),
        }
    }

    fn tick(&mut self, usage: &chat::Usage) {
        let Some(interval) = self.interval else {
            return;
        };
        if self.last.elapsed() < interval {
            return;
        }
        self.last = Instant::now();
        if usage.requests > 0 && !render::last_was_cost() {
            render::cost(usage);
        }
    }
}
