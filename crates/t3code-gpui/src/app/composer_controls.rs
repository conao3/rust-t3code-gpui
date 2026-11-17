use super::*;
use gpui_kit::component::{
    IndexPath,
    popover::Popover,
    select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
};
#[derive(Clone)]
struct Choice {
    value: String,
    label: SharedString,
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
type State = SelectState<SearchableVec<Choice>>;

#[derive(Clone)]
struct ContextChoice {
    kind: &'static str,
    value: String,
    label: String,
    checked: bool,
}

pub(super) struct Controls {
    key: Option<(String, String, Value)>,
    choices: Vec<ContextChoice>,
    host: Option<Entity<State>>,
    workspace: Option<Entity<State>>,
    subscriptions: Vec<Subscription>,
    open: bool,
    cursor: usize,
    focus: FocusHandle,
    pub(super) effort_focus: FocusHandle,
    pub(super) effort_cursor: Option<Value>,
}
impl Controls {
    pub(super) fn menu_stamp(&self) -> (bool, usize) {
        (self.open, self.cursor)
    }

    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            key: None,
            choices: vec![],
            host: None,
            workspace: None,
            subscriptions: vec![],
            open: false,
            cursor: 0,
            focus: cx.focus_handle(),
            effort_focus: cx.focus_handle(),
            effort_cursor: None,
        }
    }
}

