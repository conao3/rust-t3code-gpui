use super::*;

pub(super) struct Archive {
    root: WeakEntity<Root>,
    environment: String,
    snapshot: Value,
    error: Option<String>,
    loading: bool,
    pending: HashSet<String>,
    sequence: u64,
    refresh_pending: bool,
    clock_task: Option<Task<()>>,
    subscription: Option<Subscription>,
}
impl Archive {
    pub(super) fn new(root: WeakEntity<Root>, environment: String, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            root: root.clone(),
            environment,
            snapshot: Value::Null,
            error: None,
            loading: false,
            pending: HashSet::new(),
            sequence: 0,
            refresh_pending: false,
            clock_task: None,
            subscription: None,
        };
        if let Some(owner) = root.upgrade() {
            view.subscription = Some(cx.observe(&owner, |view, owner, cx| {
                let sequence = owner
                    .read(cx)
                    .connection(&view.environment)
                    .map(|connection| connection.shell().lock().expect("shell").sequence)
                    .unwrap_or(0);
                if sequence != view.sequence {
                    view.sequence = sequence;
                    view.refresh(cx);
                }
            }));
        }
        let this = cx.entity().downgrade();
        cx.defer(move |cx| {
            let _ = this.update(cx, |view, cx| view.refresh(cx));
        });
        view
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            self.refresh_pending = true;
            return;
        }
        self.loading = true;
        settings_controls::request(
            &self.root,
            &self.environment,
            "orchestration.getArchivedShellSnapshot",
            json!({}),
            cx,
            |view, result, cx| {
                view.loading = false;
                match result {
                    Ok(snapshot) => {
                        view.snapshot = snapshot;
                        view.error = None;
                        view.schedule_clock(cx);
                    }
                    Err(error) => view.error = Some(error.to_string()),
                }
                if std::mem::take(&mut view.refresh_pending) {
                    view.refresh(cx);
                }
            },
        );
    }
    fn schedule_clock(&mut self, cx: &mut Context<Self>) {
        let now = SystemTime::now();
        let delay = self.snapshot["threads"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|thread| [&thread["archivedAt"], &thread["createdAt"]])
            .filter_map(Value::as_str)
            .filter_map(|timestamp| humantime::parse_rfc3339(timestamp).ok())
            .map(|timestamp| {
                let seconds = now.duration_since(timestamp).unwrap_or_default().as_secs();
                let unit = if seconds < 3600 {
                    60
                } else if seconds < 86400 {
                    3600
                } else {
                    86400
                };
                Duration::from_secs(unit - seconds % unit)
            })
            .min();
        self.clock_task = delay.map(|delay| {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(delay).await;
                let _ = this.update(cx, |view, cx| {
                    view.schedule_clock(cx);
                    cx.notify();
                });
            })
        });
    }
    fn perform(&mut self, id: String, delete: bool, cx: &mut Context<Self>) {
        if !self.pending.insert(id.clone()) {
            return;
        }
        settings_controls::request(
            &self.root,
            &self.environment,
            "orchestration.dispatchCommand",
            json!({"type":if delete {"thread.delete"} else {"thread.unarchive"},"commandId":uuid::Uuid::new_v4().to_string(),"threadId":id}),
            cx,
            move |view, result, cx| {
                view.pending.remove(&id);
                match result {
                    Ok(_) => view.refresh(cx),
                    Err(error) => view.error = Some(error.to_string()),
                }
            },
        );
    }
    fn delete(&mut self, id: String, title: String, window: &mut Window, cx: &mut Context<Self>) {
        let confirm = self
            .root
            .upgrade()
            .is_some_and(|root| root.read(cx).settings.values["confirmThreadDelete"] != false);
        if !confirm {
            self.perform(id, true, cx);
            return;
        }
        let owner = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let owner = owner.clone();
            let id = id.clone();
            dialog
                .title(format!("Delete thread \"{title}\"?"))
                .centered()
                .child("This permanently clears conversation history for this thread.")
                .confirm()
                .on_ok(move |_, _, cx| {
                    let _ = owner.update(cx, |view, cx| view.perform(id.clone(), true, cx));
                    true
                })
        });
    }
}
impl Render for Archive {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let mut content = v_flex().gap_3();
        if let Some(error) = &self.error {
            content = content.child(
                div()
                    .id("archive-error")
                    .role(accesskit::Role::Alert)
                    .text_color(theme.danger)
                    .child(error.clone()),
            );
        }
        let threads = self.snapshot["threads"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let selected_project = self
            .root
            .upgrade()
            .and_then(|root| root.read(cx).settings_project.clone());
        let threads = threads
            .into_iter()
            .filter(|thread| {
                selected_project
                    .as_ref()
                    .is_none_or(|project| thread["projectId"] == *project)
            })
            .collect::<Vec<_>>();
        let projects = self.snapshot["projects"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if (self.loading && self.snapshot.is_null()) || threads.is_empty() {
            content = content
                .child(
                    settings_controls::text("Archived threads").font_weight(FontWeight::SEMIBOLD),
                )
                .child(settings_controls::text(if self.loading {
                    "Loading archived threads"
                } else if self.error.is_some() {
                    "Could not load archived threads"
                } else {
                    "No archived threads"
                }))
                .child(
                    settings_controls::text(if self.loading {
                        "Checking connected environments."
                    } else if let Some(error) = &self.error {
                        error
                    } else {
                        "Archived threads will appear here."
                    })
                    .text_sm()
                    .text_color(theme.muted_foreground),
                );
        }
        for (index, project) in projects.iter().enumerate() {
            let mut rows = threads
                .iter()
                .filter(|thread| thread["projectId"] == project["id"])
                .collect::<Vec<_>>();
            rows.sort_by(|a, b| {
                b["archivedAt"]
                    .as_str()
                    .or(b["createdAt"].as_str())
                    .cmp(&a["archivedAt"].as_str().or(a["createdAt"].as_str()))
                    .then_with(|| b["id"].as_str().cmp(&a["id"].as_str()))
            });
            if rows.is_empty() {
                continue;
            }
            let group = v_flex().id(("archive-project", index)).gap_3().child(
                settings_controls::text(project["title"].as_str().unwrap_or_default().to_string())
                    .px_4()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD),
            );
            let mut items = v_flex()
                .rounded(px(12.))
                .overflow_hidden()
                .border_1()
                .border_color(theme.border)
                .bg(theme.muted.opacity(0.25));
            for thread in rows {
                let Some(id) = thread["id"].as_str() else {
                    continue;
                };
                let id = id.to_string();
                let title = thread["title"].as_str().unwrap_or_default().to_string();
                let restore = id.clone();
                let remove = id.clone();
                let remove_title = title.clone();
                items = items.child(
                    h_flex()
                        .id(SharedString::from(format!("archive-thread-{id}")))
                        .gap_3()
                        .px_4()
                        .py_3()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(
                            v_flex()
                                .flex_1()
                                .child(settings_controls::text(title.clone()).text_sm())
                                .child(
                                    settings_controls::text(format!(
                                        "Archived {} · Created {}",
                                        relative_time(
                                            thread["archivedAt"]
                                                .as_str()
                                                .or(thread["createdAt"].as_str())
                                                .unwrap_or_default(),
                                            SystemTime::now()
                                        ),
                                        relative_time(
                                            thread["createdAt"].as_str().unwrap_or_default(),
                                            SystemTime::now()
                                        )
                                    ))
                                    .text_xs()
                                    .text_color(theme.muted_foreground),
                                ),
                        )
                        .child(
                            Button::new(SharedString::from(format!("unarchive-{id}")))
                                .icon(gpui_kit::assets::IconName::ArchiveX)
                                .outline()
                                .compact()
                                .accessibility_label(format!("Unarchive {title}"))
                                .disabled(self.pending.contains(&id))
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.perform(restore.clone(), false, cx)
                                })),
                        )
                        .child(
                            Button::new(SharedString::from(format!("delete-{id}")))
                                .icon(gpui_kit::assets::IconName::Trash)
                                .danger()
                                .outline()
                                .compact()
                                .accessibility_label(format!("Delete {title}"))
                                .disabled(self.pending.contains(&id))
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.delete(remove.clone(), remove_title.clone(), window, cx)
                                })),
                        ),
                );
            }
            content = content.child(group.child(items));
        }
        content
    }
}

fn relative_time(timestamp: &str, now: SystemTime) -> String {
    let Ok(timestamp) = humantime::parse_rfc3339(timestamp) else {
        return String::new();
    };
    let seconds = now.duration_since(timestamp).unwrap_or_default().as_secs();
    match seconds {
        0..60 => "just now".to_string(),
        60..3600 => format!("{}m ago", seconds / 60),
        3600..86400 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86400),
    }
}
