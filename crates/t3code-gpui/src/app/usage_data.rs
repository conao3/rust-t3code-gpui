use chrono::{DateTime, Datelike, Days, Duration, Offset, TimeZone, Timelike};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

pub(super) fn make_window<T: TimeZone>(days: u32, now: DateTime<T>, zone: &str) -> Value
where
    T::Offset: std::fmt::Display,
{
    if days == 1 {
        let until = now
            .with_second(0)
            .expect("minute")
            .with_nanosecond(0)
            .expect("minute");
        let since = until.clone() - Duration::hours(24);
        json!({"sinceDay":since.format("%Y-%m-%d").to_string(),"untilDay":until.format("%Y-%m-%d").to_string(),"timeZone":zone,"resolution":"hour","sinceTime":since.with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Millis,true),"untilTime":until.with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Millis,true)})
    } else {
        let until = now.date_naive();
        let since = until
            .checked_sub_days(Days::new(u64::from(days - 1)))
            .expect("usage date");
        json!({"sinceDay":since.to_string(),"untilDay":until.to_string(),"timeZone":zone,"resolution":"day"})
    }
}

pub(super) fn count(value: f64) -> String {
    let value = (value + 0.5).floor() as i64;
    let digits = value.unsigned_abs().to_string();
    let mut result = String::new();
    if value < 0 {
        result.push('-');
    }
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(ch);
    }
    result
}

pub(super) fn hour_label(source: &str, zone: chrono_tz::Tz, hour12: bool) -> String {
    let Ok(value) = DateTime::parse_from_rfc3339(source) else {
        return source.to_string();
    };
    let value = value.with_timezone(&zone);
    let repeated = [-1, 1].into_iter().any(|hours| {
        let other = value + Duration::hours(hours);
        value.date_naive() == other.date_naive() && value.hour() == other.hour()
    });
    let hour = if hour12 {
        value.format("%-I %p").to_string()
    } else {
        format!("{:02}:00", value.hour())
    };
    if repeated {
        use icu_datetime::NoCalendarFormatter;
        use icu_datetime::fieldsets::zone::SpecificShort;
        use icu_datetime::input::{
            Date, DateTime as IcuDateTime, Time, TimeZone as IcuZone, UtcOffset,
        };
        thread_local! {
            static FORMATTER: NoCalendarFormatter<SpecificShort> = NoCalendarFormatter::try_new(
                "en-US"
                    .parse::<icu_locale_core::Locale>()
                    .expect("usage locale")
                    .into(),
                SpecificShort,
            )
            .expect("usage zone formatter");
        }
        let zone = IcuZone::from_iana_id(zone.name())
            .with_offset(Some(
                UtcOffset::try_from_seconds(value.offset().fix().local_minus_utc())
                    .expect("zone offset"),
            ))
            .at_date_time(IcuDateTime {
                date: Date::try_new_gregorian(value.year(), value.month() as u8, value.day() as u8)
                    .expect("usage date"),
                time: Time::try_new(
                    value.hour() as u8,
                    value.minute() as u8,
                    value.second() as u8,
                    0,
                )
                .expect("usage time"),
            });
        FORMATTER.with(|formatter| format!("{hour} {}", formatter.format(&zone)))
    } else {
        hour
    }
}

