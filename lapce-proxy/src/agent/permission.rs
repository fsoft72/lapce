//! Tracks permission requests that are waiting for a user decision.

use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};

use futures::channel::oneshot;
use lapce_rpc::agent::AgentRequestId;
use parking_lot::Mutex;

/// The user's decision: `Some(option_id)` picks an option, `None` rejects.
type Decision = Option<String>;

/// Hands out request ids and delivers the user's decision to the waiting session.
#[derive(Default)]
pub struct PermissionBroker {
    next_id: AtomicU64,
    pending: Mutex<HashMap<AgentRequestId, oneshot::Sender<Decision>>>,
}

impl PermissionBroker {
    /// Creates an empty broker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a new pending request and returns its id and the receiver to await.
    pub fn register(&self) -> (AgentRequestId, oneshot::Receiver<Decision>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(id, tx);
        (id, rx)
    }

    /// Delivers a decision. Returns `false` if the id is unknown or already answered.
    pub fn reply(&self, id: AgentRequestId, option_id: Decision) -> bool {
        let Some(tx) = self.pending.lock().remove(&id) else {
            return false;
        };
        tx.send(option_id).is_ok()
    }

    /// Drops every pending request so all waiters see a cancellation.
    pub fn cancel_all(&self) {
        self.pending.lock().clear();
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;

    use super::*;

    #[test]
    fn reply_resolves_the_matching_request() {
        let broker = PermissionBroker::new();
        let (id, rx) = broker.register();
        assert!(broker.reply(id, Some("allow".to_string())));
        assert_eq!(block_on(rx), Ok(Some("allow".to_string())));
    }

    #[test]
    fn reply_to_unknown_id_returns_false() {
        let broker = PermissionBroker::new();
        assert!(!broker.reply(99, None));
    }

    #[test]
    fn ids_are_unique() {
        let broker = PermissionBroker::new();
        let (a, _rx_a) = broker.register();
        let (b, _rx_b) = broker.register();
        assert_ne!(a, b);
    }

    #[test]
    fn cancel_all_wakes_every_waiter_with_cancelled() {
        let broker = PermissionBroker::new();
        let (_, rx1) = broker.register();
        let (_, rx2) = broker.register();
        broker.cancel_all();
        assert!(block_on(rx1).is_err());
        assert!(block_on(rx2).is_err());
    }

    #[test]
    fn a_reply_is_delivered_once() {
        let broker = PermissionBroker::new();
        let (id, _rx) = broker.register();
        assert!(broker.reply(id, None));
        assert!(!broker.reply(id, None));
    }
}
