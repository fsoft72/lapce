//! Debouncing of bursts of events on a background thread.

use std::{
    io,
    sync::mpsc::{self, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};

/// Spawns a thread named `name` that collects the values sent to the
/// returned sender and calls `action` once per burst: when no value arrived
/// for `quiet`, or when the burst is `max_wait` old, so a steady stream of
/// events is still handled. The thread waits on the channel, so it wakes up
/// as soon as an event arrives and no event is ever lost between bursts.
/// It ends after handling the last burst once every sender is dropped.
pub fn spawn_debouncer<T: Send + 'static>(
    name: &str,
    quiet: Duration,
    max_wait: Duration,
    mut action: impl FnMut(Vec<T>) + Send + 'static,
) -> io::Result<Sender<T>> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            while let Ok(first) = rx.recv() {
                let deadline = Instant::now() + max_wait;
                let mut burst = vec![first];
                let disconnected = loop {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        break false;
                    }
                    match rx.recv_timeout(quiet.min(left)) {
                        Ok(value) => burst.push(value),
                        Err(RecvTimeoutError::Timeout) => break false,
                        Err(RecvTimeoutError::Disconnected) => break true,
                    }
                };
                action(burst);
                if disconnected {
                    return;
                }
            }
        })?;
    Ok(tx)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    const QUIET: Duration = Duration::from_millis(50);
    const LONG: Duration = Duration::from_secs(10);

    /// A debouncer that forwards each burst to the returned receiver.
    fn debouncer(max_wait: Duration) -> (Sender<u32>, mpsc::Receiver<Vec<u32>>) {
        let (bursts_tx, bursts) = mpsc::channel();
        let events = spawn_debouncer("test-debouncer", QUIET, max_wait, move |b| {
            bursts_tx.send(b).unwrap();
        })
        .unwrap();
        (events, bursts)
    }

    #[test]
    fn a_burst_is_handled_once_with_every_event() {
        let (events, bursts) = debouncer(LONG);
        for value in 0..5 {
            events.send(value).unwrap();
        }
        assert_eq!(bursts.recv_timeout(LONG).unwrap(), vec![0, 1, 2, 3, 4]);
        assert!(bursts.recv_timeout(QUIET * 4).is_err());
    }

    #[test]
    fn bursts_separated_by_a_quiet_period_are_handled_apart() {
        let (events, bursts) = debouncer(LONG);
        events.send(1).unwrap();
        assert_eq!(bursts.recv_timeout(LONG).unwrap(), vec![1]);
        events.send(2).unwrap();
        assert_eq!(bursts.recv_timeout(LONG).unwrap(), vec![2]);
    }

    #[test]
    fn a_steady_stream_is_cut_at_max_wait() {
        let (events, bursts) = debouncer(QUIET * 3);
        let sender = thread::spawn(move || {
            for value in 0..40 {
                if events.send(value).is_err() {
                    return;
                }
                thread::sleep(QUIET / 5);
            }
        });
        let first = bursts.recv_timeout(LONG).unwrap();
        assert!(first.len() < 40, "the stream was not cut: {first:?}");
        sender.join().unwrap();
    }

    #[test]
    fn dropping_the_sender_flushes_the_last_burst() {
        let (events, bursts) = debouncer(LONG);
        events.send(7).unwrap();
        drop(events);
        assert_eq!(bursts.recv_timeout(LONG).unwrap(), vec![7]);
        assert!(bursts.recv_timeout(LONG).is_err(), "the thread must end");
    }
}
