use std::{path::Path, process::Command};

use anyhow::Result;

use super::{new_command, remote::Remote};
use crate::workspace::WslHost;

pub struct WslRemote {
    pub wsl: WslHost,
}

impl Remote for WslRemote {
    fn command_builder(&self) -> Command {
        let mut cmd = new_command("wsl");
        cmd.arg("-d").arg(&self.wsl.host).arg("--");
        cmd
    }
}