pub(super) fn usd(value: f64) -> String {
    let text = value.abs().to_string();
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let mut digits = whole
        .bytes()
        .chain(fraction.bytes().chain(std::iter::repeat(b'0')).take(2))
        .collect::<Vec<_>>();
    if fraction
        .as_bytes()
        .get(2)
        .is_some_and(|digit| *digit >= b'5')
    {
        let mut carry = true;
        for digit in digits.iter_mut().rev() {
            if *digit == b'9' {
                *digit = b'0';
            } else {
                *digit += 1;
                carry = false;
                break;
            }
        }
        if carry {
            digits.insert(0, b'1');
        }
    }
    let split = digits.len() - 2;
    let mut result = if value.is_sign_negative() {
        "-$".to_string()
    } else {
        "$".to_string()
    };
    for (index, digit) in digits[..split].iter().enumerate() {
        if index > 0 && (split - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(*digit as char);
    }
    result.push('.');
    result.extend(digits[split..].iter().map(|digit| *digit as char));
    result
}

fn fixed(value: f64, digits: usize) -> String {
    let bits = value.abs().to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023 - 52;
    if exponent >= 0 {
        return format!("{value:.digits$}");
    }
    let mantissa = u128::from((bits & ((1u64 << 52) - 1)) | (1u64 << 52));
    let numerator = mantissa * 10u128.pow(digits as u32);
    let denominator = 1u128 << (-exponent);
    let rounded =
        numerator / denominator + u128::from((numerator % denominator) * 2 >= denominator);
    let scale = 10u128.pow(digits as u32);
    if digits == 0 {
        format!("{}{rounded}", if value < 0. { "-" } else { "" })
    } else {
        format!(
            "{}{}.{:0width$}",
            if value < 0. { "-" } else { "" },
            rounded / scale,
            rounded % scale,
            width = digits
        )
    }
}

pub(super) fn tokens(value: f64) -> String {
    for (factor, suffix) in [(1e12, "T"), (1e9, "B"), (1e6, "M"), (1e3, "K")] {
        if value.abs() >= factor {
            let value = value / factor;
            let digits = if value.abs() >= 100. {
                0
            } else if value.abs() >= 10. {
                1
            } else {
                2
            };
            let text = fixed(value, digits);
            let text = if text.contains('.')
                && text
                    .split('.')
                    .nth(1)
                    .is_some_and(|fraction| fraction.chars().all(|ch| ch == '0'))
            {
                text.split('.').next().expect("whole")
            } else {
                &text
            };
            return format!("{text}{suffix}");
        }
    }
    count(value)
}

pub(super) fn number(value: &Value, key: &str) -> f64 {
    value[key].as_f64().unwrap_or(0.)
}
pub(super) fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}
fn items<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key].as_array().map(Vec::as_slice).unwrap_or_default()
}
fn fingerprint(source: &Value) -> String {
    json!(
        ["hostId", "provider", "resolvedHomePath", "volumeId"]
            .map(|key| text(&source["fingerprint"], key))
    )
    .to_string()
}
fn timestamp(value: &Value) -> i64 {
    DateTime::parse_from_rfc3339(text(value, "readAt"))
        .map(|date| date.timestamp_millis())
        .unwrap_or(0)
}
fn cell(bucket: &Value) -> String {
    json!([
        bucket["day"],
        bucket["hourStart"],
        bucket["provider"],
        bucket["model"]
    ])
    .to_string()
}
fn source_buckets<'a>(summary: &'a Value, source: &Value) -> Vec<&'a Value> {
    let provider = &source["fingerprint"]["provider"];
    let single = items(summary, "sources")
        .iter()
        .filter(|candidate| candidate["fingerprint"]["provider"] == *provider)
        .count()
        == 1;
    items(summary, "buckets")
        .iter()
        .filter(|bucket| {
            bucket["provider"] == *provider
                && (bucket["sourcePath"] == source["fingerprint"]["resolvedHomePath"]
                    || bucket["sourcePath"].is_null() && single)
        })
        .collect()
}

#[derive(Clone, Default)]
pub(super) struct Totals {
    pub cost: f64,
    pub tokens: [f64; 5],
    pub category_cost: [f64; 4],
    pub speed_cost: [f64; 3],
    pub records: f64,
    pub unpriced: f64,
    pub sessions: f64,
    pub cache_savings: f64,
}
impl Totals {
    pub fn total_tokens(&self) -> f64 {
        self.tokens[..4].iter().sum()
    }
    fn add(&mut self, bucket: &Value) {
        self.cost += number(bucket, "costUsd");
        for (index, key) in [
            "uncachedInputTokens",
            "cachedInputTokens",
            "cacheCreationTokens",
            "outputTokens",
            "reasoningTokens",
        ]
        .iter()
        .enumerate()
        {
            self.tokens[index] += number(&bucket["totals"], key);
        }
        for (index, key) in ["input", "cacheRead", "cacheWrite", "output"]
            .iter()
            .enumerate()
        {
            self.category_cost[index] += number(&bucket["categoryCostUsd"], key);
        }
        for (index, key) in ["fastCostUsd", "ultrafastCostUsd", "speedPremiumUsd"]
            .iter()
            .enumerate()
        {
            self.speed_cost[index] += number(bucket, key);
        }
        self.records += number(bucket, "records");
        self.unpriced += number(bucket, "unpricedRecords");
        self.cache_savings += number(bucket, "cacheSavingsUsd");
    }
}

