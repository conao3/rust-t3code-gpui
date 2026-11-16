use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    pub platform: bool,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub chord: Chord,
    pub command: String,
    pub when: Option<Value>,
}

fn flag(shortcut: &Value, key: &str) -> bool {
    shortcut.get(key).and_then(Value::as_bool) == Some(true)
}

/// The GPUI key name for a key as T3's keybindings write it (`KeyboardEvent.key`, lower case).
fn key_name(key: &str) -> String {
    match key {
        "arrowup" => "up".into(),
        "arrowdown" => "down".into(),
        "arrowleft" => "left".into(),
        "arrowright" => "right".into(),
        " " => "space".into(),
        other => other.to_lowercase(),
    }
}

/// The chord of a resolved T3 shortcut. `mod` is Ctrl off macOS, as T3's `modKey` is.
pub fn chord(shortcut: &Value) -> Option<Chord> {
    let key = shortcut.get("key").and_then(Value::as_str)?;
    let control =
        flag(shortcut, "ctrlKey") || (!cfg!(target_os = "macos") && flag(shortcut, "modKey"));
    let platform =
        flag(shortcut, "metaKey") || (cfg!(target_os = "macos") && flag(shortcut, "modKey"));
    Some(Chord {
        control,
        alt: flag(shortcut, "altKey"),
        shift: flag(shortcut, "shiftKey"),
        platform,
        key: key_name(key),
    })
}

/// Evaluates a `when` clause with no focused panes for the parser tests.
#[cfg(test)]
pub fn holds(when: Option<&Value>) -> bool {
    holds_in(when, &|_| false)
}

pub fn holds_in(when: Option<&Value>, context: &impl Fn(&str) -> bool) -> bool {
    let Some(node) = when.filter(|w| !w.is_null()) else {
        return true;
    };
    match node.get("type").and_then(Value::as_str) {
        Some("identifier") => context(node.get("name").and_then(Value::as_str).unwrap_or_default()),
        Some("not") => !holds_in(node.get("node"), context),
        Some("and") => holds_in(node.get("left"), context) && holds_in(node.get("right"), context),
        Some("or") => holds_in(node.get("left"), context) || holds_in(node.get("right"), context),
        _ => false,
    }
}

/// Retains conditional server rules, later rules first, for evaluation at dispatch time.
pub fn rules(config: &Value) -> Vec<Rule> {
    let mut rules: Vec<Rule> = config
        .get("keybindings")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|r| {
            Some(Rule {
                chord: chord(r.get("shortcut")?)?,
                command: r.get("command")?.as_str()?.to_string(),
                when: r.get("whenAst").cloned(),
            })
        })
        .collect();
    rules.reverse();
    rules
}

/// The command a typed chord runs: the first rule for the chord whose command this app knows.
#[cfg(test)]
pub fn command_for<'a>(
    rules: &'a [Rule],
    typed: &Chord,
    known: impl Fn(&str) -> bool,
) -> Option<&'a str> {
    resolve(rules, typed, &|_| false).filter(|c| known(c))
}

pub fn resolve<'a>(
    rules: &'a [Rule],
    typed: &Chord,
    context: &impl Fn(&str) -> bool,
) -> Option<&'a str> {
    let shifted = if typed.shift {
        "!@#$%^&*()~_+{}|:\"<>?"
            .chars()
            .zip("1234567890`-=[]\\;',./".chars())
            .find_map(|(from, to)| (typed.key == from.to_string()).then(|| to.to_string()))
    } else {
        None
    };
    let mut normalized = typed.clone();
    if let Some(key) = shifted {
        normalized.key = key;
    }
    rules
        .iter()
        .find(|r| {
            (r.chord == *typed || r.chord == normalized) && holds_in(r.when.as_ref(), context)
        })
        .map(|r| r.command.as_str())
}

pub fn jump_index(command: &str) -> Option<usize> {
    let digit = command
        .strip_prefix("thread.jump.")?
        .parse::<usize>()
        .ok()?;
    (1..=9).contains(&digit).then(|| digit - 1)
}

pub fn command_label(
    rules: &[Rule],
    command: &str,
    context: &impl Fn(&str) -> bool,
) -> Option<String> {
    let rule = rules.iter().find(|rule| {
        rule.command == command
            && holds_in(rule.when.as_ref(), context)
            && resolve(rules, &rule.chord, context) == Some(command)
    })?;
    let chord = &rule.chord;
    let mut parts = Vec::new();
    if chord.control {
        parts.push("Ctrl");
    }
    if chord.alt {
        parts.push("Alt");
    }
    if chord.shift {
        parts.push("Shift");
    }
    if chord.platform {
        parts.push("Super");
    }
    let key = chord.key.to_uppercase();
    parts.push(&key);
    Some(parts.join("+"))
}

pub fn jump_labels(rules: &[Rule], context: &impl Fn(&str) -> bool) -> Vec<Option<String>> {
    (1..=9)
        .map(|index| command_label(rules, &format!("thread.jump.{index}"), context))
        .collect()
}

