#![cfg(unix)]

use cosmic_ext_applet_container_manager::{
    domain::{Action, Backend, Container},
    runtime::{RuntimeSpec, discover_all, execute_action},
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn fake(dir: &Path, name: &str, script: &str) -> RuntimeSpec {
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    thread::sleep(Duration::from_millis(5));
    RuntimeSpec::new(Backend::from_name(name).unwrap(), path)
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir()
        .join(format!("cosmic-container-{name}-{}-{sequence}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn value(state: &str) -> Container {
    Container {
        backend: Backend::Docker,
        id: "abc123".into(),
        image: "web:1".into(),
        name: "web".into(),
        state: state.into(),
        status: "status".into(),
    }
}

#[test]
fn discovery_isolates_errors_and_uses_fixed_args() {
    let dir = temp_dir("discover");
    let docker = fake(
        &dir,
        "docker",
        r#"test "$1" = ps && test "$2" = -a && test "$3" = --no-trunc && test "$4" = --format || exit 42; printf 'id1\tteam/web:1\tweb\trunning\tUp\n'"#,
    );
    let podman = fake(&dir, "podman", "printf 'unavailable' >&2; exit 127");
    let result = discover_all(&[docker, podman]);
    assert_eq!(result.containers.len(), 1);
    assert_eq!(result.successful_backends, [Backend::Docker].into());
    assert!(result.errors.contains_key(&Backend::Podman));
    assert!(!result.errors.contains_key(&Backend::Docker));
}

#[test]
fn host_launcher_wraps_discovery_command() {
    let dir = temp_dir("host-discover");
    let launcher = dir.join("flatpak-spawn");
    fs::write(
        &launcher,
        r#"#!/bin/sh
test "$1" = --host && test "$2" = --watch-bus && test "$3" = docker && test "$4" = ps && test "$5" = -a || exit 42
printf 'id1\tteam/web:1\tweb\trunning\tUp\n'
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&launcher).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&launcher, permissions).unwrap();

    let result = discover_all(&[RuntimeSpec::host(Backend::Docker, launcher)]);

    assert_eq!(result.successful_backends, [Backend::Docker].into());
    assert_eq!(result.containers.len(), 1);
}

#[test]
fn empty_successful_discovery_still_reports_the_engine() {
    let dir = temp_dir("empty-discover");
    let result = discover_all(&[fake(&dir, "podman", "exit 0")]);

    assert!(result.containers.is_empty());
    assert_eq!(result.successful_backends, [Backend::Podman].into());
}

#[test]
fn exact_cross_runtime_duplicates_prefer_docker() {
    let dir = temp_dir("dedupe");
    let output = "printf 'same-full-id\\tteam/web:1\\tweb\\trunning\\tUp\\n'";
    let result = discover_all(&[fake(&dir, "podman", output), fake(&dir, "docker", output)]);
    assert_eq!(result.containers.len(), 1);
    assert_eq!(result.containers[0].backend, Backend::Docker);
}

#[test]
fn hung_runtime_times_out_without_blocking_other_runtime() {
    let dir = temp_dir("discovery-timeout");
    let timeout = Duration::from_millis(400);
    let docker = fake(&dir, "docker", "exec sleep 1").with_timeout(timeout);
    let podman =
        fake(&dir, "podman", "sleep 0.3; printf 'podman-id\\tteam/api:1\\tapi\\trunning\\tUp\\n'")
            .with_timeout(timeout);

    let started = Instant::now();
    let result = discover_all(&[podman, docker]);
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(600),
        "discovery took {elapsed:?}, so runtimes did not overlap"
    );
    assert_eq!(result.containers.len(), 1);
    assert_eq!(result.containers[0].backend, Backend::Podman);
    assert!(
        result.errors[&Backend::Docker].contains("timed out"),
        "unexpected error: {}",
        result.errors[&Backend::Docker]
    );
    assert!(!result.errors.contains_key(&Backend::Podman));
}

#[test]
fn system_actions_allow_the_container_grace_period() {
    let runtime = RuntimeSpec::system(Backend::Docker);
    assert!(runtime.action_timeout >= Duration::from_secs(30));
    assert!(runtime.timeout <= Duration::from_secs(10));
}

#[test]
fn start_stop_and_restart_use_the_expected_subcommands() {
    let dir = temp_dir("action-subcommands");
    for (action, state, expected) in [
        (Action::Start, "exited", "start"),
        (Action::Stop, "running", "stop"),
        (Action::Restart, "running", "restart"),
    ] {
        let action_dir = dir.join(expected);
        fs::create_dir_all(&action_dir).unwrap();
        let script = format!(r#"test "$1" = {expected} && test "$2" = abc123 && test "$#" = 2"#);
        let runtime = fake(&action_dir, "docker", &script);
        execute_action(&runtime, &value(state), action).unwrap();
    }
}

#[test]
fn actions_use_direct_non_force_arguments_and_reject_running_delete() {
    let dir = temp_dir("actions");
    let marker = dir.join("called");
    let script = format!(
        r#"if test "$1" = inspect; then
  test "$2" = --format && test "$3" = '{{{{.State.Status}}}}' && test "$4" = abc123 && test "$#" = 4 || exit 42
  printf 'exited\n'
  exit 0
fi
test "$1" = rm && test "$2" = abc123 && test "$#" = 2 && : > '{}'"#,
        marker.display()
    );
    let runtime = fake(&dir, "docker", &script);
    execute_action(&runtime, &value("exited"), Action::Delete).unwrap();
    assert!(marker.exists());
    fs::remove_file(&marker).unwrap();
    assert!(execute_action(&runtime, &value("running"), Action::Delete).is_err());
    assert!(!marker.exists());
}

#[test]
fn host_launcher_wraps_inspect_and_action_commands() {
    let dir = temp_dir("host-actions");
    let marker = dir.join("removed");
    let launcher = dir.join("flatpak-spawn");
    fs::write(
        &launcher,
        format!(
            r#"#!/bin/sh
test "$1" = --host && test "$2" = --watch-bus && test "$3" = docker || exit 42
shift 3
if test "$1" = inspect; then
  printf 'exited\n'
  exit 0
fi
test "$1" = rm && test "$2" = abc123 && test "$#" = 2 && : > '{}'
"#,
            marker.display()
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&launcher).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&launcher, permissions).unwrap();

    execute_action(&RuntimeSpec::host(Backend::Docker, launcher), &value("exited"), Action::Delete)
        .unwrap();

    assert!(marker.exists());
}

#[test]
fn stale_exited_container_is_not_removed_when_inspect_reports_running() {
    let dir = temp_dir("stale-delete");
    let marker = dir.join("rm-called");
    let script = format!(
        r#"if test "$1" = inspect; then
  test "$2" = --format && test "$3" = '{{{{.State.Status}}}}' && test "$4" = abc123 && test "$#" = 4 || exit 42
  printf 'running\n'
  exit 0
fi
if test "$1" = rm; then : > '{}'; fi"#,
        marker.display()
    );
    let runtime = fake(&dir, "docker", &script);

    assert!(execute_action(&runtime, &value("exited"), Action::Delete).is_err());
    assert!(!marker.exists());
}

#[test]
fn detached_descendant_cannot_hold_action_output_pipes_open() {
    let dir = temp_dir("action-detached-descendant-timeout");
    let pid_file = dir.join("descendant-pid");
    let script = format!(
        "setsid sh -c 'printf \"%s\\\\n\" \"$$\" > \"{}\"; exec sleep 5' & wait",
        pid_file.display()
    );
    let runtime = fake(&dir, "docker", &script).with_timeout(Duration::from_millis(150));

    let started = Instant::now();
    let error = execute_action(&runtime, &value("exited"), Action::Start).unwrap_err();
    let elapsed = started.elapsed();
    let pid = fs::read_to_string(pid_file).unwrap();
    let _ = std::process::Command::new("kill").args(["-KILL", pid.trim()]).status();

    assert!(elapsed < Duration::from_millis(600), "action took {elapsed:?}");
    assert!(error.to_string().contains("timed out"), "unexpected error: {error}");
}

#[test]
fn hung_action_times_out_and_returns_an_error() {
    let dir = temp_dir("action-timeout");
    let pid_file = dir.join("pid");
    let script = format!(
        "printf '%s\\n' \"$$\" > '{}'; printf 'started action' >&2; exec sleep 1",
        pid_file.display()
    );
    let runtime = fake(&dir, "docker", &script).with_timeout(Duration::from_millis(150));

    let started = Instant::now();
    let error = execute_action(&runtime, &value("exited"), Action::Start).unwrap_err();
    let elapsed = started.elapsed();
    let pid = fs::read_to_string(pid_file).unwrap();

    assert!(elapsed < Duration::from_millis(500), "action took {elapsed:?}");
    assert!(error.to_string().contains("timed out"), "unexpected error: {error}");
    assert!(error.to_string().contains("started action"), "stderr was not captured: {error}");
    assert!(!Path::new("/proc").join(pid.trim()).exists(), "timed-out child was not reaped");
}
