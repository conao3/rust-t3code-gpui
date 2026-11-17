use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use t3_client::projection::ShellState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadStatus {
    Approval,
    Input,
    Working,
    Failed,
    Done,
    Idle,
    Settled,
}

impl ThreadStatus {
    pub fn label(self) -> &'static str {
        match self {
            ThreadStatus::Approval => "Approval",
            ThreadStatus::Input => "Input",
            ThreadStatus::Working => "Working",
            ThreadStatus::Failed => "Failed",
            ThreadStatus::Done => "Done",
            ThreadStatus::Idle => "",
            ThreadStatus::Settled => "Settled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadEntry {
    pub environment: String,
    pub thread_id: String,
    pub project_id: String,
    pub title: String,
    pub project_title: String,
    pub status: ThreadStatus,
    pub latest_user_message_at: Option<String>,
    pub pin_order: Option<String>,
    pub snoozed_until: Option<String>,
    pub launched_from: Option<(String, String)>,
    pub provider_stack: Vec<String>,
    pub activity_timestamp: Option<String>,
    pub presentation: Arc<Value>,
    pub inbox_working: Option<bool>,
    pub returned_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DisplayStatus {
    Approval,
    Input,
    Request,
    Preparing,
    Working,
    Waiting,
    Limited,
    Failed,
    Ready,
}

impl DisplayStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Approval => "Approval",
            Self::Input => "Input",
            Self::Request => "Request",
            Self::Preparing => "Setting up",
            Self::Working => "Working",
            Self::Waiting => "Waiting",
            Self::Limited => "Limited",
            Self::Failed => "Failed",
            Self::Ready => "",
        }
    }
}

pub fn display_status(thread: &Value) -> DisplayStatus {
    if thread["hasPendingApprovals"] == true || thread["pendingSubagentRequest"] == "approval" {
        return DisplayStatus::Approval;
    }
    if thread["hasPendingUserInput"] == true || thread["pendingSubagentRequest"] == "user_input" {
        return DisplayStatus::Input;
    }
    if thread["pendingGuestMessageCount"]
        .as_f64()
        .unwrap_or_default()
        > 0.
    {
        return DisplayStatus::Request;
    }
    let runtime = &thread["runtime"];
    match runtime["status"].as_str() {
        Some("running") if thread["blockedOnThreadWait"] == true => DisplayStatus::Waiting,
        Some("preparing") => DisplayStatus::Preparing,
        Some("queued" | "starting" | "running" | "waiting") => DisplayStatus::Working,
        Some("idle") => DisplayStatus::Waiting,
        Some("failed") if runtime["lastErrorClass"] == "usage_limit" => DisplayStatus::Limited,
        Some("failed") => DisplayStatus::Failed,
        _ => DisplayStatus::Ready,
    }
}

pub fn unseen_completion(thread: &Value) -> bool {
    let Some(completed) = thread
        .pointer("/latestRun/completedAt")
        .and_then(Value::as_str)
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
    else {
        return false;
    };
    let Some(visited) = thread["lastVisitedAt"]
        .as_str()
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    chrono::DateTime::parse_from_rfc3339(visited).map_or(true, |visited| completed > visited)
}

pub fn should_recede(
    status: DisplayStatus,
    unread: bool,
    woke: bool,
    active: bool,
    selected: bool,
) -> bool {
    if active || selected || matches!(status, DisplayStatus::Input | DisplayStatus::Request) {
        return false;
    }
    match status {
        DisplayStatus::Preparing | DisplayStatus::Working | DisplayStatus::Waiting => true,
        DisplayStatus::Ready | DisplayStatus::Approval => !unread && !woke,
        _ => false,
    }
}

