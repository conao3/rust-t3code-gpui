use super::*;

pub(super) struct TimelinePane {
    pub(super) views: Vec<Entity<RowView>>,
    pub(super) list: ListState,
}

impl Render for TimelinePane {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let views = self.views.clone();
        div()
            .relative()
            .flex_1()
            .w_full()
            .h_full()
            .child(
                list(self.list.clone(), move |ix, _, _| {
                    views
                        .get(ix)
                        .map(|view| view.clone().into_any_element())
                        .unwrap_or_else(|| div().into_any_element())
                })
                .w_full()
                .h_full()
                .pt_4(),
            )
            .child(
                gpui_kit::base::Scrollbar::vertical(&self.list)
                    .mode(gpui_kit::base::ScrollbarMode::Always),
            )
    }
}

impl Root {
    pub(super) fn rebuild_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &self.open else { return };
        let (rows, queued, loading) = {
            let state = open.watch.state();
            let state = state.lock().expect("thread lock");
            (
                timeline::rows(&state),
                timeline::queued(&state),
                state.projection.is_empty(),
            )
        };
        self.queued = queued;
        self.thread_loading = loading;
        let mut existing: HashMap<String, Entity<RowView>> = self
            .rows
            .iter()
            .map(|r| r.key.clone())
            .zip(self.row_views.iter().cloned())
            .collect();
        let old_keys: Vec<String> = self.rows.iter().map(|r| r.key.clone()).collect();
        let mut subscriptions = std::mem::take(&mut self.row_subscriptions);
        let mut kept = HashMap::new();
        let views: Vec<Entity<RowView>> = rows
            .iter()
            .map(|row| match existing.remove(&row.key) {
                Some(view) => {
                    view.update(cx, |view, cx| view.update(row.clone(), cx));
                    if let Some(subscription) = subscriptions.remove(&row.key) {
                        kept.insert(row.key.clone(), subscription);
                    }
                    view
                }
                None => {
                    let view = {
                        let row = row.clone();
                        cx.new(|cx| RowView::new(row, cx))
                    };
                    if matches!(
                        row.kind,
                        RowKind::Request
                            | RowKind::Plan
                            | RowKind::Subagent
                            | RowKind::Assistant
                            | RowKind::Fork
                    ) {
                        kept.insert(
                            row.key.clone(),
                            cx.subscribe_in(&view, window, Self::on_row_event),
                        );
                    }
                    view
                }
            })
            .collect();
        let appended = rows.len() >= old_keys.len()
            && old_keys.iter().zip(&rows).all(|(old, new)| *old == new.key);
        if appended {
            let old = old_keys.len();
            if rows.len() > old {
                self.user_cursor = None;
                self.list_state.splice(old..old, rows.len() - old);
            }
        } else {
            let prefix = old_keys
                .iter()
                .zip(&rows)
                .take_while(|(old, new)| **old == new.key)
                .count();
            let suffix = old_keys[prefix..]
                .iter()
                .rev()
                .zip(rows[prefix..].iter().rev())
                .take_while(|(old, new)| **old == new.key)
                .count();
            self.list_state.splice(
                prefix..old_keys.len() - suffix,
                rows.len() - prefix - suffix,
            );
        }
        self.rows = rows;
        self.row_views = views;
        for (row, view) in self.rows.iter().zip(&self.row_views) {
            if let Some(metadata) = row
                .message_part
                .as_ref()
                .and_then(|part| part.metadata.as_ref())
            {
                view.update(cx, |view, cx| {
                    view.set_message_hovered(
                        self.hovered_message.as_ref() == Some(&metadata.message_id),
                        cx,
                    )
                });
            }
        }
        self.timeline_pane.update(cx, |pane, cx| {
            pane.views = self.row_views.clone();
            cx.notify();
        });
        self.row_subscriptions = kept;
        self.find_again(false, cx);
        cx.notify();
    }

    fn on_row_event(
        &mut self,
        view: &Entity<RowView>,
        event: &RowEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            RowEvent::MessageHover {
                message_id,
                hovered,
            } => {
                let old = self.hovered_message.clone();
                if *hovered {
                    self.hovered_message = Some(message_id.clone());
                } else if self.hovered_message.as_ref() == Some(message_id) {
                    self.hovered_message = None;
                }
                if old != self.hovered_message {
                    for (row, view) in self.rows.iter().zip(&self.row_views) {
                        if let Some(metadata) = row
                            .message_part
                            .as_ref()
                            .and_then(|part| part.metadata.as_ref())
                            && (Some(&metadata.message_id) == old.as_ref()
                                || Some(&metadata.message_id) == self.hovered_message.as_ref())
                        {
                            view.update(cx, |view, cx| {
                                view.set_message_hovered(
                                    Some(&metadata.message_id) == self.hovered_message.as_ref(),
                                    cx,
                                )
                            });
                        }
                    }
                }
            }
            RowEvent::CopyMessage { message_id } => {
                let text = self.open.as_ref().and_then(|open| {
                    let state = open.watch.state();
                    let state = state.lock().expect("thread lock");
                    state
                        .items()
                        .into_iter()
                        .find(|item| item["id"].as_str() == Some(message_id))
                        .and_then(|item| item["text"].as_str())
                        .map(str::to_string)
                });
                match text
                    .context("message is not in the thread projection")
                    .and_then(|text| crate::markdown_directives::for_copy(&text))
                {
                    Ok(text) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        view.update(cx, |view, cx| view.copied(cx));
                    }
                    Err(error) => {
                        self.message = Some(format!("Failed to copy message: {error:#}"));
                        cx.notify();
                    }
                }
            }
            RowEvent::ForkResponse {
                source_thread_id,
                run_id,
            } => {
                let Some((environment, _)) = self.current() else {
                    return;
                };
                let Some(connection) = self.connection(&environment) else {
                    return;
                };
                let source = source_thread_id.clone();
                let run = run_id.clone();
                let target = uuid::Uuid::new_v4().to_string();
                let title = format!(
                    "{} fork",
                    self.open_entry()
                        .map(|entry| entry.title.as_str())
                        .unwrap_or("Thread")
                );
                let runtime = self.runtime.clone();
                let view = view.clone();
                cx.spawn(async move |this, cx| {
                    let command = json!({
                        "type": "thread.fork",
                        "createdBy": "user",
                        "creationSource": "web",
                        "sourceThreadId": source,
                        "targetThreadId": target,
                        "sourcePoint": {"type": "run", "runId": run},
                        "title": title,
                    });
                    let result =
                        on_runtime(&runtime, async move { connection.dispatch(command).await })
                            .await;
                    let _ = this.update(cx, |root, cx| {
                        view.update(cx, |view, cx| view.set_forking(false, cx));
                        match result {
                            Ok(_) => root.open_thread(environment, target, cx),
                            Err(error) => {
                                root.message =
                                    Some(format!("Failed to fork this response: {error:#}"));
                                cx.notify();
                            }
                        }
                    });
                })
                .detach();
            }
            RowEvent::Respond {
                request_id,
                decision,
            } => self.respond(view.clone(), request_id.clone(), decision.clone(), cx),
            RowEvent::Submit { request_id } => {
                self.submit_answers(view.clone(), request_id.clone(), window, cx)
            }
            RowEvent::OpenChild { thread_id } => {
                if let Some((environment, _)) = self.current() {
                    self.open_thread(environment, thread_id.clone(), cx);
                }
            }
            RowEvent::Implement { plan_id, markdown } => {
                let (plan_id, markdown) = (plan_id.clone(), markdown.clone());
                let view = view.clone();
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
                            .implement_plan(&thread_id, &plan_id, &markdown)
                            .await
                    })
                    .await;
                    if let Err(error) = result {
                        view.update(cx, |view, cx| view.set_responding(false, cx));
                        let _ = this.update(cx, |root, cx| {
                            root.message = Some(format!("Implementing the plan failed: {error:#}"));
                            cx.notify();
                        });
                    }
                })
                .detach();
            }
        }
    }

    fn respond(
        &mut self,
        view: Entity<RowView>,
        request_id: String,
        decision: String,
        cx: &mut Context<Self>,
    ) {
        let Some((environment, thread_id)) = self.current() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                connection.respond(&thread_id, &request_id, &decision).await
            })
            .await;
            if let Err(error) = result {
                view.update(cx, |view, cx| view.set_responding(false, cx));
                let _ = this.update(cx, |root, cx| {
                    root.message = Some(format!("Reply failed: {error:#}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn pending_question(&self) -> Option<(Entity<RowView>, String)> {
        self.rows
            .iter()
            .zip(&self.row_views)
            .rev()
            .find_map(|(row, view)| match &row.pending {
                Some(Pending::Question { request_id, .. }) => {
                    Some((view.clone(), request_id.clone()))
                }
                _ => None,
            })
    }

    pub(super) fn submit_answers(
        &mut self,
        view: Entity<RowView>,
        request_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((environment, thread_id)) = self.current() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let questions = self.rows.iter().find_map(|row| match &row.pending {
            Some(Pending::Question {
                request_id: id,
                questions,
            }) if *id == request_id => Some(questions.clone()),
            _ => None,
        });
        let Some(questions) = questions else { return };
        let typed = self.composer.read(cx).value().to_string();
        let Some(answers) = timeline::answers_with_text(&questions, view.read(cx).drafts(), &typed)
        else {
            self.message = Some("Answer every question first.".into());
            cx.notify();
            return;
        };
        self.message = None;
        view.update(cx, |view, cx| view.set_responding(true, cx));
        self.composer
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.stash_composer(cx);
        let runtime = self.runtime.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                connection.answer(&thread_id, &request_id, answers).await
            })
            .await;
            if let Err(error) = result {
                view.update(cx, |view, cx| view.set_responding(false, cx));
                let _ = this.update_in(cx, |root, window, cx| {
                    root.message = Some(format!("Answer failed: {error:#}"));
                    root.composer
                        .update(cx, |state, cx| state.set_value(typed, window, cx));
                    root.stash_composer(cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn jump_user_message(&mut self, forward: bool, cx: &mut Context<Self>) {
        let top = self.list_state.logical_scroll_top();
        let (from, inside) = if top.item_ix < self.rows.len() {
            (top.item_ix, top.offset_in_item > px(0.))
        } else {
            (self.user_cursor.unwrap_or(self.rows.len()), false)
        };
        let target = timeline::user_message_jump(&self.rows, from, inside, forward);
        self.user_cursor = target;
        match target {
            Some(item_ix) => self.list_state.scroll_to(ListOffset {
                item_ix,
                offset_in_item: px(0.),
            }),
            None if forward => self.list_state.set_follow_mode(FollowMode::Tail),
            None => self.list_state.scroll_to(ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            }),
        }
        cx.notify();
    }

    pub(super) fn render_thread(&self, cx: &mut Context<Self>) -> AnyElement {
        let message = self.render_message(cx);
        let theme = cx.theme();
        let Some(_) = &self.open else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.muted_foreground)
                .child("Select a thread")
                .into_any_element();
        };
        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .when(self.find_open, |pane| {
                let count = if self.find_hits.is_empty() {
                    "No results".to_string()
                } else {
                    format!("{} / {}", self.find_at + 1, self.find_hits.len())
                };
                pane.child(
                    h_flex()
                        .px_4()
                        .py_1()
                        .gap_2()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(div().flex_1().child(Input::new(&self.find_input)))
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(count),
                        )
                        .child(
                            Button::new("find-previous")
                                .label("Previous")
                                .compact()
                                .outline()
                                .on_click(cx.listener(|root, _, _, cx| root.step_find(false, cx))),
                        )
                        .child(
                            Button::new("find-next")
                                .label("Next")
                                .compact()
                                .outline()
                                .on_click(cx.listener(|root, _, _, cx| root.step_find(true, cx))),
                        )
                        .child(
                            Button::new("find-close")
                                .label("Close")
                                .compact()
                                .outline()
                                .on_click(
                                    cx.listener(|root, _, window, cx| root.close_find(window, cx)),
                                ),
                        ),
                )
            })
            .when(self.thread_loading, |pane| {
                pane.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child("Loading thread…"),
                )
            })
            .when(!self.thread_loading, |pane| {
                pane.child(
                    self.timeline_pane
                        .clone()
                        .cached(StyleRefinement::default().flex_1().w_full()),
                )
            })
            .children(message)
            .child(
                div()
                    .pt_6()
                    .children(self.render_queue(cx))
                    .child(self.render_composer(cx)),
            )
            .into_any_element()
    }
}
