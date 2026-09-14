// SPDX-License-Identifier: GPL-3.0-only

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    ffi::OsString,
    fmt,
    io::{self, Read},
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use nix::{
    fcntl::{FcntlArg, OFlag, fcntl},
    sys::signal::{Signal, killpg},
    unistd::Pid,
};

use crate::{
    domain::{Action, Backend, Container, ContainerState},
    parser::parse_container_list,
};

const LIST_FORMAT: &str = "{{.ID}}\t{{.Image}}\t{{.Names}}\t{{.State}}\t{{.Status}}";
const SYSTEM_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const SYSTEM_ACTION_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CAPTURE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct RuntimeSpec {
    pub backend: Backend,
    pub program: PathBuf,
    prefix_args: Vec<OsString>,
    pub timeout: Duration,
    pub action_timeout: Duration,
}

impl RuntimeSpec {
    pub fn new(backend: Backend, program: impl Into<PathBuf>) -> Self {
        Self {
            backend,
            program: program.into(),
            prefix_args: Vec::new(),
            timeout: SYSTEM_COMMAND_TIMEOUT,
            action_timeout: SYSTEM_ACTION_TIMEOUT,
        }
    }

    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self.action_timeout = timeout;
        self
    }

    #[must_use]
    pub fn system(backend: Backend) -> Self {
        if cfg!(feature = "flatpak") {
            Self::host(backend, "flatpak-spawn")
        } else {
            Self::new(backend, Self::backend_program(backend))
        }
    }

    #[must_use]
    pub fn host(backend: Backend, launcher: impl Into<PathBuf>) -> Self {
        let mut runtime = Self::new(backend, launcher);
        runtime.prefix_args =
            vec!["--host".into(), "--watch-bus".into(), Self::backend_program(backend).into()];
        runtime
    }

    const fn backend_program(backend: Backend) -> &'static str {
        match backend {
            Backend::Docker => "docker",
            Backend::Podman => "podman",
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.prefix_args);
        command
    }
}

#[derive(Debug, Default, Clone)]
pub struct Discovery {
    pub containers: Vec<Container>,
    pub successful_backends: BTreeSet<Backend>,
    pub errors: BTreeMap<Backend, String>,
}

#[derive(Debug)]
pub struct RuntimeError(String);

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl Error for RuntimeError {}

pub fn discover_all(runtimes: &[RuntimeSpec]) -> Discovery {
    let mut discovery = Discovery::default();
    let mut ordered = runtimes.to_vec();
    ordered.sort_by_key(|runtime| runtime.backend);
    let results = thread::scope(|scope| {
        let workers = ordered
            .iter()
            .map(|runtime| (runtime.backend, scope.spawn(move || discover(runtime))))
            .collect::<Vec<_>>();
        workers
            .into_iter()
            .map(|(backend, worker)| {
                let result = worker
                    .join()
                    .unwrap_or_else(|_| Err(RuntimeError("runtime worker panicked".into())));
                (backend, result)
            })
            .collect::<Vec<_>>()
    });
    for (backend, result) in results {
        match result {
            Ok(items) => {
                discovery.successful_backends.insert(backend);
                for item in items {
                    let duplicate = discovery.containers.iter().any(|existing| {
                        existing.id == item.id
                            && existing.image == item.image
                            && existing.name == item.name
                            && existing.state == item.state
                            && existing.status == item.status
                    });
                    if !duplicate {
                        discovery.containers.push(item);
                    }
                }
            }
            Err(error) => {
                discovery.errors.insert(backend, error.to_string());
            }
        }
    }
    discovery
}

fn discover(runtime: &RuntimeSpec) -> Result<Vec<Container>, RuntimeError> {
    let mut command = runtime.command();
    command.args(["ps", "-a", "--no-trunc", "--format", LIST_FORMAT]);
    let output = run_with_timeout(
        command,
        runtime.timeout,
        &format!("{} discovery", runtime.backend.name()),
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(RuntimeError(if stderr.is_empty() {
            format!("{} exited with {}", runtime.backend.name(), output.status)
        } else {
            stderr
        }));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| RuntimeError(format!("invalid UTF-8: {error}")))?;
    parse_container_list(runtime.backend, &stdout).map_err(|error| RuntimeError(error.to_string()))
}

