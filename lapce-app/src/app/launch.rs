//! Application startup: command line, shell environment and single-instance socket.

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::{
    io::{BufReader, IsTerminal, Read, Write},
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        mpsc::{SyncSender, channel, sync_channel},
    },
};

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use floem::{
    ext_event::{create_ext_action, create_signal_from_channel},
    reactive::{
        Scope, SignalGet, SignalUpdate, SignalWith, create_effect, provide_context,
    },
    window::WindowId,
};
use lapce_core::{
    directory::Directory,
    meta,
    syntax::{Syntax, highlight::reset_highlight_configs},
};
use lapce_rpc::{
    RpcMessage,
    core::{CoreMessage, CoreNotification},
    file::PathObject,
};
use notify::Watcher;

use super::{AppData, logging};
use crate::{
    config::{LapceConfig, watcher::ConfigWatcher},
    db::LapceDb,
    listener::Listener,
    tracing::*,
    workspace::LapceWorkspace,
};

#[derive(Parser)]
#[clap(name = "Lapce")]
#[clap(version=meta::VERSION)]
#[derive(Debug)]
struct Cli {
    /// Launch new window even if Lapce is already running
    #[clap(short, long, action)]
    new: bool,
    /// Don't return instantly when opened in a terminal
    #[clap(short, long, action)]
    wait: bool,

    /// Path(s) to plugins to load.
    ///
    /// This is primarily used for plugin development to make it easier to test changes to the
    /// plugin without needing to copy the plugin to the plugins directory.
    /// This will cause any plugin with the same author & name to not run.
    #[clap(long, action)]
    plugin_path: Vec<PathBuf>,

    /// Paths to file(s) and/or folder(s) to open.
    /// When path is a file (that exists or not),
    /// it accepts `path:line:column` syntax
    /// to specify line and column at which it should open the file
    #[clap(value_parser = lapce_proxy::cli::parse_file_line_column)]
    #[clap(value_hint = clap::ValueHint::AnyPath)]
    paths: Vec<PathObject>,
}

