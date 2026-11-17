use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use t3_client::projection::ThreadState;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Fork,
    Subagent,
    Transfer,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Relation {
    pub thread_id: String,
    pub source_thread_id: String,
    pub target_thread_id: String,
    pub kind: Kind,
    pub edge_status: Option<String>,
    pub status: Option<String>,
    pub parent: bool,
    pub title: String,
    pub missing: bool,
    pub side_chat: bool,
    pub created_at: Option<String>,
}

#[derive(Clone, Copy)]
struct Edge<'a> {
    source: &'a str,
    target: &'a str,
    kind: Kind,
    status: Option<&'a str>,
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

pub fn parent_id(thread: &Value) -> Option<&str> {
    if thread.pointer("/forkedFrom/type").and_then(Value::as_str) == Some("run") {
        thread
            .pointer("/forkedFrom/threadId")
            .and_then(Value::as_str)
    } else {
        thread
            .pointer("/lineage/parentThreadId")
            .and_then(Value::as_str)
    }
}

pub fn merge_target(thread: &Value) -> Option<&str> {
    (thread
        .pointer("/lineage/relationshipToParent")
        .and_then(Value::as_str)
        == Some("fork"))
    .then(|| parent_id(thread))
    .flatten()
}

fn status(thread: &Value) -> Option<&str> {
    text(thread, "activityRunStatus").or_else(|| text(thread, "status"))
}

fn active(status: &str) -> bool {
    matches!(status, "pending" | "running" | "waiting")
}

fn add_edge<'a>(
    nodes: &mut HashMap<&'a str, Option<&'a Value>>,
    edges: &mut Vec<Edge<'a>>,
    indexes: &mut HashMap<(&'a str, &'a str, Kind), usize>,
    mut edge: Edge<'a>,
) {
    nodes.entry(edge.source).or_insert(None);
    nodes.entry(edge.target).or_insert(None);
    if edge.kind == Kind::Subagent && edge.status == Some("rolled_back") {
        edge.status = Some("interrupted");
    }
    let key = (edge.source, edge.target, edge.kind);
    if let Some(index) = indexes.get(&key) {
        edges[*index] = edge;
    } else {
        indexes.insert(key, edges.len());
        edges.push(edge);
    }
}

fn created_millis(value: Option<&str>) -> Option<i128> {
    let time = humantime::parse_rfc3339(value?).ok()?;
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => Some(duration.as_millis() as i128),
        Err(error) => Some(-(error.duration().as_millis() as i128)),
    }
}

