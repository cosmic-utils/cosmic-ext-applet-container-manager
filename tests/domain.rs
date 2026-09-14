use cosmic_ext_applet_container_manager::domain::{
    Action, ActionErrorState, ActionTracker, Backend, Container,
};

fn container(state: &str) -> Container {
    Container {
        backend: Backend::Docker,
        id: "abc123".into(),
        image: "registry.example/team/widget-service:latest".into(),
        name: "actual-runtime-name".into(),
        state: state.into(),
        status: "Up 2 hours".into(),
    }
}

#[test]
fn image_label_strips_path_and_preserves_tag() {
    assert_eq!(container("running").image_label(), "widget-service:latest");
    let mut digest = container("exited");
    digest.image = "registry.example/team/image@sha256:deadbeef".into();
    assert_eq!(digest.image_label(), "image@sha256:deadbeef");
}

#[test]
fn running_container_cannot_be_deleted() {
    assert!(!container("running").allows(Action::Delete));
    assert!(container("exited").allows(Action::Delete));
}

#[test]
fn only_explicit_inactive_states_allow_start_or_delete() {
    for state in ["created", "exited", "stopped"] {
        assert!(container(state).allows(Action::Start), "{state} should be startable");
        assert!(container(state).allows(Action::Delete), "{state} should be removable");
    }

    for state in ["paused", "restarting", "removing", "unknown", "broken-state", ""] {
        assert!(!container(state).allows(Action::Start), "{state:?} must fail closed for start");
        assert!(!container(state).allows(Action::Delete), "{state:?} must fail closed for delete");
    }
}

#[test]
fn only_running_allows_stop_or_restart() {
    assert!(container("RUNNING").allows(Action::Stop));
    assert!(container("running").allows(Action::Restart));

    for state in ["created", "exited", "stopped", "paused", "restarting", "removing", "unknown"] {
        assert!(!container(state).allows(Action::Stop), "{state} must not allow stop");
        assert!(!container(state).allows(Action::Restart), "{state} must not allow restart");
    }
}

#[test]
fn row_identity_is_backend_aware_and_duplicate_actions_are_rejected() {
    let mut tracker = ActionTracker::default();
    assert!(tracker.begin(Backend::Docker, "abc123"));
    assert!(!tracker.begin(Backend::Docker, "abc123"));
    assert!(tracker.begin(Backend::Podman, "abc123"));
    tracker.finish(Backend::Docker, "abc123");
    assert!(tracker.begin(Backend::Docker, "abc123"));
}

#[test]
fn action_errors_are_isolated_by_backend_and_container() {
    let mut errors = ActionErrorState::default();
    errors.finish(Backend::Docker, "first", Err("permission denied".into()));
    errors.finish(Backend::Docker, "second", Err("daemon unavailable".into()));

    assert_eq!(errors.get(Backend::Docker, "first"), Some("permission denied"));
    assert_eq!(errors.get(Backend::Docker, "second"), Some("daemon unavailable"));

    errors.begin(Backend::Docker, "second");
    assert_eq!(errors.get(Backend::Docker, "first"), Some("permission denied"));
    assert_eq!(errors.get(Backend::Docker, "second"), None);

    errors.finish(Backend::Docker, "first", Ok(()));
    assert_eq!(errors.get(Backend::Docker, "first"), None);
}
