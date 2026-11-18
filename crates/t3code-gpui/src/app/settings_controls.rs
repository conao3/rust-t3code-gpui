use super::settings_catalog::{Control, Setting};
use super::*;
use gpui_kit::component::{
    IndexPath,
    input::{NumberInput, NumberInputEvent, StepAction},
    select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
    slider::{SliderEvent, SliderState},
    switch::Switch,
};

#[derive(Clone)]
pub(super) struct Choice {
    pub value: String,
    pub label: SharedString,
}
impl SelectItem for Choice {
    type Value = String;
    fn title(&self) -> SharedString {
        self.label.clone()
    }
    fn value(&self) -> &String {
        &self.value
    }
}
pub(super) type Choices = SelectState<SearchableVec<Choice>>;

pub(super) fn select(
    values: Vec<(String, String)>,
    selected: &str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<Choices> {
    select_search(values, selected, false, window, cx)
}

fn select_search(
    values: Vec<(String, String)>,
    selected: &str,
    searchable: bool,
    window: &mut Window,
    cx: &mut App,
) -> Entity<Choices> {
    let index = values
        .iter()
        .position(|(value, _)| value == selected)
        .map(IndexPath::new);
    cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(
                values
                    .into_iter()
                    .map(|(value, label)| Choice {
                        value,
                        label: label.into(),
                    })
                    .collect::<Vec<_>>(),
            ),
            index,
            window,
            cx,
        )
        .searchable(searchable)
    })
}

fn select_value(key: &str, value: &Value) -> String {
    match key {
        "pullRequestMergeMethod" if value.is_null() => "last".to_string(),
        "diffFilesCollapsed" => if value == true {
            "collapsed"
        } else {
            "expanded"
        }
        .to_string(),
        _ => value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string()),
    }
}

pub(super) struct SettingRow {
    root: WeakEntity<Root>,
    environment: String,
    project: Option<String>,
    definition: Setting,
    value: Value,
    input: Entity<InputState>,
    select: Option<Entity<choice_select::State>>,
    font_family: Option<Entity<Choices>>,
    font_family_value: String,
    font_size: Option<Entity<Choices>>,
    slider: Option<Entity<SliderState>>,
    slider_focus: FocusHandle,
    subscriptions: Vec<Subscription>,
    validation_error: Option<String>,
}

