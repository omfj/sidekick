use std::sync::atomic::{AtomicBool, Ordering};

use crate::chat::Usage;
use crate::review::{Severity, Suggestion};

static LAST_WAS_COST: AtomicBool = AtomicBool::new(false);

fn printed_other() {
    LAST_WAS_COST.store(false, Ordering::Relaxed);
}

pub fn last_was_cost() -> bool {
    LAST_WAS_COST.load(Ordering::Relaxed)
}

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";

fn color(severity: Severity) -> &'static str {
    match severity {
        Severity::Bug | Severity::Security => "\x1b[31m",
        Severity::Risk => "\x1b[33m",
        Severity::Design | Severity::Simplify => "\x1b[36m",
        Severity::Nit => "\x1b[90m",
    }
}

/// Makes untrusted text (model output, file names, API errors) safe to print:
/// control characters are shown escaped instead of being sent to the
/// terminal, where they could hide output or run escape sequences.
fn safe(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' | '\t' => c.to_string(),
            c if c.is_control() => c.escape_unicode().to_string(),
            c => c.to_string(),
        })
        .collect()
}

fn safe_line(text: &str) -> String {
    safe(&text.replace('\n', " "))
}

pub fn status(msg: &str) {
    printed_other();
    println!("{DIM}· {}{RESET}", safe_line(msg));
}

pub fn tool(desc: &str) {
    printed_other();
    println!("{DIM}  │ {}{RESET}", safe_line(desc));
}

pub fn error(msg: &str) {
    printed_other();
    eprintln!("\x1b[31m✗ {}{RESET}", safe(msg));
}

pub fn all_good(files: &[String]) {
    printed_other();
    println!(
        "{DIM}✓ {} — nothing to flag{RESET}",
        safe_line(&files.join(", "))
    );
}

pub fn suggestion(s: &Suggestion) {
    printed_other();
    let c = color(s.severity);
    let file = safe_line(&s.file);
    let loc = if s.line > 0 {
        format!("{file}:{}", s.line)
    } else {
        file
    };
    println!();
    println!(
        "{c}{BOLD}[{}]{RESET} {BOLD}{}{RESET}  {DIM}{loc}{RESET}",
        s.severity,
        safe_line(&s.title)
    );
    for line in s.explanation.lines() {
        println!("  {}", safe(line));
    }
    if !s.hint.trim().is_empty() {
        let hint: Vec<_> = s.hint.lines().map(safe).collect();
        println!("  {c}hint:{RESET} {}", hint.join("\n        "));
    }
}

pub fn cost(u: &Usage) {
    LAST_WAS_COST.store(true, Ordering::Relaxed);
    let k = |n: u64| {
        if n >= 10_000 {
            format!("{}k", n / 1000)
        } else {
            n.to_string()
        }
    };
    let cost = u
        .cost
        .map_or("cost not reported".to_string(), |c| format!("${c:.4}"));
    let cached = if u.cached_tokens > 0 {
        format!(" ({} cached)", k(u.cached_tokens))
    } else {
        String::new()
    };
    println!(
        "{DIM}$ session: {cost} · {} requests · {} in{cached} / {} out{RESET}",
        u.requests,
        k(u.prompt_tokens),
        k(u.completion_tokens)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_escapes_control_characters() {
        let text = "a\x1b]777;notify;x;y\x07b";
        let out = safe(text);
        assert!(!out.chars().any(char::is_control));
        assert_eq!(out, "a\\u{1b}]777;notify;x;y\\u{7}b");
        assert_eq!(safe("keep\ttabs\nand newlines"), "keep\ttabs\nand newlines");
        assert_eq!(safe_line("one\ntwo"), "one two");
    }
}
