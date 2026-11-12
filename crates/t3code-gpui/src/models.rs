use serde_json::{Value, json};

const DEFAULT_RUNTIME_MODE: &str = "full-access";
pub const RUNTIME_MODES: &[&str] = &[
    "approval-required",
    "auto-accept-edits",
    "auto",
    "full-access",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    pub slug: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub instance_id: String,
    pub name: String,
    pub driver: String,
    pub models: Vec<Model>,
    pub default_model: Option<String>,
    pub runtime_modes: Vec<String>,
    pub interaction_mode_toggle: bool,
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn present(value: Option<&Value>) -> Option<&Value> {
    value.filter(|v| !v.is_null())
}

fn default_model(provider: &Value) -> Option<String> {
    let models = provider.get("models").and_then(Value::as_array)?;
    let custom = |m: &Value| m.get("isCustom").and_then(Value::as_bool) == Some(true);
    models
        .iter()
        .find(|m| m.get("isDefault").and_then(Value::as_bool) == Some(true) && !custom(m))
        .or_else(|| models.iter().find(|m| !custom(m)))
        .or_else(|| models.first())
        .map(|m| text(m, "slug").to_string())
}

/// The provider instances a thread can run on: enabled, installed and not reported unavailable.
pub fn instances(config: &Value) -> Vec<Instance> {
    config
        .get("providers")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|p| p.get("enabled").and_then(Value::as_bool) == Some(true))
        .filter(|p| text(p, "availability") != "unavailable" && text(p, "status") != "error")
        .map(|p| {
            let instance_id = text(p, "instanceId").to_string();
            let name = p
                .get("displayName")
                .and_then(Value::as_str)
                .unwrap_or(&instance_id)
                .to_string();
            let runtime_modes = p
                .get("supportedRuntimeModes")
                .and_then(Value::as_array)
                .map(|modes| {
                    modes
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|m| RUNTIME_MODES.contains(m))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_else(|| RUNTIME_MODES.iter().map(|m| m.to_string()).collect());
            Instance {
                name,
                driver: text(p, "driver").to_string(),
                models: p
                    .get("models")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    .map(|m| Model {
                        slug: text(m, "slug").to_string(),
                        name: m
                            .get("shortName")
                            .or_else(|| m.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or(text(m, "slug"))
                            .to_string(),
                    })
                    .collect(),
                default_model: default_model(p),
                runtime_modes,
                interaction_mode_toggle: p
                    .get("showInteractionModeToggle")
                    .and_then(Value::as_bool)
                    == Some(true),
                instance_id,
            }
        })
        .collect()
}

pub fn selection(instance_id: &str, model: &str) -> Value {
    json!({"instanceId": instance_id, "model": model})
}

/// The model of the first instance a thread can run on, as upstream picks a provider default.
pub fn provider_default(config: &Value) -> Option<Value> {
    instances(config)
        .into_iter()
        .find_map(|i| i.default_model.map(|m| selection(&i.instance_id, &m)))
}

/// A project-scoped server setting: the project's override, then the project's own field (until
/// the server folds those into overrides), then the environment-wide setting.
fn project_setting<'a>(settings: &'a Value, project: &'a Value, key: &str) -> Option<&'a Value> {
    let project_id = text(project, "id");
    let stored = present(
        settings
            .get("projectSettingsOverrides")
            .and_then(|o| o.get(project_id))
            .and_then(|o| o.get(key)),
    );
    let folded = settings
        .get("projectSettingsFolded")
        .and_then(Value::as_bool)
        == Some(true);
    let legacy = (!folded && key == "defaultModelSelection")
        .then(|| present(project.get(key)))
        .flatten();
    stored.or(legacy).or_else(|| present(settings.get(key)))
}

/// The model and runtime mode of a new thread in the project. The model is the project default,
/// else the model of the thread the user came from, else the provider default.
pub fn new_thread(
    config: Option<&Value>,
    project: &Value,
    carry: Option<&Value>,
) -> Option<(Value, String)> {
    let empty = Value::Null;
    let settings = config.and_then(|c| c.get("settings")).unwrap_or(&empty);
    let model = project_setting(settings, project, "defaultModelSelection")
        .or(present(carry))
        .cloned()
        .or_else(|| config.and_then(provider_default))?;
    let runtime_mode = project_setting(settings, project, "defaultRuntimeMode")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_RUNTIME_MODE)
        .to_string();
    Some((model, runtime_mode))
}

/// "Instance · model" for a model selection, with the names the server reports.
pub fn label(config: Option<&Value>, selection: &Value) -> String {
    let instance_id = text(selection, "instanceId");
    let slug = text(selection, "model");
    let instance = config.and_then(|c| {
        instances(c)
            .into_iter()
            .find(|i| i.instance_id == instance_id)
    });
    match instance {
        Some(instance) => {
            let model = instance
                .models
                .iter()
                .find(|m| m.slug == slug)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| slug.to_string());
            format!("{} · {model}", instance.name)
        }
        None => format!("{instance_id} · {slug}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(settings: Value) -> Value {
        json!({
            "providers": [
                {"instanceId": "off", "driver": "codex", "enabled": false, "models": [{"slug": "x", "name": "X", "isCustom": false}]},
                {"instanceId": "gone", "driver": "codex", "enabled": true, "availability": "unavailable", "models": []},
                {"instanceId": "claude_conao3", "driver": "claudeAgent", "displayName": "Claude conao3", "enabled": true,
                 "showInteractionModeToggle": true, "supportedRuntimeModes": ["approval-required", "full-access", "later-mode"],
                 "models": [
                    {"slug": "custom", "name": "Custom", "isCustom": true, "isDefault": true},
                    {"slug": "claude-opus-5-5", "name": "Claude Opus 5.5", "shortName": "Opus 5.5", "isCustom": false},
                    {"slug": "claude-sonnet-5-5", "name": "Claude Sonnet 5.5", "isCustom": false, "isDefault": true}]},
            ],
            "settings": settings,
        })
    }

    #[test]
    fn only_runnable_instances_are_offered_with_their_default_model() {
        let list = instances(&config(json!({})));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Claude conao3");
        assert_eq!(list[0].default_model.as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(list[0].models[1].name, "Opus 5.5");
        assert_eq!(list[0].runtime_modes, ["approval-required", "full-access"]);
        assert!(list[0].interaction_mode_toggle);
        assert_eq!(
            provider_default(&config(json!({}))),
            Some(selection("claude_conao3", "claude-sonnet-5-5"))
        );
    }

    #[test]
    fn a_new_thread_takes_the_override_then_the_project_then_the_setting_then_the_carry() {
        let project = json!({"id": "p", "defaultModelSelection": selection("legacy", "l")});
        let bare = json!({"id": "p", "defaultModelSelection": null});
        let carry = selection("claude_conao3", "claude-opus-5-5");
        let overridden = config(json!({
            "projectSettingsOverrides": {"p": {"defaultModelSelection": selection("o", "m"), "defaultRuntimeMode": "auto"}},
            "defaultModelSelection": selection("g", "m"),
            "defaultRuntimeMode": "approval-required",
        }));
        assert_eq!(
            new_thread(Some(&overridden), &project, Some(&carry)),
            Some((selection("o", "m"), "auto".into()))
        );
        let global = config(
            json!({"defaultModelSelection": selection("g", "m"), "defaultRuntimeMode": "approval-required"}),
        );
        assert_eq!(
            new_thread(Some(&global), &project, None).unwrap().0,
            selection("legacy", "l")
        );
        assert_eq!(
            new_thread(Some(&global), &bare, None),
            Some((selection("g", "m"), "approval-required".into()))
        );
        let plain = config(json!({}));
        assert_eq!(
            new_thread(Some(&plain), &bare, Some(&carry)),
            Some((carry.clone(), "full-access".into()))
        );
        assert_eq!(
            new_thread(Some(&plain), &bare, None).unwrap().0,
            selection("claude_conao3", "claude-sonnet-5-5")
        );
        assert_eq!(new_thread(None, &bare, None), None);
    }

    #[test]
    fn labels_use_the_server_names() {
        let c = config(json!({}));
        assert_eq!(
            label(Some(&c), &selection("claude_conao3", "claude-opus-5-5")),
            "Claude conao3 · Opus 5.5"
        );
        assert_eq!(label(Some(&c), &selection("other", "m")), "other · m");
    }
}
