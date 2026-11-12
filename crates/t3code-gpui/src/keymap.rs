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

/// Evaluates a `when` clause against the conditions this app has: no terminal, preview or right
/// panel is focused or open yet, so `!terminalFocus` holds and every other condition does not.
pub fn holds(when: Option<&Value>) -> bool {
    let Some(node) = when.filter(|w| !w.is_null()) else {
        return true;
    };
    match node.get("type").and_then(Value::as_str) {
        Some("identifier") => false,
        Some("not") => !holds(node.get("node")),
        Some("and") => holds(node.get("left")) && holds(node.get("right")),
        Some("or") => holds(node.get("left")) || holds(node.get("right")),
        _ => false,
    }
}

/// The rules of a server config that apply here, later rules first, as T3 lets a later rule
/// override an earlier one for the same chord.
pub fn rules(config: &Value) -> Vec<Rule> {
    let mut rules: Vec<Rule> = config
        .get("keybindings")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|r| holds(r.get("whenAst")))
        .filter_map(|r| {
            Some(Rule {
                chord: chord(r.get("shortcut")?)?,
                command: r.get("command")?.as_str()?.to_string(),
            })
        })
        .collect();
    rules.reverse();
    rules
}

/// The command a typed chord runs: the first rule for the chord whose command this app knows.
pub fn command_for<'a>(
    rules: &'a [Rule],
    typed: &Chord,
    known: impl Fn(&str) -> bool,
) -> Option<&'a str> {
    rules
        .iter()
        .find(|r| r.chord == *typed)
        .map(|r| r.command.as_str())
        .filter(|c| known(c))
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
        assert_eq!(rules.len(), 3);
        let alt_o = chord(&shortcut("o", "A")).unwrap();
        assert_eq!(command_for(&rules, &alt_o, |_| true), Some("thread.settle"));
        let ctrl_n = chord(&shortcut("n", "m")).unwrap();
        assert_eq!(command_for(&rules, &ctrl_n, |_| true), Some("chat.new"));
        assert_eq!(command_for(&rules, &ctrl_n, |c| c != "chat.new"), None);
    }
}
