use super::*;

pub(super) struct SidebarPane {
    pub(super) root: WeakEntity<Root>,
}

impl Render for SidebarPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.root
            .update(cx, |root, cx| {
                root.render_sidebar(window, cx).into_any_element()
            })
            .expect("sidebar root is alive")
    }
}

impl Root {
    pub(super) fn ordered_entries(&self) -> Vec<ThreadEntry> {
        let open = self.current();
        self.shelves_of(&self.visible_entries())
            .into_iter()
            .flat_map(|shelf| {
                let collapsed = self.collapsed.contains(&shelf.section)
                    && !matches!(
                        shelf.section,
                        sidebar::Section::Pinned | sidebar::Section::Active
                    );
                sidebar::visible_rows(&shelf.rows, &self.collapsed_families)
                    .into_iter()
                    .filter(|(entry, _, _)| {
                        !collapsed
                            || open
                                .as_ref()
                                .is_some_and(|(environment, thread)| entry.is(environment, thread))
                    })
                    .map(|(entry, _, _)| entry.clone())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub(super) fn visible_entries(&self) -> Vec<ThreadEntry> {
        let groups = self.project_groups();
        let scoped: Vec<_> = groups
            .iter()
            .filter(|group| self.sidebar_header.scope_keys.contains(&group.key))
            .collect();
        let query = self.sidebar_header.query.to_lowercase();
        self.entries
            .iter()
            .filter(|entry| {
                scoped.is_empty()
                    || scoped
                        .iter()
                        .any(|group| group.contains(&entry.environment, &entry.project_id))
            })
            .filter(|entry| {
                query.is_empty()
                    || entry.title.to_lowercase().contains(&query)
                    || entry.project_title.to_lowercase().contains(&query)
                    || self.sidebar_header.matches.contains(&entry.key())
            })
            .filter(|e| self.host_filter.is_empty() || self.host_filter.contains(&e.environment))
            .cloned()
            .collect()
    }

    pub(super) fn schedule_wake(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.wake_task = None;
        if self.entries.is_empty() {
            return;
        }
        let now = chrono::Utc::now().timestamp_millis();
        let working = self.entries.iter().any(|entry| {
            matches!(
                sidebar::display_status(&entry.presentation),
                sidebar::DisplayStatus::Preparing
                    | sidebar::DisplayStatus::Working
                    | sidebar::DisplayStatus::Waiting
            )
        });
        let interval = if working { 1000 } else { 60000 };
        let mut delay = interval - now.rem_euclid(interval);
        if let Some(deadline) = self.inbox.deadline() {
            delay = delay.min((deadline - now).max(0));
        }
        for entry in &self.entries {
            if let Some(wake) = entry
                .snoozed_until
                .as_deref()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            {
                let expiry = wake.timestamp_millis() - now;
                if expiry > 0 {
                    delay = delay.min(expiry);
                }
            }

            if let Some(message) = entry.presentation["latestUserMessageAt"]
                .as_str()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            {
                let expiry = message.timestamp_millis() + 120050 - now;
                if expiry > 0 {
                    delay = delay.min(expiry);
                }
            }
        }
        self.wake_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(delay as u64))
                .await;
            let _ = this.update_in(cx, |root, window, cx| {
                root.wake_task = None;
                if root.inbox.expire(chrono::Utc::now().timestamp_millis()) {
                    root.rebuild_entries();
                }
                window.mark_view_dirty(root.sidebar_pane.entity_id());
                root.schedule_wake(window, cx);
                cx.notify();
            });
        }));
    }

    fn shelves_of(&self, entries: &[ThreadEntry]) -> Vec<sidebar::Shelf> {
        sidebar::shelves(entries, &now_iso(), self.settings.working_shelf, |id| {
            self.label_of(id)
        })
    }

    pub(super) fn rebuild_entries(&mut self) {
        self.entries_revision += 1;
        let mut entries = Vec::new();
        let mut hidden = Vec::new();
        for connection in &self.connections {
            let shell = connection.shell();
            let shell = shell.lock().expect("shell lock");
            let environment_id = connection.environment_id();
            hidden.extend(
                shell
                    .threads
                    .iter()
                    .filter(|thread| !sidebar::is_listed(thread))
                    .map(|thread| {
                        (
                            (
                                connection.label().to_string(),
                                thread["id"].as_str().expect("shell thread ID").to_string(),
                            ),
                            crate::shell_presentation::present(thread, None),
                        )
                    }),
            );
            entries.extend(
                sidebar::entries(connection.label(), &shell)
                    .into_iter()
                    .map(|mut entry| {
                        if self.promoting.contains(&entry.key()) {
                            crate::inbox::as_launching(Arc::make_mut(&mut entry.presentation));
                        }
                        Arc::make_mut(&mut entry.presentation)["environmentId"] =
                            json!(environment_id);
                        entry
                    }),
            );
        }
        if self.settings.working_shelf {
            self.inbox.observe(
                entries
                    .iter()
                    .map(|entry| (entry.key(), entry.presentation.as_ref()))
                    .chain(hidden.iter().map(|(key, thread)| (key.clone(), thread))),
                chrono::Utc::now().timestamp_millis(),
            );
        } else {
            self.inbox.clear();
        }
        for entry in &mut entries {
            entry.inbox_working = Some(
                self.settings.working_shelf
                    && self.inbox.is_working(&entry.key(), &entry.presentation),
            );
            entry.returned_at = self.inbox.returned_at(&entry.key());
        }
        self.entries = self
            .shelves_of(&entries)
            .into_iter()
            .flat_map(|shelf| shelf.rows.into_iter().map(|(entry, _)| entry))
            .collect();
    }

    pub(super) fn notify_attention(&mut self) {
        let current = self.current();
        let attention: Vec<(String, String)> =
            sidebar::newly_needing_attention(&self.statuses, &self.entries)
                .into_iter()
                .filter(|e| !(self.window_active && current.as_ref() == Some(&e.key())))
                .map(|e| {
                    let what = match e.status {
                        ThreadStatus::Input => "Waiting for input",
                        ThreadStatus::Approval => "Waiting for approval",
                        _ => "Done",
                    };
                    (format!("{what} · {}", e.environment), e.title.clone())
                })
                .collect();
        self.statuses = self.entries.iter().map(|e| (e.key(), e.status)).collect();
        if !self.settings.desktop_notifications {
            return;
        }
        let sound = self.settings.notification_sound;
        for (summary, body) in attention {
            self.runtime.spawn_blocking(move || {
                let mut notification = notify_rust::Notification::new();
                notification.appname(APP_NAME).summary(&summary).body(&body);
                if sound {
                    notification.sound_name("message-new-instant");
                }
                let _ = notification.show();
            });
        }
    }

    pub(super) fn visit_if_done(&self) {
        let Some(open) = &self.conversation.open else {
            return;
        };
        let done = self.entries.iter().any(|e| {
            e.is(&open.environment, open.watch.thread_id()) && e.status == ThreadStatus::Done
        });
        if !done {
            return;
        }
        if let Some(connection) = self.connection(&open.environment) {
            let thread_id = open.watch.thread_id().to_string();
            self.runtime.spawn(async move {
                let _ = connection.visit_thread(&thread_id, &now_iso()).await;
            });
        }
    }

    pub(super) fn open_thread(
        &mut self,
        environment: String,
        thread_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        self.pr_state.page = false;
        self.history.visit(crate::navigation::Target::Thread(
            environment.clone(),
            thread_id.clone(),
        ));
        self.stash_composer(cx);
        let watch = connection.watch_thread(&self.runtime, &thread_id);
        self.conversation.open = Some(OpenThread { environment, watch });
        self.conversation.draft = None;
        self.sync_lineage();
        self.conversation.rows.clear();
        self.conversation.row_views.clear();
        self.conversation.row_subscriptions.clear();
        self.conversation.list_state.reset(0);
        self.conversation.thread_loading = true;
        self.conversation.user_cursor = None;
        self.conversation
            .list_state
            .set_follow_mode(FollowMode::Tail);
        self.conversation.message = None;
        self.runtime.spawn({
            let thread_id = thread_id.clone();
            async move {
                let _ = connection.visit_thread(&thread_id, &now_iso()).await;
            }
        });
        self.load_composer();
        self.save_conversations();
        cx.notify();
    }

    pub(super) fn close_thread(&mut self, cx: &mut Context<Self>) {
        self.conversation.open = None;
        self.sync_lineage();
        self.conversation.rows.clear();
        self.conversation.row_views.clear();
        self.conversation.row_subscriptions.clear();
        self.conversation.list_state.reset(0);
        self.conversation.user_cursor = None;
        cx.notify();
    }

    pub(super) fn open_entry(&self) -> Option<&ThreadEntry> {
        let open = self.conversation.open.as_ref()?;
        self.entries
            .iter()
            .find(|e| e.is(&open.environment, open.watch.thread_id()))
    }

    pub(super) fn settle(&mut self, cx: &mut Context<Self>) {
        if let Some(entry) = self.open_entry().cloned() {
            let op = if entry.presentation["settledOverride"] == "settled" {
                ThreadOp::Unsettle
            } else {
                ThreadOp::Settle
            };
            self.thread_op(
                ThreadMenu {
                    environment: entry.environment,
                    thread_id: entry.thread_id,
                    op,
                },
                cx,
            );
        }
    }

    pub(super) fn finish_parks(&mut self, cx: &mut Context<Self>) {
        let plans = std::mem::take(&mut self.park_plans);
        for plan in plans {
            let Some(connection) = self.connection(&plan.target.0) else {
                continue;
            };
            if connection.shell().lock().expect("shell lock").sequence < plan.sequence {
                self.park_plans.push(plan);
                continue;
            }
            if self.current().as_ref() != Some(&plan.target) {
                continue;
            }
            let Some(thread) = self.thread_value(&plan.target.0, &plan.target.1) else {
                continue;
            };
            let presented = crate::shell_presentation::present(&thread, None);
            let parked = if plan.snooze {
                crate::snooze::effective(&presented, &now_iso())
            } else {
                presented["settledOverride"] == "settled"
            };
            if parked {
                self.focus_after_park = true;
                cx.notify();
                if let Some((environment, thread)) = plan.next {
                    self.open_thread(environment, thread, cx);
                } else {
                    self.begin_new_thread(cx);
                    if let Some(draft) = &mut self.conversation.draft {
                        draft.environment = Some(plan.target.0);
                    }
                }
            }
        }
    }

    pub(super) fn thread_op(&mut self, menu: ThreadMenu, cx: &mut Context<Self>) {
        let Some(connection) = self.connection(&menu.environment) else {
            return;
        };
        let until = match &menu.op {
            ThreadOp::Snooze(hours) => Some(
                humantime::format_rfc3339_millis(
                    SystemTime::now() + Duration::from_secs(hours * 3600),
                )
                .to_string(),
            ),
            ThreadOp::SnoozeUntil(until) => Some(until.clone()),
            _ => None,
        };
        let snooze = matches!(menu.op, ThreadOp::Snooze(_) | ThreadOp::SnoozeUntil(_));
        let parking = snooze || matches!(menu.op, ThreadOp::Settle);
        let parking_key = (menu.environment.clone(), menu.thread_id.clone(), snooze);
        if parking && !self.parking.insert(parking_key.clone()) {
            return;
        }
        let plan = if parking
            && self.current().as_ref() == Some(&(menu.environment.clone(), menu.thread_id.clone()))
        {
            let visible = self.ordered_entries();
            let now = now_iso();
            let next = sidebar::next_with(
                &visible,
                Some((&menu.environment, &menu.thread_id)),
                true,
                |entry| {
                    !matches!(
                        sidebar::section_of(entry, &now),
                        sidebar::Section::Snoozed | sidebar::Section::Settled
                    )
                },
            )
            .map(ThreadEntry::key);
            Some(ParkPlan {
                target: (menu.environment.clone(), menu.thread_id.clone()),
                next,
                snooze,
                sequence: 0,
            })
        } else {
            None
        };
        let undo = self
            .thread_value(&menu.environment, &menu.thread_id)
            .and_then(|thread| thread_commands::undo_plan(&menu.op, &thread, &menu.thread_id))
            .map(|(kind, commands)| thread_commands::UndoEntry {
                environment: menu.environment.clone(),
                thread: menu.thread_id.clone(),
                kind,
                commands,
            });
        self.thread_undos.retain(|entry| {
            entry.environment != menu.environment || entry.thread != menu.thread_id
        });
        let runtime = self.runtime.clone();
        let thread_id = menu.thread_id.clone();
        let op = menu.op.clone();
        cx.spawn(async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                match op {
                    ThreadOp::Pin => connection.pin_thread(&thread_id, true).await,
                    ThreadOp::Unpin => connection.pin_thread(&thread_id, false).await,
                    ThreadOp::Snooze(_) | ThreadOp::SnoozeUntil(_) => {
                        connection.snooze_thread(&thread_id, until.as_deref()).await
                    }
                    ThreadOp::Unsnooze => connection.snooze_thread(&thread_id, None).await,
                    ThreadOp::Settle => connection.settle_thread(&thread_id, true).await,
                    ThreadOp::Unsettle => connection.settle_thread(&thread_id, false).await,
                    ThreadOp::Archive => connection.archive_thread(&thread_id).await,
                }
            })
            .await;
            let _ = this.update(cx, |root, cx| {
                root.parking.remove(&parking_key);
                match result {
                    Err(error) => {
                        let title = match menu.op {
                            ThreadOp::Settle => "Failed to settle thread",
                            ThreadOp::Unsettle => "Failed to un-settle thread",
                            ThreadOp::Pin => "Failed to pin thread",
                            ThreadOp::Unpin => "Failed to unpin thread",
                            ThreadOp::Snooze(_) | ThreadOp::SnoozeUntil(_) => {
                                "Failed to snooze thread"
                            }
                            ThreadOp::Unsnooze => "Failed to wake thread",
                            ThreadOp::Archive => "Failed to archive thread",
                        };
                        root.show_sidebar_error(title, error.root_cause().to_string(), cx);
                    }
                    Ok(sequence) => {
                        if let Some(undo) = undo {
                            root.remember_thread_undo(undo, cx);
                        }
                        if let Some(mut plan) = plan {
                            plan.sequence = sequence;
                            root.park_plans.push(plan);
                            root.finish_parks(cx);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn open_in_browser(&mut self, cx: &mut Context<Self>) {
        let Some((environment, thread_id)) = self.current() else {
            return;
        };
        let url = self
            .connection(&environment)
            .and_then(|c| c.thread_url(&thread_id));
        match url {
            Some(url) => cx.open_url(&url),
            None => {
                self.conversation.message = Some("The environment id is not known yet.".into());
                cx.notify();
            }
        }
    }

    pub(super) fn next_done(&mut self, forward: bool, cx: &mut Context<Self>) {
        let current = self.current();
        let visible = self.visible_entries();
        let next = sidebar::next_with(
            &visible,
            current.as_ref().map(|(e, t)| (e.as_str(), t.as_str())),
            forward,
            |e| {
                sidebar::needs_attention(e.status)
                    || self
                        .drafts
                        .has(&drafts::thread_key(&e.environment, &e.thread_id))
            },
        )
        .map(ThreadEntry::key);
        if let Some((environment, thread_id)) = next {
            self.open_thread(environment, thread_id, cx);
        }
    }

    pub(super) fn adjacent_thread(&mut self, forward: bool, cx: &mut Context<Self>) {
        let current = self.current();
        let visible = self.ordered_entries();
        let next = sidebar::adjacent(
            &visible,
            current.as_ref().map(|(e, t)| (e.as_str(), t.as_str())),
            forward,
        )
        .map(ThreadEntry::key);
        if let Some((environment, thread_id)) = next {
            self.open_thread(environment, thread_id, cx);
        }
    }

    fn render_sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().colors;
        let sidebar_active = crate::appearance::sidebar_active(cx);
        let sidebar_hover = crate::appearance::sidebar_hover(cx);
        let sidebar_muted = crate::appearance::sidebar_muted(cx);
        let open = self.current();
        let primary = self.connections.first().map(|c| c.label().to_string());
        let searching = !self.sidebar_header.query.is_empty();
        let shelves = self.shelves_of(&self.visible_entries());
        let has_settled = shelves
            .iter()
            .any(|shelf| shelf.section == sidebar::Section::Settled);
        let mut lower_started = false;
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut index = 0usize;
        for shelf in shelves {
            let section = shelf.section;
            let lower = matches!(
                section,
                sidebar::Section::Working | sidebar::Section::Snoozed | sidebar::Section::Settled
            );
            let bottom = lower && !lower_started;
            lower_started |= lower;
            let collapsed = !searching && self.collapsed.contains(&section);
            if !searching && !matches!(section, sidebar::Section::Pinned | sidebar::Section::Active)
            {
                rows.push(self.render_shelf_toggle(
                    section,
                    shelf.rows.len(),
                    bottom,
                    rows.len(),
                    cx,
                ));
            }
            for (entry, depth, family) in
                sidebar::visible_rows(&shelf.rows, &self.collapsed_families)
            {
                if lower
                    && collapsed
                    && !open
                        .as_ref()
                        .is_some_and(|(environment, thread)| entry.is(environment, thread))
                {
                    continue;
                }
                index += 1;
                let jump_label = self
                    .sidebar_stamp
                    .as_ref()
                    .and_then(|stamp| stamp.jump_labels.get(index - 1))
                    .cloned()
                    .flatten();
                let family_key = entry.key();
                let focus_state = window.use_keyed_state(
                    SharedString::from(format!(
                        "sidebar-row-focus:{}:{}",
                        entry.environment, entry.thread_id
                    )),
                    cx,
                    |_, cx| sidebar_controls::RowFocus::new(cx),
                );
                let row_focus = focus_state.read(cx).clone();
                let row_focused =
                    window.last_input_was_keyboard() && row_focus.row.is_focused(window);
                let row_key_focus = row_focus.row.clone();
                let actions_focused =
                    row_focus.snooze.is_focused(window) || row_focus.settle.is_focused(window);
                let actions_visible = actions_focused || row_focus.menu_open;
                let selected = open.as_ref().is_some_and(|(env, id)| entry.is(env, id));
                let woke = crate::snooze::woke_at(&entry.presentation, &now_iso());
                let woke = crate::snooze::woke_unseen(
                    woke.as_deref(),
                    entry.presentation["settledOverride"] == "settled",
                    entry.presentation["lastVisitedAt"].as_str(),
                    None,
                );
                let receded = sidebar::should_recede(
                    sidebar::display_status(&entry.presentation),
                    sidebar::unseen_completion(&entry.presentation),
                    woke,
                    selected,
                    false,
                );
                let remote = primary.as_ref() != Some(&entry.environment);
                let (environment, thread_id) = entry.key();
                let menu_target = (environment.clone(), thread_id.clone());
                let key_target = menu_target.clone();
                let display = sidebar::display_status(&entry.presentation);
                let status_label = if display == sidebar::DisplayStatus::Working
                    && entry
                        .presentation
                        .pointer("/goal/status")
                        .and_then(Value::as_str)
                        == Some("active")
                {
                    "Goal"
                } else if display != sidebar::DisplayStatus::Ready {
                    display.label()
                } else if woke {
                    "Woke"
                } else if sidebar::unseen_completion(&entry.presentation) {
                    "Done"
                } else {
                    ""
                };
                let accessible_label = [
                    entry.title.as_str(),
                    if searching { "" } else { status_label },
                    entry.project_title.as_str(),
                ]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(", ");
                let pinned = entry.pin_order.is_some();
                let snoozed = section == sidebar::Section::Snoozed;
                let settled = section == sidebar::Section::Settled;
                let slim = settled || snoozed;
                let scroll = self.sidebar_scroll.clone();
                let child_index = rows.len();
                rows.push(
                    v_flex()
                        .group("sidebar-row")
                        .id(SharedString::from(format!(
                            "thread:{environment}:{thread_id}"
                        )))
                        .role(if searching {
                            accesskit::Role::ListBoxOption
                        } else {
                            accesskit::Role::Button
                        })
                        .aria_label(accessible_label)
                        .when(searching, |row| {
                            row.aria_selected(index - 1 == self.sidebar_header.selected)
                        })
                        .aria_current(if selected {
                            accesskit::AriaCurrent::Page
                        } else {
                            accesskit::AriaCurrent::False
                        })
                        .track_focus(&row_focus.row)
                        .on_a11y_action(accesskit::Action::ScrollIntoView, move |_, window, _| {
                            scroll.scroll_to_item(child_index);
                            window.refresh();
                        })
                        .w_full()
                        .pl(px(crate::appearance::ROW_INSET + 16. * depth as f32))
                        .pr(px(crate::appearance::ROW_INSET))
                        .my(px(2.))
                        .py_2()
                        .gap_1()
                        .when(slim, |row| row.py_0().gap_0().h(px(36.)))
                        .rounded_md()
                        .when(row_focused, |row| {
                            row.relative().child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .rounded_md()
                                    .border(px(2.))
                                    .border_color(theme.ring.opacity(0.7)),
                            )
                        })
                        .when(
                            selected || searching && index - 1 == self.sidebar_header.selected,
                            |row| row.bg(sidebar_active),
                        )
                        .when(selected && self.settings.light_theme, |row| {
                            row.relative().child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(theme.sidebar_border.opacity(0.7)),
                            )
                        })
                        .when(
                            !selected
                                && matches!(
                                    entry.status,
                                    ThreadStatus::Idle | ThreadStatus::Working
                                ),
                            |row| row.text_color(sidebar_muted.opacity(0.75)),
                        )
                        .when(settled || snoozed, |row| row.text_color(sidebar_muted))
                        .hover(|row| {
                            row.text_color(theme.sidebar_foreground).bg(if selected {
                                sidebar_active
                            } else {
                                sidebar_hover
                            })
                        })
                        .on_click(cx.listener(move |root, _, window, cx| {
                            if searching {
                                root.sidebar_header
                                    .search
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                            }
                            root.open_thread(environment.clone(), thread_id.clone(), cx);
                        }))
                        .on_key_down(cx.listener(move |root, event: &KeyDownEvent, window, cx| {
                            if row_key_focus.is_focused(window)
                                && matches!(event.keystroke.key.as_str(), "enter" | "space")
                                && event.keystroke.modifiers.is_subset_of(&Modifiers::shift())
                            {
                                window.prevent_default();
                                cx.stop_propagation();
                                root.open_thread(key_target.0.clone(), key_target.1.clone(), cx);
                            }
                        }))
                        .when(!slim, |row| {
                            row.child(
                                h_flex()
                                    .w_full()
                                    .h(px(20.))
                                    .gap(px(6.))
                                    .child(self.render_host_chip(&entry.environment, remote, cx))
                                    .child(div().flex_1())
                                    .child(self.render_row_metadata(
                                        &entry,
                                        section,
                                        &focus_state,
                                        actions_visible,
                                        jump_label.clone(),
                                        cx,
                                    )),
                            )
                        })
                        .child(
                            h_flex()
                                .h(px(if slim { 36. } else { 20. }))
                                .w_full()
                                .min_w_0()
                                .gap_1()
                                .text_sm()
                                .line_height(px(20.))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(theme.sidebar_foreground.opacity(if receded {
                                            0.75
                                        } else {
                                            1.
                                        }))
                                        .font_weight(if receded {
                                            FontWeight::NORMAL
                                        } else {
                                            FontWeight::MEDIUM
                                        })
                                        .when(row_focused, |title| {
                                            title.text_color(theme.sidebar_foreground)
                                        })
                                        .group_hover("sidebar-row", |title| {
                                            title.text_color(theme.sidebar_foreground)
                                        })
                                        .child(settings_controls::text(entry.title.clone())),
                                )
                                .when(!slim, |row| {
                                    row.children(self.render_sidebar_provider_stack(&entry, cx))
                                })
                                .when(slim, |row| {
                                    row.child(self.render_row_metadata(
                                        &entry,
                                        section,
                                        &focus_state,
                                        actions_visible,
                                        jump_label.clone(),
                                        cx,
                                    ))
                                }),
                        )
                        .when_some(family, |row, family| {
                            let label = match (family.collapsed, family.working) {
                                (true, working) if working > 0 => {
                                    format!("▸ {} · {working} working", family.children)
                                }
                                (true, _) => format!("▸ {}", family.children),
                                (false, _) => format!("▾ {}", family.children),
                            };
                            let key = family_key.clone();
                            row.child(
                                div()
                                    .id(("family", index))
                                    .flex_none()
                                    .px_1()
                                    .text_xs()
                                    .text_color(sidebar_muted)
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |root, _, _, cx| {
                                        cx.stop_propagation();
                                        if !root.collapsed_families.remove(&key) {
                                            root.collapsed_families.insert(key.clone());
                                        }
                                        cx.notify();
                                    }))
                                    .child(label),
                            )
                        })
                        .context_menu(move |menu, _, _| {
                            let (environment, thread_id) = menu_target.clone();
                            let op = |op: ThreadOp| -> Box<dyn Action> {
                                Box::new(ThreadMenu {
                                    environment: environment.clone(),
                                    thread_id: thread_id.clone(),
                                    op,
                                })
                            };
                            let menu = if pinned {
                                menu.menu("Unpin", op(ThreadOp::Unpin))
                            } else {
                                menu.menu("Pin", op(ThreadOp::Pin))
                            };
                            let menu = if snoozed {
                                menu.menu("Unsnooze", op(ThreadOp::Unsnooze))
                            } else {
                                menu.menu("Snooze 1 hour", op(ThreadOp::Snooze(1)))
                                    .menu("Snooze 4 hours", op(ThreadOp::Snooze(4)))
                                    .menu("Snooze 1 day", op(ThreadOp::Snooze(24)))
                            };
                            let menu = if settled {
                                menu.menu("Unsettle", op(ThreadOp::Unsettle))
                            } else {
                                menu.menu("Settle", op(ThreadOp::Settle))
                            };
                            menu.separator().menu("Archive", op(ThreadOp::Archive))
                        })
                        .into_any_element(),
                );
            }
        }
        if !has_settled && self.sidebar_header.query.is_empty() {
            rows.push(self.render_shelf_toggle(
                sidebar::Section::Settled,
                0,
                !lower_started,
                rows.len(),
                cx,
            ));
        }
        if searching && rows.is_empty() {
            rows.push(
                div()
                    .id("sidebar-search-status")
                    .role(accesskit::Role::Status)
                    .px_2()
                    .py_6()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(if self.sidebar_header.pending {
                        "Searching thread messages…"
                    } else {
                        "No threads found"
                    })
                    .into_any_element(),
            );
        }
        v_flex()
            .id("sidebar-navigation")
            .role(accesskit::Role::Navigation)
            .aria_label("Threads")
            .w(px(self.settings.sidebar_width))
            .h_full()
            .flex_none()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .child(self.render_sidebar_header(cx))
            .children(self.sidebar_header.error.clone().map(|error| settings_controls::text(error).text_color(theme.danger)))
            .child(
                v_flex()
                    .id("threads")
                    .role(if searching { accesskit::Role::ListBox } else { accesskit::Role::ScrollView })
                    .aria_label(if searching { "Thread search results" } else { "" })
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.sidebar_scroll)
                    .gap(px(1.))
                    .p_2()
                    .children(rows),
            )
            .when(self.pr_state.page, |sidebar| {
                sidebar.child(
                    h_flex().px_2().py_2().gap_1().child(
                        Button::new("pr-back")
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(Icon::new(IconName::ArrowLeft).size(px(16.)))
                                    .child(text!("Back")),
                            )
                            .accessibility_label("Back")
                            .ghost()
                            .flex_1()
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.pr_state.page = false;
                                cx.notify();
                            })),
                    ),
                )
            })
            .when(!self.pr_state.page, |sidebar| {
                sidebar.child(
                    h_flex()
                        .px_2()
                        .py_2()
                        .gap_1()
                        .child(
                            Button::new("settings")
                                .icon(IconName::Settings)
                                .tooltip("Settings")
                                .accessibility_label("Settings")
                                .size(px(32.))
                                .ghost()
                                .on_click(cx.listener(|root, _, _, cx| {
                                    root.usage_page = None;
                                    root.settings_open = !root.settings_open;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("pull-requests")
                                .icon(gpui_kit::assets::IconName::GitPullRequest)
                                .tooltip("Pull requests")
                                .accessibility_label("Pull requests")
                                .size(px(32.))
                                .ghost()
                                .on_click(cx.listener(|root, _, window, cx| {
                                    root.open_pull_requests(true, cx);
                                    root.pr_search
                                        .update(cx, |input, cx| input.focus(window, cx));
                                })),
                        )
                        .child(
                            Button::new("usage")
                                .icon(gpui_kit::assets::IconName::ChartNoAxesColumn)
                                .tooltip("Usage")
                                .accessibility_label("Usage")
                                .size(px(32.))
                                .ghost()
                                .on_click(cx.listener(|root, _, window, cx| {
                                    root.toggle_usage(window, cx)
                                })),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new("updates")
                                .icon(gpui_kit::assets::IconName::RefreshCw)
                                .tooltip("Updates unavailable: Automatic updates are not available because no update feed is configured.")
                                .accessibility_label("Updates unavailable: Automatic updates are not available because no update feed is configured.")
                                .size(px(32.))
                                .ghost()
                                .disabled(true),
                        ),
                )
            })
    }
}