#[derive(Default)]
pub(super) struct Merged {
    pub totals: Totals,
    pub providers: BTreeMap<String, Totals>,
    pub models: BTreeMap<(String, String), Totals>,
    pub periods: BTreeMap<String, BTreeMap<String, Totals>>,
    pub warnings: Vec<String>,
}

pub(super) fn merge(environments: &[(String, Value)], hidden: &HashSet<String>) -> Merged {
    let mut result = Merged::default();
    let mut current=environments.iter().filter(|(label,summary)| {
        let version=number(summary,"contractVersion");
        if !(4. ..=6.).contains(&version) {
            result.warnings.push(if version<4. {format!("{label} runs an older server version and is excluded from totals.")} else {format!("This client is older than the server on {label}; its usage is excluded from totals.")});false
        } else {true}
    }).collect::<Vec<_>>();
    current.sort_by(|(a_id, a), (b_id, b)| {
        timestamp(b).cmp(&timestamp(a)).then_with(|| a_id.cmp(b_id))
    });
    let mut owners = HashMap::<String, (usize, usize, f64)>::new();
    for status in ["ok", "partial", "failed"] {
        for (environment_index, (label, summary)) in current.iter().enumerate() {
            for (source_index, source) in items(summary, "sources").iter().enumerate() {
                if text(source, "status") != status
                    || hidden.contains(text(&source["fingerprint"], "provider"))
                {
                    continue;
                }
                let key = fingerprint(source);
                match owners.entry(key) {
                    std::collections::hash_map::Entry::Occupied(_) => {
                        result.warnings.push(format!(
                            "Duplicate source: {label}: {}",
                            text(&source["fingerprint"], "resolvedHomePath")
                        ))
                    }
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert((
                            environment_index,
                            source_index,
                            number(source, "distinctSessions"),
                        ));
                    }
                }
            }
        }
    }
    let mut supplemental = HashSet::<(usize, usize)>::new();
    let mut seen = HashMap::<String, HashSet<String>>::new();
    for (environment_index, (_, summary)) in current.iter().enumerate() {
        for source in items(summary, "sources") {
            if text(source, "status") != "partial" {
                continue;
            }
            let key = fingerprint(source);
            let Some((owner_index, source_index, sessions)) = owners.get_mut(&key) else {
                continue;
            };
            let owner = &current[*owner_index].1;
            let owner_source = &items(owner, "sources")[*source_index];
            if owner_source["status"] != "ok"
                || timestamp(summary) <= timestamp(owner)
                || summary["modelFilter"] != owner["modelFilter"]
            {
                continue;
            }
            let cells = seen.entry(key).or_insert_with(|| {
                source_buckets(owner, owner_source)
                    .into_iter()
                    .map(cell)
                    .collect()
            });
            let buckets = source_buckets(summary, source);
            let mut added = false;
            for (index, bucket) in items(summary, "buckets").iter().enumerate() {
                if buckets
                    .iter()
                    .any(|source_bucket| std::ptr::eq(*source_bucket, bucket))
                    && cells.insert(cell(bucket))
                {
                    supplemental.insert((environment_index, index));
                    added = true;
                }
            }
            if added {
                *sessions = sessions.max(number(source, "distinctSessions"));
            }
        }
    }
    for (environment_index, (_, summary)) in current.iter().enumerate() {
        let mut owned_providers = HashSet::new();
        let mut owned_paths = HashSet::new();
        for source in items(summary, "sources") {
            let provider = text(&source["fingerprint"], "provider");
            if hidden.contains(provider) {
                continue;
            }
            if let Some(message) = source["message"].as_str().filter(|_| {
                matches!(text(source, "status"), "partial" | "failed") || provider == "cursor"
            }) {
                result.warnings.push(message.to_string());
            }
            if let Some((owner_index, _, sessions)) = owners.get(&fingerprint(source))
                && *owner_index == environment_index
                && source["status"] != "missing"
            {
                owned_providers.insert(provider);
                owned_paths.insert((provider, text(&source["fingerprint"], "resolvedHomePath")));
                result.totals.sessions += sessions;
                result
                    .providers
                    .entry(provider.into())
                    .or_default()
                    .sessions += sessions;
            }
        }
        for (index, bucket) in items(summary, "buckets").iter().enumerate() {
            let provider = text(bucket, "provider");
            if hidden.contains(provider) {
                continue;
            }
            let owned = if let Some(path) = bucket["sourcePath"].as_str() {
                owned_paths.contains(&(provider, path))
            } else {
                owned_providers.contains(provider)
            };
            if !owned && !supplemental.contains(&(environment_index, index)) {
                continue;
            }
            result.totals.add(bucket);
            result
                .providers
                .entry(provider.into())
                .or_default()
                .add(bucket);
            result
                .models
                .entry((provider.into(), text(bucket, "model").into()))
                .or_default()
                .add(bucket);
            let period = bucket["hourStart"]
                .as_str()
                .unwrap_or_else(|| text(bucket, "day"));
            result
                .periods
                .entry(period.into())
                .or_default()
                .entry(provider.into())
                .or_default()
                .add(bucket);
        }
    }
    result.warnings.sort();
    result.warnings.dedup();
    result
}

