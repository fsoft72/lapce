//! `@file` mentions in the agent prompt box: detecting the mention being
//! typed, filtering the workspace files and collecting the finished mentions.

use std::path::Path;

use nucleo::{
    Config, Matcher, Utf32Str,
    pattern::{CaseMatching, Normalization, Pattern},
};

/// Directories that never show up in the mention list.
const EXCLUDED_DIRS: [&str; 2] = [".git", "node_modules"];

/// Most files shown in the mention list at once.
pub const MAX_MENTION_ITEMS: usize = 8;

/// A mention being typed: the byte offset of its `@` and the text after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    pub start: usize,
    pub query: String,
}

/// Finds the mention the cursor is in. `before_cursor` is the prompt text up
/// to the cursor: it must end with `@query`, where the `@` starts the text or
/// follows whitespace and `query` has no whitespace.
pub fn mention_at_end(before_cursor: &str) -> Option<Mention> {
    let start = before_cursor.rfind('@')?;
    let query = &before_cursor[start + 1..];
    if query.chars().any(char::is_whitespace) {
        return None;
    }
    let preceded_by_word = before_cursor[..start]
        .chars()
        .next_back()
        .is_some_and(|c| !c.is_whitespace());
    if preceded_by_word {
        return None;
    }
    Some(Mention {
        start,
        query: query.to_string(),
    })
}

/// Whether `path` lies inside an excluded directory such as `.git` or `node_modules`.
pub fn is_excluded(path: &Path) -> bool {
    path.components()
        .any(|part| EXCLUDED_DIRS.iter().any(|dir| part.as_os_str() == *dir))
}

/// The best `limit` files matching `query`, best match first. An empty query
/// keeps the original order.
pub fn filter_files(files: &[String], query: &str, limit: usize) -> Vec<String> {
    if query.is_empty() {
        return files.iter().take(limit).cloned().collect();
    }
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let mut buf = Vec::new();
    let mut scored: Vec<(u32, &String)> = files
        .iter()
        .filter_map(|file| {
            let haystack = Utf32Str::new(file, &mut buf);
            pattern
                .score(haystack, &mut matcher)
                .map(|score| (score, file))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, file)| file.clone())
        .collect()
}

/// The paths mentioned in `text` as `@path`, in order, without duplicates.
pub fn mentioned_paths(text: &str) -> Vec<&str> {
    let mut paths: Vec<&str> = Vec::new();
    for word in text.split_whitespace() {
        let Some(path) = word.strip_prefix('@') else {
            continue;
        };
        if !path.is_empty() && !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_a_mention_at_the_end() {
        assert_eq!(
            mention_at_end("look at @src/ma"),
            Some(Mention {
                start: 8,
                query: "src/ma".to_string()
            })
        );
        assert_eq!(
            mention_at_end("@"),
            Some(Mention {
                start: 0,
                query: String::new()
            })
        );
    }

    #[test]
    fn ignores_text_that_is_not_a_mention() {
        assert_eq!(mention_at_end("no mention"), None);
        assert_eq!(mention_at_end("mail me@example.com"), None);
        assert_eq!(mention_at_end("@done and more"), None);
    }

    #[test]
    fn excludes_git_and_node_modules() {
        assert!(is_excluded(Path::new(".git/config")));
        assert!(is_excluded(Path::new("web/node_modules/a/index.js")));
        assert!(!is_excluded(Path::new("src/git/mod.rs")));
        assert!(!is_excluded(Path::new("src/main.rs")));
    }

    #[test]
    fn filters_and_ranks_files() {
        let files = vec![
            "src/main.rs".to_string(),
            "src/agent.rs".to_string(),
            "README.md".to_string(),
        ];
        assert_eq!(filter_files(&files, "agent", 8), vec!["src/agent.rs"]);
        assert_eq!(filter_files(&files, "", 2).len(), 2);
        assert!(filter_files(&files, "zzz", 8).is_empty());
    }

    #[test]
    fn collects_finished_mentions() {
        assert_eq!(
            mentioned_paths("see @a.rs and @src/b.rs also @a.rs and @"),
            vec!["a.rs", "src/b.rs"]
        );
    }
}
