use super::*;

pub(super) struct TimelinePane {
    pub(super) queue: Entity<composer_queue::QueuePane>,
    pub(super) has_queue: bool,
    pub(super) views: Vec<Entity<RowView>>,
    pub(super) list: ListState,
    pub(super) minimap: Entity<super::minimap::Minimap>,
}

impl Render for TimelinePane {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let views = self.views.clone();
        let queue = self.has_queue.then(|| self.queue.clone());
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
                        .or_else(|| {
                            queue.clone().filter(|_| ix == views.len()).map(|queue| {
                                queue
                                    .cached(StyleRefinement::default().w_full())
                                    .into_any_element()
                            })
                        })
                        .unwrap_or_else(|| div().into_any_element())
                })
                .w_full()
                .h_full()
                .pt_4(),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(px(48.))
                    .child(self.minimap.clone()),
            )
            .child(
                gpui_kit::base::Scrollbar::vertical(&self.list)
                    .mode(gpui_kit::base::ScrollbarMode::Always),
            )
    }
}

impl Root {
    pub(super) fn rebuild_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &self.conversation.open else {
            return;
        };
        let (rows, queued, loading) = {
            let state = open.watch.state();
            let state = state.lock().expect("thread lock");
            (
                timeline::rows(&state),
                timeline::queued(&state),
                state.projection.is_empty(),
            )
        };
        self.conversation.revision += 1;
        let had_queue = !self.conversation.queued.is_empty();
        self.conversation.queued = queued;
        self.conversation.thread_loading = loading;
        let mut existing: HashMap<String, Entity<RowView>> = self
            .rows
            .iter()
            .map(|r| r.key.clone())
            .zip(self.conversation.row_views.iter().cloned())
            .collect();
        let mut old_keys: Vec<String> = self
            .conversation
            .rows
            .iter()
            .map(|r| r.key.clone())
            .collect();
        let mut subscriptions = std::mem::take(&mut self.conversation.row_subscriptions);
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
        let mut keys: Vec<String> = rows.iter().map(|row| row.key.clone()).collect();
        if had_queue {
            old_keys.push("composer-queue".into());
        }
        if !self.conversation.queued.is_empty() {
            keys.push("composer-queue".into());
        }
        let prefix = old_keys
            .iter()
            .zip(&keys)
            .take_while(|(old, new)| old == new)
            .count();
        let suffix = old_keys[prefix..]
            .iter()
            .rev()
            .zip(keys[prefix..].iter().rev())
            .take_while(|(old, new)| old == new)
            .count();
        if prefix != old_keys.len() || prefix != keys.len() {
            if rows.len() > self.conversation.rows.len() {
                self.conversation.user_cursor = None;
            }
            self.conversation.list_state.splice(
                prefix..old_keys.len() - suffix,
                keys.len() - prefix - suffix,
            );
        }
        self.conversation.rows = rows;
        self.conversation.row_views = views;
        self.load_timeline_images(cx);
        for (row, view) in self
            .conversation
            .rows
            .iter()
            .zip(&self.conversation.row_views)
        {
            if let Some(metadata) = row
                .message_part
                .as_ref()
                .and_then(|part| part.metadata.as_ref())
            {
                view.update(cx, |view, cx| {
                    view.set_message_hovered(
                        self.conversation.hovered_message.as_ref() == Some(&metadata.message_id),
                        cx,
                    )
                });
            }
        }
        let mut minimap_items: Vec<(usize, String, String)> = Vec::new();
        for (index, row) in self.conversation.rows.iter().enumerate() {
            if row.kind == RowKind::User {
                minimap_items.push((
                    index,
                    row.text.split_whitespace().collect::<Vec<_>>().join(" "),
                    String::new(),
                ));
            } else if row.kind == RowKind::Assistant
                && let Some((_, _, text)) = minimap_items.last_mut()
            {
                text.extend(
                    row.text
                        .chars()
                        .take(1500usize.saturating_sub(text.chars().count())),
                );
            }
        }
        self.conversation.timeline_pane.update(cx, |pane, cx| {
            pane.views = self.conversation.row_views.clone();
            pane.has_queue = !self.conversation.queued.is_empty();
            pane.minimap.update(cx, |map, cx| {
                if map.items != minimap_items {
                    map.items = minimap_items;
                    cx.notify();
                }
            });
            cx.notify();
        });
        self.conversation.row_subscriptions = kept;
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
        if let Some(id) = self
            .panes
            .iter()
            .find_map(|(id, pane)| pane.row_views.iter().any(|row| row == view).then_some(*id))
        {
            self.with_conversation(id, |root| root.on_row_event(view, event, window, cx));
            return;
        }
        match event {
            RowEvent::MessageHover {
                message_id,
                hovered,
            } => {
                let old = self.conversation.hovered_message.clone();
                if *hovered {
                    self.conversation.hovered_message = Some(message_id.clone());
                } else if self.conversation.hovered_message.as_ref() == Some(message_id) {
                    self.conversation.hovered_message = None;
                }
                if old != self.conversation.hovered_message {
                    for (row, view) in self
                        .conversation
                        .rows
                        .iter()
                        .zip(&self.conversation.row_views)
                    {
                        if let Some(metadata) = row
                            .message_part
                            .as_ref()
                            .and_then(|part| part.metadata.as_ref())
                            && (Some(&metadata.message_id) == old.as_ref()
                                || Some(&metadata.message_id)
                                    == self.conversation.hovered_message.as_ref())
                        {
                            view.update(cx, |view, cx| {
                                view.set_message_hovered(
                                    Some(&metadata.message_id)
                                        == self.conversation.hovered_message.as_ref(),
                                    cx,
                                )
                            });
                        }
                    }
                }
            }
            RowEvent::CopyMessage { message_id } => {
                let text = self.conversation.open.as_ref().and_then(|open| {
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
                        self.conversation.message =
                            Some(format!("Failed to copy message: {error:#}"));
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
                self.spawn_pane(cx, async move |this, cx| {
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
                                root.conversation.message =
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
                self.spawn_pane(cx, async move |this, cx| {
                    let result = on_runtime(&runtime, async move {
                        connection
                            .implement_plan(&thread_id, &plan_id, &markdown)
                            .await
                    })
                    .await;
                    if let Err(error) = result {
                        view.update(cx, |view, cx| view.set_responding(false, cx));
                        let _ = this.update(cx, |root, cx| {
                            root.conversation.message =
                                Some(format!("Implementing the plan failed: {error:#}"));
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
        self.spawn_pane(cx, async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                connection.respond(&thread_id, &request_id, &decision).await
            })
            .await;
            if let Err(error) = result {
                view.update(cx, |view, cx| view.set_responding(false, cx));
                let _ = this.update(cx, |root, cx| {
                    root.conversation.message = Some(format!("Reply failed: {error:#}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn pending_question(&self) -> Option<(Entity<RowView>, String)> {
        self.conversation
            .rows
            .iter()
            .zip(&self.conversation.row_views)
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
        let questions = self
            .conversation
            .rows
            .iter()
            .find_map(|row| match &row.pending {
                Some(Pending::Question {
                    request_id: id,
                    questions,
                }) if *id == request_id => Some(questions.clone()),
                _ => None,
            });
        let Some(questions) = questions else { return };
        let typed = self.conversation.composer.read(cx).value().to_string();
        let Some(answers) = timeline::answers_with_text(&questions, view.read(cx).drafts(), &typed)
        else {
            self.conversation.message = Some("Answer every question first.".into());
            cx.notify();
            return;
        };
        self.conversation.message = None;
        view.update(cx, |view, cx| view.set_responding(true, cx));
        self.conversation
            .composer
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.stash_composer(cx);
        let runtime = self.runtime.clone();
        self.spawn_pane_in(window, cx, async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                connection.answer(&thread_id, &request_id, answers).await
            })
            .await;
            if let Err(error) = result {
                view.update(cx, |view, cx| view.set_responding(false, cx));
                let _ = this.update_in(cx, |root, window, cx| {
                    root.conversation.message = Some(format!("Answer failed: {error:#}"));
                    root.conversation
                        .composer
                        .update(cx, |state, cx| state.set_value(typed, window, cx));
                    root.stash_composer(cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn jump_user_message(&mut self, forward: bool, cx: &mut Context<Self>) {
        let top = self.conversation.list_state.logical_scroll_top();
        let (from, inside) = if top.item_ix < self.conversation.rows.len() {
            (top.item_ix, top.offset_in_item > px(0.))
        } else {
            (
                self.conversation
                    .user_cursor
                    .unwrap_or(self.conversation.rows.len()),
                false,
            )
        };
        let target = timeline::user_message_jump(&self.conversation.rows, from, inside, forward);
        self.conversation.user_cursor = target;
        match target {
            Some(item_ix) => self.conversation.list_state.scroll_to(ListOffset {
                item_ix,
                offset_in_item: px(0.),
            }),
            None if forward => self
                .conversation
                .list_state
                .set_follow_mode(FollowMode::Tail),
            None => self.conversation.list_state.scroll_to(ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            }),
        }
        cx.notify();
    }

    pub(super) fn render_thread(&self, cx: &mut Context<Self>) -> AnyElement {
        self.sync_queue_pane(cx);
        let message = self.render_message(cx);
        let theme = cx.theme();
        let Some(_) = &self.conversation.open else {
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
            .when(self.conversation.find_open, |pane| {
                let count = if self.conversation.find_hits.is_empty() {
                    "No results".to_string()
                } else {
                    format!(
                        "{} / {}",
                        self.conversation.find_at + 1,
                        self.conversation.find_hits.len()
                    )
                };
                pane.child(
                    h_flex()
                        .px_4()
                        .py_1()
                        .gap_2()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(
                            div()
                                .flex_1()
                                .child(Input::new(&self.conversation.find_input)),
                        )
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
                                .on_click(
                                    self.pane_listener(cx, |root, _, _, cx| {
                                        root.step_find(false, cx)
                                    }),
                                ),
                        )
                        .child(
                            Button::new("find-next")
                                .label("Next")
                                .compact()
                                .outline()
                                .on_click(
                                    self.pane_listener(cx, |root, _, _, cx| {
                                        root.step_find(true, cx)
                                    }),
                                ),
                        )
                        .child(
                            Button::new("find-close")
                                .label("Close")
                                .compact()
                                .outline()
                                .on_click(self.pane_listener(cx, |root, _, window, cx| {
                                    root.close_find(window, cx)
                                })),
                        ),
                )
            })
            .when(self.conversation.thread_loading, |pane| {
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
            .when(!self.conversation.thread_loading, |pane| {
                pane.child(
                    self.conversation
                        .timeline_pane
                        .clone()
                        .cached(StyleRefinement::default().flex_1().w_full()),
                )
            })
            .children(message)
            .child(div().pt_6().child(self.render_composer(cx)))
            .into_any_element()
    }
}
