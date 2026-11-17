use super::*;

pub(super) struct Keybindings {
    error: Option<String>,
    saving: bool,
    root: WeakEntity<Root>,
    environment: String,
    search: Entity<InputState>,
    command: Entity<InputState>,
    shortcut: Entity<InputState>,
    condition: Entity<InputState>,
    editing: Option<Value>,
    adding: bool,
    subscriptions: Vec<Subscription>,
}

impl Keybindings {
    pub(super) fn new(
        root: WeakEntity<Root>,
        environment: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search keybindings"));
        let command = cx.new(|cx| InputState::new(window, cx).placeholder("Command"));
        let shortcut = cx.new(|cx| InputState::new(window, cx).placeholder("Shortcut"));
        let condition = cx.new(|cx| InputState::new(window, cx).placeholder("When"));
        let subscriptions = vec![cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify())];
        Self {
            error: None,
            saving: false,
            root,
            environment,
            search,
            command,
            shortcut,
            condition,
            editing: None,
            adding: false,
            subscriptions,
        }
    }
    fn request(&self, method: &'static str, payload: Value, cx: &mut Context<Self>) {
        let _ = self.root.update(cx, |root, cx| {
            root.settings_request(&self.environment, method, payload, cx)
        });
    }
    fn edit(&mut self, row: Option<Value>, window: &mut Window, cx: &mut Context<Self>) {
        for (input, key) in [
            (&self.command, "command"),
            (&self.shortcut, "key"),
            (&self.condition, "when"),
        ] {
            let value = row.as_ref().and_then(|row| row[key].as_str()).unwrap_or("");
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.editing = row;
        self.adding = true;
        self.shortcut
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let mut payload = json!({"command":self.command.read(cx).value().trim(),"key":self.shortcut.read(cx).value().trim()});
        let condition = self.condition.read(cx).value();
        if !condition.trim().is_empty() {
            payload["when"] = json!(condition.trim());
        }
        if let Some(row) = &self.editing {
            payload["replace"] = target(row);
        }
        self.saving = true;
        settings_controls::request(
            &self.root,
            &self.environment,
            "server.upsertKeybinding",
            payload,
            cx,
            |view, result, _| {
                view.saving = false;
                match result {
                    Ok(_) => {
                        view.adding = false;
                        view.error = None;
                    }
                    Err(error) => view.error = Some(format!("Unable to save: {error:#}")),
                }
            },
        );
        cx.notify();
    }
}

fn target(row: &Value) -> Value {
    let mut target = json!({"command":row["command"],"key":row["key"]});
    if let Some(condition) = row["when"]
        .as_str()
        .filter(|condition| !condition.is_empty())
    {
        target["when"] = json!(condition);
    }
    target
}

