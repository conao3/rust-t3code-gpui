use std::collections::HashMap;

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
    Plan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalOption {
    pub decision: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionOption {
    pub label: String,
    pub value: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
    pub allow_custom: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pending {
    Approval {
        request_id: String,
        subject: String,
        options: Vec<ApprovalOption>,
    },
    Question {
        request_id: String,
        questions: Vec<Question>,
    },
    Plan {
        plan_id: String,
        markdown: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub key: String,
    pub kind: RowKind,
    pub text: String,
    pub live: bool,
    pub pending: Option<Pending>,
}

const DEFAULT_APPROVAL_OPTIONS: &[(&str, &str)] = &[
    ("cancel", "Cancel"),
    ("decline", "Decline"),
    ("acceptForSession", "Always allow this session"),
    ("accept", "Approve"),
];

fn approval_options(item: &Value) -> Vec<ApprovalOption> {
    match item.get("options").and_then(Value::as_array) {
        Some(options) if !options.is_empty() => options
            .iter()
            .map(|o| ApprovalOption {
                decision: text(o, "decision").to_string(),
                label: text(o, "label").to_string(),
            })
            .collect(),
        _ => DEFAULT_APPROVAL_OPTIONS
            .iter()
            .map(|(decision, label)| ApprovalOption {
                decision: decision.to_string(),
                label: label.to_string(),
            })
            .collect(),
    }
}

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn questions(item: &Value) -> Vec<Question> {
    array(item, "questions")
        .iter()
        .map(|q| Question {
            id: text(q, "id").to_string(),
            header: text(q, "header").to_string(),
            question: text(q, "question").to_string(),
            options: array(q, "options")
                .iter()
                .map(|o| QuestionOption {
                    label: text(o, "label").to_string(),
                    value: o
                        .get("value")
                        .and_then(Value::as_str)
                        .unwrap_or(text(o, "label"))
                        .to_string(),
                    description: text(o, "description").to_string(),
                })
                .collect(),
            multi_select: q.get("multiSelect").and_then(Value::as_bool) == Some(true),
            allow_custom: q.get("allowCustomAnswer").and_then(Value::as_bool) != Some(false),
        })
        .collect()
}

fn pending_of(state: &ThreadState, item: &Value, prompt: &str) -> Option<Pending> {
    let request_id = text(item, "requestId");
    let pending = state
        .pending_requests()
        .iter()
        .any(|r| text(r, "id") == request_id);
    if request_id.is_empty() || !pending {
        return None;
    }
    match text(item, "type") {
        "approval_request" => Some(Pending::Approval {
            request_id: request_id.to_string(),
            subject: state
                .request_subject(request_id)
                .map(tool_summary)
                .filter(|subject| prompt.is_empty() || !subject.contains(prompt))
                .unwrap_or_default(),
            options: approval_options(item),
        }),
        "user_input_request" => Some(Pending::Question {
            request_id: request_id.to_string(),
            questions: questions(item),
        }),
        _ => None,
    }
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
    let mut open_plan: Option<(usize, String, String)> = None;
    for item in state.items() {
        let id = text(item, "id");
        let streaming = item
            .get("streaming")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if text(item, "type") == "user_message" {
            open_plan = None;
        }
        match text(item, "type") {
            "user_message" => rows.push(Row {
                key: id.to_string(),
                kind: RowKind::User,
                text: text(item, "text").to_string(),
                live: false,
                pending: None,
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
                        pending: None,
                    });
                }
                if text(item, "type") == "proposed_plan" && !streaming {
                    open_plan = Some((
                        rows.len(),
                        text(item, "planId").to_string(),
                        body.to_string(),
                    ));
                }
            }
            "reasoning" => rows.push(Row {
                key: id.to_string(),
                kind: RowKind::Reasoning,
                text: first_line(text(item, "text"), 200),
                live: streaming,
                pending: None,
            }),
            "approval_request" | "user_input_request" | "secret_request" | "link_request" => {
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
                let prompt = first_line(prompt, 200);
                let card = pending_of(state, item, &prompt);
                let pending = card.is_some();
                let state_label = if pending {
                    "waiting"
                } else {
                    text(item, "status")
                };
                let text = match &card {
                    Some(Pending::Question { .. }) => format!("{label} ({state_label})"),
                    _ => format!("{label} ({state_label}): {prompt}"),
                };
                rows.push(Row {
                    key: id.to_string(),
                    kind: RowKind::Request,
                    text,
                    live: pending,
                    pending: card,
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
                pending: None,
            }),
            kind if TOOL_TYPES.contains(&kind) => rows.push(Row {
                key: id.to_string(),
                kind: RowKind::Tool,
                text: tool_summary(item),
                live: false,
                pending: None,
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
                pending: None,
            }),
            _ => {}
        }
    }
    if let Some((at, plan_id, markdown)) = open_plan {
        rows.insert(
            at,
            Row {
                key: format!("plan:{plan_id}"),
                kind: RowKind::Plan,
                text: "Plan".into(),
                live: true,
                pending: Some(Pending::Plan { plan_id, markdown }),
            },
        );
    }
    rows
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
    pub selected: Vec<String>,
    pub custom: String,
}

impl Draft {
    pub fn toggle(&mut self, question: &Question, value: &str) {
        if question.multi_select {
            match self.selected.iter().position(|v| v == value) {
                Some(i) => {
                    self.selected.remove(i);
                }
                None => self.selected.push(value.to_string()),
            }
        } else {
            self.selected = vec![value.to_string()];
        }
    }
}

pub fn answer(question: &Question, draft: Option<&Draft>) -> Option<Value> {
    let custom = draft
        .map(|d| d.custom.trim())
        .filter(|c| question.allow_custom && !c.is_empty());
    if let Some(custom) = custom {
        return Some(Value::from(custom));
    }
    let selected: Vec<&String> = draft
        .map(|d| d.selected.iter())
        .into_iter()
        .flatten()
        .filter(|v| question.options.iter().any(|o| &o.value == *v))
        .collect();
    if question.multi_select {
        (!selected.is_empty())
            .then(|| Value::from(selected.into_iter().cloned().collect::<Vec<_>>()))
    } else {
        selected.first().map(|v| Value::from(v.as_str()))
    }
}

pub fn answers(questions: &[Question], drafts: &HashMap<String, Draft>) -> Option<Value> {
    let mut map = serde_json::Map::new();
    for question in questions {
        map.insert(
            question.id.clone(),
            answer(question, drafts.get(&question.id))?,
        );
    }
    Some(Value::Object(map))
}

/// The answers with typed text as the custom answer of the first question that takes one and
/// has no answer yet, or of the first question that takes one when every question has one.
pub fn answers_with_text(
    questions: &[Question],
    drafts: &HashMap<String, Draft>,
    typed: &str,
) -> Option<Value> {
    let typed = typed.trim();
    if typed.is_empty() {
        return answers(questions, drafts);
    }
    let target = questions
        .iter()
        .find(|q| q.allow_custom && answer(q, drafts.get(&q.id)).is_none())
        .or_else(|| questions.iter().find(|q| q.allow_custom))?;
    let mut drafts = drafts.clone();
    drafts.entry(target.id.clone()).or_default().custom = typed.to_string();
    answers(questions, &drafts)
}

/// The user message to jump to from row `from`, the row at the top of the view. `inside` means
/// the view starts partway into that row, so jumping back lands on its start.
pub fn user_message_jump(rows: &[Row], from: usize, inside: bool, forward: bool) -> Option<usize> {
    let mut users = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.kind == RowKind::User)
        .map(|(i, _)| i);
    if forward {
        users.find(|i| *i > from)
    } else {
        users.rev().find(|i| *i < from || (*i == from && inside))
    }
}

/// The rows whose text contains the query, ignoring case. A blank query finds nothing.
pub fn find_rows(rows: &[Row], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    rows.iter()
        .enumerate()
        .filter(|(_, row)| row.text.to_lowercase().contains(&query))
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state(items: Vec<Value>) -> ThreadState {
        state_with(items, vec![])
    }

    fn state_with(items: Vec<Value>, requests: Vec<Value>) -> ThreadState {
        ThreadState::from_snapshot(&json!({
            "kind": "snapshot",
            "snapshotSequence": 1,
            "projection": {"thread": {"id": "t1"}, "turnItems": items, "runtimeRequests": requests},
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
    fn a_pending_approval_is_a_card_with_the_default_options() {
        let s = state_with(
            vec![
                json!({"id": "a", "ordinal": 1, "type": "approval_request", "requestId": "r1",
                       "prompt": "Run rm -rf build?"}),
                json!({"id": "b", "ordinal": 2, "type": "approval_request", "requestId": "r0",
                       "prompt": "old"}),
            ],
            vec![
                json!({"id": "r1", "status": "pending"}),
                json!({"id": "r0", "status": "resolved"}),
            ],
        );
        let rows = rows(&s);
        assert!(rows[0].live);
        assert_eq!(rows[0].text, "Approval (waiting): Run rm -rf build?");
        let Some(Pending::Approval { options, .. }) = &rows[0].pending else {
            panic!("{:?}", rows[0].pending);
        };
        let decisions: Vec<&str> = options.iter().map(|o| o.decision.as_str()).collect();
        assert_eq!(
            decisions,
            ["cancel", "decline", "acceptForSession", "accept"]
        );
        assert!(rows[1].pending.is_none());
        assert!(!rows[1].live);
    }

    fn question_card() -> Vec<Question> {
        let s = state_with(
            vec![
                json!({"id": "q", "ordinal": 1, "type": "user_input_request", "requestId": "r1",
                "questions": [
                    {"id": "one", "header": "Pick", "question": "Which?",
                     "options": [{"label": "A", "description": ""}, {"label": "B", "value": "b-value", "description": ""}]},
                    {"id": "many", "header": "Many", "question": "Which ones?", "multiSelect": true,
                     "allowCustomAnswer": false,
                     "options": [{"label": "X", "description": ""}, {"label": "Y", "description": ""}]},
                ]}),
            ],
            vec![json!({"id": "r1", "status": "pending"})],
        );
        let row = rows(&s).remove(0);
        assert_eq!(row.text, "Question (waiting)");
        match row.pending {
            Some(Pending::Question { questions, .. }) => questions,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn answers_wait_for_every_question_and_take_the_option_value() {
        let questions = question_card();
        let mut drafts: HashMap<String, Draft> = HashMap::new();
        drafts
            .entry("one".into())
            .or_default()
            .toggle(&questions[0], "A");
        drafts
            .entry("one".into())
            .or_default()
            .toggle(&questions[0], "b-value");
        assert_eq!(answers(&questions, &drafts), None);
        let many = drafts.entry("many".into()).or_default();
        many.toggle(&questions[1], "X");
        many.toggle(&questions[1], "Y");
        many.toggle(&questions[1], "X");
        many.custom = "ignored".into();
        assert_eq!(
            answers(&questions, &drafts),
            Some(json!({"one": "b-value", "many": ["Y"]}))
        );
        drafts.get_mut("one").unwrap().custom = "  typed  ".into();
        assert_eq!(
            answers(&questions, &drafts),
            Some(json!({"one": "typed", "many": ["Y"]}))
        );
    }

    #[test]
    fn typed_text_answers_the_first_open_question_that_takes_text() {
        let questions = question_card();
        let mut drafts: HashMap<String, Draft> = HashMap::new();
        assert_eq!(answers_with_text(&questions, &drafts, "  "), None);
        drafts
            .entry("many".into())
            .or_default()
            .toggle(&questions[1], "X");
        assert_eq!(
            answers_with_text(&questions, &drafts, "free text"),
            Some(json!({"one": "free text", "many": ["X"]}))
        );
        drafts
            .entry("one".into())
            .or_default()
            .toggle(&questions[0], "A");
        assert_eq!(
            answers_with_text(&questions, &drafts, "override"),
            Some(json!({"one": "override", "many": ["X"]}))
        );
    }

    #[test]
    fn an_approval_names_the_gated_command_only_when_the_prompt_does_not() {
        let card = |prompt: &str| {
            let s = state_with(
                vec![
                    json!({"id": "c", "ordinal": 1, "type": "command_execution", "nodeId": "n-tool",
                           "status": "running", "input": {"command": "touch approved.txt"}}),
                    json!({"id": "a", "ordinal": 2, "type": "approval_request", "requestId": "r1",
                           "prompt": prompt}),
                ],
                vec![json!({"id": "r1", "status": "pending", "nodeId": "n-request"})],
            );
            let mut s = s;
            s.apply(&json!({"kind": "event", "sequence": 2, "event": {
                "type": "node.updated", "payload": {"id": "n-request", "parentNodeId": "n-tool"}}}));
            match rows(&s).remove(1).pending {
                Some(Pending::Approval { subject, .. }) => subject,
                other => panic!("{other:?}"),
            }
        };
        assert_eq!(card("touch approved.txt"), "");
        assert_eq!(card("Allow Bash?"), "$ touch approved.txt (running)");
    }

    #[test]
    fn user_message_jumps_step_over_the_rows_between() {
        let row = |kind: RowKind| Row {
            key: String::new(),
            kind,
            text: String::new(),
            live: false,
            pending: None,
        };
        let rows = [
            row(RowKind::User),
            row(RowKind::Assistant),
            row(RowKind::User),
            row(RowKind::Tool),
        ];
        assert_eq!(user_message_jump(&rows, 4, false, false), Some(2));
        assert_eq!(user_message_jump(&rows, 2, false, false), Some(0));
        assert_eq!(user_message_jump(&rows, 2, true, false), Some(2));
        assert_eq!(user_message_jump(&rows, 0, false, false), None);
        assert_eq!(user_message_jump(&rows, 0, false, true), Some(2));
        assert_eq!(user_message_jump(&rows, 2, false, true), None);
    }

    #[test]
    fn the_latest_plan_offers_implementation_until_the_user_answers() {
        let plan = json!({"id": "p", "ordinal": 2, "type": "proposed_plan", "planId": "plan-1",
                          "markdown": "1. Do it", "streaming": false});
        let open = state(vec![
            json!({"id": "u", "ordinal": 1, "type": "user_message", "text": "plan it"}),
            plan.clone(),
        ]);
        let list = rows(&open);
        assert_eq!(list.last().unwrap().kind, RowKind::Plan);
        assert_eq!(
            list.last().unwrap().pending,
            Some(Pending::Plan {
                plan_id: "plan-1".into(),
                markdown: "1. Do it".into()
            })
        );
        let answered = state(vec![
            plan,
            json!({"id": "u2", "ordinal": 3, "type": "user_message", "text": "go"}),
        ]);
        assert!(rows(&answered).iter().all(|r| r.kind != RowKind::Plan));
    }

    #[test]
    fn find_matches_rows_ignoring_case() {
        let row = |text: &str| Row {
            key: String::new(),
            kind: RowKind::Assistant,
            text: text.into(),
            live: false,
            pending: None,
        };
        let list = [
            row("Hello World"),
            row("nothing"),
            row("say HELLO"),
            row("日本語のテキスト"),
        ];
        assert_eq!(find_rows(&list, "hello"), [0, 2]);
        assert_eq!(find_rows(&list, "テキスト"), [3]);
        assert!(find_rows(&list, "  ").is_empty());
    }
}