impl SettingRow {
    pub(super) fn new(
        root: WeakEntity<Root>,
        environment: String,
        project: Option<String>,
        definition: Setting,
        value: (Value, String),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (value, font_family_value) = value;
        let text = value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string());
        let input = cx.new(|cx| {
            let input = InputState::new(window, cx).default_value(text);
            if let Control::Number(min, max) | Control::Font(_, min, max) = definition.control {
                input.min(min).max(max).step(1.)
            } else {
                input
            }
        });
        let select = match definition.control {
            Control::Select(options) => Some(choice_select::state(
                options,
                &select_value(definition.key, &value),
                window,
                cx,
            )),
            _ => None,
        };
        let font_family = if let Control::Font(key, _, _) = definition.control {
            let default = if key == "fontFamilySans" || key == "fontFamilyComposer" {
                UI_FONT.to_string()
            } else {
                crate::appearance::default_code_family(cx)
            };
            let mut fonts = cx.text_system().all_font_names();
            fonts.sort();
            fonts.dedup();
            if matches!(key, "fontFamilyCode" | "fontFamilyTerminal") {
                fonts.retain(|family| monospace_family(family, cx));
            }
            let mut options = vec![(String::new(), default.to_string())];
            options.extend(fonts.into_iter().map(|name| (name.clone(), name)));
            Some(select_search(options, &font_family_value, true, window, cx))
        } else {
            None
        };
        let font_size = if let Control::Font(_, min, max) = definition.control {
            Some(self::select(
                (min as i64..=max as i64)
                    .map(|size| (size.to_string(), format!("{size} px")))
                    .collect(),
                &value.to_string(),
                window,
                cx,
            ))
        } else {
            None
        };
        let slider = if let Control::Range(min, max, step) = definition.control {
            Some(cx.new(|_| {
                SliderState::new()
                    .min(min)
                    .max(max)
                    .step(step)
                    .default_value(value.as_f64().expect("range value") as f32)
            }))
        } else {
            None
        };
        let mut view = Self {
            root,
            environment,
            project,
            definition,
            value,
            input,
            select,
            font_family,
            font_family_value,
            font_size,
            slider,
            slider_focus: cx.focus_handle(),
            subscriptions: vec![],
            validation_error: None,
        };
        if let Some(slider) = &view.slider {
            view.subscriptions
                .push(cx.subscribe(slider, |view, _, event: &SliderEvent, cx| {
                    if let SliderEvent::Change(value) = event {
                        view.value = json!(value.start().round() as i64);
                        view.save(view.value.clone(), cx);
                        cx.notify();
                    }
                }));
        }
        view.subscriptions.push(
            cx.subscribe_in(&view.input, window, |view, _, event, _, cx| {
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    view.commit(cx);
                }
            }),
        );
        view.subscriptions.push(cx.subscribe_in(
            &view.input,
            window,
            |view, input, event: &NumberInputEvent, window, cx| {
                let NumberInputEvent::Step(action) = event;
                if let Control::Number(min, max) | Control::Font(_, min, max) =
                    view.definition.control
                {
                    let current = input
                        .read(cx)
                        .value()
                        .parse::<f64>()
                        .unwrap_or(view.value.as_f64().unwrap_or(min));
                    let next = (current
                        + if *action == StepAction::Increment {
                            1.
                        } else {
                            -1.
                        })
                    .clamp(min, max);
                    input.update(cx, |input, cx| {
                        input.set_value(next.to_string(), window, cx)
                    });
                    view.commit(cx);
                }
            },
        ));
        if let Some(font_family) = &view.font_family {
            view.subscriptions.push(cx.subscribe_in(
                font_family,
                window,
                |view, _, event: &SelectEvent<_>, _, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event
                        && let Control::Font(key, _, _) = view.definition.control
                    {
                        let _ = view
                            .root
                            .update(cx, |root, cx| root.change_setting(key, json!(value), cx));
                    }
                },
            ));
        }
        if let Some(font_size) = &view.font_size {
            view.subscriptions.push(cx.subscribe_in(
                font_size,
                window,
                |view, _, event: &SelectEvent<_>, _, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        view.save(json!(value.parse::<i64>().expect("font size option")), cx);
                    }
                },
            ));
        }
        if let Some(select) = &view.select {
            view.subscriptions.push(cx.subscribe_in(
                select,
                window,
                |view, _, event: &SelectEvent<_>, _, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        let value = match view.definition.key {
                            "pullRequestMergeMethod" if *value == "last" => Value::Null,
                            "diffFilesCollapsed" => json!(*value == "collapsed"),
                            "browserRecordingFrameRate" => {
                                json!(value.parse::<u32>().expect("frame rate option"))
                            }
                            _ => json!(*value),
                        };
                        view.save(value, cx);
                    }
                },
            ));
        }
        view
    }

    fn range_slider(
        &self,
        label: &'static str,
        cx: &Context<Self>,
    ) -> gpui_kit::base::slider::Slider {
        use gpui_kit::base::slider::{Slider, SliderIndicator, SliderThumb, SliderTrack};
        let state = self.slider.as_ref().expect("range state");
        let value = state.read(cx);
        let position =
            (value.value().end() - value.min_value()) / (value.max_value() - value.min_value());
        Slider::new(state)
            .track_focus(&self.slider_focus)
            .accessibility_label(label)
            .flex()
            .flex_1()
            .items_center()
            .child(
                SliderTrack::new(state)
                    .h_6()
                    .w_full()
                    .flex()
                    .items_center()
                    .child(
                        SliderIndicator::new(state)
                            .relative()
                            .w_full()
                            .h(px(4.))
                            .rounded_full()
                            .bg(cx.theme().border)
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .h_full()
                                    .w(relative(position))
                                    .rounded_full()
                                    .bg(cx.theme().primary),
                            )
                            .child(
                                SliderThumb::new(state)
                                    .absolute()
                                    .left(relative(position))
                                    .top(px(-4.))
                                    .ml(px(-6.))
                                    .size(px(12.))
                                    .rounded_full()
                                    .border_2()
                                    .border_color(cx.theme().primary)
                                    .bg(cx.theme().background),
                            ),
                    ),
            )
    }

    pub(super) fn sync(
        &mut self,
        value: Value,
        family: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.value != value {
            self.value = value.clone();
            self.validation_error = None;
            if let Some(slider) = &self.slider {
                slider.update(cx, |slider, cx| {
                    slider.set_value(value.as_f64().expect("range value") as f32, window, cx)
                });
            }
            if let Some(select) = &self.select
                && let Control::Select(options) = self.definition.control
                && let Some((selected, _)) = options
                    .iter()
                    .find(|(option, _)| *option == select_value(self.definition.key, &value))
            {
                select.update(cx, |select, cx| {
                    select.set_selected_value(selected, window, cx)
                });
            }
            if matches!(self.definition.control, Control::Text | Control::Number(..)) {
                let text = value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string());
                self.input
                    .update(cx, |input, cx| input.set_value(text, window, cx));
            }
            if let Some(size) = &self.font_size {
                size.update(cx, |size, cx| {
                    size.set_selected_value(&value.to_string(), window, cx)
                });
            }
            cx.notify();
        }
        if let Some(font) = &self.font_family
            && self.font_family_value != family
        {
            self.font_family_value = family.to_string();
            font.update(cx, |font, cx| {
                font.set_selected_value(&family.to_string(), window, cx)
            });
        }
    }

    fn commit(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().to_string();
        let value = match self.definition.control {
            Control::Number(min, max) | Control::Font(_, min, max) => match text.parse::<f64>() {
                Ok(number) if number.is_finite() && (min..=max).contains(&number) => {
                    json!(number.round() as i64)
                }
                _ => {
                    self.validation_error = Some(format!("Enter a number from {min} to {max}."));
                    cx.notify();
                    return;
                }
            },
            _ => json!(text),
        };
        if value != self.value {
            self.save(value, cx);
        }
    }

    fn save(&mut self, value: Value, cx: &mut Context<Self>) {
        self.validation_error = None;
        let definition = self.definition.clone();
        let environment = self.environment.clone();
        let _ = self.root.update(cx, |root, cx| {
            root.save_setting(
                &definition,
                &environment,
                self.project.as_deref(),
                Some(value),
                cx,
            )
        });
    }
    fn reset(&mut self, cx: &mut Context<Self>) {
        let _ = self.root.update(cx, |root, cx| {
            if let Control::Font(key, _, _) = self.definition.control {
                root.change_setting(key, json!(""), cx);
            }
            root.save_setting(
                &self.definition,
                &self.environment,
                self.project.as_deref(),
                None,
                cx,
            )
        });
    }
}

