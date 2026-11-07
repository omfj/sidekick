use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::duration::TimeDuration;

/// Settings from `config.toml`.
///
/// Every field is optional. Flags and env vars override whatever is set here.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub effort: Option<String>,
    pub idle: Option<TimeDuration>,
    pub max_wait: Option<TimeDuration>,
    pub cost_every: Option<TimeDuration>,
    pub notify: Option<bool>,
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let (file, explicit) = match path {
            Some(p) => (p.to_path_buf(), true),
            None => (default_path()?, false),
        };
        let text = match fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound && !explicit => {
                return Ok(Self::default());
            }
            Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
        };
        let config: Self =
            toml::from_str(&text).with_context(|| format!("invalid config {}", file.display()))?;
        if config.api_key.is_some() {
            warn_if_readable_by_others(&file);
        }
        Ok(config)
    }
}

pub fn default_path() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir).join("sidekick/config.toml"));
    }
    let home = std::env::var_os("HOME").context("neither XDG_CONFIG_HOME nor HOME is set")?;
    Ok(PathBuf::from(home).join(".config/sidekick/config.toml"))
}

fn warn_if_readable_by_others(file: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = fs::metadata(file)
        && meta.permissions().mode() & 0o077 != 0
    {
        crate::render::error(&format!(
            "{} holds an API key but other users can read it; run `chmod 600 {}`",
            file.display(),
            file.display()
        ));
    }
}
