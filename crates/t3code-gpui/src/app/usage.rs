use super::usage_data::{self as data, Merged, Totals, number, text};
use super::*;
use gpui_kit::component::button::{Toggle, ToggleGroup};
use gpui_kit::component::popover::Popover;
use std::collections::BTreeMap;
use t3_client::rpc::StreamLimits;

const PROVIDERS: &[(&str, &str, u32)] = &[
    ("codex", "Codex", 0xffffff),
    ("claude", "Claude Code", 0xd97757),
    ("grok", "Grok Build", 0xaaaaaa),
    ("cursor", "Cursor", 0x8b8b8b),
    ("opencode", "OpenCode", 0x5b9bbd),
    ("antigravity", "Antigravity", 0x8c7bd1),
    ("pi", "Pi", 0x4d9f8a),
    ("muse", "Muse Code", 0x0866ff),
];
const PERIODS: &[(u32, &str)] = &[
    (1, "Past 24h"),
    (7, "7 days"),
    (30, "30 days"),
    (90, "90 days"),
];

pub(super) struct UsagePage {
    root: WeakEntity<Root>,
    focus: FocusHandle,
    metric: String,
    days: u32,
    window: Value,
    summaries: Vec<(String, Value)>,
    configs: BTreeMap<String, Value>,
    selected: HashSet<String>,
    hidden: HashSet<String>,
    merged: Merged,
    errors: Vec<String>,
    loading: bool,
    generation: u64,
    hovered_period: Option<usize>,
    breakdown_time: bool,
    workers: Vec<tokio::task::JoinHandle<()>>,
}
impl Drop for UsagePage {
    fn drop(&mut self) {
        for task in &self.workers {
            task.abort();
        }
    }
}
impl UsagePage {
    pub(super) fn new(
        root: WeakEntity<Root>,
        owner: &Root,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let metric = owner.settings.values["gpuiUsageMetric"]
            .as_str()
            .filter(|value| matches!(*value, "cost" | "tokens" | "limits"))
            .unwrap_or("limits")
            .to_string();
        let days = owner.settings.values["gpuiUsageWindowDays"]
            .as_u64()
            .filter(|value| [1, 7, 30, 90].contains(value))
            .unwrap_or(30) as u32;
        let hidden = owner.settings.values["gpuiUsageHiddenProviders"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        let selected = owner
            .connections
            .iter()
            .map(|connection| connection.label().to_string())
            .collect();
        let configs = owner
            .connections
            .iter()
            .filter_map(|connection| {
                owner
                    .config_of(connection.label())
                    .map(|config| (connection.label().to_string(), config))
            })
            .collect();
        let connections = owner.connections.clone();
        let runtime = owner.runtime.clone();
        let mut view = Self {
            root,
            focus: cx.focus_handle(),
            metric,
            days,
            window: Value::Null,
            summaries: vec![],
            configs,
            selected,
            hidden,
            merged: Merged::default(),
            errors: vec![],
            loading: false,
            generation: 0,
            hovered_period: None,
            breakdown_time: false,
            workers: vec![],
        };
        view.focus.focus(window, cx);
        let (sender, receiver) = async_channel::bounded(16);
        for connection in connections {
            let sender = sender.clone();
            view.workers.push(runtime.spawn(async move {
                let label = connection.label().to_string();
                let result: anyhow::Result<()> = async {
                    let mut config = None;
                    let mut stream = connection
                        .subscribe(
                            "subscribeServerConfig",
                            json!({"usageLimitSources":true,"usageLimitsCommand":true}),
                            StreamLimits::new(8, 8 << 20).expect("stream limits"),
                        )
                        .await?;
                    while let Some(chunk) = stream.next().await {
                        let chunk = chunk?;
                        for event in chunk.values() {
                            t3_client::projection::apply_config(&mut config, event);
                            if event["type"] == "usageLimitSourcesUpdated"
                                && let Some(config) = config.as_mut()
                            {
                                config["usageLimitSources"] = event["payload"]["sources"].clone();
                            }
                        }
                        if let Some(config) = &config
                            && sender
                                .send((label.clone(), Ok(config.clone())))
                                .await
                                .is_err()
                        {
                            break;
                        }
                        chunk.accept();
                    }
                    Ok(())
                }
                .await;
                if let Err(error) = result {
                    let _ = sender.send((label, Err(error.to_string()))).await;
                }
            }));
        }
        drop(sender);
        cx.spawn(async move |this, cx| {
            while let Ok(first) = receiver.recv().await {
                cx.background_executor().timer(REDRAW_INTERVAL).await;
                let mut updates = vec![first];
                while let Ok(next) = receiver.try_recv() {
                    updates.push(next);
                }
                if this
                    .update(cx, |view, cx| {
                        for (label, result) in updates {
                            match result {
                                Ok(config) => {
                                    view.configs.insert(label, config);
                                }
                                Err(error) => view.errors.push(format!("{label}: {error}")),
                            }
                        }
                        if view.metric == "limits" {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let this = cx.entity().downgrade();
        cx.defer(move |cx| {
            let _ = this.update(cx, |view, cx| view.refresh(false, cx));
        });
        view
    }
    fn save_preferences(&self, cx: &mut Context<Self>) {
        let _ = self.root.update(cx, |root, cx| {
            root.change_setting("gpuiUsageMetric", json!(self.metric), cx);
            root.change_setting("gpuiUsageWindowDays", json!(self.days), cx);
            root.change_setting("gpuiUsageHiddenProviders", json!(self.hidden), cx);
        });
    }
    pub(super) fn choose(
        &mut self,
        metric: Option<&str>,
        days: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        if let Some(metric) = metric {
            self.metric = metric.to_string();
        }
        if let Some(days) = days
            && self.metric != "limits"
        {
            self.days = days;
            self.refresh(false, cx);
        }
        if metric.is_some() {
            self.refresh(false, cx);
        }
        self.save_preferences(cx);
        cx.notify();
    }
    fn recompute(&mut self) {
        self.merged = data::merge(
            &self
                .summaries
                .iter()
                .filter(|(label, _)| self.selected.contains(label))
                .cloned()
                .collect::<Vec<_>>(),
            &self.hidden,
        );
    }
    fn refresh(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(owner) = self.root.upgrade() else {
            return;
        };
        let owner = owner.read(cx);
        let runtime = owner.runtime.clone();
        let connections = owner.connections.clone();
        let zone = match iana_time_zone::get_timezone() {
            Ok(zone) => zone,
            Err(error) => {
                self.errors = vec![format!("Cannot determine the usage time zone: {error}")];
                cx.notify();
                return;
            }
        };
        self.window = data::make_window(self.days, chrono::Local::now(), &zone);
        self.generation += 1;
        let generation = self.generation;
        let input = self.window.clone();
        let limits = self.metric == "limits";
        let filters = self
            .configs
            .iter()
            .map(|(label, config)| {
                (
                    label.clone(),
                    config["settings"]["usageModelFilter"].clone(),
                )
            })
            .collect::<HashMap<_, _>>();
        self.loading = true;
        self.errors.clear();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                let mut requests = tokio::task::JoinSet::new();
                for connection in connections {
                    let mut input = input.clone();
                    if let Some(filter) = filters
                        .get(connection.label())
                        .filter(|filter| !filter.is_null())
                    {
                        input["modelFilter"] = filter.clone();
                    }
                    requests.spawn(async move {
                        let label = connection.label().to_string();
                        let result: anyhow::Result<Value> = async {
                            if limits {
                                connection
                                    .request("server.refreshProviders", json!({}))
                                    .await?;
                                Ok(Value::Null)
                            } else {
                                if force {
                                    connection
                                        .request("server.refreshUsageRates", json!({}))
                                        .await?;
                                }
                                let mut summary = connection
                                    .request("server.getUsageSummary", input.clone())
                                    .await?;
                                if summary["sources"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .any(|source| source["refreshing"] == true)
                                {
                                    input["awaitRefresh"] = json!(true);
                                    summary =
                                        connection.request("server.getUsageSummary", input).await?;
                                }
                                Ok(summary)
                            }
                        }
                        .await;
                        (label, result)
                    });
                }
                let mut results = Vec::new();
                while let Some(result) = requests.join_next().await {
                    results.push(result?);
                }
                Ok(results)
            })
            .await;
            let _ = this.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.loading = false;
                match result {
                    Ok(results) => {
                        if !limits {
                            view.summaries.clear();
                        }
                        for (label, result) in results {
                            match result {
                                Ok(summary) if !limits => view.summaries.push((label, summary)),
                                Ok(_) => {}
                                Err(error) => view.errors.push(format!("{label}: {error:#}")),
                            }
                        }
                        view.recompute();
                    }
                    Err(error) => view.errors.push(format!("Unable to load usage: {error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn filter_environment(&mut self, label: String, cx: &mut Context<Self>) {
        if !self.selected.remove(&label) {
            self.selected.insert(label);
        }
        self.recompute();
        cx.notify();
    }
    fn filter_provider(&mut self, provider: String, cx: &mut Context<Self>) {
        if !self.hidden.remove(&provider) {
            self.hidden.insert(provider);
        }
        self.recompute();
        self.save_preferences(cx);
        cx.notify();
    }
    fn open_model(&self, key: &(String, String), window: &mut Window, cx: &mut Context<Self>) {
        let Some(total) = self.merged.models.get(key).cloned() else {
            return;
        };
        let title = key.1.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            dialog
                .title(title.clone())
                .accessibility_label(title.clone())
                .centered()
                .width(px(720.))
                .child(
                    v_flex()
                        .gap_3()
                        .child(
                            h_flex()
                                .justify_between()
                                .child("Total cost")
                                .child(data::usd(total.cost)),
                        )
                        .child(
                            h_flex()
                                .justify_between()
                                .child("Total tokens")
                                .child(data::tokens(total.total_tokens())),
                        )
                        .children(
                            ["Input", "Cache read", "Cache write", "Output", "Reasoning"]
                                .iter()
                                .zip(total.tokens)
                                .map(|(label, value)| {
                                    h_flex()
                                        .justify_between()
                                        .child(*label)
                                        .child(data::tokens(value))
                                }),
                        )
                        .child(
                            h_flex()
                                .justify_between()
                                .child("Cache savings")
                                .child(data::usd(total.cache_savings)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{} records", data::count(total.records))),
                        ),
                )
        });
    }
    fn render_chart(&self, cx: &Context<Self>) -> AnyElement {
        let mut periods = Vec::new();
        if self.days == 1 {
            if let (Ok(mut start), Ok(end)) = (
                chrono::DateTime::parse_from_rfc3339(text(&self.window, "sinceTime")),
                chrono::DateTime::parse_from_rfc3339(text(&self.window, "untilTime")),
            ) {
                while start < end {
                    periods.push(start.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
                    start += chrono::Duration::hours(1);
                }
            }
        } else if let (Ok(mut start), Ok(end)) = (
            chrono::NaiveDate::parse_from_str(text(&self.window, "sinceDay"), "%Y-%m-%d"),
            chrono::NaiveDate::parse_from_str(text(&self.window, "untilDay"), "%Y-%m-%d"),
        ) {
            while start <= end {
                periods.push(start.to_string());
                start = start.succ_opt().expect("usage date");
            }
        }
        let metric = |total: &Totals| {
            if self.metric == "tokens" {
                total.total_tokens()
            } else {
                total.cost
            }
        };
        let theme = cx.theme().colors;
        let mut series = PROVIDERS
            .iter()
            .filter(|(provider, _, _)| self.merged.providers.contains_key(*provider))
            .map(|(provider, label, color)| {
                let values = periods
                    .iter()
                    .map(|period| {
                        self.merged
                            .periods
                            .get(period)
                            .and_then(|entries| entries.get(*provider))
                            .map(metric)
                            .unwrap_or(0.)
                    })
                    .collect::<Vec<_>>();
                let color: Hsla = if *provider == "codex" {
                    theme.foreground
                } else {
                    rgb(*color).into()
                };
                (*label, values, color)
            })
            .collect::<Vec<_>>();
        series.sort_by(|a, b| b.1.iter().sum::<f64>().total_cmp(&a.1.iter().sum::<f64>()));
        let peak = series
            .iter()
            .flat_map(|(_, values, _)| values)
            .copied()
            .fold(0., f64::max);
        let (maximum, ticks) = data::chart_scale(peak);
        let plot_series = series.clone();
        let plot_ticks = ticks.clone();
        let hovered = self.hovered_period.filter(|index| *index < periods.len());
        let count = periods.len();
        let format = |value| {
            if self.metric == "tokens" {
                data::tokens(value)
            } else {
                data::usd(value)
            }
        };
        let mut chart = div()
            .id("usage-chart")
            .relative()
            .flex_1()
            .h(px(224.))
            .child(
                div()
                    .id("usage-chart-image")
                    .role(accesskit::Role::Image)
                    .aria_label(format!(
                        "{} {} by provider",
                        if self.days == 1 { "Hourly" } else { "Daily" },
                        if self.metric == "tokens" {
                            "processed tokens"
                        } else {
                            "cost"
                        }
                    ))
                    .size_full()
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, _| {
                                let width = f32::from(bounds.size.width);
                                let height = f32::from(bounds.size.height);
                                let y = |value: f64| {
                                    height
                                        - if maximum > 0. {
                                            (value / maximum) as f32 * height * (252. / 260.)
                                        } else {
                                            0.
                                        }
                                };
                                for tick in &plot_ticks {
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(bounds.left(), bounds.top() + px(y(*tick))),
                                            size(bounds.size.width, px(1.)),
                                        ),
                                        theme.border,
                                    ));
                                }
                                for (_, values, color) in &plot_series {
                                    if values.len() < 2 {
                                        continue;
                                    }
                                    let dx = width / (values.len() - 1) as f32;
                                    let points = values
                                        .iter()
                                        .enumerate()
                                        .map(|(index, value)| (index as f32 * dx, y(*value)))
                                        .collect::<Vec<_>>();
                                    let tangents = data::chart_tangents(&points);
                                    for filled in [true, false] {
                                        let mut path = if filled {
                                            PathBuilder::fill()
                                        } else {
                                            PathBuilder::stroke(px(2.))
                                        };
                                        let position = |x, y| {
                                            point(bounds.left() + px(x), bounds.top() + px(y))
                                        };
                                        path.move_to(position(points[0].0, points[0].1));
                                        for index in 0..points.len() - 1 {
                                            let (x, y) = points[index];
                                            let (next_x, next_y) = points[index + 1];
                                            path.cubic_bezier_to(
                                                position(next_x, next_y),
                                                position(
                                                    x + dx / 3.,
                                                    y + tangents[index] * dx / 3.,
                                                ),
                                                position(
                                                    next_x - dx / 3.,
                                                    next_y - tangents[index + 1] * dx / 3.,
                                                ),
                                            );
                                        }
                                        if filled {
                                            path.line_to(position(width, height));
                                            path.line_to(position(0., height));
                                            path.close();
                                        }
                                        window.paint_path(
                                            path.build().expect("valid provider usage curve"),
                                            if filled { color.opacity(0.12) } else { *color },
                                        );
                                    }
                                }
                                if let Some(index) = hovered
                                    && count > 1
                                {
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(
                                                bounds.left()
                                                    + px(index as f32 / (count - 1) as f32 * width),
                                                bounds.top(),
                                            ),
                                            size(px(1.), bounds.size.height),
                                        ),
                                        theme.muted_foreground,
                                    ));
                                }
                            },
                        )
                        .size_full(),
                    ),
            )
            .child(
                h_flex()
                    .absolute()
                    .inset_0()
                    .children(periods.iter().enumerate().map(|(index, _)| {
                        div()
                            .id(("usage-period", index))
                            .h_full()
                            .flex_grow(if index == 0 || index + 1 == count {
                                0.5
                            } else {
                                1.
                            })
                            .on_hover(cx.listener(move |view, over: &bool, _, cx| {
                                let next = if *over { Some(index) } else { None };
                                if next != view.hovered_period {
                                    view.hovered_period = next;
                                    cx.notify();
                                }
                            }))
                    })),
            );
        if let Some(index) = hovered {
            let mut tooltip = v_flex()
                .absolute()
                .top_2()
                .right_2()
                .min_w(px(144.))
                .p_2()
                .gap_1()
                .rounded_lg()
                .border_1()
                .border_color(theme.border)
                .bg(theme.popover)
                .text_xs()
                .child(
                    settings_controls::text(self.period_label(&periods[index]))
                        .text_color(theme.muted_foreground),
                );
            for (label, values, _) in &series {
                tooltip = tooltip.child(
                    h_flex()
                        .id(SharedString::from(*label))
                        .justify_between()
                        .gap_3()
                        .child(settings_controls::text(*label))
                        .child(settings_controls::text(format(values[index]))),
                );
            }
            tooltip = tooltip.child(
                h_flex()
                    .id("usage-tooltip-total")
                    .border_t_1()
                    .border_color(theme.border)
                    .justify_between()
                    .child(settings_controls::text("Total"))
                    .child(settings_controls::text(format(
                        series.iter().map(|(_, values, _)| values[index]).sum(),
                    ))),
            );
            chart = chart.child(tooltip);
        }
        v_flex()
            .id("usage-chart-layout")
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(div().relative().w(px(56.)).h(px(224.)).children(
                        ticks.iter().enumerate().map(|(index, tick)| {
                            settings_controls::text(if *tick == 0. {
                                "0".to_string()
                            } else {
                                format(*tick)
                            })
                            .id(("usage-value-axis", index))
                            .absolute()
                            .right_0()
                            .top(px(224.
                                - if maximum > 0. {
                                    (*tick / maximum) as f32 * 224. * (252. / 260.)
                                } else {
                                    0.
                                }
                                - 8.))
                            .text_xs()
                            .text_color(theme.muted_foreground)
                        }),
                    ))
                    .child(chart),
            )
            .child(
                h_flex()
                    .pl(px(64.))
                    .justify_between()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .children(
                        [0, periods.len() / 2, periods.len().saturating_sub(1)]
                            .into_iter()
                            .enumerate()
                            .filter_map(|(index, period)| {
                                periods.get(period).map(|period| {
                                    div()
                                        .id(("usage-axis", index))
                                        .child(settings_controls::text(self.period_label(period)))
                                })
                            }),
                    ),
            )
            .into_any_element()
    }
    fn period_label(&self, period: &str) -> String {
        if self.days == 1 {
            chrono::DateTime::parse_from_rfc3339(period)
                .map(|value| {
                    value
                        .with_timezone(&chrono::Local)
                        .format("%-I %p")
                        .to_string()
                })
                .unwrap_or_else(|_| period.to_string())
        } else {
            chrono::NaiveDate::parse_from_str(period, "%Y-%m-%d")
                .map(|date| date.format("%b %-d").to_string())
                .unwrap_or_else(|_| period.to_string())
        }
    }
    fn render_limits(&self, cx: &Context<Self>) -> AnyElement {
        let mut accounts = Vec::<(String, Value)>::new();
        let mut fingerprints = HashSet::new();
        for (environment, config) in &self.configs {
            if !self.selected.contains(environment) {
                continue;
            }
            for provider in config["providers"].as_array().into_iter().flatten() {
                let driver = text(provider, "driver");
                let driver = if driver == "claudeAgent" {
                    "claude"
                } else {
                    driver
                };
                if self.hidden.contains(driver) || provider["enabled"] == false {
                    continue;
                }
                let limits = &provider["usageLimits"];
                if limits.is_null() {
                    continue;
                }
                let fingerprint = limits["credentialFingerprint"]
                    .as_str()
                    .map(|value| format!("{driver}:{value}"));
                if fingerprint.is_some_and(|key| !fingerprints.insert(key)) {
                    continue;
                }
                accounts.push((
                    format!(
                        "{} · {environment}",
                        provider["displayName"]
                            .as_str()
                            .unwrap_or_else(|| text(provider, "instanceId"))
                    ),
                    limits.clone(),
                ));
            }
            for source in config["usageLimitSources"].as_array().into_iter().flatten() {
                for account in source["accounts"].as_array().into_iter().flatten() {
                    let driver = text(account, "driver");
                    let driver = if driver == "claudeAgent" {
                        "claude"
                    } else {
                        driver
                    };
                    if self.hidden.contains(driver) {
                        continue;
                    }
                    let limits = &account["usageLimits"];
                    let fingerprint = limits["credentialFingerprint"]
                        .as_str()
                        .map(|value| format!("{driver}:{value}"));
                    if fingerprint.is_some_and(|key| !fingerprints.insert(key)) {
                        continue;
                    }
                    accounts.push((
                        format!(
                            "{} · {} · {environment}",
                            account["email"]
                                .as_str()
                                .unwrap_or_else(|| text(account, "id")),
                            text(source, "label")
                        ),
                        limits.clone(),
                    ));
                }
            }
        }
        let theme = cx.theme().colors;
        v_flex()
            .gap_4()
            .when(accounts.is_empty(), |view| {
                view.child(div().text_sm().child(if self.loading {
                    "Loading limits…"
                } else {
                    "No usage limits available"
                }))
            })
            .children(
                accounts
                    .into_iter()
                    .enumerate()
                    .map(|(index, (label, limits))| {
                        v_flex()
                            .id(("usage-account", index))
                            .role(accesskit::Role::Group)
                            .aria_label(label.clone())
                            .gap_3()
                            .p_4()
                            .rounded_lg()
                            .border_1()
                            .border_color(theme.border)
                            .child(div().font_weight(FontWeight::MEDIUM).child(label))
                            .when_some(
                                limits["unavailable"]["message"].as_str(),
                                |view, message| {
                                    view.child(
                                        div()
                                            .text_sm()
                                            .text_color(theme.muted_foreground)
                                            .child(message.to_string()),
                                    )
                                },
                            )
                            .children(
                                limits["windows"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .enumerate()
                                    .map(|(window_index, limit)| {
                                        let used = number(limit, "usedPercent");
                                        v_flex()
                                            .gap_1()
                                            .child(
                                                h_flex()
                                                    .justify_between()
                                                    .child(text(limit, "label").to_string())
                                                    .child(format!("{used:.0}% used")),
                                            )
                                            .child(
                                                div()
                                                    .id(("limit", window_index))
                                                    .role(accesskit::Role::ProgressIndicator)
                                                    .aria_label(text(limit, "label").to_string())
                                                    .h(px(6.))
                                                    .w_full()
                                                    .bg(theme.muted)
                                                    .rounded_full()
                                                    .child(
                                                        div()
                                                            .h_full()
                                                            .w(relative(
                                                                (used.clamp(0., 100.) / 100.)
                                                                    as f32,
                                                            ))
                                                            .bg(if used >= 100. {
                                                                theme.danger
                                                            } else {
                                                                theme.primary
                                                            })
                                                            .rounded_full(),
                                                    ),
                                            )
                                            .when_some(limit["resetsAt"].as_str(), |view, reset| {
                                                view.child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(theme.muted_foreground)
                                                        .child(format!("Resets {reset}")),
                                                )
                                            })
                                    }),
                            )
                    }),
            )
            .into_any_element()
    }
}

