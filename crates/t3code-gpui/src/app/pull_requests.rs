use super::*;

#[derive(Default)]
pub(super) struct State {
    pub page: bool,
    pub page_panel_open: bool,
    pub selected: Option<String>,
    pub list_state: String,
    pub filters: HashMap<String, String>,
    pub sort: String,
    pub entries: Vec<Value>,
    pub cursors: HashMap<String, Value>,
    pub loading: bool,
    pub error: Option<String>,
    pub request: u64,
    pub views: HashMap<String, Detail>,
    pub submitted_comment: Option<String>,
}

pub(super) struct Detail {
    environment: String,
    reference: Value,
    value: Value,
    activity: Value,
    body: Option<Entity<TextViewState>>,
    comments: Vec<Entity<TextViewState>>,
    diff: Option<Entity<DiffState>>,
    tab: &'static str,
    loading: bool,
    busy: bool,
    request: u64,
    error: Option<String>,
}

pub(super) struct Pane {
    pub root: WeakEntity<Root>,
}
impl Render for Pane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.root
            .update(cx, |root, cx| root.render_pull_requests(cx))
            .expect("pull requests root")
    }
}

impl Root {
    fn render_pr_filters(&self, cx: &Context<Self>) -> AnyElement {
        use gpui_kit::component::menu::PopupMenuItem;
        let weak = cx.entity().downgrade();
        let sort_root = weak.clone();
        let filters_root = weak.clone();
        let current_sort = self.pr_state.sort.clone();
        let current_state = self.pr_state.list_state.clone();
        let current_filters = self.pr_state.filters.clone();
        h_flex()
            .gap_2()
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.pr_search)
                        .role(accesskit::Role::SearchInput)
                        .aria_label("Search pull requests")
                        .prefix(Icon::new(IconName::Search).size(px(16.))),
                ),
            )
            .child(
                Button::new("pr-sort")
                    .outline()
                    .label("Sort")
                    .icon(gpui_kit::assets::IconName::ArrowDownUp)
                    .accessibility_label(format!(
                        "Sort pull requests: {}",
                        sort_label(&current_sort)
                    ))
                    .dropdown_menu(move |mut menu, _, _| {
                        for (value, label) in SORTS {
                            let weak = sort_root.clone();
                            menu = menu.item(
                                PopupMenuItem::new(*label)
                                    .checked(
                                        current_sort == *value
                                            || current_sort.is_empty() && *value == "ready",
                                    )
                                    .on_click(move |_, _, cx| {
                                        let _ = weak.update(cx, |root, cx| {
                                            root.pr_state.sort = (*value).into();
                                            root.panel_revision += 1;
                                            cx.notify();
                                        });
                                    }),
                            );
                        }
                        menu.radio_group()
                    }),
            )
            .child(
                Button::new("pr-filters")
                    .outline()
                    .label("Filters")
                    .icon(gpui_kit::assets::IconName::ListFilter)
                    .dropdown_menu(move |mut menu, window, cx| {
                        for (key, label, options) in FILTERS {
                            let selected = if *key == "state" {
                                current_state.clone()
                            } else {
                                current_filters.get(*key).cloned().unwrap_or_default()
                            };
                            let weak = filters_root.clone();
                            let current_label = options
                                .iter()
                                .find(|(value, _)| *value == selected)
                                .map(|(_, label)| *label)
                                .unwrap_or(options[0].1);
                            menu = menu.submenu(
                                format!("{label} {current_label}"),
                                window,
                                cx,
                                move |mut menu, _, _| {
                                    for (value, label) in *options {
                                        let weak = weak.clone();
                                        menu = menu.item(
                                            PopupMenuItem::new(*label)
                                                .checked(selected == *value)
                                                .on_click(move |_, _, cx| {
                                                    let _ = weak.update(cx, |root, cx| {
                                                        if *key == "state" {
                                                            root.pr_state.list_state =
                                                                (*value).into();
                                                        } else if value.is_empty() {
                                                            root.pr_state.filters.remove(*key);
                                                        } else {
                                                            root.pr_state.filters.insert(
                                                                (*key).into(),
                                                                (*value).into(),
                                                            );
                                                        }
                                                        root.load_pull_requests(false, cx);
                                                    });
                                                }),
                                        );
                                    }
                                    menu.radio_group()
                                },
                            );
                        }
                        menu
                    }),
            )
            .child(
                Button::new("pr-provider")
                    .outline()
                    .label("All")
                    .icon(gpui_kit::assets::IconName::Plug2)
                    .accessibility_label("Filter by provider: All")
                    .dropdown_menu(move |menu, _, _| {
                        let weak = weak.clone();
                        menu.item(PopupMenuItem::new("All").checked(true).on_click(
                            move |_, _, cx| {
                                let _ = weak.update(cx, |root, cx| {
                                    root.pr_state.filters.remove("host");
                                    root.load_pull_requests(false, cx);
                                });
                            },
                        ))
                    }),
            )
            .child(
                Button::new("pr-refresh")
                    .outline()
                    .icon(IconName::RefreshCw)
                    .accessibility_label("Refresh pull requests")
                    .disabled(self.pr_state.loading)
                    .on_click(cx.listener(|root, _, _, cx| root.load_pull_requests(false, cx))),
            )
            .into_any_element()
    }

    fn render_pr_empty(&self, cx: &Context<Self>) -> AnyElement {
        let query = self.pr_search.read(cx).value().trim().to_string();
        let filtered = !self.pr_state.filters.is_empty()
            || !matches!(self.pr_state.list_state.as_str(), "" | "open");
        let title = if !query.is_empty() {
            format!(
                "Nothing matches “{}{}”",
                query.chars().take(48).collect::<String>(),
                if query.chars().count() > 48 {
                    "…"
                } else {
                    ""
                }
            )
        } else if filtered {
            "Nothing under these filters".into()
        } else {
            "No pull requests".into()
        };
        let description = if !query.is_empty() {
            "The hosts were searched for it. Try fewer words, or search by number, author or branch."
        } else if filtered {
            "Widen the state, involvement or project filter to see more."
        } else {
            "Pull requests from every project in this workspace appear here."
        };
        v_flex()
            .flex_1()
            .size_full()
            .items_center()
            .justify_center()
            .gap_6()
            .child(
                svg()
                    .path("icons/pr-empty.svg")
                    .w(px(128.))
                    .h(px(80.))
                    .text_color(cx.theme().muted_foreground.opacity(0.6)),
            )
            .child(
                v_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(text!(title)),
                    )
                    .child(
                        div()
                            .max_w(px(384.))
                            .text_center()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(text!(description)),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .when(!query.is_empty(), |row| {
                        row.child(
                            Button::new("pr-clear-search")
                                .outline()
                                .icon(IconName::Search)
                                .label("Clear search")
                                .small()
                                .on_click(cx.listener(|root, _, window, cx| {
                                    root.pr_search
                                        .update(cx, |input, cx| input.set_value("", window, cx));
                                    root.load_pull_requests(false, cx);
                                })),
                        )
                    })
                    .child(
                        Button::new("pr-check-again")
                            .outline()
                            .icon(IconName::RefreshCw)
                            .label("Check again")
                            .small()
                            .on_click(
                                cx.listener(|root, _, _, cx| root.load_pull_requests(false, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn open_pull_requests(&mut self, page: bool, cx: &mut Context<Self>) {
        self.pr_state.page = page;
        self.pr_state.selected = None;
        if !page {
            self.activate_panel("pull-requests".into(), cx);
        }
        self.load_pull_requests(false, cx);
    }

    pub(super) fn open_current_pull_request(&mut self, cx: &mut Context<Self>) {
        let Some((environment, id)) = self.current() else {
            return;
        };
        let Some(thread) = self.thread_value(&environment, &id) else {
            return;
        };
        if let Some(link) = thread["pullRequests"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|link| link["source"].as_str() != Some("stack-dismissed"))
        {
            let mut link = link.clone();
            link["projectId"] = thread["projectId"].clone();
            self.pr_state.page = false;
            self.open_pull_request(environment, link, cx);
        } else {
            self.open_pull_requests(false, cx);
        }
    }

    pub(super) fn load_pull_requests(&mut self, more: bool, cx: &mut Context<Self>) {
        if !self.pr_state.page {
            self.pr_state.entries.clear();
            self.pr_state.error = None;
            self.pr_state.loading = false;
            self.pr_state.request += 1;
            self.pr_state.cursors.clear();
            if let Some((environment, id)) = self.current()
                && let Some(thread) = self.thread_value(&environment, &id)
            {
                self.pr_state.entries = thread["pullRequests"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|link| link["source"].as_str() != Some("stack-dismissed"))
                    .map(|link| {
                        let mut entry = link.clone();
                        if let Some(snapshot) = link["snapshot"].as_object() {
                            entry
                                .as_object_mut()
                                .expect("PR link")
                                .extend(snapshot.clone());
                        }
                        entry["projectId"] = thread["projectId"].clone();
                        entry["environment"] = json!(environment);
                        entry
                    })
                    .collect();
            }
            self.panel_revision += 1;
            cx.notify();
            return;
        }
        self.pr_state.request += 1;
        let request = self.pr_state.request;
        self.pr_state.loading = true;
        self.pr_state.error = None;
        if !more {
            self.pr_state.entries.clear();
            self.pr_state.cursors.clear();
        }
        let state = if self.pr_state.list_state.is_empty() {
            "open"
        } else {
            &self.pr_state.list_state
        }
        .to_string();
        let query = self.pr_search.read(cx).value().trim().to_string();
        let filters = self.pr_state.filters.clone();
        let cursors = self.pr_state.cursors.clone();
        let connections = self.connections.clone();
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                let mut results = Vec::new();
                for connection in connections {
                    let mut input = json!({"state":state,"limit":99});
                    let mut qualifiers = serde_json::Map::new();
                    for (key, value) in &filters {
                        if matches!(key.as_str(), "involvement" | "host" | "projectId") {
                            input[key] = json!(value);
                        } else {
                            qualifiers.insert(key.clone(), json!(value));
                        }
                    }
                    if !qualifiers.is_empty() {
                        input["filters"] = Value::Object(qualifiers);
                    }
                    if !query.is_empty() {
                        input["query"] = json!(query);
                    }
                    if more {
                        let Some(cursor) = cursors.get(connection.label()) else {
                            continue;
                        };
                        input["cursors"] = cursor.clone();
                    }
                    results.push((
                        connection.label().to_string(),
                        connection.request("pullRequests.list", input).await,
                    ));
                }
                Ok(results)
            })
            .await;
            let _ = this.update(cx, |root, cx| {
                if root.pr_state.request != request {
                    return;
                }
                root.pr_state.loading = false;
                let mut errors = Vec::new();
                match result {
                    Ok(results) => {
                        for (environment, result) in results {
                            match result {
                                Ok(value) => {
                                    for mut entry in
                                        value["entries"].as_array().cloned().unwrap_or_default()
                                    {
                                        entry["environment"] = json!(environment);
                                        root.pr_state.entries.push(entry);
                                    }
                                    if let Some(next) = value["nextCursors"]
                                        .as_object()
                                        .filter(|next| !next.is_empty())
                                    {
                                        root.pr_state
                                            .cursors
                                            .insert(environment.clone(), json!(next));
                                    } else {
                                        root.pr_state.cursors.remove(&environment);
                                    }
                                    for error in value["errors"].as_array().into_iter().flatten() {
                                        errors.push(format!(
                                            "{}: {}",
                                            error["projectTitle"].as_str().unwrap_or(&environment),
                                            error["message"].as_str().unwrap_or_default()
                                        ));
                                    }
                                }
                                Err(error) => errors.push(format!("{environment}: {error:#}")),
                            }
                        }
                    }
                    Err(error) => errors.push(format!("{error:#}")),
                }
                root.pr_state
                    .entries
                    .sort_by(|a, b| b["updatedAt"].as_str().cmp(&a["updatedAt"].as_str()));
                root.pr_state.error = (!errors.is_empty()).then(|| errors.join("\n"));
                root.panel_revision += 1;
                cx.notify();
            });
        })
        .detach();
        self.panel_revision += 1;
        cx.notify();
    }

    pub(super) fn open_pull_request(
        &mut self,
        environment: String,
        entry: Value,
        cx: &mut Context<Self>,
    ) {
        let reference = reference(&entry);
        let id = format!(
            "pull-request:{environment}:{}:{}",
            reference["repository"].as_str().unwrap_or_default(),
            reference["number"]
        );
        if !self.pr_state.page {
            self.right_panel
                .pull_requests
                .insert(id.clone(), (environment.clone(), reference.clone()));
            self.activate_panel(id.clone(), cx);
        }
        self.pr_state.selected = Some(id.clone());
        self.pr_state.page_panel_open = true;
        self.pr_state.views.entry(id.clone()).or_insert(Detail {
            environment,
            reference,
            value: Value::Null,
            activity: Value::Null,
            body: None,
            comments: Vec::new(),
            diff: None,
            tab: "summary",
            loading: false,
            busy: false,
            request: 0,
            error: None,
        });
        self.load_pull_request(id, cx);
    }

    fn load_pull_request(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(view) = self.pr_state.views.get_mut(&id) else {
            return;
        };
        view.loading = true;
        view.request += 1;
        let request = view.request;
        view.error = None;
        let environment = view.environment.clone();
        let reference = view.reference.clone();
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                let (detail, activity, checks) = tokio::join!(
                    connection.request("pullRequests.detail", reference.clone()),
                    connection.request("pullRequests.activity", reference.clone()),
                    connection.request("pullRequests.checks", reference)
                );
                Ok((detail?, activity, checks))
            })
            .await;
            let _ = this.update(cx, |root, cx| {
                let Some(view) = root.pr_state.views.get_mut(&id) else {
                    return;
                };
                if view.request != request {
                    return;
                }
                view.loading = false;
                match result {
                    Ok((mut detail, activity, checks)) => {
                        let mut errors = Vec::new();
                        match checks {
                            Ok(checks) if !checks.is_null() => {
                                detail["checks"] = checks["checks"].clone()
                            }
                            Err(error) => errors.push(format!("Checks: {error:#}")),
                            _ => {}
                        }
                        match activity {
                            Ok(activity) => view.activity = activity,
                            Err(error) => errors.push(format!("Activity: {error:#}")),
                        }
                        view.body = Some(cx.new(|cx| {
                            TextViewState::markdown(detail["body"].as_str().unwrap_or_default(), cx)
                        }));
                        view.comments = view.activity["comments"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|comment| {
                                cx.new(|cx| {
                                    TextViewState::markdown(
                                        comment["body"].as_str().unwrap_or_default(),
                                        cx,
                                    )
                                })
                            })
                            .collect();
                        view.value = detail;
                        view.error = (!errors.is_empty()).then(|| errors.join("\n"));
                    }
                    Err(error) => view.error = Some(format!("{error:#}")),
                }
                root.panel_revision += 1;
                root.workspace_revision += 1;
                cx.notify();
            });
        })
        .detach();
        self.panel_revision += 1;
        cx.notify();
    }

    fn pull_request_id(&self) -> Option<String> {
        if self.pr_state.page {
            self.pr_state.selected.clone()
        } else {
            self.right_panel
                .active
                .clone()
                .filter(|id| id.starts_with("pull-request:"))
        }
    }

    pub(super) fn copy_pull_request_number(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.pull_request_id() else {
            return;
        };
        let Some(view) = self.pr_state.views.get(&id) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(format!(
            "#{}",
            view.reference["number"]
        )));
    }

    fn select_pr_tab(&mut self, tab: &'static str, cx: &mut Context<Self>) {
        let Some(id) = self.pull_request_id() else {
            return;
        };
        let Some(view) = self.pr_state.views.get_mut(&id) else {
            return;
        };
        view.tab = tab;
        if tab == "code" && view.diff.is_none() {
            let reference = view.reference.clone();
            let environment = view.environment.clone();
            let Some(connection) = self.connection(&environment) else {
                return;
            };
            let runtime = self.runtime.clone();
            cx.spawn(async move |this, cx| {
                let result = on_runtime(&runtime, async move {
                    connection.pull_request_diff(reference).await
                })
                .await;
                let result = match result {
                    Ok(value) => {
                        cx.background_spawn(async move {
                            let files = DiffFile::parse_with(
                                value["patch"].as_str().unwrap_or_default(),
                                DiffParseOptions::new().truncated_hunks(true),
                            )
                            .map_err(|error| anyhow::anyhow!("Diff parse error: {error:?}"))?;
                            Ok((
                                files,
                                value["truncated"].as_bool() == Some(true),
                                value["nextCursor"].is_string(),
                            ))
                        })
                        .await
                    }
                    Err(error) => Err(error),
                };
                let _ = this.update(cx, |root, cx| {
                    let Some(view) = root.pr_state.views.get_mut(&id) else {
                        return;
                    };
                    match result {
                        Ok((files, truncated, more)) => {
                            view.diff = Some(cx.new(|cx| DiffState::new(files, cx)));
                            if truncated || more {
                                view.error = Some("The host returned a partial diff.".into());
                            }
                        }
                        Err(error) => view.error = Some(format!("{error:#}")),
                    }
                    root.panel_revision += 1;
                    cx.notify();
                });
            })
            .detach();
        }
        self.panel_revision += 1;
        cx.notify();
    }

    fn mutate_pr(&mut self, method: &'static str, additions: Value, cx: &mut Context<Self>) {
        let Some(id) = self.pull_request_id() else {
            return;
        };
        let Some(view) = self.pr_state.views.get_mut(&id) else {
            return;
        };
        if view.busy {
            return;
        }
        view.busy = true;
        let mut input = view.reference.clone();
        input
            .as_object_mut()
            .expect("PR reference")
            .extend(additions.as_object().expect("PR action").clone());
        let environment = view.environment.clone();
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let submitted = (method == "pullRequests.comment")
            .then(|| additions["body"].as_str().unwrap_or_default().to_string());
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let result = on_runtime(
                &runtime,
                async move { connection.request(method, input).await },
            )
            .await;
            let _ = this.update(cx, |root, cx| {
                let Some(view) = root.pr_state.views.get_mut(&id) else {
                    return;
                };
                view.busy = false;
                match result {
                    Ok(_) => {
                        if root.pull_request_id().as_ref() == Some(&id) {
                            root.pr_state.submitted_comment = submitted;
                        }
                        root.load_pull_request(id, cx);
                    }
                    Err(error) => view.error = Some(format!("{error:#}")),
                }
                root.panel_revision += 1;
                cx.notify();
            });
        })
        .detach();
        self.panel_revision += 1;
        cx.notify();
    }

    fn confirm_pr_action(
        &mut self,
        action: &'static str,
        method: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.pull_request_id() else {
            return;
        };
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let weak = weak.clone();
            let method = method.clone();
            let id = id.clone();
            dialog
                .title(format!("{} pull request", action_label(action)))
                .footer(git_controls::dialog_footer(action_label(action)))
                .child(text!("Confirm this action on the pull request host."))
                .on_ok(move |_, _, cx| {
                    let _ = weak.update(cx, |root, cx| {
                        if root.pull_request_id().as_ref() != Some(&id) {
                            return;
                        }
                        let mut value = json!({"action":action});
                        if let Some(method) = &method {
                            value["mergeMethod"] = json!(method);
                        }
                        root.mutate_pr("pullRequests.runAction", value, cx);
                    });
                    true
                })
        });
    }

    pub(super) fn render_pull_requests(&self, cx: &Context<Self>) -> AnyElement {
        if !self.pr_state.page
            && let Some(id) = self.pull_request_id()
            && let Some(view) = self.pr_state.views.get(&id)
        {
            return self.render_pr_detail(view, cx);
        }
        let mut body = v_flex()
            .id("pull-request-list")
            .role(accesskit::Role::Region)
            .aria_label("Pull requests")
            .size_full()
            .min_h_0()
            .gap_4()
            .px_6()
            .pt_6()
            .pb_12()
            .child(self.render_pr_filters(cx))
            .children(
                self.pr_state
                    .error
                    .clone()
                    .map(|error| div().text_color(cx.theme().danger).child(error)),
            );
        let mut list = v_flex()
            .id("pr-entries")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        let mut entries = self.pr_state.entries.iter().collect::<Vec<_>>();
        entries.sort_by(|a, b| compare_entries(a, b, &self.pr_state.sort));
        for entry in entries {
            let label = format!(
                "#{} {}",
                entry["number"],
                entry["title"].as_str().unwrap_or_default()
            );
            let entry = entry.clone();
            list = list.child(
                Button::new(SharedString::from(format!(
                    "{}:{}:{}",
                    entry["environment"], entry["repository"], entry["number"]
                )))
                .child(
                    v_flex().items_start().child(label.clone()).child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "{} · {} · {}",
                                entry["repository"].as_str().unwrap_or_default(),
                                entry["state"].as_str().unwrap_or_default(),
                                entry["author"]["login"].as_str().unwrap_or_default()
                            )),
                    ),
                )
                .accessibility_label(label)
                .ghost()
                .w_full()
                .h_auto()
                .py_2()
                .on_click(cx.listener(move |root, _, _, cx| {
                    root.open_pull_request(
                        entry["environment"].as_str().unwrap_or_default().into(),
                        entry.clone(),
                        cx,
                    )
                })),
            );
        }
        if self.pr_state.loading {
            list = list.child(div().h(px(160.)));
        } else if self.pr_state.entries.is_empty() && self.pr_state.error.is_none() {
            list = list.child(self.render_pr_empty(cx));
        }
        body = body.child(list);
        if !self.pr_state.cursors.is_empty() {
            body = body.child(
                Button::new("more-prs")
                    .label("Load more pull requests")
                    .on_click(cx.listener(|root, _, _, cx| root.load_pull_requests(true, cx))),
            );
        }
        if self.pr_state.page
            && self.pr_state.page_panel_open
            && let Some(view) = self
                .pr_state
                .selected
                .as_ref()
                .and_then(|id| self.pr_state.views.get(id))
        {
            return h_flex()
                .size_full()
                .child(body.flex_1().w_0())
                .child(
                    div()
                        .w(px(self.right_panel_width))
                        .h_full()
                        .border_l_1()
                        .border_color(cx.theme().border)
                        .child(self.render_pr_detail(view, cx)),
                )
                .into_any_element();
        }
        body.into_any_element()
    }

    fn render_pr_detail(&self, view: &Detail, cx: &Context<Self>) -> AnyElement {
        let value = &view.value;
        let mut content = v_flex()
            .id("pr-detail-content")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap_3()
            .p_4();
        if view.loading {
            content = content.child(text!("Loading pull request…"));
        }
        if let Some(error) = &view.error {
            content = content.child(div().text_color(cx.theme().danger).child(error.clone()));
        }
        match view.tab {
            "code" => {
                if let Some(diff) = &view.diff {
                    content = content.child(Diff::new(diff));
                }
            }
            "timeline" => {
                for (index, body) in view.comments.iter().enumerate() {
                    let comment = &view.activity["comments"][index];
                    content = content.child(
                        v_flex()
                            .gap_1()
                            .child(div().text_xs().child(format!(
                                "{} · {}",
                                comment["author"]["login"].as_str().unwrap_or_default(),
                                comment["createdAt"].as_str().unwrap_or_default()
                            )))
                            .child(TextView::new(body).selectable(true)),
                    );
                }
                for commit in view.activity["commits"].as_array().into_iter().flatten() {
                    content = content.child(
                        div().text_sm().child(
                            commit["messageHeadline"]
                                .as_str()
                                .unwrap_or_default()
                                .to_string(),
                        ),
                    );
                }
                if value["viewerPermissions"]["comment"].as_bool() == Some(true) {
                    content = content.child(Textarea::new(&self.pr_comment)).child(
                        Button::new("post-pr-comment")
                            .label("Comment")
                            .disabled(view.busy)
                            .on_click(cx.listener(|root, _, _, cx| {
                                let body = root.pr_comment.read(cx).value().to_string();
                                if !body.trim().is_empty() {
                                    root.mutate_pr(
                                        "pullRequests.comment",
                                        json!({"body":body}),
                                        cx,
                                    );
                                }
                            })),
                    );
                }
            }
            _ => {
                if let Some(body) = &view.body {
                    content = content.child(TextView::new(body).selectable(true));
                }
                content = content.child(div().text_sm().child(format!(
                    "{} ← {}   +{} −{}",
                    value["baseBranch"].as_str().unwrap_or_default(),
                    value["headBranch"].as_str().unwrap_or_default(),
                    value["additions"],
                    value["deletions"]
                )));
                for check in value["checks"].as_array().into_iter().flatten() {
                    let url = check["url"].as_str().map(str::to_string);
                    content = content.child(
                        Button::new(SharedString::from(format!("check-{}", check["name"])))
                            .label(format!(
                                "{} · {}",
                                check["status"].as_str().unwrap_or_default(),
                                check["name"].as_str().unwrap_or_default()
                            ))
                            .ghost()
                            .on_click(move |_, _, cx| {
                                if let Some(url) = &url {
                                    cx.open_url(url);
                                }
                            }),
                    );
                }
                let actions = value["viewerPermissions"]["actions"].as_array();
                for action in [
                    "merge",
                    "ready",
                    "draft",
                    "close",
                    "reopen",
                    "update-branch",
                    "delete-branch",
                    "enable-auto-merge",
                    "disable-auto-merge",
                ] {
                    if actions
                        .is_some_and(|actions| actions.iter().any(|a| a.as_str() == Some(action)))
                    {
                        if action == "merge" {
                            for method in value["capabilities"]["mergeMethods"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str)
                            {
                                if value["mergeCapabilities"][method].as_bool() != Some(true) {
                                    continue;
                                }
                                let method = method.to_string();
                                content = content.child(
                                    Button::new(SharedString::from(format!("merge-{method}")))
                                        .label(format!("Merge ({method})"))
                                        .disabled(view.busy)
                                        .on_click(cx.listener(move |root, _, window, cx| {
                                            root.confirm_pr_action(
                                                "merge",
                                                Some(method.clone()),
                                                window,
                                                cx,
                                            )
                                        })),
                                );
                            }
                        } else {
                            content = content.child(
                                Button::new(action)
                                    .label(action_label(action))
                                    .disabled(view.busy)
                                    .on_click(cx.listener(move |root, _, window, cx| {
                                        root.confirm_pr_action(action, None, window, cx)
                                    })),
                            );
                        }
                    }
                }
            }
        }
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                h_flex()
                    .p_3()
                    .gap_2()
                    .child(
                        Button::new("pr-back")
                            .label("Back")
                            .ghost()
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.open_pull_requests(root.pr_state.page, cx);
                            })),
                    )
                    .child(
                        Button::new("copy-pr-number")
                            .label(format!("#{}", view.reference["number"]))
                            .accessibility_label("Copy pull request number")
                            .ghost()
                            .on_click(
                                cx.listener(|root, _, _, cx| root.copy_pull_request_number(cx)),
                            ),
                    )
                    .child(
                        div().flex_1().child(
                            value["title"]
                                .as_str()
                                .unwrap_or("Pull request")
                                .to_string(),
                        ),
                    )
                    .child(Button::new("refresh-pr").label("Refresh").ghost().on_click(
                        cx.listener(|root, _, _, cx| {
                            if let Some(id) = root.pull_request_id() {
                                root.load_pull_request(id, cx);
                            }
                        }),
                    )),
            )
            .child(
                div()
                    .px_4()
                    .text_sm()
                    .child(if value["isDraft"].as_bool() == Some(true) {
                        "Draft".to_string()
                    } else {
                        value["state"].as_str().unwrap_or_default().to_string()
                    }),
            )
            .child(
                h_flex()
                    .id("pr-tabs")
                    .aria_label("Pull request tabs")
                    .p_2()
                    .gap_1()
                    .children(
                        [
                            ("summary", "Summary"),
                            ("timeline", "Timeline"),
                            ("code", "Code"),
                        ]
                        .into_iter()
                        .map(|(tab, label)| {
                            Button::new(label)
                                .label(label)
                                .ghost()
                                .selected(view.tab == tab)
                                .on_click(
                                    cx.listener(move |root, _, _, cx| root.select_pr_tab(tab, cx)),
                                )
                        }),
                    ),
            )
            .child(content)
            .into_any_element()
    }
}

