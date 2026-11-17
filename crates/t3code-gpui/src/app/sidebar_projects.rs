use super::*;

#[derive(Clone)]
enum Step {
    Hosts,
    Sources,
    New,
    Browse,
    Remote(String),
    Destination { url: String, title: String },
}

struct Item {
    key: String,
    title: String,
    description: String,
    enabled: bool,
}

struct AddProject {
    root: WeakEntity<Root>,
    connections: Vec<Arc<Connection>>,
    hosts: Vec<String>,
    host: usize,
    runtime: Handle,
    input: Entity<InputState>,
    step: Step,
    back: Vec<(Step, String)>,
    selected: Option<usize>,
    discovery: Value,
    browse: Value,
    request: u64,
    busy: bool,
    error: Option<String>,
    _subscription: Subscription,
}

const SOURCES: [(&str, &str, &str); 7] = [
    ("azure-devops", "Azure DevOps", "project/repository"),
    ("bitbucket", "Bitbucket", "workspace/repository"),
    ("cursor-origin", "Cursor Origin", "owner/repo"),
    ("forgejo", "Forgejo / Gitea", "owner/repo"),
    ("gitcafe", "GitCafe", "owner/repo"),
    ("github", "GitHub", "owner/repo"),
    ("gitlab", "GitLab", "group/project"),
];

impl AddProject {
    fn connection(&self) -> Arc<Connection> {
        self.connections[self.host].clone()
    }

    fn config(&self) -> Value {
        self.connection()
            .config()
            .lock()
            .expect("config lock")
            .clone()
            .unwrap_or(Value::Null)
    }

    fn initial_path(&self) -> String {
        let config = self.config();
        let path = config
            .pointer("/settings/addProjectBaseDirectory")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if path.is_empty() {
            "~/".into()
        } else {
            format!("{}/", path.trim_end_matches('/'))
        }
    }

    fn ready(&self, kind: &str) -> bool {
        self.discovery["sourceControlProviders"]
            .as_array()
            .and_then(|items| items.iter().find(|p| p["kind"] == kind))
            .is_some_and(|p| p["status"] == "available" && p["auth"]["status"] != "unauthenticated")
    }

    fn items(&self, cx: &App) -> Vec<Item> {
        let query = self.input.read(cx).value().to_string();
        let mut items = match &self.step {
            Step::Hosts => self
                .connections
                .iter()
                .enumerate()
                .map(|(index, connection)| Item {
                    key: index.to_string(),
                    title: self.hosts[index].clone(),
                    description: if index == 0 {
                        "This device".into()
                    } else {
                        connection.environment_id().unwrap_or_default()
                    },
                    enabled: matches!(connection.status(), ConnectionStatus::Connected),
                })
                .collect(),
            Step::Sources => {
                let mut items = vec![];
                if self.config()["newProjectsRoot"].is_string() {
                    items.push(Item {
                        key: "new".into(),
                        title: "New project".into(),
                        description: "Start a new Git repository from a name".into(),
                        enabled: true,
                    });
                }
                items.push(Item {
                    key: "local".into(),
                    title: "Local folder".into(),
                    description: "Browse a folder on disk".into(),
                    enabled: true,
                });
                items.push(Item {
                    key: "url".into(),
                    title: "Git URL".into(),
                    description: "Clone from a remote URL".into(),
                    enabled: true,
                });
                let mut sources = SOURCES.to_vec();
                sources.sort_by_key(|(kind, title, _)| (!self.ready(kind), *title));
                items.extend(sources.into_iter().map(|(kind, title, hint)| Item {
                    key: kind.into(),
                    title: format!("{title} repository"),
                    description: format!("Clone {title} {hint}"),
                    enabled: self.ready(kind),
                }));
                items
            }
            Step::Browse | Step::Destination { .. } => {
                let mut items = vec![];
                let path = if query.ends_with('/') {
                    query.trim_end_matches('/')
                } else {
                    query.rsplit_once('/').map_or("", |(parent, _)| parent)
                };
                if path != "~" && !path.is_empty() {
                    items.push(Item {
                        key: "..".into(),
                        title: "..".into(),
                        description: String::new(),
                        enabled: true,
                    });
                }
                let filter =
                    if query.ends_with('/') || matches!(self.step, Step::Destination { .. }) {
                        ""
                    } else {
                        query.rsplit('/').next().unwrap_or("")
                    };
                items.extend(
                    self.browse["entries"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|entry| {
                            entry["name"].as_str().is_some_and(|name| {
                                name.to_lowercase().contains(&filter.to_lowercase())
                            })
                        })
                        .map(|entry| Item {
                            key: entry["fullPath"].as_str().unwrap_or_default().into(),
                            title: entry["name"].as_str().unwrap_or_default().into(),
                            description: String::new(),
                            enabled: true,
                        }),
                );
                items
            }
            _ => vec![],
        };
        if matches!(self.step, Step::Hosts | Step::Sources) {
            items.retain(|item| {
                format!("{} {}", item.title, item.description)
                    .to_lowercase()
                    .contains(&query.to_lowercase())
            });
        }
        items
    }

