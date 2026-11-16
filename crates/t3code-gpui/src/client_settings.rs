use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendShortcut {
    Enter,
    ModEnterMultiline,
    ModEnter,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClientSettings {
    pub send_shortcut: SendShortcut,
    pub steer_follow_ups: bool,
    pub desktop_notifications: bool,
    pub notification_sound: bool,
    pub plan_mode_enabled: bool,
    pub font_size_interface: f32,
    pub font_size_prompt: f32,
    pub font_size_code: f32,
    pub sidebar_width: f32,
    pub light_theme: bool,
    pub preferred_editor: Option<String>,
}

impl Default for ClientSettings {
    fn default() -> Self {
        Self::from_value(&Value::Null)
    }
}

impl ClientSettings {
    /// The settings as T3's `ClientSettings` schema decodes them, with its defaults for absent keys.
    pub fn from_value(value: &Value) -> Self {
        let text = |key: &str| value.get(key).and_then(Value::as_str);
        let number = |key: &str, default: f32| {
            value
                .get(key)
                .and_then(Value::as_f64)
                .map_or(default, |n| n as f32)
        };
        let notification_mode = text("notificationMode").unwrap_or("off");
        Self {
            send_shortcut: match text("sendShortcut") {
                Some("mod-enter") => SendShortcut::ModEnter,
                Some("mod-enter-multiline") => SendShortcut::ModEnterMultiline,
                _ => SendShortcut::Enter,
            },
            steer_follow_ups: text("followUpBehavior") != Some("queue"),
            desktop_notifications: matches!(
                notification_mode,
                "notifications" | "notifications-and-sound"
            ),
            notification_sound: matches!(notification_mode, "sound" | "notifications-and-sound"),
            plan_mode_enabled: value.get("planModeEnabled").and_then(Value::as_bool) == Some(true),
            font_size_interface: number("fontSizeInterface", 16.),
            font_size_prompt: number("fontSizePrompt", 14.),
            font_size_code: number("fontSizeCode", 13.),
            sidebar_width: number("gpuiSidebarWidth", 256.).max(208.),
            light_theme: text("gpuiAppearance") == Some("light"),
            preferred_editor: text("gpuiPreferredEditor").map(str::to_string),
        }
    }

    /// Whether Enter alone submits `text`, or the submit needs the modifier.
    pub fn requires_modifier(&self, text: &str) -> bool {
        match self.send_shortcut {
            SendShortcut::Enter => false,
            SendShortcut::ModEnter => true,
            SendShortcut::ModEnterMultiline => text.contains(['\n', '\r']),
        }
    }

    /// The settings file of the T3 home: `$T3CODE_HOME`, else `~/.t3`. A missing file means
    /// the defaults, as in T3.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let value: Value = serde_json::from_str(&text)
                    .with_context(|| format!("{} is not valid JSON", path.display()))?;
                Ok(Self::from_value(&value))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("cannot read {}", path.display())),
        }
    }
}

/// Sets keys in the settings file, keeping every other key the file holds (the official app's
/// settings this app does not use), and returns the settings as they now read.
pub fn update(path: &Path, changes: &[(&str, Value)]) -> Result<ClientSettings> {
    let mut value = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<Value>(&text)
            .with_context(|| format!("{} is not valid JSON", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Value::Object(Default::default())
        }
        Err(error) => return Err(error).with_context(|| format!("cannot read {}", path.display())),
    };
    let object = value
        .as_object_mut()
        .with_context(|| format!("{} does not hold an object", path.display()))?;
    for (key, change) in changes {
        object.insert(key.to_string(), change.clone());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    let partial = path.with_extension("json.partial");
    std::fs::write(&partial, serde_json::to_vec_pretty(&value)?)
        .with_context(|| format!("cannot write {}", partial.display()))?;
    std::fs::rename(&partial, path)
        .with_context(|| format!("cannot replace {}", path.display()))?;
    Ok(ClientSettings::from_value(&value))
}

pub fn default_path() -> Result<PathBuf> {
    let home = match std::env::var_os("T3CODE_HOME") {
        Some(home) => PathBuf::from(home),
        None => PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?).join(".t3"),
    };
    Ok(home.join("userdata").join("client-settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn absent_keys_take_the_t3_defaults() {
        let settings = ClientSettings::default();
        assert_eq!(settings.send_shortcut, SendShortcut::Enter);
        assert!(settings.steer_follow_ups);
        assert!(!settings.desktop_notifications);
        assert!(!settings.plan_mode_enabled);
        assert_eq!(
            (
                settings.font_size_interface,
                settings.font_size_prompt,
                settings.font_size_code
            ),
            (16., 14., 13.)
        );
    }

    #[test]
    fn the_send_shortcut_decides_when_enter_submits() {
        let multiline = ClientSettings::from_value(&json!({"sendShortcut": "mod-enter-multiline"}));
        assert!(!multiline.requires_modifier("one line"));
        assert!(multiline.requires_modifier("two\nlines"));
        let always = ClientSettings::from_value(&json!({"sendShortcut": "mod-enter"}));
        assert!(always.requires_modifier("one line"));
        assert!(!ClientSettings::default().requires_modifier("a\nb"));
    }

    #[test]
    fn the_eos_settings_read_as_written() {
        let eos = ClientSettings::from_value(&json!({
            "sendShortcut": "enter", "followUpBehavior": "queue", "notificationMode": "notifications-and-sound",
            "fontSizeInterface": 17, "fontSizePrompt": 16, "fontSizeCode": 14, "planModeEnabled": true,
        }));
        assert!(!eos.steer_follow_ups);
        assert!(eos.desktop_notifications && eos.notification_sound);
        assert!(eos.plan_mode_enabled);
        assert_eq!(eos.font_size_interface, 17.);
    }

    #[test]
    fn a_missing_file_is_the_defaults_and_a_broken_one_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            ClientSettings::load(&dir.path().join("none.json")).unwrap(),
            ClientSettings::default()
        );
        let broken = dir.path().join("broken.json");
        std::fs::write(&broken, "{").unwrap();
        assert!(ClientSettings::load(&broken).is_err());
    }

    #[test]
    fn updates_keep_the_keys_this_app_does_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("userdata/client-settings.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"browserSearchEngine": "duckduckgo", "sendShortcut": "enter"}"#,
        )
        .unwrap();
        let settings = update(
            &path,
            &[
                ("sendShortcut", json!("mod-enter")),
                ("fontSizeInterface", json!(18)),
            ],
        )
        .unwrap();
        assert_eq!(settings.send_shortcut, SendShortcut::ModEnter);
        assert_eq!(settings.font_size_interface, 18.);
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["browserSearchEngine"], "duckduckgo");
    }
}
