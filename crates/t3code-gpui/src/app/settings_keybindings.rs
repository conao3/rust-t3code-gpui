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
    recording: bool,
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
            recording: false,
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

fn command_label(command: &str) -> String {
    match command {
        "workspace.focusNext" => "Focus Next Workspace Area".into(),
        "workspace.focusPrevious" => "Focus Previous Workspace Area".into(),
        "composer.sendAlternate" => "Composer: Opposite Queue or Steer Action".into(),
        "composer.sendBackground" => "Composer: Start in Background".into(),
        "composer.sendAndNewThread" => "Composer: Send and Start New Thread".into(),
        "composer.dictation" => "Composer: Start or Finish Dictation".into(),
        "thread.steerQueuedMessage" => "Queue: Send First Queued Message".into(),
        "thread.editQueuedMessage" => "Queue: Edit Last Queued Message".into(),
        "thread.copyReference" => "Pull Request: Copy Link or Thread".into(),
        "view.reopenClosed" => "Reopen Closed Tab".into(),
        "preview.focusUrl" => "Preview: Focus URL".into(),
        "usage.period.day" => "Usage: Period: Past 24h".into(),
        "usage.period.week" => "Usage: Period: 7 days".into(),
        "usage.period.month" => "Usage: Period: 30 days".into(),
        "usage.period.quarter" => "Usage: Period: 90 days".into(),
        _ => {
            let title = |part: &str| {
                let mut result = String::new();
                let mut previous = ' ';
                for character in part.chars() {
                    if character.is_uppercase()
                        && (previous.is_lowercase() || previous.is_ascii_digit())
                    {
                        result.push(' ');
                    }
                    if matches!(character, '-' | '_' | ' ') {
                        result.push(' ');
                    } else if result.is_empty() || result.ends_with(' ') {
                        result.extend(character.to_uppercase());
                    } else {
                        result.push(character);
                    }
                    previous = character;
                }
                result.split_whitespace().collect::<Vec<_>>().join(" ")
            };
            if let Some(script) = command
                .strip_prefix("script.")
                .and_then(|value| value.strip_suffix(".run"))
            {
                format!("Run Script: {}", title(script))
            } else {
                command.split('.').map(title).collect::<Vec<_>>().join(": ")
            }
        }
    }
}

fn group_index(command: &str) -> usize {
    match command.split('.').next().unwrap_or_default() {
        "sidebar" | "rightPanel" | "commandPalette" | "filePicker" | "projectSearch" | "editor" => {
            0
        }
        "thread" | "chat" | "pullRequest" => 1,
        "composer" | "modelPicker" => 2,
        "terminal" => 3,
        "preview" | "diff" => 4,
        "theme" | "appearance" | "themeEditor" => 5,
        "script" => 6,
        _ => 7,
    }
}

fn command_order(command: &str) -> (String, usize) {
    let usage = [
        "usage.open",
        "usage.cost",
        "usage.tokens",
        "usage.limits",
        "usage.period.day",
        "usage.period.week",
        "usage.period.month",
        "usage.period.quarter",
    ];
    match usage.iter().position(|value| *value == command) {
        Some(index) => ("usage.cost".into(), index),
        None => (command.into(), 0),
    }
}

fn shortcut_label(key: &str) -> String {
    let (modifiers, key) = key.rsplit_once('+').map_or(("", key), |(modifiers, key)| {
        if key.is_empty() {
            (modifiers.strip_suffix('+').unwrap_or(modifiers), "+")
        } else {
            (modifiers, key)
        }
    });
    let mut parts = modifiers
        .split('+')
        .filter(|part| !part.is_empty())
        .map(|part| match part {
            "mod" | "ctrl" => "Ctrl".to_string(),
            "shift" => "Shift".to_string(),
            "alt" => "Alt".to_string(),
            "meta" => "Meta".to_string(),
            _ => part.to_string(),
        })
        .collect::<Vec<_>>();
    parts.push(match key {
        "arrowup" => "Up".into(),
        "arrowdown" => "Down".into(),
        "arrowleft" => "Left".into(),
        "arrowright" => "Right".into(),
        "enter" => "Enter".into(),
        "space" => "Space".into(),
        "esc" => "Esc".into(),
        _ if key.len() == 1 || key.starts_with('f') => key.to_uppercase(),
        _ => command_label(key),
    });
    parts.join("+")
}

