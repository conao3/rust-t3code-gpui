use anyhow::{Context as _, Result};
use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
use gpui_kit::{App, Global};
use icu_calendar::Date;
use icu_datetime::fieldsets::{E, M, MD, T, YMD};
use icu_datetime::input::Time;
use icu_datetime::options::YearStyle;
use icu_datetime::preferences::HourCycle;
use icu_datetime::{
    DateTimeFormatter, DateTimeFormatterPreferences, FixedCalendarDateTimeFormatter,
    NoCalendarFormatter,
};
use icu_locale_core::Locale;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Locale,
    Clock12,
    Clock24,
}

impl Format {
    pub fn from_setting(value: Option<&str>) -> Self {
        match value {
            Some("12-hour") => Self::Clock12,
            Some("24-hour") => Self::Clock24,
            _ => Self::Locale,
        }
    }
}

pub fn locale_tag(identifier: &str) -> Option<Locale> {
    let (base, keywords) = identifier.split_once('@').unwrap_or((identifier, ""));
    let base = base.split('.').next()?.replace('_', "-");
    if matches!(base.as_str(), "C" | "POSIX" | "") {
        return None;
    }
    let mut locale: Locale = base.parse().ok()?;
    for keyword in keywords.split(';') {
        if let Some(region) = keyword
            .split_once('=')
            .filter(|(key, _)| key.eq_ignore_ascii_case("rg"))
            .and_then(|(_, value)| {
                value
                    .get(..value.len().saturating_sub(4))
                    .filter(|_| value.to_ascii_lowercase().ends_with("zzzz"))
            })
            && let Ok(region) = region.to_ascii_uppercase().parse()
        {
            locale.id.region = Some(region);
        }
    }
    Some(locale)
}

pub fn system_locale() -> Locale {
    let time_locale = ["LC_ALL", "LC_TIME", "LANG"]
        .into_iter()
        .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()));
    time_locale
        .as_deref()
        .and_then(|value| value.split('.').next())
        .and_then(locale_tag)
        .or_else(|| sys_locale::get_locale().as_deref().and_then(locale_tag))
        .unwrap_or_else(|| "en-US".parse().expect("valid locale"))
}

pub struct Formatter {
    locale: Locale,
    time: NoCalendarFormatter<T>,
    date: DateTimeFormatter<MD>,
    year_date: DateTimeFormatter<YMD>,
    english_month: FixedCalendarDateTimeFormatter<icu_calendar::cal::Gregorian, M>,
    weekday: FixedCalendarDateTimeFormatter<icu_calendar::cal::Gregorian, E>,
}

impl Formatter {
    pub fn new(locale: Locale, format: Format) -> Result<Self> {
        let mut preferences = DateTimeFormatterPreferences::from(locale.clone());
        match format {
            Format::Locale => {}
            Format::Clock12 => preferences.hour_cycle = Some(HourCycle::Clock12),
            Format::Clock24 => preferences.hour_cycle = Some(HourCycle::Clock24),
        };
        Ok(Self {
            locale: locale.clone(),
            weekday: FixedCalendarDateTimeFormatter::try_new(locale.clone().into(), E::short())
                .context("load weekday format")?,
            time: NoCalendarFormatter::try_new(preferences, T::hm())
                .context("load timestamp time format")?,
            date: DateTimeFormatter::try_new(locale.clone().into(), MD::short())
                .context("load timestamp date format")?,
            year_date: DateTimeFormatter::try_new(
                locale.into(),
                YMD::short().with_year_style(YearStyle::Full),
            )
            .context("load timestamp year format")?,
            english_month: FixedCalendarDateTimeFormatter::try_new(
                "en-US".parse::<Locale>().expect("valid locale").into(),
                M::long(),
            )
            .context("load timestamp month format")?,
        })
    }

