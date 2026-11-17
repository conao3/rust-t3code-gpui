use std::collections::{HashMap, HashSet};

use serde_json::Value;

pub type Key = (String, String);

pub fn is_working(thread: &Value) -> bool {
    if thread["hasPendingApprovals"] == true
        || thread["hasPendingUserInput"] == true
        || !thread["pendingSubagentRequest"].is_null()
    {
        return false;
    }
    let runtime = &thread["runtime"];
    if !matches!(
        runtime["status"].as_str(),
        Some("preparing" | "queued" | "starting" | "running" | "waiting" | "idle")
    ) {
        return false;
    }
    let run = &thread["latestRun"];
    let settled = !run.is_null()
        && !matches!(
            run["status"].as_str(),
            Some("preparing" | "queued" | "starting" | "running" | "waiting")
        )
        && runtime["activeRunId"] != run["runId"];
    !(thread["interactionMode"] == "plan" && thread["hasActionableProposedPlan"] == true && settled)
}

pub fn as_launching(thread: &mut Value) {
    if !thread["latestRun"].is_null() || !thread["runtime"].is_null() {
        return;
    }
    let mut runtime = serde_json::json!({"status":"preparing","activeRunId":null,"providerName":null,"lastError":null});
    for field in ["providerInstanceId", "updatedAt"] {
        if let Some(value) = thread.get(field) {
            runtime[field] = value.clone();
        }
    }
    thread["runtime"] = runtime;
}

struct State {
    working: bool,
    deadline: Option<i64>,
    placement: Option<Vec<Value>>,
}

#[derive(Default)]
pub struct Tracker {
    states: HashMap<Key, State>,
    returns: HashMap<Key, i64>,
}

impl Tracker {
    pub fn clear(&mut self) {
        self.states.clear();
        self.returns.clear();
    }