pub fn launch() {
    let cli = Cli::parse();

    if !cli.wait {
        logging::panic_hook();
    }

    let (reload_handle, _guard) = logging::logging();
    trace!(TraceLevel::INFO, "Starting up Lapce..");

    #[cfg(feature = "vendored-fonts")]
    {
        use floem::text::{FONT_SYSTEM, fontdb::Source};

        const FONT_DEJAVU_SANS_REGULAR: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../extra/fonts/DejaVu/DejaVuSans.ttf"
        ));
        const FONT_DEJAVU_SANS_MONO_REGULAR: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../extra/fonts/DejaVu/DejaVuSansMono.ttf"
        ));

        FONT_SYSTEM
            .lock()
            .db_mut()
            .load_font_source(Source::Binary(Arc::new(FONT_DEJAVU_SANS_REGULAR)));
        FONT_SYSTEM
            .lock()
            .db_mut()
            .load_font_source(Source::Binary(Arc::new(
                FONT_DEJAVU_SANS_MONO_REGULAR,
            )));
    }

    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        trace!(TraceLevel::INFO, "Loading custom environment from shell");
        load_shell_env();
    }

    // small hack to unblock terminal if launched from it
    // launch it as a separate process that waits
    if !cli.wait {
        let mut args = std::env::args().collect::<Vec<_>>();
        args.push("--wait".to_string());
        let mut cmd = std::process::Command::new(&args[0]);
        #[cfg(target_os = "windows")]
        cmd.creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW);

        let stderr_file_path =
            Directory::logs_directory().unwrap().join("stderr.log");
        let stderr_file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .read(true)
            .open(stderr_file_path)
            .unwrap();
        let stderr = Stdio::from(stderr_file);

        let stdout_file_path =
            Directory::logs_directory().unwrap().join("stdout.log");
        let stdout_file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .read(true)
            .open(stdout_file_path)
            .unwrap();
        let stdout = Stdio::from(stdout_file);

        if let Err(why) = cmd
            .args(&args[1..])
            .stderr(stderr)
            .stdout(stdout)
            .env("LAPCE_LOG", "lapce_app::app=error,off")
            .spawn()
        {
            eprintln!("Failed to launch lapce: {why}");
            std::process::exit(1);
        };
        return;
    }

    // If the cli is not requesting a new window, and we're not developing a plugin, we try to open
    // in the existing Lapce process
    if !cli.new {
        match get_socket() {
            Ok(socket) => {
                if let Err(e) = try_open_in_existing_process(socket, &cli.paths) {
                    trace!(TraceLevel::ERROR, "failed to open path(s): {e}");
                };
                return;
            }
            Err(err) => {
                tracing::error!("{:?}", err);
            }
        }
    }

    #[cfg(feature = "updater")]
    crate::update::cleanup();

    if let Err(err) = lapce_proxy::register_lapce_path() {
        tracing::error!("{:?}", err);
    }
    let db = match LapceDb::new() {
        Ok(db) => Arc::new(db),
        Err(e) => {
            #[cfg(windows)]
            logging::error_modal("Error", &format!("Failed to create LapceDb: {e}"));

            trace!(TraceLevel::ERROR, "Failed to create LapceDb: {e}");
            std::process::exit(1);
        }
    };
    let scope = Scope::new();
    provide_context(db.clone());

    let window_scale = scope.create_rw_signal(1.0);
    let latest_release = scope.create_rw_signal(Arc::new(None));
    let app_command = Listener::new_empty(scope);

    let plugin_paths = Arc::new(cli.plugin_path);

    let (tx, rx) = channel();
    let mut watcher = notify::recommended_watcher(ConfigWatcher::new(tx))
        .context("Failed to spawn file watcher")
        .unwrap();
    if let Some(path) = LapceConfig::settings_file() {
        if let Err(err) = watcher.watch(&path, notify::RecursiveMode::Recursive) {
            tracing::error!("{:?}", err);
        }
    }
    if let Some(path) = Directory::themes_directory() {
        if let Err(err) = watcher.watch(&path, notify::RecursiveMode::Recursive) {
            tracing::error!("{:?}", err);
        }
    }
    if let Some(path) = LapceConfig::keymaps_file() {
        if let Err(err) = watcher.watch(&path, notify::RecursiveMode::Recursive) {
            tracing::error!("{:?}", err);
        }
    }
    if let Some(path) = Directory::plugins_directory() {
        if let Err(err) = watcher.watch(&path, notify::RecursiveMode::Recursive) {
            tracing::error!("{:?}", err);
        }
    }

    let windows = scope.create_rw_signal(im::HashMap::new());
    let config = LapceConfig::load(&LapceWorkspace::default(), &[], &plugin_paths);

    // Restore scale from config
    window_scale.set(config.ui.scale());

    let config = scope.create_rw_signal(Arc::new(config));
    let app_data = AppData {
        windows,
        active_window: scope.create_rw_signal(WindowId::from_raw(0)),
        window_scale,
        app_terminated: scope.create_rw_signal(false),
        watcher: Arc::new(watcher),
        latest_release,
        app_command,
        tracing_handle: reload_handle,
        config,
        plugin_paths,
    };

    let app = app_data.create_windows(db.clone(), cli.paths);

    {
        let app_data = app_data.clone();
        let notification = create_signal_from_channel(rx);
        create_effect(move |_| {
            if notification.get().is_some() {
                tracing::debug!("notification reload_config");
                app_data.reload_config();
            }
        });
    }

    {
        let cx = Scope::new();
        let app_data = app_data.clone();
        let send = create_ext_action(cx, move |updated| {
            if updated {
                trace!(
                    TraceLevel::INFO,
                    "grammar or query got updated, reset highlight configs"
                );
                reset_highlight_configs();
                for (_, window) in app_data.windows.get_untracked() {
                    for (_, tab) in window.window_tabs.get_untracked() {
                        for (_, doc) in tab.main_split.docs.get_untracked() {
                            doc.syntax.update(|syntax| {
                                *syntax = Syntax::from_language(syntax.language);
                            });
                            doc.trigger_syntax_change(None);
                        }
                    }
                }
            }
        });
        std::thread::Builder::new()
            .name("FindGrammar".to_owned())
            .spawn(move || {
                use super::grammars::*;
                let updated = match find_grammar_release() {
                    Ok(release) => {
                        let mut updated = false;
                        match fetch_grammars(&release) {
                            Err(e) => {
                                trace!(
                                    TraceLevel::ERROR,
                                    "failed to fetch grammars: {e}"
                                );
                            }
                            Ok(u) => updated |= u,
                        }
                        match fetch_queries(&release) {
                            Err(e) => {
                                trace!(
                                    TraceLevel::ERROR,
                                    "failed to fetch grammars: {e}"
                                );
                            }
                            Ok(u) => updated |= u,
                        }
                        updated
                    }
                    Err(e) => {
                        trace!(
                            TraceLevel::ERROR,
                            "failed to obtain release info: {e}"
                        );
                        false
                    }
                };
                send(updated);
            })
            .unwrap();
    }

    #[cfg(feature = "updater")]
    {
        let (tx, rx) = sync_channel(1);
        let notification = create_signal_from_channel(rx);
        let latest_release = app_data.latest_release;
        create_effect(move |_| {
            if let Some(release) = notification.get() {
                latest_release.set(Arc::new(Some(release)));
            }
        });
        std::thread::Builder::new()
            .name("LapceUpdater".to_owned())
            .spawn(move || {
                loop {
                    if let Ok(release) = crate::update::get_latest_release() {
                        if let Err(err) = tx.send(release) {
                            tracing::error!("{:?}", err);
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_secs(60 * 60));
                }
            })
            .unwrap();
    }

    {
        let (tx, rx) = sync_channel(1);
        let notification = create_signal_from_channel(rx);
        let app_data = app_data.clone();
        create_effect(move |_| {
            if let Some(CoreNotification::OpenPaths { paths }) = notification.get() {
                if let Some(window_tab) = app_data.active_window_tab() {
                    window_tab.open_paths(&paths);
                    // focus window after open doc
                    floem::action::focus_window();
                }
            }
        });
        std::thread::Builder::new()
            .name("ListenLocalSocket".to_owned())
            .spawn(move || {
                if let Err(err) = listen_local_socket(tx) {
                    tracing::error!("{:?}", err);
                }
            })
            .unwrap();
    }

    {
        let app_data = app_data.clone();
        app_data.app_command.listen(move |command| {
            app_data.run_app_command(command);
        });
    }

    app.on_event(move |event| match event {
        floem::AppEvent::WillTerminate => {
            app_data.app_terminated.set(true);
            app_data.windows.with_untracked(|windows| {
                for window_data in windows.values() {
                    window_data.stop_agents();
                }
            });
            if let Err(err) = db.insert_app(app_data.clone()) {
                tracing::error!("{:?}", err);
            }
        }
        floem::AppEvent::Reopen {
            has_visible_windows,
        } => {
            if !has_visible_windows {
                app_data.new_window(None);
            }
        }
    })
    .run();
}

