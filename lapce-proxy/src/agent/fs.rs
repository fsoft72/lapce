//! File access for the agent, backed by the dispatcher's synced buffers.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use lapce_rpc::core::{CoreNotification, CoreRpcHandler};
use lapce_xi_rope::Rope;

use crate::buffer::Buffer;

/// Agent writes to open files that the UI has not applied yet, by path.
///
/// A write to an open file is only a request to the UI; the proxy's copy of
/// the buffer changes when the UI's `Update` comes back. Until then a read
/// must see the written content, or an agent that reads right after writing
/// would build its next write on the old text and lose the first edit.
pub type PendingWrites = HashMap<PathBuf, String>;

/// Reads a text file. A pending agent write wins; then, if the file is open
/// in the editor, the (possibly unsaved) buffer content is returned;
/// otherwise the file is read from disk. Non UTF-8 files are an error.
pub fn read_text(
    buffers: &HashMap<PathBuf, Buffer>,
    pending: &PendingWrites,
    path: &Path,
) -> Result<String> {
    if let Some(content) = pending.get(path) {
        return Ok(content.clone());
    }
    if let Some(buffer) = buffers.get(path) {
        return Ok(buffer.rope.to_string());
    }
    fs::read_to_string(path)
        .with_context(|| format!("cannot read {}", path.display()))
}

