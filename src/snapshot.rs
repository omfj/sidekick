use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use similar::TextDiff;

const MAX_FILE_BYTES: u64 = 256 * 1024;

/// Editor temp/backup files. A project .gitignore can un-ignore them with `!pattern`.
const EDITOR_JUNK: &[&str] = &[
    "*~",
    "*.swp",
    "*.swo",
    "*.swx",
    "4913", // Neovim: https://github.com/neovim/neovim/issues/3460
    ".#*",
    "\\#*#",         // escaped: a leading # is a comment
    "*___jb_tmp___", // JetBrains safe-write
    "*___jb_old___",
    ".DS_Store",
];

pub struct Snapshot {
    root: PathBuf,
    /// Lowest priority first; deepest directories last.
    ignores: Vec<Gitignore>,
    files: HashMap<PathBuf, String>,
}

pub struct Change {
    pub path: PathBuf,
    pub diff: String,
    /// `None` if the file was deleted.
    pub contents: Option<String>,
}

impl Snapshot {
    pub fn new(root: &Path) -> Self {
        let mut files = HashMap::new();
        // Project .gitignores found below override these.
        let mut ignores = vec![
            editor_junk(root),
            GitignoreBuilder::new(root).build_global().0,
        ];
        let walker = WalkBuilder::new(root)
            .require_git(false)
            .hidden(false)
            .build();
        for entry in walker.flatten() {
            let path = entry.path();
            if entry.file_name() == ".gitignore" {
                let mut builder = GitignoreBuilder::new(path.parent().unwrap_or(root));
                builder.add(path);
                ignores.extend(builder.build().ok());
            }
            if entry.file_type().is_some_and(|t| t.is_file())
                && !is_git_internal(root, path)
                && let Some(text) = read_text(path)
            {
                files.insert(path.to_path_buf(), text);
            }
        }

        ignores.sort_by_key(|g| g.path().components().count());
        let mut snapshot = Self {
            root: root.to_path_buf(),
            ignores,
            files,
        };
        // The walker doesn't know about the built-in junk patterns.
        let ignored: Vec<_> = snapshot
            .files
            .keys()
            .filter(|p| snapshot.is_ignored(p))
            .cloned()
            .collect();
        for p in ignored {
            snapshot.files.remove(&p);
        }
        snapshot
    }

    pub fn is_tracked(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn tracked_count(&self) -> usize {
        self.files.len()
    }

    pub fn is_ignored(&self, path: &Path) -> bool {
        if is_git_internal(&self.root, path) || !path.starts_with(&self.root) {
            return true;
        }
        // The deepest .gitignore with an opinion wins, like git.
        let is_dir = path.is_dir();
        for gi in self
            .ignores
            .iter()
            .rev()
            .filter(|g| path.starts_with(g.path()))
        {
            let m = gi.matched_path_or_any_parents(path, is_dir);
            if !m.is_none() {
                return m.is_ignore();
            }
        }
        false
    }

    pub fn relative<'a>(&self, path: &'a Path) -> &'a Path {
        path.strip_prefix(&self.root).unwrap_or(path)
    }

    pub fn update(&mut self, path: &Path) -> Option<Change> {
        let old = self.files.get(path).cloned();
        let new = if path.is_file() {
            read_text(path)
        } else {
            None
        };

        if old == new {
            return None;
        }

        let rel = self.relative(path).display().to_string();
        let diff = TextDiff::from_lines(old.as_deref().unwrap_or(""), new.as_deref().unwrap_or(""))
            .unified_diff()
            .context_radius(3)
            .header(&format!("a/{rel}"), &format!("b/{rel}"))
            .to_string();

        match &new {
            Some(text) => self.files.insert(path.to_path_buf(), text.clone()),
            None => self.files.remove(path),
        };

        Some(Change {
            path: path.to_path_buf(),
            diff,
            contents: new,
        })
    }
}

fn editor_junk(root: &Path) -> Gitignore {
    let mut builder = GitignoreBuilder::new(root);
    for pattern in EDITOR_JUNK {
        builder
            .add_line(None, pattern)
            .expect("built-in ignore pattern is valid");
    }
    builder.build().expect("built-in ignore patterns are valid")
}

fn is_git_internal(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root)
        .ok()
        .and_then(|rel| rel.components().next())
        .is_some_and(|c| c.as_os_str() == ".git")
}

/// Read the contents of a file to a UTF-8 string.
///
/// Returns `None` for large, binary or non-UTF-8 files.
pub fn read_text(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    if meta.len() > MAX_FILE_BYTES {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}
