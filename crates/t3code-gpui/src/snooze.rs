use chrono::{
    DateTime, Datelike, Duration, LocalResult, NaiveDateTime, Offset, SecondsFormat, TimeZone,
    Timelike, Utc,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn millis(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|date| date.timestamp_millis())
}

fn after(left: Option<&str>, right: Option<&str>) -> bool {
    left.and_then(millis)
        .zip(right.and_then(millis))
        .is_some_and(|(left, right)| left > right)
}

fn non_null(value: &Value, key: &str) -> bool {
    value.get(key).is_some_and(|value| !value.is_null())
}

fn runtime(shell: &Value) -> &Value {
    shell
        .get("runtime")
        .filter(|value| !value.is_null())
        .unwrap_or(&shell["session"])
}

fn latest_run(shell: &Value) -> &Value {
    shell
        .get("latestRun")
        .filter(|value| !value.is_null())
        .unwrap_or(&shell["latestTurn"])
}

pub fn queued(shell: &Value, now: &str) -> bool {
    if matches!(
        shell.pointer("/runtime/status").and_then(Value::as_str),
        Some("preparing" | "queued" | "starting")
    ) {
        return true;
    }
    if shell.pointer("/session/status").and_then(Value::as_str) == Some("error") {
        return false;
    }
    let Some((message, now)) = text(shell, "latestUserMessageAt")
        .and_then(millis)
        .zip(millis(now))
    else {
        return false;
    };
    if (now - message).abs() > 120_000 {
        return false;
    }
    let run = latest_run(shell);
    run.is_null()
        || ["requestedAt", "startedAt", "completedAt"]
            .into_iter()
            .all(|key| {
                !non_null(run, key)
                    || text(run, key)
                        .and_then(millis)
                        .is_some_and(|time| time < message)
            })
}

pub fn can_snooze(shell: &Value, now: &str) -> bool {
    let request = &shell["source"]["pendingRuntimeRequest"];
    shell["hasPendingApprovals"] != true
        && (request.is_null() || text(request, "kind") == Some("user_input"))
        && !non_null(shell, "pendingSubagentRequest")
        && shell["pendingGuestMessageCount"]
            .as_f64()
            .unwrap_or_default()
            <= 0.
        && !queued(shell, now)
}

fn question_at(shell: &Value) -> Option<&str> {
    text(shell, "pendingUserInputAt").or_else(|| {
        let request = &shell["source"]["pendingRuntimeRequest"];
        (text(request, "kind") == Some("user_input"))
            .then(|| text(request, "createdAt"))
            .flatten()
    })
}

fn failure_at(shell: &Value) -> Option<Option<&str>> {
    let runtime = runtime(shell);
    if !matches!(text(runtime, "status"), Some("error" | "failed")) {
        return None;
    }
    let run = latest_run(shell);
    Some(if text(run, "status") == Some("failed") {
        text(run, "completedAt")
    } else {
        text(runtime, "updatedAt")
    })
}

pub fn raised_hand(shell: &Value) -> bool {
    let request = &shell["source"]["pendingRuntimeRequest"];
    if shell["hasPendingApprovals"] == true
        || (!request.is_null() && text(request, "kind") != Some("user_input"))
    {
        return true;
    }
    let snoozed = text(shell, "snoozedAt");
    let question = question_at(shell);
    if (shell["hasPendingUserInput"] == true || text(request, "kind") == Some("user_input"))
        && (snoozed.is_none()
            || question.is_none()
            || !question
                .and_then(millis)
                .zip(snoozed.and_then(millis))
                .is_some_and(|(question, snoozed)| question <= snoozed))
    {
        return true;
    }
    if non_null(shell, "pendingSubagentRequest")
        || shell["pendingGuestMessageCount"]
            .as_f64()
            .unwrap_or_default()
            > 0.
    {
        return true;
    }
    failure_at(shell).is_some_and(|failed| snoozed.is_none() || after(failed, snoozed))
}

pub fn effective(shell: &Value, now: &str) -> bool {
    let Some(wake) = text(shell, "snoozedUntil").and_then(millis) else {
        return false;
    };
    !millis(now).is_some_and(|now| wake <= now) && !raised_hand(shell)
}