    pub fn observe<'a>(&mut self, threads: impl IntoIterator<Item = (Key, &'a Value)>, now: i64) {
        let mut present = HashSet::new();
        let observed_at = chrono::DateTime::from_timestamp_millis(now)
            .expect("inbox clock timestamp")
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        for (key, thread) in threads {
            present.insert(key.clone());
            let previous = self.states.get(&key);
            let actual = is_working(thread);
            let placement = (actual || previous.is_some_and(|state| state.working)).then(|| {
                [
                    "archivedAt",
                    "deletedAt",
                    "pinnedAt",
                    "settledOverride",
                    "settledAt",
                    "snoozedAt",
                    "snoozedUntil",
                    "activeOrderKey",
                ]
                .map(|field| thread[field].clone())
                .to_vec()
            });
            let actionable = thread["hasPendingApprovals"] == true
                || thread["hasPendingUserInput"] == true
                || !thread["pendingSubagentRequest"].is_null()
                || thread["pendingGuestMessageCount"].as_f64().unwrap_or(0.) > 0.
                || (thread["interactionMode"] == "plan"
                    && thread["hasActionableProposedPlan"] == true)
                || !thread["runtime"]["lastError"].is_null()
                || thread["latestRun"]["status"] == "failed";
            let moved = previous.is_some_and(|state| state.placement != placement);
            let excluded = ["archivedAt", "deletedAt", "pinnedAt"]
                .iter()
                .any(|field| !thread[*field].is_null())
                || thread["settledOverride"] == "settled"
                || crate::snooze::effective(thread, &observed_at);
            let deadline = previous
                .filter(|state| state.working && !actual && !actionable && !moved && !excluded)
                .map(|state| state.deadline.unwrap_or(now + 2000));
            let working = actual || deadline.is_some_and(|deadline| deadline > now);
            if previous.is_some_and(|state| state.working) && !working {
                self.returns.insert(key.clone(), now);
            }
            self.states.insert(
                key,
                State {
                    working,
                    deadline: deadline.filter(|_| working),
                    placement,
                },
            );
        }
        self.states.retain(|key, _| present.contains(key));
        self.returns.retain(|key, _| present.contains(key));
    }

    pub fn expire(&mut self, now: i64) -> bool {
        let mut changed = false;
        for (key, state) in &mut self.states {
            if state.deadline.is_some_and(|deadline| deadline <= now) {
                state.working = false;
                state.deadline = None;
                self.returns.insert(key.clone(), now);
                changed = true;
            }
        }
        changed
    }

    pub fn deadline(&self) -> Option<i64> {
        self.states
            .values()
            .filter_map(|state| state.deadline)
            .min()
    }

    pub fn is_working(&self, key: &Key, thread: &Value) -> bool {
        self.states
            .get(key)
            .map_or_else(|| is_working(thread), |state| state.working)
    }

    pub fn returned_at(&self, key: &Key) -> Option<i64> {
        self.returns.get(key).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key(value: &Value) -> Key {
        (
            value["environmentId"].as_str().unwrap().into(),
            value["id"].as_str().unwrap().into(),
        )
    }

    #[test]
    fn working_state_and_grace_transitions_match_the_original() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/inbox-oracle.json")).unwrap();
        for case in fixture["working"].as_array().unwrap() {
            assert_eq!(is_working(&case["input"]), case["expected"], "{case}");
        }
        for case in fixture["launching"].as_array().unwrap() {
            let mut input = case["input"].clone();
            as_launching(&mut input);
            assert_eq!(input, case["expected"], "{case}");
        }
        for scenario in fixture["scenarios"].as_array().unwrap() {
            let mut tracker = Tracker::default();
            let mut threads = vec![];
            let mut version = 0;
            for step in scenario["steps"].as_array().unwrap() {
                let now = step["at"].as_i64().unwrap();
                match step["action"].as_str().unwrap() {
                    "observe" => {
                        if let Some(values) = step["threads"].as_array() {
                            threads = values.clone();
                            tracker.observe(threads.iter().map(|value| (key(value), value)), now);
                        } else {
                            threads.clear();
                            tracker.clear();
                        }
                    }
                    "advance" => {
                        while let Some(deadline) = tracker.deadline().filter(|time| *time <= now) {
                            let at = if step["late"] == true { now } else { deadline };
                            version += i64::from(tracker.expire(at));
                        }
                    }
                    action => panic!("Unknown inbox oracle action: {action}"),
                }
                let result = json!({
                    "version":version,
                    "deadline":tracker.deadline(),
                    "threads":threads.iter().map(|thread|json!({
                        "id":thread["id"],"environmentId":thread["environmentId"],
                        "working":tracker.is_working(&key(thread),thread),
                        "returnedAt":tracker.returned_at(&key(thread)),
                    })).collect::<Vec<_>>(),
                });
                assert_eq!(result, step["expected"], "{}: {step}", scenario["name"]);
            }
        }
    }
    #[test]
    fn shelf_ordering_matches_the_original() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/inbox-oracle.json")).unwrap();
        let locale: icu_locale_core::Locale = "en-US".parse().unwrap();
        let collator = icu_collator::Collator::try_new(locale.into(), Default::default()).unwrap();
        for case in fixture["sorting"].as_array().unwrap() {
            for (name, order) in [
                ("pinned", Order::Pinned),
                ("active", Order::Active),
                ("inbox", Order::Inbox),
                ("working", Order::Working),
                ("settled", Order::Settled),
                ("snoozed", Order::Snoozed),
            ] {
                let mut rows = case["input"].as_array().unwrap().iter().collect::<Vec<_>>();
                rows.sort_by(|left, right| {
                    compare(
                        left,
                        right,
                        left["observedReturnAt"].as_i64(),
                        right["observedReturnAt"].as_i64(),
                        order,
                        &collator,
                    )
                });
                let result = json!(
                    rows.iter()
                        .map(|value| vec![value["environmentId"].clone(), value["id"].clone()])
                        .collect::<Vec<_>>()
                );
                assert_eq!(result, case["expected"][name], "{name}: {case}");
            }
        }
    }
}

#[derive(Clone, Copy)]
pub enum Order {
    Pinned,
    Active,
    Inbox,
    Working,
    Snoozed,
    Settled,
}

