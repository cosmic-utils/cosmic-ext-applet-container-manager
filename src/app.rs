// SPDX-License-Identifier: GPL-3.0-only

use std::{collections::BTreeSet, time::Duration};

use cosmic_ext_applet_container_manager::{
    domain::{Action, ActionErrorState, ActionTracker, Backend, Container},
    runtime::{self, Discovery, RuntimeSpec},
};

use crate::fl;
use cosmic::iced::advanced::text::EllipsizeHeightLimit;
use cosmic::iced::widget::text::{Ellipsize, Wrapping};
use cosmic::iced::{Alignment, Length, Limits, Subscription, window::Id};
use cosmic::prelude::*;
use cosmic::surface::action::{app_popup, destroy_popup};
use cosmic::widget;
use cosmic::widget::space::horizontal as horizontal_space;

pub struct AppModel {
    core: cosmic::Core,
    popup: Option<Id>,
    containers: Vec<Container>,
    successful_backends: BTreeSet<Backend>,
    action_errors: ActionErrorState,
    refreshing: bool,
    refresh_pending: bool,
    actions: ActionTracker,
    confirm_delete: Option<(Backend, String)>,
}

#[derive(Debug, Clone)]
pub enum Message {
    TogglePopup,
    PopupClosed(Id),
    Refresh,
    Refreshed(Discovery),
    RequestDelete(Backend, String),
    CancelDelete,
    RunAction(Container, Action),
    ActionFinished(Backend, String, Result<(), String>),
}

impl AppModel {
    fn refresh_task() -> Task<cosmic::Action<Message>> {
        Task::perform(
            async {
                tokio::task::spawn_blocking(|| {
                    runtime::discover_all(&[
                        RuntimeSpec::system(Backend::Docker),
                        RuntimeSpec::system(Backend::Podman),
                    ])
                })
                .await
                .unwrap_or_else(|error| {
                    let mut result = Discovery::default();
                    result.errors.insert(Backend::Docker, format!("refresh task failed: {error}"));
                    result.errors.insert(Backend::Podman, format!("refresh task failed: {error}"));
                    result
                })
            },
            |result| cosmic::Action::App(Message::Refreshed(result)),
        )
    }

    fn action_task(container: Container, action: Action) -> Task<cosmic::Action<Message>> {
        let backend = container.backend;
        let id = container.id.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    runtime::execute_action(&RuntimeSpec::system(backend), &container, action)
                        .map_err(|error| error.to_string())
                })
                .await
                .unwrap_or_else(|error| Err(format!("action task failed: {error}")))
            },
            move |result| cosmic::Action::App(Message::ActionFinished(backend, id, result)),
        )
    }

    fn action_button<'a>(
        &self,
        container: &Container,
        action: Action,
        label: String,
    ) -> cosmic::Element<'a, Message> {
        let mut button = if matches!(action, Action::Start | Action::Stop) {
            widget::button::suggested(label)
        } else {
            widget::button::text(label)
        };
        if container.allows(action) && !self.actions.contains(container.backend, &container.id) {
            button = button.on_press(Message::RunAction(container.clone(), action));
        }
        button.into()
    }
}

fn visible_actions(container: &Container) -> Vec<Action> {
    [Action::Start, Action::Stop, Action::Restart, Action::Delete]
        .into_iter()
        .filter(|action| container.allows(*action))
        .collect()
}

fn engine_names(backends: &BTreeSet<Backend>) -> String {
    backends.iter().map(|backend| backend.name()).collect::<Vec<_>>().join(", ")
}