pub(super) fn chart_scale(peak: f64) -> (f64, Vec<f64>) {
    if peak <= 0. {
        return (0., vec![0.]);
    }
    let raw = peak / 4.;
    let magnitude = 10_f64.powf(raw.log10().floor());
    let normalized = raw / magnitude;
    let step = if normalized > 5. {
        10.
    } else if normalized > 2. {
        5.
    } else if normalized > 1. {
        2.
    } else {
        1.
    } * magnitude;
    let max = (peak / step).ceil() * step;
    (
        max,
        (0..=(max / step).round() as usize)
            .map(|index| index as f64 * step)
            .collect(),
    )
}

pub(super) fn chart_tangents(points: &[(f32, f32)]) -> Vec<f32> {
    if points.len() < 2 {
        return vec![0.; points.len()];
    }
    let slopes = points
        .windows(2)
        .map(|pair| {
            let dx = pair[1].0 - pair[0].0;
            if dx == 0. {
                0.
            } else {
                (pair[1].1 - pair[0].1) / dx
            }
        })
        .collect::<Vec<_>>();
    let mut tangents = vec![0.; points.len()];
    tangents[0] = slopes[0];
    tangents[points.len() - 1] = slopes[slopes.len() - 1];
    for index in 1..points.len() - 1 {
        tangents[index] = if slopes[index - 1] * slopes[index] <= 0. {
            0.
        } else {
            (slopes[index - 1] + slopes[index]) / 2.
        };
    }
    for (index, slope) in slopes.into_iter().enumerate() {
        if slope == 0. {
            tangents[index] = 0.;
            tangents[index + 1] = 0.;
            continue;
        }
        let a = tangents[index] / slope;
        let b = tangents[index + 1] / slope;
        let magnitude = a * a + b * b;
        if magnitude > 9. {
            let scale = 3. / magnitude.sqrt();
            tangents[index] = scale * a * slope;
            tangents[index + 1] = scale * b * slope;
        }
    }
    tangents
}