impl Render for SettingRow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = self.definition.label;
        let accessible_label = match self.definition.key {
            "wordWrap" => "Wrap code, diffs, and file previews by default",
            "timestampFormat" => "Timestamp format",
            "defaultAutoPull" => "Default automatic pull",
            "pullRequestMergeMethod" => "Default pull request merge method",
            "simlockDeviceHostsEnabled" => "Simlock device hosts",
            "confirmThreadUnpin" => "Confirm thread unpinning",
            "confirmThreadArchive" => "Confirm thread archiving",
            "confirmThreadDelete" => "Confirm thread deletion",
            "confirmQuit" => "Quit shortcut behavior",
            "voiceShortcutMode" => "Dictation shortcut mode",
            "diffIgnoreWhitespace" => "Hide whitespace changes by default",
            "proactivePanelsEnabled" => "Open pull requests and changes panels proactively",
            "enableAgentBrowserAccess" => "Agent browser access",
            "addProjectBaseDirectory" => "Add project base directory",
            "newWorktreesStartFromOrigin" => "Start new worktrees from remote by default",
            "fontSizeInterface" => "Interface font size",
            "fontSizeCode" => "Code font size",
            "fontSizePrompt" => "Prompt font size",
            "fontSizeTerminal" => "Terminal font size",
            _ => label,
        };
        let value = self.value.clone();
        let key = self.definition.key;
        let denial = (key == "guestSharingEnabled" && value != true)
            .then(|| {
                self.root.upgrade().and_then(|root| {
                    settings_labs::guest_links_denial(root.read(cx), &self.environment)
                })
            })
            .flatten();
        let description = if key == "defaultAutoPull" && self.project.is_some() {
            "Keeps this project's default branch current when the checkout has no local changes or commits.".to_string()
        } else if key == "pullRequestMergeMethod" && self.project.is_some() {
            "Pull requests in this project start with this method.".to_string()
        } else if key == "followUpBehavior" {
            let shortcut = self
                .root
                .upgrade()
                .map(|root| root.read(cx).settings.send_shortcut)
                .unwrap_or(client_settings::SendShortcut::Enter);
            format!(
                "Queue follow-ups while the agent runs or steer the current run. {}",
                match shortcut {
                    client_settings::SendShortcut::ModEnterMultiline =>
                        "Press Ctrl + Enter for single-line prompts or Ctrl + Shift + Enter for multiline prompts to do the opposite for one message.",
                    client_settings::SendShortcut::ModEnter =>
                        "Press Ctrl + Shift + Enter to do the opposite for one message.",
                    client_settings::SendShortcut::Enter =>
                        "Press Ctrl + Enter to do the opposite for one message.",
                }
            )
        } else if key == "responseStreamingMode" {
            if value == "turn" {
                "Text appears once the agent finishes its turn."
            } else {
                "Each paragraph or code block appears as soon as it is complete."
            }
            .to_string()
        } else {
            self.definition.description.to_string()
        };
        let control = match self.definition.control {
            Control::Switch => Switch::new(key)
                .accessibility_label(accessible_label)
                .disabled(denial.is_some())
                .checked(if key == "sidebarAutoSettleAfterDays" {
                    !value.is_null()
                } else if key == "sidebarProjectGroupingMode" {
                    value != "separate"
                } else {
                    value.as_bool().unwrap_or(false)
                })
                .on_change(cx.listener(move |view, checked, _, cx| {
                    let value = if key == "sidebarAutoSettleAfterDays" {
                        if *checked { json!(7) } else { Value::Null }
                    } else if key == "sidebarProjectGroupingMode" {
                        json!(if *checked { "repository" } else { "separate" })
                    } else {
                        json!(*checked)
                    };
                    view.save(value, cx);
                }))
                .into_any_element(),
            Control::Select(_) => Select::new(self.select.as_ref().expect("select state"))
                .accessibility_label(accessible_label)
                .w(px(match key {
                    "notificationMode" => 224.,
                    "sidebarProjectSortOrder" => 176.,
                    "screenReaderAnnouncements" => 192.,
                    "sendShortcut" if value == "mod-enter-multiline" => 288.,
                    "sendShortcut" if value == "mod-enter" => 208.,
                    _ => 160.,
                }))
                .h(px(28.))
                .into_any_element(),
            Control::Range(..) => h_flex()
                .id("range-keyboard")
                .capture_any_mouse_down(cx.listener(|view, event: &MouseDownEvent, window, cx| {
                    if event.button == MouseButton::Left {
                        view.slider_focus.focus(window, cx);
                    }
                }))
                .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                    let Control::Range(min, max, step) = view.definition.control else {
                        return;
                    };
                    let delta = match event.keystroke.key.as_str() {
                        "left" | "down" => -step,
                        "right" | "up" => step,
                        "home" => min - max,
                        "end" => max - min,
                        "pageup" => step * 10.,
                        "pagedown" => -step * 10.,
                        _ => return,
                    };
                    view.slider
                        .as_ref()
                        .expect("range state")
                        .update(cx, |slider, cx| slider.step_thumb(false, delta, window, cx));
                    cx.stop_propagation();
                }))
                .w(px(208.))
                .gap_3()
                .child(
                    h_flex()
                        .gap_0()
                        .w(px(48.))
                        .justify_center()
                        .text_xs()
                        .children(
                            [self.value.to_string(), "%".to_string()]
                                .into_iter()
                                .map(text),
                        ),
                )
                .child(self.range_slider(accessible_label, cx))
                .into_any_element(),
            Control::Number(_, _) => NumberInput::new(&self.input)
                .accessibility_label(accessible_label)
                .step_labels("Decrease", "Increase")
                .w(px(110.))
                .h(px(32.))
                .into_any_element(),
            Control::Text => Input::new(&self.input)
                .aria_label(accessible_label)
                .w(px(260.))
                .h(px(32.))
                .into_any_element(),
            Control::Font(_, _, _) => h_flex()
                .gap_2()
                .child(
                    Select::new(self.font_family.as_ref().expect("font family selector"))
                        .search_placeholder("Search fonts…")
                        .accessibility_label(format!("{label} family"))
                        .w(px(180.)),
                )
                .child(
                    Select::new(self.font_size.as_ref().expect("font size selector"))
                        .accessibility_label(accessible_label)
                        .w(px(88.)),
                )
                .into_any_element(),
        };
        h_flex()
            .id(key)
            .role(accesskit::Role::Group)
            .aria_label(label)
            .gap_4()
            .px_4()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(text(label).text_sm())
                    .child(
                        settings_labs::description(key, &self.root, cx).unwrap_or_else(|| {
                            text(description)
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .into_any_element()
                        }),
                    )
                    .when_some(denial, |view, denial| {
                        view.child(
                            text(denial)
                                .text_xs()
                                .text_color(cx.theme().muted_foreground),
                        )
                    }),
            )
            .when(
                self.project.is_some()
                    || self.value != self.definition.default
                    || self.font_family.as_ref().is_some_and(|font| {
                        font.read(cx)
                            .selected_value()
                            .is_some_and(|value| !value.is_empty())
                    }),
                |row| {
                    row.child(
                        Button::new("reset")
                            .icon(gpui_kit::assets::IconName::Undo2)
                            .accessibility_label(format!(
                                "Reset {} to default",
                                match key {
                                    "showSkillsInSlashMenu" => "skills in slash menu".to_string(),
                                    "defaultAutoPull" => "default automatic pull".to_string(),
                                    "removeAgentCreditsOnMerge" =>
                                        "agent credit removal".to_string(),
                                    _ => label.to_lowercase(),
                                }
                            ))
                            .ghost()
                            .compact()
                            .on_click(cx.listener(|view, _, _, cx| view.reset(cx))),
                    )
                },
            )
            .child(v_flex().gap_1().child(control).when_some(
                self.validation_error.clone(),
                |view, error| {
                    view.child(
                        div()
                            .id("invalid-setting")
                            .role(accesskit::Role::Alert)
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(error),
                    )
                },
            ))
    }
}

