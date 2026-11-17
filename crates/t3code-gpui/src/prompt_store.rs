use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
pub struct SavedAttachment {
    pub name: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub created_at: u64,
    pub prompt: String,
    pub attachments: Vec<SavedAttachment>,
    pub dropped: Vec<String>,
}

pub fn relative_time(created_at: u64, now: u64) -> String {
    let seconds = now.saturating_sub(created_at);
    match seconds {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", seconds / 60),
        3600..86400 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86400),
    }
}
#[derive(Default, Serialize, Deserialize)]
pub struct Store {
    pub entries: Vec<Entry>,
}
impl Store {
    pub fn path(drafts: &Path, pane_id: u64) -> PathBuf {
        drafts.with_file_name(format!("prompt-stash-{pane_id}.json"))
    }
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("Invalid prompt stash"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
    fn save_entries(path: &Path, entries: &[Entry]) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let partial = path.with_extension("json.partial");
        std::fs::write(
            &partial,
            serde_json::to_vec(&serde_json::json!({"entries": entries}))?,
        )?;
        std::fs::rename(partial, path)?;
        Ok(())
    }
    pub fn push(&mut self, path: &Path, entry: Entry) -> Result<bool> {
        let mut next = self.entries.clone();
        next.insert(0, entry);
        let evicted = next.len() > 20;
        next.truncate(20);
        Self::save_entries(path, &next)?;
        self.entries = next;
        Ok(evicted)
    }
    pub fn take(&mut self, path: &Path, id: &str) -> Result<Option<Entry>> {
        let Some(index) = self.entries.iter().position(|entry| entry.id == id) else {
            return Ok(None);
        };
        let mut next = self.entries.clone();
        let entry = next.remove(index);
        Self::save_entries(path, &next)?;
        self.entries = next;
        Ok(Some(entry))
    }
}

#[derive(Clone)]
pub struct Position {
    pub target: String,
    pub id: String,
    pub recalled: String,
}

pub fn history(messages: &[Value]) -> Vec<(String, String)> {
    let mut entries: Vec<(String, String)> = vec![];
    for message in messages {
        if !message["context"]["guest"].is_null()
            || !message["scheduledTaskId"].is_null()
            || (message["role"] != "user" && message["type"] != "user_message")
            || message["createdBy"].as_str().is_some_and(|v| v != "user")
        {
            continue;
        }
        let Some(raw) = message["text"].as_str() else {
            continue;
        };
        let prompt = recallable(raw);
        if prompt.is_empty() {
            continue;
        }
        let Some(id) = message["id"].as_str() else {
            continue;
        };
        if entries
            .last()
            .is_some_and(|(_, previous)| previous == &prompt)
        {
            entries.pop();
        }
        entries.push((id.into(), prompt));
    }
    entries
}

pub fn recallable(text: &str) -> String {
    use std::sync::LazyLock;
    static REVIEW: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?s)<review_comment\b[^>]*>.*?</review_comment>")
            .expect("review comments")
    });
    static HEADER: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?m)^- (.+):$").expect("context headers"));
    static CONTEXT: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"!?\[[^\]\n]{0,512}\]\(t3-context://v1/[a-z][a-z0-9-]{0,39}/[a-zA-Z0-9_-]{1,128}\)",
        )
        .expect("context references")
    });
    let mut prompt = text
        .trim()
        .strip_prefix("Ultrathink:\n")
        .unwrap_or(text.trim())
        .to_string();
    loop {
        let mut cut = prompt.len();
        let matches = REVIEW.find_iter(&prompt).collect::<Vec<_>>();
        for found in matches.into_iter().rev() {
            if !prompt[found.end()..cut].trim().is_empty() {
                break;
            }
            cut = found.start();
        }
        if cut < prompt.len() {
            prompt.truncate(cut);
            prompt = prompt.trim_end().to_string();
            continue;
        }
        let mut stripped = false;
        for tag in ["terminal_context", "element_context", "preview_annotation"] {
            let closing = format!("\n</{tag}>");
            let Some(body_end) = prompt.strip_suffix(&closing).map(str::len) else {
                continue;
            };
            let opening = format!("<{tag}>\n");
            let Some(start) = prompt[..body_end].rfind(&opening) else {
                continue;
            };
            let body = &prompt[start + opening.len()..body_end];
            let headers = HEADER
                .captures_iter(body)
                .map(|c| c[1].to_string())
                .collect::<Vec<_>>();
            if tag != "preview_annotation" && headers.is_empty() {
                continue;
            }
            if tag == "preview_annotation"
                && body
                    .lines()
                    .any(|line| line.trim() == "<preview_annotation>")
                && !body.lines().any(|line| {
                    ["Id: ", "Page: ", "Comment: ", "Targets: "]
                        .iter()
                        .any(|prefix| line.starts_with(prefix))
                })
            {
                continue;
            }
            prompt.truncate(start);
            prompt = prompt.trim_end().to_string();
            if tag == "terminal_context" {
                for header in headers {
                    let Some((label, range)) = header
                        .rsplit_once(" lines ")
                        .or_else(|| header.rsplit_once(" line "))
                    else {
                        continue;
                    };
                    if !range.chars().all(|c| c.is_ascii_digit() || c == '-') {
                        continue;
                    }
                    let label = format!(
                        "@{}:{range}",
                        label
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join("-")
                            .to_lowercase()
                    );
                    if let Some(at) = prompt.find(&label) {
                        let end = at + label.len();
                        let before = prompt[..at].chars().next_back();
                        let after = prompt[end..].chars().next();
                        if before.is_some_and(|c| c.is_alphanumeric() || "@.-_".contains(c))
                            || after.is_some_and(|c| c.is_ascii_digit() || c == '-')
                        {
                            continue;
                        }
                        let (start, end) = if prompt[end..].starts_with(' ') {
                            (at, end + 1)
                        } else if prompt[..at].ends_with(' ') {
                            (at - 1, end)
                        } else {
                            (at, end)
                        };
                        prompt.replace_range(start..end, "");
                    }
                }
            }
            stripped = true;
            break;
        }
        if !stripped {
            break;
        }
    }
    for found in CONTEXT
        .find_iter(&prompt)
        .map(|m| m.range())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let (start, end) = if prompt[found.end..].starts_with(' ') {
            (found.start, found.end + 1)
        } else if prompt[..found.start].ends_with(' ') {
            (found.start - 1, found.end)
        } else {
            (found.start, found.end)
        };
        prompt.replace_range(start..end, "");
    }
    let trimmed = prompt.trim();
    if trimmed.starts_with("PLEASE IMPLEMENT THIS PLAN:\n")
        || trimmed
            == "[User attached one or more files without additional text. Respond using the conversation context and the attached files.]"
    {
        String::new()
    } else {
        trimmed.into()
    }
}

