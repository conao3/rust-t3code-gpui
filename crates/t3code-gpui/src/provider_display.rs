use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub instance_id: String,
    pub driver: String,
    pub name: String,
    pub enabled: bool,
    pub accent: Option<String>,
    pub registry_agent: Option<String>,
    pub registry_icon: Option<String>,
    pub badge: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    pub machine: String,
    pub entries: Vec<Entry>,
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn js_space(character: char) -> bool {
    matches!(character, '\u{0009}'..='\u{000d}'|'\u{0020}'|'\u{00a0}'|'\u{1680}'|'\u{2000}'..='\u{200a}'|'\u{2028}'|'\u{2029}'|'\u{202f}'|'\u{205f}'|'\u{3000}'|'\u{feff}')
}

fn trimmed(value: &str) -> &str {
    value.trim_matches(js_space)
}

pub fn initials(label: &str) -> String {
    static PUNCTUATION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\p{P}&&[^'’]]+").unwrap());
    let separated = PUNCTUATION.replace_all(label, " ");
    let words: Vec<_> = separated
        .split(js_space)
        .map(|word| word.trim_matches(['\'', '’']))
        .filter(|word| !word.is_empty())
        .collect();
    let letters: String = match words.as_slice() {
        [] => String::new(),
        [word] => word.chars().take(2).collect(),
        words => words
            .iter()
            .take(2)
            .filter_map(|word| word.chars().next())
            .collect(),
    };
    letters.to_uppercase()
}

fn humanize(slug: &str) -> String {
    static CAMEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([a-z])([A-Z])").unwrap());
    static SEPARATORS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[_-]+").unwrap());
    static INITIAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?-u:\b\w)").unwrap());
    let spaced = CAMEL.replace_all(slug, "$1 $2");
    let spaced = SEPARATORS.replace_all(&spaced, " ");
    INITIAL
        .replace_all(trimmed(&spaced), |captures: &regex::Captures<'_>| {
            captures[0].to_ascii_uppercase()
        })
        .into_owned()
}

pub fn display_name(provider: &Value) -> String {
    let driver = text(provider, "driver");
    let instance = text(provider, "instanceId");
    let label = match driver {
        "antigravity" => "Antigravity",
        "codex" => "Codex",
        "claudeAgent" => "Claude",
        "cursor" => "Cursor",
        "grok" => "Grok",
        "fx" => "fx",
        "kiro" => "Kiro",
        "muse" => "Muse Code",
        "acpRegistry" => "ACP Registry",
        "pi" => "Pi",
        "opencode" => "OpenCode",
        _ => "",
    };
    let kind_label = if label.is_empty() {
        humanize(driver)
    } else {
        label.to_string()
    };
    let name = trimmed(text(provider, "displayName"));
    if !name.is_empty() && name != kind_label {
        return name.to_string();
    }
    if instance != driver {
        let name = humanize(instance);
        if !name.is_empty() {
            return name;
        }
    }
    if name.is_empty() {
        kind_label
    } else {
        name.to_string()
    }
}

pub fn accent(value: &str) -> Option<String> {
    let value = trimmed(value);
    (value.len() == 7
        && value.starts_with('#')
        && value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit))
    .then(|| value.to_string())
}

pub fn accent_foreground(value: &str) -> &'static str {
    let channel = |offset| {
        let value = u8::from_str_radix(&value[offset..offset + 2], 16).unwrap() as f64 / 255.;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
    if (luminance + 0.05) / 0.05 >= 1.05 / (luminance + 0.05) {
        "#000000"
    } else {
        "#ffffff"
    }
}

fn show_badge(entry: &Entry, entries: &[Entry]) -> bool {
    entry.accent.is_some()
        || entries
            .iter()
            .filter(|other| {
                other.driver == entry.driver
                    && (!entry.enabled || other.enabled)
                    && (entry.driver != "acpRegistry"
                        || entry.registry_agent == other.registry_agent)
            })
            .take(2)
            .count()
            > 1
}

