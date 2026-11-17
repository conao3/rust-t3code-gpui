use super::*;

struct AddProject {
    root: WeakEntity<Root>,
    environment: String,
    connection: Arc<Connection>,
    runtime: Handle,
    input: Entity<InputState>,
    mode: Option<bool>,
    busy: bool,
    error: Option<String>,
    _subscription: Subscription,
}

impl AddProject {
    fn submit(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_string();
        let Some(create) = self.mode else {
            return;
        };
        if text.is_empty() || self.busy {
            return;
        }
        let connection = self.connection.clone();
        let runtime = self.runtime.clone();
        self.busy = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                let value = if create {
                    connection.request("projects.createNew", json!({"name":text})).await?
                } else {
                    let title = text.trim_end_matches('/').rsplit('/').next().context("Project path missing")?;
                    connection.request("projects.mutate", json!({"type":"project.create","commandId":uuid::Uuid::new_v4().to_string(),"projectId":uuid::Uuid::new_v4().to_string(),"title":title,"workspaceRoot":text,"createWorkspaceRootIfMissing":true,"defaultModelSelection":null})).await?
                };
                let id = value[if create { "projectId" } else { "id" }].as_str().context("Created project ID missing")?.to_string();
                for _ in 0..100 {
                    if connection.shell().lock().expect("shell").projects.iter().any(|project| project["id"] == id) { return Ok(id); }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                anyhow::bail!("Created project did not arrive in the shell")
            }).await;
            let _ = this.update(cx, |view, cx| {
                view.busy = false;
                match result {
                    Ok(project) => {
                        let environment = view.environment.clone();
                        let _ = view.root.update(cx, |root, cx| {
                            let group = root.project_groups().into_iter().find(|group| group.contains(&environment, &project));
                            root.begin_new_thread(cx);
                            if let (Some(draft), Some(group)) = (&mut root.conversation.draft, group) {
                                draft.environment = Some(environment);
                                draft.group = Some(group.key);
                            }
                        });
                        cx.emit(ProjectAdded);
                    }
                    Err(error) => view.error = Some(format!("Failed to add project: {error:#}")),
                }
                cx.notify();
            });
        }).detach();
    }
}

struct ProjectAdded;
impl EventEmitter<ProjectAdded> for AddProject {}

impl Render for AddProject {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_3()
            .when(self.mode.is_none(), |view| {
                view.child(
                    v_flex()
                        .id("sidebar-project-sources")
                        .role(accesskit::Role::ListBox)
                        .aria_label("Sources")
                        .children(
                            [
                                (
                                    true,
                                    "New project",
                                    "Start a new Git repository from a name",
                                ),
                                (false, "Local folder", "Browse a folder on disk"),
                            ]
                            .into_iter()
                            .map(|(create, title, description)| {
                                Button::new(title)
                                    .role(accesskit::Role::ListBoxOption)
                                    .accessibility_label(format!("{title} {description}"))
                                    .ghost()
                                    .w_full()
                                    .h(px(52.))
                                    .child(
                                        v_flex().w_full().items_start().child(title).child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(description),
                                        ),
                                    )
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.mode = Some(create);
                                        view.input.update(cx, |input, cx| input.focus(window, cx));
                                        cx.notify();
                                    }))
                            }),
                        ),
                )
            })
            .when_some(self.mode, |view, create| {
                view.child(
                    Input::new(&self.input)
                        .role(accesskit::Role::ComboBox)
                        .aria_label(if create {
                            "New project"
                        } else {
                            "Project path"
                        })
                        .disabled(self.busy),
                )
                .child(
                    Button::new("confirm-add-project")
                        .label(if create {
                            "Create project"
                        } else {
                            "Add project"
                        })
                        .primary()
                        .disabled(self.busy || self.input.read(cx).value().trim().is_empty())
                        .on_click(cx.listener(|view, _, _, cx| view.submit(cx))),
                )
            })
            .children(
                self.error
                    .clone()
                    .map(|error| div().text_color(cx.theme().danger).child(error)),
            )
    }
}

impl Root {
    pub(super) fn open_add_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection) = self.connections.first().cloned() else {
            return;
        };
        let root = cx.entity().downgrade();
        let environment = connection.label().to_string();
        let runtime = self.runtime.clone();
        let view = cx.new(|cx| {
            let input = cx.new(|cx| InputState::new(window, cx));
            let subscription = cx.subscribe(&input, |view: &mut AddProject, _, event, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    view.submit(cx);
                }
                cx.notify();
            });
            AddProject {
                root,
                environment,
                connection,
                runtime,
                input,
                mode: None,
                busy: false,
                error: None,
                _subscription: subscription,
            }
        });
        cx.subscribe_in(&view, window, |root, _, _: &ProjectAdded, window, cx| {
            window.close_dialog(cx);
            root.conversation
                .composer
                .update(cx, |input, cx| input.focus(window, cx));
        })
        .detach();
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Add project")
                .accessibility_label("Add project")
                .width(px(640.))
                .child(view.clone())
        });
    }
}