    fn placeholder(&self) -> String {
        match &self.step {
            Step::Hosts | Step::Sources => "Search...".into(),
            Step::New => "Project name".into(),
            Step::Browse | Step::Destination { .. } => "Enter path (e.g. ~/projects/my-app)".into(),
            Step::Remote(kind) if kind == "url" => "Enter Git clone URL".into(),
            Step::Remote(kind) => {
                let (_, title, hint) = SOURCES
                    .iter()
                    .find(|(key, _, _)| key == kind)
                    .expect("remote source");
                format!("Enter {title} repository ({hint})")
            }
        }
    }

    fn change_step(
        &mut self,
        step: Step,
        query: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.back
            .push((self.step.clone(), self.input.read(cx).value().to_string()));
        self.step = step;
        self.selected = None;
        let placeholder = self.placeholder();
        self.input.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
            input.set_value(query, window, cx);
            input.focus(window, cx);
        });
        self.refresh(cx);
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.request += 1;
        self.error = None;
        let (method, payload) = match self.step {
            Step::Sources => ("server.discoverSourceControl", json!({})),
            Step::Browse | Step::Destination { .. } => {
                let query = self.input.read(cx).value().to_string();
                let directory = if query.ends_with('/') {
                    query
                } else {
                    query
                        .rsplit_once('/')
                        .map(|(dir, _)| format!("{dir}/"))
                        .unwrap_or_default()
                };
                if directory.is_empty() {
                    self.browse = Value::Null;
                    cx.notify();
                    return;
                }
                let mut payload = json!({"partialPath":directory});
                if let Ok(Some(path)) = self.root.read_with(cx, |root, _| {
                    root.open_entry()
                        .filter(|entry| entry.environment == self.connection().label())
                        .and_then(|entry| root.project_value(&entry.environment, &entry.project_id))
                        .and_then(|project| project["workspaceRoot"].as_str().map(str::to_owned))
                }) {
                    payload["cwd"] = json!(path);
                }
                ("filesystem.browse", payload)
            }
            _ => {
                cx.notify();
                return;
            }
        };
        let connection = self.connection();
        let runtime = self.runtime.clone();
        let request = self.request;
        cx.spawn(async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                connection.request(method, payload).await
            })
            .await;
            let _ = this.update(cx, |view, cx| {
                if view.request != request {
                    return;
                }
                match result {
                    Ok(value) if method == "filesystem.browse" => view.browse = value,
                    Ok(value) => view.discovery = value,
                    Err(error) => view.error = Some(error.root_cause().to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn choose(&mut self, key: String, window: &mut Window, cx: &mut Context<Self>) {
        match self.step {
            Step::Hosts => {
                self.host = key.parse().expect("host index");
                self.change_step(Step::Sources, String::new(), window, cx);
            }
            Step::Sources => match key.as_str() {
                "new" => self.change_step(Step::New, String::new(), window, cx),
                "local" => self.change_step(Step::Browse, self.initial_path(), window, cx),
                _ => self.change_step(Step::Remote(key), String::new(), window, cx),
            },
            Step::Browse | Step::Destination { .. } => {
                let path = if key == ".." {
                    let value = self.input.read(cx).value().to_string();
                    value
                        .trim_end_matches('/')
                        .rsplit_once('/')
                        .map(|(parent, _)| format!("{parent}/"))
                        .unwrap_or_else(|| "~/".into())
                } else {
                    format!("{}/", key.trim_end_matches('/'))
                };
                self.selected = None;
                self.input
                    .update(cx, |input, cx| input.set_value(path, window, cx));
                self.refresh(cx);
            }
            _ => {}
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_string();
        if text.is_empty() || self.busy {
            return;
        }
        if let Step::Remote(kind) = &self.step {
            let kind = kind.clone();
            if kind == "url" {
                self.destination(text.clone(), text, window, cx);
                return;
            }
            let connection = self.connection();
            let runtime = self.runtime.clone();
            self.busy = true;
            cx.spawn_in(window, async move |this, cx| {
                let result = on_runtime(&runtime, async move {
                    connection
                        .request(
                            "sourceControl.lookupRepository",
                            json!({"provider":kind,"repository":text}),
                        )
                        .await
                })
                .await;
                let _ = this.update_in(cx, |view, window, cx| {
                    view.busy = false;
                    match result {
                        Ok(repository) => view.destination(
                            repository[if matches!(
                                repository["provider"].as_str(),
                                Some("github" | "forgejo" | "cursor-origin")
                            ) {
                                "url"
                            } else {
                                "sshUrl"
                            }]
                            .as_str()
                            .unwrap_or_default()
                            .into(),
                            repository["nameWithOwner"]
                                .as_str()
                                .unwrap_or_default()
                                .into(),
                            window,
                            cx,
                        ),
                        Err(error) => view.error = Some(error.root_cause().to_string()),
                    }
                    cx.notify();
                });
            })
            .detach();
            cx.notify();
            return;
        }
        let connection = self.connection();
        let runtime = self.runtime.clone();
        let step = self.step.clone();
        if !matches!(step, Step::New | Step::Browse | Step::Destination { .. }) {
            return;
        }
        let environment = connection.label().to_string();
        let existing = connection
            .shell()
            .lock()
            .expect("shell lock")
            .projects
            .iter()
            .find(|project| project["workspaceRoot"] == text)
            .cloned();
        self.busy = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                let id = if let Some(existing) = existing {
                    let id = existing["id"].as_str().context("Project ID missing")?.to_string();
                    if !existing["archivedAt"].is_null() { connection.request("projects.mutate", json!({"type":"project.update","commandId":uuid::Uuid::new_v4().to_string(),"projectId":id,"archived":false})).await?; }
                    id
                } else if matches!(step, Step::New) {
                    connection.request("projects.createNew", json!({"name":text})).await?["projectId"].as_str().context("Created project ID missing")?.to_string()
                } else {
                    let id = uuid::Uuid::new_v4().to_string();
                    let title = text.trim_end_matches('/').rsplit('/').next().context("Project path missing")?;
                    if let Step::Destination { url, .. } = step {
                        connection.request("projectClone.start", json!({"projectId":id,"title":title,"createdAt":now_iso(),"remoteUrl":url,"destinationPath":text})).await?;
                    } else {
                        connection.request("projects.mutate", json!({"type":"project.create","commandId":uuid::Uuid::new_v4().to_string(),"projectId":id,"title":title,"workspaceRoot":text,"createWorkspaceRootIfMissing":true,"defaultModelSelection":null})).await?;
                    }
                    id
                };
                for _ in 0..100 {
                    if connection.shell().lock().expect("shell lock").projects.iter().any(|project| project["id"] == id) { return Ok(id); }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                anyhow::bail!("Created project did not arrive in the shell")
            }).await;
            let _ = this.update(cx, |view, cx| {
                view.busy = false;
                match result {
                    Ok(project) => {
                        let _ = view.root.update(cx, |root, cx| {
                            let group = root.project_groups().into_iter().find(|group| group.contains(&environment, &project));
                            root.begin_new_thread(cx);
                            if let (Some(draft), Some(group)) = (&mut root.conversation.draft, group) { draft.environment = Some(environment); draft.group = Some(group.key); }
                        });
                        cx.emit(ProjectAdded);
                    }
                    Err(error) => view.error = Some(error.root_cause().to_string()),
                }
                cx.notify();
            });
        }).detach();
    }

    fn destination(
        &mut self,
        url: String,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = url
            .split(['?', '#'])
            .next()
            .unwrap_or("")
            .trim_end_matches('/')
            .rsplit(['/', ':'])
            .next()
            .unwrap_or("")
            .trim_end_matches(".git");
        let path = format!("{}{name}", self.initial_path());
        self.change_step(Step::Destination { url, title }, path, window, cx);
    }
}

struct ProjectAdded;
impl EventEmitter<ProjectAdded> for AddProject {}

impl Render for AddProject {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let items = self.items(cx);
        let browsing = matches!(self.step, Step::Browse | Step::Destination { .. });
        let query = self.input.read(cx).value().to_string();
        let create = browsing
            && self.selected.is_none()
            && !query.ends_with('/')
            && !self.browse["entries"].as_array().is_some_and(|entries| {
                entries
                    .iter()
                    .any(|entry| entry["name"].as_str() == query.rsplit('/').next())
            });
        let action = match self.step {
            Step::New => Some("Create"),
            Step::Browse => Some(if create { "Create & Add" } else { "Add" }),
            Step::Remote(ref kind) => Some(if kind == "url" { "Continue" } else { "Lookup" }),
            Step::Destination { .. } => Some(if create { "Create & Clone" } else { "Clone" }),
            _ => None,
        };
        let group = match self.step {
            Step::Hosts => "Environments",
            Step::Sources => "Sources",
            Step::Browse => "Directories",
            Step::Destination { .. } => "Select where to clone",
            _ => "",
        };
        let shortcut = if browsing && self.selected.is_some() {
            "Ctrl Enter"
        } else {
            "Enter"
        };
        v_flex()
            .gap_0()
            .text_sm()
            .line_height(px(20.))
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                let items = view.items(cx);
                match event.keystroke.key.as_str() {
                    "down" | "up" if !items.is_empty() => {
                        let next = match (view.selected, event.keystroke.key.as_str()) {
                            (Some(i), "up") => (i + items.len() - 1) % items.len(),
                            (Some(i), _) => (i + 1) % items.len(),
                            (None, "up") => items.len() - 1,
                            _ => 0,
                        };
                        view.selected = Some(next);
                    }
                    "backspace" if view.input.read(cx).value().is_empty() => {
                        if let Some((step, query)) = view.back.pop() {
                            view.step = step;
                            view.selected = Some(0);
                            let placeholder = view.placeholder();
                            view.input.update(cx, |input, cx| {
                                input.set_placeholder(placeholder, window, cx);
                                input.set_value(query, window, cx);
                            });
                            view.refresh(cx);
                        }
                    }
                    _ => return,
                }
                window.prevent_default();
                cx.stop_propagation();
                cx.notify();
            }))
            .child(
                h_flex()
                    .px_4()
                    .h(px(56.))
                    .gap_2()
                    .child(Icon::new(IconName::ArrowLeft).size(px(16.)))
                    .child(
                        Input::new(&self.input)
                            .role(accesskit::Role::EditableComboBox)
                            .aria_label(self.placeholder())
                            .appearance(false)
                            .disabled(self.busy)
                            .flex_1(),
                    )
                    .when_some(action, |row, action| {
                        row.child(
                            Button::new("submit-project")
                                .label(format!("{action}  {shortcut}"))
                                .accessibility_label(format!("{action} ({shortcut})"))
                                .compact()
                                .disabled(
                                    self.busy || self.input.read(cx).value().trim().is_empty(),
                                )
                                .on_click(
                                    cx.listener(|view, _, window, cx| view.submit(window, cx)),
                                ),
                        )
                    }),
            )
            .when_some(
                if let Step::Destination { url, title } = &self.step {
                    Some((url.clone(), title.clone()))
                } else {
                    None
                },
                |view, (url, title)| {
                    view.child(
                        v_flex()
                            .p_3()
                            .gap_1()
                            .child(settings_controls::text("Repository"))
                            .child(settings_controls::text(title).id("repository-title"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(settings_controls::text(url).id("repository-url")),
                            ),
                    )
                },
            )
            .child(
                v_flex()
                    .id("project-source-list")
                    .role(accesskit::Role::ListBox)
                    .max_h(px(if matches!(self.step, Step::Destination { .. }) {
                        230.
                    } else {
                        324.
                    }))
                    .overflow_y_scroll()
                    .p_2()
                    .when(!group.is_empty(), |list| {
                        list.child(
                            div()
                                .px_2()
                                .py_1()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(settings_controls::text(group)),
                        )
                    })
                    .children(items.into_iter().enumerate().map(|(index, item)| {
                        let title = item.title.clone();
                        let label = if item.description.is_empty() {
                            title.clone()
                        } else {
                            format!("{} {}", title, item.description)
                        };
                        h_flex()
                            .id(("project-option-row", index))
                            .gap_1()
                            .child(
                                Button::new(("project-option", index))
                                    .role(if item.enabled {
                                        accesskit::Role::ListBoxOption
                                    } else {
                                        accesskit::Role::Group
                                    })
                                    .accessibility_label(if item.enabled {
                                        label
                                    } else {
                                        String::new()
                                    })
                                    .ghost()
                                    .flex_1()
                                    .min_w_0()
                                    .h(px(if item.description.is_empty() {
                                        32.
                                    } else {
                                        48.
                                    }))
                                    .selected(self.selected == Some(index))
                                    .disabled(!item.enabled)
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .w_full()
                                            .child(
                                                Icon::new(gpui_kit::assets::IconName::Folder)
                                                    .size(px(16.)),
                                            )
                                            .child(
                                                v_flex()
                                                    .items_start()
                                                    .child(settings_controls::text(title))
                                                    .when(!item.description.is_empty(), |row| {
                                                        row.child(
                                                            div()
                                                                .text_xs()
                                                                .text_color(
                                                                    cx.theme().muted_foreground,
                                                                )
                                                                .child(settings_controls::text(
                                                                    item.description,
                                                                )),
                                                        )
                                                    }),
                                            ),
                                    )
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.choose(item.key.clone(), window, cx)
                                    })),
                            )
                            .when(!item.enabled && matches!(self.step, Step::Sources), |row| {
                                row.child(
                                    Button::new(("source-setup", index))
                                        .label("Setup Required")
                                        .compact()
                                        .on_click(cx.listener(|view, _, window, cx| {
                                            window.close_dialog(cx);
                                            let _ = view.root.update(cx, |root, cx| {
                                                root.settings_page = "source-control";
                                                root.settings_open = true;
                                                cx.notify();
                                            });
                                        })),
                                )
                            })
                    }))
                    .when(
                        matches!(&self.step, Step::Remote(kind) if kind == "url"),
                        |list| {
                            list.child(div().p_4().text_color(cx.theme().muted_foreground).child(
                                settings_controls::text(
                                    "Enter a Git clone URL and press Enter to continue.",
                                ),
                            ))
                        },
                    ),
            )
            .children(
                self.error
                    .clone()
                    .map(|error| div().px_3().text_color(cx.theme().danger).child(error)),
            )
            .child(
                h_flex()
                    .px_3()
                    .h(px(40.))
                    .gap_3()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(settings_controls::text("Navigate"))
                    .when(!browsing, |row| {
                        row.child(settings_controls::text("Enter"))
                            .child(settings_controls::text(action.unwrap_or("Select")))
                    })
                    .child(settings_controls::text("Backspace"))
                    .child(settings_controls::text("Back"))
                    .child(settings_controls::text("Esc"))
                    .child(settings_controls::text("Close")),
            )
    }
}

