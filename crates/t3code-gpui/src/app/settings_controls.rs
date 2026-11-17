use super::settings_catalog::{Control, Setting};
use super::*;
use gpui_kit::component::{
    IndexPath,
    input::{NumberInput, NumberInputEvent, StepAction},
    select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
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
    })
}

pub(super) struct SettingRow {
    root: WeakEntity<Root>,
    environment: String,
    project: Option<String>,
    definition: Setting,
    value: Value,
    input: Entity<InputState>,
    select: Option<Entity<choice_select::State>>,
    subscriptions: Vec<Subscription>,
    validation_error: Option<String>,
}

impl SettingRow {
    pub(super) fn new(
        root: WeakEntity<Root>,
        environment: String,
        project: Option<String>,
        definition: Setting,
        value: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let text = value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string());
        let input = cx.new(|cx| {
            let input = InputState::new(window, cx).default_value(text);
            if let Control::Number(min, max) = definition.control {
                input.min(min).max(max).step(1.)
            } else {
                input
            }
        });
        let select = match definition.control {
            Control::Select(options) => Some(choice_select::state(
                options,
                &match definition.key {
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
                },
                window,
                cx,
            )),
            _ => None,
        };
        let mut view = Self {
            root,
            environment,
            project,
            definition,
            value,
            input,
            select,
            subscriptions: vec![],
            validation_error: None,
        };
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
                if let Control::Number(min, max) = view.definition.control {
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
        if let Some(select) = &view.select {
            view.subscriptions.push(cx.subscribe_in(
                select,
                window,
                |view, _, event: &SelectEvent<_>, _, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        let value = match view.definition.key {
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

    fn commit(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().to_string();
        let value = match self.definition.control {
            Control::Number(min, max) => match text.parse::<f64>() {
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
            "timestampFormat" => "Timestamp format",
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
        let description = if key == "followUpBehavior" {
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
                        text(description)
                            .text_xs()
                            .text_color(cx.theme().muted_foreground),
                    ),
            )
            .when(
                self.project.is_some() || self.value != self.definition.default,
                |row| {
                    row.child(
                        Button::new("reset")
                            .icon(gpui_kit::assets::IconName::Undo2)
                            .accessibility_label(format!(
                                "Reset {} to default",
                                if key == "showSkillsInSlashMenu" {
                                    "skills in slash menu".to_string()
                                } else {
                                    label.to_lowercase()
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
