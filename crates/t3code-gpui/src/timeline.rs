use serde_json::Value;
use t3_client::projection::ThreadState;

const BLOCK_BYTES: usize = 1200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    User,
    Assistant,
    Reasoning,
    Tool,
    Request,
    Error,
    Notice,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub key: String,
    pub kind: RowKind,
    pub text: String,
    pub live: bool,
}

pub fn split_blocks(text: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    let mut start = 0;
    let mut offset = 0;
    let mut in_fence = false;
    for line in text.split_inclusive('\n') {
        offset += line.len();
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        }
        let complete_blank_line = line.ends_with('\n') && line.trim().is_empty();
        if !in_fence && complete_blank_line && offset - start >= BLOCK_BYTES {
            blocks.push(&text[start..offset]);
            start = offset;
        }
    }
    if start < text.len() || blocks.is_empty() {
        blocks.push(&text[start..]);
    }
    blocks
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn first_line(value: &str, limit: usize) -> String {
    let line = value
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .trim();
    if line.chars().count() > limit {
        let cut: String = line.chars().take(limit).collect();
        format!("{cut}…")
    } else {
        line.to_string()
    }
}

fn tool_summary(item: &Value) -> String {
    let status = text(item, "status");
    let title = text(item, "title");
    let detail = match text(item, "type") {
        "command_execution" => {
            let input = item.get("input").unwrap_or(&Value::Null);
            let command = input
                .get("command")
                .and_then(Value::as_str)
                .or_else(|| input.as_str())
                .unwrap_or_default();
            format!("$ {}", first_line(command, 160))
        }
        "file_change" => {
            let name = text(item, "fileName");
            let additions = item.get("additions").and_then(Value::as_u64).unwrap_or(0);
            let deletions = item.get("deletions").and_then(Value::as_u64).unwrap_or(0);
            format!("{name} +{additions} -{deletions}")
        }
        "dynamic_tool" => {
            let name = item
                .get("toolName")
                .and_then(Value::as_str)
                .unwrap_or(title);
            name.to_string()
        }
        "web_search" | "file_search" => first_line(title, 160),
        other => {
            if title.is_empty() {
                other.to_string()
            } else {
                first_line(title, 160)
            }
        }
    };
    if status.is_empty() || status == "completed" {
        detail
    } else {
        format!("{detail} ({status})")
    }
}

const TOOL_TYPES: &[&str] = &[
    "command_execution",
    "file_change",
    "file_search",
    "web_search",
    "dynamic_tool",
    "todo_list",
    "subagent",
    "checkpoint",
    "compaction",
    "handoff",
];