fn select(
    choices: Vec<(String, String)>,
    selected: &str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<State> {
    let index = choices
        .iter()
        .position(|(value, _)| value == selected)
        .map(IndexPath::new);
    let items = SearchableVec::new(
        choices
            .into_iter()
            .map(|(value, label)| Choice {
                value,
                label: label.into(),
            })
            .collect::<Vec<_>>(),
    );
    cx.new(|cx| SelectState::new(items, index, window, cx))
}

impl Root {
    pub(super) fn sync_composer_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self
            .draft_project()
            .zip(self.conversation.draft.as_ref())
            .map(|((env, project), draft)| (env, project, draft.workspace_strategy.clone()));
        if self.conversation.composer_choices.key == key {
            return;
        }
        self.conversation.composer_choices.key = key.clone();
        self.conversation.composer_choices.choices.clear();
        self.conversation.composer_choices.host = None;
        self.conversation.composer_choices.workspace = None;
        self.conversation.composer_choices.subscriptions.clear();
        self.conversation.composer_choices.open = false;
        let Some((environment, _, strategy)) = key else {
            return;
        };
        let mut choices = vec![];
        if self.connections.len() > 1
            && let Some(group) = self.project_groups().iter().find(|g| {
                self.conversation
                    .draft
                    .as_ref()
                    .and_then(|d| d.group.as_ref())
                    == Some(&g.key)
            })
        {
            for member in &group.members {
                choices.push(ContextChoice {
                    kind: "host",
                    value: member.environment.clone(),
                    label: member.environment.clone(),
                    checked: member.environment == environment,
                });
            }
        }
        choices.push(ContextChoice {
            kind: "mode",
            value: "root".into(),
            label: if strategy["type"] == "existing_worktree" {
                "Current worktree"
            } else {
                "Current checkout"
            }
            .into(),
            checked: strategy["type"] != "worktree",
        });
        choices.push(ContextChoice {
            kind: "mode",
            value: "worktree".into(),
            label: "New worktree".into(),
            checked: strategy["type"] == "worktree",
        });
        if let Some(previous) = self.previous_worktree() {
            choices.push(ContextChoice {
                kind: "mode",
                value: "previous".into(),
                label: previous["branch"]
                    .as_str()
                    .map(|branch| format!("Previous worktree ({branch})"))
                    .unwrap_or_else(|| "Previous worktree".into()),
                checked: false,
            });
        }
        self.conversation.composer_choices.cursor = choices
            .iter()
            .position(|choice| choice.checked)
            .unwrap_or(0);
        let pane_id = self.conversation.id;
        for (kind, name) in [("host", "host"), ("mode", "workspace")] {
            let items: Vec<_> = choices
                .iter()
                .filter(|choice| choice.kind == kind)
                .collect();
            if items.is_empty() {
                continue;
            }
            let selected = items
                .iter()
                .find(|choice| choice.checked)
                .map(|choice| choice.value.as_str())
                .unwrap_or_default();
            let state = select(
                items
                    .iter()
                    .map(|choice| (choice.value.clone(), choice.label.clone()))
                    .collect(),
                selected,
                window,
                cx,
            );
            self.conversation
                .composer_choices
                .subscriptions
                .push(cx.subscribe_in(
                    &state,
                    window,
                    move |root, _, event: &SelectEvent<_>, window, cx| {
                        root.with_conversation(pane_id, |root| {
                            if let SelectEvent::Confirm(Some(value)) = event
                                && let Some(index) =
                                    root.composer_choices.choices.iter().position(|choice| {
                                        choice.kind == kind && &choice.value == value
                                    })
                            {
                                root.choose_context(index, window, cx);
                            }
                        });
                    },
                ));
            if name == "host" {
                self.conversation.composer_choices.host = Some(state);
            } else {
                self.conversation.composer_choices.workspace = Some(state);
            }
        }
        self.conversation.composer_choices.choices = choices;
    }

    pub(super) fn render_composer_context(&self, cx: &Context<Self>) -> AnyElement {
        h_flex()
            .gap_1()
            .when(!self.compact_composer_context(), |row| {
                row.children(
                    self.conversation
                        .composer_choices
                        .host
                        .as_ref()
                        .map(|state| {
                            Select::new(state)
                                .accessibility_label("Run on")
                                .appearance(false)
                                .w(px(110.))
                                .small()
                        }),
                )
            })
            .when(
                self.conversation.workspace_status.is_repo() || self.connections.len() > 1,
                |row| row.child(self.render_workspace_mode(cx)),
            )
            .into_any_element()
    }

    pub(super) fn composer_width(&self) -> f32 {
        self.pane_layout
            .width(
                self.conversation.id,
                self.canvas_width
                    - workspace::layout(self.canvas_width, self.details_open).inset_end,
            )
            .expect("composer pane is in the layout")
    }

    fn compact_composer_context(&self) -> bool {
        self.composer_width() - 48. < 576.
    }

    pub(super) fn render_workspace_select(&self, cx: &Context<Self>) -> AnyElement {
        if !self.compact_composer_context() {
            return self
                .conversation
                .composer_choices
                .workspace
                .as_ref()
                .map(|state| {
                    Select::new(state)
                        .accessibility_label("Workspace")
                        .appearance(false)
                        .w(px(165.))
                        .small()
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element());
        }
        let root = self.pane_root(cx);
        let label = self
            .conversation
            .composer_choices
            .choices
            .iter()
            .find(|choice| {
                choice.checked
                    && choice.kind
                        == if self.connections.len() > 1 {
                            "host"
                        } else {
                            "mode"
                        }
            })
            .map(|choice| choice.label.clone())
            .unwrap_or_default();
        Popover::new("composer-run-context")
            .anchor(Anchor::BottomLeft)
            .open(self.conversation.composer_choices.open)
            .track_focus(&self.conversation.composer_choices.focus)
            .on_open_change(self.pane_listener(cx, |root, open, _, cx| {
                root.conversation.composer_choices.open = *open;
                cx.notify();
            }))
            .trigger(
                Button::new("composer-run-context-trigger")
                    .label(label.clone())
                    .accessibility_label(label)
                    .icon(IconName::Folder)
                    .small()
                    .ghost()
                    .compact(),
            )
            .content(move |_, _, cx| {
                root.update(cx, |root, cx| root.render_context_menu(cx))
                    .expect("composer root is alive")
            })
            .into_any_element()
    }

    fn render_context_menu(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let mut menu = v_flex()
            .id("composer-context-menu")
            .role(accesskit::Role::Menu)
            .aria_label("Run context")
            .min_w(px(190.))
            .p_1()
            .gap_0p5();
        let mut previous_kind = None;
        for (index, choice) in self
            .conversation
            .composer_choices
            .choices
            .iter()
            .enumerate()
        {
            if previous_kind != Some(choice.kind) {
                menu = menu.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(if choice.kind == "host" {
                            "Run on"
                        } else {
                            "Workspace"
                        }),
                );
                previous_kind = Some(choice.kind);
            }
            menu = menu.child(
                h_flex()
                    .id(("context-choice", index))
                    .role(accesskit::Role::MenuItemRadio)
                    .aria_label(choice.label.clone())
                    .aria_toggled(if choice.checked {
                        accesskit::Toggled::True
                    } else {
                        accesskit::Toggled::False
                    })
                    .when(index == self.conversation.composer_choices.cursor, |row| {
                        row.track_focus(&self.conversation.composer_choices.focus)
                            .bg(theme.list_active)
                    })
                    .px_2()
                    .h(px(28.))
                    .gap_2()
                    .rounded_sm()
                    .text_sm()
                    .cursor_pointer()
                    .child(choice.label.clone())
                    .child(div().flex_1())
                    .when(choice.checked, |row| {
                        row.child(Icon::new(IconName::Check).size(px(14.)))
                    })
                    .on_click(self.pane_listener(cx, move |root, _, window, cx| {
                        root.choose_context(index, window, cx)
                    }))
                    .on_key_down(self.pane_listener(
                        cx,
                        |root, event: &KeyDownEvent, window, cx| {
                            match event.keystroke.key.as_str() {
                                "up" | "down" => {
                                    let count = root.conversation.composer_choices.choices.len();
                                    if count > 0 {
                                        root.conversation.composer_choices.cursor =
                                            if event.keystroke.key == "down" {
                                                (root.conversation.composer_choices.cursor + 1)
                                                    % count
                                            } else {
                                                (root.conversation.composer_choices.cursor + count
                                                    - 1)
                                                    % count
                                            };
                                    }
                                    cx.notify();
                                }
                                "enter" | "space" => root.choose_context(
                                    root.conversation.composer_choices.cursor,
                                    window,
                                    cx,
                                ),
                                "escape" => {
                                    root.conversation.composer_choices.open = false;
                                    root.conversation
                                        .composer
                                        .update(cx, |state, cx| state.focus(window, cx));
                                    cx.notify();
                                }
                                _ => return,
                            }
                            cx.stop_propagation();
                        },
                    )),
            );
        }
        menu.into_any_element()
    }

    fn choose_context(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(choice) = self
            .conversation
            .composer_choices
            .choices
            .get(index)
            .cloned()
        else {
            return;
        };
        self.conversation.composer_choices.open = false;
        if choice.value == "previous" {
            self.use_previous_worktree(window, cx);
        } else if !(choice.value == "root"
            && self
                .conversation
                .draft
                .as_ref()
                .is_some_and(|draft| draft.workspace_strategy["type"] == "existing_worktree"))
        {
            self.workspace_action(
                &WorkspaceAction {
                    kind: choice.kind.into(),
                    value: json!(choice.value),
                },
                window,
                cx,
            );
        }
        self.conversation
            .composer
            .update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub(super) fn open_composer_select(
        &mut self,
        host: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_composer_controls(window, cx);
        if self.conversation.draft.is_none()
            || self.conversation.composer_choices.choices.is_empty()
        {
            return;
        }
        if !self.compact_composer_context() {
            let state = if host {
                &self.conversation.composer_choices.host
            } else {
                &self.conversation.composer_choices.workspace
            };
            if let Some(state) = state {
                state.update(cx, |state, cx| state.open(window, cx));
            }
            return;
        }
        self.conversation.composer_choices.open = !self.conversation.composer_choices.open;
        if self.conversation.composer_choices.open {
            self.conversation.composer_choices.focus.focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn cycle_composer_host(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((environment, _)) = self.draft_project() else {
            return;
        };
        let groups = self.project_groups();
        let Some(group) = groups.iter().find(|g| {
            self.conversation
                .draft
                .as_ref()
                .and_then(|d| d.group.as_ref())
                == Some(&g.key)
        }) else {
            return;
        };
        if group.members.is_empty() {
            return;
        }
        let index = group
            .members
            .iter()
            .position(|m| m.environment == environment)
            .unwrap_or(0);
        let next = group.members[(index + 1) % group.members.len()]
            .environment
            .clone();
        self.workspace_action(
            &WorkspaceAction {
                kind: "host".into(),
                value: json!(next),
            },
            window,
            cx,
        );
    }

    fn previous_worktree(&self) -> Option<Value> {
        let (env, project) = self.draft_project()?;
        let connection = self.connection(&env)?;
        let shell = connection.shell();
        let shell = shell.lock().expect("shell");
        let current = self.conversation.draft.as_ref()?.workspace_strategy["worktreePath"].as_str();
        shell.threads.iter().filter(|t| t["projectId"].as_str() == Some(&project)
            && t["worktreePath"].as_str().is_some_and(|path| !path.is_empty() && Some(path) != current)
            && t["archivedAt"].is_null())
            .filter_map(|t| chrono::DateTime::parse_from_rfc3339(t["updatedAt"].as_str()?).ok().map(|date| (date, t)))
            .max_by_key(|(date, _)| *date)
            .map(|(_, t)| json!({"type":"existing_worktree", "worktreePath":t["worktreePath"], "branch":t["branch"]}))
    }

    pub(super) fn use_previous_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.conversation.workspace_status.is_repo() {
            return;
        }
        if let Some(previous) = self.previous_worktree()
            && let Some(draft) = self.conversation.draft.as_mut()
        {
            draft.workspace_strategy = previous;
            self.conversation
                .composer
                .update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
        }
    }

    pub(super) fn open_effort(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.conversation.picker_open {
            self.toggle_model_picker(window, cx);
        }
        self.conversation.composer_choices.effort_cursor = None;
        let has_options =
            self.composer_target()
                .is_some_and(|(environment, selection, ..)| {
                    let Some(selection) = selection else {
                        return false;
                    };
                    self.model_catalogs
                        .get(&environment)
                        .is_some_and(|(_, instances)| {
                            instances
                                .iter()
                                .find(|instance| {
                                    selection["instanceId"].as_str() == Some(&instance.instance_id)
                                })
                                .and_then(|instance| {
                                    instance.models.iter().find(|model| {
                                        selection["model"].as_str() == Some(&model.slug)
                                    })
                                })
                                .is_some_and(|model| {
                                    model.option_descriptors.iter().any(|descriptor| {
                                        (descriptor["role"] == "reasoning"
                                            || descriptor["id"] == "reasoningEffort")
                                            && descriptor["options"]
                                                .as_array()
                                                .is_some_and(|options| !options.is_empty())
                                    })
                                })
                        })
                });
        if has_options {
            self.conversation
                .composer_choices
                .effort_focus
                .focus(window, cx);
        } else {
            self.conversation
                .model_picker
                .update(cx, |state, cx| state.focus(window, cx));
        }
    }

    pub(super) fn step_effort(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_question().is_some() || self.composer_composing(window, cx) {
            return;
        }
        let Some((environment, Some(selection), ..)) = self.composer_target() else {
            return;
        };
        let Some((_, instances)) = self.model_catalogs.get(&environment) else {
            return;
        };
        let descriptor = instances
            .iter()
            .find(|i| selection["instanceId"].as_str() == Some(&i.instance_id))
            .and_then(|i| {
                i.models
                    .iter()
                    .find(|m| selection["model"].as_str() == Some(&m.slug))
            })
            .and_then(|m| {
                m.option_descriptors
                    .iter()
                    .find(|d| d["role"] == "reasoning" || d["id"] == "reasoningEffort")
            });
        let Some(descriptor) = descriptor else {
            return;
        };
        let prompt = self.conversation.composer.read(cx).value().to_string();
        let Some((value, next_prompt)) = reasoning_step(descriptor, &selection, &prompt, forward)
        else {
            return;
        };
        let id = descriptor["id"].as_str().unwrap_or_default().to_string();
        self.set_model_option(&id, value, cx);
        if next_prompt != prompt {
            let cursor = self
                .conversation
                .composer
                .read(cx)
                .cursor()
                .saturating_sub(prompt.len() - next_prompt.len());
            let (line, column) = completion::line_column(&next_prompt, cursor);
            self.conversation.composer.update(cx, |state, cx| {
                state.set_value(next_prompt, window, cx);
                state.set_cursor_position(Position::new(line, column), window, cx);
            });
            self.stash_composer(cx);
        }
    }
}

fn reasoning_step(
    descriptor: &Value,
    selection: &Value,
    prompt: &str,
    forward: bool,
) -> Option<(Value, String)> {
    let options = descriptor["options"].as_array()?;
    let injected = descriptor["promptInjectedValues"].as_array();
    let pickable = |option: &&Value| !injected.is_some_and(|values| values.contains(&option["id"]));
    if options.iter().filter(pickable).count() < 2 {
        return None;
    }
    let mut current = models::option_value(selection, descriptor);
    let mut prompt = prompt.to_string();
    if injected.is_some_and(|values| values.iter().any(|v| v == "ultrathink")) {
        let word = regex::Regex::new(r"(?i)\bultrathink\b").expect("ultrathink");
        let prefix = regex::Regex::new(r"(?i)^Ultrathink:\s*").expect("prefix");
        if word.is_match(&prompt) {
            let remainder = prefix.replace(&prompt, "");
            if word.is_match(&remainder) {
                return None;
            }
            current = Some(json!("ultrathink"));
            prompt = remainder.into_owned();
        }
    }
    let mut ordered = options.iter().collect::<Vec<_>>();
    if !forward {
        ordered.reverse();
    }
    let at = ordered
        .iter()
        .position(|o| Some(&o["id"]) == current.as_ref());
    let next = ordered
        .iter()
        .skip(at.map_or(0, |i| i + 1))
        .chain(ordered.iter().take(at.unwrap_or(0)))
        .find(|option| pickable(option))?;
    Some((next["id"].clone(), prompt))
}

#[cfg(test)]
mod tests {
    use super::reasoning_step;
    use serde_json::json;
    #[test]
    fn reasoning_cycles_catalog_and_skips_prompt_injection() {
        let descriptor = json!({"id":"reasoningEffort","defaultValue":"medium","promptInjectedValues":["ultrathink"],"options":[{"id":"low"},{"id":"medium"},{"id":"high"},{"id":"ultrathink"}]});
        for (current, prompt, forward, expected, text) in [
            ("low", "hello", true, "medium", "hello"),
            ("high", "hello", true, "low", "hello"),
            ("low", "hello", false, "high", "hello"),
            ("high", "Ultrathink: hello", false, "high", "hello"),
            ("high", "Ultrathink: hello", true, "low", "hello"),
        ] {
            let selection = json!({"options":[{"id":"reasoningEffort","value":current}]});
            assert_eq!(
                reasoning_step(&descriptor, &selection, prompt, forward),
                Some((json!(expected), text.into()))
            );
        }
        assert_eq!(
            reasoning_step(&descriptor, &json!({}), "please ultrathink here", true),
            None
        );
    }
}