pub fn working_started_at(thread: &Value) -> Option<&str> {
    fn valid(value: &Value) -> Option<&str> {
        value
            .as_str()
            .filter(|value| chrono::DateTime::parse_from_rfc3339(value).is_ok())
    }
    let runtime = &thread["runtime"];
    if let Some(value) = runtime.get("activityStartedAt") {
        return valid(value);
    }
    let run = &thread["latestRun"];
    if run.get("completedAt") == Some(&Value::Null)
        && run.get("runId").is_some()
        && run["runId"] == runtime["activeRunId"]
    {
        valid(&run["startedAt"]).or_else(|| valid(&run["requestedAt"]))
    } else {
        None
    }
}

pub fn duration_label(elapsed_ms: f64) -> String {
    let seconds = if elapsed_ms.is_finite() {
        (elapsed_ms / 1000.).floor().max(0.)
    } else {
        0.
    };
    if seconds < 60. {
        format!("{seconds:.0}s")
    } else {
        let minutes = (seconds / 60.).floor();
        if minutes < 60. {
            format!("{minutes:.0}m")
        } else {
            format!("{:.0}h {:.0}m", (minutes / 60.).floor(), minutes % 60.)
        }
    }
}

pub fn activity_timestamp(thread: &Value) -> Option<String> {
    let candidates = [
        thread.get("latestUserMessageAt"),
        thread.pointer("/latestRun/requestedAt"),
        thread.pointer("/latestRun/startedAt"),
        thread.pointer("/latestRun/completedAt"),
    ];
    let mut latest = None;
    for candidate in candidates.into_iter().flatten().filter_map(Value::as_str) {
        if let Ok(timestamp) = humantime::parse_rfc3339(candidate)
            && latest
                .as_ref()
                .is_none_or(|(_, previous)| timestamp > *previous)
        {
            latest = Some((candidate, timestamp));
        }
    }
    latest.map(|(value, _)| value.to_string()).or_else(|| {
        let value = text(thread, "updatedAt");
        humantime::parse_rfc3339(value)
            .ok()
            .map(|_| value.to_string())
    })
}

