//! Resolves the agent command before it is spawned, and describes it for errors.

use std::path::{Path, PathBuf};

/// Extensions tried on Windows when `PATHEXT` is not set.
#[cfg(windows)]
const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

/// Returns the program to spawn for `command`.
///
/// On Windows, `std::process::Command` only finds `.exe` files on its own, so a
/// bare name such as `npx` (really `npx.cmd`) would not start. A command with
/// no extension that is not a path is looked up in `PATH` with each `PATHEXT`
/// extension, and the first match wins. On other platforms, and when nothing
/// matches, the command is returned unchanged.
pub fn resolve_command(command: &str) -> String {
    #[cfg(windows)]
    {
        let path_dirs = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
            .unwrap_or_default();
        let pathext =
            std::env::var("PATHEXT").unwrap_or_else(|_| DEFAULT_PATHEXT.to_string());
        let extensions = pathext
            .split(';')
            .filter(|ext| !ext.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if let Some(found) =
            find_in_path(command, &path_dirs, &extensions, |path| path.is_file())
        {
            return found.to_string_lossy().to_string();
        }
    }
    command.to_string()
}

/// Looks `command` up in `path_dirs`, trying each of `extensions` appended to
/// the name, in order: directories first, then extensions. Returns the first
/// candidate for which `exists` holds.
///
/// Returns `None` without looking when `command` already has an extension or
/// is a path (it contains a directory separator), because those are spawned
/// as they are.
pub fn find_in_path(
    command: &str,
    path_dirs: &[PathBuf],
    extensions: &[String],
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if command.is_empty() || command.contains(['/', '\\']) {
        return None;
    }
    if Path::new(command).extension().is_some() {
        return None;
    }
    path_dirs.iter().find_map(|dir| {
        extensions
            .iter()
            .map(|ext| dir.join(format!("{command}{ext}")))
            .find(|candidate| exists(candidate))
    })
}

/// Formats a command and its arguments as one line for error messages.
/// Arguments containing whitespace are quoted.
pub fn describe_command(command: &str, args: &[String]) -> String {
    std::iter::once(command)
        .chain(args.iter().map(String::as_str))
        .map(|part| {
            if part.is_empty() || part.contains(char::is_whitespace) {
                format!("\"{part}\"")
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    /// The Windows default extensions, as the lookup receives them.
    fn windows_exts() -> Vec<String> {
        [".COM", ".EXE", ".BAT", ".CMD"]
            .iter()
            .map(|ext| ext.to_string())
            .collect()
    }

    /// An `exists` predicate backed by a fixed set of files.
    fn files(paths: &[&str]) -> impl Fn(&Path) -> bool {
        let set: HashSet<PathBuf> = paths.iter().map(PathBuf::from).collect();
        move |path| set.contains(path)
    }

    #[test]
    fn a_bare_name_finds_the_cmd_shim() {
        let dirs = vec![PathBuf::from("/nodejs")];
        let found =
            find_in_path("npx", &dirs, &windows_exts(), files(&["/nodejs/npx.CMD"]));
        assert_eq!(found, Some(PathBuf::from("/nodejs/npx.CMD")));
    }

    #[test]
    fn earlier_directories_win_over_earlier_extensions() {
        let dirs = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        let found = find_in_path(
            "tool",
            &dirs,
            &windows_exts(),
            files(&["/a/tool.CMD", "/b/tool.EXE"]),
        );
        assert_eq!(found, Some(PathBuf::from("/a/tool.CMD")));
    }

    #[test]
    fn extensions_are_tried_in_order_within_a_directory() {
        let dirs = vec![PathBuf::from("/a")];
        let found = find_in_path(
            "tool",
            &dirs,
            &windows_exts(),
            files(&["/a/tool.CMD", "/a/tool.EXE"]),
        );
        assert_eq!(found, Some(PathBuf::from("/a/tool.EXE")));
    }

    #[test]
    fn a_command_with_an_extension_is_not_looked_up() {
        let dirs = vec![PathBuf::from("/a")];
        let found = find_in_path(
            "tool.exe",
            &dirs,
            &windows_exts(),
            files(&["/a/tool.exe.CMD"]),
        );
        assert_eq!(found, None);
    }

    #[test]
    fn a_path_is_not_looked_up() {
        let dirs = vec![PathBuf::from("/a")];
        for command in ["bin/tool", "bin\\tool", "/usr/bin/tool"] {
            let found =
                find_in_path(command, &dirs, &windows_exts(), |_: &Path| true);
            assert_eq!(found, None, "{command} must be used as is");
        }
    }

    #[test]
    fn no_match_returns_none() {
        let dirs = vec![PathBuf::from("/a")];
        let found = find_in_path("tool", &dirs, &windows_exts(), files(&[]));
        assert_eq!(found, None);
    }

    #[cfg(not(windows))]
    #[test]
    fn resolve_is_the_identity_outside_windows() {
        assert_eq!(resolve_command("npx"), "npx");
        assert_eq!(resolve_command("definitely-missing"), "definitely-missing");
    }

    #[test]
    fn describe_joins_and_quotes_arguments() {
        let args = vec!["-y".to_string(), "a b".to_string()];
        assert_eq!(describe_command("npx", &args), "npx -y \"a b\"");
    }
}