    pub fn display_in<Tz: TimeZone>(
        &self,
        source: &str,
        now: &DateTime<Tz>,
    ) -> Option<(String, String)> {
        let value = DateTime::parse_from_rfc3339(source)
            .ok()?
            .with_timezone(&now.timezone());
        let time = Time::try_new(
            value.hour() as u8,
            value.minute() as u8,
            value.second() as u8,
            0,
        )
        .ok()?;
        let date =
            Date::try_new_gregorian(value.year(), value.month() as u8, value.day() as u8).ok()?;
        let time = self
            .time
            .format(&time)
            .to_string()
            .replace(['\u{202f}', '\u{2009}'], " ");
        let days = now
            .date_naive()
            .signed_duration_since(value.date_naive())
            .num_days();
        let label = match days {
            ..=0 => time.clone(),
            1 => format!("yesterday at {time}"),
            _ if value.year() == now.year() => format!("{} {time}", self.date.format(&date)),
            _ => format!("{} {time}", self.year_date.format(&date)),
        };
        let day = value.day();
        let suffix = match day % 100 {
            11..=13 => "th",
            _ => match day % 10 {
                1 => "st",
                2 => "nd",
                3 => "rd",
                _ => "th",
            },
        };
        let tooltip = format!(
            "{time}, {day}{suffix} {} {}",
            self.english_month.format(&date),
            value.year()
        );
        Some((label, tooltip))
    }

    pub fn short_in<Tz: TimeZone>(&self, value: &DateTime<Tz>) -> String {
        Time::try_new(
            value.hour() as u8,
            value.minute() as u8,
            value.second() as u8,
            0,
        )
        .ok()
        .map(|time| {
            self.time
                .format(&time)
                .to_string()
                .replace(['\u{202f}', '\u{2009}'], " ")
        })
        .unwrap_or_default()
    }

    pub fn weekday_in<Tz: TimeZone>(&self, value: &DateTime<Tz>) -> String {
        Date::try_new_gregorian(value.year(), value.month() as u8, value.day() as u8)
            .ok()
            .map(|date| self.weekday.format(&date).to_string())
            .unwrap_or_default()
    }
}

impl Global for Formatter {}

pub fn initialize(format: Format, cx: &mut App) -> Result<()> {
    cx.set_global(Formatter::new(system_locale(), format)?);
    Ok(())
}

pub fn change_format(format: Format, cx: &mut App) -> Result<()> {
    let locale = cx.global::<Formatter>().locale.clone();
    cx.set_global(Formatter::new(locale, format)?);
    cx.refresh_windows();
    Ok(())
}

pub fn display(source: &str, cx: &App) -> (String, String) {
    cx.global::<Formatter>()
        .display_in(source, &Local::now())
        .unwrap_or_default()
}

pub fn short<Tz: TimeZone>(value: &DateTime<Tz>, cx: &App) -> String {
    cx.global::<Formatter>().short_in(value)
}

pub fn weekday<Tz: TimeZone>(value: &DateTime<Tz>, cx: &App) -> String {
    cx.global::<Formatter>().weekday_in(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_locale_identifier_normalization_matches_desktop() {
        for (source, expected) in [
            ("en_US.UTF-8", "en-US"),
            ("ja_JP.UTF-8", "ja-JP"),
            ("en_US@rg=gbzzzz", "en-GB"),
            ("de_DE@rg=419zzzz", "de-419"),
        ] {
            assert_eq!(locale_tag(source).unwrap().to_string(), expected);
        }
        for source in ["C.UTF-8", "POSIX", "", "invalid_!tag"] {
            assert!(locale_tag(source).is_none());
        }
    }
    #[test]
    fn actual_electron_timestamp_oracle_matches_across_locales_and_dst() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/timestamp-oracle.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let zone: chrono_tz::Tz = case["zone"].as_str().unwrap().parse().unwrap();
            let now = DateTime::parse_from_rfc3339(case["now"].as_str().unwrap())
                .unwrap()
                .with_timezone(&zone);
            let formatter = Formatter::new(
                case["locale"].as_str().unwrap().parse().unwrap(),
                Format::from_setting(case["format"].as_str()),
            )
            .unwrap();
            let (label, tooltip) = formatter
                .display_in(case["timestamp"].as_str().unwrap(), &now)
                .unwrap();
            assert_eq!(label, case["label"].as_str().unwrap(), "{case}");
            assert_eq!(tooltip, case["tooltip"].as_str().unwrap(), "{case}");
        }
    }

    #[test]
    fn japanese_clock_and_day_labels_keep_the_reference_english_prefix() {
        let now = DateTime::parse_from_rfc3339("2026-11-17T12:00:00+09:00").unwrap();
        let formatter = Formatter::new("ja-JP".parse().unwrap(), Format::Clock12).unwrap();
        let value = formatter
            .display_in("2026-11-16T00:20:00+09:00", &now)
            .unwrap();
        assert_eq!(
            value,
            (
                "yesterday at 午前0:20".into(),
                "午前0:20, 16th November 2026".into()
            )
        );
        assert!(formatter.display_in("invalid", &now).is_none());
    }
}
