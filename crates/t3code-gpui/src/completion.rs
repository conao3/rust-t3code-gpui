use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerKind {
    SlashCommand,
    Skill,
    Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trigger {
    pub kind: TriggerKind,
    pub query: String,
    pub start: usize,
    pub end: usize,
}

/// The completion the word before the cursor asks for, as T3's composer reads it: `/` for a
/// slash command (unless the word is a path such as `/usr/bin`), a currency sign for a skill,
/// and `@` for a file. `cursor` is a byte offset into `text`.
pub fn trigger(text: &str, cursor: usize) -> Option<Trigger> {
    let cursor = cursor.min(text.len());
    let cursor = (0..=cursor).rev().find(|&i| text.is_char_boundary(i))?;
    let start = text[..cursor]
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(i, c)| i + c.len_utf8());
    let token = &text[start..cursor];
    let mut chars = token.chars();
    let first = chars.next()?;
    let rest: String = chars.collect();
    let kind = match first {
        '/' if !rest.contains('/') => TriggerKind::SlashCommand,
        '@' => TriggerKind::Path,
        '$' | '¥' | '€' | '£' if !rest.starts_with(|c: char| c.is_ascii_digit()) => {
            TriggerKind::Skill
        }
        _ => return None,
    };
    Some(Trigger {
        kind,
        query: rest,
        start,
        end: cursor,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub label: String,
    pub detail: String,
    pub insert: String,
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// The slash commands and user-invocable skills a provider instance offers, matching the query
/// by prefix, then by substring.
pub fn provider_suggestions(provider: &Value, kind: TriggerKind, query: &str) -> Vec<Suggestion> {
    let query = query.to_lowercase();
    let list = |key: &str| {
        provider
            .get(key)
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .to_vec()
    };
    let mut found: Vec<(bool, Suggestion)> = match kind {
        TriggerKind::SlashCommand => list("slashCommands")
            .iter()
            .chain(
                list("skills")
                    .iter()
                    .filter(|s| s.get("userInvocationOnly").and_then(Value::as_bool) == Some(true)),
            )
            .map(|c| Suggestion {
                label: format!("/{}", text(c, "name")),
                detail: text(c, "description").to_string(),
                insert: format!("/{} ", text(c, "name")),
            })
            .collect::<Vec<_>>(),
        TriggerKind::Skill => list("skills")
            .iter()
            .filter(|s| s.get("enabled").and_then(Value::as_bool) != Some(false))
            .filter(|s| s.get("userInvocable").and_then(Value::as_bool) != Some(false))
            .map(|s| Suggestion {
                label: format!("${}", text(s, "name")),
                detail: s
                    .get("shortDescription")
                    .or_else(|| s.get("description"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                insert: format!("${} ", text(s, "name")),
            })
            .collect(),
        TriggerKind::Path => Vec::new(),
    }
    .into_iter()
    .filter_map(|s| {
        let name = s.label[1..].to_lowercase();
        if name.starts_with(&query) {
            Some((true, s))
        } else if name.contains(&query) {
            Some((false, s))
        } else {
            None
        }
    })
    .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.label.cmp(&b.1.label)));
    found.dedup_by(|a, b| a.1.label == b.1.label);
    found.into_iter().map(|(_, s)| s).collect()
}

pub fn path_suggestions(result: &Value) -> Vec<Suggestion> {
    result
        .get("entries")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|e| {
            let path = text(e, "path");
            Suggestion {
                label: format!("@{path}"),
                detail: text(e, "kind").to_string(),
                insert: format!("@{path} "),
            }
        })
        .collect()
}

/// The text with the trigger's word replaced by the suggestion, and the byte offset after it.
pub fn apply(text: &str, trigger: &Trigger, suggestion: &Suggestion) -> (String, usize) {
    let mut out = String::with_capacity(text.len() + suggestion.insert.len());
    out.push_str(&text[..trigger.start]);
    out.push_str(&suggestion.insert);
    let cursor = out.len();
    out.push_str(&text[trigger.end..]);
    (out, cursor)
}

/// The 0-based line and column, in characters, of a byte offset, as the input's cursor takes it.
pub fn line_column(text: &str, offset: usize) -> (u32, u32) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count();
    let column = before
        .rsplit('\n')
        .next()
        .unwrap_or_default()
        .chars()
        .count();
    (line as u32, column as u32)
}

/// A file as the markdown the preview renders: markdown as it is, anything else inside a fence
/// named by its extension, so code is highlighted. The fence is longer than any run of backticks
/// in the file.
pub fn file_markdown(path: &str, contents: &str) -> String {
    let extension = path
        .rsplit('.')
        .next()
        .filter(|e| *e != path)
        .unwrap_or_default();
    if matches!(extension, "md" | "markdown") {
        return contents.to_string();
    }
    let longest = contents
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}{extension}\n{contents}\n{fence}\n")
}

