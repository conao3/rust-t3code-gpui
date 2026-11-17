use super::*;
use gpui_kit::component::{
    IndexPath,
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

pub(super) struct Controls {
    key: Option<(String, String, Value)>,
    host: Option<Entity<State>>,
    workspace: Option<Entity<State>>,
    subscriptions: Vec<Subscription>,
    pub(super) effort_focus: FocusHandle,
    pub(super) effort_cursor: Option<Value>,
}
impl Controls {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            key: None,
            host: None,
            workspace: None,
            subscriptions: vec![],
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
            .zip(self.draft.as_ref())
            .map(|((env, project), draft)| (env, project, draft.workspace_strategy.clone()));
        if self.composer_choices.key == key {
            return;
        }
        self.composer_choices.key = key.clone();
        self.composer_choices.subscriptions.clear();
        self.composer_choices.host = None;
        self.composer_choices.workspace = None;
        let Some((environment, _, strategy)) = key else {
            return;
        };
        let groups = self.project_groups();
        let choices = groups
            .iter()
            .find(|g| self.draft.as_ref().and_then(|d| d.group.as_ref()) == Some(&g.key))
            .map(|g| {
                g.members
                    .iter()
                    .map(|m| (m.environment.clone(), m.environment.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let host = select(choices, &environment, window, cx);
        let mut workspaces = vec![
            (
                "root".into(),
                if strategy["type"] == "existing_worktree" {
                    "Current worktree"
                } else {
                    "Current checkout"
                }
                .into(),
            ),
            ("worktree".into(), "New worktree".into()),
        ];
        if let Some(previous) = self.previous_worktree() {
            let label = previous["branch"]
                .as_str()
                .map(|branch| format!("Previous worktree ({branch})"))
                .unwrap_or_else(|| "Previous worktree".into());
            workspaces.push(("previous".into(), label));
        }
        let workspace = select(
            workspaces,
            if strategy["type"] == "worktree" {
                "worktree"
            } else {
                "root"
            },
            window,
            cx,
        );
        for (state, kind) in [(&host, "host"), (&workspace, "mode")] {
            self.composer_choices.subscriptions.push(cx.subscribe_in(
                state,
                window,
                move |root, _, event: &SelectEvent<_>, window, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        if kind == "mode"
                            && value == "root"
                            && root.draft.as_ref().is_some_and(|draft| {
                                draft.workspace_strategy["type"] == "existing_worktree"
                            })
                        {
                            return;
                        }
                        if value == "previous" {
                            root.use_previous_worktree(window, cx);
                        } else {
                            root.workspace_action(
                                &WorkspaceAction {
                                    kind: kind.into(),
                                    value: json!(value),
                                },
                                window,
                                cx,
                            );
                        }
                    }
                },
            ));
        }
        self.composer_choices.host = Some(host);
        self.composer_choices.workspace = Some(workspace);
    }

    pub(super) fn render_composer_context(&self, cx: &Context<Self>) -> AnyElement {
        h_flex()
            .gap_1()
            .children(
                self.composer_choices
                    .host
                    .as_ref()
                    .filter(|_| self.connections.len() > 1)
                    .map(|state| {
                        Select::new(state)
                            .accessibility_label("Run on")
                            .appearance(false)
                            .w(px(110.))
                            .small()
                    }),
            )
            .when(self.workspace_status.is_repo(), |row| {
                row.child(self.render_workspace_mode(cx))
            })
            .into_any_element()
    }

    pub(super) fn render_workspace_select(&self) -> AnyElement {
        self.composer_choices
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
            .unwrap_or_else(|| div().into_any_element())
    }

    pub(super) fn open_composer_select(
        &mut self,
        host: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_composer_controls(window, cx);
        let state = if host {
            &self.composer_choices.host
        } else {
            &self.composer_choices.workspace
        };
        if let Some(state) = state {
            state.update(cx, |state, cx| state.open(window, cx));
        }
    }

    pub(super) fn cycle_composer_host(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((environment, _)) = self.draft_project() else {
            return;
        };
        let groups = self.project_groups();
        let Some(group) = groups
            .iter()
            .find(|g| self.draft.as_ref().and_then(|d| d.group.as_ref()) == Some(&g.key))
        else {
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
        let current = self.draft.as_ref()?.workspace_strategy["worktreePath"].as_str();
        shell.threads.iter().filter(|t| t["projectId"].as_str() == Some(&project)
            && t["worktreePath"].as_str().is_some_and(|path| !path.is_empty() && Some(path) != current)
            && t["archivedAt"].is_null())
            .filter_map(|t| chrono::DateTime::parse_from_rfc3339(t["updatedAt"].as_str()?).ok().map(|date| (date, t)))
            .max_by_key(|(date, _)| *date)
            .map(|(_, t)| json!({"type":"existing_worktree", "worktreePath":t["worktreePath"], "branch":t["branch"]}))
    }

    pub(super) fn use_previous_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.workspace_status.is_repo() {
            return;
        }
        if let Some(previous) = self.previous_worktree()
            && let Some(draft) = self.draft.as_mut()
        {
            draft.workspace_strategy = previous;
            self.composer
                .update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
        }
    }

    pub(super) fn open_effort(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.picker_open {
            self.toggle_model_picker(window, cx);
        }
        self.composer_choices.effort_cursor = None;
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
            self.composer_choices.effort_focus.focus(window, cx);
        } else {
            self.model_picker
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
        let prompt = self.composer.read(cx).value().to_string();
        let Some((value, next_prompt)) = reasoning_step(descriptor, &selection, &prompt, forward)
        else {
            return;
        };
        let id = descriptor["id"].as_str().unwrap_or_default().to_string();
        self.set_model_option(&id, value, cx);
        if next_prompt != prompt {
            let cursor = self
                .composer
                .read(cx)
                .cursor()
                .saturating_sub(prompt.len() - next_prompt.len());
            let (line, column) = completion::line_column(&next_prompt, cursor);
            self.composer.update(cx, |state, cx| {
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
