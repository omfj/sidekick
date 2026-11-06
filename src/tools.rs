use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use ignore::WalkBuilder;
use regex::Regex;
use serde_json::{Value, json};

use crate::snapshot::Snapshot;

const MAX_READ_LINES: usize = 1500;
const MAX_LIST_ENTRIES: usize = 400;
const MAX_SEARCH_HITS: usize = 60;

pub fn definitions() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a file in the repository, with line numbers. Use to check code the change depends on (callers, callees, types, config).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Path relative to the repository root." },
                        "start_line": { "type": "integer", "description": "First line to read, 1-based. Defaults to 1." },
                        "end_line": { "type": "integer", "description": "Last line to read, inclusive. Defaults to end of file." }
                    },
                    "required": ["path"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "list_files",
                "description": "List files and directories under a path in the repository (gitignored entries are omitted).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Directory relative to the repository root. Defaults to the root." },
                        "depth": { "type": "integer", "description": "How many levels deep to list. Defaults to 2." }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "search",
                "description": "Search file contents in the repository with a regular expression. Returns matching lines as path:line: text.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string", "description": "Regular expression (Rust regex syntax)." },
                        "path": { "type": "string", "description": "Directory or file to search in, relative to the root. Defaults to the root." }
                    },
                    "required": ["pattern"]
                }
            }
        }
    ])
}

/// Errors are returned as text for the model to read.
pub fn run(snapshot: &Snapshot, name: &str, args: &Value) -> String {
    let result = match name {
        "read_file" => read_file(snapshot, args),
        "list_files" => list_files(snapshot, args),
        "search" => search(snapshot, args),
        other => Err(anyhow!("unknown tool {other}")),
    };
    result.unwrap_or_else(|e| format!("error: {e:#}"))
}

pub fn describe(name: &str, args: &Value) -> String {
    let s = |k: &str| args[k].as_str().unwrap_or(".").to_string();
    match name {
        "read_file" => format!("read {}", s("path")),
        "list_files" => format!("listed {}", s("path")),
        "search" => format!(
            "searched for /{}/ in {}",
            args["pattern"].as_str().unwrap_or(""),
            s("path")
        ),
        other => other.to_string(),
    }
}

/// Refuses anything outside the root or gitignored.
fn resolve(snapshot: &Snapshot, rel: Option<&str>) -> Result<PathBuf> {
    let root = snapshot.root();
    let rel = rel.unwrap_or(".").trim_start_matches('/');
    let path = root
        .join(rel)
        .canonicalize()
        .map_err(|_| anyhow!("{rel} does not exist"))?;
    if !path.starts_with(root) || (path != root && snapshot.is_ignored(&path)) {
        bail!("{rel} is outside the repository or ignored");
    }
    Ok(path)
}

fn read_file(snapshot: &Snapshot, args: &Value) -> Result<String> {
    let path = resolve(snapshot, args["path"].as_str())?;
    let text = crate::snapshot::read_text(&path)
        .ok_or_else(|| anyhow!("not a readable text file (binary or too large)"))?;
    let lines: Vec<_> = text.lines().collect();
    let start = args["start_line"].as_u64().unwrap_or(1).max(1) as usize;
    let end = args["end_line"]
        .as_u64()
        .map_or(lines.len(), |e| e as usize)
        .min(lines.len());
    if start > end {
        return Ok(format!("(file has {} lines)", lines.len()));
    }
    let end_capped = end.min(start + MAX_READ_LINES - 1);
    let mut out: String = lines[start - 1..end_capped]
        .iter()
        .enumerate()
        .map(|(i, l)| format!("{:>5}\t{l}\n", start + i))
        .collect();
    if end_capped < end {
        out.push_str(&format!(
            "… truncated; file has {} lines, request a later range\n",
            lines.len()
        ));
    }
    Ok(out)
}

fn list_files(snapshot: &Snapshot, args: &Value) -> Result<String> {
    let dir = resolve(snapshot, args["path"].as_str())?;
    let depth = args["depth"].as_u64().unwrap_or(2).clamp(1, 6) as usize;
    let mut out = String::new();
    let mut count = 0;
    for entry in walker(&dir).max_depth(Some(depth)).build().flatten() {
        if entry.depth() == 0 {
            continue;
        }
        if count == MAX_LIST_ENTRIES {
            out.push_str("… more entries omitted; list a subdirectory\n");
            break;
        }
        let rel = snapshot.relative(entry.path()).display();
        let slash = if entry.file_type().is_some_and(|t| t.is_dir()) {
            "/"
        } else {
            ""
        };
        out.push_str(&format!("{rel}{slash}\n"));
        count += 1;
    }
    Ok(if out.is_empty() {
        "(empty)".into()
    } else {
        out
    })
}

fn search(snapshot: &Snapshot, args: &Value) -> Result<String> {
    let pattern = args["pattern"]
        .as_str()
        .ok_or_else(|| anyhow!("pattern is required"))?;
    let re = Regex::new(pattern)?;
    let base = resolve(snapshot, args["path"].as_str())?;
    let mut out = String::new();
    let mut hits = 0;
    for entry in walker(&base).build().flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Some(text) = crate::snapshot::read_text(entry.path()) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            if re.is_match(line) {
                let line = if line.len() > 200 {
                    format!("{}…", &line[..line.floor_char_boundary(200)])
                } else {
                    line.to_string()
                };
                out.push_str(&format!(
                    "{}:{}: {}\n",
                    snapshot.relative(entry.path()).display(),
                    i + 1,
                    line.trim()
                ));
                hits += 1;
                if hits == MAX_SEARCH_HITS {
                    out.push_str("… more matches omitted; narrow the pattern or path\n");
                    return Ok(out);
                }
            }
        }
    }
    Ok(if out.is_empty() {
        "no matches".into()
    } else {
        out
    })
}

fn walker(path: &Path) -> WalkBuilder {
    let mut b = WalkBuilder::new(path);
    b.require_git(false)
        .hidden(false)
        .filter_entry(|e| e.file_name() != ".git");
    b.sort_by_file_name(|a, b| a.cmp(b));
    b
}