pub(super) fn percent(share: f64) -> String {
    let value = share * 100.;
    if value > 0. && value < 0.1 {
        "<0.1%".into()
    } else {
        format!("{value:.1}%")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn hourly_labels_distinguish_repeated_hours_and_follow_clock_format() {
        let zone = chrono_tz::America::New_York;
        for (source, hour12, expected) in [
            ("2026-11-01T05:37:00Z", true, "1 AM EDT"),
            ("2026-11-01T06:37:00Z", true, "1 AM EST"),
            ("2026-11-01T05:37:00Z", false, "01:00 EDT"),
            ("2026-11-01T06:37:00Z", false, "01:00 EST"),
            ("2026-11-01T07:37:00Z", false, "02:00"),
            ("2026-11-01T17:37:00Z", true, "12 PM"),
            ("invalid", false, "invalid"),
        ] {
            assert_eq!(super::hour_label(source, zone, hour12), expected);
        }
        for (source, expected) in [
            ("2026-10-25T00:37:00Z", "02:00 GMT+2"),
            ("2026-10-25T01:37:00Z", "02:00 GMT+1"),
        ] {
            assert_eq!(
                super::hour_label(source, chrono_tz::Europe::Berlin, false),
                expected
            );
        }
    }
    #[test]
    fn unpriced_share_retains_small_positive_values_and_decimal_precision() {
        for (share, expected) in [
            (0., "0.0%"),
            (0.00001, "<0.1%"),
            (0.001, "0.1%"),
            (0.125, "12.5%"),
            (1., "100.0%"),
        ] {
            assert_eq!(super::percent(share), expected);
        }
    }

    use super::*;

    #[test]
    fn chart_matches_pinned_reference() {
        let oracle: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/usage-chart-oracle.json"))
                .unwrap();
        for row in oracle["scales"].as_array().unwrap() {
            let (max, ticks) = chart_scale(row["peak"].as_f64().unwrap());
            assert!((max - row["max"].as_f64().unwrap()).abs() < 1e-8);
            let expected = row["ticks"].as_array().unwrap();
            assert_eq!(ticks.len(), expected.len());
            for (actual, expected) in ticks.iter().zip(expected) {
                assert!((actual - expected.as_f64().unwrap()).abs() < 1e-8);
            }
        }
        for row in oracle["curves"].as_array().unwrap() {
            let points = row["points"]
                .as_array()
                .unwrap()
                .iter()
                .map(|pair| {
                    (
                        pair[0].as_f64().unwrap() as f32,
                        pair[1].as_f64().unwrap() as f32,
                    )
                })
                .collect::<Vec<_>>();
            let tangents = chart_tangents(&points);
            let expected = row["tangents"].as_array().unwrap();
            assert_eq!(tangents.len(), expected.len());
            for (actual, expected) in tangents.iter().zip(expected) {
                assert!((f64::from(*actual) - expected.as_f64().unwrap()).abs() < 1e-5);
            }
        }
    }
    #[test]
    fn windows_match_calendar_days_and_rolling_minutes() {
        let now = DateTime::parse_from_rfc3339("2026-11-18T01:12:34.567+09:00").unwrap();
        let week = make_window(7, now, "Asia/Tokyo");
        assert_eq!(week["sinceDay"], "2026-11-12");
        assert_eq!(week["untilDay"], "2026-11-18");
        let day = make_window(1, now, "Asia/Tokyo");
        assert_eq!(day["sinceTime"], "2026-11-16T16:12:00.000Z");
        assert_eq!(day["untilTime"], "2026-11-17T16:12:00.000Z");
    }
    #[test]
    fn labels_match_the_pinned_original_functions() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/usage-format-oracle.json"
        ))
        .unwrap();
        for row in fixture["rows"].as_array().unwrap() {
            let value = row["value"].as_f64().unwrap();
            assert_eq!(tokens(value), row["tokens"], "tokens {value}");
            assert_eq!(usd(value), row["usd"], "usd {value}");
            assert_eq!(count(value), row["count"], "count {value}");
        }
    }
    #[test]
    fn compact_token_labels_retain_sign_and_precision() {
        for (value, expected) in [
            (0., "0"),
            (999., "999"),
            (1000., "1K"),
            (1234., "1.23K"),
            (12500., "12.5K"),
            (999999., "1000K"),
            (1e6, "1M"),
            (-1250., "-1.25K"),
        ] {
            assert_eq!(tokens(value), expected);
        }
    }
    #[test]
    fn duplicate_sources_and_reasoning_do_not_inflate_totals() {
        let summary = json!({"contractVersion":6,"readAt":"2026-11-18T00:00:00Z","sources":[{"fingerprint":{"hostId":"host","provider":"codex","resolvedHomePath":"/home","volumeId":"1"},"status":"ok","distinctSessions":2}],"buckets":[{"provider":"codex","model":"model","day":"2026-11-18","totals":{"uncachedInputTokens":100,"cachedInputTokens":40,"cacheCreationTokens":10,"outputTokens":30,"reasoningTokens":20},"costUsd":1.5,"categoryCostUsd":{"input":0.5,"cacheRead":0.125,"cacheWrite":0.125,"output":0.75},"fastCostUsd":0.5,"ultrafastCostUsd":0.25,"speedPremiumUsd":0.125,"records":2,"unpricedRecords":0,"sessions":2}]});
        let merged = merge(
            &[("a".into(), summary.clone()), ("b".into(), summary)],
            &HashSet::new(),
        );
        assert_eq!(merged.totals.total_tokens(), 180.);
        assert_eq!(merged.totals.sessions, 2.);
        assert_eq!(merged.totals.cost, 1.5);
        assert_eq!(merged.totals.category_cost, [0.5, 0.125, 0.125, 0.75]);
        assert_eq!(merged.totals.speed_cost, [0.5, 0.25, 0.125]);
        assert_eq!(
            merged.models.values().next().unwrap().category_cost,
            merged.totals.category_cost
        );
        assert_eq!(merged.warnings.len(), 1);
    }
}