fn key_pill(key: &str, background: Hsla) -> impl IntoElement {
    h_flex()
        .gap_1()
        .children(key.split('+').enumerate().map(|(index, part)| {
            let label = match part {
                "mod" => "Ctrl".into(),
                "shift" => "⇧".into(),
                "alt" => "Alt".into(),
                "ctrl" => "⌃".into(),
                _ if part.len() == 1 => part.to_uppercase(),
                _ => part.into(),
            };
            settings_controls::text(label)
                .id(("shortcut-part", index))
                .bg(background)
                .text_xs()
                .px_1()
                .rounded_sm()
        }))
}

impl Render for Keybindings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use settings_controls::text;
        let _ = &self.subscriptions;
        let config = self
            .root
            .upgrade()
            .and_then(|root| root.read(cx).config_of(&self.environment))
            .unwrap_or(Value::Null);
        let query = self.search.read(cx).value().trim().to_lowercase();
        let mut rows = config["keybindings"]
            .as_array()
            .into_iter()
            .flatten()
            .map(binding_row)
            .filter(|row| {
                let label = command_label(row["command"].as_str().unwrap_or_default());
                query.is_empty() || format!("{label} {row}").to_lowercase().contains(&query)
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| {
            (
                command_order(row["command"].as_str().unwrap_or_default()),
                row["key"].as_str().unwrap_or_default().to_string(),
            )
        });
        rows.dedup();
        let config_path = config["keybindingsConfigPath"].as_str().map(str::to_string);
        v_flex().gap_8()
            .when_some(self.error.clone(), |view, error| view.child(div().id("settings-error").role(accesskit::Role::Alert).text_color(cx.theme().danger).child(error)))
            .child(v_flex().gap_3().child(text("Keybindings").px_4().font_weight(FontWeight::SEMIBOLD)).child(h_flex().gap_2().px_4().py_3().rounded(px(12.)).border_1().border_color(cx.theme().border).bg(cx.theme().muted.opacity(0.25))
                .child(Input::new(&self.search).role(accesskit::Role::SearchInput).aria_label("Search keybindings").flex_1())
                .child(Button::new("add-keybinding").label("Add keybinding").outline().disabled(self.adding).on_click(cx.listener(|view, _, window, cx| view.edit(None, window, cx))))
                .child(Button::new("open-keybindings").icon(gpui_kit::assets::IconName::File).accessibility_label("Open keybindings.json").outline().disabled(config_path.is_none()).on_click(cx.listener(move |view, _, _, cx| {
                    if let Some(path) = &config_path {
                        let editor = view.root.upgrade().map(|root| root.read(cx).settings.values.get("gpuiPreferredEditor").cloned().unwrap_or(json!("code")));
                        if let Some(editor) = editor { view.request("shell.openInEditor", json!({"cwd":path,"editor":editor}), cx); }
                    }
                })))))
            .when(self.adding, |view| view.child(v_flex().id("keybinding-editor").role(accesskit::Role::Group).aria_label("Edit keybinding").gap_2().p_3().border_1().border_color(cx.theme().border)
                .child(Input::new(&self.command).aria_label("Command"))
                .child(div().child(Input::new(&self.shortcut).aria_label(format!("Keybinding for {}", command_label(self.command.read(cx).value().as_ref()))))
                    .capture_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" {
                            view.adding = false;
                            view.recording = false;
                        } else {
                            let key = &event.keystroke;
                            let mut parts = Vec::new();
                            if key.modifiers.control { parts.push("mod"); }
                            if key.modifiers.platform { parts.push("meta"); }
                            if key.modifiers.alt { parts.push("alt"); }
                            if key.modifiers.shift { parts.push("shift"); }
                            parts.push(match key.key.as_str() { "up" => "arrowup", "down" => "arrowdown", "left" => "arrowleft", "right" => "arrowright", "space" => "space", value => value });
                            view.shortcut.update(cx, |input, cx| input.set_value(parts.join("+"), window, cx));
                            view.recording = false;
                        }
                        cx.stop_propagation();
                        window.prevent_default();
                        cx.notify();
                    })))
                .child(Input::new(&self.condition).aria_label("When expression"))
                .child(h_flex().gap_2()
                    .child(Button::new("save-binding").label("Save").disabled(self.saving).primary().on_click(cx.listener(|view, _, _, cx| view.save(cx))))
                    .child(Button::new("cancel-binding").label("Cancel").ghost().on_click(cx.listener(|view, _, _, cx| { view.adding = false; cx.notify(); }))))))
            .when(rows.is_empty(), |view| view.child(text("No keybindings match your search.").p_4()))
            .children(["Navigation", "Threads", "Composer", "Terminal", "Preview & diff", "Appearance", "Project scripts", "Other"].into_iter().enumerate().filter_map(|(group, title)| {
                let group_rows = rows.iter().enumerate().filter(|(_, row)| group_index(row["command"].as_str().unwrap_or_default()) == group).collect::<Vec<_>>();
                if group_rows.is_empty() { return None; }
                Some(v_flex().gap_3().child(text(title).px_4().font_weight(FontWeight::SEMIBOLD))
                    .child(v_flex().rounded(px(12.)).overflow_hidden().border_1().border_color(cx.theme().border).bg(cx.theme().muted.opacity(0.25)).children(group_rows.into_iter().map(|(index, row)| {
                        let label = command_label(row["command"].as_str().unwrap_or_default());
                        let key = row["key"].as_str().unwrap_or_default();
                        let when = row["when"].as_str().unwrap_or_default();
                        let edit_key = row.clone();
                        let edit_when = row.clone();
                        let reset = default_binding(row).filter(|default| default["key"] != row["key"] || default["when"].as_str().unwrap_or_default() != when);
                        let custom = reset.is_some() || default_binding(row).is_none();
                        let remove = row.clone();
                        h_flex().id(("keybinding", index)).gap_4().px_4().py_3().border_b_1().border_color(cx.theme().border)
                            .child(v_flex().flex_1().min_w_0().child(text(label.clone()).text_sm()).child(h_flex().gap_1().child(text("When").text_xs().text_color(cx.theme().muted_foreground))
                                .child(Button::new(("when", index)).child(text(if when.is_empty() { "Always".to_string() } else { when.to_string() }).text_xs().font_family(cx.theme().mono_font_family.clone())).accessibility_label(format!("Edit when clause for {label}")).ghost().compact().h(px(24.)).on_click(cx.listener(move |view, _, window, cx| { view.edit(Some(edit_when.clone()), window, cx); view.condition.update(cx, |input, cx| input.focus(window, cx)); })))))
                            .when(custom, |view| view.child(text("Custom").text_xs()).child(Button::new(("remove", index)).label("Remove").ghost().compact().on_click(cx.listener(move |view, _, _, cx| view.request("server.removeKeybinding", target(&remove), cx)))))
                            .when_some(reset, |view, reset| view.child(Button::new(("reset", index)).label("Reset to default").ghost().compact().on_click(cx.listener(move |view, _, _, cx| view.request("server.upsertKeybinding", reset.clone(), cx)))))
                            .child(Button::new(("edit-key", index)).accessibility_label(format!("Edit shortcut for {label}: {}", shortcut_label(key))).ghost().child(key_pill(key, cx.theme().muted)).on_click(cx.listener(move |view, _, window, cx| { view.edit(Some(edit_key.clone()), window, cx); view.recording = true; })))
                    }))))
            }))
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
    ("thread.nextDone", "alt+o", "!terminalFocus"),
    ("chat.previousUserMessage", "alt+p", "!terminalFocus"),
    ("chat.nextUserMessage", "alt+n", "!terminalFocus"),
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
    use super::{
        binding_row, command_label, default_binding, group_index, shortcut_label, when_expression,
    };
    use serde_json::json;
    #[test]
    fn settings_labels_follow_the_reference_commands_and_chords() {
        for (command, label, group) in [
            ("workspace.focusNext", "Focus Next Workspace Area", 7),
            (
                "composer.sendBackground",
                "Composer: Start in Background",
                2,
            ),
            ("modelPicker.jump.3", "Model Picker: Jump: 3", 2),
            ("usage.period.day", "Usage: Period: Past 24h", 7),
            ("script.run_tests.run", "Run Script: Run Tests", 6),
            ("pullRequest.copyNumber", "Pull Request: Copy Number", 1),
        ] {
            assert_eq!(command_label(command), label);
            assert_eq!(group_index(command), group);
        }
        for (key, label) in [
            ("mod+shift+o", "Ctrl+Shift+O"),
            ("mod++", "Ctrl++"),
            ("mod+shift+arrowup", "Ctrl+Shift+Up"),
            ("f6", "F6"),
        ] {
            assert_eq!(shortcut_label(key), label);
        }
    }

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
