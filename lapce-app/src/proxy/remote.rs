use std::{
    io::BufReader,
    process::{Command, Stdio},
};

use anyhow::{Result, anyhow};
use lapce_core::meta::{self, ReleaseType};
use lapce_rpc::{
    RpcMessage,
    core::CoreRpcHandler,
    proxy::{ProxyRpc, ProxyRpcHandler},
    stdio_transport,
};
use thiserror::Error;
use tracing::{debug, error};

#[derive(Clone, Copy, Error, Debug, PartialEq, Eq, strum_macros::Display)]
#[strum(ascii_case_insensitive)]
enum HostPlatform {
    UnknownOS,
    #[strum(serialize = "windows")]
    Windows,
    #[strum(serialize = "linux")]
    Linux,
    #[strum(serialize = "darwin")]
    Darwin,
    #[strum(serialize = "bsd")]
    Bsd,
}

/// serialise via strum to arch name that is used
/// in CI artefacts
#[derive(Clone, Copy, Error, Debug, PartialEq, Eq, strum_macros::Display)]
#[strum(ascii_case_insensitive)]
enum HostArchitecture {
    UnknownArch,
    #[strum(serialize = "x86_64")]
    AMD64,
    #[strum(serialize = "x86")]
    X86,
    #[strum(serialize = "aarch64")]
    ARM64,
    #[strum(serialize = "armv7")]
    ARM32v7,
    #[strum(serialize = "armhf")]
    ARM32v6,
}

pub trait Remote: Sized {
    #[allow(unused)]
    fn home_dir(&self) -> Result<String> {
        let cmd = self
            .command_builder()
            .arg("echo")
            .arg("-n")
            .arg("$HOME")
            .stdout(Stdio::piped())
            .output()?;

        Ok(String::from_utf8(cmd.stdout)?)
    }

    fn command_builder(&self) -> Command;
}

pub fn start_remote(
    remote: impl Remote,
    core_rpc: CoreRpcHandler,
    proxy_rpc: ProxyRpcHandler,
) -> Result<()> {
    // Note about platforms:
    // Windows can use either cmd.exe, powershell.exe or pwsh.exe as
    // SSH shell, syntax logic varies significantly that's why we bet on
    // cmd.exe as it doesn't add unwanted newlines and use powershell only
    // for proxy install
    //
    // Unix-like systems due to POSIX, always have /bin/sh which should not
    // be necessary to use explicitly most of the time, as many wide-spread
    // shells retain similar syntax, although shells like Nushell might not
    // work (hopefully no one uses it as login shell)
    use HostPlatform::*;
    let (platform, architecture) = host_specification(&remote).unwrap();

    if platform == UnknownOS || architecture == HostArchitecture::UnknownArch {
        error!("detected remote host: {platform}/{architecture}");
        return Err(anyhow!("Unknown OS and/or architecture"));
    }

    // ! Below paths have to be synced with what is
    // ! returned by Config::proxy_directory()
    let remote_proxy_path = match platform {
        Windows => format!(
            "%HOMEDRIVE%%HOMEPATH%\\AppData\\Local\\lapce\\{}\\data\\proxy",
            meta::NAME
        ),
        Darwin => format!(
            "~/Library/Application\\ Support/dev.lapce.{}/proxy",
            meta::NAME
        ),
        _ => {
            format!("~/.local/share/{}/proxy", meta::NAME.to_lowercase())
        }
    };

    let remote_proxy_file = match platform {
        Windows => format!("{remote_proxy_path}\\lapce.exe"),
        _ => format!("{remote_proxy_path}/lapce"),
    };

    if !remote
        .command_builder()
        .args([&remote_proxy_file, "--version"])
        .output()
        .map(|output| {
            if meta::RELEASE == ReleaseType::Debug {
                String::from_utf8_lossy(&output.stdout).starts_with("Lapce-proxy")
            } else {
                String::from_utf8_lossy(&output.stdout).trim()
                    == format!("Lapce-proxy {}", meta::VERSION)
            }
        })
        .unwrap_or(false)
    {
        return Err(anyhow!(
            "remote proxy not found at {remote_proxy_file} (platform: {platform}, architecture: {architecture}); automatic download is disabled, install it manually"
        ));
    };

    debug!("remote proxy path: {remote_proxy_path}");

    let mut child = match platform {
        // Force cmd.exe usage to resolve %envvar% variables
        Windows => remote
            .command_builder()
            .args(["cmd", "/c"])
            .arg(&remote_proxy_file)
            .arg("--proxy")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?,
        _ => remote
            .command_builder()
            .arg(&remote_proxy_file)
            .arg("--proxy")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?,
    };
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("can't find stdin"))?;
    let stdout = BufReader::new(
        child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("can't find stdout"))?,
    );
    debug!("process id: {}", child.id());

    let (writer_tx, writer_rx) = crossbeam_channel::unbounded();
    let (reader_tx, reader_rx) = crossbeam_channel::unbounded();
    stdio_transport(stdin, writer_rx, stdout, reader_tx);

    let local_proxy_rpc = proxy_rpc.clone();
    let local_writer_tx = writer_tx.clone();
    std::thread::Builder::new()
        .name("ProxyRpcHandler".to_owned())
        .spawn(move || {
            for msg in local_proxy_rpc.rx() {
                match msg {
                    ProxyRpc::Request(id, rpc) => {
                        if let Err(err) =
                            local_writer_tx.send(RpcMessage::Request(id, rpc))
                        {
                            tracing::error!("{:?}", err);
                        }
                    }
                    ProxyRpc::Notification(rpc) => {
                        if let Err(err) =
                            local_writer_tx.send(RpcMessage::Notification(rpc))
                        {
                            tracing::error!("{:?}", err);
                        }
                    }
                    ProxyRpc::Shutdown => {
                        if let Err(err) = child.kill() {
                            tracing::error!("{:?}", err);
                        }
                        if let Err(err) = child.wait() {
                            tracing::error!("{:?}", err);
                        }
                        return;
                    }
                }
            }
        })
        .unwrap();

    for msg in reader_rx {
        match msg {
            RpcMessage::Request(id, req) => {
                let writer_tx = writer_tx.clone();
                let core_rpc = core_rpc.clone();
                std::thread::spawn(move || match core_rpc.request(req) {
                    Ok(resp) => {
                        if let Err(err) =
                            writer_tx.send(RpcMessage::Response(id, resp))
                        {
                            tracing::error!("{:?}", err);
                        }
                    }
                    Err(e) => {
                        if let Err(err) = writer_tx.send(RpcMessage::Error(id, e)) {
                            tracing::error!("{:?}", err);
                        }
                    }
                });
            }
            RpcMessage::Notification(n) => {
                core_rpc.notification(n);
            }
            RpcMessage::Response(id, resp) => {
                proxy_rpc.handle_response(id, Ok(resp));
            }
            RpcMessage::Error(id, err) => {
                proxy_rpc.handle_response(id, Err(err));
            }
        }
    }

    Ok(())
}

