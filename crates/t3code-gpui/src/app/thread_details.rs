use super::*;

impl Root {
    pub(super) fn follow_context_usage(&mut self) {
        let stamp = self.current().and_then(|(environment, id)| {
            self.open.as_ref().map(|open| {
                (
                    environment,
                    id,
                    open.watch.state().lock().expect("thread lock").sequence,
                )
            })
        });
        if stamp == self.context_usage_stamp {
            return;
        }
        self.context_usage_stamp = stamp;
        self.context_usage = self.thread_context_usage();
    }

    pub(super) fn thread_extra_height(&self) -> f32 {
        let usage_rows = self
            .composer_target()
            .and_then(|(environment, model, _, _)| {
                let model = model?;
                let instance = model["instanceId"].as_str()?;
                self.model_catalogs
                    .get(&environment)
                    .map(|(config, _)| models::usage(config, instance).len())
            })
            .unwrap_or(0)
            + usize::from(self.context_usage.is_some());
        let links = self
            .current()
            .and_then(|(environment, id)| self.thread_value(&environment, &id))
            .map(|thread| {
                thread["pullRequests"].as_array().map_or(0, |links| {
                    links
                        .iter()
                        .filter(|link| link["source"].as_str() != Some("stack-dismissed"))
                        .count()
                })
            })
            .unwrap_or(0);
        (usage_rows as f32 * 24. + if usage_rows > 0 { 16. } else { 0. }) + links as f32 * 34.
    }

    pub(super) fn thread_context_usage(&self) -> Option<Value> {
        let state = self.open.as_ref()?.watch.state();
        let state = state.lock().expect("thread lock");
        let projection = &state.projection;
        let current = projection.get("thread")?["activeProviderThreadId"].as_str();
        let live = projection
            .get("providerTurns")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|turn| turn.get("tokenUsage").filter(|usage| usage.is_object()))
            .max_by_key(|usage| {
                usage["updatedAt"]
                    .as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|d| d.timestamp_millis())
            });
        if let Some(live) = live {
            return Some(live.clone());
        }
        if let Some(usage) = projection
            .get("providerThreads")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|thread| thread["id"].as_str() == current)
            .and_then(|thread| thread.get("contextUsage"))
            .filter(|usage| usage.is_object())
        {
            return Some(usage.clone());
        }
        let thread = projection.get("thread")?;
        let forked = &thread["forkedFrom"];
        if forked["type"].as_str() == Some("run")
            && let Some(usage) = projection
                .get("contextTransfers")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|transfer| {
                    transfer["type"].as_str() == Some("fork")
                        && transfer["targetThreadId"] == thread["id"]
                        && transfer["sourceThreadId"] == forked["threadId"]
                        && transfer["sourcePoint"]["runId"] == forked["runId"]
                        && transfer["sourceProviderInstanceId"]
                            == thread["modelSelection"]["instanceId"]
                        && (transfer["status"].as_str() == Some("pending")
                            || transfer["resolution"]["strategy"].as_str() == Some("native_fork"))
                })
                .and_then(|transfer| transfer.get("sourceTokenUsage"))
                .filter(|usage| usage.is_object())
        {
            return Some(usage.clone());
        }
        projection
            .get("turnItems")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .rev()
            .find(|item| {
                item["type"].as_str() == Some("compaction")
                    && item["afterTokenCount"].as_f64().is_some_and(|n| n >= 0.)
            })
            .map(|item| json!({"usedTokens": item["afterTokenCount"]}))
    }

    pub(super) fn render_thread_usage(&self, cx: &Context<Self>) -> AnyElement {
        let mut panel = v_flex().gap_2().px_3().py_2();
        if let Some((environment, Some(model), _, _)) = self.composer_target()
            && let Some((config, _)) = self.model_catalogs.get(&environment)
            && let Some(instance) = model["instanceId"].as_str()
        {
            for (label, percent) in models::usage(config, instance) {
                let remaining = (100. - percent.clamp(0., 100.)).round();
                panel = panel.child(usage_row(
                    label,
                    format!("{remaining:.0}%"),
                    Some(remaining),
                    true,
                    cx,
                ));
            }
        }
        if let Some(usage) = &self.context_usage {
            let used = usage["usedTokens"].as_f64().unwrap_or(0.).max(0.);
            let max = usage["maxTokens"].as_f64().filter(|max| *max > 0.);
            let label = max
                .map(|max| format!("{} / {}", format_tokens(used), format_tokens(max)))
                .unwrap_or_else(|| format_tokens(used));
            panel = panel.child(usage_row(
                "Context".into(),
                label,
                max.map(|max| (100. * used / max).min(100.)),
                false,
                cx,
            ));
        }
        panel.into_any_element()
    }

    pub(super) fn render_thread_pull_requests(&self, cx: &Context<Self>) -> AnyElement {
        let Some((environment, id)) = self.current() else {
            return div().into_any_element();
        };
        let Some(thread) = self.thread_value(&environment, &id) else {
            return div().into_any_element();
        };
        let links = thread["pullRequests"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut panel = v_flex().px_2().gap_1();
        for link in links
            .into_iter()
            .filter(|link| link["source"].as_str() != Some("stack-dismissed"))
        {
            let mut entry = link.clone();
            entry["projectId"] = thread["projectId"].clone();
            let environment = environment.clone();
            let label = format!(
                "#{} {}",
                link["number"],
                link["snapshot"]["title"].as_str().unwrap_or("Pull request")
            );
            panel = panel.child(
                Button::new(SharedString::from(format!("linked-pr-{}", link["number"])))
                    .label(label)
                    .ghost()
                    .w_full()
                    .on_click(cx.listener(move |root, _, _, cx| {
                        root.pr_state.page = false;
                        root.open_pull_request(environment.clone(), entry.clone(), cx);
                    })),
            );
        }
        panel.into_any_element()
    }
}

