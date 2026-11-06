use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct ProjectRecord {
    pub path: PathBuf,
    /// Unix seconds.
    pub allowed_at: u64,
    /// Unix seconds.
    pub last_opened: u64,
}

pub fn ensure_allowed(root: &Path) -> Result<ProjectRecord> {
    let file = record_path(root)?;

    let mut records: Vec<ProjectRecord> = match fs::read_to_string(&file) {
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("corrupt project record {}", file.display()))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
    };

    let index = match records.iter().position(|r| r.path == root) {
        Some(i) => i,
        None => {
            if !ask(root)? {
                bail!("not allowed to read {}", root.display());
            }
            records.push(ProjectRecord {
                path: root.to_path_buf(),
                allowed_at: now(),
                last_opened: 0,
            });
            records.len() - 1
        }
    };

    records[index].last_opened = now();
    fs::create_dir_all(file.parent().unwrap())?;
    fs::write(&file, serde_json::to_string_pretty(&records)?)
        .with_context(|| format!("writing {}", file.display()))?;
    Ok(records.swap_remove(index))
}

fn ask(root: &Path) -> Result<bool> {
    if !io::stdin().is_terminal() {
        bail!(
            "{} has not been allowed yet; run sidekick interactively once to approve it",
            root.display()
        );
    }
    print!(
        "sidekick will read files in {} and its subdirectories and send changed files \
         to your model provider.\nAllow? [y/N] ",
        root.display()
    );
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
}

fn data_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir).join("sidekick"));
    }
    let home = std::env::var_os("HOME").context("neither XDG_DATA_HOME nor HOME is set")?;
    Ok(PathBuf::from(home).join(".local/share/sidekick"))
}

/// Same naming as Claude Code: `/Users/me/app` -> `-Users-me-app.json`.
fn record_path(root: &Path) -> Result<PathBuf> {
    let flat: String = root
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    Ok(data_dir()?.join(format!("{flat}.json")))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
