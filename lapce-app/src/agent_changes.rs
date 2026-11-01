//! The files the agent changed during the current chat, with the text each
//! file had before the first change, so the changes can be reviewed and
//! reverted.

use std::{borrow::Cow, path::PathBuf, sync::atomic::AtomicU64, sync::Arc};

use lapce_core::buffer::diff::{rope_diff, DiffLines};
use lapce_xi_rope::Rope;

/// Largest number of table cells the line diff may use to count the changed
/// lines of a file. Beyond it a file is listed without counts.
const MAX_DIFF_CELLS: usize = 4_000_000;

/// One file changed by the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: PathBuf,
    /// The text before the first change; `None` when the agent created the file.
    pub original: Option<String>,
    /// The text after the latest change.
    pub current: String,
}

impl ChangedFile {
    /// The number of lines added and removed from the original, or `None`
    /// when the file is too big to count them.
    pub fn line_counts(&self) -> Option<(usize, usize)> {
        count_changed_lines(self.original.as_deref().unwrap_or(""), &self.current)
    }
}

/// The files changed by the agent since the chat started or the last review.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangedFiles {
    files: Vec<ChangedFile>,
}

impl ChangedFiles {
    /// The changed files, in the order of their first change.
    pub fn files(&self) -> &[ChangedFile] {
        &self.files
    }

    /// True when the agent changed nothing.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Records a change of `path` to `new_text`. A file keeps the original
    /// of its first change; it leaves the list when it is back to it.
    pub fn record(
        &mut self,
        path: PathBuf,
        old_text: Option<String>,
        new_text: String,
    ) {
        match self.files.iter().position(|file| file.path == path) {
            Some(index) => self.files[index].current = new_text,
            None => self.files.push(ChangedFile {
                path: path.clone(),
                original: old_text,
                current: new_text,
            }),
        }
        self.files.retain(|file| {
            file.path != path || file.original.as_deref() != Some(&file.current)
        });
    }

    /// The file changed at `path`, if any.
    pub fn file(&self, path: &std::path::Path) -> Option<&ChangedFile> {
        self.files.iter().find(|file| file.path == path)
    }

    /// Forgets the change of `path`.
    pub fn remove(&mut self, path: &std::path::Path) {
        self.files.retain(|file| file.path != path);
    }

    /// Forgets every change.
    pub fn clear(&mut self) {
        self.files.clear();
    }
}

/// Counts the lines added and removed going from `old` to `new`. Returns
/// `None` when the part that differs is too big to diff cheaply.
fn count_changed_lines(old: &str, new: &str) -> Option<(usize, usize)> {
    let old_lines: Vec<Cow<str>> = old.lines().map(Cow::Borrowed).collect();
    let new_lines: Vec<Cow<str>> = new.lines().map(Cow::Borrowed).collect();
    let head = old_lines
        .iter()
        .zip(&new_lines)
        .take_while(|(a, b)| a == b)
        .count();
    let tail = old_lines[head..]
        .iter()
        .rev()
        .zip(new_lines[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let old_middle = &old_lines[head..old_lines.len() - tail];
    let new_middle = &new_lines[head..new_lines.len() - tail];
    if (old_middle.len() + 1).saturating_mul(new_middle.len() + 1) > MAX_DIFF_CELLS {
        return None;
    }
    let join = |lines: &[Cow<str>]| {
        lines.iter().fold(String::new(), |mut text, line| {
            text.push_str(line);
            text.push('\n');
            text
        })
    };
    let changes = rope_diff(
        Rope::from(join(old_middle)),
        Rope::from(join(new_middle)),
        0,
        Arc::new(AtomicU64::new(0)),
        None,
    )?;
    Some(
        changes
            .iter()
            .fold((0, 0), |(added, removed), change| match change {
                DiffLines::Right(range) => (added + range.len(), removed),
                DiffLines::Left(range) => (added, removed + range.len()),
                DiffLines::Both(_) => (added, removed),
            }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(files: &mut ChangedFiles, path: &str, old: Option<&str>, new: &str) {
        files.record(path.into(), old.map(String::from), new.to_string());
    }

    #[test]
    fn a_file_keeps_the_original_of_its_first_change() {
        let mut files = ChangedFiles::default();
        record(&mut files, "/w/a", Some("one"), "two");
        record(&mut files, "/w/a", Some("two"), "three");
        assert_eq!(files.files().len(), 1);
        assert_eq!(files.files()[0].original.as_deref(), Some("one"));
        assert_eq!(files.files()[0].current, "three");
    }

    #[test]
    fn a_file_back_to_its_original_leaves_the_list() {
        let mut files = ChangedFiles::default();
        record(&mut files, "/w/a", Some("one"), "two");
        record(&mut files, "/w/a", Some("two"), "one");
        assert!(files.is_empty());
    }

    #[test]
    fn a_created_file_has_no_original() {
        let mut files = ChangedFiles::default();
        record(&mut files, "/w/new", None, "hello\n");
        assert_eq!(files.file("/w/new".as_ref()).unwrap().original, None);
    }

    #[test]
    fn counts_the_added_and_removed_lines() {
        let file = ChangedFile {
            path: "/w/a".into(),
            original: Some("a\nb\nc\nd\n".into()),
            current: "a\nB\nB2\nc\nd\ne\n".into(),
        };
        // "b" replaced by "B" and "B2", "e" appended.
        assert_eq!(file.line_counts(), Some((3, 1)));
    }

    #[test]
    fn a_created_file_counts_all_its_lines_as_added() {
        let file = ChangedFile {
            path: "/w/new".into(),
            original: None,
            current: "x\ny\n".into(),
        };
        assert_eq!(file.line_counts(), Some((2, 0)));
    }

    #[test]
    fn a_huge_difference_is_not_counted() {
        let old = "a\n".repeat(3000);
        let new = "b\n".repeat(3000);
        assert_eq!(count_changed_lines(&old, &new), None);
    }
}