fn host_specification(
    remote: &impl Remote,
) -> Result<(HostPlatform, HostArchitecture)> {
    use HostArchitecture::*;
    use HostPlatform::*;

    let cmd = remote.command_builder().args(["uname", "-sm"]).output();

    let spec = match cmd {
        Ok(cmd) => {
            let stdout = String::from_utf8_lossy(&cmd.stdout).to_lowercase();
            let stdout = stdout.trim();
            debug!("{}", &stdout);
            match stdout {
                // If empty, then we probably deal with Windows and not Unix
                // or something went wrong with command output
                "" => {
                    // Try cmd explicitly
                    let cmd = remote
                        .command_builder()
                        .args(["cmd", "/c", "echo %OS% %PROCESSOR_ARCHITECTURE%"])
                        .output();
                    match cmd {
                        Ok(cmd) => {
                            let stdout =
                                String::from_utf8_lossy(&cmd.stdout).to_lowercase();
                            let stdout = stdout.trim();
                            debug!("{}", &stdout);
                            match stdout.split_once(' ') {
                                Some((os, arch)) => (parse_os(os), parse_arch(arch)),
                                None => {
                                    // PowerShell fallback
                                    let cmd = remote
                                            .command_builder()
                                            .args(["echo", "\"${env:OS} ${env:PROCESSOR_ARCHITECTURE}\""])
                                            .output();
                                    match cmd {
                                        Ok(cmd) => {
                                            let stdout =
                                                String::from_utf8_lossy(&cmd.stdout)
                                                    .to_lowercase();
                                            let stdout = stdout.trim();
                                            debug!("{}", &stdout);
                                            match stdout.split_once(' ') {
                                                Some((os, arch)) => {
                                                    (parse_os(os), parse_arch(arch))
                                                }
                                                None => (UnknownOS, UnknownArch),
                                            }
                                        }
                                        Err(e) => {
                                            error!("{e}");
                                            (UnknownOS, UnknownArch)
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!("{e}");
                            (UnknownOS, UnknownArch)
                        }
                    }
                }
                v => {
                    if let Some((os, arch)) = v.split_once(' ') {
                        (parse_os(os), parse_arch(arch))
                    } else {
                        (UnknownOS, UnknownArch)
                    }
                }
            }
        }
        Err(e) => {
            error!("{e}");
            (UnknownOS, UnknownArch)
        }
    };
    Ok(spec)
}

fn parse_arch(arch: &str) -> HostArchitecture {
    use HostArchitecture::*;
    // processor architectures be like that
    match arch {
        "amd64" | "x64" | "x86_64" => AMD64,
        "x86" | "i386" | "i586" | "i686" => X86,
        "arm" | "armhf" | "armv6" => ARM32v6,
        "armv7" | "armv7l" => ARM32v7,
        "arm64" | "armv8" | "aarch64" => ARM64,
        _ => UnknownArch,
    }
}

fn parse_os(os: &str) -> HostPlatform {
    use HostPlatform::*;
    match os {
        "linux" => Linux,
        "darwin" => Darwin,
        "windows_nt" => Windows,
        v if v.ends_with("bsd") => Bsd,
        _ => UnknownOS,
    }
}