pub fn rows(state: &ThreadState) -> Vec<Row> {
    let mut rows = Vec::new();
    for item in state.items() {
        let id = text(item, "id");
        let streaming = item
            .get("streaming")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        match text(item, "type") {
            "user_message" => rows.push(Row {
                key: id.to_string(),
                kind: RowKind::User,
                text: text(item, "text").to_string(),
                live: false,
            }),
            "assistant_message" | "proposed_plan" => {
                let body = match text(item, "type") {
                    "proposed_plan" => text(item, "markdown"),
                    _ => text(item, "text"),
                };
                let blocks = split_blocks(body);
                let last = blocks.len() - 1;
                for (i, block) in blocks.into_iter().enumerate() {
                    rows.push(Row {
                        key: format!("{id}#{i}"),
                        kind: RowKind::Assistant,
                        text: block.to_string(),
                        live: streaming && i == last,
                    });
                }
            }
            "reasoning" => rows.push(Row {
                key: id.to_string(),
                kind: RowKind::Reasoning,
                text: first_line(text(item, "text"), 200),
                live: streaming,
            }),
            "approval_request" | "user_input_request" | "secret_request" | "link_request" => {
                let pending = matches!(text(item, "status"), "pending" | "waiting" | "running");
                let label = match text(item, "type") {
                    "approval_request" => "Approval",
                    "user_input_request" => "Question",
                    _ => "Request",
                };
                let prompt = item
                    .get("prompt")
                    .and_then(Value::as_str)
                    .or_else(|| {
                        item.pointer("/questions/0/question")
                            .and_then(Value::as_str)
                    })
                    .unwrap_or(text(item, "title"));
                let state_label = if pending {
                    "waiting"
                } else {
                    text(item, "status")
                };
                rows.push(Row {
                    key: id.to_string(),
                    kind: RowKind::Request,
                    text: format!("{label} ({state_label}): {}", first_line(prompt, 200)),
                    live: pending,
                });
            }
            "error" => rows.push(Row {
                key: id.to_string(),
                kind: RowKind::Error,
                text: item
                    .pointer("/failure/message")
                    .and_then(Value::as_str)
                    .unwrap_or("error")
                    .to_string(),
                live: false,
            }),
            kind if TOOL_TYPES.contains(&kind) => rows.push(Row {
                key: id.to_string(),
                kind: RowKind::Tool,
                text: tool_summary(item),
                live: false,
            }),
            "system_notice" | "run_interrupt_request" | "run_interrupt_result" => rows.push(Row {
                key: id.to_string(),
                kind: RowKind::Notice,
                text: first_line(
                    item.get("message")
                        .and_then(Value::as_str)
                        .unwrap_or(text(item, "title")),
                    200,
                ),
                live: false,
            }),
            _ => {}
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state(items: Vec<Value>) -> ThreadState {
        ThreadState::from_snapshot(&json!({
            "kind": "snapshot",
            "snapshotSequence": 1,
            "projection": {"thread": {"id": "t1"}, "turnItems": items},
        }))
        .unwrap()
    }

    #[test]
    fn a_short_message_is_one_block() {
        assert_eq!(split_blocks("hello\n\nworld"), vec!["hello\n\nworld"]);
        assert_eq!(split_blocks(""), vec![""]);
    }

    #[test]
    fn blocks_are_stable_as_the_text_grows() {
        let paragraph = format!("{}\n\n", "a".repeat(700));
        let full = paragraph.repeat(6);
        let blocks = split_blocks(&full);
        assert!(blocks.len() > 1);
        assert_eq!(blocks.concat(), full);
        for cut in (0..full.len()).step_by(97) {
            let partial = split_blocks(&full[..cut]);
            for (i, block) in partial
                .iter()
                .enumerate()
                .take(partial.len().saturating_sub(1))
            {
                assert_eq!(*block, blocks[i], "block {i} changed at length {cut}");
            }
        }
    }

    #[test]
    fn a_fence_is_never_split() {
        let code = format!(
            "```rust\n{}\n\n{}\n```\n\nafter\n",
            "x".repeat(1500),
            "y".repeat(10)
        );
        let blocks = split_blocks(&code);
        assert!(blocks[0].ends_with("```\n\n"), "{:?}", blocks[0]);
        assert_eq!(blocks.concat(), code);
    }

    #[test]
    fn rows_follow_items_and_mark_the_streaming_tail() {
        let long = format!("{}\n\n{}\n\ntail", "a".repeat(1300), "b".repeat(10));
        let s = state(vec![
            json!({"id": "u", "ordinal": 1, "type": "user_message", "text": "質問"}),
            json!({"id": "r", "ordinal": 2, "type": "reasoning", "text": "\nthinking hard\nmore", "streaming": false}),
            json!({"id": "c", "ordinal": 3, "type": "command_execution", "status": "completed", "input": {"command": "ls -la\npwd"}}),
            json!({"id": "a", "ordinal": 4, "type": "assistant_message", "text": long, "streaming": true}),
            json!({"id": "x", "ordinal": 5, "type": "future_kind"}),
        ]);
        let rows = rows(&s);
        let kinds: Vec<&RowKind> = rows.iter().map(|r| &r.kind).collect();
        assert_eq!(
            kinds,
            [
                &RowKind::User,
                &RowKind::Reasoning,
                &RowKind::Tool,
                &RowKind::Assistant,
                &RowKind::Assistant
            ]
        );
        assert_eq!(rows[1].text, "thinking hard");
        assert_eq!(rows[2].text, "$ ls -la");
        assert_eq!(rows[3].key, "a#0");
        assert!(!rows[3].live);
        assert!(rows[4].live);
    }

    #[test]
    fn a_pending_approval_is_a_live_request_row() {
        let s = state(vec![json!({
            "id": "q", "ordinal": 1, "type": "approval_request", "status": "pending",
            "prompt": "Run rm -rf build?"
        })]);
        let rows = rows(&s);
        assert_eq!(rows[0].kind, RowKind::Request);
        assert!(rows[0].live);
        assert_eq!(rows[0].text, "Approval (waiting): Run rm -rf build?");
    }
}
