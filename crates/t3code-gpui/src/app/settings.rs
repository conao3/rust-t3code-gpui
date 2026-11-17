use super::*;

impl Root {
    /// Writes client settings to the official app's file and applies them at once.
    pub(super) fn change_setting(
        &mut self,
        key: &'static str,
        value: Value,
        cx: &mut Context<Self>,
    ) {
        match client_settings::update(&self.settings_path, &[(key, value)]) {
            Ok(settings) => {
                let (interface, code) = (settings.font_size_interface, settings.font_size_code);
                self.settings = settings;
                if key == "sidebarWorkingShelfEnabled" {
                    self.rebuild_entries();
                }
                if key == "timestampFormat"
                    && let Err(error) =
                        crate::timestamp::change_format(self.settings.timestamp_format, cx)
                {
                    self.conversation.message = Some(format!("Timestamp format failed: {error:#}"));
                }
                if key == "gpuiAppearance" {
                    crate::appearance::apply(self.settings.light_theme, cx);
                }
                Theme::update(cx, |theme| {
                    theme.font_size = px(interface);
                    theme.mono_font_size = px(code);
                });
            }
            Err(error) => {
                self.conversation.message = Some(format!("Saving the settings failed: {error:#}"))
            }
        }
        cx.notify();
    }

    pub(super) fn settings_environment(&self) -> String {
        self.settings_environment
            .clone()
            .or_else(|| self.current().map(|(environment, _)| environment))
            .or_else(|| {
                self.connections
                    .first()
                    .map(|connection| connection.label().to_string())
            })
            .unwrap_or_default()
    }

    pub(super) fn save_setting(
        &mut self,
        definition: &settings_catalog::Setting,
        environment: &str,
        project: Option<&str>,
        value: Option<Value>,
        cx: &mut Context<Self>,
    ) {
        if !definition.server {
            self.change_setting(
                definition.key,
                value.unwrap_or_else(|| definition.default.clone()),
                cx,
            );
            return;
        }
        let patch = if let Some(project) = project {
            if !settings_catalog::project_scoped(definition.key) {
                self.conversation.message =
                    Some("Select All projects to change this environment setting.".into());
                cx.notify();
                return;
            }
            let config = self.config_of(environment).unwrap_or(Value::Null);
            let mut overrides = config["settings"]["projectSettingsOverrides"][project]
                .as_object()
                .cloned()
                .unwrap_or_default();
            if let Some(value) = value {
                overrides.insert(definition.key.to_string(), value);
            } else {
                overrides.remove(definition.key);
            }
            json!({"projectSettingsOverrides":{project:overrides}})
        } else {
            json!({definition.key:value.unwrap_or_else(||definition.default.clone())})
        };
        self.settings_request(
            environment,
            "server.updateSettings",
            json!({"patch":patch}),
            cx,
        );
    }