fn reference(entry: &Value) -> Value {
    let mut value = json!({"projectId":entry["projectId"], "repository":entry["repository"], "number":entry["number"]});
    if let Some(host) = entry["host"].as_str() {
        value["host"] = json!(host);
    }
    value
}

fn action_label(action: &str) -> &'static str {
    match action {
        "merge" => "Merge",
        "ready" => "Mark ready for review",
        "draft" => "Convert to draft",
        "close" => "Close",
        "reopen" => "Reopen",
        "update-branch" => "Update branch",
        "delete-branch" => "Delete branch",
        "enable-auto-merge" => "Enable auto-merge",
        "disable-auto-merge" => "Disable auto-merge",
        _ => "Confirm",
    }
}

const SORTS: &[(&str, &str)] = &[
    ("ready", "Merge readiness"),
    ("blocked", "Blocked on me"),
    ("updated", "Recently updated"),
    ("newest", "Newest shown"),
    ("oldest", "Oldest shown"),
    ("largest", "Largest shown"),
    ("smallest", "Smallest shown"),
];

type FilterOptions = (
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
);
const FILTERS: &[FilterOptions] = &[
    (
        "state",
        "State",
        &[
            ("all", "All"),
            ("", "Open"),
            ("closed", "Closed"),
            ("merged", "Merged"),
        ],
    ),
    (
        "involvement",
        "Involvement",
        &[
            ("", "All"),
            ("reviewing", "Reviewing"),
            ("authored", "Authored"),
        ],
    ),
    (
        "draft",
        "Draft",
        &[
            ("", "All"),
            ("only", "Drafts only"),
            ("hide", "Hide drafts"),
        ],
    ),
    (
        "review",
        "Review",
        &[
            ("", "All"),
            ("approved", "Approved"),
            ("changes-requested", "Changes requested"),
            ("review-required", "Review required"),
            ("none", "No reviews"),
        ],
    ),
    (
        "checks",
        "Checks",
        &[("", "All"), ("passing", "Passing"), ("failing", "Failing")],
    ),
];