impl Render for Keybindings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = &self.subscriptions;
        let config = self
            .root
            .upgrade()
            .and_then(|root| root.read(cx).config_of(&self.environment))
            .unwrap_or(Value::Null);
        let query = self.search.read(cx).value().to_lowercase();
        let mut rows = config["keybindings"]
            .as_array()
            .into_iter()
            .flatten()
            .map(binding_row)
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| a["command"].as_str().cmp(&b["command"].as_str()));
        v_flex()
            .when_some(self.error.clone(), |view, error| {
                view.child(
                    div()
                        .id("settings-error")
                        .role(accesskit::Role::Alert)
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Input::new(&self.search)
                            .role(accesskit::Role::SearchInput)
                            .aria_label("Search keybindings")
                            .flex_1(),
                    )
                    .child(
                        Button::new("add-keybinding")
                            .label("Add keybinding")
                            .outline()
                            .on_click(
                                cx.listener(|view, _, window, cx| view.edit(None, window, cx)),
                            ),
                    ),
            )
            .when(self.adding, |view| {
                view.child(
                    v_flex()
                        .id("keybinding-editor")
                        .role(accesskit::Role::Group)
                        .aria_label("Edit keybinding")
                        .gap_2()
                        .p_3()
                        .border_1()
                        .border_color(cx.theme().border)
                        .child(Input::new(&self.command).aria_label("Command"))
                        .child(Input::new(&self.shortcut).aria_label("Shortcut"))
                        .child(Input::new(&self.condition).aria_label("When"))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("save-binding")
                                        .label("Save")
                                        .disabled(self.saving)
                                        .primary()
                                        .on_click(cx.listener(|view, _, _, cx| view.save(cx))),
                                )
                                .child(
                                    Button::new("cancel-binding")
                                        .label("Cancel")
                                        .ghost()
                                        .on_click(cx.listener(|view, _, _, cx| {
                                            view.adding = false;
                                            cx.notify();
                                        })),
                                ),
                        ),
                )
            })
            .children(
                rows.into_iter()
                    .enumerate()
                    .filter(|(_, row)| {
                        query.is_empty() || row.to_string().to_lowercase().contains(&query)
                    })
                    .map(|(index, row)| {
                        let edit = row.clone();
                        let reset = default_binding(&row).filter(|default| {
                            default["key"] != row["key"]
                                || default["when"].as_str().unwrap_or_default()
                                    != row["when"].as_str().unwrap_or_default()
                        });
                        h_flex()
                            .id(("keybinding", index))
                            .role(accesskit::Role::Group)
                            .aria_label(row["command"].as_str().unwrap_or_default().to_string())
                            .gap_2()
                            .py_2()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                v_flex()
                                    .flex_1()
                                    .child(row["command"].as_str().unwrap_or_default().to_string())
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(
                                                row["when"]
                                                    .as_str()
                                                    .unwrap_or_default()
                                                    .to_string(),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .child(row["key"].as_str().unwrap_or_default().to_string()),
                            )
                            .child(
                                Button::new(("edit", index))
                                    .label("Edit")
                                    .ghost()
                                    .compact()
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.edit(Some(edit.clone()), window, cx)
                                    })),
                            )
                            .when_some(reset, |element, reset| {
                                element.child(
                                    Button::new(("reset", index))
                                        .label("Reset")
                                        .ghost()
                                        .compact()
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.request(
                                                "server.upsertKeybinding",
                                                reset.clone(),
                                                cx,
                                            )
                                        })),
                                )
                            })
                            .child(
                                Button::new(("remove", index))
                                    .label("Remove")
                                    .ghost()
                                    .compact()
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.request("server.removeKeybinding", target(&row), cx)
                                    })),
                            )
                    }),
            )
    }
}

