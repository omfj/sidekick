use std::io::{self, IsTerminal, Write};

use crate::review::Suggestion;

/// Sends a desktop notification via the terminal (OSC 777).
pub fn notify_review(suggestions: &[Suggestion]) {
    let Some(top) = suggestions.iter().min_by_key(|s| s.severity) else {
        return;
    };
    let mut stdout = io::stdout();
    if !stdout.is_terminal() {
        return;
    }

    let title = match suggestions.len() {
        1 => "sidekick: 1 suggestion".to_string(),
        n => format!("sidekick: {n} suggestions"),
    };
    let location = if top.line > 0 {
        format!("{}:{}", top.file, top.line)
    } else {
        top.file.clone()
    };
    let mut body = format!("[{}] {} ({location})", top.severity, top.title);
    if suggestions.len() > 1 {
        body.push_str(&format!(" +{} more", suggestions.len() - 1));
    }

    let _ = write!(
        stdout,
        "\x1b]777;notify;{};{}\x07",
        clean(&title, true),
        clean(&body, false)
    );
    let _ = stdout.flush();
}

/// The text comes from the model, so strip anything that could end the escape
/// sequence early or inject another one. `;` separates the title from the body.
fn clean(text: &str, is_title: bool) -> String {
    text.chars()
        .map(|c| match c {
            ';' if is_title => ',',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_strips_escape_sequences() {
        let evil = "a\x1b]777;notify;x;y\x07b\nc";
        let body = clean(evil, false);
        assert!(!body.chars().any(char::is_control));
        assert!(!clean(evil, true).contains(';'));
    }
}