pub fn execute_action(
    runtime: &RuntimeSpec,
    container: &Container,
    action: Action,
) -> Result<(), RuntimeError> {
    if runtime.backend != container.backend {
        return Err(RuntimeError("runtime does not match container backend".into()));
    }
    if !container.allows(action) {
        return Err(RuntimeError(format!(
            "{} is not allowed for this container",
            action.subcommand()
        )));
    }
    if action == Action::Delete {
        let mut command = runtime.command();
        command.args(["inspect", "--format", "{{.State.Status}}", container.id.as_str()]);
        let output = run_with_timeout(
            command,
            runtime.timeout,
            &format!("{} inspect", runtime.backend.name()),
        )?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(RuntimeError(if stderr.is_empty() {
                format!("{} inspect exited with {}", runtime.backend.name(), output.status)
            } else {
                stderr
            }));
        }
        let state = String::from_utf8(output.stdout)
            .map_err(|error| RuntimeError(format!("invalid inspect UTF-8: {error}")))?;
        if !ContainerState::from_runtime(state.trim()).allows(Action::Delete) {
            return Err(RuntimeError(format!(
                "container is not removable in current state {:?}",
                state.trim()
            )));
        }
    }
    let mut command = runtime.command();
    command.args([action.subcommand(), container.id.as_str()]);
    let output = run_with_timeout(
        command,
        runtime.action_timeout,
        &format!("{} {}", runtime.backend.name(), action.subcommand()),
    )?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(RuntimeError(if stderr.is_empty() {
        format!("{} exited with {}", runtime.backend.name(), output.status)
    } else {
        stderr
    }))
}

fn run_with_timeout(
    mut command: Command,
    timeout: Duration,
    description: &str,
) -> Result<Output, RuntimeError> {
    // Isolate the CLI and descendants so a timeout closes every inherited pipe.
    command.process_group(0);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| RuntimeError(format!("could not run {description}: {error}")))?;
    let mut stdout = child.stdout.take().expect("stdout was configured as piped");
    let mut stderr = child.stderr.take().expect("stderr was configured as piped");
    if let Err(error) =
        set_nonblocking(&stdout, description).and_then(|()| set_nonblocking(&stderr, description))
    {
        let _ = killpg(Pid::from_raw(child.id() as i32), Signal::SIGKILL);
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let started = Instant::now();

    loop {
        drain_pipe(&mut stdout, &mut stdout_bytes, description)?;
        drain_pipe(&mut stderr, &mut stderr_bytes, description)?;
        match child.try_wait() {
            Ok(Some(status)) => {
                drain_pipe(&mut stdout, &mut stdout_bytes, description)?;
                drain_pipe(&mut stderr, &mut stderr_bytes, description)?;
                return Ok(Output { status, stdout: stdout_bytes, stderr: stderr_bytes });
            }
            Ok(None) if started.elapsed() >= timeout => {
                let kill_error = killpg(Pid::from_raw(child.id() as i32), Signal::SIGKILL).err();
                if kill_error.is_some() {
                    let _ = child.kill();
                }
                child.wait().map_err(|error| {
                    RuntimeError(format!("could not reap timed out {description}: {error}"))
                })?;
                drain_pipe(&mut stdout, &mut stdout_bytes, description)?;
                drain_pipe(&mut stderr, &mut stderr_bytes, description)?;
                if let Some(error) = kill_error {
                    return Err(RuntimeError(format!(
                        "could not terminate timed out {description}: {error}"
                    )));
                }
                let stderr = String::from_utf8_lossy(&stderr_bytes).trim().to_owned();
                let detail = if stderr.is_empty() { String::new() } else { format!(": {stderr}") };
                return Err(RuntimeError(format!(
                    "{description} timed out after {timeout:?}{detail}"
                )));
            }
            Ok(None) => {
                thread::sleep(
                    Duration::from_millis(10).min(timeout.saturating_sub(started.elapsed())),
                );
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RuntimeError(format!("could not check {description} status: {error}")));
            }
        }
    }
}

fn set_nonblocking(pipe: &impl std::os::fd::AsFd, description: &str) -> Result<(), RuntimeError> {
    let flags = fcntl(pipe, FcntlArg::F_GETFL).map(OFlag::from_bits_truncate).map_err(|error| {
        RuntimeError(format!("could not read {description} pipe flags: {error}"))
    })?;
    fcntl(pipe, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))
        .map(|_| ())
        .map_err(|error| RuntimeError(format!("could not set {description} pipe flags: {error}")))
}

fn drain_pipe(
    pipe: &mut impl Read,
    captured: &mut Vec<u8>,
    description: &str,
) -> Result<(), RuntimeError> {
    let mut buffer = [0_u8; 8192];
    for _ in 0..8 {
        match pipe.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => {
                let keep = count.min(MAX_CAPTURE_BYTES.saturating_sub(captured.len()));
                captured.extend_from_slice(&buffer[..keep]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                return Err(RuntimeError(format!("could not read {description} output: {error}")));
            }
        }
    }
    Ok(())
}

#[cfg(all(test, feature = "flatpak"))]
mod tests {
    use super::*;

    #[test]
    fn system_runtime_uses_host_launcher_in_flatpak_builds() {
        let runtime = RuntimeSpec::system(Backend::Podman);

        assert_eq!(runtime.program, PathBuf::from("flatpak-spawn"));
        assert_eq!(runtime.prefix_args, ["--host", "--watch-bus", "podman"]);
    }
}