pub fn relative_label(timestamp: &str, now: SystemTime) -> String {
    let Ok(timestamp) = humantime::parse_rfc3339(timestamp) else {
        return String::new();
    };
    let minutes = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() / 60;
    let elapsed = (UNIX_EPOCH + std::time::Duration::from_secs(minutes * 60))
        .duration_since(timestamp)
        .unwrap_or_default()
        .as_secs();
    match elapsed {
        0..60 => "now".into(),
        60..3600 => format!("{}m", elapsed / 60),
        3600..86400 => format!("{}h", elapsed / 3600),
        _ => format!("{}d", elapsed / 86400),
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn present(value: &Value, key: &str) -> bool {
    value.get(key).is_some_and(|v| !v.is_null())
}

pub fn is_listed(thread: &Value) -> bool {
    let lineage = thread.get("lineage").unwrap_or(&Value::Null);
    let subagent = text(lineage, "relationshipToParent") == "subagent";
    let side_chat = lineage
        .get("sideChat")
        .is_some_and(|v| !v.is_null() && v != false);
    !subagent && !side_chat && !present(thread, "archivedAt") && !present(thread, "deletedAt")
}

pub fn status_of(thread: &Value) -> ThreadStatus {
    if present(thread, "pendingRuntimeRequest") {
        return match text(&thread["pendingRuntimeRequest"], "kind") {
            "user_input" => ThreadStatus::Input,
            _ => ThreadStatus::Approval,
        };
    }
    if present(thread, "activeRunId") {
        return ThreadStatus::Working;
    }
    if is_settled(thread) {
        return ThreadStatus::Settled;
    }
    if text(thread, "activityRunStatus") == "failed" || text(thread, "status") == "failed" {
        return ThreadStatus::Failed;
    }
    if has_unseen_completion(thread) {
        return ThreadStatus::Done;
    }
    ThreadStatus::Idle
}

fn is_settled(thread: &Value) -> bool {
    match text(thread, "settledOverride") {
        "settled" => true,
        "active" => false,
        _ => present(thread, "settledAt"),
    }
}

pub fn has_unseen_completion(thread: &Value) -> bool {
    let completed = text(thread, "latestRunCompletedAt");
    let visited = text(thread, "lastVisitedAt");
    !completed.is_empty() && !visited.is_empty() && completed > visited
}

pub fn entries(environment: &str, shell: &ShellState) -> Vec<ThreadEntry> {
    shell
        .threads
        .iter()
        .filter(|thread| is_listed(thread))
        .map(|thread| {
            let presentation = crate::shell_presentation::present(thread, None);
            let project_id = text(thread, "projectId");
            let project_title = shell
                .projects
                .iter()
                .find(|p| text(p, "id") == project_id)
                .map(|p| text(p, "title").to_string())
                .unwrap_or_default();
            ThreadEntry {
                environment: environment.to_string(),
                thread_id: text(thread, "id").to_string(),
                project_id: project_id.to_string(),
                title: text(thread, "title").to_string(),
                project_title,
                status: status_of(thread),
                latest_user_message_at: thread
                    .get("latestUserMessageAt")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                pin_order: present(thread, "pinnedAt").then(|| {
                    thread
                        .get("pinOrderKey")
                        .and_then(Value::as_str)
                        .unwrap_or(text(thread, "pinnedAt"))
                        .to_string()
                }),
                snoozed_until: present(thread, "snoozedUntil")
                    .then(|| text(thread, "snoozedUntil").to_string()),
                launched_from: thread.get("launchedFrom").and_then(|l| {
                    Some((
                        l.get("environmentId")?.as_str()?.to_string(),
                        l.get("threadId")?.as_str()?.to_string(),
                    ))
                }),
                provider_stack: crate::provider_display::stack(&presentation),
                activity_timestamp: activity_timestamp(&presentation),
                presentation: Arc::new(presentation),
                inbox_working: None,
                returned_at: None,
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Section {
    Pinned,
    Working,
    Active,
    Snoozed,
    Settled,
}

impl Section {
    pub fn label(self) -> &'static str {
        match self {
            Section::Pinned => "Pinned",
            Section::Working => "Working",
            Section::Active => "Threads",
            Section::Snoozed => "Snoozed",
            Section::Settled => "Settled",
        }
    }
}

/// The shelf a thread sits on, as T3's sidebar places it: snoozed before settled before pinned,
/// and a working thread on the Working shelf unless it is snoozed, settled or pinned.
pub fn section_of(entry: &ThreadEntry, now: &str) -> Section {
    if crate::snooze::effective(&entry.presentation, now) {
        return Section::Snoozed;
    }
    if entry.presentation["settledOverride"] == "settled" {
        return Section::Settled;
    }
    if entry.pin_order.is_some() {
        return Section::Pinned;
    }
    if entry
        .inbox_working
        .unwrap_or_else(|| crate::inbox::is_working(&entry.presentation))
    {
        return Section::Working;
    }
    Section::Active
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shelf {
    pub section: Section,
    pub rows: Vec<(ThreadEntry, usize)>,
}

/// The threads in shelves, newest first (pinned threads in their pin order), with a launched
/// thread under the thread that launched it when both are on the same shelf. `label_of` names
/// the host of a server environment id.
pub fn shelves(
    entries: &[ThreadEntry],
    now: &str,
    working_shelf: bool,
    label_of: impl Fn(&str) -> Option<String>,
) -> Vec<Shelf> {
    let mut shelves: Vec<Shelf> = Vec::new();
    for section in [
        Section::Pinned,
        Section::Active,
        Section::Working,
        Section::Snoozed,
        Section::Settled,
    ] {
        let mut members: Vec<&ThreadEntry> = entries
            .iter()
            .filter(|e| section_of(e, now) == section)
            .collect();
        let order = match section {
            Section::Pinned => crate::inbox::Order::Pinned,
            Section::Active if working_shelf => crate::inbox::Order::Inbox,
            Section::Active => crate::inbox::Order::Active,
            Section::Working => crate::inbox::Order::Working,
            Section::Snoozed => crate::inbox::Order::Snoozed,
            Section::Settled => crate::inbox::Order::Settled,
        };
        members.sort_by(|a, b| {
            crate::inbox::compare(
                &a.presentation,
                &b.presentation,
                a.returned_at,
                b.returned_at,
                order,
                crate::inbox::collator(),
            )
        });
        let parent_of = |e: &ThreadEntry| -> Option<(String, String)> {
            let (environment_id, thread_id) = e.launched_from.as_ref()?;
            let parent = (label_of(environment_id)?, thread_id.clone());
            members.iter().any(|m| m.key() == parent).then_some(parent)
        };
        let mut rows = Vec::new();
        fn place<'a>(
            entry: &'a ThreadEntry,
            depth: usize,
            children: &dyn Fn(&ThreadEntry) -> Vec<&'a ThreadEntry>,
            rows: &mut Vec<(ThreadEntry, usize)>,
        ) {
            rows.push((entry.clone(), depth));
            for child in children(entry) {
                place(child, depth + 1, children, rows);
            }
        }
        let children = |parent: &ThreadEntry| -> Vec<&ThreadEntry> {
            members
                .iter()
                .filter(|m| parent_of(m).is_some_and(|p| p == parent.key()))
                .copied()
                .collect()
        };
        for entry in members.iter().filter(|m| parent_of(m).is_none()) {
            place(entry, 0, &children, &mut rows);
        }
        if !rows.is_empty() {
            shelves.push(Shelf { section, rows });
        }
    }
    shelves
}

impl ThreadEntry {
    pub fn is(&self, environment: &str, thread_id: &str) -> bool {
        self.environment == environment && self.thread_id == thread_id
    }

    pub fn key(&self) -> (String, String) {
        (self.environment.clone(), self.thread_id.clone())
    }
}

/// The entries after the current one, wrapping around, without the current one. With no
/// current entry, every entry from the top.
fn ring<'a>(
    entries: &'a [ThreadEntry],
    current: Option<(&str, &str)>,
    forward: bool,
) -> Vec<&'a ThreadEntry> {
    let position = current.and_then(|(env, id)| entries.iter().position(|e| e.is(env, id)));
    let mut ordered: Vec<&ThreadEntry> = match position {
        Some(i) => entries[i + 1..].iter().chain(&entries[..i]).collect(),
        None => entries.iter().collect(),
    };
    if !forward {
        ordered.reverse();
    }
    ordered
}

pub fn adjacent<'a>(
    entries: &'a [ThreadEntry],
    current: Option<(&str, &str)>,
    forward: bool,
) -> Option<&'a ThreadEntry> {
    ring(entries, current, forward).into_iter().next()
}

pub fn next_with<'a>(
    entries: &'a [ThreadEntry],
    current: Option<(&str, &str)>,
    forward: bool,
    wanted: impl Fn(&ThreadEntry) -> bool,
) -> Option<&'a ThreadEntry> {
    ring(entries, current, forward)
        .into_iter()
        .find(|e| wanted(e))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Family {
    pub children: usize,
    pub working: usize,
    pub collapsed: bool,
}

