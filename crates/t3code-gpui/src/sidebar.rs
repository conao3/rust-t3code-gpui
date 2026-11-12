use serde_json::Value;
use t3_client::projection::ShellState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadStatus {
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
    pub title: String,
    pub project_title: String,
    pub status: ThreadStatus,
    pub updated_at: String,
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
        return ThreadStatus::Input;
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
                title: text(thread, "title").to_string(),
                project_title,
                status: status_of(thread),
                updated_at: text(thread, "updatedAt").to_string(),
            }
        })
        .collect()
}

pub fn sort_entries(entries: &mut [ThreadEntry]) {
    entries.sort_by(|a, b| {
        let settled = |e: &ThreadEntry| e.status == ThreadStatus::Settled;
        settled(a)
            .cmp(&settled(b))
            .then_with(|| b.updated_at.cmp(&a.updated_at))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let t = thread(json!({"activeRunId": "r1", "pendingRuntimeRequest": {"id": "q"}}));
        assert_eq!(status_of(&t), ThreadStatus::Input);
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
    fn entries_name_the_project_and_sort_settled_last() {
        let shell = ShellState {
            sequence: 1,
            projects: vec![json!({"id": "p1", "title": "idea"})],
            threads: vec![
                thread(json!({"id": "old", "updatedAt": "2026-11-11T00:00:00.000Z"})),
                thread(
                    json!({"id": "settled", "settledAt": "2026-11-12T03:00:00.000Z", "updatedAt": "2026-11-12T03:00:00.000Z"}),
                ),
                thread(json!({"id": "new", "updatedAt": "2026-11-12T02:00:00.000Z"})),
            ],
            synchronized: true,
        };
        let mut list = entries("eos", &shell);
        sort_entries(&mut list);
        let ids: Vec<&str> = list.iter().map(|e| e.thread_id.as_str()).collect();
        assert_eq!(ids, ["new", "old", "settled"]);
        assert_eq!(list[0].project_title, "idea");
        assert_eq!(list[0].environment, "eos");
    }
}