    pub(super) fn settings_request(
        &mut self,
        environment: &str,
        method: &'static str,
        payload: Value,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection(environment) else {
            self.conversation.message =
                Some("Connect an environment to change its settings.".into());
            cx.notify();
            return;
        };
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                connection.request(method, payload).await
            })
            .await;
            let _ = this.update(cx, |root, cx| {
                root.conversation.message = result
                    .err()
                    .map(|error| format!("Setting not saved: {error:#}"));
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_settings(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use gpui_kit::component::select::{Select, SelectEvent};
        if self
            .conversation
            .composer
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
        {
            self.settings_focus.focus(window, cx);
        }
        let theme = cx.theme().colors;
        let environment = self.settings_environment();
        let server = self.config_of(&environment).unwrap_or(Value::Null);
        let page = self.settings_page;
        let root = cx.entity().downgrade();
        let selected_environment = environment.clone();
        let choices = self
            .connections
            .iter()
            .map(|connection| {
                (
                    connection.label().to_string(),
                    connection.label().to_string(),
                )
            })
            .collect::<Vec<_>>();
        let scope = window.use_keyed_state(
            SharedString::from(format!("settings-scope:{environment}:{}", choices.len())),
            cx,
            |window, cx| {
                let state = settings_controls::select(choices, &selected_environment, window, cx);
                cx.subscribe_in(
                    &state,
                    window,
                    move |_, _, event: &SelectEvent<_>, _, cx| {
                        if let SelectEvent::Confirm(Some(environment)) = event {
                            let _ = root.update(cx, |root, cx| {
                                root.settings_environment = Some(environment.clone());
                                root.settings_project = None;
                                cx.notify();
                            });
                        }
                    },
                )
                .detach();
                state
            },
        );
        let scope = scope.read(cx).clone();
        let mut content = v_flex()
            .id("settings-content")
            .w_full()
            .max_w(px(896.))
            .mx_auto()
            .min_w_0()
            .px_6()
            .pt_6()
            .pb_12()
            .gap_6();
        let title = settings_catalog::PAGES
            .iter()
            .find(|(id, _, _)| *id == page)
            .map(|(_, label, _)| *label)
            .unwrap_or("Settings");
        if matches!(
            page,
            "providers"
                | "threads"
                | "scheduled-tasks"
                | "keybindings"
                | "maintenance"
                | "archived"
        ) {
            content = content.child(
                h_flex()
                    .gap_2()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Applying settings across")
                    .child(
                        Select::new(&scope)
                            .accessibility_label("Settings environment")
                            .w(px(200.))
                            .h(px(32.)),
                    ),
            );
        }
        if matches!(page, "threads" | "maintenance") {
            let root = cx.entity().downgrade();
            let mut projects = vec![(String::new(), "All projects".to_string())];
            if let Some(connection) = self.connection(&environment) {
                projects.extend(
                    connection
                        .shell()
                        .lock()
                        .expect("shell")
                        .projects
                        .iter()
                        .filter_map(|project| {
                            Some((
                                project["id"].as_str()?.to_string(),
                                project["title"].as_str()?.to_string(),
                            ))
                        }),
                );
            }
            let selected = self.settings_project.clone().unwrap_or_default();
            let selector = window.use_keyed_state(
                SharedString::from(format!(
                    "settings-project:{environment}:{selected}:{}",
                    projects.len()
                )),
                cx,
                |window, cx| {
                    let state = settings_controls::select(projects, &selected, window, cx);
                    cx.subscribe_in(
                        &state,
                        window,
                        move |_, _, event: &SelectEvent<_>, _, cx| {
                            if let SelectEvent::Confirm(Some(project)) = event {
                                let _ = root.update(cx, |root, cx| {
                                    root.settings_project =
                                        (!project.is_empty()).then(|| project.clone());
                                    cx.notify();
                                });
                            }
                        },
                    )
                    .detach();
                    state
                },
            );
            content = content.child(
                Select::new(selector.read(cx))
                    .accessibility_label("Settings project")
                    .w(px(260.)),
            );
        }
        if let Some(error) = &self.conversation.message {
            content = content.child(
                div()
                    .id("settings-error")
                    .role(accesskit::Role::Alert)
                    .text_color(theme.danger)
                    .child(error.clone()),
            );
        }
        let mut section = "";
        let group = || {
            v_flex()
                .rounded_lg()
                .overflow_hidden()
                .border_1()
                .border_color(theme.border)
                .bg(theme.muted.opacity(0.25))
        };
        let mut section_rows = group();
        for definition in settings_catalog::settings()
            .into_iter()
            .filter(|setting| setting.page == page)
        {
            if section != definition.section {
                if !section.is_empty() {
                    content = content.child(
                        v_flex()
                            .gap_3()
                            .child(
                                div()
                                    .px_4()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(section),
                            )
                            .child(section_rows),
                    );
                    section_rows = group();
                }
                section = definition.section;
            }
            if definition.server
                && self.settings_project.is_some()
                && !settings_catalog::project_scoped(definition.key)
            {
                continue;
            }
            let value = if definition.server {
                self.settings_project
                    .as_ref()
                    .and_then(|project| {
                        server["settings"]["projectSettingsOverrides"][project].get(definition.key)
                    })
                    .or_else(|| server["settings"].get(definition.key))
            } else {
                self.settings.values.get(definition.key)
            }
            .cloned()
            .unwrap_or_else(|| definition.default.clone());
            if definition.key == "sidebarAutoSettleAfterDays"
                && value.is_null()
                && matches!(definition.control, settings_catalog::Control::Number(..))
            {
                continue;
            }
            let root = cx.entity().downgrade();
            let environment = environment.clone();
            let control = window.use_keyed_state(
                SharedString::from(format!(
                    "setting:{}:{}:{}:{:?}:{}",
                    definition.key, definition.label, environment, self.settings_project, value
                )),
                cx,
                |window, cx| {
                    settings_controls::SettingRow::new(
                        root,
                        environment,
                        self.settings_project.clone(),
                        definition,
                        value,
                        window,
                        cx,
                    )
                },
            );
            section_rows = section_rows.child(control);
        }
        if !section.is_empty() {
            content = content.child(
                v_flex()
                    .gap_3()
                    .child(
                        div()
                            .px_4()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(section),
                    )
                    .child(section_rows),
            );
        }
        if page == "threads" && self.settings_project.is_none() {
            let root = cx.entity().downgrade();
            let saved = self.snooze_settings(&environment);
            let environment = environment.clone();
            let snooze = window.use_keyed_state(
                SharedString::from(format!("settings-snooze-options:{environment}")),
                cx,
                |window, cx| {
                    snooze_options::SnoozeOptions::new(root, environment, saved, window, cx)
                },
            );
            content = content.child(snooze);
        }
        if page == "keybindings" {
            let root = cx.entity().downgrade();
            let environment = environment.clone();
            let panel = window.use_keyed_state(
                SharedString::from(format!("settings-keybindings:{environment}")),
                cx,
                |window, cx| settings_keybindings::Keybindings::new(root, environment, window, cx),
            );
            content = content.child(panel);
        }
        if page == "providers" {
            let root = cx.entity().downgrade();
            let environment = environment.clone();
            let panel = window.use_keyed_state(
                SharedString::from(format!("settings-providers:{environment}")),
                cx,
                |window, cx| {
                    settings_providers::Providers::new(root, environment, self, window, cx)
                },
            );
            content = content.child(panel);
        }
        if page == "scheduled-tasks" {
            let root = cx.entity().downgrade();
            let environment = environment.clone();
            let panel = window.use_keyed_state(
                SharedString::from(format!("settings-tasks:{environment}")),
                cx,
                |window, cx| settings_tasks::Tasks::new(root, environment, self, window, cx),
            );
            content = content.child(panel);
        }
        if page == "connections" {
            content = content.child(self.render_host_content(cx));
        }
        if page == "archived" {
            let root = cx.entity().downgrade();
            let environment = environment.clone();
            let panel = window.use_keyed_state(
                SharedString::from(format!("settings-archive:{environment}")),
                cx,
                |_, cx| settings_archive::Archive::new(root, environment, cx),
            );
            content = content.child(panel);
        }
        if page == "about" {
            content = content.child(div().child(APP_NAME)).child(
                div()
                    .text_sm()
                    .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
            );
        }
        let navigation_focus = settings_catalog::PAGES
            .iter()
            .map(|(id, _, _)| {
                window
                    .use_keyed_state(
                        SharedString::from(format!("settings-nav-focus:{id}")),
                        cx,
                        |_, cx| cx.focus_handle(),
                    )
                    .read(cx)
                    .clone()
            })
            .collect::<Vec<_>>();
        let navigation = v_flex()
            .id("settings-navigation")
            .role(accesskit::Role::Tree)
            .aria_label("Settings sections")
            .w(px(240.))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .p_2()
            .gap_1()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .child(
                div()
                    .h(px(40.))
                    .px_3()
                    .py_2()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Settings"),
            )
            .children(settings_catalog::PAGES.iter().enumerate().map(
                |(index, &(id, label, group))| {
                    let focus = navigation_focus[index].clone();
                    let key_focus = navigation_focus.clone();
                    v_flex()
                        .when(!group.is_empty(), |item| {
                            item.child(
                                div()
                                    .mt_3()
                                    .px_2()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(group),
                            )
                        })
                        .child(
                            div()
                                .id(SharedString::from(format!("settings-{id}")))
                                .track_focus(&focus)
                                .role(accesskit::Role::TreeItem)
                                .aria_label(label)
                                .aria_selected(id == page)
                                .tab_stop(true)
                                .px_3()
                                .py_2()
                                .rounded_md()
                                .text_sm()
                                .cursor_pointer()
                                .when(id == page, |item| item.bg(theme.accent))
                                .hover(|style| style.bg(theme.accent))
                                .on_click(cx.listener(move |root, _, window, cx| {
                                    focus.focus(window, cx);
                                    root.settings_page = id;
                                    cx.notify();
                                }))
                                .on_key_down(cx.listener(
                                    move |root, event: &KeyDownEvent, window, cx| {
                                        if matches!(event.keystroke.key.as_str(), "enter" | "space")
                                        {
                                            root.settings_page = id;
                                            cx.notify();
                                        }
                                        if matches!(
                                            event.keystroke.key.as_str(),
                                            "up" | "down" | "home" | "end"
                                        ) {
                                            let pages = settings_catalog::PAGES;
                                            let index = pages
                                                .iter()
                                                .position(|(page, _, _)| *page == id)
                                                .unwrap_or(0);
                                            let index = match event.keystroke.key.as_str() {
                                                "up" => index.saturating_sub(1),
                                                "down" => (index + 1).min(pages.len() - 1),
                                                "home" => 0,
                                                _ => pages.len() - 1,
                                            };
                                            root.settings_page = pages[index].0;
                                            key_focus[index].focus(window, cx);
                                            cx.stop_propagation();
                                            cx.notify();
                                        }
                                    },
                                ))
                                .child(label),
                        )
                },
            ));
        div()
            .track_focus(&self.settings_focus)
            .absolute()
            .inset_0()
            .p(px(40.))
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.background.opacity(0.8))
            .child(
                h_flex()
                    .id("settings-dialog")
                    .role(accesskit::Role::Dialog)
                    .aria_label("Settings")
                    .size_full()
                    .max_w(px(1280.))
                    .max_h(px(960.))
                    .rounded(px(16.))
                    .overflow_hidden()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .child(navigation)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(
                                h_flex()
                                    .h(px(48.))
                                    .flex_none()
                                    .px_5()
                                    .gap_3()
                                    .border_b_1()
                                    .border_color(theme.border)
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(theme.muted_foreground)
                                            .child("Settings"),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(theme.muted_foreground)
                                            .child("/"),
                                    )
                                    .child(div().flex_1().text_sm().child(title))
                                    .child(
                                        Button::new("close-settings")
                                            .icon(gpui_kit::assets::IconName::X)
                                            .accessibility_label("Close")
                                            .ghost()
                                            .compact()
                                            .on_click(cx.listener(|root, _, window, cx| {
                                                root.settings_open = false;
                                                root.conversation
                                                    .composer
                                                    .update(cx, |input, cx| {
                                                        input.focus(window, cx)
                                                    });
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .child(
                                div()
                                    .id("settings-scroll")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .child(content),
                            ),
                    ),
            )
            .into_any_element()
    }
}
