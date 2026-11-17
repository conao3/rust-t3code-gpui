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
    pub(super) value: Value,
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
        let projects = self
            .connections
            .iter()
            .flat_map(|connection| {
                let environment = connection.label().to_string();
                connection
                    .shell()
                    .lock()
                    .expect("shell lock")
                    .projects
                    .iter()
                    .filter(|p| p["deletedAt"].is_null())
                    .map(|p| {
                        (
                            environment.clone(),
                            p["id"].as_str().unwrap_or_default().to_string(),
                            p["title"].as_str().unwrap_or_default().to_string(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
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
                                    .icon(sort_icon(value))
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
                            if *key == "draft" {
                                menu = menu.separator();
                                for (kind, label, default) in
                                    [("authors", "Author", "Anyone"), ("labels", "Labels", "Any")]
                                {
                                    let root = filters_root.clone();
                                    let dismiss = cx.weak_entity();
                                    let current = if kind == "authors" {
                                        current_filters
                                            .get("author")
                                            .cloned()
                                            .unwrap_or_else(|| default.into())
                                    } else {
                                        current_filters
                                            .get("labels")
                                            .and_then(|value| {
                                                serde_json::from_str::<Vec<Vec<String>>>(value).ok()
                                            })
                                            .map(|labels| format!("{} selected", labels.len()))
                                            .unwrap_or_else(|| default.into())
                                    };
                                    menu = menu.submenu(
                                        format!("{label} {current}"),
                                        window,
                                        cx,
                                        move |menu, window, cx| {
                                            let facet = cx.new(|cx| {
                                                FacetMenu::new(
                                                    root.clone(),
                                                    dismiss.clone(),
                                                    kind,
                                                    window,
                                                    cx,
                                                )
                                            });
                                            menu.item(PopupMenuItem::element(move |_, _| {
                                                facet.clone()
                                            }))
                                        },
                                    );
                                }
                            }
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
                            menu = menu.submenu_with_icon(
                                Some(Icon::new(if *key == "state" && selected.is_empty() {
                                    gpui_kit::assets::IconName::GitPullRequest
                                } else {
                                    gpui_kit::assets::IconName::Layers
                                })),
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
                        let selected = current_filters.get("projectId").cloned();
                        let selected_environment = current_filters.get("environment").cloned();
                        let title = projects
                            .iter()
                            .find(|(environment, id, _)| {
                                Some(id) == selected.as_ref()
                                    && Some(environment) == selected_environment.as_ref()
                            })
                            .map(|(_, _, title)| title.as_str())
                            .unwrap_or("All projects");
                        let root = filters_root.clone();
                        let projects = projects.clone();
                        menu.separator().submenu(
                            format!("Project {title}"),
                            window,
                            cx,
                            move |mut menu, _, _| {
                                let clear = root.clone();
                                menu = menu.item(
                                    PopupMenuItem::new("All projects")
                                        .checked(selected.is_none())
                                        .on_click(move |_, _, cx| {
                                            let _ = clear.update(cx, |root, cx| {
                                                root.pr_state.filters.remove("projectId");
                                                root.pr_state.filters.remove("environment");
                                                root.load_pull_requests(false, cx);
                                            });
                                        }),
                                );
                                for (environment, id, title) in &projects {
                                    let root = root.clone();
                                    let environment = environment.clone();
                                    let id = id.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(title.clone())
                                            .checked(
                                                Some(&id) == selected.as_ref()
                                                    && Some(&environment)
                                                        == selected_environment.as_ref(),
                                            )
                                            .on_click(move |_, _, cx| {
                                                let _ = root.update(cx, |root, cx| {
                                                    root.pr_state
                                                        .filters
                                                        .insert("projectId".into(), id.clone());
                                                    root.pr_state.filters.insert(
                                                        "environment".into(),
                                                        environment.clone(),
                                                    );
                                                    root.load_pull_requests(false, cx);
                                                });
                                            }),
                                    );
                                }
                                menu.radio_group()
                            },
                        )
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
                            .max_w(px(256.))
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
                    if filters
                        .get("environment")
                        .is_some_and(|environment| environment != connection.label())
                    {
                        continue;
                    }
                    let mut input = json!({"state":state,"limit":99});
                    let mut qualifiers = serde_json::Map::new();
                    for (key, value) in &filters {
                        if key == "environment" {
                            continue;
                        }
                        if key == "labels" {
                            qualifiers.insert(key.clone(), serde_json::from_str(value)?);
                        } else if matches!(key.as_str(), "involvement" | "host" | "projectId") {
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
                root.conversation.workspace_revision += 1;
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

struct FacetMenu {
    root: WeakEntity<Root>,
    dismiss: WeakEntity<gpui_kit::component::menu::PopupMenu>,
    kind: &'static str,
    input: Entity<InputState>,
    options: Vec<String>,
    selected: Vec<String>,
    loading: bool,
    error: bool,
    started: bool,
    cursor: usize,
    request: u64,
    _subscription: Subscription,
}

impl FacetMenu {
    fn new(
        root: WeakEntity<Root>,
        dismiss: WeakEntity<gpui_kit::component::menu::PopupMenu>,
        kind: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(if kind == "authors" {
                "Search authors"
            } else {
                "Search labels"
            })
        });
        let subscription = cx.subscribe(&input, |view, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                view.load(cx);
            }
        });
        let selected = root
            .upgrade()
            .map(|root| {
                let filters = &root.read(cx).pr_state.filters;
                if kind == "authors" {
                    filters.get("author").cloned().into_iter().collect()
                } else {
                    filters
                        .get("labels")
                        .and_then(|value| serde_json::from_str::<Vec<Vec<String>>>(value).ok())
                        .unwrap_or_default()
                        .into_iter()
                        .flatten()
                        .collect()
                }
            })
            .unwrap_or_default();
        Self {
            root,
            dismiss,
            kind,
            input,
            options: Vec::new(),
            selected,
            loading: false,
            error: false,
            started: false,
            cursor: 0,
            request: 0,
            _subscription: subscription,
        }
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.upgrade() else {
            return;
        };
        let root = root.read(cx);
        let connections = root.connections.clone();
        let runtime = root.runtime.clone();
        let filters = root.pr_state.filters.clone();
        let kind = self.kind;
        let query = self
            .input
            .read(cx)
            .value()
            .trim()
            .chars()
            .take(100)
            .collect::<String>();
        self.request += 1;
        let request = self.request;
        self.loading = true;
        self.error = false;
        self.cursor = 0;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            if this
                .update(cx, |view, _| view.request != request)
                .unwrap_or(true)
            {
                return;
            }
            let result = on_runtime(&runtime, async move {
                let mut options = Vec::new();
                for connection in connections {
                    if filters
                        .get("environment")
                        .is_some_and(|environment| environment != connection.label())
                    {
                        continue;
                    }
                    let mut input = json!({"kind":kind});
                    for key in ["host", "projectId"] {
                        if let Some(value) = filters.get(key) {
                            input[key] = json!(value);
                        }
                    }
                    if !query.is_empty() {
                        input["query"] = json!(query);
                    }
                    let response = connection
                        .request("pullRequests.filterOptions", input)
                        .await?;
                    for option in response[kind].as_array().into_iter().flatten() {
                        if let Some(name) =
                            option[if kind == "authors" { "login" } else { "name" }].as_str()
                        {
                            options.push(name.to_string());
                        }
                    }
                }
                options.sort_by_key(|name| name.to_lowercase());
                options.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
                Ok(options)
            })
            .await;
            let _ = this.update(cx, |view, cx| {
                if view.request != request {
                    return;
                }
                view.loading = false;
                match result {
                    Ok(options) => view.options = options,
                    Err(_) => {
                        view.options.clear();
                        view.error = true;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn choose(&mut self, value: String, cx: &mut Context<Self>) {
        if self.kind == "authors" {
            self.selected = (!value.is_empty()).then_some(value).into_iter().collect();
        } else if let Some(index) = self
            .selected
            .iter()
            .position(|name| name.eq_ignore_ascii_case(&value))
        {
            self.selected.remove(index);
        } else if self.selected.len() < 10 {
            self.selected.push(value);
        }
        let _ = self.root.update(cx, |root, cx| {
            let key = if self.kind == "authors" {
                "author"
            } else {
                "labels"
            };
            if self.selected.is_empty() {
                root.pr_state.filters.remove(key);
            } else {
                let value = if self.kind == "authors" {
                    self.selected[0].clone()
                } else {
                    serde_json::to_string(
                        &self
                            .selected
                            .iter()
                            .map(|name| vec![name])
                            .collect::<Vec<_>>(),
                    )
                    .expect("label groups")
                };
                root.pr_state.filters.insert(key.into(), value);
            }
            root.load_pull_requests(false, cx);
        });
        if self.kind == "authors" {
            let _ = self.dismiss.update(cx, |_, cx| cx.emit(gpui::DismissEvent));
        }
        cx.notify();
    }
}

impl Render for FacetMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.started {
            self.started = true;
            self.load(cx);
            let input = self.input.clone();
            window.defer(cx, move |window, cx| {
                input.update(cx, |input, cx| input.focus(window, cx))
            });
        }
        let mut options = self.options.clone();
        if self.kind == "authors" {
            options.insert(0, String::new());
        }
        let keyboard_options = options.clone();
        v_flex()
            .w(px(256.))
            .gap_1()
            .child(
                Input::new(&self.input).aria_label(if self.kind == "authors" {
                    "Search authors"
                } else {
                    "Search labels"
                }),
            )
            .on_key_down(cx.listener(move |view, event: &KeyDownEvent, _, cx| {
                match event.keystroke.key.as_str() {
                    "down" => {
                        view.cursor =
                            (view.cursor + 1).min(keyboard_options.len().saturating_sub(1))
                    }
                    "up" => view.cursor = view.cursor.saturating_sub(1),
                    "enter" => {
                        if let Some(value) = keyboard_options.get(view.cursor) {
                            view.choose(value.clone(), cx);
                        }
                    }
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .children(options.into_iter().enumerate().map(|(index, value)| {
                let checked = if value.is_empty() {
                    self.selected.is_empty()
                } else {
                    self.selected
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(&value))
                };
                let label = if value.is_empty() {
                    "Anyone".to_string()
                } else {
                    value.clone()
                };
                h_flex()
                    .id(("pr-facet-option", index))
                    .role(if self.kind == "authors" {
                        accesskit::Role::MenuItemRadio
                    } else {
                        accesskit::Role::MenuItemCheckBox
                    })
                    .aria_label(label.clone())
                    .aria_toggled(if checked {
                        accesskit::Toggled::True
                    } else {
                        accesskit::Toggled::False
                    })
                    .p_2()
                    .gap_2()
                    .rounded_md()
                    .when(index == self.cursor, |row| row.bg(cx.theme().accent))
                    .child(
                        Icon::new(IconName::Check)
                            .size(px(14.))
                            .when(!checked, |icon| icon.invisible()),
                    )
                    .child(label)
                    .on_click(cx.listener(move |view, _, _, cx| view.choose(value.clone(), cx)))
            }))
            .when(!self.loading && self.options.is_empty(), |list| {
                list.child(
                    Button::new("facet-empty")
                        .role(accesskit::Role::MenuItem)
                        .disabled(true)
                        .ghost()
                        .label(match (self.kind, self.error) {
                            ("authors", false) => "No authors found",
                            ("authors", true) => "Couldn't search authors on the host",
                            (_, false) => "No labels found",
                            (_, true) => "Couldn't search labels on the host",
                        }),
                )
            })
    }
}

fn sort_icon(sort: &str) -> gpui_kit::assets::IconName {
    use gpui_kit::assets::IconName;
    match sort {
        "ready" => IconName::ListChecks,
        "blocked" => IconName::UserLock,
        "updated" => IconName::Clock,
        "newest" => IconName::CalendarArrowDown,
        "oldest" => IconName::CalendarArrowUp,
        "largest" => IconName::Maximize2,
        _ => IconName::Minimize2,
    }
}
