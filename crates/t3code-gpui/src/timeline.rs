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
    Subagent,
    Working,
    Worked,
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
pub struct AssistantMetadata {
    pub message_id: String,
    pub source_thread_id: String,
    pub run_id: Option<String>,
    pub updated_at: String,
    pub status: String,
    pub can_fork: bool,
    pub can_copy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessagePart {
    pub first: bool,
    pub last: bool,
    pub metadata: Option<AssistantMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub message_part: Option<MessagePart>,
    pub key: String,
    pub kind: RowKind,
    pub text: String,
    pub live: bool,
    pub pending: Option<Pending>,
    pub detail: String,
    pub link: Option<String>,
    pub children: Vec<Row>,
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
                .map(|item| first_line(&tool_detail(item), 160))
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

pub fn thought_preview(value: &str) -> String {
    let line = value
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .trim();
    let plain = markdown::to_mdast(line, &markdown::ParseOptions::gfm())
        .map(|node| node.to_string())
        .unwrap_or_else(|_| line.to_string());
    plain.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn present_activity(mut rows: Vec<Row>, state: &ThreadState) -> Vec<Row> {
    rows = fold_completed_turns(rows, state);
    let Some(run) = state.active_run() else {
        return compact_history(rows);
    };
    let tail = rows
        .iter()
        .rposition(|row| !matches!(row.kind, RowKind::Reasoning | RowKind::Tool))
        .map_or(0, |at| at + 1);
    let activity = rows.split_off(tail);
    rows = compact_history(rows);
    let tail = rows.len();
    rows.extend(activity);
    if rows.len().saturating_sub(tail) > 2 {
        let mut activity = rows.split_off(tail);
        let last_tool = activity
            .iter()
            .rposition(|row| row.kind == RowKind::Tool)
            .map(|at| activity.remove(at));
        if !activity.is_empty() {
            let latest = activity
                .iter()
                .rfind(|row| row.kind == RowKind::Reasoning)
                .unwrap_or_else(|| activity.last().unwrap());
            let mut group = latest.clone();
            group.key = activity[0].key.clone();
            group.children = activity;
            rows.push(group);
        }
        rows.extend(last_tool);
    }
    let boundary = rows
        .iter()
        .rposition(|row| row.kind == RowKind::User)
        .map_or(0, |at| at + 1);
    rows.insert(
        boundary.min(rows.len()),
        Row {
            message_part: None,
            key: format!("working:{}", text(run, "id")),
            kind: RowKind::Working,
            text: "Working".into(),
            live: true,
            pending: None,
            detail: run
                .get("startedAt")
                .and_then(Value::as_str)
                .unwrap_or(text(run, "createdAt"))
                .into(),
            link: None,
            children: Vec::new(),
        },
    );
    rows
}

fn fold_completed_turns(rows: Vec<Row>, state: &ThreadState) -> Vec<Row> {
    let items = state.items();
    let by_id: HashMap<_, _> = items.iter().map(|item| (text(item, "id"), *item)).collect();
    let terminal_by_run: HashMap<_, _> = items
        .iter()
        .filter(|item| text(item, "type") == "assistant_message")
        .map(|item| (text(item, "runId"), text(item, "id")))
        .collect();
    let mut by_run: HashMap<&str, Vec<(usize, &str)>> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        if !matches!(
            row.kind,
            RowKind::Assistant | RowKind::Reasoning | RowKind::Tool
        ) {
            continue;
        }
        let source = by_id.get(row.key.as_str()).copied().or_else(|| {
            row.key
                .rsplit_once('#')
                .and_then(|(key, _)| by_id.get(key).copied())
        });
        if let Some(item) = source {
            by_run
                .entry(text(item, "runId"))
                .or_default()
                .push((index, text(item, "id")));
        }
    }
    let mut folds = HashMap::new();
    for run in state
        .list("runs")
        .iter()
        .filter(|run| text(run, "status") == "completed")
    {
        let run_id = text(run, "id");
        let Some(terminal) = terminal_by_run.get(run_id) else {
            continue;
        };
        let candidates: Vec<usize> = by_run
            .get(run_id)
            .into_iter()
            .flatten()
            .filter(|(_, id)| id != terminal)
            .map(|(index, _)| *index)
            .collect();
        if candidates.is_empty() || candidates.iter().any(|index| rows[*index].live) {
            continue;
        }
        let elapsed = humantime::parse_rfc3339(text(run, "startedAt"))
            .ok()
            .zip(humantime::parse_rfc3339(text(run, "completedAt")).ok())
            .and_then(|(start, end)| end.duration_since(start).ok());
        let label = elapsed
            .map(|duration| format!("Worked for {}", format_duration(duration)))
            .unwrap_or_else(|| "Worked".into());
        folds.insert(
            candidates[0],
            (
                Row {
                    message_part: None,
                    key: format!("worked:{run_id}"),
                    kind: RowKind::Worked,
                    text: label,
                    live: false,
                    pending: None,
                    detail: String::new(),
                    link: None,
                    children: Vec::new(),
                },
                candidates,
            ),
        );
    }
    let mut slots: Vec<_> = rows.into_iter().map(Some).collect();
    for (fold, indices) in folds.values_mut() {
        fold.children = indices
            .iter()
            .map(|index| slots[*index].take().expect("one fold per item"))
            .collect();
    }
    let mut out = Vec::new();
    for (index, row) in slots.into_iter().enumerate() {
        if let Some((fold, _)) = folds.remove(&index) {
            out.push(fold);
        }
        if let Some(row) = row {
            out.push(row);
        }
    }
    out
}

/// T3's shared/orchestrationTiming.ts rounds before choosing the display unit.
pub fn format_duration(duration: std::time::Duration) -> String {
    let millis = duration.as_millis();
    if millis < 1_000 {
        return format!("{}ms", millis.max(1));
    }
    if millis < 10_000 {
        return format!("{}s", ((millis + 50) / 100) as f64 / 10.);
    }
    let seconds = ((millis + 500) / 1_000) as u64;
    let mut parts = Vec::new();
    for (value, unit) in [
        (seconds / 3600, "h"),
        (seconds / 60 % 60, "m"),
        (seconds % 60, "s"),
    ] {
        if value > 0 {
            parts.push(format!("{value}{unit}"));
        }
    }
    parts.join(" ")
}

fn compact_history(rows: Vec<Row>) -> Vec<Row> {
    fn append(out: &mut Vec<Row>, group: &mut Vec<Row>) {
        let children = std::mem::take(group);
        if children.len() < 2 {
            out.extend(children);
            return;
        }
        let tools: Vec<_> = children
            .iter()
            .filter(|row| row.kind == RowKind::Tool)
            .collect();
        let mut row = tools
            .first()
            .copied()
            .unwrap_or_else(|| children.last().unwrap())
            .clone();
        row.key = children[0].key.clone();
        if !tools.is_empty() {
            let commands = tools.iter().all(|row| {
                ["Ran ", "Running ", "Failed ", "Stopped ", "Declined "]
                    .iter()
                    .any(|verb| row.text.starts_with(verb))
            });
            row.text = if commands {
                format!(
                    "Ran {} {}",
                    tools.len(),
                    if tools.len() == 1 {
                        "command"
                    } else {
                        "commands"
                    }
                )
            } else {
                tools
                    .iter()
                    .take(2)
                    .map(|row| row.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" and ")
            };
        }
        row.children = children;
        out.push(row);
    }
    let mut out = Vec::new();
    let mut group = Vec::new();
    for row in rows {
        if matches!(row.kind, RowKind::Reasoning | RowKind::Tool) {
            group.push(row);
        } else {
            append(&mut out, &mut group);
            out.push(row);
        }
    }
    append(&mut out, &mut group);
    out
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
            let command = command.trim();
            let executable = command
                .split_whitespace()
                .next()
                .unwrap_or("command")
                .rsplit('/')
                .next()
                .unwrap_or("command");
            let executable = if matches!(executable, "sh" | "bash" | "zsh") {
                command
                    .split_once(" -lc ")
                    .or_else(|| command.split_once(" -c "))
                    .and_then(|(_, body)| {
                        body.trim_start_matches(['\'', '"'])
                            .split_whitespace()
                            .next()
                    })
                    .unwrap_or(executable)
            } else {
                executable
            };
            let verb = match status {
                "running" | "inProgress" | "pending" => "Running",
                "failed" => "Failed",
                "declined" => "Declined",
                "cancelled" | "interrupted" | "stopped" => "Stopped",
                _ => "Ran",
            };
            return format!("{verb} {executable}");
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
    if status.is_empty() || matches!(status, "completed" | "running" | "inProgress" | "pending") {
        detail
    } else {
        format!("{detail} ({status})")
    }
}

/// What a tool row shows when opened: the whole command and its output, the diff of a file
/// change, or the input and output of another tool.
fn tool_detail(item: &Value) -> String {
    let pretty = |v: &Value| match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    };
    let omitted = item.get("outputOmitted").and_then(Value::as_bool) == Some(true);
    let output = || {
        let output = pretty(item.get("output").unwrap_or(&Value::Null));
        if omitted && output.is_empty() {
            "(output left out; open the thread in the browser to see it)".to_string()
        } else {
            output
        }
    };
    match text(item, "type") {
        "command_execution" => {
            let input = item.get("input").unwrap_or(&Value::Null);
            let command = input
                .get("command")
                .map(pretty)
                .unwrap_or_else(|| pretty(input));
            let exit = item
                .get("exitCode")
                .and_then(Value::as_i64)
                .map(|code| format!("\n[exit {code}]"))
                .unwrap_or_default();
            format!("$ {command}\n{}{exit}", output())
        }
        "file_change" => {
            let diff = text(item, "diffStr");
            if diff.is_empty() {
                pretty(item.get("changes").unwrap_or(&Value::Null))
            } else {
                diff.to_string()
            }
        }
        _ => {
            let input = pretty(item.get("input").unwrap_or(&Value::Null));
            [input, output()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n")
        }
    }
}

/// A user message's text with a line naming each attached file.
fn user_text(item: &Value) -> String {
    let mut out = text(item, "text").to_string();
    for attachment in array(item, "attachments") {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!(
            "[{}] {}",
            text(attachment, "type"),
            text(attachment, "name")
        ));
    }
    out
}

fn subagent_summary(item: &Value) -> String {
    let status = text(item, "status");
    let prompt = first_line(text(item, "prompt"), 160);
    if status.is_empty() || status == "completed" {
        format!("Subagent: {prompt}")
    } else {
        format!("Subagent ({status}): {prompt}")
    }
}

const TOOL_TYPES: &[&str] = &[
    "command_execution",
    "file_change",
    "file_search",
    "web_search",
    "dynamic_tool",
    "todo_list",
    "compaction",
    "handoff",
];

fn assistant_metadata(state: &ThreadState) -> HashMap<String, AssistantMetadata> {
    let items = state.items();
    let mut terminal = HashMap::new();
    let mut user = 0;
    for item in &items {
        match text(item, "type") {
            "user_message" => user += 1,
            "assistant_message" => {
                let run = text(item, "runId");
                let response = if run.is_empty() {
                    format!("unkeyed:{user}")
                } else {
                    format!("run:{run}")
                };
                terminal.insert(response, *item);
            }
            _ => {}
        }
    }
    let active = state.active_run();
    let mut active_ids = std::collections::HashSet::new();
    if let Some(active) = active {
        active_ids.insert(text(active, "id"));
        let boundary = items
            .iter()
            .rposition(|item| text(item, "type") == "user_message")
            .map_or(0, |index| index + 1);
        for item in &items[boundary..] {
            if !text(item, "runId").is_empty() {
                active_ids.insert(text(item, "runId"));
            }
        }
    }
    terminal
        .into_values()
        .filter_map(|item| {
            let run_id = text(item, "runId");
            if item.get("streaming").and_then(Value::as_bool) == Some(true)
                || (!run_id.is_empty() && active_ids.contains(run_id))
                || (run_id.is_empty() && active.is_some())
            {
                return None;
            }
            let capabilities = state
                .run(run_id)
                .and_then(|run| run.get("providerSessionId"))
                .and_then(Value::as_str)
                .and_then(|id| {
                    state
                        .list("providerSessions")
                        .iter()
                        .find(|session| text(session, "id") == id)
                })
                .and_then(|session| session.get("capabilities"))
                .filter(|value| !value.is_null());
            let can_fork = !run_id.is_empty()
                && text(item, "status") == "completed"
                && capabilities.is_none_or(|caps| {
                    (caps["threads"]["canForkThread"] == true
                        && caps["threads"]["canForkFromTurn"] == true
                        && caps["identity"]["nativeThreadIds"] == "strong")
                        || caps["context"]["supportsFullThreadHandoff"] == true
                });
            let id = text(item, "id").to_string();
            Some((
                id.clone(),
                AssistantMetadata {
                    message_id: id,
                    source_thread_id: if text(item, "threadId").is_empty() {
                        state.thread_id().to_string()
                    } else {
                        text(item, "threadId").to_string()
                    },
                    run_id: (!run_id.is_empty()).then(|| run_id.to_string()),
                    updated_at: item
                        .get("updatedAt")
                        .and_then(Value::as_str)
                        .unwrap_or(text(item, "createdAt"))
                        .to_string(),
                    status: text(item, "status").to_string(),
                    can_fork,
                    can_copy: !text(item, "text").trim().is_empty(),
                },
            ))
        })
        .collect()
}

pub fn rows(state: &ThreadState) -> Vec<Row> {
    let metadata = assistant_metadata(state);
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
                message_part: None,
                key: id.to_string(),
                kind: RowKind::User,
                text: user_text(item),
                live: false,
                pending: None,
                detail: if text(item, "createdBy") == "agent" {
                    "agent".into()
                } else {
                    String::new()
                },
                children: Vec::new(),
                link: None,
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
                        message_part: Some(MessagePart {
                            first: i == 0,
                            last: i == last,
                            metadata: (i == last).then(|| metadata.get(id).cloned()).flatten(),
                        }),
                        key: format!("{id}#{i}"),
                        kind: RowKind::Assistant,
                        text: block.to_string(),
                        live: streaming && i == last,
                        pending: None,
                        detail: String::new(),
                        children: Vec::new(),
                        link: None,
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
            "reasoning" if !text(item, "text").trim().is_empty() => rows.push(Row {
                message_part: None,
                key: id.to_string(),
                kind: RowKind::Reasoning,
                text: first_line(text(item, "text"), 200),
                live: streaming,
                pending: None,
                detail: text(item, "text").to_string(),
                children: Vec::new(),
                link: None,
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
                    message_part: None,
                    key: id.to_string(),
                    kind: RowKind::Request,
                    text,
                    live: pending,
                    pending: card,
                    detail: String::new(),
                    children: Vec::new(),
                    link: None,
                });
            }
            "error" => rows.push(Row {
                message_part: None,
                key: id.to_string(),
                kind: RowKind::Error,
                text: item
                    .pointer("/failure/message")
                    .and_then(Value::as_str)
                    .unwrap_or("error")
                    .to_string(),
                live: false,
                pending: None,
                detail: String::new(),
                children: Vec::new(),
                link: None,
            }),
            "subagent" => rows.push(Row {
                message_part: None,
                key: id.to_string(),
                kind: RowKind::Subagent,
                text: subagent_summary(item),
                live: matches!(text(item, "status"), "running" | "pending" | "waiting"),
                pending: None,
                detail: [
                    text(item, "prompt"),
                    text(item, "progress"),
                    text(item, "result"),
                ]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n"),
                children: Vec::new(),
                link: item
                    .get("childThreadId")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            }),
            kind if TOOL_TYPES.contains(&kind) => rows.push(Row {
                message_part: None,
                key: id.to_string(),
                kind: RowKind::Tool,
                text: tool_summary(item),
                live: matches!(text(item, "status"), "running" | "inProgress" | "pending"),
                pending: None,
                detail: tool_detail(item),
                children: Vec::new(),
                link: None,
            }),
            "system_notice" | "run_interrupt_result" => rows.push(Row {
                message_part: None,
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
                detail: text(item, "type").to_string(),
                children: Vec::new(),
                link: None,
            }),
            _ => {}
        }
    }
    if let Some((at, plan_id, markdown)) = open_plan {
        rows.insert(
            at,
            Row {
                message_part: None,
                key: format!("plan:{plan_id}"),
                kind: RowKind::Plan,
                text: "Plan".into(),
                live: true,
                pending: Some(Pending::Plan { plan_id, markdown }),
                detail: String::new(),
                children: Vec::new(),
                link: None,
            },
        );
    }
    present_activity(rows, state)
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    pub run_id: String,
    pub text: String,
}

