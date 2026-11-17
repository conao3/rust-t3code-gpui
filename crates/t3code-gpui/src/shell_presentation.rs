use chrono::{DateTime, SecondsFormat};
use serde_json::{Value, json};

fn iso(value: &Value) -> Value {
    value
        .as_str()
        .and_then(|source| DateTime::parse_from_rfc3339(source).ok())
        .map(|value| {
            Value::String(
                value
                    .with_timezone(&chrono::Utc)
                    .to_rfc3339_opts(SecondsFormat::Millis, true),
            )
        })
        .unwrap_or(Value::Null)
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

pub fn present(thread: &Value, pending_subagent: Option<&str>) -> Value {
    let updated = iso(&thread["updatedAt"]);
    let status = text(thread, "status");
    let holding_background = thread
        .get("pendingBackgroundTasks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|task| {
            task["awaited"] == true || matches!(text(task, "kind"), "subagent" | "background_task")
        });
    let park_at_idle = holding_background && status != "failed";
    let runtime = if thread["latestRunId"].is_null()
        && thread["activeProviderThreadId"].is_null()
        && !park_at_idle
    {
        Value::Null
    } else {
        let mut runtime = json!({
            "status":if park_at_idle { "idle" } else { thread.get("activityRunStatus").and_then(Value::as_str).unwrap_or(status) },
            "activeRunId":thread["activeRunId"],
            "providerInstanceId":thread["providerInstanceId"],
            "providerName":null,
            "lastError":thread.get("lastError").unwrap_or(&Value::Null),
            "lastErrorClass":thread.get("lastErrorClass").unwrap_or(&Value::Null),
            "lastErrorCode":thread.get("lastErrorCode").unwrap_or(&Value::Null),
            "usageLimitResetAt":thread.get("usageLimitResetAt").unwrap_or(&Value::Null),
            "updatedAt":updated,
        });
        if let Some(started) = thread.get("activityRunStartedAt") {
            runtime["activityStartedAt"] = iso(started);
        }
        runtime
    };
    let latest_run = if thread["latestRunId"].is_null() {
        Value::Null
    } else {
        json!({
            "runId":thread["latestRunId"],
            "status":if status == "idle" { "completed" } else { status },
            "requestedAt":iso(&thread["latestRunRequestedAt"]),
            "startedAt":iso(&thread["latestRunStartedAt"]),
            "completedAt":thread.get("latestRunCompletedAt").map(iso).unwrap_or_else(|| {
                if matches!(status,"idle"|"completed"|"interrupted"|"failed"|"cancelled"|"rolled_back") { updated.clone() } else { Value::Null }
            }),
            "assistantMessageId":null,
        })
    };
    let request = &thread["pendingRuntimeRequest"];
    let request_kind = text(request, "kind");
    let mut state = json!({
        "runtime":runtime,"latestRun":latest_run,
        "hasPendingApprovals":!request.is_null() && !matches!(request_kind,"user_input"|"auth_refresh"),
        "hasPendingUserInput":request_kind == "user_input",
        "pendingUserInputAt":if request_kind == "user_input" { iso(&request["createdAt"]) } else { Value::Null },
        "pendingSubagentRequest":pending_subagent,
        "pendingGuestMessageCount":thread.get("pendingGuestMessageCount").filter(|value| !value.is_null()).unwrap_or(&json!(0)),
        "latestUserMessageAt":iso(&thread["latestUserMessageAt"]),
        "snoozedUntil":iso(&thread["snoozedUntil"]),"snoozedAt":iso(&thread["snoozedAt"]),
        "settledAt":iso(&thread["settledAt"]),
        "blockedOnThreadWait":thread.get("blockedOnThreadWait").filter(|value| !value.is_null()).unwrap_or(&json!(false)),
        "goal":thread.get("goal").unwrap_or(&Value::Null),
        "providerInstanceHistory":thread.get("providerInstanceHistory").filter(|value| !value.is_null()).unwrap_or(&json!([])),
        "modelSelection":thread["modelSelection"],"updatedAt":updated,
        "source":{"pendingRuntimeRequest":request},
    });
    if let Some(settled) = thread.get("settledOverride") {
        state["settledOverride"] = settled.clone();
    }
    if let Some(visited) = thread.get("lastVisitedAt") {
        state["lastVisitedAt"] = iso(visited);
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_projection_and_actual_sidebar_entries_match_the_original() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/sidebar-state-oracle.json"))
                .unwrap();
        let mut raw_stack_mismatches = 0;
        let mut raw_activity_mismatches = 0;
        for case in fixture["presentation"].as_array().unwrap() {
            let raw = &case["input"];
            let state = present(raw, case["pendingSubagent"].as_str());
            assert_eq!(state, case["expected"], "{case}");
            assert_eq!(
                serde_json::to_value(crate::provider_display::stack(&state)).unwrap(),
                case["stack"],
                "{case}"
            );
            assert_eq!(
                crate::sidebar::activity_timestamp(&state).as_deref(),
                case["activity"].as_str(),
                "{case}"
            );
            let shell = t3_client::projection::ShellState {
                threads: vec![raw.clone()],
                ..Default::default()
            };
            let entries = crate::sidebar::entries("fixture", &shell);
            assert_eq!(entries.len(), 1, "{case}");
            assert_eq!(
                serde_json::to_value(&entries[0].provider_stack).unwrap(),
                case["stack"],
                "{case}"
            );
            assert_eq!(
                entries[0].activity_timestamp.as_deref(),
                case["activity"].as_str(),
                "{case}"
            );
            raw_stack_mismatches += usize::from(
                serde_json::to_value(crate::provider_display::stack(raw)).unwrap() != case["stack"],
            );
            raw_activity_mismatches += usize::from(
                crate::sidebar::activity_timestamp(raw).as_deref() != case["activity"].as_str(),
            );
        }
        assert!(raw_stack_mismatches > 0);
        assert!(raw_activity_mismatches > 0);
        assert_eq!(
            raw_stack_mismatches,
            fixture["negativeRawWiring"]["stack"].as_u64().unwrap() as usize
        );
        assert_eq!(
            raw_activity_mismatches,
            fixture["negativeRawWiring"]["activity"].as_u64().unwrap() as usize
        );
    }
}
