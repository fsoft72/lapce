//! Desktop notifications, shown when something finishes while the window is
//! in the background.

/// Application name the notification is shown under.
const APP_NAME: &str = "dev.lapce.lapce";

/// The `notify-send` arguments of a notification.
#[cfg(unix)]
fn notify_send_args<'a>(title: &'a str, message: &'a str) -> Vec<&'a str> {
    vec!["-a", APP_NAME, "-i", APP_NAME, title, message]
}

/// Shows a desktop notification with `title` and `message`. A missing
/// `notify-send` is only logged: the notification is a convenience.
#[cfg(unix)]
pub fn desktop_notification(title: &str, message: &str) {
    let result = std::process::Command::new("notify-send")
        .args(notify_send_args(title, message))
        .spawn()
        .and_then(|mut child| child.wait());
    if let Err(err) = result {
        tracing::debug!("can't run notify-send for \"{title}\": {err}");
    }
}

/// Desktop notifications are only supported on unix for now.
#[cfg(not(unix))]
pub fn desktop_notification(title: &str, _message: &str) {
    tracing::debug!("no desktop notifications on this platform (\"{title}\")");
}

#[cfg(all(test, unix))]
mod tests {
    use super::notify_send_args;

    #[test]
    fn the_title_and_message_come_last() {
        let args = notify_send_args("Agent finished", "Task completed");
        assert_eq!(
            &args[args.len() - 2..],
            ["Agent finished", "Task completed"]
        );
        assert!(args.contains(&"-a"));
    }
}
