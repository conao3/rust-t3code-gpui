use super::*;

#[derive(PartialEq)]
pub(super) struct QueueStamp {
    queued: Vec<timeline::Queued>,
    editing: Option<String>,
    expanded: bool,
    light: bool,
    font_size: f32,
    timestamp_format: crate::timestamp::Format,
}

pub(super) struct QueuePane {
    pub(super) root: WeakEntity<Root>,
    pub(super) pane_id: u64,
    pub(super) stamp: Option<QueueStamp>,
}

impl Render for QueuePane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.root
            .update(cx, |root, cx| {
                root.with_conversation(self.pane_id, |root| {
                    div().w_full().children(root.render_queue(cx))
                })
            })
            .expect("queue root is alive")
            .expect("queue pane exists")
    }
}

pub(super) struct Edit {
    target: (String, String),
    pub(super) run_id: String,
    saved_text: String,
    saved_attachments: Vec<attachments::Pending>,
    pub(super) saving: bool,
    existing_attachments: Vec<Value>,
    context: Option<Value>,
}

fn resume_target(state: &t3_client::projection::ThreadState) -> Option<(Option<String>, bool)> {
    let held = state
        .list("runs")
        .iter()
        .any(|run| run["status"] == "queued" && run["queueHeld"] == true);
    let latest = state
        .list("runs")
        .iter()
        .filter(|run| {
            run["status"] != "queued"
                && !(run["status"] == "cancelled" && run["startedAt"].is_null())
        })
        .max_by_key(|run| {
            (
                run["completedAt"].is_null(),
                run["completedAt"].as_str().unwrap_or_default(),
                run["ordinal"].as_u64().unwrap_or(0),
            )
        });
    let session_error = state
        .list("providerSessions")
        .iter()
        .rev()
        .find(|session| session["providerInstanceId"] == state.thread()["providerInstanceId"])
        .and_then(|session| session["lastError"].as_str());
    let run = latest
        .filter(|run| {
            run["status"] == "interrupted"
                || (run["status"] == "failed"
                    && state
                        .list("turnItems")
                        .iter()
                        .filter(|item| {
                            item["type"] == "error"
                                && item["status"] == "failed"
                                && item["runId"] == run["id"]
                                && item["nodeId"] == run["rootNodeId"]
                        })
                        .max_by_key(|item| {
                            (
                                item["updatedAt"].as_str().unwrap_or_default(),
                                item["ordinal"].as_u64().unwrap_or(0),
                                item["id"].as_str().unwrap_or_default(),
                            )
                        })
                        .is_some_and(|item| {
                            item["failure"]["class"] == "usage_limit"
                                && session_error
                                    .is_none_or(|error| item["failure"]["message"] == error)
                        }))
        })
        .and_then(|run| run["id"].as_str())
        .map(str::to_owned);
    (run.is_some() || held).then_some((run, held))
}

impl Root {
    pub(super) fn sync_queue_pane(&self, cx: &mut Context<Self>) {
        let stamp = QueueStamp {
            queued: self.conversation.queued.clone(),
            editing: self
                .conversation
                .queue_edit
                .as_ref()
                .map(|edit| edit.run_id.clone()),
            expanded: self.conversation.queue_expanded,
            light: self.settings.light_theme,
            font_size: self.settings.font_size_interface,
            timestamp_format: self.settings.timestamp_format,
        };
        let queue = self.conversation.timeline_pane.read(cx).queue.clone();
        let changed = queue.update(cx, |pane, cx| {
            if pane.stamp.as_ref() == Some(&stamp) {
                return false;
            }
            pane.stamp = Some(stamp);
            cx.notify();
            true
        });
        if changed && !self.conversation.queued.is_empty() {
            let index = self.conversation.rows.len();
            self.conversation.list_state.splice(index..index + 1, 1);
            self.conversation
                .timeline_pane
                .update(cx, |_, cx| cx.notify());
        }
    }

    pub(super) fn can_resume_composer(&self) -> bool {
        self.conversation.open.as_ref().is_some_and(|open| {
            resume_target(&open.watch.state().lock().expect("thread")).is_some()
        })
    }