pub(super) fn request<T: 'static>(
    root: &WeakEntity<Root>,
    environment: &str,
    method: &'static str,
    payload: Value,
    cx: &mut Context<T>,
    finish: impl FnOnce(&mut T, anyhow::Result<Value>, &mut Context<T>) + 'static,
) {
    let owner = root.upgrade().expect("settings root");
    let owner = owner.read(cx);
    let connections = owner.settings_request_targets(environment, method);
    let runtime = owner.runtime.clone();
    cx.spawn(async move |this, cx| {
        let result = on_runtime(&runtime, async move {
            request_targets(connections, method, payload).await
        })
        .await;
        let _ = this.update(cx, |view, cx| {
            finish(view, result, cx);
            cx.notify();
        });
    })
    .detach();
}

pub(super) async fn request_targets(
    connections: Vec<Arc<Connection>>,
    method: &'static str,
    payload: Value,
) -> anyhow::Result<Value> {
    request_operations(
        connections
            .into_iter()
            .map(|connection| (connection, payload.clone()))
            .collect(),
        method,
    )
    .await
}

pub(super) async fn request_operations(
    operations: Vec<(Arc<Connection>, Value)>,
    method: &'static str,
) -> anyhow::Result<Value> {
    anyhow::ensure!(
        !operations.is_empty(),
        "Connect an environment to change its settings."
    );
    let mut result = Value::Null;
    let mut errors = Vec::new();
    for (connection, payload) in operations {
        match connection.request(method, payload).await {
            Ok(value) => result = value,
            Err(error) => errors.push(format!("{}: {error:#}", connection.label())),
        }
    }
    anyhow::ensure!(errors.is_empty(), "{}", errors.join("\n"));
    Ok(result)
}

pub(super) fn text(value: impl Into<SharedString>) -> gpui::Stateful<gpui::Div> {
    let value = value.into();
    div()
        .id(value.clone())
        .role(accesskit::Role::Label)
        .aria_value(value.clone())
        .child(value)
}

fn monospace_family(family: &str, cx: &App) -> bool {
    let base = font(family.to_string());
    [
        base.clone(),
        base.clone().bold(),
        base.clone().italic(),
        base.bold().italic(),
    ]
    .into_iter()
    .all(|font| {
        let id = cx.text_system().resolve_font(&font);
        let advances = ['i', 'M', 'W', '0', '@', '#', '.', ' ']
            .into_iter()
            .map(|character| {
                cx.text_system()
                    .advance(id, px(32.), character)
                    .map(|advance| f32::from(advance.width))
            })
            .collect::<anyhow::Result<Vec<_>>>();
        advances.is_ok_and(|advances| {
            advances[0] > 0.
                && advances
                    .iter()
                    .all(|advance| (*advance - advances[0]).abs() < 0.01)
        })
    })
}