fn sort_label(sort: &str) -> &'static str {
    SORTS
        .iter()
        .find(|(value, _)| *value == sort)
        .unwrap_or(&SORTS[0])
        .1
}

fn compare_entries(a: &Value, b: &Value, sort: &str) -> std::cmp::Ordering {
    let size = |entry: &Value| {
        entry["additions"].as_i64().unwrap_or(0) + entry["deletions"].as_i64().unwrap_or(0)
    };
    let ready = |entry: &Value| {
        if entry["mergeability"] == "conflicting" {
            4
        } else if entry["state"] != "open" {
            3
        } else if entry["isDraft"] == true {
            2
        } else if entry["checksState"] == "passing" && entry["reviewDecision"] == "approved" {
            0
        } else if entry["checksState"] == "passing" {
            1
        } else {
            2
        }
    };
    let order = match sort {
        "newest" => b["createdAt"].as_str().cmp(&a["createdAt"].as_str()),
        "oldest" => a["createdAt"].as_str().cmp(&b["createdAt"].as_str()),
        "largest" => size(b).cmp(&size(a)),
        "smallest" => size(a).cmp(&size(b)),
        "ready" | "" => ready(a)
            .cmp(&ready(b))
            .then_with(|| (size(b) > 0).cmp(&(size(a) > 0)))
            .then_with(|| size(a).cmp(&size(b))),
        _ => std::cmp::Ordering::Equal,
    };
    order.then_with(|| b["updatedAt"].as_str().cmp(&a["updatedAt"].as_str()))
}
