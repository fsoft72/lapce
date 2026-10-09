use std::path::PathBuf;

use lapce_core::directory::Directory;
use tracing::level_filters::LevelFilter;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{filter::Targets, reload::Handle};

use crate::tracing::*;

/// Name of the log file that records the communication with the AI agent.
const AGENT_LOG_FILE_NAME: &str = "lapce-agent.log";

/// Environment variable naming the directory of the agent log. Without it
/// the log goes to the Lapce logs directory.
const AGENT_LOG_DIR_ENV: &str = "LAPCE_AGENT_LOG_DIR";

#[inline(always)]
pub(super) fn logging() -> (Handle<Targets>, Option<WorkerGuard>) {
    use tracing_subscriber::{filter, fmt, prelude::*, reload};

    let (log_file, guard) = match Directory::logs_directory()
        .and_then(|dir| {
            tracing_appender::rolling::Builder::new()
                .max_log_files(10)
                .rotation(tracing_appender::rolling::Rotation::DAILY)
                .filename_prefix("lapce")
                .filename_suffix("log")
                .build(dir)
                .ok()
        })
        .map(tracing_appender::non_blocking)
    {
        Some((log_file, guard)) => (Some(log_file), Some(guard)),
        None => (None, None),
    };

    let log_file_filter_targets = filter::Targets::new()
        .with_target("lapce_app", LevelFilter::DEBUG)
        .with_target("lapce_proxy", LevelFilter::DEBUG)
        .with_target("lapce_core", LevelFilter::DEBUG)
        .with_default(LevelFilter::from_level(TraceLevel::INFO));
    let (log_file_filter, reload_handle) =
        reload::Subscriber::new(log_file_filter_targets);

    let console_filter_targets = std::env::var("LAPCE_LOG")
        .unwrap_or_default()
        .parse::<filter::Targets>()
        .unwrap_or_default();

    let agent_layer = agent_log_file().map(|file| {
        tracing_subscriber::fmt::subscriber()
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(file))
            .with_filter(
                filter::Targets::new()
                    .with_target("lapce_proxy::agent", LevelFilter::DEBUG),
            )
    });

    let registry = tracing_subscriber::registry().with(agent_layer);
    if let Some(log_file) = log_file {
        let file_layer = tracing_subscriber::fmt::subscriber()
            .with_ansi(false)
            .with_writer(log_file)
            .with_filter(log_file_filter);
        registry
            .with(file_layer)
            .with(
                fmt::Subscriber::default()
                    .with_line_number(true)
                    .with_target(true)
                    .with_thread_names(true)
                    .with_filter(console_filter_targets),
            )
            .init();
    } else {
        registry
            .with(fmt::Subscriber::default().with_filter(console_filter_targets))
            .init();
    };

    (reload_handle, guard)
}

/// Where the agent log is written: in the directory named by
/// [`AGENT_LOG_DIR_ENV`] when set and not empty, else in `logs_dir`, else in
/// the current working directory.
fn agent_log_path(env_dir: Option<String>, logs_dir: Option<PathBuf>) -> PathBuf {
    let dir = env_dir
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or(logs_dir)
        .unwrap_or_default();
    dir.join(AGENT_LOG_FILE_NAME)
}

/// Opens (appending) the agent log, see [`agent_log_path`]. Returns `None`,
/// after a note on stderr, when it cannot be opened.
fn agent_log_file() -> Option<std::fs::File> {
    let path = agent_log_path(
        std::env::var(AGENT_LOG_DIR_ENV).ok(),
        Directory::logs_directory(),
    );
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| eprintln!("cannot open {}: {err}", path.display()))
        .ok()
}

pub(super) fn panic_hook() {
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let thread = thread.name().unwrap_or("main");
        let backtrace = backtrace::Backtrace::new();

        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s
        } else {
            "<unknown>"
        };

        match info.location() {
            Some(loc) => {
                trace!(
                    target: "lapce_app::panic_hook",
                    TraceLevel::ERROR,
                    "thread {thread} panicked at {} | file://./{}:{}:{}\n{:?}",
                    payload,
                    loc.file(), loc.line(), loc.column(),
                    backtrace,
                );
            }
            None => {
                trace!(
                    target: "lapce_app::panic_hook",
                    TraceLevel::ERROR,
                    "thread {thread} panicked at {}\n{:?}",
                    payload,
                    backtrace,
                );
            }
        }

        #[cfg(windows)]
        error_modal("Error", &info.to_string());

        #[cfg(unix)]
        error_notification("Error", &info.to_string());
    }))
}

#[cfg(windows)]
pub(super) fn error_modal(title: &str, msg: &str) -> i32 {
    use std::{ffi::OsStr, iter::once, mem, os::windows::prelude::OsStrExt};

    use windows::Win32::UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_SYSTEMMODAL, MessageBoxW,
    };

    let result: i32;

    let title = OsStr::new(title)
        .encode_wide()
        .chain(once(0u16))
        .collect::<Vec<u16>>();
    let msg = OsStr::new(msg)
        .encode_wide()
        .chain(once(0u16))
        .collect::<Vec<u16>>();
    unsafe {
        result = MessageBoxW(
            mem::zeroed(),
            msg.as_ptr(),
            title.as_ptr(),
            MB_ICONERROR | MB_SYSTEMMODAL,
        );
    }

    result
}

/// Shows a desktop notification for an error. A missing `notify-send` is
/// only logged, since this already reports another failure.
#[cfg(unix)]
pub fn error_notification(title: &str, msg: &str) {
    let result = std::process::Command::new("notify-send")
        .args([
            "-a",
            "dev.lapce.lapce",
            "-w",
            "-n",
            "dev.lapce.lapce",
            "-c",
            "error",
            title,
            msg,
        ])
        .spawn();
    if let Err(err) = result {
        tracing::debug!("can't run notify-send for \"{title}\": {err}");
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{AGENT_LOG_FILE_NAME, agent_log_path};

    #[test]
    fn the_agent_log_goes_to_the_logs_directory_by_default() {
        let logs = PathBuf::from("/data/lapce/logs");
        assert_eq!(
            agent_log_path(None, Some(logs.clone())),
            logs.join(AGENT_LOG_FILE_NAME)
        );
        assert_eq!(
            agent_log_path(Some(String::new()), Some(logs.clone())),
            logs.join(AGENT_LOG_FILE_NAME)
        );
    }

    #[test]
    fn the_environment_variable_chooses_the_agent_log_directory() {
        assert_eq!(
            agent_log_path(Some(".".to_string()), Some(PathBuf::from("/logs"))),
            PathBuf::from(".").join(AGENT_LOG_FILE_NAME)
        );
    }

    #[test]
    fn without_a_logs_directory_the_agent_log_uses_the_working_directory() {
        assert_eq!(
            agent_log_path(None, None),
            PathBuf::from(AGENT_LOG_FILE_NAME)
        );
    }
}
