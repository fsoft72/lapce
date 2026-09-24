//! Path safety for file requests coming from the agent, and line slicing.

use std::path::{Component, Path, PathBuf};

use anyhow::{Result, bail};

/// Normalizes `path` lexically (no filesystem access): drops `.` and resolves `..`.
/// Fails if a `..` would climb above the root.
fn normalize(path: &Path) -> Result<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    bail!("path climbs above the root: {}", path.display());
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    Ok(out)
}

/// Returns the real location of `path`: the deepest existing ancestor is
/// canonicalized (symlinks resolved) and the missing tail is appended as is.
fn canonical_prefix(path: &Path) -> PathBuf {
    let mut tail = Vec::new();
    let mut current = path;
    loop {
        if let Ok(real) = current.canonicalize() {
            return tail.iter().rev().fold(real, |acc, name| acc.join(name));
        }
        let (Some(parent), Some(name)) = (current.parent(), current.file_name())
        else {
            return path.to_path_buf();
        };
        tail.push(name.to_owned());
        current = parent;
    }
}

/// Validates a path sent by the agent and returns its normalized absolute form.
///
/// The path must be absolute, must stay inside `workspace` after normalization,
/// and its real location (symlinks resolved) must also stay inside the real
/// workspace directory.
pub fn resolve_workspace_path(workspace: &Path, path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("path must be absolute: {}", path.display());
    }
    let target = normalize(path)?;
    let root = normalize(workspace)?;
    if !target.starts_with(&root) {
        bail!("path is outside the workspace: {}", path.display());
    }
    if !canonical_prefix(&target).starts_with(canonical_prefix(&root)) {
        bail!(
            "path escapes the workspace through a link: {}",
            path.display()
        );
    }
    Ok(target)
}

/// Returns `limit` lines of `content` starting at 1-based `line`.
/// `line == 0` counts as the first line; missing bounds mean "no bound".
pub fn slice_lines(content: &str, line: Option<u32>, limit: Option<u32>) -> String {
    if line.is_none() && limit.is_none() {
        return content.to_string();
    }
    let skip = line.unwrap_or(1).saturating_sub(1) as usize;
    let take = limit.map_or(usize::MAX, |l| l as usize);
    content
        .split_inclusive('\n')
        .skip(skip)
        .take(take)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::*;

    /// Creates a fresh temp directory unique to this test.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("lapce-agent-paths-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn accepts_a_file_inside_the_workspace() {
        let ws = temp_dir("inside");
        let ok = resolve_workspace_path(&ws, &ws.join("src/main.rs")).unwrap();
        assert_eq!(ok, ws.join("src/main.rs"));
    }

    #[test]
    fn rejects_relative_paths() {
        let ws = temp_dir("relative");
        assert!(resolve_workspace_path(&ws, Path::new("src/main.rs")).is_err());
    }

    #[test]
    fn rejects_parent_dir_escape() {
        let ws = temp_dir("dotdot");
        let escape = ws.join("../outside.txt");
        assert!(resolve_workspace_path(&ws, &escape).is_err());
    }

    #[test]
    fn rejects_sibling_directory_sharing_the_prefix() {
        let base = temp_dir("sibling");
        let ws = base.join("ws");
        let evil = base.join("ws-evil");
        fs::create_dir_all(&ws).unwrap();
        fs::create_dir_all(&evil).unwrap();
        assert!(resolve_workspace_path(&ws, &evil.join("a.txt")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        let base = temp_dir("symlink");
        let ws = base.join("ws");
        let outside = base.join("outside");
        fs::create_dir_all(&ws).unwrap();
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, ws.join("link")).unwrap();
        assert!(resolve_workspace_path(&ws, &ws.join("link/secret.txt")).is_err());
    }

    #[test]
    fn slice_lines_returns_everything_without_bounds() {
        assert_eq!(slice_lines("a\nb\nc\n", None, None), "a\nb\nc\n");
    }

    #[test]
    fn slice_lines_applies_line_and_limit() {
        assert_eq!(slice_lines("a\nb\nc\nd\n", Some(2), Some(2)), "b\nc\n");
        assert_eq!(slice_lines("a\nb\nc", Some(3), None), "c");
    }

    #[test]
    fn slice_lines_treats_zero_as_first_line_and_past_end_as_empty() {
        assert_eq!(slice_lines("a\nb\n", Some(0), Some(1)), "a\n");
        assert_eq!(slice_lines("a\nb\n", Some(9), None), "");
    }
}
