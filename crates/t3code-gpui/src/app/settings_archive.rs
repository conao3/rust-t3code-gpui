use super::*;

pub(super) struct Archive {
    root: WeakEntity<Root>,
    environment: String,
    snapshot: Value,
    error: Option<String>,
    loading: bool,
    pending: HashSet<String>,
}
impl Archive {
    pub(super) fn new(root: WeakEntity<Root>, environment: String, cx: &mut Context<Self>) -> Self {
        let view = Self {
            root,
            environment,
            snapshot: Value::Null,
            error: None,
            loading: false,
            pending: HashSet::new(),
        };
        let this = cx.entity().downgrade();
        cx.defer(move |cx| {
            let _ = this.update(cx, |view, cx| view.refresh(cx));
        });
        view
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        settings_controls::request(
            &self.root,
            &self.environment,
            "orchestration.getArchivedShellSnapshot",
            json!({}),
            cx,
            |view, result, _| {
                view.loading = false;
                match result {
                    Ok(snapshot) => {
                        view.snapshot = snapshot;
                        view.error = None;
                    }
                    Err(error) => view.error = Some(error.to_string()),
                }
            },
        );
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
        let projects = self.snapshot["projects"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if self.loading || threads.is_empty() {
            content = content.child(div().child(if self.loading {
                "Loading archived threads"
            } else if self.error.is_some() {
                "Could not load archived threads"
            } else {
                "No archived threads"
            }));
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
            let mut group = v_flex().id(("archive-project", index)).gap_2().child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(project["title"].as_str().unwrap_or_default().to_string()),
            );
            for thread in rows {
                let Some(id) = thread["id"].as_str() else {
                    continue;
                };
                let id = id.to_string();
                let title = thread["title"].as_str().unwrap_or_default().to_string();
                let restore = id.clone();
                let remove = id.clone();
                let remove_title = title.clone();
                group = group.child(
                    h_flex()
                        .gap_3()
                        .py_3()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(
                            v_flex().flex_1().child(title.clone()).child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(format!(
                                        "Archived {} · Created {}",
                                        thread["archivedAt"].as_str().unwrap_or_default(),
                                        thread["createdAt"].as_str().unwrap_or_default()
                                    )),
                            ),
                        )
                        .child(
                            Button::new(SharedString::from(format!("unarchive-{id}")))
                                .label("Unarchive")
                                .accessibility_label(format!("Unarchive {title}"))
                                .disabled(self.pending.contains(&id))
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.perform(restore.clone(), false, cx)
                                })),
                        )
                        .child(
                            Button::new(SharedString::from(format!("delete-{id}")))
                                .label("Delete")
                                .accessibility_label(format!("Delete {title}"))
                                .disabled(self.pending.contains(&id))
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.delete(remove.clone(), remove_title.clone(), window, cx)
                                })),
                        ),
                );
            }
            content = content.child(group);
        }
        content
    }
}