/// The runs waiting in the thread's queue, in queue order, with the text of their messages.
pub fn queued(state: &ThreadState) -> Vec<Queued> {
    let mut runs: Vec<&Value> = state
        .list("runs")
        .iter()
        .filter(|r| text(r, "status") == "queued")
        .collect();
    runs.sort_by_key(|r| {
        r.get("queuePosition")
            .and_then(Value::as_i64)
            .unwrap_or(i64::MAX)
    });
    runs.into_iter()
        .map(|run| {
            let message_id = text(run, "userMessageId");
            let body = state
                .list("messages")
                .iter()
                .find(|m| text(m, "id") == message_id)
                .map(|m| text(m, "text").to_string())
                .unwrap_or_default();
            Queued {
                run_id: text(run, "id").to_string(),
                text: body,
            }
        })
        .collect()
}

/// The rows whose text contains the query, ignoring case. A blank query finds nothing.
pub fn find_rows(rows: &[Row], query: &str) -> Vec<usize> {
    fn contains(row: &Row, query: &str) -> bool {
        row.text.to_lowercase().contains(query)
            || row.detail.to_lowercase().contains(query)
            || row.children.iter().any(|row| contains(row, query))
    }
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    rows.iter()
        .enumerate()
        .filter(|(_, row)| contains(row, &query))
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn work_duration_matches_t3_rounding_and_units() {
        for (millis, expected) in [
            (0, "1ms"),
            (410, "410ms"),
            (1250, "1.3s"),
            (9990, "10s"),
            (30550, "31s"),
            (59500, "1m"),
            (79500, "1m 20s"),
            (86400000, "24h"),
        ] {
            assert_eq!(
                super::format_duration(std::time::Duration::from_millis(millis)),
                expected
            );
        }
    }

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
    fn thought_labels_flatten_markdown_without_damaging_code_or_link_text() {
        assert_eq!(
            thought_preview("**Researching current Lean formatter**\nMore"),
            "Researching current Lean formatter"
        );
        assert_eq!(
            thought_preview("Checking `document_formatting` and [Lean](https://lean-lang.org)"),
            "Checking document_formatting and Lean"
        );
    }

    #[test]
    fn completed_turns_fold_intermediate_work_and_keep_the_whole_final_answer() {
        let mut state = state(vec![
            json!({"id":"u", "runId":"run", "ordinal":1, "type":"user_message", "createdBy":"agent", "text":"format?"}),
            json!({"id":"r", "runId":"run", "ordinal":2, "type":"reasoning", "text":"**Researching formatter**"}),
            json!({"id":"a", "runId":"run", "ordinal":3, "type":"assistant_message", "text":"Checking support."}),
            json!({"id":"c", "runId":"run", "ordinal":4, "type":"command_execution", "input":"git status --short", "status":"completed", "output":"clean"}),
            json!({"id":"f", "runId":"run", "ordinal":5, "type":"assistant_message", "text":format!("{}\n\nConclusion", "A".repeat(1300))}),
        ]);
        state.projection.insert("runs".into(), json!([{"id":"run", "ordinal":1, "status":"completed", "startedAt":"2026-11-14T00:00:00Z", "completedAt":"2026-11-14T00:01:19Z"}]));
        let result = rows(&state);
        assert_eq!(
            result.iter().map(|row| &row.kind).collect::<Vec<_>>(),
            [
                &RowKind::User,
                &RowKind::Worked,
                &RowKind::Assistant,
                &RowKind::Assistant
            ]
        );
        assert_eq!(result[0].detail, "agent");
        assert_eq!(result[1].text, "Worked for 1m 19s");
        assert_eq!(result[1].children.len(), 3);
        assert_eq!(find_rows(&result, "git status"), [1]);
        assert_eq!(result[2].key, "f#0");
        assert_eq!(result[3].text, "Conclusion");
        state.projection.get_mut("runs").unwrap()[0]["status"] = json!("failed");
        assert!(!rows(&state).iter().any(|row| row.kind == RowKind::Worked));
    }

    #[test]
    fn active_work_shows_the_latest_thought_and_tool_without_losing_history() {
        let mut state = state(vec![
            json!({"id":"u", "ordinal":1, "type":"user_message", "text":"format?"}),
            json!({"id":"r1", "ordinal":2, "type":"reasoning", "text":"**Researching current Lean formatter**\nMore detail"}),
            json!({"id":"a", "ordinal":3, "type":"assistant_message", "text":"Checking."}),
            json!({"id":"r2", "ordinal":4, "type":"reasoning", "text":"**Checking server capability access**"}),
            json!({"id":"c1", "ordinal":5, "type":"command_execution", "input":"emacsclient --eval long", "status":"failed", "output":"error"}),
            json!({"id":"r3", "ordinal":6, "type":"reasoning", "text":"**Saving the research document**"}),
            json!({"id":"c2", "ordinal":7, "type":"command_execution", "input":"/run/current-system/sw/bin/zsh -lc 'git add design.md'", "status":"running"}),
        ]);
        state.projection.insert("runs".into(), json!([{"id":"run", "ordinal":1, "status":"running", "createdAt":"2026-11-14T00:00:00Z"}]));
        let rows = rows(&state);
        assert_eq!(
            rows.iter().map(|row| &row.kind).collect::<Vec<_>>(),
            [
                &RowKind::User,
                &RowKind::Working,
                &RowKind::Reasoning,
                &RowKind::Assistant,
                &RowKind::Reasoning,
                &RowKind::Tool,
            ]
        );
        assert_eq!(
            thought_preview(&rows[2].text),
            "Researching current Lean formatter"
        );
        assert_eq!(
            thought_preview(&rows[4].text),
            "Saving the research document"
        );
        assert_eq!(rows[4].children.len(), 3);
        assert!(rows[4].children[1].detail.contains("error"));
        assert_eq!(rows[5].text, "Running git");
        assert_eq!(find_rows(&rows, "emacsclient"), [4]);
    }

    #[test]
    fn only_the_completed_terminal_response_exposes_message_actions() {
        let mut projection = state(vec![
            json!({"id":"u1","type":"user_message"}),
            json!({"id":"a1","type":"assistant_message","runId":"r1","status":"completed","text":"intermediate"}),
            json!({"id":"a2","type":"assistant_message","runId":"r1","status":"completed","text":"final"}),
            json!({"id":"u2","type":"user_message"}),
            json!({"id":"a3","type":"assistant_message","runId":"r2","status":"completed","text":"working response"}),
        ]);
        projection.projection.insert(
            "runs".into(),
            json!([{"id":"r1","status":"completed"},{"id":"r2","status":"running"}]),
        );
        let metadata = assistant_metadata(&projection);
        assert_eq!(metadata.keys().collect::<Vec<_>>(), vec!["a2"]);
        assert!(metadata["a2"].can_fork);
        projection.projection["runs"][1]["status"] = json!("completed");
        let metadata = assistant_metadata(&projection);
        assert_eq!(metadata.len(), 2);
        assert!(metadata.contains_key("a3"));
    }

    #[test]
    fn known_provider_capabilities_gate_fork_without_hiding_copy() {
        let mut projection = state(vec![
            json!({"id":"a","type":"assistant_message","runId":"r","status":"completed","text":"reply"}),
        ]);
        projection.projection.insert(
            "runs".into(),
            json!([{"id":"r","status":"completed","providerSessionId":"p"}]),
        );
        projection.projection.insert("providerSessions".into(), json!([{"id":"p","capabilities":{"threads":{"canForkThread":false,"canForkFromTurn":false},"identity":{"nativeThreadIds":"weak"},"context":{"supportsFullThreadHandoff":false}}}]));
        let metadata = assistant_metadata(&projection);
        assert!(!metadata["a"].can_fork);
        assert!(metadata["a"].can_copy);
        projection.projection["providerSessions"][0]["capabilities"]["context"]["supportsFullThreadHandoff"] =
            json!(true);
        assert!(assistant_metadata(&projection)["a"].can_fork);
    }

    #[test]
    fn whitespace_tail_chunk_keeps_the_full_message_copy_action() {
        let text = format!("{}\n\n  ", "x".repeat(BLOCK_BYTES));
        let projection = state(vec![
            json!({"id":"a","type":"assistant_message","status":"completed","text":text}),
        ]);
        let message = rows(&projection);
        let last = message.last().unwrap();
        assert!(last.text.trim().is_empty());
        assert!(
            last.message_part
                .as_ref()
                .unwrap()
                .metadata
                .as_ref()
                .unwrap()
                .can_copy
        );
    }

    #[test]
    fn message_boundaries_survive_virtualization_chunks() {
        for body in ["short".to_string(), "paragraph\n\n".repeat(500)] {
            let state = state(vec![
                json!({"id": "m1", "type": "assistant_message", "text": body}),
            ]);
            let message = rows(&state);
            assert_eq!(
                message
                    .iter()
                    .map(|row| row.text.as_str())
                    .collect::<String>(),
                body
            );
            for (index, row) in message.iter().enumerate() {
                assert_eq!(
                    row.message_part,
                    Some(MessagePart {
                        first: index == 0,
                        last: index + 1 == message.len(),
                        metadata: if index + 1 == message.len() {
                            Some(AssistantMetadata {
                                message_id: "m1".into(),
                                source_thread_id: "t1".into(),
                                run_id: None,
                                updated_at: String::new(),
                                status: String::new(),
                                can_fork: false,
                                can_copy: true,
                            })
                        } else {
                            None
                        },
                    })
                );
            }
        }
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
    fn an_interruption_has_one_visible_result_and_checkpoint_stays_out_of_chat() {
        let s = state(vec![
            json!({"id": "u", "ordinal": 1, "type": "user_message", "text": "hello"}),
            json!({"id": "r", "ordinal": 2, "type": "run_interrupt_request", "message": "Stopped by user"}),
            json!({"id": "c", "ordinal": 3, "type": "checkpoint", "status": "completed"}),
            json!({"id": "a", "ordinal": 4, "type": "run_interrupt_result", "message": "Stopped by user"}),
            json!({"id": "n", "ordinal": 5, "type": "system_notice", "message": "Notice"}),
        ]);
        let rows = rows(&s);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].text, "Stopped by user");
        assert_eq!(rows[1].detail, "run_interrupt_result");
        assert_eq!(rows[2].text, "Notice");
        assert_eq!(rows[2].detail, "system_notice");
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
                &RowKind::Tool,
                &RowKind::Assistant,
                &RowKind::Assistant
            ]
        );
        assert_eq!(rows[1].text, "Ran 1 command");
        assert_eq!(rows[1].children[0].text, "thinking hard");
        assert_eq!(rows[1].children[1].text, "Ran ls");
        assert_eq!(rows[2].key, "a#0");
        assert!(!rows[2].live);
        assert!(rows[3].live);
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
        assert_eq!(card("Allow Bash?"), "$ touch approved.txt");
    }

    #[test]
    fn user_message_jumps_step_over_the_rows_between() {
        let row = |kind: RowKind| Row {
            message_part: None,
            key: String::new(),
            kind,
            text: String::new(),
            live: false,
            pending: None,
            detail: String::new(),
            children: Vec::new(),
            link: None,
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
            message_part: None,
            key: String::new(),
            kind: RowKind::Assistant,
            text: text.into(),
            live: false,
            pending: None,
            detail: String::new(),
            children: Vec::new(),
            link: None,
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

    #[test]
    fn opened_tool_rows_show_the_command_output_and_diffs() {
        let command = json!({"type": "command_execution", "input": {"command": "ls"}, "output": "a\nb", "exitCode": 0});
        assert_eq!(tool_detail(&command), "$ ls\na\nb\n[exit 0]");
        let omitted = json!({"type": "command_execution", "input": {"command": "cat big"}, "outputOmitted": true});
        assert!(tool_detail(&omitted).contains("left out"));
        let change =
            json!({"type": "file_change", "fileName": "a.rs", "diffStr": "@@ -1 +1 @@\n-a\n+b"});
        assert_eq!(tool_detail(&change), "@@ -1 +1 @@\n-a\n+b");
        let tool = json!({"type": "dynamic_tool", "input": {"q": 1}, "output": "done"});
        assert_eq!(tool_detail(&tool), "{\n  \"q\": 1\n}\n\ndone");
    }

    #[test]
    fn queued_runs_come_in_queue_order_with_their_text() {
        let s = ThreadState::from_snapshot(&json!({
            "kind": "snapshot",
            "snapshotSequence": 1,
            "projection": {
                "thread": {"id": "t1"},
                "turnItems": [],
                "runs": [
                    {"id": "r2", "status": "queued", "queuePosition": 2, "userMessageId": "m2"},
                    {"id": "r0", "status": "running", "userMessageId": "m0"},
                    {"id": "r1", "status": "queued", "queuePosition": 1, "userMessageId": "m1"},
                ],
                "messages": [{"id": "m1", "text": "first"}, {"id": "m2", "text": "second"}],
            },
        }))
        .unwrap();
        let ids: Vec<(String, String)> =
            queued(&s).into_iter().map(|q| (q.run_id, q.text)).collect();
        assert_eq!(
            ids,
            [
                ("r1".into(), "first".into()),
                ("r2".into(), "second".into())
            ]
        );
    }

    #[test]
    fn user_rows_name_their_attachments() {
        let item = json!({"type": "user_message", "text": "look",
                          "attachments": [{"type": "image", "name": "shot.png"}, {"type": "file", "name": "a.md"}]});
        assert_eq!(user_text(&item), "look\n[image] shot.png\n[file] a.md");
        assert_eq!(
            user_text(&json!({"type": "user_message", "text": "plain"})),
            "plain"
        );
    }

    #[test]
    fn subagent_rows_link_their_child_thread() {
        let s = state(vec![
            json!({"id": "s", "ordinal": 1, "type": "subagent", "status": "running",
            "prompt": "Review the diff\nmore", "progress": "half way", "result": null, "childThreadId": "child-1"}),
        ]);
        let row = rows(&s).remove(0);
        assert_eq!(row.kind, RowKind::Subagent);
        assert_eq!(row.text, "Subagent (running): Review the diff");
        assert_eq!(row.link.as_deref(), Some("child-1"));
        assert_eq!(row.detail, "Review the diff\nmore\n\nhalf way");
    }
}