    pub(super) fn resume_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.conversation.open.as_ref() else {
            return;
        };
        let Some((run_id, held)) = resume_target(&open.watch.state().lock().expect("thread"))
        else {
            return;
        };
        let Some((environment, model, mode, _)) = self.composer_target() else {
            return;
        };
        let thread = open.watch.thread_id().to_string();
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let Some(key) = self.composer_key() else {
            return;
        };
        if !self.conversation.composer_sends.insert(key.clone()) {
            return;
        }
        let runtime = self.runtime.clone();
        self.spawn_pane_in(window, cx, async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                if let Some(run) = run_id {
                    connection.set_runtime_mode(&thread, &mode).await?;
                    connection.set_interaction_mode(&thread, "default").await?;
                    let mut command = json!({"type":"message.dispatch", "threadId":thread, "messageId":uuid::Uuid::new_v4().to_string(), "createdBy":"user", "creationSource":"web", "manualContinuationOfRunId":run, "text":"Continue where you left off.", "attachments":[], "dispatchMode":{"type":"start_immediately"}});
                    if let Some(model) = model { command["modelSelection"] = model; }
                    connection.dispatch(command).await?;
                }
                if held { connection.dispatch(json!({"type":"queue.resume", "threadId":thread})).await?; }
                Ok::<_, anyhow::Error>(())
            }).await;
            let _ = this.update_in(cx, |root, _, cx| {
                root.conversation.composer_sends.remove(&key);
                root.conversation.message = result.err().map(|error| format!("Could not resume thread: {error:#}"));
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub(super) fn edit_latest_queued(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.conversation.composer.read(cx).cursor() != 0
            || !self
                .conversation
                .composer
                .read(cx)
                .selected_range()
                .is_empty()
        {
            return;
        }
        if self.conversation.queue_edit.is_none()
            && let Some(run) = self.conversation.queued.last()
        {
            self.edit_queued(run.run_id.clone(), window, cx);
            if self.conversation.queue_edit.is_some() {
                self.conversation.queue_expanded = true;
            }
        }
    }
    pub(super) fn steer_next_queued(&mut self, cx: &mut Context<Self>) {
        if let Some(run) = self.conversation.queued.first() {
            self.steer_queued(run.run_id.clone(), cx);
        }
    }
    pub(super) fn edit_queued(
        &mut self,
        run_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.conversation.queue_edit.is_some()
            || self.pending_question().is_some()
            || self.composer_composing(window, cx)
        {
            return;
        }
        let Some(target) = self.current() else {
            return;
        };
        let Some(run) = self
            .conversation
            .queued
            .iter()
            .find(|run| run.run_id == run_id)
        else {
            return;
        };
        let text = run.text.clone();
        let message = self.conversation.open.as_ref().and_then(|open| {
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
            self.conversation.message = Some("The queued message is no longer available.".into());
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
        self.stash_composer(cx);
        self.conversation.queue_edit = Some(Edit {
            target,
            run_id,
            saved_text: self.conversation.composer.read(cx).value().to_string(),
            saved_attachments: std::mem::take(&mut self.conversation.attachments),
            saving: false,
            existing_attachments,
            context,
        });
        self.conversation.composer.update(cx, |state, cx| {
            state.set_value(super::composer_tokens::content(text), window, cx);
            state.focus(window, cx);
        });
        self.conversation.completion = None;
        cx.notify();
    }
    pub(super) fn abandon_queue_edit(&mut self) {
        if let Some(edit) = self.conversation.queue_edit.take() {
            self.conversation.attachments = edit.saved_attachments;
        }
    }
    pub(super) fn cancel_queue_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.conversation.queue_edit.take() else {
            return;
        };
        if self.current() == Some(edit.target) {
            self.conversation.composer.update(cx, |state, cx| {
                state.set_value(edit.saved_text, window, cx);
                state.focus(window, cx);
            });
            self.conversation.attachments = edit.saved_attachments;
            self.stash_composer(cx);
        }
        cx.notify();
    }
    pub(super) fn save_queue_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer_composing(window, cx) {
            return;
        }
        let Some(edit) = self.conversation.queue_edit.as_ref() else {
            return;
        };
        if edit.saving {
            return;
        }
        let target = edit.target.clone();
        let run_id = edit.run_id.clone();
        let text = self
            .conversation
            .composer
            .read(cx)
            .value()
            .trim()
            .to_string();
        if text.is_empty()
            && edit.existing_attachments.is_empty()
            && self.conversation.attachments.is_empty()
        {
            return;
        }
        let mut attachments = edit.existing_attachments.clone();
        let context = edit.context.clone();
        let pending = self.conversation.attachments.clone();
        let Some(connection) = self.connection(&target.0) else {
            return;
        };
        self.conversation.queue_edit.as_mut().expect("edit").saving = true;
        let runtime = self.runtime.clone();
        let editing = (target.clone(), run_id.clone());
        self.spawn_pane_in(window, cx, async move |this,cx| {
            let result = on_runtime(&runtime, async move {
                attachments.extend(super::composer::upload_all(&connection,&pending).await?);
                let mut command = json!({"type":"queued-run.edit","threadId":target.1,"runId":run_id,"text":text,"attachments":attachments});
                if let Some(context) = context { command["context"] = context; }
                connection.dispatch(command).await
            }).await;
            let _ = this.update_in(cx, |root,window,cx| {
                if root.conversation.queue_edit.as_ref().is_none_or(|edit| edit.target != editing.0 || edit.run_id != editing.1) {
                    if let Err(error) = result { root.conversation.message = Some(format!("Could not save queued message: {error:#}")); cx.notify(); }
                    return;
                }
                match result {
                    Ok(_) => root.cancel_queue_edit(window,cx),
                    Err(error) => {
                        if let Some(edit) = root.conversation.queue_edit.as_mut() { edit.saving = false; }
                        root.conversation.message = Some(format!("Could not save queued message: {error:#}"));
                    }
                }
                cx.notify();
            });
        }).detach();
    }
    pub(super) fn render_queue_attachments(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let edit = self.conversation.queue_edit.as_ref()?;
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
                            .on_click(self.pane_listener(cx, move |root, _, _, cx| {
                                if let Some(edit) = root.conversation.queue_edit.as_mut() {
                                    edit.existing_attachments.remove(index);
                                }
                                cx.notify();
                            }))
                    },
                ))
                .into_any_element(),
        )
    }

    pub(super) fn render_force_stop(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let open = self.conversation.open.as_ref()?;
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
                        .on_click(self.pane_listener(cx, move |root, _, _, cx| {
                            root.force_stop(run_id.clone(), cx)
                        })),
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
        self.spawn_pane(cx,async move |this, cx| {
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
                root.conversation.message = match result {
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

#[cfg(test)]
mod resume_tests {
    use super::resume_target;
    use serde_json::{Value, json};

    #[test]
    fn resume_follows_execution_order_and_held_queue() {
        let mut state = t3_client::projection::ThreadState::default();
        state.projection.insert("runs".into(), json!([
            {"id":"stopped", "ordinal":1, "status":"interrupted", "completedAt":"2026-11-18T10:00:00Z"},
            {"id":"held", "ordinal":2, "status":"queued", "queueHeld":true},
            {"id":"cancelled", "ordinal":3, "status":"cancelled", "startedAt":null}
        ]));
        assert_eq!(resume_target(&state), Some((Some("stopped".into()), true)));
        state.projection.get_mut("runs").unwrap()[0]["status"] = json!("completed");
        assert_eq!(resume_target(&state), Some((None, true)));
        state.projection.get_mut("runs").unwrap()[1]["queueHeld"] = json!(false);
        assert_eq!(resume_target(&state), None);
        state.projection.get_mut("runs").unwrap()[0]["status"] = json!("failed");
        state.projection.get_mut("runs").unwrap()[0]["rootNodeId"] = json!("root");
        state.projection.insert("turnItems".into(), json!([
            {"id":"error", "ordinal":1, "type":"error", "status":"failed", "runId":"stopped", "nodeId":"root", "failure":{"class":"usage_limit"}}
        ]));
        assert_eq!(resume_target(&state), Some((Some("stopped".into()), false)));
        state
            .projection
            .insert("thread".into(), json!({"providerInstanceId":"codex"}));
        state.projection.get_mut("turnItems").unwrap()[0]["failure"]["message"] =
            json!("Usage limit reached");
        state.projection.insert(
            "providerSessions".into(),
            json!([
                {"providerInstanceId":"codex", "lastError":"Usage limit reached"},
                {"providerInstanceId":"other", "lastError":"Disconnected"}
            ]),
        );
        assert_eq!(resume_target(&state), Some((Some("stopped".into()), false)));
        state.projection.get_mut("providerSessions").unwrap()[0]["lastError"] =
            json!("Disconnected");
        assert_eq!(resume_target(&state), None);
        state.projection.get_mut("providerSessions").unwrap()[0]["lastError"] = Value::Null;
        state.projection.get_mut("turnItems").unwrap()[0]["nodeId"] = json!("child");
        assert_eq!(resume_target(&state), None);
    }
}