pub fn woke_at(shell: &Value, now: &str) -> Option<String> {
    let wake = text(shell, "snoozedUntil")?;
    let wake_ms = millis(wake)?;
    if raised_hand(shell) {
        let failed = failure_at(shell).flatten();
        let question = question_at(shell);
        let snoozed = text(shell, "snoozedAt");
        let fresh_failure = failed.is_some() && (snoozed.is_none() || after(failed, snoozed));
        let fresh_question = question.is_some() && snoozed.is_some() && after(question, snoozed);
        if fresh_failure
            && (!fresh_question
                || failed
                    .and_then(millis)
                    .zip(question.and_then(millis))
                    .is_some_and(|(failed, question)| failed >= question))
        {
            return failed.map(str::to_string);
        }
        if fresh_question {
            return question.map(str::to_string);
        }
        return text(runtime(shell), "updatedAt")
            .or(snoozed)
            .map(str::to_string);
    }
    millis(now)
        .filter(|now| wake_ms <= *now)
        .map(|_| wake.to_string())
}

pub fn woke_unseen(
    woke: Option<&str>,
    settled: bool,
    visited: Option<&str>,
    completed: Option<&str>,
) -> bool {
    let Some(woke) = woke.and_then(millis).filter(|_| !settled) else {
        return false;
    };
    [visited, completed]
        .into_iter()
        .flatten()
        .filter_map(millis)
        .all(|seen| seen < woke)
}

pub fn wake_label(wake: &str, now: &str) -> String {
    let Some((wake, now)) = millis(wake).zip(millis(now)) else {
        return "now".into();
    };
    let remaining = wake - now;
    match remaining {
        ..=0 => "now".into(),
        1..3_600_000 => format!("{}m", (remaining + 59_999) / 60_000),
        3_600_000..86_400_000 => format!("{}h", (remaining + 3_599_999) / 3_600_000),
        _ => format!("{}d", (remaining + 86_399_999) / 86_400_000),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Setting {
    After { minutes: i64 },
    At { day: String, time: String },
}

pub fn settings(value: Option<&Value>) -> Vec<Setting> {
    value.and_then(|value| serde_json::from_value(value.clone()).ok()).unwrap_or_else(|| serde_json::from_value(json!([
        {"kind":"after","minutes":60},{"kind":"after","minutes":180},{"kind":"at","day":"today","time":"18:00"},{"kind":"at","day":"tomorrow","time":"09:00"},{"kind":"at","day":"monday","time":"09:00"}
    ])).unwrap())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub id: String,
    pub label: String,
    pub when_label: String,
    pub snoozed_until: String,
}

fn compatible_local<Tz: TimeZone>(timezone: &Tz, local: NaiveDateTime) -> Option<DateTime<Tz>> {
    match timezone.from_local_datetime(&local) {
        LocalResult::Single(value) => Some(value),
        LocalResult::Ambiguous(first, second) => Some(first.min(second)),
        LocalResult::None => {
            for minutes in 1..=2880 {
                let prior = local.checked_sub_signed(Duration::minutes(minutes))?;
                if let Some(before) = timezone.from_local_datetime(&prior).earliest() {
                    let utc = local.checked_sub_signed(Duration::seconds(
                        before.offset().fix().local_minus_utc() as i64,
                    ))?;
                    return Some(timezone.from_utc_datetime(&utc));
                }
            }
            None
        }
    }
}

pub fn preset_id(setting: &Setting) -> String {
    match setting {
        Setting::After { minutes } => format!("after:{minutes}"),
        Setting::At { day, time } => format!("at:{day}:{time}"),
    }
}

fn capitalize(value: &str) -> String {
    let mut letters = value.chars();
    letters
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + letters.as_str())
        .unwrap_or_default()
}

fn duration_label(minutes: i64) -> String {
    let (amount, unit) = if minutes % 1440 == 0 {
        (minutes / 1440, "day")
    } else if minutes % 60 == 0 {
        (minutes / 60, "hour")
    } else {
        (minutes, "minute")
    };
    format!("{amount} {unit}{}", if amount == 1 { "" } else { "s" })
}

pub fn resolve<Tz: TimeZone>(
    setting: &Setting,
    now: &DateTime<Tz>,
    format_time: &impl Fn(&DateTime<Tz>) -> String,
    format_weekday: &impl Fn(&DateTime<Tz>) -> String,
) -> Option<Preset> {
    let (wake, label, names_day) = match setting {
        Setting::After { minutes } => (
            now.clone()
                .checked_add_signed(Duration::minutes(*minutes))?,
            format!("In {}", duration_label(*minutes)),
            false,
        ),
        Setting::At { day, time } => {
            let target = [
                "sunday",
                "monday",
                "tuesday",
                "wednesday",
                "thursday",
                "friday",
                "saturday",
            ]
            .iter()
            .position(|name| *name == day);
            let days = match day.as_str() {
                "today" => 0,
                "tomorrow" => 1,
                _ => {
                    let gap =
                        (target? as i64 - now.weekday().num_days_from_sunday() as i64 + 7) % 7;
                    if gap == 0 { 7 } else { gap }
                }
            };
            let advanced = compatible_local(
                &now.timezone(),
                now.naive_local().checked_add_signed(Duration::days(days))?,
            )?;
            let (hour, minute) = time.split_once(':')?;
            let local =
                advanced
                    .date_naive()
                    .and_hms_opt(hour.parse().ok()?, minute.parse().ok()?, 0)?;
            let wake = compatible_local(&now.timezone(), local)?;
            if wake <= *now {
                return None;
            }
            let label = match days {
                0 => if wake.hour() < 12 {
                    "This morning"
                } else if wake.hour() < 17 {
                    "This afternoon"
                } else {
                    "This evening"
                }
                .to_string(),
                1 => "Tomorrow".to_string(),
                _ if day == "monday" => "Next week".to_string(),
                7 => format!("Next {}", capitalize(day)),
                _ => capitalize(day),
            };
            let names_day = label != "Next week";
            (wake, label, names_day)
        }
    };
    let time = format_time(&wake);
    let when_label = if names_day || wake.date_naive() == now.date_naive() {
        time
    } else {
        format!("{} {time}", format_weekday(&wake))
    };
    Some(Preset {
        id: preset_id(setting),
        label,
        when_label,
        snoozed_until: wake
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Millis, true),
    })
}