fn popup_surface_action() -> cosmic::surface::Action {
    app_popup::<AppModel>(
        |_| Default::default(),
        |app| {
            let id = Id::unique();
            app.popup = Some(id);
            let mut settings = app.core.applet.get_popup_settings(
                app.core.main_window_id().expect("main applet window"),
                id,
                None,
                None,
                None,
            );
            settings.positioner.size_limits =
                Limits::NONE.min_width(360.0).max_width(520.0).min_height(180.0).max_height(720.0);
            settings
        },
        None,
    )
}

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = "org.cosmic_utils.CosmicExtAppletContainerManager";

    fn core(&self) -> &cosmic::Core {
        &self.core
    }
    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(core: cosmic::Core, _flags: ()) -> (Self, Task<cosmic::Action<Message>>) {
        (
            Self {
                core,
                popup: None,
                containers: Vec::new(),
                successful_backends: BTreeSet::new(),
                action_errors: ActionErrorState::default(),
                refreshing: true,
                refresh_pending: false,
                actions: ActionTracker::default(),
                confirm_delete: None,
            },
            Self::refresh_task(),
        )
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn view(&self) -> Element<'_, Message> {
        self.core
            .applet
            .icon_button("org.cosmic_utils.CosmicExtAppletContainerManager-symbolic")
            .on_press(Message::TogglePopup)
            .into()
    }

    fn view_window(&self, _id: Id) -> Element<'_, Message> {
        let mut content: Vec<Element<'_, Message>> = Vec::new();
        let mut refresh = widget::button::icon(
            widget::icon::from_name("view-refresh-symbolic").size(16).symbolic(true),
        )
        .padding(8);
        if !self.refreshing {
            refresh = refresh.on_press(Message::Refresh);
        }
        let refresh_tooltip = if self.refreshing { fl!("loading") } else { fl!("refresh") };
        content.push(
            widget::row::with_children(vec![
                widget::text::heading(fl!("app-title")).into(),
                horizontal_space().into(),
                widget::tooltip(
                    refresh,
                    widget::text::caption(refresh_tooltip),
                    widget::tooltip::Position::Bottom,
                )
                .into(),
            ])
            .align_y(Alignment::Center)
            .into(),
        );

        if !self.successful_backends.is_empty() {
            content.push(
                widget::text::caption(fl!(
                    "available-engines",
                    engines = engine_names(&self.successful_backends)
                ))
                .into(),
            );
        }
        if self.refreshing && self.containers.is_empty() {
            content.push(widget::text::body(fl!("loading")).into());
        } else if self.containers.is_empty() {
            content.push(widget::text::body(fl!("empty")).into());
        }

        for (index, container) in self.containers.iter().enumerate() {
            if index > 0 {
                content.push(
                    widget::container(widget::divider::horizontal::default())
                        .padding([2, 8])
                        .into(),
                );
            }

            let busy = self.actions.contains(container.backend, &container.id);
            let metadata = format!("{} · {}", container.image_label(), container.backend.name());
            let name = widget::text::heading(&container.name)
                .width(Length::Fill)
                .wrapping(Wrapping::None)
                .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)));
            let state = widget::text::caption(&container.state).align_x(Alignment::End);
            let mut row = widget::column::with_children(vec![
                widget::row::with_children(vec![name.into(), state.into()])
                    .align_y(Alignment::Center)
                    .spacing(8)
                    .into(),
                widget::text::caption(metadata)
                    .width(Length::Fill)
                    .wrapping(Wrapping::None)
                    .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
                    .into(),
            ])
            .spacing(4);

            if let Some(error) = self.action_errors.get(container.backend, &container.id) {
                row = row.push(widget::text::caption(error));
            }

            if busy {
                row = row.push(widget::text::caption(fl!("working")));
            } else if self.confirm_delete.as_ref()
                == Some(&(container.backend, container.id.clone()))
            {
                row = row.push(
                    widget::row::with_children(vec![
                        widget::text::caption(fl!("delete-warning")).width(Length::Fill).into(),
                        widget::button::destructive(fl!("confirm-delete"))
                            .on_press(Message::RunAction(container.clone(), Action::Delete))
                            .into(),
                        widget::button::text(fl!("cancel")).on_press(Message::CancelDelete).into(),
                    ])
                    .align_y(Alignment::Center)
                    .spacing(6),
                );
            } else {
                let mut actions = vec![
                    widget::text::caption(&container.status)
                        .width(Length::Fill)
                        .wrapping(Wrapping::None)
                        .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
                        .into(),
                ];
                for action in visible_actions(container) {
                    let button = match action {
                        Action::Start => self.action_button(container, action, fl!("start")),
                        Action::Stop => self.action_button(container, action, fl!("stop")),
                        Action::Restart => self.action_button(container, action, fl!("restart")),
                        Action::Delete => widget::button::text(fl!("delete"))
                            .on_press(Message::RequestDelete(
                                container.backend,
                                container.id.clone(),
                            ))
                            .into(),
                    };
                    actions.push(button);
                }
                row = row.push(
                    widget::row::with_children(actions).align_y(Alignment::Center).spacing(6),
                );
            }
            content.push(widget::container(row).padding([6, 8]).width(Length::Fill).into());
        }

        self.core
            .applet
            .popup_container(
                widget::scrollable(
                    widget::column::with_children(content)
                        .spacing(6)
                        .padding(12)
                        .width(Length::Fixed(440.0)),
                )
                .height(Length::Shrink),
            )
            .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        cosmic::iced::time::every(Duration::from_secs(15)).map(|_| Message::Refresh)
    }

    fn update(&mut self, message: Message) -> Task<cosmic::Action<Message>> {
        match message {
            Message::TogglePopup => {
                if let Some(id) = self.popup.take() {
                    return cosmic::surface::surface_task(destroy_popup(id));
                }
                let popup = cosmic::surface::surface_task(popup_surface_action());
                if !self.refreshing {
                    self.refreshing = true;
                    return Task::batch([popup, Self::refresh_task()]);
                }
                self.refresh_pending = true;
                return popup;
            }
            Message::PopupClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                }
            }
            Message::Refresh => {
                if !self.refreshing {
                    self.refreshing = true;
                    return Self::refresh_task();
                }
                self.refresh_pending = true;
            }
            Message::Refreshed(result) => {
                self.containers = result.containers;
                self.successful_backends = result.successful_backends;
                if self.refresh_pending {
                    self.refresh_pending = false;
                    return Self::refresh_task();
                }
                self.refreshing = false;
            }
            Message::RequestDelete(backend, id) => self.confirm_delete = Some((backend, id)),
            Message::CancelDelete => self.confirm_delete = None,
            Message::RunAction(container, action) => {
                if !container.allows(action)
                    || !self.actions.begin(container.backend, &container.id)
                {
                    return Task::none();
                }
                self.action_errors.begin(container.backend, &container.id);
                self.confirm_delete = None;
                return Self::action_task(container, action);
            }
            Message::ActionFinished(backend, id, result) => {
                self.actions.finish(backend, &id);
                self.action_errors.finish(backend, &id, result);
                if !self.refreshing {
                    self.refreshing = true;
                    return Self::refresh_task();
                }
                self.refresh_pending = true;
            }
        }
        Task::none()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn container(running: bool) -> Container {
        Container {
            backend: Backend::Docker,
            id: "container-id".into(),
            name: "container-name".into(),
            image: "example/image:latest".into(),
            status: if running { "Up 2 minutes".into() } else { "Exited (0) 2 minutes ago".into() },
            state: if running { "running".into() } else { "exited".into() },
        }
    }

    #[test]
    fn running_rows_only_show_valid_actions() {
        assert_eq!(visible_actions(&container(true)), vec![Action::Stop, Action::Restart]);
    }

    #[test]
    fn stopped_rows_only_show_valid_actions() {
        assert_eq!(visible_actions(&container(false)), vec![Action::Start, Action::Delete]);
    }

    #[test]
    fn engine_names_only_include_successfully_queried_backends() {
        assert_eq!(engine_names(&[Backend::Podman, Backend::Docker].into()), "Docker, Podman");
    }
}