pub fn jump_hints_held(rules: &[Rule], modifiers: &Chord, context: &impl Fn(&str) -> bool) -> bool {
    !context("terminalFocus")
        && rules.iter().any(|rule| {
            jump_index(&rule.command).is_some()
                && rule.chord.control == modifiers.control
                && rule.chord.alt == modifiers.alt
                && rule.chord.shift == modifiers.shift
                && rule.chord.platform == modifiers.platform
                && resolve(rules, &rule.chord, context) == Some(rule.command.as_str())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn shortcut(key: &str, m: &str) -> Value {
        json!({"key": key, "metaKey": m.contains('M'), "ctrlKey": m.contains('C'), "shiftKey": m.contains('S'),
               "altKey": m.contains('A'), "modKey": m.contains('m')})
    }

    #[test]
    fn shifted_punctuation_matches_physical_shortcuts_and_later_overrides() {
        let rules = rules(&json!({"keybindings": [
            {"command": "thread.next", "shortcut": shortcut("]", "CS")},
            {"command": "thread.previous", "shortcut": shortcut("[", "CS")},
            {"command": "custom", "shortcut": shortcut("+", "CS")}
        ]}));
        for (key, command) in [
            ("}", "thread.next"),
            ("{", "thread.previous"),
            ("+", "custom"),
        ] {
            assert_eq!(
                resolve(&rules, &chord(&shortcut(key, "CS")).unwrap(), &|_| false),
                Some(command)
            );
        }
    }

    #[test]
    fn desktop_jump_rules_and_overrides_follow_the_current_context() {
        let id = |name| json!({"type": "identifier", "name": name});
        let rules = rules(&json!({"keybindings": [
            {"command": "thread.jump.1", "shortcut": shortcut("1", "m"), "whenAst": id("isDesktop")},
            {"command": "modelPicker.jump.1", "shortcut": shortcut("1", "m"), "whenAst": id("modelPickerOpen")},
            {"command": "thread.jump.2", "shortcut": shortcut("8", "CA")},
            {"command": "chat.new", "shortcut": shortcut("8", "CA"), "whenAst": id("composerFocus")}
        ]}));
        let desktop = |name: &str| name == "isDesktop";
        let picker = |name: &str| matches!(name, "isDesktop" | "modelPickerOpen");
        let typed = chord(&shortcut("1", "m")).unwrap();
        assert_eq!(resolve(&rules, &typed, &desktop), Some("thread.jump.1"));
        assert_eq!(resolve(&rules, &typed, &picker), Some("modelPicker.jump.1"));
        assert_eq!(jump_labels(&rules, &desktop)[0].as_deref(), Some("Ctrl+1"));
        assert_eq!(
            jump_labels(&rules, &desktop)[1].as_deref(),
            Some("Ctrl+Alt+8")
        );
        assert!(jump_labels(&rules, &picker)[0].is_none());
        assert!(
            jump_labels(&rules, &|name| matches!(
                name,
                "isDesktop" | "composerFocus"
            ))[1]
                .is_none()
        );
        assert!(jump_hints_held(&rules, &typed, &desktop));
        assert!(!jump_hints_held(&rules, &typed, &picker));
        assert!(!jump_hints_held(&rules, &typed, &|name| matches!(
            name,
            "isDesktop" | "terminalFocus"
        )));
        assert_eq!(jump_index("thread.jump.9"), Some(8));
        assert_eq!(jump_index("thread.jump.0"), None);
        assert_eq!(jump_index("thread.jump.10"), None);
    }

    #[test]
    fn shortcuts_become_gpui_chords() {
        let c = chord(&shortcut("arrowup", "mS")).unwrap();
        assert_eq!(
            (c.control, c.shift, c.alt, c.platform, c.key.as_str()),
            (true, true, false, false, "up")
        );
        assert_eq!(chord(&shortcut("O", "A")).unwrap().key, "o");
    }

    #[test]
    fn when_clauses_hold_only_without_terminal_and_panel_focus() {
        let id = |n: &str| json!({"type": "identifier", "name": n});
        assert!(holds(None));
        assert!(holds(Some(
            &json!({"type": "not", "node": id("terminalFocus")})
        )));
        assert!(!holds(Some(&id("terminalFocus"))));
        assert!(!holds(Some(
            &json!({"type": "and", "left": id("rightPanelOpen"), "right": {"type": "not", "node": id("terminalFocus")}})
        )));
        assert!(holds(Some(
            &json!({"type": "or", "left": id("x"), "right": {"type": "not", "node": id("y")}})
        )));
    }

    #[test]
    fn a_later_rule_wins_and_unknown_commands_fall_through_to_none() {
        let config = json!({"keybindings": [
            {"command": "thread.nextDone", "shortcut": shortcut("o", "A")},
            {"command": "chat.new", "shortcut": shortcut("n", "m"), "whenAst": {"type": "not", "node": {"type": "identifier", "name": "terminalFocus"}}},
            {"command": "terminal.new", "shortcut": shortcut("n", "m"), "whenAst": {"type": "identifier", "name": "terminalFocus"}},
            {"command": "thread.settle", "shortcut": shortcut("o", "A")},
        ]});
        let rules = rules(&config);
        assert_eq!(rules.len(), 4);
        let alt_o = chord(&shortcut("o", "A")).unwrap();
        assert_eq!(command_for(&rules, &alt_o, |_| true), Some("thread.settle"));
        let ctrl_n = chord(&shortcut("n", "m")).unwrap();
        assert_eq!(command_for(&rules, &ctrl_n, |_| true), Some("chat.new"));
        assert_eq!(command_for(&rules, &ctrl_n, |c| c != "chat.new"), None);
    }
}