pub fn relationships(
    threads: &[Value],
    projection: Option<&ThreadState>,
    current: &str,
) -> Vec<Relation> {
    let mut nodes = HashMap::new();
    let mut ordered = Vec::new();
    for thread in threads {
        let Some(id) = text(thread, "id") else {
            continue;
        };
        if let std::collections::hash_map::Entry::Vacant(entry) = nodes.entry(id) {
            entry.insert(Some(thread));
            ordered.push(thread);
        }
    }
    let selected = projection.map(ThreadState::thread);
    let omitted =
        selected.is_some_and(|thread| text(thread, "id").is_some_and(|id| !nodes.contains_key(id)));
    if omitted {
        ordered.extend(selected);
    }
    let mut edges = Vec::new();
    let mut indexes = HashMap::new();
    for thread in ordered {
        let (Some(parent), Some(id)) = (parent_id(thread), text(thread, "id")) else {
            continue;
        };
        let subagent = thread
            .pointer("/lineage/relationshipToParent")
            .and_then(Value::as_str)
            == Some("subagent");
        add_edge(
            &mut nodes,
            &mut edges,
            &mut indexes,
            Edge {
                source: parent,
                target: id,
                kind: if subagent { Kind::Subagent } else { Kind::Fork },
                status: if omitted && selected.and_then(|selected| text(selected, "id")) == Some(id)
                {
                    None
                } else {
                    status(thread)
                },
            },
        );
    }
    if let Some(projection) = projection {
        let owner = projection.thread_id();
        for agent in projection.list("subagents") {
            let Some(child) = text(agent, "childThreadId") else {
                continue;
            };
            let child_shell = nodes.get(child).copied().flatten();
            let agent_status = text(agent, "status");
            let child_status = child_shell
                .and_then(|thread| text(thread, "activityRunStatus"))
                .or_else(|| {
                    if child_shell.is_some_and(|thread| text(thread, "latestRunId").is_some())
                        && !agent_status.is_some_and(active)
                    {
                        child_shell.and_then(|thread| text(thread, "status"))
                    } else {
                        agent_status
                    }
                });
            add_edge(
                &mut nodes,
                &mut edges,
                &mut indexes,
                Edge {
                    source: owner,
                    target: child,
                    kind: Kind::Subagent,
                    status: child_status,
                },
            );
        }
        for transfer in projection.list("contextTransfers") {
            let (Some(source), Some(target)) = (
                text(transfer, "sourceThreadId"),
                text(transfer, "targetThreadId"),
            ) else {
                continue;
            };
            if source == target {
                continue;
            }
            add_edge(
                &mut nodes,
                &mut edges,
                &mut indexes,
                Edge {
                    source,
                    target,
                    kind: Kind::Transfer,
                    status: text(transfer, "status"),
                },
            );
        }
    }
    let mut visited = HashSet::new();
    let mut rows = Vec::new();
    for edge in edges {
        let id = if edge.source == current {
            edge.target
        } else if edge.target == current {
            edge.source
        } else {
            continue;
        };
        if !visited.insert(id) {
            continue;
        }
        let thread = nodes.get(id).copied().flatten();
        let parent = edge.kind != Kind::Transfer && edge.target == current;
        let row_status = if edge.kind == Kind::Transfer || id == edge.target {
            edge.status
        } else {
            thread.and_then(status)
        };
        rows.push(Relation {
            thread_id: id.into(),
            source_thread_id: edge.source.into(),
            target_thread_id: edge.target.into(),
            kind: edge.kind,
            edge_status: edge.status.map(str::to_owned),
            status: row_status.map(str::to_owned),
            parent,
            title: thread
                .and_then(|thread| text(thread, "title"))
                .unwrap_or(id)
                .into(),
            missing: thread.is_none(),
            side_chat: thread
                .and_then(|thread| thread.pointer("/lineage/sideChat"))
                .and_then(Value::as_bool)
                == Some(true),
            created_at: thread
                .and_then(|thread| text(thread, "createdAt"))
                .map(str::to_owned),
        });
    }
    let target = selected.and_then(merge_target);
    let rank = |row: &Relation| {
        if row.parent {
            0
        } else if Some(row.thread_id.as_str()) == target {
            1
        } else {
            2
        }
    };
    rows.sort_by(|left, right| {
        rank(left)
            .cmp(&rank(right))
            .then_with(|| {
                let left_created = created_millis(left.created_at.as_deref());
                let right_created = created_millis(right.created_at.as_deref());
                right_created.cmp(&left_created)
            })
            .then_with(|| left.thread_id.cmp(&right.thread_id))
    });
    rows
}

pub fn latest_merge_run(state: &ThreadState) -> Option<&str> {
    let finished = state
        .list("runs")
        .iter()
        .filter(|run| matches!(text(run, "status"), Some("waiting" | "completed")))
        .max_by_key(|run| run["ordinal"].as_u64().unwrap_or(0))?;
    let ordinal = finished["ordinal"].as_u64().unwrap_or(0);
    if state.list("runs").iter().any(|run| {
        run["ordinal"].as_u64().unwrap_or(0) > ordinal
            && matches!(
                text(run, "status"),
                Some("preparing" | "starting" | "running")
            )
    }) {
        return None;
    }
    text(finished, "id")
}

