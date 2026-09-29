//! Logs the raw JSON-RPC lines exchanged with the agent process.
//!
//! Lines are logged at `debug` level with the target `lapce_proxy::agent::wire`,
//! so they end up in the Lapce log file and can be shown on the console with
//! `LAPCE_LOG=lapce_proxy::agent::wire=debug`.

use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{AsyncRead, AsyncWrite};

/// Longest line (in characters) that is logged in full.
const MAX_LOGGED_CHARS: usize = 2000;

/// Which side of the pipe a line travelled on.
#[derive(Clone, Copy)]
pub enum WireDirection {
    /// Lapce to agent (agent stdin).
    ToAgent,
    /// Agent to Lapce (agent stdout).
    FromAgent,
    /// Diagnostic output of the agent (agent stderr).
    Stderr,
}

impl WireDirection {
    /// Short marker written at the start of every logged line.
    fn marker(self) -> &'static str {
        match self {
            WireDirection::ToAgent => "->",
            WireDirection::FromAgent => "<-",
            WireDirection::Stderr => "stderr",
        }
    }
}

/// Splits a byte stream into lines and logs each complete one.
pub struct LineLogger {
    direction: WireDirection,
    pending: Vec<u8>,
}

impl LineLogger {
    /// Creates a logger for one direction.
    pub fn new(direction: WireDirection) -> Self {
        Self {
            direction,
            pending: Vec::new(),
        }
    }

    /// Feeds bytes and logs every line completed by them.
    pub fn feed(&mut self, bytes: &[u8]) {
        for line in self.split_lines(bytes) {
            tracing::debug!(
                target: "lapce_proxy::agent::wire",
                "{} {}",
                self.direction.marker(),
                truncate(&line)
            );
        }
    }

    /// Returns the non-empty lines completed by `bytes`, keeping the rest pending.
    fn split_lines(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for &byte in bytes {
            if byte != b'\n' {
                self.pending.push(byte);
                continue;
            }
            let line = String::from_utf8_lossy(&self.pending).trim().to_string();
            self.pending.clear();
            if !line.is_empty() {
                lines.push(line);
            }
        }
        lines
    }
}

/// Cuts a line to [`MAX_LOGGED_CHARS`] characters, marking the cut.
fn truncate(line: &str) -> String {
    match line.char_indices().nth(MAX_LOGGED_CHARS) {
        Some((end, _)) => format!("{}... [truncated]", &line[..end]),
        None => line.to_string(),
    }
}

/// An [`AsyncRead`] that logs every line it reads.
pub struct LoggedRead<R> {
    inner: R,
    logger: LineLogger,
}

impl<R> LoggedRead<R> {
    /// Wraps `inner`, logging lines with `direction`.
    pub fn new(inner: R, direction: WireDirection) -> Self {
        Self {
            inner,
            logger: LineLogger::new(direction),
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for LoggedRead<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        let poll = Pin::new(&mut this.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(read)) = &poll {
            this.logger.feed(&buf[..*read]);
        }
        poll
    }
}

/// An [`AsyncWrite`] that logs every line written through it.
pub struct LoggedWrite<W> {
    inner: W,
    logger: LineLogger,
}

impl<W> LoggedWrite<W> {
    /// Wraps `inner`, logging lines with `direction`.
    pub fn new(inner: W, direction: WireDirection) -> Self {
        Self {
            inner,
            logger: LineLogger::new(direction),
        }
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for LoggedWrite<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        let poll = Pin::new(&mut this.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(written)) = &poll {
            this.logger.feed(&buf[..*written]);
        }
        poll
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_close(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_split_across_chunks_are_reassembled() {
        let mut logger = LineLogger::new(WireDirection::FromAgent);
        assert!(logger.split_lines(b"{\"a\":").is_empty());
        assert_eq!(
            logger.split_lines(b"1}\n{\"b\":2}\n\n{\"c\""),
            vec!["{\"a\":1}".to_string(), "{\"b\":2}".to_string()]
        );
        assert_eq!(logger.split_lines(b":3}\n"), vec!["{\"c\":3}".to_string()]);
    }

    #[test]
    fn long_lines_are_truncated_on_a_char_boundary() {
        let line = "è".repeat(MAX_LOGGED_CHARS + 10);
        let cut = truncate(&line);
        assert!(cut.ends_with("... [truncated]"));
        assert_eq!(cut.chars().filter(|c| *c == 'è').count(), MAX_LOGGED_CHARS);
        assert_eq!(truncate("short"), "short");
    }
}