const DEFAULTS: &[(&str, &str, &str)] = &[
    ("sidebar.toggle", "mod+b", ""),
    ("workspace.focusNext", "f6", ""),
    ("workspace.focusPrevious", "shift+f6", ""),
    ("workspace.focusNext", "mod+f6", ""),
    ("workspace.focusPrevious", "mod+shift+f6", ""),
    ("navigation.back", "mod+[", "!terminalFocus"),
    ("navigation.forward", "mod+]", "!terminalFocus"),
    ("terminal.toggle", "mod+j", ""),
    ("rightPanel.toggle", "mod+alt+b", ""),
    ("threadPanel.toggle", "mod+shift+b", "!terminalFocus"),
    (
        "rightPanel.new",
        "mod+t",
        "rightPanelOpen && !terminalFocus",
    ),
    ("terminal.split", "mod+d", "terminalFocus"),
    ("terminal.splitVertical", "mod+shift+d", "terminalFocus"),
    ("composer.dictation", "mod+shift+d", "!terminalFocus"),
    ("terminal.new", "mod+n", "terminalFocus"),
    ("terminal.close", "mod+w", "terminalFocus"),
    ("rightPanel.close", "mod+w", "!terminalFocus"),
    ("view.reopenClosed", "mod+shift+t", ""),
    ("diff.toggle", "mod+d", "!terminalFocus"),
    ("preview.toggle", "mod+shift+j", ""),
    ("preview.refresh", "mod+r", "previewFocus"),
    ("preview.focusUrl", "mod+l", "previewFocus"),
    ("preview.zoomIn", "mod+=", "previewFocus"),
    ("preview.zoomIn", "mod++", "previewFocus"),
    ("preview.zoomOut", "mod+-", "previewFocus"),
    ("preview.resetZoom", "mod+0", "previewFocus"),
    ("commandPalette.toggle", "mod+k", "!terminalFocus"),
    ("filePicker.toggle", "mod+p", "!terminalFocus"),
    ("projectSearch.toggle", "mod+shift+f", "!terminalFocus"),
    ("usage.open", "mod+u", "!terminalFocus"),
    ("theme.select", "mod+alt+a", "!terminalFocus"),
    ("appearance.cycle", "mod+alt+shift+a", "!terminalFocus"),
    ("themeEditor.toggle", "mod+alt+shift+t", ""),
    ("composer.stash", "mod+s", "!terminalFocus"),
    (
        "thread.steerQueuedMessage",
        "mod+shift+enter",
        "!terminalFocus",
    ),
    ("thread.editQueuedMessage", "alt+arrowup", "composerFocus"),
    (
        "composer.sendAlternate",
        "mod+enter",
        "composerFocus && turnRunning",
    ),
    (
        "composer.sendBackground",
        "mod+enter",
        "composerFocus && draftThreadRoute",
    ),
    (
        "composer.sendBackground",
        "mod+alt+enter",
        "composerFocus && draftThreadRoute",
    ),
    (
        "composer.sendAndNewThread",
        "mod+alt+enter",
        "composerFocus && !draftThreadRoute",
    ),
    ("chat.new", "mod+n", "!terminalFocus"),
    ("chat.new", "mod+shift+o", "!terminalFocus"),
    ("chat.newLocal", "mod+shift+n", "!terminalFocus"),
    ("chat.newWithoutProject", "mod+alt+n", "!terminalFocus"),
    ("chat.find", "mod+f", "!terminalFocus && !previewFocus"),
    ("modelPicker.toggle", "mod+shift+m", "!terminalFocus"),
    ("composer.host", "mod+shift+h", "!terminalFocus"),
    ("composer.effort", "mod+shift+e", "!terminalFocus"),
    ("composer.mode", "mod+shift+a", "!terminalFocus"),
    ("composer.workspace", "mod+shift+x", "!terminalFocus"),
    ("composer.branch", "mod+shift+g", "!terminalFocus"),
    ("composer.previousWorktree", "mod+shift+l", "!terminalFocus"),
    ("pullRequest.copyNumber", "mod+shift+k", "!terminalFocus"),
    (
        "modelPicker.previousProvider",
        "mod+shift+arrowup",
        "modelPickerOpen",
    ),
    (
        "modelPicker.nextProvider",
        "mod+shift+arrowdown",
        "modelPickerOpen",
    ),
    ("editor.openFavorite", "mod+o", ""),
    ("thread.previous", "mod+shift+[", ""),
    ("thread.next", "mod+shift+]", ""),
    ("thread.copyReference", "mod+shift+c", "!terminalFocus"),
    ("thread.settle", "mod+shift+s", "!terminalFocus"),
    ("thread.pin", "mod+shift+p", "!terminalFocus"),
    ("thread.undo", "mod+z", "!terminalFocus && !editableFocus"),
    ("usage.cost", "c", "usagePageOpen"),
    ("usage.tokens", "t", "usagePageOpen"),
    ("usage.limits", "l", "usagePageOpen"),
    ("usage.period.day", "mod+shift+1", "usagePageOpen"),
    ("usage.period.week", "mod+shift+2", "usagePageOpen"),
    ("usage.period.month", "mod+shift+3", "usagePageOpen"),
    ("usage.period.quarter", "mod+shift+4", "usagePageOpen"),
    ("thread.jump.1", "mod+1", "!modelPickerOpen && isDesktop"),
    ("thread.jump.2", "mod+2", "!modelPickerOpen && isDesktop"),
    ("thread.jump.3", "mod+3", "!modelPickerOpen && isDesktop"),
    ("thread.jump.4", "mod+4", "!modelPickerOpen && isDesktop"),
    ("thread.jump.5", "mod+5", "!modelPickerOpen && isDesktop"),
    ("thread.jump.6", "mod+6", "!modelPickerOpen && isDesktop"),
    ("thread.jump.7", "mod+7", "!modelPickerOpen && isDesktop"),
    ("thread.jump.8", "mod+8", "!modelPickerOpen && isDesktop"),
    ("thread.jump.9", "mod+9", "!modelPickerOpen && isDesktop"),
    (
        "modelPicker.jump.1",
        "mod+1",
        "modelPickerOpen && isDesktop",
    ),
    (
        "modelPicker.jump.2",
        "mod+2",
        "modelPickerOpen && isDesktop",
    ),
    (
        "modelPicker.jump.3",
        "mod+3",
        "modelPickerOpen && isDesktop",
    ),
    (
        "modelPicker.jump.4",
        "mod+4",
        "modelPickerOpen && isDesktop",
    ),
    (
        "modelPicker.jump.5",
        "mod+5",
        "modelPickerOpen && isDesktop",
    ),
    (
        "modelPicker.jump.6",
        "mod+6",
        "modelPickerOpen && isDesktop",
    ),
    (
        "modelPicker.jump.7",
        "mod+7",
        "modelPickerOpen && isDesktop",
    ),
    (
        "modelPicker.jump.8",
        "mod+8",
        "modelPickerOpen && isDesktop",
    ),
    (
        "modelPicker.jump.9",
        "mod+9",
        "modelPickerOpen && isDesktop",
    ),
];