/// Writes a text file. If the file is open in the editor, the UI is asked to
/// apply the content as an undoable buffer edit (and to save it when the
/// buffer had no unsaved changes), and the content is remembered in `pending`
/// until the UI's update arrives. A read-only open file is refused.
/// Otherwise parent directories are created and the file is written to disk.
pub fn write_text(
    buffers: &HashMap<PathBuf, Buffer>,
    pending: &mut PendingWrites,
    core_rpc: &CoreRpcHandler,
    path: &Path,
    content: &str,
) -> Result<()> {
    if let Some(buffer) = buffers.get(path) {
        if buffer.read_only {
            bail!("{} is read-only", path.display());
        }
        if buffer.rope.to_string() == content {
            pending.remove(path);
        } else {
            pending.insert(path.to_path_buf(), content.to_string());
        }
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

/// Called after the buffer at `path` changed: forgets the pending agent
/// write once the buffer holds exactly the written content, which means
/// the UI applied it. Other updates (the user typing before the agent edit
/// landed) keep it pending.
pub fn settle_pending_write(pending: &mut PendingWrites, path: &Path, rope: &Rope) {
    let Some(content) = pending.get(path) else {
        return;
    };
    if rope.to_string() == *content {
        pending.remove(path);
    }
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
        assert_eq!(
            read_text(&buffers, &PendingWrites::new(), &path).unwrap(),
            "unsaved edit"
        );
    }

    #[test]
    fn read_falls_back_to_disk() {
        let dir = temp_dir("read-disk");
        let path = dir.join("a.txt");
        fs::write(&path, "on disk").unwrap();
        assert_eq!(
            read_text(&HashMap::new(), &PendingWrites::new(), &path).unwrap(),
            "on disk"
        );
    }

    #[test]
    fn read_of_non_utf8_file_is_an_error_not_a_panic() {
        let dir = temp_dir("non-utf8");
        let path = dir.join("bin.dat");
        fs::write(&path, [0xff, 0xfe, 0x00, 0x80]).unwrap();
        assert!(read_text(&HashMap::new(), &PendingWrites::new(), &path).is_err());
    }

    #[test]
    fn write_to_closed_file_creates_parents_and_writes_disk() {
        let dir = temp_dir("write-disk");
        let path = dir.join("new/dir/a.txt");
        let core_rpc = CoreRpcHandler::new();
        write_text(
            &HashMap::new(),
            &mut PendingWrites::new(),
            &core_rpc,
            &path,
            "hello",
        )
        .unwrap();
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
        write_text(
            &buffers,
            &mut PendingWrites::new(),
            &core_rpc,
            &path,
            "from agent",
        )
        .unwrap();
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

    /// An open buffer for `path` holding `text`.
    fn open_buffer(path: &Path, text: &str) -> Buffer {
        let mut buffer = Buffer::new(BufferId::next(), path.to_path_buf());
        buffer.rope = Rope::from(text);
        buffer.read_only = false;
        buffer
    }

    #[test]
    fn a_read_right_after_a_write_to_an_open_file_sees_the_write() {
        let dir = temp_dir("pending-read");
        let path = dir.join("a.txt");
        let buffers = HashMap::from([(path.clone(), open_buffer(&path, "old"))]);
        let mut pending = PendingWrites::new();
        let core_rpc = CoreRpcHandler::new();
        write_text(&buffers, &mut pending, &core_rpc, &path, "first").unwrap();
        assert_eq!(read_text(&buffers, &pending, &path).unwrap(), "first");
    }

    #[test]
    fn back_to_back_writes_keep_both_edits() {
        let dir = temp_dir("pending-twice");
        let path = dir.join("a.txt");
        let buffers = HashMap::from([(path.clone(), open_buffer(&path, "a\n"))]);
        let mut pending = PendingWrites::new();
        let core_rpc = CoreRpcHandler::new();
        // The agent appends a line, reads the file back, appends another.
        let first = format!("{}b\n", read_text(&buffers, &pending, &path).unwrap());
        write_text(&buffers, &mut pending, &core_rpc, &path, &first).unwrap();
        let second = format!("{}c\n", read_text(&buffers, &pending, &path).unwrap());
        write_text(&buffers, &mut pending, &core_rpc, &path, &second).unwrap();
        assert_eq!(read_text(&buffers, &pending, &path).unwrap(), "a\nb\nc\n");
    }

    #[test]
    fn the_pending_write_is_settled_once_the_buffer_holds_it() {
        let dir = temp_dir("pending-settle");
        let path = dir.join("a.txt");
        let mut buffers = HashMap::from([(path.clone(), open_buffer(&path, "old"))]);
        let mut pending = PendingWrites::new();
        let core_rpc = CoreRpcHandler::new();
        write_text(&buffers, &mut pending, &core_rpc, &path, "new").unwrap();

        // A user keystroke that reached the proxy first does not settle it.
        buffers.get_mut(&path).unwrap().rope = Rope::from("old!");
        settle_pending_write(&mut pending, &path, &buffers[&path].rope);
        assert_eq!(read_text(&buffers, &pending, &path).unwrap(), "new");

        // The UI's update carrying the agent edit does.
        buffers.get_mut(&path).unwrap().rope = Rope::from("new");
        settle_pending_write(&mut pending, &path, &buffers[&path].rope);
        assert!(pending.is_empty());

        // Later edits by the user are visible again.
        buffers.get_mut(&path).unwrap().rope = Rope::from("new, edited");
        assert_eq!(read_text(&buffers, &pending, &path).unwrap(), "new, edited");
    }

    #[test]
    fn a_write_that_changes_nothing_is_not_left_pending() {
        let dir = temp_dir("pending-same");
        let path = dir.join("a.txt");
        let buffers = HashMap::from([(path.clone(), open_buffer(&path, "same"))]);
        let mut pending = PendingWrites::new();
        let core_rpc = CoreRpcHandler::new();
        write_text(&buffers, &mut pending, &core_rpc, &path, "same").unwrap();
        assert!(pending.is_empty());
    }

    #[test]
    fn a_write_to_a_read_only_open_file_is_refused() {
        let dir = temp_dir("read-only");
        let path = dir.join("a.txt");
        let mut buffer = open_buffer(&path, "old");
        buffer.read_only = true;
        let buffers = HashMap::from([(path.clone(), buffer)]);
        let mut pending = PendingWrites::new();
        let core_rpc = CoreRpcHandler::new();
        let err =
            write_text(&buffers, &mut pending, &core_rpc, &path, "new").unwrap_err();
        assert!(format!("{err:#}").contains("read-only"));
        assert!(pending.is_empty());
        assert!(
            core_rpc.rx().try_recv().is_err(),
            "the UI must not be asked"
        );
    }
}
