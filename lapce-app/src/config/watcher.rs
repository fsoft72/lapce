use std::{io, sync::mpsc::Sender, time::Duration};

use lapce_core::debounce::spawn_debouncer;

/// How long the config files must stay unchanged before they are reloaded.
const RELOAD_QUIET_PERIOD: Duration = Duration::from_millis(500);

/// The longest a reload waits while the config files keep changing.
const RELOAD_MAX_WAIT: Duration = Duration::from_secs(2);

/// Watches the config files and asks for one reload per burst of changes.
pub struct ConfigWatcher {
    changes: Sender<()>,
}

impl notify::EventHandler for ConfigWatcher {
    /// Forwards the changes of config files to the debouncer.
    fn handle_event(&mut self, event: notify::Result<notify::Event>) {
        match event {
            Ok(event) => match event.kind {
                notify::EventKind::Create(_)
                | notify::EventKind::Modify(_)
                | notify::EventKind::Remove(_) => {
                    if let Err(err) = self.changes.send(()) {
                        tracing::error!("{:?}", err);
                    }
                }
                _ => {}
            },
            Err(err) => {
                tracing::error!("{:?}", err);
            }
        }
    }
}

impl ConfigWatcher {
    /// Creates a watcher that sends to `tx` once the config files stop
    /// changing. Fails when the debouncer thread cannot be spawned.
    pub fn new(tx: Sender<()>) -> io::Result<Self> {
        let changes = spawn_debouncer(
            "ConfigWatcher",
            RELOAD_QUIET_PERIOD,
            RELOAD_MAX_WAIT,
            move |_| {
                if let Err(err) = tx.send(()) {
                    tracing::error!("{:?}", err);
                }
            },
        )?;
        Ok(Self { changes })
    }
}
