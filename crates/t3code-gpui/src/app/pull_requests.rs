use super::*;

#[derive(Default)]
pub(super) struct State {
    pub page: bool,
    pub page_panel_open: bool,
    pub page_maximized: bool,
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
    guide: Value,
    guide_position: usize,
    guide_rail: bool,
    guide_seen: HashSet<usize>,
    review: Option<Entity<ReviewComposer>>,
    composer_open: bool,
    loading: bool,
    busy: bool,
    request: u64,
    error: Option<String>,
    collapsed: HashSet<String>,
    oldest_comments: bool,
    show_bots: bool,
    show_resolved: bool,
}

pub(super) struct Pane {
    pub root: WeakEntity<Root>,
}
impl Render for Pane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.root
            .update(cx, |root, cx| {
                root.render_pull_requests(f32::from(window.viewport_size().width), cx)
            })
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
                    let involvements = input["involvement"]
                        .as_str()
                        .map(|value| vec![value.to_string()])
                        .unwrap_or_else(|| {
                            vec!["authored".into(), "reviewing".into(), "all".into()]
                        });
                    let mut seen = HashSet::new();
                    for involvement in involvements {
                        input["involvement"] = json!(involvement);
                        let result = connection
                            .request("pullRequests.list", input.clone())
                            .await
                            .map(|mut result| {
                                if let Some(entries) = result["entries"].as_array_mut() {
                                    entries.retain(|entry| {
                                        seen.insert(format!(
                                            "{}:{}:{}",
                                            entry["host"], entry["repository"], entry["number"]
                                        ))
                                    });
                                    for entry in entries {
                                        entry["_group"] = json!(involvement);
                                    }
                                }
                                result
                            });
                        results.push((connection.label().to_string(), result));
                    }
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
            guide: Value::Null,
            guide_position: 0,
            guide_rail: false,
            guide_seen: HashSet::new(),
            review: None,
            composer_open: false,
            loading: false,
            busy: false,
            request: 0,
            error: None,
            collapsed: HashSet::from(["Checks".into()]),
            oldest_comments: false,
            show_bots: false,
            show_resolved: false,
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
                let (detail, activity, checks, guide) = tokio::join!(
                    connection.request("pullRequests.detail", reference.clone()),
                    connection.request("pullRequests.activity", reference.clone()),
                    connection.request("pullRequests.checks", reference.clone()),
                    connection.request("pullRequests.reviewGuide", reference)
                );
                Ok((detail?, activity, checks, guide))
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
                    Ok((mut detail, activity, checks, guide)) => {
                        let mut errors = Vec::new();
                        match guide {
                            Ok(guide) => view.guide = guide,
                            Err(error) => errors.push(format!("Review guide: {error:#}")),
                        }
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
        if tab == "guide" && !view.guide["guide"].is_null() {
            if self.pr_state.page {
                self.pr_state.page_maximized = true;
            } else {
                self.right_panel.maximized = true;
            }
        }
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

    pub(super) fn render_pull_requests(&self, width: f32, cx: &Context<Self>) -> AnyElement {
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
        entries.sort_by(|a, b| {
            pr_group_rank(a)
                .cmp(&pr_group_rank(b))
                .then_with(|| compare_entries(a, b, &self.pr_state.sort))
        });
        let mut group = None;
        for entry in &entries {
            let rank = pr_group_rank(entry);
            if self.pr_state.page && group != Some(rank) {
                group = Some(rank);
                list = list.child(
                    h_flex()
                        .id(("pr-group", rank))
                        .gap_2()
                        .px_3()
                        .py_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(text!(match rank {
                            0 => "Authored",
                            1 => "Review requested",
                            _ => "Others",
                        }))
                        .child(text!(
                            entries
                                .iter()
                                .filter(|entry| pr_group_rank(entry) == rank)
                                .count()
                                .to_string()
                        )),
                );
            }
            let label = format!(
                "#{} {}",
                entry["number"],
                entry["title"].as_str().unwrap_or_default()
            );
            let entry = (*entry).clone();
            let status = if entry["isDraft"] == true {
                "Draft"
            } else {
                match entry["state"].as_str() {
                    Some("merged") => "Merged",
                    Some("closed") => "Closed",
                    _ => "Open",
                }
            };
            let author = entry["author"]["login"].as_str().unwrap_or_default();
            let repository = entry["repository"].as_str().unwrap_or_default();
            let elapsed = pr_relative_time(entry["updatedAt"].as_str().unwrap_or_default());
            let accessible = format!(
                "{status} {label} +{} -{} {author} {repository} {elapsed}",
                entry["additions"].as_u64().unwrap_or_default(),
                entry["deletions"].as_u64().unwrap_or_default()
            );
            list = list.child(
                Button::new(SharedString::from(format!(
                    "{}:{}:{}",
                    entry["environment"], entry["repository"], entry["number"]
                )))
                .child(
                    v_flex()
                        .w_full()
                        .items_start()
                        .child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .child(
                                    Icon::new(gpui_kit::assets::IconName::GitPullRequest)
                                        .size(px(14.)),
                                )
                                .child(label.clone())
                                .child(div().flex_1())
                                .child(div().text_xs().text_color(cx.theme().success).child(
                                    format!("+{}", entry["additions"].as_u64().unwrap_or_default()),
                                ))
                                .child(div().text_xs().text_color(cx.theme().danger).child(
                                    format!("-{}", entry["deletions"].as_u64().unwrap_or_default()),
                                )),
                        )
                        .child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .pl(px(22.))
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(author.to_string())
                                .child(repository.to_string())
                                .child(div().flex_1())
                                .child(elapsed),
                        ),
                )
                .accessibility_label(accessible)
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
            if self.pr_state.page_maximized {
                return self.render_pr_detail(view, cx);
            }
            return h_flex()
                .size_full()
                .child(body.flex_1().w_0())
                .child(
                    div()
                        .w(px((width / 2.).floor()))
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
            "guide" => {
                content = content.child(self.render_pr_guide(view, cx));
            }
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
                content = content.child(self.render_pr_summary(view, cx));
            }
        }
        v_flex()
            .id("pull-request-detail")
            .size_full()
            .min_h_0()
            .when(self.pr_state.page, |body| {
                body.role(accesskit::Role::Region)
                    .aria_label(format!("Workspace panel: #{}", view.reference["number"]))
            })
            .when(self.pr_state.page, |panel| {
                panel.child(self.render_pr_page_tabs(view, cx))
            })
            .child(self.render_pr_header(view, cx))
            .child(
                h_flex()
                    .id("pr-tabs")
                    .aria_label("Pull request tabs")
                    .p_2()
                    .gap_1()
                    .children(
                        [
                            ("summary", "Summary"),
                            ("guide", "Guide"),
                            ("timeline", "Timeline"),
                            ("code", "Code"),
                        ]
                        .into_iter()
                        .filter(|(tab, _)| {
                            *tab != "guide"
                                || self.config_of(&view.environment).is_some_and(|config| {
                                    config["settings"]["reviewGuidesEnabled"] == true
                                })
                        })
                        .map(|(tab, label)| {
                            Button::new(label)
                                .label(label)
                                .small()
                                .ghost()
                                .selected(view.tab == tab)
                                .on_click(
                                    cx.listener(move |root, _, _, cx| root.select_pr_tab(tab, cx)),
                                )
                        }),
                    )
                    .when(view.tab == "summary", |row| {
                        row.child(
                            div()
                                .flex_1()
                                .text_right()
                                .text_xs()
                                .child(text!(pr_checks_label(value))),
                        )
                    }),
            )
            .child(content)
            .child(self.render_pr_composer(view, cx))
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

impl Root {
    fn step_pr_guide(&mut self, position: usize, cx: &mut Context<Self>) {
        if let Some(view) = self
            .pull_request_id()
            .and_then(|id| self.pr_state.views.get_mut(&id))
        {
            let count = view.guide["guide"]["stops"].as_array().map_or(0, Vec::len);
            if position > view.guide_position {
                view.guide_seen.insert(view.guide_position);
            }
            view.guide_position = position.min(count.saturating_sub(1));
            view.guide_rail = false;
            self.panel_revision += 1;
            cx.notify();
        }
    }

    fn render_pr_guide(&self, view: &Detail, cx: &Context<Self>) -> AnyElement {
        let guide = &view.guide["guide"];
        let running = view.busy || view.guide["run"]["_tag"] == "Running";
        let failed = view.guide["run"]["_tag"] == "Failed"
            && view.guide["run"]["headSha"] == view.guide["currentHeadSha"];
        if guide.is_null() {
            return v_flex().size_full().justify_center().items_center().gap_2().child(
                text!(if running { "Writing a review guide" } else if failed { "Could not write a review guide" } else { "No review guide yet" })
            ).child(div().text_xs().text_color(cx.theme().muted_foreground).child(if running {
                "An agent is reading the change. A large one takes a few minutes. You can leave; you will be told when it is ready."
            } else if failed { "Check the review guide model in Settings → Source control." } else {
                "A guide is a tour of the change: stops through the code in reading order, with the calls only a maintainer can make."
            })).child(Button::new("write-guide").outline().label(if running { "Reading the change..." } else if failed { "Try again" } else { "Write guide" }).disabled(running).on_click(cx.listener(|root, _, _, cx| root.mutate_pr("pullRequests.generateReviewGuide", json!({}), cx))))
            .into_any_element();
        }
        let stops = guide["stops"].as_array().cloned().unwrap_or_default();
        let position = view.guide_position.min(stops.len().saturating_sub(1));
        let wide = self.pr_state.page && self.pr_state.page_maximized || self.right_panel.maximized;
        let mut body = v_flex().gap_4().flex_1().min_w_0();
        if running || failed || guide["headSha"] != view.guide["currentHeadSha"] {
            body = body.child(div().text_sm().child(if running { "Rewriting this guide. You will be told when the new one is ready." } else if failed { "Could not rewrite this guide. Check the review guide model in Settings → Source control." } else { "Written before the latest push." }));
            if !running {
                body = body.child(Button::new("rewrite-guide").label("Rewrite").on_click(
                    cx.listener(|root, _, _, cx| {
                        root.mutate_pr("pullRequests.generateReviewGuide", json!({}), cx)
                    }),
                ));
            }
        }
        let Some(stop) = stops.get(position) else {
            return body
                .child(text!("This change has no files to tour."))
                .into_any_element();
        };
        body = body.child(
            h_flex()
                .justify_between()
                .child(if wide {
                    div()
                        .text_xs()
                        .child(
                            h_flex()
                                .gap_1()
                                .child(text!("Stop"))
                                .child(text!((position + 1).to_string()))
                                .child(text!("of"))
                                .child(text!(stops.len().to_string())),
                        )
                        .into_any_element()
                } else {
                    Button::new("guide-stops")
                        .ghost()
                        .small()
                        .label(format!("Stop {} of {}", position + 1, stops.len()))
                        .on_click(cx.listener(|root, _, _, cx| {
                            if let Some(view) = root
                                .pull_request_id()
                                .and_then(|id| root.pr_state.views.get_mut(&id))
                            {
                                view.guide_rail = !view.guide_rail;
                            }
                            root.panel_revision += 1;
                            cx.notify();
                        }))
                        .into_any_element()
                })
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("guide-previous")
                                .icon(IconName::ChevronUp)
                                .accessibility_label("Previous stop")
                                .outline()
                                .small()
                                .disabled(position == 0)
                                .on_click(cx.listener(move |root, _, _, cx| {
                                    root.step_pr_guide(position.saturating_sub(1), cx)
                                })),
                        )
                        .child(
                            Button::new("guide-next")
                                .icon(IconName::ChevronDown)
                                .accessibility_label("Next stop")
                                .outline()
                                .small()
                                .disabled(position + 1 >= stops.len())
                                .on_click(cx.listener(move |root, _, _, cx| {
                                    root.step_pr_guide(position + 1, cx)
                                })),
                        ),
                ),
        );
        let mut rail = v_flex()
            .id("review-tour")
            .role(accesskit::Role::Navigation)
            .aria_label("Review tour")
            .gap_2();
        if wide {
            rail = rail.child(
                h_flex()
                    .gap_1()
                    .child(text!(view.guide_seen.len().to_string()))
                    .child(text!("of"))
                    .child(text!(stops.len().to_string()))
                    .child(text!("read")),
            );
        }
        if view.guide_rail || wide {
            for (index, stop) in stops.iter().enumerate() {
                rail = rail.child(
                    Button::new(("guide-stop", index))
                        .ghost()
                        .w_full()
                        .h_auto()
                        .py_2()
                        .child(
                            v_flex()
                                .items_start()
                                .gap_1()
                                .w_full()
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .when(view.guide_seen.contains(&index), |row| {
                                            row.child(
                                                Icon::new(IconName::Check)
                                                    .size(px(14.))
                                                    .text_color(cx.theme().success),
                                            )
                                        })
                                        .when(!view.guide_seen.contains(&index), |row| {
                                            row.child((index + 1).to_string())
                                        })
                                        .child(
                                            stop["title"].as_str().unwrap_or_default().to_string(),
                                        ),
                                )
                                .when(!stop["attention"].is_null(), |column| {
                                    column.child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().warning)
                                            .child("Look closely"),
                                    )
                                })
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(
                                            if stop["files"].as_array().is_none_or(Vec::is_empty) {
                                                "Context, no code".to_string()
                                            } else {
                                                format!(
                                                    "{} files",
                                                    stop["files"].as_array().unwrap().len()
                                                )
                                            },
                                        ),
                                ),
                        )
                        .accessibility_label(format!(
                            "{} {} {}{}",
                            if view.guide_seen.contains(&index) {
                                format!("Stop {}, read", index + 1)
                            } else {
                                (index + 1).to_string()
                            },
                            stop["title"].as_str().unwrap_or_default(),
                            if stop["attention"].is_null() {
                                ""
                            } else {
                                "Look closely "
                            },
                            if stop["files"].as_array().is_none_or(Vec::is_empty) {
                                "Context, no code".to_string()
                            } else {
                                format!("{} files", stop["files"].as_array().unwrap().len())
                            }
                        ))
                        .selected(index == position)
                        .on_click(cx.listener(move |root, _, _, cx| root.step_pr_guide(index, cx))),
                );
            }
        }
        body = body
            .child(
                div()
                    .id("guide-title")
                    .role(accesskit::Role::Heading)
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(text!(
                        stop["title"].as_str().unwrap_or_default().to_string()
                    )),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(text!(
                        stop["summary"].as_str().unwrap_or_default().to_string()
                    )),
            );
        if let Some(kind) = stop["attention"]["kind"].as_str() {
            let label = match kind {
                "tradeoff" => "A tradeoff to weigh",
                "contract" => "A contract to check",
                "product" => "A product call",
                "risk" => "A risk to judge",
                "accessibility" => "An accessibility call",
                _ => kind,
            };
            body = body.child(
                v_flex()
                    .gap_2()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded_lg()
                    .p_3()
                    .child(text!(label.to_string()))
                    .child(text!(
                        stop["attention"]["note"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string()
                    )),
            );
        }
        for file in stop["files"].as_array().into_iter().flatten() {
            let path = file["path"].as_str().unwrap_or_default().to_string();
            body = body.child(
                Button::new(SharedString::from(format!("guide-file-{path}")))
                    .label(path)
                    .ghost()
                    .disabled(true)
                    .tooltip("Guide excerpts are not implemented in T3 Code GPUI."),
            );
        }
        body = body.child(
            h_flex().child(
                Button::new("ask-guide")
                    .ghost()
                    .label("Ask about this stop")
                    .disabled(true)
                    .tooltip("Guide questions are not implemented in T3 Code GPUI."),
            ),
        );
        body = body.child(
            h_flex()
                .justify_end()
                .border_t_1()
                .border_color(cx.theme().border)
                .pt_4()
                .child(if let Some(next) = stops.get(position + 1) {
                    Button::new("guide-continue")
                        .label(format!(
                            "Next: {}",
                            next["title"].as_str().unwrap_or_default()
                        ))
                        .on_click(
                            cx.listener(move |root, _, _, cx| root.step_pr_guide(position + 1, cx)),
                        )
                } else {
                    Button::new("guide-finish")
                        .primary()
                        .label("Finish review")
                        .on_click(cx.listener(move |root, _, window, cx| {
                            root.step_pr_guide(position + 1, cx);
                            root.open_pr_composer(true, window, cx);
                        }))
                }),
        );
        if wide {
            h_flex()
                .items_start()
                .gap_6()
                .child(rail.w(px(256.)))
                .child(body)
                .into_any_element()
        } else {
            v_flex()
                .when(view.guide_rail, |layout| layout.child(rail))
                .child(body)
                .into_any_element()
        }
    }

    fn open_pr_composer(&mut self, review_mode: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.pull_request_id() else {
            return;
        };
        let Some(view) = self.pr_state.views.get_mut(&id) else {
            return;
        };
        if view.review.is_none() {
            let verdicts = [
                ("comment", "Comment"),
                ("approve", "Approve"),
                ("request-changes", "Request changes"),
            ]
            .into_iter()
            .filter(|(value, _)| {
                view.value["capabilities"]["review"]["verdicts"]
                    .as_array()
                    .is_some_and(|values| values.iter().any(|v| v == value))
                    && view.value["viewerPermissions"]["verdicts"]
                        .as_array()
                        .is_some_and(|values| values.iter().any(|v| v == value))
            })
            .collect::<Vec<_>>();
            let summary = cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder("Summarize your review or add line comments")
            });
            let comment =
                cx.new(|cx| TextareaState::new(window, cx).placeholder("Leave a comment"));
            let verdict = choice_select::state(&verdicts, "comment", window, cx);
            cx.observe(&summary, |root, _, cx| {
                root.panel_revision += 1;
                cx.notify();
            })
            .detach();
            let root = cx.entity().downgrade();
            let environment = view.environment.clone();
            let reference = view.reference.clone();
            let require_summary = view.value["provider"] == "forgejo";
            view.review = Some(cx.new(|cx| {
                cx.observe(&summary, |_, _, cx| cx.notify()).detach();
                cx.observe(&comment, |_, _, cx| cx.notify()).detach();
                cx.observe(&verdict, |_, _, cx| cx.notify()).detach();
                ReviewComposer {
                    root,
                    id,
                    environment,
                    reference,
                    summary,
                    comment,
                    verdict,
                    review_mode,
                    require_summary,
                    busy: false,
                    error: None,
                }
            }));
        }
        view.composer_open = true;
        view.review.as_ref().unwrap().update(cx, |editor, cx| {
            editor.review_mode = review_mode;
            editor
                .input()
                .update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
        });
        self.panel_revision += 1;
        cx.notify();
    }

    fn render_pr_composer(&self, view: &Detail, cx: &Context<Self>) -> AnyElement {
        if view.value["viewerPermissions"]["comment"] != true {
            return div().into_any_element();
        }
        h_flex()
            .relative()
            .justify_end()
            .p_2()
            .child(
                Button::new("pr-composer-trigger")
                    .icon(gpui_kit::assets::IconName::MessageSquare)
                    .accessibility_label(
                        if view.review.as_ref().is_some_and(|review| {
                            review.read(cx).review_mode
                                && !review.read(cx).summary.read(cx).value().is_empty()
                        }) {
                            "Review pull request"
                        } else {
                            "Comment on pull request"
                        },
                    )
                    .ghost()
                    .on_click(
                        cx.listener(|root, _, window, cx| root.open_pr_composer(false, window, cx)),
                    ),
            )
            .when(view.composer_open, |footer| {
                footer.child(
                    div()
                        .absolute()
                        .bottom(px(48.))
                        .right(px(16.))
                        .w(px(384.))
                        .id("pr-composer-popup")
                        .occlude()
                        .rounded_lg()
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().popover)
                        .on_mouse_down_out(cx.listener(|root, _, _, cx| {
                            if let Some(view) = root
                                .pull_request_id()
                                .and_then(|id| root.pr_state.views.get_mut(&id))
                            {
                                view.composer_open = false;
                            }
                            root.panel_revision += 1;
                            cx.notify();
                        }))
                        .children(view.review.clone()),
                )
            })
            .into_any_element()
    }
}