fn binding_row(binding: &Value) -> Value {
    let shortcut = &binding["shortcut"];
    let mut keys = Vec::new();
    for (flag, name) in [
        ("modKey", "mod"),
        ("metaKey", "meta"),
        ("ctrlKey", "ctrl"),
        ("altKey", "alt"),
        ("shiftKey", "shift"),
    ] {
        if shortcut[flag] == true {
            keys.push(name);
        }
    }
    keys.push(match shortcut["key"].as_str().unwrap_or_default() {
        " " => "space",
        "escape" => "esc",
        key => key,
    });
    json!({"command":binding["command"],"key":keys.join("+"),"when":when_expression(&binding["whenAst"])})
}

fn when_expression(node: &Value) -> String {
    let wrap = |child: &Value| {
        let value = when_expression(child);
        if matches!(child["type"].as_str(), Some("and" | "or")) {
            format!("({value})")
        } else {
            value
        }
    };
    match node["type"].as_str() {
        Some("identifier") => node["name"].as_str().unwrap_or_default().to_string(),
        Some("not") => format!("!{}", wrap(&node["node"])),
        Some(kind @ ("and" | "or")) => format!(
            "{} {} {}",
            wrap(&node["left"]),
            if kind == "and" { "&&" } else { "||" },
            wrap(&node["right"])
        ),
        _ => String::new(),
    }
}

fn default_binding(row: &Value) -> Option<Value> {
    let command = row["command"].as_str()?;
    let (_, key, when) = DEFAULTS
        .iter()
        .filter(|(id, _, _)| *id == command)
        .min_by_key(|(_, key, when)| {
            if row["key"] == *key && row["when"] == *when {
                0
            } else if row["when"] == *when {
                1
            } else {
                2
            }
        })?;
    let mut binding = json!({"command":command,"key":key,"replace":target(row)});
    if !when.is_empty() {
        binding["when"] = json!(when);
    }
    Some(binding)
}

#[cfg(test)]
mod tests {
    use super::{binding_row, default_binding, when_expression};
    use serde_json::json;
    #[test]
    fn resolved_shortcuts_and_conditions_keep_the_original_editable_form() {
        for (shortcut, expected) in [
            (json!({"key":" ","modKey":true}), "mod+space"),
            (
                json!({"key":"escape","ctrlKey":true,"shiftKey":true}),
                "ctrl+shift+esc",
            ),
            (json!({"key":"+","altKey":true}), "alt++"),
        ] {
            assert_eq!(
                binding_row(&json!({"command":"usage.open","shortcut":shortcut}))["key"],
                expected
            );
        }
        let expression = json!({"type":"not","node":{"type":"or","left":{"type":"identifier","name":"terminalFocus"},"right":{"type":"identifier","name":"editableFocus"}}});
        assert_eq!(
            when_expression(&expression),
            "!(terminalFocus || editableFocus)"
        );
        let row = json!({"command":"usage.cost","key":"x","when":"usagePageOpen"});
        assert_eq!(default_binding(&row).unwrap()["key"], "c");
        assert!(
            default_binding(&json!({"command":"sidebar.toggle","key":"x","when":""}))
                .unwrap()
                .get("when")
                .is_none()
        );
    }
}
