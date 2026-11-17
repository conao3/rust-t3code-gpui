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
                    self.message = Some(format!("Timestamp format failed: {error:#}"));
                }
                if key == "gpuiAppearance" {
                    crate::appearance::apply(self.settings.light_theme, cx);
                }
                Theme::update(cx, |theme| {
                    theme.font_size = px(interface);
                    theme.mono_font_size = px(code);
                });
            }
            Err(error) => self.message = Some(format!("Saving the settings failed: {error:#}")),
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
                self.message =
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
            self.message = Some("Connect an environment to change its settings.".into());
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
                root.message = result
                    .err()
                    .map(|error| format!("Setting not saved: {error:#}"));
                cx.notify();
            });
        })
        .detach();
    }

    fn restore_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let changed = settings_catalog::settings()
            .into_iter()
            .filter(|setting| {
                !setting.server
                    && self
                        .settings
                        .values
                        .get(setting.key)
                        .is_some_and(|value| *value != setting.default)
            })
            .collect::<Vec<_>>();
        if changed.is_empty() {
            return;
        }
        let labels = changed
            .iter()
            .map(|setting| setting.label)
            .collect::<Vec<_>>()
            .join(", ");
        let owner = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let owner = owner.clone();
            let changed = changed.clone();
            dialog
                .title("Restore default settings?")
                .description(format!("This will reset: {labels}."))
                .confirm()
                .on_ok(move |_, _, cx| {
                    let _ = owner.update(cx, |root, cx| {
                        for setting in &changed {
                            root.change_setting(setting.key, setting.default.clone(), cx);
                        }
                    });
                    true
                })
        });
    }

    fn open_settings_result(
        &mut self,
        setting: &settings_catalog::Setting,
        search: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_page = setting.page;
        if setting.advanced {
            window
                .use_keyed_state(
                    SharedString::from(format!(
                        "settings-advanced:{}:{}",
                        setting.page, setting.section
                    )),
                    cx,
                    |_, _| false,
                )
                .update(cx, |expanded, cx| {
                    *expanded = true;
                    cx.notify();
                });
        }
        search.update(cx, |input, cx| input.set_value("", window, cx));
        self.settings_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn render_settings(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use gpui_kit::component::select::{Select, SelectEvent};
        if self.composer.read(cx).focus_handle(cx).is_focused(window) {
            self.settings_focus.focus(window, cx);
        }
        let search_root = cx.entity().downgrade();
        let search = window
            .use_keyed_state("settings-search-input", cx, |window, cx| {
                let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
                cx.subscribe(&input, move |_, _, _: &InputEvent, cx| {
                    let _ = search_root.update(cx, |_, cx| cx.notify());
                })
                .detach();
                input
            })
            .read(cx)
            .clone();
        let query = search.read(cx).value().trim().to_lowercase();
        let results = settings_catalog::settings()
            .into_iter()
            .filter(|setting| {
                let searchable =
                    format!("{} {} {}", setting.label, setting.description, setting.page)
                        .to_lowercase();
                query
                    .split_whitespace()
                    .all(|word| searchable.contains(word))
            })
            .collect::<Vec<_>>();
        let search_index = window.use_keyed_state(
            SharedString::from(format!("settings-search-index:{query}")),
            cx,
            |_, _| 0usize,
        );
        let active_result = (*search_index.read(cx)).min(results.len().saturating_sub(1));
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
            .gap_8();
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
                | "browser"
                | "devices"
                | "voice"
                | "worktrees"
                | "labs"
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
        if matches!(
            page,
            "threads" | "maintenance" | "archived" | "projects" | "worktrees"
        ) {
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
        if let Some(error) = &self.message {
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
                .rounded(px(12.))
                .overflow_hidden()
                .border_1()
                .border_color(theme.border)
                .bg(theme.muted.opacity(0.25))
        };
        let mut section_rows = group();
        let mut advanced_started = false;
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
                                settings_controls::text(section)
                                    .px_4()
                                    .font_weight(FontWeight::SEMIBOLD),
                            )
                            .child(section_rows),
                    );
                    section_rows = group();
                }
                section = definition.section;
                advanced_started = false;
            }
            if definition.key == "branchNamePrefix"
                && server["settings"]["branchNamingMode"] != "static"
                || definition.key == "branchNameInstructions"
                    && server["settings"]["branchNamingMode"] != "custom"
            {
                continue;
            }
            if definition.key == "boringSidebarMultilineRows"
                && self.settings.values["boringSidebarEnabled"] != true
            {
                continue;
            }
            if definition.advanced {
                let expanded = window.use_keyed_state(
                    SharedString::from(format!("settings-advanced:{page}:{section}")),
                    cx,
                    |_, _| false,
                );
                let open = *expanded.read(cx);
                if !advanced_started {
                    let toggle = expanded.clone();
                    section_rows = section_rows.child(
                        Button::new(SharedString::from(format!("advanced-{section}")))
                            .accessibility_label("Advanced")
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .text_xs()
                                    .child(
                                        Icon::new(if open {
                                            gpui_kit::assets::IconName::ChevronDown
                                        } else {
                                            gpui_kit::assets::IconName::ChevronRight
                                        })
                                        .size(px(12.)),
                                    )
                                    .child("Advanced"),
                            )
                            .ghost()
                            .aria_expanded(open)
                            .w_full()
                            .h(px(40.))
                            .justify_start()
                            .px_4()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .on_click(cx.listener(move |_, _, _, cx| {
                                toggle.update(cx, |value, cx| {
                                    *value = !*value;
                                    cx.notify();
                                });
                                cx.notify();
                            })),
                    );
                    advanced_started = true;
                }
                if !open {
                    continue;
                }
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
                        settings_controls::text(section)
                            .px_4()
                            .font_weight(FontWeight::SEMIBOLD),
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
        let nav_scroll = window
            .use_keyed_state(
                SharedString::from(format!("settings-nav-scroll:{query}")),
                cx,
                |_, _| ScrollHandle::new(),
            )
            .read(cx)
            .clone();
        let navigation = v_flex()
            .id("settings-navigation")
            .track_scroll(&nav_scroll)
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
                h_flex()
                    .h(px(40.))
                    .gap_2()
                    .px_2()
                    .child(
                        gpui_kit::component::Icon::new(gpui_kit::assets::IconName::Search)
                            .size(px(14.))
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        Input::new(&search)
                            .role(accesskit::Role::ComboBox)
                            .aria_label("Search settings")
                            .appearance(false)
                            .flex_1()
                            .h(px(32.)),
                    )
                    .when(query.is_empty(), |row| {
                        row.child(
                            settings_controls::text("/")
                                .text_xs()
                                .px_1()
                                .rounded_sm()
                                .bg(theme.muted)
                                .text_color(theme.muted_foreground),
                        )
                    })
                    .when(!query.is_empty(), |row| {
                        let search = search.clone();
                        row.child(
                            Button::new("clear-settings-search")
                                .icon(gpui_kit::assets::IconName::X)
                                .accessibility_label("Clear settings search")
                                .ghost()
                                .compact()
                                .on_click(cx.listener(move |_, _, window, cx| {
                                    search.update(cx, |input, cx| {
                                        input.set_value("", window, cx);
                                        input.focus(window, cx);
                                    });
                                })),
                        )
                    }),
            )
            .children(
                settings_catalog::PAGES
                    .iter()
                    .enumerate()
                    .filter(|_| query.is_empty())
                    .map(|(index, &(id, label, group))| {
                        let focus = navigation_focus[index].clone();
                        let key_focus = navigation_focus.clone();
                        let scroll = nav_scroll.clone();
                        let key_scroll = nav_scroll.clone();
                        let search = search.clone();
                        v_flex()
                            .when(!group.is_empty() && query.is_empty(), |item| {
                                if group.trim().is_empty() {
                                    item.mt_4()
                                } else {
                                    item.child(
                                        settings_controls::text(group.trim())
                                            .mt_4()
                                            .px_2()
                                            .text_xs()
                                            .text_color(theme.muted_foreground),
                                    )
                                }
                            })
                            .child(
                                div()
                                    .id(SharedString::from(format!("settings-{id}")))
                                    .track_focus(&focus)
                                    .role(accesskit::Role::TreeItem)
                                    .aria_label(label)
                                    .aria_selected(id == page)
                                    .on_a11y_action(
                                        accesskit::Action::ScrollIntoView,
                                        move |_, window, _| {
                                            scroll.scroll_to_item(index + 1);
                                            window.refresh();
                                        },
                                    )
                                    .tab_stop(true)
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .px_2()
                                    .h(px(32.))
                                    .rounded_md()
                                    .text_sm()
                                    .text_color(if id == page {
                                        theme.foreground
                                    } else {
                                        theme.muted_foreground
                                    })
                                    .cursor_pointer()
                                    .when(id == page, |item| item.bg(theme.accent))
                                    .hover(|style| style.bg(theme.accent))
                                    .on_click(cx.listener(move |root, _, window, cx| {
                                        focus.focus(window, cx);
                                        root.settings_page = id;
                                        search.update(cx, |input, cx| {
                                            input.set_value("", window, cx)
                                        });
                                        cx.notify();
                                    }))
                                    .on_key_down(cx.listener(
                                        move |root, event: &KeyDownEvent, window, cx| {
                                            if matches!(
                                                event.keystroke.key.as_str(),
                                                "enter" | "space"
                                            ) {
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
                                                key_scroll.scroll_to_item(index + 1);
                                                root.settings_page = pages[index].0;
                                                key_focus[index].focus(window, cx);
                                                cx.stop_propagation();
                                                cx.notify();
                                            }
                                        },
                                    ))
                                    .child(
                                        gpui_kit::component::Icon::new(
                                            settings_catalog::page_icon(id),
                                        )
                                        .size(px(16.))
                                        .text_color(theme.muted_foreground),
                                    )
                                    .child(label),
                            )
                    }),
            )
            .when(!query.is_empty(), |navigation| {
                navigation.child(
                    v_flex()
                        .id("settings-search-results")
                        .role(accesskit::Role::ListBox)
                        .aria_label("Settings search results")
                        .when(results.is_empty(), |list| {
                            list.child(settings_controls::text("No settings found"))
                        })
                        .children(results.iter().enumerate().map(|(index, setting)| {
                            let setting = setting.clone();
                            let search = search.clone();
                            let page_label = settings_catalog::PAGES
                                .iter()
                                .find(|(page, _, _)| *page == setting.page)
                                .map(|(_, label, _)| *label)
                                .unwrap_or(setting.page);
                            h_flex()
                                .id(("settings-search-result", index))
                                .role(accesskit::Role::ListBoxOption)
                                .aria_label(format!("{} {}", setting.label, page_label))
                                .aria_selected(index == active_result)
                                .gap_2()
                                .p_2()
                                .min_h(px(40.))
                                .cursor_pointer()
                                .when(index == active_result, |row| row.bg(theme.accent))
                                .child(settings_catalog::page_icon(setting.page).size(px(16.)))
                                .child(
                                    v_flex()
                                        .child(settings_controls::text(setting.label).text_sm())
                                        .child(
                                            settings_controls::text(page_label)
                                                .text_xs()
                                                .text_color(theme.muted_foreground),
                                        ),
                                )
                                .on_click(cx.listener(move |root, _, window, cx| {
                                    root.open_settings_result(&setting, &search, window, cx)
                                }))
                        })),
                )
            });
        div()
            .track_focus(&self.settings_focus)
            .capture_key_down(cx.listener(move |root, event: &KeyDownEvent, window, cx| {
                if search.read(cx).focus_handle(cx).is_focused(window) && !query.is_empty() {
                    match event.keystroke.key.as_str() {
                        "escape" => search.update(cx, |input, cx| input.set_value("", window, cx)),
                        "enter" => {
                            if let Some(setting) = results.get(active_result) {
                                root.open_settings_result(setting, &search, window, cx);
                            }
                        }
                        "up" | "down" => {
                            search_index.update(cx, |index, _| {
                                *index = if event.keystroke.key == "up" {
                                    index.saturating_sub(1)
                                } else {
                                    (*index + 1).min(results.len().saturating_sub(1))
                                };
                            });
                            cx.notify();
                        }
                        _ => return,
                    }
                    cx.stop_propagation();
                    return;
                }
                if (event.keystroke.key == "/"
                    && !window
                        .context_stack()
                        .iter()
                        .any(|context| context.contains("Input")))
                    || (event.keystroke.key == "f" && event.keystroke.modifiers.control)
                {
                    search.update(cx, |input, cx| input.focus(window, cx));
                    cx.stop_propagation();
                }
            }))
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
                            .id("settings-content-area")
                            .role(accesskit::Role::Region)
                            .aria_label("Settings content")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(
                                h_flex()
                                    .id("settings-breadcrumb")
                                    .role(accesskit::Role::Navigation)
                                    .aria_label("Settings breadcrumb")
                                    .h(px(48.))
                                    .flex_none()
                                    .px_5()
                                    .gap_3()
                                    .border_b_1()
                                    .border_color(theme.border)
                                    .child(
                                        settings_controls::text("Settings")
                                            .text_sm()
                                            .text_color(theme.muted_foreground),
                                    )
                                    .child(
                                        div()
                                            .child("/")
                                            .text_sm()
                                            .text_color(theme.muted_foreground),
                                    )
                                    .child(settings_controls::text(title).flex_1().text_sm())
                                    .when(page == "general", |header| {
                                        header.child(
                                            Button::new("restore-device-defaults")
                                                .label("Restore device defaults")
                                                .icon(gpui_kit::assets::IconName::RotateCcw)
                                                .ghost()
                                                .compact()
                                                .disabled(!settings_catalog::settings().iter().any(
                                                    |setting| {
                                                        !setting.server
                                                            && self
                                                                .settings
                                                                .values
                                                                .get(setting.key)
                                                                .is_some_and(|value| {
                                                                    *value != setting.default
                                                                })
                                                    },
                                                ))
                                                .on_click(cx.listener(|root, _, window, cx| {
                                                    root.restore_settings(window, cx)
                                                })),
                                        )
                                    })
                                    .child(
                                        Button::new("close-settings")
                                            .icon(gpui_kit::assets::IconName::X)
                                            .accessibility_label("Close")
                                            .ghost()
                                            .compact()
                                            .on_click(cx.listener(|root, _, window, cx| {
                                                root.settings_open = false;
                                                root.composer.update(cx, |input, cx| {
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
