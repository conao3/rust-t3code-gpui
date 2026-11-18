use super::*;
use gpui_kit::component::{
    select::{Select, SelectEvent},
    switch::Switch,
};
use t3_client::rpc::StreamLimits;

pub(super) struct Tasks {
    saving: bool,
    root: WeakEntity<Root>,
    environment: String,
    environment_label: String,
    project_filter: Option<String>,
    tasks: Vec<Value>,
    error: Option<String>,
    worker: Option<tokio::task::JoinHandle<()>>,
    editing: Option<Value>,
    inputs: [Entity<InputState>; 5],
    project: Entity<settings_controls::Choices>,
    model: Entity<settings_controls::Choices>,
    schedule: Entity<choice_select::State>,
    workspace: Entity<choice_select::State>,
    weekdays: [bool; 7],
    enabled: bool,
    start_from_origin: bool,
    subscriptions: Vec<Subscription>,
}
impl Drop for Tasks {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.abort();
        }
    }
}
impl Tasks {
    pub(super) fn new(
        root: WeakEntity<Root>,
        environment: String,
        owner: &Root,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let connection = owner.connection(&environment);
        let runtime = owner.runtime.clone();
        let projects = connection
            .as_ref()
            .map(|connection| connection.shell().lock().expect("shell").projects.clone())
            .unwrap_or_default();
        let config = owner.config_of(&environment).unwrap_or(Value::Null);
        let project = settings_controls::select(
            projects
                .iter()
                .filter_map(|project| {
                    Some((
                        project["id"].as_str()?.to_string(),
                        project["title"].as_str()?.to_string(),
                    ))
                })
                .collect(),
            owner.settings_project.as_deref().unwrap_or(""),
            window,
            cx,
        );
        let models = models::instances(&config)
            .into_iter()
            .flat_map(|instance| {
                instance.models.into_iter().map(move |model| {
                    (
                        format!("{}:{}", instance.instance_id, model.slug),
                        format!("{} · {}", instance.name, model.name),
                    )
                })
            })
            .collect();
        let model = settings_controls::select(models, "", window, cx);
        let schedule = choice_select::state(
            &[
                ("fixed_time", "Fixed time"),
                ("interval", "Interval"),
                ("once", "Once"),
            ],
            "fixed_time",
            window,
            cx,
        );
        let workspace = choice_select::state(
            &[
                ("worktree", "Create a new worktree"),
                ("root", "Use the project checkout"),
                ("existing_worktree", "Use a specific checkout"),
            ],
            "worktree",
            window,
            cx,
        );
        let inputs = ["Task name", "Prompt", "09:00", "main", "Checkout path"]
            .map(|label| cx.new(|cx| InputState::new(window, cx).placeholder(label)));
        let mut view = Self {
            saving: false,
            root,
            environment,
            environment_label: config["environment"]["label"]
                .as_str()
                .unwrap_or(&owner.settings_environment())
                .to_string(),
            project_filter: owner.settings_project.clone(),
            tasks: vec![],
            error: None,
            worker: None,
            editing: None,
            inputs,
            project,
            model,
            schedule,
            workspace,
            weekdays: [true, true, true, true, true, false, false],
            enabled: true,
            start_from_origin: true,
            subscriptions: vec![],
        };
        for select in [&view.schedule, &view.workspace] {
            view.subscriptions
                .push(cx.subscribe(select, |_, _, _: &SelectEvent<_>, cx| cx.notify()));
        }
        if let Some(connection) = connection {
            let (sender, receiver) = async_channel::bounded(8);
            view.worker = Some(runtime.spawn(async move {
                let result: anyhow::Result<()> = async {
                    let mut stream = connection
                        .subscribe(
                            "scheduledTasks.subscribe",
                            json!({}),
                            StreamLimits::new(8, 4 << 20).expect("positive stream limits"),
                        )
                        .await?;
                    while let Some(chunk) = stream.next().await {
                        let chunk = chunk?;
                        for value in chunk.values() {
                            if sender.send(Ok(value.clone())).await.is_err() {
                                return Ok(());
                            }
                        }
                        chunk.accept();
                    }
                    Ok(())
                }
                .await;
                if let Err(error) = result {
                    let _ = sender
                        .send(Err(format!("Unable to load scheduled tasks: {error:#}")))
                        .await;
                }
            }));
            cx.spawn(async move |this, cx| {
                while let Ok(mut value) = receiver.recv().await {
                    cx.background_executor().timer(REDRAW_INTERVAL).await;
                    while let Ok(next) = receiver.try_recv() {
                        value = next;
                    }
                    if this
                        .update(cx, |view, cx| {
                            match value {
                                Ok(value) => {
                                    view.tasks = value["tasks"]
                                        .as_array()
                                        .into_iter()
                                        .flatten()
                                        .filter(|task| {
                                            view.project_filter
                                                .as_ref()
                                                .is_none_or(|project| task["projectId"] == *project)
                                        })
                                        .cloned()
                                        .collect();
                                    view.error = None;
                                }
                                Err(error) => view.error = Some(error),
                            }
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }
        view
    }
    pub(super) fn edit(&mut self, task: Value, window: &mut Window, cx: &mut Context<Self>) {
        let schedule = task["schedule"]["type"].as_str().unwrap_or("fixed_time");
        let schedule_value = match schedule {
            "interval" => {
                (task["schedule"]["everyMs"].as_i64().unwrap_or(900000) / 60000).to_string()
            }
            "once" => task["schedule"]["runAt"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            _ => task["schedule"]["timeOfDay"]
                .as_str()
                .unwrap_or("09:00")
                .to_string(),
        };
        for (input, value) in self.inputs.iter().zip([
            task["title"].as_str().unwrap_or_default().to_string(),
            task["prompt"].as_str().unwrap_or_default().to_string(),
            schedule_value,
            task["workspaceStrategy"]["baseRef"]
                .as_str()
                .unwrap_or("main")
                .to_string(),
            task["workspaceStrategy"]["worktreePath"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        ]) {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        let schedule = match schedule {
            "interval" => "interval",
            "once" => "once",
            _ => "fixed_time",
        };
        self.schedule.update(cx, |select, cx| {
            select.set_selected_value(&schedule, window, cx)
        });
        let workspace = match task["workspaceStrategy"]["type"].as_str() {
            Some("root") => "root",
            Some("existing_worktree") => "existing_worktree",
            _ => "worktree",
        };
        self.workspace.update(cx, |select, cx| {
            select.set_selected_value(&workspace, window, cx)
        });
        if let Some(project) = task["projectId"].as_str() {
            self.project.update(cx, |select, cx| {
                select.set_selected_value(&project.to_string(), window, cx)
            });
        }
        if let (Some(instance), Some(model)) = (
            task["modelSelection"]["instanceId"].as_str(),
            task["modelSelection"]["model"].as_str(),
        ) {
            self.model.update(cx, |select, cx| {
                select.set_selected_value(&format!("{instance}:{model}"), window, cx)
            });
        }
        self.weekdays = std::array::from_fn(|index| {
            task["schedule"]["weekdays"]
                .as_array()
                .map_or(index < 5, |days| days.contains(&json!((index + 1) % 7)))
        });
        self.enabled = task["enabled"].as_bool().unwrap_or(true);
        self.start_from_origin = task["workspaceStrategy"]["startFromOrigin"]
            .as_bool()
            .unwrap_or(true);
        self.editing = Some(task);
        cx.notify();
    }
    fn request(&self, method: &'static str, payload: Value, cx: &mut Context<Self>) {
        let _ = self.root.update(cx, |root, cx| {
            root.settings_request(&self.environment, method, payload, cx)
        });
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let [title, prompt, time, base, path] = self
            .inputs
            .clone()
            .map(|input| input.read(cx).value().trim().to_string());
        let Some(project) = self.project.read(cx).selected_value().cloned() else {
            self.error = Some("Choose a project.".into());
            cx.notify();
            return;
        };
        let Some(model) = self.model.read(cx).selected_value().cloned() else {
            self.error = Some("Choose a model.".into());
            cx.notify();
            return;
        };
        let Some((instance, model)) = model.split_once(':') else {
            return;
        };
        let schedule = match self.schedule.read(cx).selected_value().copied() {
            Some("interval") => match time
                .parse::<u64>()
                .ok()
                .filter(|minutes| *minutes >= 1 && *minutes <= u64::MAX / 60000)
            {
                Some(minutes) => json!({"type":"interval","everyMs":minutes*60000}),
                None => {
                    self.error = Some("Interval must be at least one minute.".into());
                    cx.notify();
                    return;
                }
            },
            Some("once") => json!({"type":"once","runAt":time}),
            _ => {
                json!({"type":"fixed_time","timeOfDay":time,"weekdays":self.weekdays.iter().enumerate().filter(|(_,selected)|**selected).map(|(index,_)|(index+1)%7).collect::<Vec<_>>()})
            }
        };
        let workspace = match self.workspace.read(cx).selected_value().copied() {
            Some("root") => json!({"type":"root"}),
            Some("existing_worktree") => json!({"type":"existing_worktree","worktreePath":path}),
            _ => json!({"type":"worktree","baseRef":base,"startFromOrigin":self.start_from_origin}),
        };
        let original = self.editing.as_ref().expect("task editor");
        let mut payload = json!({"title":title,"prompt":prompt,"enabled":self.enabled,"schedule":schedule,"projectId":project,"threadId":original["threadId"],"threadMode":original["threadMode"].as_str().unwrap_or(if original["threadId"].is_null() {"new_thread"} else {"continue"}),"workspaceStrategy":workspace,"modelSelection":{"instanceId":instance,"model":model},"runtimeMode":original["runtimeMode"].as_str().unwrap_or("full-access"),"interactionMode":original["interactionMode"].as_str().unwrap_or("default"),"creationSource":"web"});
        let mut selection = original["modelSelection"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        selection.insert("instanceId".into(), json!(instance));
        selection.insert("model".into(), json!(model));
        payload["modelSelection"] = json!(selection);
        if let Some(id) = original["id"].as_str() {
            payload["id"] = json!(id);
            payload["requireExisting"] = json!(true);
        }
        self.saving = true;
        settings_controls::request(
            &self.root,
            &self.environment,
            "scheduledTasks.upsert",
            payload,
            cx,
            |view, result, _| {
                view.saving = false;
                match result {
                    Ok(_) => {
                        view.editing = None;
                        view.error = None;
                    }
                    Err(error) => view.error = Some(format!("Unable to save: {error:#}")),
                }
            },
        );
        cx.notify();
    }
}
impl Render for Tasks {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let schedule = self
            .schedule
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or("fixed_time");
        v_flex()
            .id("scheduled-tasks")
            .role(accesskit::Role::Region)
            .aria_label(self.environment_label.clone())
            .gap_3()
            .child(
                h_flex()
                    .px_4()
                    .justify_between()
                    .child(
                        settings_controls::text("Scheduled tasks")
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        Button::new("add-task")
                            .label("New task")
                            .icon(gpui_kit::assets::IconName::Plus)
                            .ghost()
                            .h(px(24.))
                            .text_xs()
                            .on_click(
                                cx.listener(|view, _, window, cx| view.edit(json!({}), window, cx)),
                            ),
                    ),
            )
            .when_some(self.error.clone(), |view, error| {
                view.child(
                    div()
                        .id("settings-error")
                        .role(accesskit::Role::Alert)
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
            .when(self.editing.is_some(), |view| {
                view.child(
                    v_flex()
                        .gap_2()
                        .p_3()
                        .border_1()
                        .border_color(cx.theme().border)
                        .child(Input::new(&self.inputs[0]).aria_label("Task name"))
                        .child(Input::new(&self.inputs[1]).aria_label("Prompt"))
                        .child(Select::new(&self.project).accessibility_label("Project"))
                        .child(Select::new(&self.model).accessibility_label("Model"))
                        .child(Select::new(&self.schedule).accessibility_label("Schedule"))
                        .child(Input::new(&self.inputs[2]).aria_label(match schedule {
                            "interval" => "Interval (minutes)",
                            "once" => "Run at",
                            _ => "Time",
                        }))
                        .when(schedule == "fixed_time", |view| {
                            view.child(
                                h_flex().gap_1().children(
                                    ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"]
                                        .iter()
                                        .enumerate()
                                        .map(|(index, label)| {
                                            Button::new(("weekday", index))
                                                .label(*label)
                                                .selected(self.weekdays[index])
                                                .outline()
                                                .compact()
                                                .on_click(cx.listener(move |view, _, _, cx| {
                                                    view.weekdays[index] = !view.weekdays[index];
                                                    cx.notify();
                                                }))
                                        }),
                                ),
                            )
                        })
                        .child(Select::new(&self.workspace).accessibility_label("Workspace"))
                        .child(Input::new(&self.inputs[3]).aria_label("Base branch"))
                        .child(Input::new(&self.inputs[4]).aria_label("Checkout path"))
                        .child(
                            Switch::new("start-from-origin")
                                .accessibility_label("Start from origin")
                                .label("Start from origin")
                                .checked(self.start_from_origin)
                                .on_change(cx.listener(|view, value, _, cx| {
                                    view.start_from_origin = *value;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Switch::new("task-enabled")
                                .accessibility_label("Enabled")
                                .label("Enabled")
                                .checked(self.enabled)
                                .on_change(cx.listener(|view, value, _, cx| {
                                    view.enabled = *value;
                                    cx.notify();
                                })),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("save-task")
                                        .label("Save")
                                        .disabled(self.saving)
                                        .primary()
                                        .on_click(cx.listener(|view, _, _, cx| view.save(cx))),
                                )
                                .child(
                                    Button::new("cancel-task").label("Cancel").ghost().on_click(
                                        cx.listener(|view, _, _, cx| {
                                            view.editing = None;
                                            cx.notify();
                                        }),
                                    ),
                                ),
                        ),
                )
            })
            .when(self.tasks.is_empty() && self.editing.is_none(), |view| {
                view.child(
                    v_flex()
                        .id("empty-tasks")
                        .p_4()
                        .gap_1()
                        .rounded_lg()
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().muted.opacity(0.25))
                        .child(settings_controls::text("No scheduled tasks").text_sm())
                        .child(
                            settings_controls::text(
                                "No tasks match this environment and project selection.",
                            )
                            .text_xs()
                            .text_color(cx.theme().muted_foreground),
                        ),
                )
            })
            .children(self.tasks.iter().cloned().enumerate().map(|(index, task)| {
                let edit = task.clone();
                let id = task["id"].as_str().unwrap_or_default().to_string();
                let enabled_id = id.clone();
                let run_id = id.clone();
                h_flex()
                    .id(("task", index))
                    .role(accesskit::Role::Group)
                    .aria_label(task["title"].as_str().unwrap_or_default().to_string())
                    .gap_2()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        v_flex()
                            .flex_1()
                            .child(task["title"].as_str().unwrap_or_default().to_string())
                            .child(div().text_xs().child(format!(
                                "Next: {}",
                                task["nextRunAt"].as_str().unwrap_or("—")
                            )))
                            .when_some(task["lastRunError"].as_str(), |view, error| {
                                view.child(
                                    div().text_color(cx.theme().danger).child(error.to_string()),
                                )
                            }),
                    )
                    .child(
                        Switch::new(("task-enabled", index))
                            .accessibility_label("Enabled")
                            .checked(task["enabled"] == true)
                            .on_change(cx.listener(move |view, value, _, cx| {
                                view.request(
                                    "scheduledTasks.setEnabled",
                                    json!({"id":enabled_id,"enabled":*value}),
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new(("run-task", index))
                            .label("Run now")
                            .ghost()
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.request("scheduledTasks.runNow", json!({"id":run_id}), cx)
                            })),
                    )
                    .child(
                        Button::new(("edit-task", index))
                            .label("Edit")
                            .ghost()
                            .disabled(task["schedule"]["type"] == "webhook")
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.edit(edit.clone(), window, cx)
                            })),
                    )
                    .child(
                        Button::new(("delete-task", index))
                            .label("Delete")
                            .ghost()
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.request("scheduledTasks.delete", json!({"id":id}), cx)
                            })),
                    )
            }))
    }
}