pub fn presets<Tz: TimeZone>(
    settings: &[Setting],
    now: &DateTime<Tz>,
    format_time: &impl Fn(&DateTime<Tz>) -> String,
    format_weekday: &impl Fn(&DateTime<Tz>) -> String,
) -> Vec<Preset> {
    let mut presets: Vec<_> = settings
        .iter()
        .filter_map(|setting| resolve(setting, now, format_time, format_weekday))
        .collect();
    presets.sort_by_key(|preset| millis(&preset.snoozed_until).unwrap().div_euclid(60_000));
    let mut seen = std::collections::HashSet::new();
    presets.retain(|preset| seen.insert(millis(&preset.snoozed_until).unwrap().div_euclid(60_000)));
    presets
}

pub fn pick<Tz: TimeZone>(
    shown: &[Preset],
    id: &str,
    settings: &[Setting],
    now: &DateTime<Tz>,
    format_time: &impl Fn(&DateTime<Tz>) -> String,
    format_weekday: &impl Fn(&DateTime<Tz>) -> String,
) -> Option<Preset> {
    settings
        .iter()
        .find(|setting| matches!(setting, Setting::After { .. }) && preset_id(setting) == id)
        .and_then(|setting| resolve(setting, now, format_time, format_weekday))
        .or_else(|| {
            shown
                .iter()
                .find(|preset| {
                    preset.id == id
                        && millis(&preset.snoozed_until)
                            .is_some_and(|wake| wake > now.timestamp_millis())
                })
                .cloned()
        })
}

fn js_number(value: &str) -> Option<f64> {
    let value = value.trim_matches(|character: char| {
        (character.is_whitespace() && character != '\u{0085}') || character == '\u{feff}'
    });
    if value.is_empty() {
        return Some(0.);
    }
    for (prefix, radix) in [
        ("0x", 16),
        ("0X", 16),
        ("0b", 2),
        ("0B", 2),
        ("0o", 8),
        ("0O", 8),
    ] {
        if let Some(number) = value.strip_prefix(prefix) {
            return u64::from_str_radix(number, radix)
                .ok()
                .map(|value| value as f64);
        }
    }
    value.parse().ok()
}

