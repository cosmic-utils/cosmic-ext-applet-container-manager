# Container Manager Applet for COSMIC™ Desktop

[![Sponsor](https://img.shields.io/badge/sponsor-FreddyFunk-ea4aaa?logo=github-sponsors)](https://github.com/sponsors/FreddyFunk)
[![CI](https://github.com/cosmic-utils/cosmic-ext-applet-container-manager/actions/workflows/ci.yml/badge.svg)](https://github.com/cosmic-utils/cosmic-ext-applet-container-manager/actions/workflows/ci.yml)
[![Translation status](https://hosted.weblate.org/widget/cosmic-utils/applet-container-manager/svg-badge.svg)](https://hosted.weblate.org/engage/cosmic-utils/)

A libcosmic panel applet that discovers local Docker and Podman containers and lets you start, stop, restart, or remove them.

![Container Manager applet showing example Podman containers](screenshots/container-manager-popup.png)

## Build

Requires a current stable Rust toolchain, Cargo, Git, and the native development libraries required by libcosmic.

```sh
cargo build --release
```

Run checks with:

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## Install

With [`just`](https://github.com/casey/just), install under `/usr`:

```sh
sudo just install
```

For packaging or a staged install, override the variables, for example:

```sh
just rootdir="$DESTDIR" prefix=/usr install
```

Build, install, or remove the Flatpak with:

```sh
just flatpak-build
just flatpak-install
just flatpak-uninstall
```

Restart the panel in COSMIC™ Desktop or log out and back in, then add **Container Manager** through the panel applet settings. `just uninstall` removes the installed files.

## Use

Open the panel popup to see every container returned by `docker ps -a` and `podman ps -a`. The popup lists the engines that were found and queried successfully; unavailable engines are omitted. Each row shows a short image label, its actual runtime name, backend, and status. Actions are enabled only when valid. Delete requires a second confirmation and is disabled for running containers. Discovery runs on startup, whenever the popup opens, every 15 seconds, and after actions.

For screenshot preparation, run:

```sh
just setup-screenshot-demo
```

The command shows every container in each successfully queried engine and requires typing `DELETE ALL CONTAINERS` before it proceeds. It then pulls the mutable demonstration image tags, force-removes the displayed containers, and creates labelled nginx, Jellyfin, Home Assistant, and Nextcloud examples in running, exited, and created states. Pulling can update each engine's image store, but the command does not remove images, volumes, networks, or engine configuration.

## Security and runtime behavior

- Native builds invoke `docker` and `podman` directly with fixed `std::process::Command` argument vectors. Flatpak builds use `flatpak-spawn --host --watch-bus` to invoke the host's container clients. Neither path invokes a shell.
- The Flatpak therefore requests access to `org.freedesktop.Flatpak`, which permits host command execution and means the sandbox is not a security boundary for this applet. This access is required because the containers and engine sockets belong to the host. `--watch-bus` ensures a timed-out host command is terminated when its launcher is killed.
- Before removal, the applet re-reads the container state with `inspect`; it proceeds only for an explicit inactive-state allowlist. Removal then uses only `rm <full-container-id>`, with no force option.
- Container IDs are never interpolated into a command string. Runtime lookup follows the applet process's `PATH`.
- The applet has the same container-engine privileges as the logged-in user. It does not elevate privileges. Docker socket membership commonly grants root-equivalent control; review your engine configuration before installing.
- Exact duplicate records exposed by both a Podman Docker-compatibility wrapper and Podman are displayed once, preferring the Docker-labelled result deterministically. Distinct engine records retain backend-aware identities and actions.

## License

GPL-3.0-only.