/// The diff sources T3 reports for a checkout as markdown: a heading per source with its file
/// counts, and its diff in a `diff` fence.
pub fn diff_markdown(preview: &Value) -> String {
    let sources = preview
        .get("sources")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if sources.is_empty() {
        return "No changes.".into();
    }
    let mut out = String::new();
    for source in sources {
        out.push_str(&format!("## {}\n\n", text(source, "title")));
        for file in source
            .get("files")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let count = |key: &str| file.get(key).and_then(Value::as_u64).unwrap_or(0);
            out.push_str(&format!(
                "- `{}` +{} -{}\n",
                text(file, "path"),
                count("additions"),
                count("deletions")
            ));
        }
        let diff = text(source, "diff");
        if diff.is_empty() {
            out.push_str("\nNo changes.\n\n");
        } else {
            out.push('\n');
            out.push_str(&file_markdown("changes.diff", diff));
            out.push('\n');
        }
        if source.get("truncated").and_then(Value::as_bool) == Some(true) {
            out.push_str("The diff is longer than T3 sends; the rest is left out.\n\n");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_word_before_the_cursor_picks_the_completion() {
        let t = trigger("please /rev", 11).unwrap();
        assert_eq!(
            (t.kind, t.query.as_str(), t.start, t.end),
            (TriggerKind::SlashCommand, "rev", 7, 11)
        );
        assert_eq!(trigger("see /usr/bin", 12), None);
        let at = trigger("look at @src/ma", 15).unwrap();
        assert_eq!((at.kind, at.query.as_str()), (TriggerKind::Path, "src/ma"));
        assert_eq!(trigger("$tdd", 4).unwrap().kind, TriggerKind::Skill);
        assert_eq!(trigger("costs $5", 8), None);
        assert_eq!(trigger("plain words", 11), None);
        assert_eq!(
            trigger("日本語 @ファ", "日本語 @ファ".len()).unwrap().query,
            "ファ"
        );
    }

    #[test]
    fn provider_suggestions_rank_prefix_matches_first() {
        let provider = json!({
            "slashCommands": [{"name": "review", "description": "Review"}, {"name": "preview"}, {"name": "init"}],
            "skills": [
                {"name": "tdd", "enabled": true, "shortDescription": "Test first"},
                {"name": "hidden", "enabled": true, "userInvocable": false},
                {"name": "manual", "enabled": true, "userInvocationOnly": true},
            ],
        });
        let labels = |kind, q| {
            provider_suggestions(&provider, kind, q)
                .into_iter()
                .map(|s| s.label)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            labels(TriggerKind::SlashCommand, "rev"),
            ["/review", "/preview"]
        );
        assert_eq!(labels(TriggerKind::SlashCommand, "man"), ["/manual"]);
        assert_eq!(labels(TriggerKind::Skill, ""), ["$manual", "$tdd"]);
    }

    #[test]
    fn applying_a_suggestion_replaces_only_the_word() {
        let t = trigger("do /re now", 6).unwrap();
        let s = Suggestion {
            label: "/review".into(),
            detail: String::new(),
            insert: "/review ".into(),
        };
        assert_eq!(
            apply("do /re now", &t, &s),
            ("do /review  now".to_string(), 11)
        );
        let paths = path_suggestions(
            &json!({"entries": [{"path": "src/main.rs", "kind": "file"}], "truncated": false}),
        );
        assert_eq!(paths[0].insert, "@src/main.rs ");
    }

    #[test]
    fn offsets_turn_into_lines_and_columns_in_characters() {
        assert_eq!(line_column("abc", 3), (0, 3));
        assert_eq!(
            line_column("one\nにほんご x", "one\nにほんご".len()),
            (1, 4)
        );
    }

    #[test]
    fn files_render_as_markdown_or_fenced_code() {
        assert_eq!(file_markdown("README.md", "# hi"), "# hi");
        assert_eq!(
            file_markdown("src/main.rs", "fn main() {}"),
            "```rs\nfn main() {}\n```\n"
        );
        assert_eq!(
            file_markdown("notes.txt", "a ```` b"),
            "`````txt\na ```` b\n`````\n"
        );
        assert_eq!(file_markdown("Makefile", "all:"), "```\nall:\n```\n");
    }

    #[test]
    fn diffs_render_per_source_with_their_files() {
        let preview = json!({"sources": [{"title": "Working tree", "diff": "--- a\n+++ b\n-x\n+y",
            "files": [{"path": "a.rs", "additions": 1, "deletions": 1}], "truncated": false}]});
        assert_eq!(
            diff_markdown(&preview),
            "## Working tree\n\n- `a.rs` +1 -1\n\n```diff\n--- a\n+++ b\n-x\n+y\n```\n\n"
        );
        assert_eq!(diff_markdown(&json!({"sources": []})), "No changes.");
    }
}