pub fn custom<Tz: TimeZone>(input: &Value, now: &DateTime<Tz>) -> Option<String> {
    let wake = if text(input, "mode") == Some("duration") {
        let amount = js_number(text(input, "amount")?)?;
        if !amount.is_finite() || amount <= 0. {
            return None;
        }
        let unit = match text(input, "unit")? {
            "minutes" => 60_000.,
            "hours" => 3_600_000.,
            "days" => 86_400_000.,
            _ => return None,
        };
        let wake = now.timestamp_millis() as f64 + amount * unit;
        if !wake.is_finite() || wake.abs() > 8.64e15 {
            return None;
        }
        Utc.timestamp_millis_opt(wake.trunc() as i64)
            .single()?
            .with_timezone(&now.timezone())
    } else {
        let date = text(input, "date")?;
        let time = text(input, "time")?;
        if date.len() != 10
            || time.len() != 5
            || date.as_bytes()[4] != b'-'
            || date.as_bytes()[7] != b'-'
            || time.as_bytes()[2] != b':'
        {
            return None;
        }
        if !date
            .bytes()
            .enumerate()
            .all(|(index, value)| matches!(index, 4 | 7) || value.is_ascii_digit())
            || !time
                .bytes()
                .enumerate()
                .all(|(index, value)| index == 2 || value.is_ascii_digit())
        {
            return None;
        }
        let naive =
            NaiveDateTime::parse_from_str(&format!("{date}T{time}:00"), "%Y-%m-%dT%H:%M:%S")
                .ok()?;
        let wake = compatible_local(&now.timezone(), naive)?;
        if wake.naive_local() != naive {
            return None;
        }
        wake
    };
    (wake > *now).then(|| {
        wake.with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Millis, true)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_gates_early_wakes_deadlines_and_unseen_flags_match_the_original() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/snooze-oracle.json")).unwrap();
        for case in fixture["states"].as_array().unwrap() {
            let input = &case["input"];
            let now = case["now"].as_str().unwrap();
            let woke = woke_at(input, now);
            let actual = json!({
                "queued":queued(input,now),"canSnooze":can_snooze(input,now),"raisedHand":raised_hand(input),"effective":effective(input,now),
                "wokeAt":woke,"wakeLabel":wake_label(text(input,"snoozedUntil").unwrap_or_default(),now),
                "unseen":woke_unseen(woke.as_deref(),input["settled"]==true,text(input,"lastVisitedAt"),input.pointer("/latestRun/completedAt").and_then(Value::as_str)),
            });
            assert_eq!(actual, case["expected"], "{case}");
        }
    }

    #[test]
    fn presets_and_their_selection_match_the_original_across_calendar_boundaries() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/snooze-oracle.json")).unwrap();
        for case in fixture["schedules"].as_array().unwrap() {
            let zone: chrono_tz::Tz = case["zone"].as_str().unwrap().parse().unwrap();
            let now = DateTime::parse_from_rfc3339(case["now"].as_str().unwrap())
                .unwrap()
                .with_timezone(&zone);
            let settings = settings(case.get("settings"));
            let time = |value: &DateTime<chrono_tz::Tz>| value.format("%H:%M").to_string();
            let weekday = |value: &DateTime<chrono_tz::Tz>| value.format("%a").to_string();
            let actual = presets(&settings, &now, &time, &weekday);
            assert_eq!(
                serde_json::to_value(&actual).unwrap(),
                case["expected"],
                "{case}"
            );
            for choice in case["picks"].as_array().unwrap() {
                let now = DateTime::parse_from_rfc3339(choice["now"].as_str().unwrap())
                    .unwrap()
                    .with_timezone(&zone);
                let selected = pick(
                    &actual,
                    choice["id"].as_str().unwrap(),
                    &settings,
                    &now,
                    &time,
                    &weekday,
                );
                assert_eq!(
                    serde_json::to_value(selected).unwrap(),
                    choice["expected"],
                    "{case}: {choice}"
                );
            }
        }
    }

    #[test]
    fn custom_dates_and_fractional_durations_match_the_original_across_dst() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/snooze-oracle.json")).unwrap();
        for case in fixture["customs"].as_array().unwrap() {
            let zone: chrono_tz::Tz = case["zone"].as_str().unwrap().parse().unwrap();
            let now = DateTime::parse_from_rfc3339(case["now"].as_str().unwrap())
                .unwrap()
                .with_timezone(&zone);
            assert_eq!(
                custom(&case["input"], &now).as_deref(),
                case["expected"].as_str(),
                "{case}"
            );
        }
    }
}
