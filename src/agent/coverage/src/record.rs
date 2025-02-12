// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use debuggable_module::loader::Loader;

use crate::binary::{BinaryCoverage, DebugInfoCache};
use crate::AllowList;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "windows")]
pub mod windows;

pub struct CoverageRecorder {
    module_allowlist: AllowList,
    cache: Arc<DebugInfoCache>,
    cmd: Command,
    loader: Arc<Loader>,
    timeout: Duration,
}

impl CoverageRecorder {
    pub fn new(mut cmd: Command) -> Self {
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let loader = Arc::new(Loader::new());
        let timeout = Duration::from_secs(5);

        Self {
            module_allowlist: AllowList::default(),
            cache: Arc::new(DebugInfoCache::new(AllowList::default())),
            cmd,
            loader,
            timeout,
        }
    }

    pub fn module_allowlist(mut self, module_allowlist: AllowList) -> Self {
        self.module_allowlist = module_allowlist;
        self
    }

    pub fn loader(mut self, loader: impl Into<Arc<Loader>>) -> Self {
        self.loader = loader.into();
        self
    }

    pub fn debuginfo_cache(mut self, cache: impl Into<Arc<DebugInfoCache>>) -> Self {
        self.cache = cache.into();
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[cfg(target_os = "linux")]
    pub fn record(self) -> Result<Recorded> {
        use std::sync::Mutex;

        use anyhow::bail;

        use crate::timer;
        use linux::debugger::Debugger;
        use linux::LinuxRecorder;

        let loader = self.loader.clone();

        let child_pid: Arc<Mutex<Option<u32>>> = Arc::new(Mutex::new(None));

        let recorded = {
            let child_pid = child_pid.clone();

            timer::timed(self.timeout, move || {
                let mut recorder = LinuxRecorder::new(&loader, self.module_allowlist, &self.cache);
                let mut dbg = Debugger::new(&mut recorder);
                let child = dbg.spawn(self.cmd)?;

                // Save child PID so we can send SIGKILL on timeout.
                if let Ok(mut pid) = child_pid.lock() {
                    *pid = Some(child.id());
                } else {
                    bail!("couldn't lock mutex to save child PID ");
                }

                let output = dbg.wait(child)?;
                let coverage = recorder.coverage;

                Ok(Recorded { coverage, output })
            })
        };

        if let Err(timer::TimerError::Timeout(..)) = &recorded {
            let Ok(pid) = child_pid.lock() else {
                bail!("couldn't lock mutex to kill child PID");
            };

            if let Some(pid) = *pid {
                use nix::sys::signal::{kill, SIGKILL};

                let pid = pete::Pid::from_raw(pid as i32);

                // Try to clean up, ignore errors due to earlier exits.
                let _ = kill(pid, SIGKILL);
            } else {
                warn!("timeout before PID set for child process");
            }
        }

        recorded?
    }

    #[cfg(target_os = "windows")]
    pub fn record(self) -> Result<Recorded> {
        use debugger::Debugger;
        use windows::WindowsRecorder;

        let loader = self.loader.clone();

        crate::timer::timed(self.timeout, move || {
            let mut recorder =
                WindowsRecorder::new(&loader, self.module_allowlist, self.cache.as_ref());
            let (mut dbg, child) = Debugger::init(self.cmd, &mut recorder)?;
            dbg.run(&mut recorder)?;

            // If the debugger callbacks fail, this may return with a spurious clean exit.
            let output = child.wait_with_output()?.into();

            // Check if debugging was stopped due to a callback error.
            //
            // If so, the debugger terminated the target, and the recorded coverage and
            // output are both invalid.
            if let Some(err) = recorder.stop_error {
                return Err(err);
            }

            let coverage = recorder.coverage;

            Ok(Recorded { coverage, output })
        })?
    }
}

#[derive(Clone, Debug)]
pub struct Recorded {
    pub coverage: BinaryCoverage,
    pub output: Output,
}

#[derive(Clone, Debug, Default)]
pub struct Output {
    pub status: Option<ExitStatus>,
    pub stderr: String,
    pub stdout: String,
}

impl From<std::process::Output> for Output {
    fn from(output: std::process::Output) -> Self {
        let status = Some(output.status);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        Self {
            status,
            stdout,
            stderr,
        }
    }
}