/// The rows of a shelf to draw: a thread with launched threads under it carries their count and
/// how many are working, and the threads under a collapsed one are left out.
pub fn visible_rows(
    rows: &[(ThreadEntry, usize)],
    collapsed: &HashSet<(String, String)>,
) -> Vec<(ThreadEntry, usize, Option<Family>)> {
    let mut out = Vec::new();
    let mut skip_deeper_than: Option<usize> = None;
    for (i, (entry, depth)) in rows.iter().enumerate() {
        if let Some(limit) = skip_deeper_than {
            if *depth > limit {
                continue;
            }
            skip_deeper_than = None;
        }
        let descendants: Vec<&ThreadEntry> = rows[i + 1..]
            .iter()
            .take_while(|(_, d)| d > depth)
            .map(|(e, _)| e)
            .collect();
        let is_collapsed = !descendants.is_empty() && collapsed.contains(&entry.key());
        let family = (!descendants.is_empty()).then(|| Family {
            children: descendants.len(),
            working: descendants
                .iter()
                .filter(|e| e.status == ThreadStatus::Working)
                .count(),
            collapsed: is_collapsed,
        });
        if is_collapsed {
            skip_deeper_than = Some(*depth);
        }
        out.push((entry.clone(), *depth, family));
    }
    out
}

