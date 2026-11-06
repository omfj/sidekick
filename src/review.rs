use std::fmt;
use std::fs;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::chat::{ChatClient, ChatRequest, Usage};
use crate::snapshot::{Change, Snapshot};
use crate::{render, tools};

const GUIDELINE_FILES: &[&str] = &["AGENTS.md", "CLAUDE.md", ".claude/CLAUDE.md"];

const MAX_TOOL_ROUNDS: usize = 8;

const SYSTEM_PROMPT: &str = r#"You are sidekick, a code reviewer sitting next to a developer while they work.
You do not write their code. Each time they save, you see the diff and the current
contents of the changed files, and you decide whether anything is worth mentioning.

Only raise things that a sharp senior colleague would actually interrupt for: bugs,
edge cases that will bite, security problems, misleading names, design choices that
will be painful later, or a clearly simpler approach. Most saves are fine or are
work in progress; in that case return an empty suggestions list. Do not comment on
code that is obviously half-written mid-edit unless it reveals a real misconception.

You have read-only tools to look at the rest of the repository. Use them when the
change can't be judged on its own, for example to check how a function it calls
behaves, whether a type has the field being used, or how other callers do it. Don't
explore for its own sake; a typical review needs no tools or just a few calls.

For each suggestion, explain the problem and give a hint pointing the developer
toward the fix. Keep code in hints to a line or two at most; the goal is for the
developer to write the fix themselves and understand why. Do not repeat suggestions
listed under 'already mentioned' unless the new change makes them more urgent."#;

#[derive(Debug, Deserialize)]
pub struct Review {
    pub suggestions: Vec<Suggestion>,
}

/// Declared most severe first, so the derived `Ord` ranks them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Bug,
    Security,
    Risk,
    Design,
    Simplify,
    Nit,
}

impl Severity {
    const ALL: [Severity; 6] = [
        Severity::Bug,
        Severity::Security,
        Severity::Risk,
        Severity::Design,
        Severity::Simplify,
        Severity::Nit,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Bug => "bug",
            Severity::Security => "security",
            Severity::Risk => "risk",
            Severity::Design => "design",
            Severity::Simplify => "simplify",
            Severity::Nit => "nit",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Deserialize)]
pub struct Suggestion {
    pub file: String,
    /// 1-based line in the current file, or 0 when it doesn't apply to one line.
    pub line: u32,
    pub severity: Severity,
    pub title: String,
    pub explanation: String,
    pub hint: String,
}

pub struct Reviewer {
    client: ChatClient,
    /// Recent suggestion titles, so the model doesn't repeat itself.
    history: Vec<String>,
}

impl Reviewer {
    pub fn new(client: ChatClient) -> Self {
        Self {
            client,
            history: Vec::new(),
        }
    }

    pub fn usage(&self) -> &Usage {
        self.client.usage()
    }

    pub fn review(&mut self, snapshot: &Snapshot, changes: &[(String, &Change)]) -> Result<Review> {
        let mut messages = vec![
            json!({ "role": "system", "content": system_prompt(snapshot) }),
            json!({ "role": "user", "content": self.prompt(changes) }),
        ];

        for round in 0..=MAX_TOOL_ROUNDS {
            let message = self.client.chat(ChatRequest {
                messages: &messages,
                tools: Some(tools::definitions()),
                tools_allowed: round < MAX_TOOL_ROUNDS,
                schema: Some(("review", review_schema())),
            })?;

            let calls = message["tool_calls"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if calls.is_empty() {
                let text = message["content"]
                    .as_str()
                    .context("response had no message content")?;
                let review: Review =
                    serde_json::from_str(text).context("could not parse review JSON")?;
                self.remember(&review);
                return Ok(review);
            }

            // Echo the assistant turn back unchanged: it may carry reasoning the provider needs.
            messages.push(message);
            for call in &calls {
                let name = call["function"]["name"].as_str().unwrap_or("");
                let args: Value = call["function"]["arguments"]
                    .as_str()
                    .and_then(|a| serde_json::from_str(a).ok())
                    .unwrap_or(Value::Null);
                let output = if args.is_object() {
                    render::tool(&tools::describe(name, &args));
                    tools::run(snapshot, name, &args)
                } else {
                    "error: arguments were not a valid JSON object".to_string()
                };
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call["id"],
                    "content": output,
                }));
            }
        }
        unreachable!("the last round runs without tools, so it always returns")
    }

    fn remember(&mut self, review: &Review) {
        self.history.extend(
            review
                .suggestions
                .iter()
                .map(|s| format!("{}: {}", s.file, s.title)),
        );
        let excess = self.history.len().saturating_sub(30);
        self.history.drain(..excess);
    }

    fn prompt(&self, changes: &[(String, &Change)]) -> String {
        let mut out = String::new();
        if !self.history.is_empty() {
            out.push_str("<already_mentioned>\n");
            for h in &self.history {
                out.push_str(&format!("- {h}\n"));
            }
            out.push_str("</already_mentioned>\n\n");
        }
        for (rel, change) in changes {
            out.push_str(&format!(
                "<file path=\"{rel}\">\n<diff>\n{}</diff>\n",
                change.diff
            ));
            match &change.contents {
                Some(text) => {
                    out.push_str(&format!("<current_contents>\n{text}</current_contents>\n"))
                }
                None => out.push_str("<deleted/>\n"),
            }
            out.push_str("</file>\n\n");
        }
        out.push_str("Review this save.");
        out
    }
}

/// Re-read on every review so guideline edits apply without a restart.
fn system_prompt(snapshot: &Snapshot) -> String {
    let mut prompt = SYSTEM_PROMPT.to_string();
    for name in GUIDELINE_FILES {
        let path = snapshot.root().join(name);
        if let Ok(text) = fs::read_to_string(&path) {
            prompt.push_str(&format!(
                "\n\nThe project's {name} describes its conventions. Use it to judge \
                 whether the code fits the project; it was written for coding agents, so \
                 ignore parts about how an agent should behave.\n\
                 <project_guidelines file=\"{name}\">\n{}\n</project_guidelines>",
                text.trim()
            ));
        }
    }
    prompt
}

pub fn guideline_files(snapshot: &Snapshot) -> Vec<&'static str> {
    GUIDELINE_FILES
        .iter()
        .copied()
        .filter(|f| snapshot.root().join(f).is_file())
        .collect()
}

fn review_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "suggestions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "file": { "type": "string" },
                        "line": { "type": "integer" },
                        "severity": { "type": "string", "enum": Severity::ALL.map(Severity::as_str) },
                        "title": { "type": "string" },
                        "explanation": { "type": "string" },
                        "hint": { "type": "string" }
                    },
                    "required": ["file", "line", "severity", "title", "explanation", "hint"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["suggestions"],
        "additionalProperties": false
    })
}