struct ReviewComposer {
    root: WeakEntity<Root>,
    id: String,
    environment: String,
    reference: Value,
    summary: Entity<TextareaState>,
    comment: Entity<TextareaState>,
    verdict: Entity<choice_select::State>,
    review_mode: bool,
    require_summary: bool,
    busy: bool,
    error: Option<String>,
}

impl ReviewComposer {
    fn input(&self) -> Entity<TextareaState> {
        if self.review_mode {
            self.summary.clone()
        } else {
            self.comment.clone()
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor_input = self.input();
        let body = editor_input.read(cx).value().to_string();
        let verdict = if self.review_mode {
            self.verdict
                .read(cx)
                .selected_value()
                .copied()
                .unwrap_or("comment")
        } else {
            "comment"
        };
        if self.busy || !review_can_submit(verdict, &body, 0, self.require_summary) {
            return;
        }
        let Some((connection, runtime)) = self.root.upgrade().and_then(|root| {
            root.update(cx, |root, _| {
                root.connection(&self.environment)
                    .map(|connection| (connection, root.runtime.clone()))
            })
        }) else {
            return;
        };
        self.busy = true;
        self.error = None;
        let mut input = self.reference.clone();
        input["body"] = json!(body);
        let method = if self.review_mode {
            input["verdict"] = json!(verdict);
            input["comments"] = json!([]);
            "pullRequests.submitReview"
        } else {
            "pullRequests.comment"
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = on_runtime(
                &runtime,
                async move { connection.request(method, input).await },
            )
            .await;
            let _ = this.update_in(cx, |editor, window, cx| {
                editor.busy = false;
                match result {
                    Ok(_) => {
                        editor_input.update(cx, |input, cx| {
                            if input.value().as_ref() == body {
                                input.set_value("", window, cx);
                            }
                        });
                        let _ = editor.root.update(cx, |root, cx| {
                            if let Some(view) = root.pr_state.views.get_mut(&editor.id) {
                                view.composer_open = false;
                            }
                            root.load_pull_request(editor.id.clone(), cx);
                        });
                    }
                    Err(error) => {
                        editor.error = Some(format!("The review could not be submitted: {error:#}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for ReviewComposer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::select::Select;
        let verdict = if self.review_mode {
            self.verdict
                .read(cx)
                .selected_value()
                .copied()
                .unwrap_or("comment")
        } else {
            "comment"
        };
        let disabled = self.busy
            || !review_can_submit(
                verdict,
                &self.input().read(cx).value(),
                0,
                self.require_summary,
            );
        v_flex()
            .id("pr-review-form")
            .role(accesskit::Role::Dialog)
            .aria_label("Pull request composer")
            .on_key_down(cx.listener(|editor, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    let _ = editor.root.update(cx, |root, cx| {
                        if let Some(view) = root.pr_state.views.get_mut(&editor.id) {
                            view.composer_open = false;
                        }
                        root.panel_focus.focus(window, cx);
                        root.panel_revision += 1;
                        cx.notify();
                    });
                }
                if event.keystroke.key == "enter"
                    && (event.keystroke.modifiers.control || event.keystroke.modifiers.platform)
                {
                    cx.stop_propagation();
                    editor.submit(window, cx);
                }
            }))
            .gap_3()
            .p_3()
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("pr-mode-comment")
                            .label("Comment")
                            .ghost()
                            .selected(!self.review_mode)
                            .on_click(cx.listener(|editor, _, window, cx| {
                                editor.review_mode = false;
                                editor
                                    .input()
                                    .update(cx, |input, cx| input.focus(window, cx));
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("pr-mode-review")
                            .label("Review")
                            .ghost()
                            .selected(self.review_mode)
                            .on_click(cx.listener(|editor, _, window, cx| {
                                editor.review_mode = true;
                                editor
                                    .input()
                                    .update(cx, |input, cx| input.focus(window, cx));
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("close-pr-composer")
                            .icon(IconName::Close)
                            .accessibility_label("Close composer")
                            .ghost()
                            .on_click(cx.listener(|editor, _, _, cx| {
                                let _ = editor.root.update(cx, |root, cx| {
                                    if let Some(view) = root.pr_state.views.get_mut(&editor.id) {
                                        view.composer_open = false;
                                    }
                                    root.panel_revision += 1;
                                    cx.notify();
                                });
                            })),
                    ),
            )
            .child(
                Textarea::new(&self.input())
                    .h(px(70.))
                    .aria_label(if self.review_mode {
                        "Review summary"
                    } else {
                        "Comment"
                    }),
            )
            .children(
                self.error
                    .clone()
                    .map(|error| div().text_color(cx.theme().danger).child(error)),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .when(self.review_mode, |row| {
                        row.child(
                            Select::new(&self.verdict)
                                .w(px(100.))
                                .accessibility_label("Review verdict")
                                .disabled(self.busy),
                        )
                    })
                    .child(
                        Button::new("submit-pr-review")
                            .primary()
                            .label(if self.busy {
                                "Submitting..."
                            } else if self.review_mode {
                                "Submit review"
                            } else {
                                "Comment"
                            })
                            .disabled(disabled)
                            .on_click(
                                cx.listener(|editor, _, window, cx| editor.submit(window, cx)),
                            ),
                    ),
            )
    }
}

fn review_can_submit(verdict: &str, summary: &str, comments: usize, required: bool) -> bool {
    if verdict == "request-changes" && required {
        !summary.trim().is_empty()
    } else {
        verdict == "approve" || !summary.trim().is_empty() || comments > 0
    }
}

impl Root {
    pub(super) fn pr_guide_key(
        &mut self,
        key: &Keystroke,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if key.modifiers.control
            || key.modifiers.platform
            || key.modifiers.alt
            || key.modifiers.shift
            || self.key_context("editableFocus", window, cx)
            || !matches!(key.key.as_str(), "j" | "k")
        {
            return false;
        }
        let Some(view) = self
            .pull_request_id()
            .and_then(|id| self.pr_state.views.get(&id))
            .filter(|view| view.tab == "guide")
        else {
            return false;
        };
        let position = if key.key == "j" {
            view.guide_position + 1
        } else {
            view.guide_position.saturating_sub(1)
        };
        self.step_pr_guide(position, cx);
        true
    }
}

impl Root {
    fn render_pr_page_tabs(&self, view: &Detail, cx: &Context<Self>) -> AnyElement {
        h_flex()
            .h(px(28.))
            .px_2()
            .gap_1()
            .child(
                Button::new("close-page-pr")
                    .icon(IconName::Close)
                    .ghost()
                    .compact()
                    .accessibility_label(format!("Close #{}", view.reference["number"]))
                    .on_click(cx.listener(|root, _, _, cx| {
                        root.pr_state.page_panel_open = false;
                        root.panel_revision += 1;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("page-pr-tab")
                    .label(format!("#{}", view.reference["number"]))
                    .ghost()
                    .compact(),
            )
            .child(
                Button::new("page-pr-add")
                    .icon(IconName::Plus)
                    .ghost()
                    .compact()
                    .accessibility_label("Add panel surface")
                    .on_click(cx.listener(|root, _, _, cx| {
                        root.pr_state.selected = None;
                        root.pr_state.page_panel_open = false;
                        root.panel_revision += 1;
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .child(
                Button::new("page-pr-maximize")
                    .icon(if self.pr_state.page_maximized {
                        gpui_kit::assets::IconName::Minimize2
                    } else {
                        gpui_kit::assets::IconName::Maximize2
                    })
                    .ghost()
                    .compact()
                    .accessibility_label(if self.pr_state.page_maximized {
                        "Restore panel size"
                    } else {
                        "Maximize panel"
                    })
                    .on_click(cx.listener(|root, _, _, cx| {
                        root.pr_state.page_maximized = !root.pr_state.page_maximized;
                        root.panel_revision += 1;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn render_pr_header(&self, view: &Detail, cx: &Context<Self>) -> AnyElement {
        use gpui_kit::component::menu::PopupMenuItem;
        let value = &view.value;
        let url = value["url"].as_str().unwrap_or_default().to_string();
        let repository = value["repository"].as_str().unwrap_or_default().to_string();
        let repository_url = url.split("/pull/").next().unwrap_or(&url).to_string();
        let author = value["author"]["login"]
            .as_str()
            .unwrap_or("ghost")
            .to_string();
        let profile = value["author"]["url"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "https://{}/{author}",
                    value["host"].as_str().unwrap_or("github.com")
                )
            });
        let branch = value["headBranch"].as_str().unwrap_or_default().to_string();
        let checkout = format!("gh pr checkout {}", view.reference["number"]);
        let weak = cx.entity().downgrade();
        let actions = value["viewerPermissions"]["actions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let menu_url = url.clone();
        let number = view.reference["number"].to_string();
        let files = value["changedFiles"].as_u64().unwrap_or_default();
        v_flex()
            .gap_1()
            .px_4()
            .pb_2()
            .child(
                h_flex()
                    .h(px(22.))
                    .gap_2()
                    .child(
                        Button::new("pr-repository")
                            .label(repository)
                            .ghost()
                            .compact()
                            .on_click(move |_, _, cx| cx.open_url(&repository_url)),
                    )
                    .child(
                        Button::new("pr-on-host")
                            .label(format!("#{number}"))
                            .accessibility_label(format!("Open pull request #{number} on host"))
                            .ghost()
                            .compact()
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    )
                    .child(div().flex_1())
                    .when(actions.iter().any(|action| action == "merge"), |row| {
                        row.children(
                            value["capabilities"]["mergeMethods"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str)
                                .filter(|method| value["mergeCapabilities"][*method] == true)
                                .map(|method| {
                                    let method = method.to_string();
                                    Button::new(SharedString::from(format!("merge-{method}")))
                                        .label(format!("Merge ({method})"))
                                        .primary()
                                        .small()
                                        .disabled(view.busy)
                                        .on_click(cx.listener(move |root, _, window, cx| {
                                            root.confirm_pr_action(
                                                "merge",
                                                Some(method.clone()),
                                                window,
                                                cx,
                                            )
                                        }))
                                }),
                        )
                    })
                    .child(
                        Button::new("pr-checkout")
                            .icon(gpui_kit::assets::IconName::GitBranch)
                            .label("Check out")
                            .outline()
                            .small()
                            .disabled(view.busy)
                            .dropdown_menu({
                                let weak = weak.clone();
                                move |mut menu, _, _| {
                                    for (mode, label) in [
                                        ("worktree", "In a separate worktree"),
                                        ("local", "In this repository"),
                                    ] {
                                        let weak = weak.clone();
                                        menu = menu.item(PopupMenuItem::new(label).on_click(
                                            move |_, _, cx| {
                                                let _ = weak.update(cx, |root, cx| {
                                                    root.checkout_pr(mode, cx)
                                                });
                                            },
                                        ));
                                    }
                                    menu
                                }
                            }),
                    )
                    .child(
                        Button::new("pr-more")
                            .icon(gpui_kit::assets::IconName::Ellipsis)
                            .ghost()
                            .compact()
                            .accessibility_label("More pull request actions")
                            .dropdown_menu(move |mut menu, _, _| {
                                for (key, label) in [
                                    ("refresh", "Refresh from host"),
                                    ("copy", "Copy pull request number"),
                                    ("open", "Open on GitHub"),
                                    ("guide", "Rewrite guide"),
                                ] {
                                    let weak = weak.clone();
                                    let url = menu_url.clone();
                                    menu = menu.item(PopupMenuItem::new(label).on_click(
                                        move |_, _, cx| {
                                            let _ = weak.update(cx, |root, cx| match key {
                                                "refresh" => {
                                                    if let Some(id) = root.pull_request_id() {
                                                        root.load_pull_request(id, cx);
                                                    }
                                                }
                                                "copy" => root.copy_pull_request_number(cx),
                                                "open" => cx.open_url(&url),
                                                _ => root.mutate_pr(
                                                    "pullRequests.generateReviewGuide",
                                                    json!({}),
                                                    cx,
                                                ),
                                            });
                                        },
                                    ));
                                }
                                for action in [
                                    "ready",
                                    "draft",
                                    "close",
                                    "reopen",
                                    "update-branch",
                                    "delete-branch",
                                    "enable-auto-merge",
                                    "disable-auto-merge",
                                ] {
                                    if !actions.iter().any(|value| value.as_str() == Some(action)) {
                                        continue;
                                    }
                                    let weak = weak.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(action_label(action)).on_click(
                                            move |_, window, cx| {
                                                let _ = weak.update(cx, |root, cx| {
                                                    root.confirm_pr_action(action, None, window, cx)
                                                });
                                            },
                                        ),
                                    );
                                }
                                menu
                            }),
                    ),
            )
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(text!(
                        value["title"]
                            .as_str()
                            .unwrap_or("Pull request")
                            .to_string()
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        Button::new("pr-author")
                            .label(author.clone())
                            .accessibility_label(format!("Open {author}'s profile"))
                            .ghost()
                            .compact()
                            .on_click(move |_, _, cx| cx.open_url(&profile)),
                    )
                    .child(text!("updated"))
                    .child(text!(pr_relative_time(
                        value["updatedAt"].as_str().unwrap_or_default()
                    )))
                    .child(div().flex_1())
                    .child(
                        Button::new("pr-checkout-copy")
                            .label(checkout.clone())
                            .accessibility_label("Copy checkout command")
                            .ghost()
                            .compact()
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(checkout.clone()))
                            }),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(text!(
                        value["baseBranch"].as_str().unwrap_or_default().to_string()
                    ))
                    .child(Icon::new(IconName::ArrowLeft).size(px(14.)))
                    .child(
                        Button::new("pr-branch-copy")
                            .label(branch.clone())
                            .accessibility_label("Copy pull request branch")
                            .ghost()
                            .compact()
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(branch.clone()))
                            }),
                    )
                    .child(div().flex_1())
                    .child(text!(format!(
                        "{files} {}",
                        if files == 1 { "file" } else { "files" }
                    )))
                    .child(
                        h_flex()
                            .text_color(cx.theme().success)
                            .child(text!("+"))
                            .child(text!(
                                value["additions"].as_u64().unwrap_or_default().to_string()
                            )),
                    )
                    .child(
                        h_flex()
                            .text_color(cx.theme().danger)
                            .child(text!("-"))
                            .child(text!(
                                value["deletions"].as_u64().unwrap_or_default().to_string()
                            )),
                    ),
            )
            .into_any_element()
    }

    fn render_pr_summary(&self, view: &Detail, cx: &Context<Self>) -> AnyElement {
        use gpui_kit::component::menu::PopupMenuItem;
        let mut body = v_flex().gap_2().text_xs();
        let reviewers = view.value["reviewers"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        body = body.child(
            h_flex()
                .gap_2()
                .child(div().w(px(96.)).child(text!("Reviewers")))
                .child(text!(if reviewers.is_empty() {
                    "None".into()
                } else {
                    reviewers
                        .iter()
                        .filter_map(|v| v["login"].as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })),
        );
        for section in ["Description", "Checks", "Comments"] {
            let open = !view.collapsed.contains(section);
            let count = view.activity["commentCount"]
                .as_u64()
                .unwrap_or(view.comments.len() as u64);
            let label = if section == "Comments" {
                format!("Comments ({count})")
            } else {
                section.into()
            };
            let mut header = h_flex().gap_1().child(
                Button::new(SharedString::from(format!("pr-section-{section}")))
                    .label(label)
                    .icon(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .ghost()
                    .small()
                    .on_click(cx.listener(move |root, _, _, cx| {
                        if let Some(view) = root
                            .pull_request_id()
                            .and_then(|id| root.pr_state.views.get_mut(&id))
                            && !view.collapsed.remove(section)
                        {
                            view.collapsed.insert(section.into());
                        }
                        root.panel_revision += 1;
                        cx.notify();
                    })),
            );
            if section == "Comments" {
                let weak = cx.entity().downgrade();
                let bots = view.show_bots;
                let resolved = view.show_resolved;
                header = header
                    .child(div().flex_1())
                    .child(
                        Button::new("pr-comment-filter")
                            .label("Filter")
                            .accessibility_label("Filter comments")
                            .ghost()
                            .small()
                            .dropdown_menu(move |mut menu, _, _| {
                                menu = menu.label("Show");
                                for (key, label, checked) in [
                                    ("bots", "Bot comments", bots),
                                    ("resolved", "Resolved or dismissed", resolved),
                                ] {
                                    let weak = weak.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(label).checked(checked).on_click(
                                            move |_, _, cx| {
                                                let _ = weak.update(cx, |root, cx| {
                                                    if let Some(view) =
                                                        root.pull_request_id().and_then(|id| {
                                                            root.pr_state.views.get_mut(&id)
                                                        })
                                                    {
                                                        if key == "bots" {
                                                            view.show_bots = !view.show_bots;
                                                        } else {
                                                            view.show_resolved =
                                                                !view.show_resolved;
                                                        }
                                                    }
                                                    root.panel_revision += 1;
                                                    cx.notify();
                                                });
                                            },
                                        ),
                                    );
                                }
                                menu
                            }),
                    )
                    .child(
                        Button::new("pr-comment-order")
                            .label(if view.oldest_comments {
                                "Oldest first"
                            } else {
                                "Newest first"
                            })
                            .accessibility_label(if view.oldest_comments {
                                "Show newest comments first"
                            } else {
                                "Show oldest comments first"
                            })
                            .ghost()
                            .small()
                            .on_click(cx.listener(|root, _, _, cx| {
                                if let Some(view) = root
                                    .pull_request_id()
                                    .and_then(|id| root.pr_state.views.get_mut(&id))
                                {
                                    view.oldest_comments = !view.oldest_comments;
                                }
                                root.panel_revision += 1;
                                cx.notify();
                            })),
                    );
            }
            body = body.child(header);
            if !open {
                continue;
            }
            if section == "Description" {
                if let Some(document) = &view.body {
                    body = body.child(
                        div()
                            .id("pr-description-text")
                            .role(accesskit::Role::Label)
                            .aria_value(view.value["body"].as_str().unwrap_or_default().to_string())
                            .child(TextView::new(document).selectable(true)),
                    );
                }
            } else if section == "Checks" {
                let checks = view.value["checks"].as_array().cloned().unwrap_or_default();
                if checks.is_empty() {
                    body = body.child(text!("No checks reported."));
                }
                for (index, check) in checks.iter().enumerate() {
                    let url = check["url"].as_str().map(str::to_string);
                    body = body.child(
                        Button::new(("pr-check", index))
                            .label(format!(
                                "{} {}",
                                check["name"].as_str().unwrap_or_default(),
                                check["status"].as_str().unwrap_or_default()
                            ))
                            .ghost()
                            .disabled(url.is_none())
                            .on_click(move |_, _, cx| {
                                if let Some(url) = &url {
                                    cx.open_url(url);
                                }
                            }),
                    );
                }
            } else if view.comments.is_empty() {
                body = body.child(text!("No comments yet."));
            } else {
                let mut indices = (0..view.comments.len()).collect::<Vec<_>>();
                indices.sort_by(|a, b| {
                    view.activity["comments"][*a]["createdAt"]
                        .as_str()
                        .cmp(&view.activity["comments"][*b]["createdAt"].as_str())
                });
                if !view.oldest_comments {
                    indices.reverse();
                }
                for index in indices {
                    let comment = &view.activity["comments"][index];
                    if !view.show_bots && comment["author"]["isBot"] == true
                        || !view.show_resolved && comment["isResolved"] == true
                    {
                        continue;
                    }
                    body = body.child(
                        v_flex()
                            .id(("pr-comment", index))
                            .gap_2()
                            .child(text!(
                                comment["author"]["login"]
                                    .as_str()
                                    .unwrap_or("ghost")
                                    .to_string()
                            ))
                            .child(TextView::new(&view.comments[index]).selectable(true)),
                    );
                }
            }
        }
        body.into_any_element()
    }
}

fn pr_checks_label(value: &Value) -> String {
    match value["checksState"].as_str() {
        Some("passing") => "All checks passed".into(),
        Some("failing") => "Some checks failed".into(),
        Some("pending") => "Checks pending".into(),
        _ => "No checks reported".into(),
    }
}

fn pr_relative_time(value: &str) -> String {
    let Ok(time) = humantime::parse_rfc3339(value) else {
        return String::new();
    };
    let elapsed = SystemTime::now()
        .duration_since(time)
        .unwrap_or_default()
        .as_secs();
    match elapsed {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", elapsed / 60),
        3600..86400 => format!("{}h ago", elapsed / 3600),
        _ => format!("{}d ago", elapsed / 86400),
    }
}

impl Root {
    fn checkout_pr(&mut self, mode: &'static str, cx: &mut Context<Self>) {
        let Some(id) = self.pull_request_id() else {
            return;
        };
        let Some(view) = self.pr_state.views.get_mut(&id) else {
            return;
        };
        let environment = view.environment.clone();
        let project_id = view.reference["projectId"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let input =
            json!({"cwd":view.value["workspaceRoot"],"reference":view.value["url"],"mode":mode});
        view.busy = true;
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                connection
                    .request("git.preparePullRequestThread", input)
                    .await
            })
            .await;
            let _ = this.update(cx, |root, cx| {
                if let Some(view) = root.pr_state.views.get_mut(&id) {
                    view.busy = false;
                    if let Err(error) = &result {
                        view.error = Some(format!(
                            "Could not prepare the pull request checkout: {error:#}"
                        ));
                    }
                }
                if let Ok(value) = result {
                    let group = root
                        .project_groups()
                        .iter()
                        .find(|group| group.contains(&environment, &project_id))
                        .map(|group| group.key.clone());
                    root.begin_new_thread(cx);
                    if let Some(draft) = root.conversation.draft.as_mut() {
                        draft.environment = Some(environment);
                        draft.group = group;
                        draft.workspace_strategy = if value["worktreePath"].is_string() {
                            json!({"type":"existing_worktree","worktreePath":value["worktreePath"],"branch":value["branch"]})
                        } else {
                            json!({"type":"root"})
                        };
                    }
                    root.pr_state.page = false;
                }
                root.panel_revision += 1;
                cx.notify();
            });
        })
        .detach();
        self.panel_revision += 1;
        cx.notify();
    }
}

fn pr_group_rank(entry: &Value) -> usize {
    match entry["_group"].as_str() {
        Some("authored") => 0,
        Some("reviewing") => 1,
        _ => 2,
    }
}

#[cfg(test)]
mod review_tests {
    use super::review_can_submit;
    #[test]
    fn review_validation_matches_host_requirements() {
        assert!(review_can_submit("approve", "", 0, false));
        assert!(!review_can_submit("comment", " \n", 0, false));
        assert!(review_can_submit("request-changes", "", 1, false));
        assert!(!review_can_submit("request-changes", "", 1, true));
        assert!(review_can_submit("request-changes", "Reason", 0, true));
    }
}