pub fn needs_attention(status: ThreadStatus) -> bool {
    matches!(status, ThreadStatus::Done | ThreadStatus::Input)
}

/// Threads that became Done or started waiting for input since the previous listing. Threads
/// the previous listing did not have are not reported, so the first listing reports nothing.
pub fn newly_needing_attention<'a>(
    previous: &HashMap<(String, String), ThreadStatus>,
    entries: &'a [ThreadEntry],
) -> Vec<&'a ThreadEntry> {
    entries
        .iter()
        .filter(|e| needs_attention(e.status) || e.status == ThreadStatus::Approval)
        .filter(|e| {
            previous
                .get(&e.key())
                .is_some_and(|before| *before != e.status)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_settlement_parks_an_active_run() {
        let shell = ShellState {
            threads: vec![
                serde_json::json!({"id":"thread","projectId":"project","title":"Working parking fixture","status":"running","activeRunId":"run","latestRunId":"run","settledOverride":"settled","updatedAt":"2026-11-17T12:00:00Z"}),
            ],
            ..Default::default()
        };
        let entries = entries("fixture", &shell);
        assert_eq!(entries[0].status, ThreadStatus::Working);
        assert_eq!(
            section_of(&entries[0], "2026-11-17T12:00:00Z"),
            Section::Settled
        );
    }

    #[test]
    fn display_state_and_receding_match_original() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/sidebar-status-oracle.json"))
                .unwrap();
        for case in fixture["statuses"].as_array().unwrap() {
            let expected: DisplayStatus = serde_json::from_value(case["expected"].clone()).unwrap();
            assert_eq!(display_status(&case["input"]), expected, "{case}");
        }
        for case in fixture["receding"].as_array().unwrap() {
            let input = &case["input"];
            let status = serde_json::from_value(input["status"].clone()).unwrap();
            assert_eq!(
                should_recede(
                    status,
                    input["isUnread"] == true,
                    input["isWoke"] == true,
                    input["isActive"] == true,
                    input["isSelected"] == true
                ),
                case["expected"] == true,
                "{case}"
            );
        }
        for case in fixture["starts"].as_array().unwrap() {
            assert_eq!(
                working_started_at(&case["input"]),
                case["expected"].as_str(),
                "{case}"
            );
        }
        for case in fixture["durations"].as_array().unwrap() {
            assert_eq!(
                duration_label(case["input"].as_f64().unwrap()),
                case["expected"].as_str().unwrap(),
                "{case}"
            );
        }
        for case in fixture["unread"].as_array().unwrap() {
            assert_eq!(
                unseen_completion(&case["input"]),
                case["expected"] == true,
                "{case}"
            );
        }
    }

    #[test]
    fn activity_and_compact_time_match_the_original() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/provider-display-oracle.json"
        ))
        .unwrap();
        for case in fixture["activity"].as_array().unwrap() {
            assert_eq!(
                activity_timestamp(&case["input"]).as_deref(),
                case["expected"].as_str(),
                "{case}"
            );
        }
        for case in fixture["times"].as_array().unwrap() {
            let now = UNIX_EPOCH + std::time::Duration::from_millis(case["now"].as_u64().unwrap());
            assert_eq!(
                relative_label(case["timestamp"].as_str().unwrap(), now),
                case["expected"].as_str().unwrap(),
                "{case}"
            );
        }
    }
    use serde_json::json;

    fn thread(extra: Value) -> Value {
        let mut base = json!({
            "id": "t1",
            "projectId": "p1",
            "title": "日本語のタイトル #t3code",
            "lineage": {"parentThreadId": null, "relationshipToParent": null, "rootThreadId": "t1"},
            "activeRunId": null,
            "pendingRuntimeRequest": null,
            "settledOverride": null,
            "settledAt": null,
            "archivedAt": null,
            "deletedAt": null,
            "updatedAt": "2026-11-12T01:00:00.000Z",
        });
        for (k, v) in extra.as_object().unwrap() {
            base[k] = v.clone();
        }
        base
    }

    #[test]
    fn a_pending_request_wins_over_a_running_thread() {
        let question = thread(
            json!({"activeRunId": "r1", "pendingRuntimeRequest": {"id": "q", "kind": "user_input"}}),
        );
        assert_eq!(status_of(&question), ThreadStatus::Input);
        let approval = thread(
            json!({"activeRunId": "r1", "pendingRuntimeRequest": {"id": "q", "kind": "command"}}),
        );
        assert_eq!(status_of(&approval), ThreadStatus::Approval);
        assert_eq!(
            status_of(&thread(json!({"activeRunId": "r1"}))),
            ThreadStatus::Working
        );
    }

    #[test]
    fn done_means_completed_after_the_last_visit() {
        let done = thread(json!({
            "latestRunCompletedAt": "2026-11-12T02:00:00.000Z",
            "lastVisitedAt": "2026-11-12T01:00:00.000Z",
        }));
        let seen = thread(json!({
            "latestRunCompletedAt": "2026-11-12T01:00:00.000Z",
            "lastVisitedAt": "2026-11-12T02:00:00.000Z",
        }));
        let never_visited = thread(json!({"latestRunCompletedAt": "2026-11-12T02:00:00.000Z"}));
        assert_eq!(status_of(&done), ThreadStatus::Done);
        assert_eq!(status_of(&seen), ThreadStatus::Idle);
        assert_eq!(status_of(&never_visited), ThreadStatus::Idle);
    }

    #[test]
    fn settled_override_decides_over_settled_at() {
        let settled = thread(json!({"settledAt": "2026-11-12T00:00:00.000Z"}));
        let reopened =
            thread(json!({"settledAt": "2026-11-12T00:00:00.000Z", "settledOverride": "active"}));
        assert_eq!(status_of(&settled), ThreadStatus::Settled);
        assert_eq!(status_of(&reopened), ThreadStatus::Idle);
    }

    #[test]
    fn subagents_side_chats_and_archived_threads_are_not_listed() {
        let subagent = thread(json!({"lineage": {"relationshipToParent": "subagent"}}));
        let side =
            thread(json!({"lineage": {"relationshipToParent": null, "sideChat": {"anchor": "x"}}}));
        let archived = thread(json!({"archivedAt": "2026-11-12T00:00:00.000Z"}));
        assert!(is_listed(&thread(json!({}))));
        assert!(!is_listed(&subagent));
        assert!(!is_listed(&side));
        assert!(!is_listed(&archived));
    }

    #[test]
    fn entries_go_on_shelves_newest_first_with_settled_last() {
        let shell = ShellState {
            sequence: 1,
            projects: vec![json!({"id": "p1", "title": "idea"})],
            threads: vec![
                thread(
                    json!({"id": "old", "createdAt": "2026-11-11T00:00:00.000Z", "updatedAt": "2026-11-13T00:00:00.000Z"}),
                ),
                thread(
                    json!({"id": "settled", "settledOverride": "settled", "settledAt": "2026-11-12T03:00:00.000Z", "updatedAt": "2026-11-12T03:00:00.000Z"}),
                ),
                thread(
                    json!({"id": "new", "createdAt": "2026-11-12T02:00:00.000Z", "updatedAt": "2026-11-12T02:00:00.000Z"}),
                ),
            ],
            synchronized: true,
        };
        let list = entries("eos", &shell);
        let shelves = shelves(&list, "2026-11-12T05:00:00.000Z", true, |_| None);
        let layout: Vec<(Section, Vec<&str>)> = shelves
            .iter()
            .map(|s| {
                (
                    s.section,
                    s.rows.iter().map(|(e, _)| e.thread_id.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            layout,
            [
                (Section::Active, vec!["new", "old"]),
                (Section::Settled, vec!["settled"])
            ]
        );
        assert_eq!(shelves[0].rows[0].0.project_title, "idea");
        assert_eq!(shelves[0].rows[0].0.environment, "eos");
    }

    fn entry(id: &str, status: ThreadStatus) -> ThreadEntry {
        ThreadEntry {
            environment: "eos".into(),
            thread_id: id.into(),
            project_id: "p".into(),
            title: id.into(),
            project_title: String::new(),
            status,
            latest_user_message_at: None,
            pin_order: None,
            snoozed_until: None,
            launched_from: None,
            provider_stack: Vec::new(),
            activity_timestamp: None,
            presentation: Arc::new(
                serde_json::json!({"id":id,"settledOverride":if status==ThreadStatus::Settled {"settled"} else {"active"},"runtime":if status==ThreadStatus::Working {serde_json::json!({"status":"running"})} else {Value::Null}}),
            ),
            inbox_working: None,
            returned_at: None,
        }
    }

    #[test]
    fn adjacent_threads_wrap_around_the_list() {
        let list = [
            entry("a", ThreadStatus::Idle),
            entry("b", ThreadStatus::Done),
            entry("c", ThreadStatus::Settled),
        ];
        let id = |e: Option<&ThreadEntry>| e.map(|e| e.thread_id.clone());
        assert_eq!(
            id(adjacent(&list, Some(("eos", "c")), true)),
            Some("a".into())
        );
        assert_eq!(
            id(adjacent(&list, Some(("eos", "a")), false)),
            Some("c".into())
        );
        assert_eq!(id(adjacent(&list, None, true)), Some("a".into()));
        assert_eq!(id(adjacent(&list, None, false)), Some("c".into()));
        let open = |e: &ThreadEntry| e.status != ThreadStatus::Settled;
        assert_eq!(
            id(next_with(&list, Some(("eos", "b")), true, open)),
            Some("a".into())
        );
        assert_eq!(
            id(next_with(&list[..1], Some(("eos", "a")), true, open)),
            None
        );
    }

    #[test]
    fn attention_is_reported_once_per_change() {
        let before: HashMap<(String, String), ThreadStatus> = [
            (("eos".into(), "a".into()), ThreadStatus::Working),
            (("eos".into(), "b".into()), ThreadStatus::Done),
            (("eos".into(), "c".into()), ThreadStatus::Working),
        ]
        .into();
        let now = [
            entry("a", ThreadStatus::Done),
            entry("b", ThreadStatus::Done),
            entry("c", ThreadStatus::Input),
            entry("new", ThreadStatus::Done),
        ];
        let ids: Vec<&str> = newly_needing_attention(&before, &now)
            .iter()
            .map(|e| e.thread_id.as_str())
            .collect();
        assert_eq!(ids, ["a", "c"]);
    }

    #[test]
    fn shelves_follow_t3_and_launched_threads_nest_under_their_launcher() {
        let now = "2026-11-12T05:00:00.000Z";
        let at = |id: &str, status: ThreadStatus, updated: &str| {
            let mut e = entry(id, status);
            Arc::make_mut(&mut e.presentation)["createdAt"] =
                json!(format!("2026-11-12T{:0>2}:00:00.000Z", updated));
            e
        };
        let mut pinned_b = at("pin-b", ThreadStatus::Idle, "1");
        pinned_b.pin_order = Some("b".into());
        Arc::make_mut(&mut pinned_b.presentation)["pinOrderKey"] = json!("b");
        let mut pinned_a = at("pin-a", ThreadStatus::Working, "0");
        pinned_a.pin_order = Some("a".into());
        Arc::make_mut(&mut pinned_a.presentation)["pinOrderKey"] = json!("a");
        let mut snoozed = at("snoozed", ThreadStatus::Settled, "9");
        snoozed.snoozed_until = Some("2026-11-12T06:00:00.000Z".into());
        Arc::make_mut(&mut snoozed.presentation)["snoozedUntil"] = json!(snoozed.snoozed_until);
        let mut woke = at("woke", ThreadStatus::Idle, "2");
        woke.snoozed_until = Some("2026-11-12T04:00:00.000Z".into());
        Arc::make_mut(&mut woke.presentation)["snoozedUntil"] = json!(woke.snoozed_until);
        let parent = at("parent", ThreadStatus::Idle, "3");
        let mut child = at("child", ThreadStatus::Done, "8");
        child.launched_from = Some(("env-eos".into(), "parent".into()));
        let mut orphan = at("orphan", ThreadStatus::Idle, "7");
        orphan.launched_from = Some(("env-eos".into(), "missing".into()));
        let working = at("working", ThreadStatus::Working, "5");
        let list = [
            pinned_b, pinned_a, snoozed, woke, parent, child, orphan, working,
        ];
        let label = |id: &str| (id == "env-eos").then(|| "eos".to_string());
        let layout: Vec<(Section, Vec<(String, usize)>)> = shelves(&list, now, true, label)
            .into_iter()
            .map(|s| {
                (
                    s.section,
                    s.rows.into_iter().map(|(e, d)| (e.thread_id, d)).collect(),
                )
            })
            .collect();
        let expected: Vec<(Section, Vec<(&str, usize)>)> = vec![
            (Section::Pinned, vec![("pin-a", 0), ("pin-b", 0)]),
            (
                Section::Active,
                vec![("orphan", 0), ("parent", 0), ("child", 1), ("woke", 0)],
            ),
            (Section::Working, vec![("working", 0)]),
            (Section::Snoozed, vec![("snoozed", 0)]),
        ];
        let expected: Vec<(Section, Vec<(String, usize)>)> = expected
            .into_iter()
            .map(|(s, rows)| {
                (
                    s,
                    rows.into_iter()
                        .map(|(id, d)| (id.to_string(), d))
                        .collect(),
                )
            })
            .collect();
        assert_eq!(layout, expected);
    }

    #[test]
    fn collapsing_a_launcher_hides_its_family_and_counts_the_working() {
        let rows = vec![
            (entry("parent", ThreadStatus::Idle), 0),
            (entry("child", ThreadStatus::Working), 1),
            (entry("grandchild", ThreadStatus::Done), 2),
            (entry("other", ThreadStatus::Idle), 0),
        ];
        let open = visible_rows(&rows, &HashSet::new());
        assert_eq!(open.len(), 4);
        assert_eq!(
            open[0].2,
            Some(Family {
                children: 2,
                working: 1,
                collapsed: false
            })
        );
        assert_eq!(
            open[1].2,
            Some(Family {
                children: 1,
                working: 0,
                collapsed: false
            })
        );
        assert_eq!(open[3].2, None);
        let closed = visible_rows(
            &rows,
            &HashSet::from([("eos".to_string(), "parent".to_string())]),
        );
        let ids: Vec<&str> = closed
            .iter()
            .map(|(e, _, _)| e.thread_id.as_str())
            .collect();
        assert_eq!(ids, ["parent", "other"]);
        assert!(closed[0].2.unwrap().collapsed);
    }
}