/// Uses a login shell to load the correct shell environment for the current user.
pub fn load_shell_env() {
    use std::process::Command;

    use tracing::warn;

    #[cfg(not(windows))]
    let shell = match std::env::var("SHELL") {
        Ok(s) => s,
        Err(error) => {
            // Shell variable is not set, so we can't determine the correct shell executable.
            trace!(
                TraceLevel::ERROR,
                "Failed to obtain shell environment: {error}"
            );
            return;
        }
    };

    #[cfg(windows)]
    let shell = "powershell";

    let mut command = Command::new(shell);

    #[cfg(not(windows))]
    command.args(["--login", "-c", "printenv"]);

    #[cfg(windows)]
    command.args([
        "-Command",
        "Get-ChildItem env: | ForEach-Object { \"{0}={1}\" -f $_.Name, $_.Value }",
    ]);

    #[cfg(windows)]
    command.creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW);

    let env = match command.output() {
        Ok(output) => String::from_utf8(output.stdout).unwrap_or_default(),

        Err(error) => {
            trace!(
                TraceLevel::ERROR,
                "Failed to obtain shell environment: {error}"
            );
            return;
        }
    };

    env.split('\n')
        .filter_map(|line| line.split_once('='))
        .for_each(|(key, value)| unsafe {
            let value = value.trim_matches('\r');
            if let Ok(v) = std::env::var(key) {
                if v != value {
                    warn!("Overwriting '{key}', previous value: '{v}', new value '{value}'");
                }
            };
            std::env::set_var(key, value);
        })
}

pub fn get_socket() -> Result<interprocess::local_socket::LocalSocketStream> {
    let local_socket = Directory::local_socket()
        .ok_or_else(|| anyhow!("can't get local socket folder"))?;
    let socket =
        interprocess::local_socket::LocalSocketStream::connect(local_socket)?;
    Ok(socket)
}

pub fn try_open_in_existing_process(
    mut socket: interprocess::local_socket::LocalSocketStream,
    paths: &[PathObject],
) -> Result<()> {
    let msg: CoreMessage = RpcMessage::Notification(CoreNotification::OpenPaths {
        paths: paths.to_vec(),
    });
    lapce_rpc::stdio::write_msg(&mut socket, msg)?;

    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::spawn(move || {
        let mut buf = [0; 100];
        let received = if let Ok(n) = socket.read(&mut buf) {
            &buf[..n] == b"received"
        } else {
            false
        };
        tx.send(received)
    });

    let received = rx.recv_timeout(std::time::Duration::from_millis(500))?;
    if !received {
        return Err(anyhow!("didn't receive response"));
    }

    Ok(())
}

fn listen_local_socket(tx: SyncSender<CoreNotification>) -> Result<()> {
    let local_socket = Directory::local_socket()
        .ok_or_else(|| anyhow!("can't get local socket folder"))?;
    if local_socket.exists() {
        if let Err(err) = std::fs::remove_file(&local_socket) {
            tracing::error!("{:?}", err);
        }
    }
    let socket =
        interprocess::local_socket::LocalSocketListener::bind(local_socket)?;

    for stream in socket.incoming().flatten() {
        let tx = tx.clone();
        std::thread::spawn(move || -> Result<()> {
            let mut reader = BufReader::new(stream);
            loop {
                let msg: Option<CoreMessage> =
                    lapce_rpc::stdio::read_msg(&mut reader)?;

                if let Some(RpcMessage::Notification(msg)) = msg {
                    tx.send(msg)?;
                } else {
                    trace!(TraceLevel::ERROR, "Unhandled message: {msg:?}");
                }

                let stream_ref = reader.get_mut();
                if let Err(err) = stream_ref.write_all(b"received") {
                    tracing::error!("{:?}", err);
                }
                if let Err(err) = stream_ref.flush() {
                    tracing::error!("{:?}", err);
                }
            }
        });
    }
    Ok(())
}