impl Root {
    pub(super) fn open_add_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connections.is_empty() {
            self.hosts_open = true;
            cx.notify();
            return;
        }
        let root = cx.entity().downgrade();
        let connections = self.connections.clone();
        let hosts = connections
            .iter()
            .map(|connection| self.host_label(connection.label()))
            .collect();
        let step = if connections.len() > 1 {
            Step::Hosts
        } else {
            Step::Sources
        };
        let runtime = self.runtime.clone();
        let view = cx.new(|cx| {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search..."));
            let subscription = cx.subscribe_in(
                &input,
                window,
                |view: &mut AddProject, _, event, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        let items = view.items(cx);
                        if !window.modifiers().control
                            && let Some(item) = view.selected.and_then(|i| items.get(i))
                        {
                            if item.enabled {
                                view.choose(item.key.clone(), window, cx);
                            }
                        } else {
                            view.submit(window, cx);
                        }
                    } else if matches!(event, InputEvent::Change) {
                        view.selected = if matches!(view.step, Step::Hosts | Step::Sources) {
                            Some(0)
                        } else {
                            None
                        };
                        view.refresh(cx);
                    }
                },
            );
            input.update(cx, |input, cx| input.focus(window, cx));
            let mut view = AddProject {
                root,
                connections,
                hosts,
                host: 0,
                runtime,
                input,
                step,
                back: vec![],
                selected: None,
                discovery: Value::Null,
                browse: Value::Null,
                request: 0,
                busy: false,
                error: None,
                _subscription: subscription,
            };
            view.refresh(cx);
            view
        });
        cx.subscribe_in(&view, window, |root, _, _: &ProjectAdded, window, cx| {
            window.close_dialog(cx);
            root.conversation
                .composer
                .update(cx, |input, cx| input.focus(window, cx));
        })
        .detach();
        window.open_dialog(cx, move |dialog, window, _| {
            dialog
                .accessibility_label("Command palette")
                .on_ok(|_, _, _| false)
                .close_button(false)
                .margin_top((window.viewport_size().height * 0.1).max(px(48.)))
                .width(px(576.))
                .p_0()
                .child(view.clone())
        });
    }
}
