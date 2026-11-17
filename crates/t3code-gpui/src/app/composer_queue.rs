use super::*;

pub(super) struct Edit {
    target: (String, String),
    pub(super) run_id: String,
    saved_text: String,
    saved_attachments: Vec<attachments::Pending>,
    pub(super) saving: bool,
    existing_attachments: Vec<Value>,
    context: Option<Value>,
}

impl Root {
    pub(super) fn edit_latest_queued(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer.read(cx).cursor() != 0
            || !self.composer.read(cx).selected_range().is_empty()
        {
            return;
        }
        if let Some(run) = self.queued.last() {
            self.edit_queued(run.run_id.clone(), window, cx);
        }
    }
    pub(super) fn steer_next_queued(&mut self, cx: &mut Context<Self>) {
        if let Some(run) = self.queued.first() {
            self.steer_queued(run.run_id.clone(), cx);
        }
    }
    pub(super) fn edit_queued(
        &mut self,
        run_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.queue_edit.is_some()
            || self.pending_question().is_some()
            || self.composer_composing(window, cx)
        {
            return;
        }
        let Some(target) = self.current() else {
            return;
        };
        let Some(run) = self.queued.iter().find(|run| run.run_id == run_id) else {
            return;
        };
        let text = run.text.clone();
        let message = self.open.as_ref().and_then(|open| {
            let state = open.watch.state();
            let state = state.lock().expect("thread");
            let run = state.run(&run_id)?;
            state
                .list("messages")
                .iter()
                .find(|message| message["id"] == run["userMessageId"])
                .cloned()
        });
        let Some(message) = message else {
            self.message = Some("The queued message is no longer available.".into());
            cx.notify();
            return;
        };
        if !message["context"]["guest"].is_null() {
            return;
        }
        let existing_attachments = message["attachments"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let context = message
            .get("context")
            .filter(|context| !context.is_null())
            .cloned();
        self.queue_edit = Some(Edit {
            target,
            run_id,
            saved_text: self.composer.read(cx).value().to_string(),
            saved_attachments: std::mem::take(&mut self.attachments),
            saving: false,
            existing_attachments,
            context,
        });
        self.composer.update(cx, |state, cx| {
            state.set_value(text, window, cx);
            state.focus(window, cx);
        });
        self.completion = None;
        cx.notify();
    }
    pub(super) fn abandon_queue_edit(&mut self) {
        if let Some(edit) = self.queue_edit.take() {
            self.attachments = edit.saved_attachments;
        }
    }
    pub(super) fn cancel_queue_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.queue_edit.take() else {
            return;
        };
        if self.current() == Some(edit.target) {
            self.composer.update(cx, |state, cx| {
                state.set_value(edit.saved_text, window, cx);
                state.focus(window, cx);
            });
            self.attachments = edit.saved_attachments;
            self.stash_composer(cx);
        }
        cx.notify();
    }
    pub(super) fn save_queue_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer_composing(window, cx) {
            return;
        }
        let Some(edit) = self.queue_edit.as_ref() else {
            return;
        };
        if edit.saving {
            return;
        }
        let target = edit.target.clone();
        let run_id = edit.run_id.clone();
        let text = self.composer.read(cx).value().trim().to_string();
        if text.is_empty() && edit.existing_attachments.is_empty() && self.attachments.is_empty() {
            return;
        }
        let mut attachments = edit.existing_attachments.clone();
        let context = edit.context.clone();
        let pending = self.attachments.clone();
        let Some(connection) = self.connection(&target.0) else {
            return;
        };
        self.queue_edit.as_mut().expect("edit").saving = true;
        let runtime = self.runtime.clone();
        let editing = (target.clone(), run_id.clone());
        cx.spawn_in(window, async move |this,cx| {
            let result = on_runtime(&runtime, async move {
                attachments.extend(super::composer::upload_all(&connection,&pending).await?);
                let mut command = json!({"type":"queued-run.edit","threadId":target.1,"runId":run_id,"text":text,"attachments":attachments});
                if let Some(context) = context { command["context"] = context; }
                connection.dispatch(command).await
            }).await;
            let _ = this.update_in(cx, |root,window,cx| {
                if root.queue_edit.as_ref().is_none_or(|edit| edit.target != editing.0 || edit.run_id != editing.1) {
                    if let Err(error) = result { root.message = Some(format!("Could not save queued message: {error:#}")); cx.notify(); }
                    return;
                }
                match result {
                    Ok(_) => root.cancel_queue_edit(window,cx),
                    Err(error) => {
                        if let Some(edit) = root.queue_edit.as_mut() { edit.saving = false; }
                        root.message = Some(format!("Could not save queued message: {error:#}"));
                    }
                }
                cx.notify();
            });
        }).detach();
    }
    pub(super) fn render_queue_attachments(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let edit = self.queue_edit.as_ref()?;
        Some(
            h_flex()
                .px_3()
                .gap_1()
                .flex_wrap()
                .children(edit.existing_attachments.iter().enumerate().map(
                    |(index, attachment)| {
                        let name = attachment["name"]
                            .as_str()
                            .unwrap_or("Attachment")
                            .to_string();
                        Button::new(("queued-attachment", index))
                            .label(name.clone())
                            .accessibility_label(format!("Remove {name}"))
                            .ghost()
                            .compact()
                            .on_click(cx.listener(move |root, _, _, cx| {
                                if let Some(edit) = root.queue_edit.as_mut() {
                                    edit.existing_attachments.remove(index);
                                }
                                cx.notify();
                            }))
                    },
                ))
                .into_any_element(),
        )
    }

    pub(super) fn move_queued(&mut self, index: usize, forward: bool, cx: &mut Context<Self>) {
        let Some(run) = self.queued.get(index) else {
            return;
        };
        let run_id = run.run_id.clone();
        let before = if forward {
            self.queued.get(index + 2)
        } else {
            index.checked_sub(1).and_then(|i| self.queued.get(i))
        }
        .map(|r| r.run_id.clone());
        self.dispatch_on_open("Reordering queued message",cx,move |connection,thread_id| Box::pin(async move {
            connection.dispatch(json!({"type":"queued-run.reorder","threadId":thread_id,"runId":run_id,"beforeRunId":before})).await
        }));
    }
    pub(super) fn render_force_stop(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let open = self.open.as_ref()?;
        if !self
            .config_of(&open.environment)
            .is_some_and(|config| config["environment"]["capabilities"]["threadForceStop"] == true)
        {
            return None;
        }
        let state = open.watch.state();
        let state = state.lock().expect("thread");
        let active = state.active_run()?;
        let item = state.items().into_iter().rev().find(|item| {
            item["type"] == "run_interrupt_request"
                && !item["unresponsiveAt"].is_null()
                && item["runId"] == active["id"]
        })?;
        let run_id = item["runId"].as_str()?.to_string();
        Some(
            h_flex()
                .px_3()
                .pt_2()
                .gap_2()
                .child("Agent not responding to Stop")
                .child(
                    Button::new("force-stop")
                        .label("Force stop")
                        .ghost()
                        .compact()
                        .on_click(
                            cx.listener(move |root, _, _, cx| root.force_stop(run_id.clone(), cx)),
                        ),
                )
                .into_any_element(),
        )
    }
    pub(super) fn force_stop(&mut self, run_id: String, cx: &mut Context<Self>) {
        let Some((environment, thread_id)) = self.current() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                connection
                    .request(
                        "orchestration.forceStopThread",
                        json!({"threadId":thread_id,"runId":run_id}),
                    )
                    .await
            })
            .await;
            let _ = this.update(cx, |root, cx| {
                root.message = match result {
                    Ok(value) if value["outcome"] == "releasing" => Some("Turn stopped; the agent is still shutting down".into()),
                    Ok(value) if value["outcome"] == "shared" => Some("Turn stopped; the agent process kept running because other working threads share it.".into()),
                    Ok(_) => None,
                    Err(error) => Some(format!("Force stop failed: {error:#}")),
                };
                cx.notify();
            });
        })
        .detach();
    }
}