pub fn collator() -> &'static icu_collator::CollatorBorrowed<'static> {
    static COLLATOR: std::sync::OnceLock<icu_collator::CollatorBorrowed<'static>> =
        std::sync::OnceLock::new();
    COLLATOR.get_or_init(|| {
        let locale = sys_locale::get_locale()
            .as_deref()
            .and_then(crate::timestamp::locale_tag)
            .unwrap_or_else(|| "en-US".parse().expect("valid default locale"));
        icu_collator::Collator::try_new(locale.into(), Default::default())
            .expect("load compiled collation data")
    })
}

fn timestamp(value: &Value) -> Option<i64> {
    value
        .as_str()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp_millis())
}

fn newest(values: &[&Value], extra: Option<i64>) -> i64 {
    values
        .iter()
        .map(|value| timestamp(value).unwrap_or(0))
        .chain(extra)
        .max()
        .unwrap_or(0)
}

fn text<'a>(thread: &'a Value, field: &str) -> &'a str {
    thread[field].as_str().unwrap_or_default()
}

fn activity(thread: &Value) -> i64 {
    [
        &thread["latestUserMessageAt"],
        &thread["latestRun"]["requestedAt"],
        &thread["latestRun"]["startedAt"],
        &thread["latestRun"]["completedAt"],
    ]
    .into_iter()
    .filter_map(timestamp)
    .max()
    .or_else(|| timestamp(&thread["updatedAt"]))
    .unwrap_or(0)
}

pub fn compare(
    left: &Value,
    right: &Value,
    left_returned_at: Option<i64>,
    right_returned_at: Option<i64>,
    order: Order,
    collator: &icu_collator::CollatorBorrowed<'_>,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let identity = || {
        collator
            .compare(text(left, "id"), text(right, "id"))
            .then_with(|| {
                collator.compare(text(left, "environmentId"), text(right, "environmentId"))
            })
    };
    let newest_first = |get: &dyn Fn(&Value, Option<i64>) -> i64| {
        get(right, right_returned_at)
            .cmp(&get(left, left_returned_at))
            .then_with(identity)
    };
    let ordered =
        |field: &str, keyless_first: bool| match (left[field].as_str(), right[field].as_str()) {
            (Some(left), Some(right)) => left
                .encode_utf16()
                .cmp(right.encode_utf16())
                .then_with(identity),
            (Some(_), None) => {
                if keyless_first {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
            (None, Some(_)) => {
                if keyless_first {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            }
            (None, None) => newest_first(&|thread, _| {
                if keyless_first {
                    newest(&[&thread["createdAt"], &thread["unsettledAt"]], None)
                } else {
                    timestamp(&thread["createdAt"]).unwrap_or(0)
                }
            }),
        };
    match order {
        Order::Pinned => ordered("pinOrderKey", false),
        Order::Active => ordered("activeOrderKey", true),
        Order::Inbox => newest_first(&|thread, returned| {
            newest(
                &[
                    &thread["createdAt"],
                    &thread["unsettledAt"],
                    &thread["latestRun"]["requestedAt"],
                    &thread["latestRun"]["completedAt"],
                ],
                Some(returned.unwrap_or(0)),
            )
        }),
        Order::Working => newest_first(&|thread, _| {
            newest(
                &[
                    &thread["createdAt"],
                    thread
                        .get("latestUserAuthoredMessageAt")
                        .unwrap_or(&thread["latestRun"]["requestedAt"]),
                ],
                None,
            )
        }),
        Order::Snoozed => timestamp(&left["snoozedUntil"])
            .unwrap_or(0)
            .cmp(&timestamp(&right["snoozedUntil"]).unwrap_or(0)),
        Order::Settled => timestamp(&right["settledAt"])
            .unwrap_or_else(|| activity(right))
            .cmp(&timestamp(&left["settledAt"]).unwrap_or_else(|| activity(left)))
            .then_with(|| {
                text(left, "id")
                    .encode_utf16()
                    .cmp(text(right, "id").encode_utf16())
            }),
    }
}