impl Catalog {
    pub fn from_config(config: &Value) -> Self {
        let mut entries: Vec<_> = config
            .get("providers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|provider| {
                let driver = text(provider, "driver").to_string();
                let id = text(provider, "instanceId");
                let mut entry = Entry {
                    instance_id: id.to_string(),
                    name: display_name(provider),
                    enabled: provider.get("enabled").and_then(Value::as_bool) == Some(true),
                    accent: accent(text(provider, "accentColor")),
                    registry_icon: (driver == "acpRegistry")
                        .then(|| text(provider, "iconUrl").to_string())
                        .filter(|value| !value.is_empty()),
                    registry_agent: None,
                    badge: false,
                    driver,
                };
                if entry.driver == "acpRegistry"
                    && let Some(instance) = config
                        .pointer("/settings/providerInstances")
                        .and_then(|instances| instances.get(id))
                {
                    let source = &instance["config"];
                    if text(source, "source") == "local" {
                        entry.registry_icon = None;
                    } else {
                        entry.registry_agent = source
                            .get("agentId")
                            .and_then(Value::as_str)
                            .map(trimmed)
                            .filter(|value| !value.is_empty())
                            .map(str::to_string);
                        if let Some(icon) = source
                            .get("registryIconUrl")
                            .and_then(Value::as_str)
                            .map(trimmed)
                            .filter(|value| !value.is_empty())
                        {
                            entry.registry_icon = Some(icon.to_string());
                        }
                    }
                }
                entry
            })
            .collect();
        let badges: Vec<_> = entries
            .iter()
            .map(|entry| show_badge(entry, &entries))
            .collect();
        for (entry, badge) in entries.iter_mut().zip(badges) {
            entry.badge = badge;
        }
        Self {
            entries,
            machine: config
                .pointer("/settings/environmentIcon")
                .and_then(Value::as_str)
                .or_else(|| {
                    config
                        .pointer("/environment/platform/machine")
                        .and_then(Value::as_str)
                })
                .unwrap_or("server")
                .to_string(),
        }
    }
}

pub fn stack(thread: &Value) -> Vec<String> {
    let current = thread
        .pointer("/runtime/providerInstanceId")
        .and_then(Value::as_str)
        .or_else(|| {
            thread
                .pointer("/modelSelection/instanceId")
                .and_then(Value::as_str)
        })
        .unwrap_or_default();
    let mut previous: Vec<_> = thread
        .get("providerInstanceHistory")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|value| *value != current)
        .collect();
    previous.drain(..previous.len().saturating_sub(2));
    previous.push(current);
    previous.into_iter().map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_names_badges_colors_history_and_machines_match_the_original() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/provider-display-oracle.json"
        ))
        .unwrap();
        for case in fixture["initials"].as_array().unwrap() {
            assert_eq!(
                initials(case["input"].as_str().unwrap()),
                case["expected"].as_str().unwrap(),
                "{case}"
            );
        }
        for case in fixture["names"].as_array().unwrap() {
            assert_eq!(
                display_name(&case["input"]),
                case["expected"].as_str().unwrap(),
                "{case}"
            );
        }
        for case in fixture["accents"].as_array().unwrap() {
            assert_eq!(
                accent(case["input"].as_str().unwrap()).as_deref(),
                case["expected"].as_str(),
                "{case}"
            );
        }
        for case in fixture["contrast"].as_array().unwrap() {
            assert_eq!(
                accent_foreground(case["input"].as_str().unwrap()),
                case["expected"].as_str().unwrap(),
                "{case}"
            );
        }
        for case in fixture["stacks"].as_array().unwrap() {
            assert_eq!(
                serde_json::to_value(stack(&case["input"])).unwrap(),
                case["expected"],
                "{case}"
            );
        }
        for case in fixture["catalogs"].as_array().unwrap() {
            let catalog = Catalog::from_config(&case["input"]);
            assert_eq!(catalog.machine, case["machine"].as_str().unwrap(), "{case}");
            assert_eq!(
                serde_json::to_value(catalog.entries).unwrap(),
                case["entries"],
                "{case}"
            );
        }
    }
}