pub fn can_detach(state: &ThreadState) -> bool {
    let active_run = state.list("runs").iter().rev().find(|run| {
        matches!(
            text(run, "status"),
            Some("preparing" | "starting" | "running" | "waiting")
        )
    });
    let provider_thread_id = active_run
        .and_then(|run| text(run, "providerThreadId"))
        .or_else(|| text(state.thread(), "activeProviderThreadId"));
    let attached = provider_thread_id
        .and_then(|id| {
            state
                .list("providerThreads")
                .iter()
                .find(|thread| text(thread, "id") == Some(id))
        })
        .or_else(|| {
            state.list("providerThreads").iter().find(|thread| {
                text(thread, "appThreadId") == Some(state.thread_id())
                    && text(thread, "providerSessionId").is_some()
            })
        });
    let session = if let Some(id) = attached.and_then(|thread| text(thread, "providerSessionId")) {
        state
            .list("providerSessions")
            .iter()
            .find(|session| text(session, "id") == Some(id))
    } else {
        state
            .list("providerSessions")
            .iter()
            .rev()
            .find(|session| !matches!(text(session, "status"), Some("stopped" | "error")))
    };
    session.is_some_and(|session| !matches!(text(session, "status"), Some("stopped" | "error")))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Details {
    pub rows: Vec<Relation>,
    pub merge_target: Option<String>,
    pub merge_run: Option<String>,
    pub can_detach: bool,
    pub running: usize,
}

impl Details {
    pub fn new(threads: &[Value], state: Option<&ThreadState>, current: &str) -> Self {
        let rows = relationships(threads, state, current);
        let running = rows
            .iter()
            .filter(|row| {
                !row.parent
                    && row.kind == Kind::Subagent
                    && !row.side_chat
                    && row.edge_status.as_deref() == Some("running")
            })
            .count()
            + state
                .map(|state| {
                    state
                        .list("subagents")
                        .iter()
                        .filter(|agent| {
                            text(agent, "childThreadId").is_none()
                                && text(agent, "status") == Some("running")
                        })
                        .count()
                })
                .unwrap_or(0);
        Self {
            rows,
            merge_target: state
                .and_then(|state| merge_target(state.thread()))
                .map(str::to_owned),
            merge_run: state.and_then(latest_merge_run).map(str::to_owned),
            can_detach: state.is_some_and(can_detach),
            running,
        }
    }
}

pub fn group(row: &Relation) -> usize {
    if !row.parent && row.side_chat {
        3
    } else if row.parent || row.kind != Kind::Subagent {
        0
    } else if matches!(
        row.edge_status.as_deref(),
        Some("completed" | "failed" | "error" | "cancelled" | "interrupted" | "idle")
    ) {
        2
    } else {
        1
    }
}

pub fn status_label(status: Option<&str>) -> &'static str {
    match status {
        Some("preparing" | "starting") => "Starting",
        Some("running" | "in_progress") => "Running",
        Some("pending" | "queued") => "Queued",
        Some("waiting" | "blocked") => "Waiting",
        Some("completed") => "Done",
        Some("failed" | "error") => "Failed",
        Some("cancelled" | "interrupted") => "Stopped",
        Some("rolled_back") => "Reverted",
        Some("resolved_native") => "Resolved (native)",
        Some("resolved_portable") => "Resolved (portable)",
        Some("consumed") => "Consumed",
        Some("superseded") => "Superseded",
        Some("idle") => "Idle",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn relationship_graph_matches_the_original_typescript() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/lineage-oracle.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let projection = (!case["projection"].is_null()).then(|| {
                ThreadState::from_snapshot(&json!({"projection":case["projection"]})).unwrap()
            });
            let actual = relationships(
                case["threads"].as_array().unwrap(),
                projection.as_ref(),
                case["current"].as_str().unwrap(),
            );
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                case["expected"],
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn merge_uses_latest_completed_or_waiting_run_unless_a_newer_run_is_active() {
        let mut state =
            ThreadState::from_snapshot(&json!({"projection":{"thread":{"id":"t"},"runs":[
                {"id":"old","ordinal":1,"status":"completed"},
                {"id":"waiting","ordinal":2,"status":"waiting"},
                {"id":"failed","ordinal":3,"status":"failed"}
            ]}}))
            .unwrap();
        assert_eq!(latest_merge_run(&state), Some("waiting"));
        state.projection["runs"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"new","ordinal":4,"status":"starting"}));
        assert_eq!(latest_merge_run(&state), None);
    }

    #[test]
    fn detach_follows_the_attached_session_and_does_not_fall_back_from_a_missing_attachment() {
        let mut state = ThreadState::from_snapshot(&json!({"projection":{
            "thread":{"id":"t","activeProviderThreadId":"pt"},
            "providerThreads":[{"id":"pt","appThreadId":"t","providerSessionId":"attached"}],
            "providerSessions":[{"id":"other","status":"running"}]
        }}))
        .unwrap();
        assert!(!can_detach(&state));
        state.projection["providerSessions"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"attached","status":"stopped"}));
        assert!(!can_detach(&state));
        state.projection["providerSessions"][1]["status"] = json!("running");
        assert!(can_detach(&state));
        state.projection["providerThreads"] = json!([]);
        assert!(can_detach(&state));
        state.projection["providerSessions"] = json!([]);
        assert!(!can_detach(&state));
    }
}
