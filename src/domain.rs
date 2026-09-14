// SPDX-License-Identifier: GPL-3.0-only

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Backend {
    Docker,
    Podman,
}

impl Backend {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Docker => "Docker",
            Self::Podman => "Podman",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "docker" => Some(Self::Docker),
            "podman" => Some(Self::Podman),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Start,
    Stop,
    Restart,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerState {
    Configured,
    Created,
    Dead,
    Exited,
    Initialized,
    Paused,
    Removing,
    Restarting,
    Running,
    Stopped,
    Unknown,
    Unrecognized,
}

impl ContainerState {
    #[must_use]
    pub fn from_runtime(state: &str) -> Self {
        if state.eq_ignore_ascii_case("configured") {
            Self::Configured
        } else if state.eq_ignore_ascii_case("created") {
            Self::Created
        } else if state.eq_ignore_ascii_case("dead") {
            Self::Dead
        } else if state.eq_ignore_ascii_case("exited") {
            Self::Exited
        } else if state.eq_ignore_ascii_case("initialized") {
            Self::Initialized
        } else if state.eq_ignore_ascii_case("paused") {
            Self::Paused
        } else if state.eq_ignore_ascii_case("removing") {
            Self::Removing
        } else if state.eq_ignore_ascii_case("restarting") {
            Self::Restarting
        } else if state.eq_ignore_ascii_case("running") {
            Self::Running
        } else if state.eq_ignore_ascii_case("stopped") {
            Self::Stopped
        } else if state.eq_ignore_ascii_case("unknown") {
            Self::Unknown
        } else {
            Self::Unrecognized
        }
    }

    #[must_use]
    pub const fn allows(self, action: Action) -> bool {
        match action {
            Action::Start => matches!(
                self,
                Self::Configured | Self::Created | Self::Exited | Self::Initialized | Self::Stopped
            ),
            Action::Stop | Action::Restart => matches!(self, Self::Running),
            Action::Delete => matches!(
                self,
                Self::Configured
                    | Self::Created
                    | Self::Dead
                    | Self::Exited
                    | Self::Initialized
                    | Self::Stopped
            ),
        }
    }
}

impl Action {
    #[must_use]
    pub const fn subcommand(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Delete => "rm",
        }
    }
}

use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Default)]
pub struct ActionErrorState {
    errors: BTreeMap<(Backend, String), String>,
}

impl ActionErrorState {
    pub fn begin(&mut self, backend: Backend, id: &str) {
        self.errors.remove(&(backend, id.to_owned()));
    }

    pub fn finish(&mut self, backend: Backend, id: &str, result: Result<(), String>) {
        let key = (backend, id.to_owned());
        match result {
            Ok(()) => {
                self.errors.remove(&key);
            }
            Err(error) => {
                self.errors.insert(key, error);
            }
        }
    }

    #[must_use]
    pub fn get(&self, backend: Backend, id: &str) -> Option<&str> {
        self.errors.get(&(backend, id.to_owned())).map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&(Backend, String), &String)> {
        self.errors.iter()
    }
}

#[derive(Debug, Default)]
pub struct ActionTracker {
    active: BTreeSet<(Backend, String)>,
}

impl ActionTracker {
    pub fn begin(&mut self, backend: Backend, id: &str) -> bool {
        self.active.insert((backend, id.to_owned()))
    }

    pub fn finish(&mut self, backend: Backend, id: &str) {
        self.active.remove(&(backend, id.to_owned()));
    }

    #[must_use]
    pub fn contains(&self, backend: Backend, id: &str) -> bool {
        self.active.contains(&(backend, id.to_owned()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    pub backend: Backend,
    pub id: String,
    pub image: String,
    pub name: String,
    pub state: String,
    pub status: String,
}

impl Container {
    #[must_use]
    pub fn image_label(&self) -> &str {
        self.image.rsplit('/').next().unwrap_or(&self.image)
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        ContainerState::from_runtime(&self.state) == ContainerState::Running
    }

    #[must_use]
    pub fn allows(&self, action: Action) -> bool {
        ContainerState::from_runtime(&self.state).allows(action)
    }
}
