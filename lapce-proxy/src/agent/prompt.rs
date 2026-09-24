//! Builds the text sent to the agent from the user's message and editor context.

use std::path::PathBuf;

/// Maximum number of context bytes attached per file or selection.
pub const MAX_CONTEXT_BYTES: usize = 64_000;

/// Marker appended when context was cut to `MAX_CONTEXT_BYTES`.
const TRUNCATED_MARKER: &str = "\n[truncated]";

/// A context item whose text has already been read.
pub struct ResolvedContext {
    pub path: PathBuf,
    pub selection: Option<String>,
    pub content: Option<String>,
}

/// Cuts `text` to at most `max` bytes without splitting a UTF-8 character.
fn truncate_utf8(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Builds the prompt: one fenced block per context item, then the user text.
pub fn build_prompt_text(user_text: &str, contexts: &[ResolvedContext]) -> String {
    let mut out = String::new();
    for ctx in contexts {
        let (kind, body) = match (&ctx.selection, &ctx.content) {
            (Some(selection), _) => ("selection", selection),
            (None, Some(content)) => ("file", content),
            (None, None) => continue,
        };
        let shown = truncate_utf8(body, MAX_CONTEXT_BYTES);
        let marker = if shown.len() < body.len() {
            TRUNCATED_MARKER
        } else {
            ""
        };
        out.push_str(&format!(
            "Context from {} ({kind}):\n```\n{shown}{marker}\n```\n\n",
            ctx.path.display()
        ));
    }
    out.push_str(user_text);
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// Builds a resolved context item for the tests.
    fn ctx(selection: Option<&str>, content: Option<&str>) -> ResolvedContext {
        ResolvedContext {
            path: PathBuf::from("/ws/a.rs"),
            selection: selection.map(str::to_string),
            content: content.map(str::to_string),
        }
    }

    #[test]
    fn no_context_returns_the_user_text_unchanged() {
        assert_eq!(build_prompt_text("fix it", &[]), "fix it");
    }

    #[test]
    fn selection_wins_over_file_content() {
        let text = build_prompt_text(
            "why?",
            &[ctx(Some("let x = 1;"), Some("whole file"))],
        );
        assert!(text.contains("/ws/a.rs (selection)"));
        assert!(text.contains("let x = 1;"));
        assert!(!text.contains("whole file"));
        assert!(text.ends_with("why?"));
    }

    #[test]
    fn file_content_is_used_when_there_is_no_selection() {
        let text = build_prompt_text("why?", &[ctx(None, Some("whole file"))]);
        assert!(text.contains("/ws/a.rs (file)"));
        assert!(text.contains("whole file"));
    }

    #[test]
    fn context_without_any_text_is_skipped() {
        assert_eq!(build_prompt_text("hi", &[ctx(None, None)]), "hi");
    }

    #[test]
    fn huge_content_is_truncated_on_a_char_boundary() {
        // Each 'e' with an accent is 2 bytes, so the limit falls mid-character.
        let content = "\u{e9}".repeat(MAX_CONTEXT_BYTES);
        let text = build_prompt_text("q", &[ctx(None, Some(&content))]);
        assert!(text.contains("[truncated]"));
        assert!(text.len() < MAX_CONTEXT_BYTES + 200);
    }
}