impl Render for UsagePage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().colors;
        let environments = self.configs.keys().cloned().collect::<Vec<_>>();
        let selected = self.selected.clone();
        let hidden = self.hidden.clone();
        let menu_root = cx.entity().downgrade();
        let provider_root = menu_root.clone();
        let mut body = v_flex()
            .gap_6()
            .w_full()
            .max_w(px(1024.))
            .mx_auto()
            .p_6()
            .children(
                self.errors
                    .iter()
                    .chain(self.merged.warnings.iter())
                    .enumerate()
                    .map(|(index, error)| {
                        div()
                            .id(("usage-warning", index))
                            .role(accesskit::Role::Alert)
                            .text_sm()
                            .text_color(theme.danger)
                            .child(error.clone())
                    }),
            );
        if self.selected.is_empty() {
            body = body.child("Select an environment to see usage.");
        } else if self.metric == "limits" {
            body = body.child(self.render_limits(cx));
        } else if self.loading && self.summaries.is_empty() {
            body = body.child("Loading usage…");
        } else {
            let total = &self.merged.totals;
            let quality = format!(
                "API estimate excludes {} unpriced records.",
                data::percent(if total.records > 0. {
                    total.unpriced / total.records
                } else {
                    0.
                })
            );
            let mut summary = v_flex()
                .id("usage-summary")
                .w(px(288.))
                .flex_none()
                .gap_5()
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_size(rems(2.25))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(if self.metric == "tokens" {
                                    data::tokens(total.total_tokens())
                                } else {
                                    data::usd(total.cost)
                                }),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(settings_controls::text(format!(
                                    "{} sessions{}",
                                    data::count(total.sessions),
                                    if self.metric == "cost" {
                                        " · API estimate"
                                    } else {
                                        ""
                                    }
                                )))
                                .when(self.metric == "cost" && total.unpriced > 0., |row| {
                                    let content = quality.clone();
                                    row.child(
                                        Popover::new("usage-unpriced")
                                            .anchor(Anchor::BottomCenter)
                                            .trigger(
                                                Button::new("usage-unpriced-trigger")
                                                    .icon(IconName::Info)
                                                    .accessibility_label("Unpriced usage details")
                                                    .tooltip(quality.clone())
                                                    .ghost()
                                                    .small()
                                                    .compact(),
                                            )
                                            .content(move |_, _, _| {
                                                settings_controls::text(content.clone())
                                                    .text_xs()
                                                    .max_w(px(300.))
                                            }),
                                    )
                                }),
                        ),
                );
            let mut providers = self.merged.providers.iter().collect::<Vec<_>>();
            providers.sort_by(|(_, a), (_, b)| {
                let metric = |value: &Totals| {
                    if self.metric == "tokens" {
                        value.total_tokens()
                    } else {
                        value.cost
                    }
                };
                metric(b).total_cmp(&metric(a))
            });
            for (provider, provider_total) in providers {
                let (label, color) = PROVIDERS
                    .iter()
                    .find(|(id, _, _)| id == provider)
                    .map(|(_, label, color)| (*label, *color))
                    .unwrap_or((provider, 0x888888));
                let share = if self.metric == "tokens" {
                    if total.total_tokens() > 0. {
                        provider_total.total_tokens() / total.total_tokens()
                    } else {
                        0.
                    }
                } else if total.cost > 0. {
                    provider_total.cost / total.cost
                } else {
                    0.
                };
                summary = summary.child(
                    v_flex()
                        .id(SharedString::from(format!("usage-provider:{provider}")))
                        .gap_1()
                        .child(
                            h_flex()
                                .gap_2()
                                .text_sm()
                                .child(div().size(px(8.)).rounded_full().bg(rgb(color)))
                                .child(settings_controls::text(label.to_string()))
                                .child(
                                    settings_controls::text(format!(
                                        "{} {}",
                                        data::count(provider_total.sessions),
                                        if provider_total.sessions == 1. {
                                            "session"
                                        } else {
                                            "sessions"
                                        }
                                    ))
                                    .text_xs()
                                    .text_color(theme.muted_foreground),
                                )
                                .child(div().flex_1())
                                .child(settings_controls::text(if self.metric == "tokens" {
                                    data::tokens(provider_total.total_tokens())
                                } else {
                                    data::usd(provider_total.cost)
                                })),
                        )
                        .child(
                            settings_controls::text(if self.metric == "tokens" {
                                format!(
                                    "{} of tokens · {}",
                                    data::percent(share),
                                    data::usd(provider_total.cost)
                                )
                            } else {
                                format!(
                                    "{} of cost · {} tokens",
                                    data::percent(share),
                                    data::tokens(provider_total.total_tokens())
                                )
                            })
                            .text_xs()
                            .text_color(theme.muted_foreground),
                        ),
                );
            }
            body = body
                .child(
                    h_flex().items_start().gap_6().child(summary).child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_3()
                            .child(
                                settings_controls::text(format!(
                                    "{} {}",
                                    if self.days == 1 { "Hourly" } else { "Daily" },
                                    if self.metric == "tokens" {
                                        "processed tokens"
                                    } else {
                                        "cost"
                                    }
                                ))
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM),
                            )
                            .child(self.render_chart(cx)),
                    ),
                )
                .child(
                    v_flex()
                        .id("usage-totals")
                        .gap_2()
                        .child(
                            settings_controls::text("Totals")
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM),
                        )
                        .child(
                            h_flex().gap_6().children(
                                [
                                    ("Processed tokens", data::tokens(total.total_tokens())),
                                    ("Cached input", data::tokens(total.tokens[1])),
                                    ("Uncached input", data::tokens(total.tokens[0])),
                                    ("Output", data::tokens(total.tokens[3])),
                                    ("Cache savings", data::usd(total.cache_savings)),
                                ]
                                .into_iter()
                                .map(|(label, value)| {
                                    v_flex()
                                        .id(SharedString::from(label))
                                        .flex_1()
                                        .gap_1()
                                        .child(
                                            settings_controls::text(label)
                                                .text_xs()
                                                .text_color(theme.muted_foreground),
                                        )
                                        .child(settings_controls::text(value).text_lg())
                                }),
                            ),
                        ),
                );
            body = body.child(
                h_flex()
                    .justify_between()
                    .child(
                        settings_controls::text("Breakdown")
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM),
                    )
                    .child(
                        div()
                            .id("usage-breakdown-group")
                            .role(accesskit::Role::Group)
                            .aria_label("Usage breakdown")
                            .child(
                                ToggleGroup::new("usage-breakdown")
                                    .segmented()
                                    .single()
                                    .children([
                                        Toggle::new("model")
                                            .label("Model")
                                            .tooltip("Model")
                                            .checked(!self.breakdown_time),
                                        Toggle::new("time")
                                            .label(if self.days == 1 { "Hour" } else { "Day" })
                                            .tooltip(if self.days == 1 { "Hour" } else { "Day" })
                                            .checked(self.breakdown_time),
                                    ])
                                    .on_click(cx.listener(|view, selected: &Vec<bool>, _, cx| {
                                        if selected.iter().any(|selected| *selected) {
                                            view.breakdown_time =
                                                selected.get(1).copied().unwrap_or(false);
                                            cx.notify();
                                        }
                                    })),
                            ),
                    ),
            );
            if self.breakdown_time {
                let providers = PROVIDERS
                    .iter()
                    .filter(|(provider, _, _)| self.merged.providers.contains_key(*provider))
                    .collect::<Vec<_>>();
                let mut table = v_flex()
                    .id("usage-time-breakdown")
                    .role(accesskit::Role::Region)
                    .aria_label("Usage breakdown by time")
                    .child(
                        h_flex()
                            .gap_4()
                            .py_2()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(
                                settings_controls::text(if self.days == 1 {
                                    "Hour"
                                } else {
                                    "Day"
                                })
                                .flex_1(),
                            )
                            .children(providers.iter().map(|(_, label, _)| {
                                settings_controls::text(*label).w(px(100.)).text_right()
                            }))
                            .child(settings_controls::text("Total").w(px(100.)).text_right())
                            .child(settings_controls::text("Tokens").w(px(100.)).text_right()),
                    );
                if self.merged.periods.is_empty() {
                    table =
                        table.child(settings_controls::text("No activity in this window.").py_6());
                }
                for (period, values) in self.merged.periods.iter().rev() {
                    let cost = values.values().map(|value| value.cost).sum();
                    let tokens = values.values().map(Totals::total_tokens).sum();
                    let label = self.period_label(period);
                    table = table.child(
                        h_flex()
                            .id(SharedString::from(format!("usage-time:{period}")))
                            .gap_4()
                            .py_2()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(settings_controls::text(label).flex_1())
                            .children(providers.iter().map(|(provider, _, _)| {
                                div()
                                    .id(SharedString::from(*provider))
                                    .child(settings_controls::text(data::usd(
                                        values.get(*provider).map(|value| value.cost).unwrap_or(0.),
                                    )))
                                    .w(px(100.))
                                    .text_right()
                            }))
                            .child(
                                div()
                                    .id("total")
                                    .child(settings_controls::text(data::usd(cost)))
                                    .w(px(100.))
                                    .text_right(),
                            )
                            .child(
                                settings_controls::text(data::tokens(tokens))
                                    .w(px(100.))
                                    .text_right(),
                            ),
                    );
                }
                body = body.child(table);
            } else {
                let mut models = self.merged.models.iter().collect::<Vec<_>>();
                models.sort_by(|(_, a), (_, b)| {
                    if self.metric == "tokens" {
                        b.total_tokens().total_cmp(&a.total_tokens())
                    } else {
                        b.cost.total_cmp(&a.cost)
                    }
                });
                body =
                    body.children(models.into_iter().enumerate().map(|(index, (key, total))| {
                        let key = key.clone();
                        h_flex()
                            .gap_3()
                            .py_2()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(
                                Button::new(("usage-model", index))
                                    .label(key.1.clone())
                                    .ghost()
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.open_model(&key, window, cx)
                                    })),
                            )
                            .child(div().flex_1())
                            .child(if total.records > 0. && total.unpriced >= total.records {
                                "Unpriced".to_string()
                            } else {
                                data::usd(total.cost)
                            })
                            .child(data::tokens(total.total_tokens()))
                    }));
            }
        }
        v_flex()
            .id("usage-dialog")
            .role(accesskit::Role::Dialog)
            .aria_label("Usage")
            .size_full()
            .track_focus(&self.focus)
            .bg(theme.background)
            .child(
                h_flex()
                    .h(px(48.))
                    .px_4()
                    .gap_3()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().text_sm().child("Usage"))
                    .child(
                        Button::new("usage-environments")
                            .label("Environments")
                            .ghost()
                            .dropdown_menu(move |menu, _, _| {
                                environments.iter().fold(menu, |menu, label| {
                                    let label = label.clone();
                                    let root = menu_root.clone();
                                    menu.item(
                                        PopupMenuItem::new(label.clone())
                                            .checked(selected.contains(&label))
                                            .on_click(move |_, _, cx| {
                                                let _ = root.update(cx, |view, cx| {
                                                    view.filter_environment(label.clone(), cx)
                                                });
                                            }),
                                    )
                                })
                            }),
                    )
                    .child(
                        Button::new("usage-providers")
                            .label("Providers")
                            .ghost()
                            .dropdown_menu(move |menu, _, _| {
                                PROVIDERS.iter().fold(menu, |menu, (provider, label, _)| {
                                    let provider = provider.to_string();
                                    let root = provider_root.clone();
                                    menu.item(
                                        PopupMenuItem::new(*label)
                                            .checked(!hidden.contains(&provider))
                                            .on_click(move |_, _, cx| {
                                                let _ = root.update(cx, |view, cx| {
                                                    view.filter_provider(provider.clone(), cx)
                                                });
                                            }),
                                    )
                                })
                            }),
                    )
                    .child(div().flex_1())
                    .when(self.metric != "limits", |row| {
                        row.child(
                            div()
                                .id("usage-period-group")
                                .role(accesskit::Role::Group)
                                .aria_label("Usage period")
                                .child(
                                    ToggleGroup::new("usage-period")
                                        .segmented()
                                        .single()
                                        .children(PERIODS.iter().map(|&(days, label)| {
                                            Toggle::new(("usage-days", days as usize))
                                                .label(label)
                                                .tooltip(label)
                                                .checked(self.days == days)
                                        }))
                                        .on_click(cx.listener(
                                            |view, selected: &Vec<bool>, _, cx| {
                                                if let Some(index) =
                                                    selected.iter().position(|checked| *checked)
                                                {
                                                    view.choose(None, Some(PERIODS[index].0), cx);
                                                }
                                            },
                                        )),
                                ),
                        )
                    })
                    .child(
                        div()
                            .id("usage-metric-group")
                            .role(accesskit::Role::Group)
                            .aria_label("Usage metric")
                            .child(
                                ToggleGroup::new("usage-metric")
                                    .segmented()
                                    .single()
                                    .children(
                                        [
                                            ("cost", "Cost"),
                                            ("tokens", "Tokens"),
                                            ("limits", "Limits"),
                                        ]
                                        .into_iter()
                                        .map(
                                            |(metric, label)| {
                                                Toggle::new(metric)
                                                    .label(label)
                                                    .tooltip(label)
                                                    .checked(self.metric == metric)
                                            },
                                        ),
                                    )
                                    .on_click(cx.listener(|view, selected: &Vec<bool>, _, cx| {
                                        if let Some(index) =
                                            selected.iter().position(|checked| *checked)
                                        {
                                            view.choose(
                                                Some(["cost", "tokens", "limits"][index]),
                                                None,
                                                cx,
                                            );
                                        }
                                    })),
                            ),
                    )
                    .child(
                        Button::new("refresh-usage")
                            .icon(IconName::RefreshCw)
                            .accessibility_label(if self.metric == "limits" {
                                "Refresh limits"
                            } else {
                                "Refresh usage"
                            })
                            .ghost()
                            .disabled(self.loading)
                            .on_click(cx.listener(|view, _, _, cx| view.refresh(true, cx))),
                    )
                    .child(
                        Button::new("close-usage")
                            .icon(gpui_kit::assets::IconName::X)
                            .accessibility_label("Close Usage")
                            .ghost()
                            .on_click(cx.listener(|view, _, window, cx| {
                                let _ = view.root.update(cx, |root, cx| {
                                    root.usage_page = None;
                                    root.conversation
                                        .composer
                                        .update(cx, |input, cx| input.focus(window, cx));
                                    cx.notify();
                                });
                            })),
                    ),
            )
            .child(
                div()
                    .id("usage-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            )
    }
}

impl Root {
    pub(super) fn toggle_usage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.usage_page.is_none() {
            self.settings_open = false;
            let root = cx.entity().downgrade();
            self.usage_page = Some(cx.new(|cx| usage::UsagePage::new(root, self, window, cx)));
        }
        cx.notify();
    }
    pub(super) fn select_usage(
        &mut self,
        metric: Option<&str>,
        days: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        if let Some(page) = self.usage_page.clone() {
            let metric = metric.map(str::to_string);
            cx.spawn(async move |_, cx| {
                page.update(cx, |page, cx| page.choose(metric.as_deref(), days, cx));
            })
            .detach();
        }
    }
}
