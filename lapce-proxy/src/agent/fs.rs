//! File access for the agent, backed by the dispatcher's synced buffers.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use lapce_rpc::core::{CoreNotification, CoreRpcHandler};

use crate::buffer::Buffer;

/// Reads a text file. If the file is open in the editor, the (possibly unsaved)
/// buffer content is returned; otherwise the file is read from disk.
/// Non UTF-8 files are an error.
pub fn read_text(buffers: &HashMap<PathBuf, Buffer>, path: &Path) -> Result<String> {
    if let Some(buffer) = buffers.get(path) {
        return Ok(buffer.rope.to_string());
    }
    fs::read_to_string(path)
        .with_context(|| format!("cannot read {}", path.display()))
}

/// Writes a text file. If the file is open in the editor, the UI is asked to
/// apply the content as an undoable buffer edit and the disk is left alone.
/// Otherwise parent directories are created and the file is written to disk.
pub fn write_text(
    buffers: &HashMap<PathBuf, Buffer>,
    core_rpc: &CoreRpcHandler,
    path: &Path,
    content: &str,
) -> Result<()> {
    if buffers.contains_key(path) {
        core_rpc.notification(CoreNotification::AgentApplyEdit {
            path: path.to_path_buf(),
            content: content.to_string(),
        });
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    fs::write(path, content)
        .with_context(|| format!("cannot write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, path::PathBuf};

    use lapce_rpc::{
        buffer::BufferId,
        core::{CoreNotification, CoreRpc, CoreRpcHandler},
    };
    use lapce_xi_rope::rope::Rope;

    use super::*;
    use crate::buffer::Buffer;

    /// Creates a fresh temp directory unique to this test.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("lapce-agent-fs-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn read_prefers_the_unsaved_buffer_over_disk() {
        let dir = temp_dir("read-buffer");
        let path = dir.join("a.txt");
        fs::write(&path, "on disk").unwrap();
        let mut buffer = Buffer::new(BufferId::next(), path.clone());
        buffer.rope = Rope::from("unsaved edit");
        let buffers = HashMap::from([(path.clone(), buffer)]);
        assert_eq!(read_text(&buffers, &path).unwrap(), "unsaved edit");
    }

    #[test]
    fn read_falls_back_to_disk() {
        let dir = temp_dir("read-disk");
        let path = dir.join("a.txt");
        fs::write(&path, "on disk").unwrap();
        assert_eq!(read_text(&HashMap::new(), &path).unwrap(), "on disk");
    }

    #[test]
    fn read_of_non_utf8_file_is_an_error_not_a_panic() {
        let dir = temp_dir("non-utf8");
        let path = dir.join("bin.dat");
        fs::write(&path, [0xff, 0xfe, 0x00, 0x80]).unwrap();
        assert!(read_text(&HashMap::new(), &path).is_err());
    }

    #[test]
    fn write_to_closed_file_creates_parents_and_writes_disk() {
        let dir = temp_dir("write-disk");
        let path = dir.join("new/dir/a.txt");
        let core_rpc = CoreRpcHandler::new();
        write_text(&HashMap::new(), &core_rpc, &path, "hello").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
        assert!(core_rpc.rx().try_recv().is_err());
    }

    #[test]
    fn write_to_open_file_goes_to_the_ui_and_leaves_disk_alone() {
        let dir = temp_dir("write-open");
        let path = dir.join("a.txt");
        fs::write(&path, "on disk").unwrap();
        let buffers = HashMap::from([(
            path.clone(),
            Buffer::new(BufferId::next(), path.clone()),
        )]);
        let core_rpc = CoreRpcHandler::new();
        write_text(&buffers, &core_rpc, &path, "from agent").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "on disk");
        match core_rpc.rx().try_recv().unwrap() {
            CoreRpc::Notification(n) => match *n {
                CoreNotification::AgentApplyEdit {
                    path: edited,
                    content,
                } => {
                    assert_eq!(edited, path);
                    assert_eq!(content, "from agent");
                }
                other => panic!("unexpected notification: {other:?}"),
            },
            _ => panic!("expected a notification"),
        }
    }
}