pub fn step(
    entries: &[(String, String)],
    position: Option<&Position>,
    target: &str,
    current: &str,
    backward: bool,
) -> Option<(Option<Position>, String)> {
    let active = position
        .filter(|p| p.target == target && p.recalled == current)
        .and_then(|p| {
            entries
                .iter()
                .position(|(id, _)| id == &p.id)
                .or_else(|| entries.iter().rposition(|(_, text)| text == &p.recalled))
        });
    let index = if backward {
        if active.is_none() && !current.is_empty() {
            return None;
        }
        active.unwrap_or(entries.len()).checked_sub(1)?
    } else {
        active? + 1
    };
    let Some((id, prompt)) = entries.get(index) else {
        return Some((None, String::new()));
    };
    Some((
        Some(Position {
            target: target.into(),
            id: id.clone(),
            recalled: prompt.clone(),
        }),
        prompt.clone(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stash_relative_time_matches_reference_boundaries() {
        for (elapsed, expected) in [
            (0, "just now"),
            (59, "just now"),
            (60, "1m ago"),
            (3599, "59m ago"),
            (3600, "1h ago"),
            (86399, "23h ago"),
            (86400, "1d ago"),
        ] {
            assert_eq!(relative_time(100, 100 + elapsed), expected);
        }
        assert_eq!(relative_time(101, 100), "just now");
    }
    #[test]
    fn pane_stashes_survive_independent_updates() {
        let directory = tempfile::tempdir().unwrap();
        let drafts = directory.path().join("drafts.json");
        let first = Store::path(&drafts, 7);
        let second = Store::path(&drafts, 8);
        for (path, prompt) in [(&first, "first pane"), (&second, "second pane")] {
            Store::default()
                .push(
                    path,
                    Entry {
                        id: "same-entry-id".into(),
                        created_at: 100,
                        prompt: prompt.into(),
                        attachments: vec![],
                        dropped: vec![],
                    },
                )
                .unwrap();
        }
        let mut store = Store::load(&first).unwrap();
        assert_eq!(
            store.take(&first, "same-entry-id").unwrap().unwrap().prompt,
            "first pane"
        );
        assert!(Store::load(&first).unwrap().entries.is_empty());
        assert_eq!(
            Store::load(&second).unwrap().entries[0].prompt,
            "second pane"
        );
    }

    #[test]
    fn stash_is_durable_and_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stash.json");
        let mut store = Store::default();
        for n in 0..21 {
            store
                .push(
                    &path,
                    Entry {
                        id: n.to_string(),
                        created_at: 100,
                        prompt: "日本語".into(),
                        attachments: vec![],
                        dropped: vec![],
                    },
                )
                .unwrap();
        }
        let mut loaded = Store::load(&path).unwrap();
        assert_eq!(loaded.entries.len(), 20);
        assert_eq!(loaded.entries[0].id, "20");
        assert!(loaded.take(&path, "20").unwrap().is_some());
        assert_eq!(Store::load(&path).unwrap().entries.len(), 19);
        let before = loaded.entries.len();
        assert!(
            loaded
                .push(
                    dir.path(),
                    Entry {
                        id: "failed".into(),
                        created_at: 100,
                        prompt: "keep".into(),
                        attachments: vec![],
                        dropped: vec![]
                    }
                )
                .is_err()
        );
        assert_eq!(loaded.entries.len(), before);
    }
    #[test]
    fn history_keeps_user_prompts_and_restores_empty_draft() {
        let entries = history(
            &serde_json::from_value::<Vec<Value>>(serde_json::json!([
                {"id":"1","role":"user","text":"one","createdBy":"user"},
                {"id":"2","role":"user","text":"one","createdBy":"user"},
                {"id":"3","role":"user","text":"two","createdBy":"user"},
                {"id":"4","role":"assistant","text":"answer"}
            ]))
            .unwrap(),
        );
        assert_eq!(entries.len(), 2);
        let (position, text) = step(&entries, None, "a", "", true).unwrap();
        assert_eq!(text, "two");
        assert!(step(&entries, position.as_ref(), "b", &text, true).is_none());
        assert!(step(&entries, position.as_ref(), "a", "edited", true).is_none());
        assert_eq!(
            step(&entries, position.as_ref(), "a", &text, false)
                .unwrap()
                .1,
            ""
        );
    }
}
