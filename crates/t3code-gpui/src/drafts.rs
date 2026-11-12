use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

/// Unsent composer text per thread (`<host>/<thread id>`) and for the new-thread pane (`new`),
/// kept on disk so it survives a restart.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Drafts {
    texts: BTreeMap<String, String>,
}

pub const NEW_THREAD: &str = "new";

pub fn thread_key(environment: &str, thread_id: &str) -> String {
    format!("{environment}/{thread_id}")
}

impl Drafts {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.texts.get(key).map(String::as_str)
    }

    /// Keeps `text` as the draft of `key`, or forgets the draft when the text is blank. Returns
    /// whether anything changed.
    pub fn set(&mut self, key: &str, text: &str) -> bool {
        if text.trim().is_empty() {
            self.texts.remove(key).is_some()
        } else if self.texts.get(key).map(String::as_str) != Some(text) {
            self.texts.insert(key.to_string(), text.to_string());
            true
        } else {
            false
        }
    }

    pub fn has(&self, key: &str) -> bool {
        self.texts.contains_key(key)
    }

    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Self {
                texts: serde_json::from_str(&text)
                    .with_context(|| format!("{} is not a drafts file", path.display()))?,
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
        let partial = path.with_extension("json.partial");
        std::fs::write(&partial, serde_json::to_vec_pretty(&self.texts)?)
            .with_context(|| format!("cannot write {}", partial.display()))?;
        std::fs::rename(&partial, path)
            .with_context(|| format!("cannot replace {}", path.display()))
    }
}

/// `$XDG_STATE_HOME/t3code-gpui/drafts.json`, else under `~/.local/state`.
pub fn default_path() -> Result<PathBuf> {
    let state = match std::env::var_os("XDG_STATE_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => {
            PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?).join(".local/state")
        }
    };
    Ok(state.join("t3code-gpui").join("drafts.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_text_forgets_the_draft() {
        let mut drafts = Drafts::default();
        assert!(drafts.set("eos/t1", "half a thought"));
        assert!(!drafts.set("eos/t1", "half a thought"));
        assert!(drafts.has("eos/t1"));
        assert!(drafts.set("eos/t1", "  \n"));
        assert!(!drafts.has("eos/t1"));
        assert!(!drafts.set("eos/t2", ""));
    }

    #[test]
    fn drafts_survive_a_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/drafts.json");
        assert_eq!(Drafts::load(&path).unwrap(), Drafts::default());
        let mut drafts = Drafts::default();
        drafts.set(&thread_key("eos", "t1"), "日本語の下書き");
        drafts.set(NEW_THREAD, "new idea");
        drafts.save(&path).unwrap();
        let loaded = Drafts::load(&path).unwrap();
        assert_eq!(loaded.get("eos/t1"), Some("日本語の下書き"));
        assert_eq!(loaded, drafts);
    }
}