fn usage_row(
    label: String,
    value: String,
    percent: Option<f64>,
    remaining: bool,
    cx: &Context<Root>,
) -> AnyElement {
    let theme = cx.theme();
    let danger = percent.is_some_and(|p| if remaining { p <= 10. } else { p >= 90. });
    let warning = percent.is_some_and(|p| if remaining { p <= 20. } else { p >= 80. });
    let color = if danger {
        theme.danger
    } else if warning {
        theme.warning_foreground
    } else {
        theme.muted_foreground
    };
    h_flex()
        .gap_2()
        .text_xs()
        .child(div().w(px(64.)).child(label.clone()))
        .child(
            div()
                .id(SharedString::from(format!("usage-{label}")))
                .role(accesskit::Role::ProgressIndicator)
                .aria_label(format!(
                    "{label}: {value}{}",
                    if remaining { " left" } else { " used" }
                ))
                .flex_1()
                .h(px(3.))
                .rounded_full()
                .bg(theme.muted)
                .children(percent.map(|p| {
                    div()
                        .h_full()
                        .w(relative((p / 100.) as f32))
                        .rounded_full()
                        .bg(color)
                })),
        )
        .child(div().text_color(color).child(value))
        .into_any_element()
}

fn format_tokens(value: f64) -> String {
    if !value.is_finite() {
        return "0".into();
    }
    if value < 1000. {
        return format!("{:.0}", value.round());
    }
    if value < 10000. {
        return format!("{}k", fixed_one(value / 1000.));
    }
    if value < 999500. {
        return format!("{:.0}k", (value / 1000.).round());
    }
    format!("{}m", fixed_one(value / 1000000.))
}

fn fixed_one(value: f64) -> String {
    let fraction = value.fract();
    let value = if fraction == 0.25 || fraction == 0.75 {
        (value * 10.).round() / 10.
    } else {
        value
    };
    format!("{value:.1}").trim_end_matches(".0").to_string()
}

#[cfg(test)]
mod tests {
    use super::format_tokens;

    #[test]
    fn context_token_labels_match_reference_boundaries() {
        for (tokens, label) in [
            (0., "0"),
            (999., "999"),
            (1000., "1k"),
            (1500., "1.5k"),
            (1250., "1.3k"),
            (1150., "1.1k"),
            (9999., "10k"),
            (10000., "10k"),
            (999499., "999k"),
            (999500., "1m"),
            (1500000., "1.5m"),
        ] {
            assert_eq!(format_tokens(tokens), label);
        }
    }
}
