use super::*;

impl Root {
    pub(super) fn side_chat_title(&self, id: &str) -> Option<String> {
        let thread = id.strip_prefix("side-chat:")?;
        let (environment, _) = self.right_panel_target.as_ref()?;
        self.thread_value(environment, thread)?["title"]
            .as_str()
            .map(str::to_owned)
    }

    pub(super) fn side_chat_disabled_reason(&self) -> Option<&'static str> {
        let current = self.current();
        let thread = current
            .as_ref()
            .and_then(|(env, id)| self.thread_value(env, id));
        if self.side_chat_parent.is_some()
            || thread.as_ref().is_some_and(|t| {
                !t["lineage"]["sideChat"].is_null() && t["lineage"]["sideChat"] != false
            })
        {
            return Some("Side chats cannot start side chats.");
        }
        if thread
            .as_ref()
            .is_some_and(|t| t["lineage"]["relationshipToParent"] == "subagent")
        {
            return Some("Subagent threads cannot start side chats.");
        }
        let environment = current
            .as_ref()
            .map(|(env, _)| env.clone())
            .or_else(|| self.composer_target().map(|target| target.0));
        let Some(config) = environment.and_then(|env| self.config_of(&env)) else {
            return Some("Side chats are not ready yet. Try again in a moment.");
        };
        if config["environment"]["capabilities"]["sideChats"] != true {
            return Some("Update this environment's T3 Code server to use side chats.");
        }
        if current.is_none() {
            return Some("Side chats start after the first message.");
        }
        if thread
            .as_ref()
            .is_some_and(|thread| thread["worktreePath"].is_null())
            && self.open.as_ref().is_some_and(|open| {
                open.watch
                    .state()
                    .lock()
                    .expect("thread state")
                    .list("runs")
                    .iter()
                    .any(|run| {
                        matches!(
                            run["status"].as_str(),
                            Some("preparing" | "failed" | "interrupted")
                        ) && run["workspacePreparation"]["type"] == "worktree"
                    })
            })
        {
            return Some("Available once the thread's worktree is set up.");
        }
        None
    }

    pub(super) fn open_side_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.side_chat_pending || self.side_chat_disabled_reason().is_some() {
            return;
        }
        let Some((environment, source)) = self.current() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let Some((_, _model, mode, _)) = self.composer_target() else {
            return;
        };
        self.side_chat_pending = true;
        let target = uuid::Uuid::new_v4().to_string();
        let parent = (environment.clone(), source.clone());
        let runtime = self.runtime.clone();
        self.spawn_pane_in(window, cx, async move |this, cx| {
            let target_id = target.clone();
            let result = on_runtime(&runtime, async move {
                connection.dispatch(json!({"type":"thread.fork","createdBy":"user","creationSource":"web","sourceThreadId":source,"targetThreadId":target_id,"sourcePoint":{"type":"latest_stable"},"sideChat":true,"title":"Side chat"})).await?;
                let start = async {
                    let shell = tokio::time::timeout(Duration::from_secs(5), async {
                        loop {
                            let shell = connection.shell().lock().expect("shell lock").threads.iter().find(|t| t["id"] == target_id).cloned();
                            if let Some(shell) = shell { return shell; }
                            tokio::time::sleep(Duration::from_millis(25)).await;
                        }
                    }).await.context("The side chat was created, but it did not reach this client.")?;
                    if shell["runtimeMode"] != mode {
                        connection.set_runtime_mode(&target_id, &mode).await?;
                    }
                    Ok::<_, anyhow::Error>(())
                }.await;
                if start.is_err() {
                    connection.dispatch(json!({"type":"thread.delete","threadId":target_id,"onlyIfUnused":true})).await?;
                }
                start
            }).await;
            let _ = this.update_in(cx, |root, window, cx| {
                root.side_chat_pending = false;
                match result {
                    Ok(()) => {
                        let id = root.pane_next_id;
                        root.pane_next_id += 1;
                        let mut pane = ConversationState::new(id, None, &root.drafts_path, window, cx);
                        pane.side_chat_parent = Some(parent);
                        root.panes.insert(id, pane);
                        root.pane_layout.split(root.conversation.id, id, false);
                        root.with_conversation(id, |root| {
                            root.open_thread(environment, target.clone(), cx);
                        });
                        root.activate_panel(format!("side-chat:{target}"), cx);
                        root.save_conversations();
                    }
                    Err(error) => root.conversation.message = Some(format!("Could not start a side chat: {error:#}")),
                }
                cx.notify();
            });
        }).detach();
    }

    pub(super) fn render_side_chat_surface(&self, thread: &str) -> AnyElement {
        if self
            .right_panel_target
            .as_ref()
            .is_some_and(|(env, _)| self.thread_value(env, thread).is_none())
        {
            return div()
                .id("side-chat-deleted")
                .role(accesskit::Role::Label)
                .aria_value("This side chat was deleted.")
                .p_3()
                .text_xs()
                .child("This side chat was deleted.")
                .into_any_element();
        }
        std::iter::once(&self.conversation)
            .chain(self.panes.values())
            .find(|pane| {
                pane.open
                    .as_ref()
                    .is_some_and(|open| open.watch.thread_id() == thread)
                    && pane.side_chat_parent == self.right_panel_target
            })
            .map(|pane| {
                pane.view
                    .clone()
                    .cached(StyleRefinement::default().size_full())
                    .into_any_element()
            })
            .unwrap_or_else(|| {
                div()
                    .p_3()
                    .text_xs()
                    .child("This side chat was deleted.")
                    .into_any_element()
            })
    }

    pub(super) fn close_side_chat_surface(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(thread) = id.strip_prefix("side-chat:") else {
            return;
        };
        let Some((environment, _)) = self.right_panel_target.clone() else {
            return;
        };
        for pane in std::iter::once(&mut self.conversation).chain(self.panes.values_mut()) {
            if pane.side_chat_parent == self.right_panel_target
                && pane
                    .open
                    .as_ref()
                    .is_some_and(|open| open.watch.thread_id() == thread)
            {
                pane.side_chat_closing = true;
            }
        }
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        if self.thread_value(&environment, thread).is_none() {
            return;
        }
        let thread = thread.to_owned();
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                connection
                    .dispatch(json!({"type":"thread.delete","threadId":thread,"onlyIfUnused":true}))
                    .await
            })
            .await;
            if let Err(error) = result {
                let _ = this.update(cx, |root, cx| {
                    root.conversation.message =
                        Some(format!("Could not delete the side chat: {error:#}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn side_chat_bring_back_reason(&self) -> Option<&'static str> {
        let (environment, parent) = self.side_chat_parent.as_ref()?;
        let parent = self.thread_value(environment, parent);
        if parent.as_ref().is_some_and(|parent| {
            matches!(
                crate::shell_presentation::present(parent, None)["runtime"]["status"].as_str(),
                Some("running" | "starting" | "preparing" | "queued" | "waiting")
            )
        }) {
            return Some("Available when the main thread is idle.");
        }
        if self
            .open
            .as_ref()
            .and_then(|open| {
                crate::lineage::latest_merge_run(&open.watch.state().lock().expect("thread state"))
                    .map(str::to_owned)
            })
            .is_none()
        {
            return Some("Available after a finished reply.");
        }
        None
    }

    fn bring_back_side_chat(&mut self, cx: &mut Context<Self>) {
        if self.side_chat_busy || self.side_chat_bring_back_reason().is_some() {
            return;
        }
        let Some((environment, parent)) = self.side_chat_parent.clone() else {
            return;
        };
        let Some((_, thread)) = self.current() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let Some(run) = self.open.as_ref().and_then(|open| {
            crate::lineage::latest_merge_run(&open.watch.state().lock().expect("thread state"))
                .map(str::to_owned)
        }) else {
            return;
        };
        self.side_chat_busy = true;
        let runtime = self.runtime.clone();
        self.spawn_pane(cx, async move |this, cx| {
            let result = on_runtime(&runtime, async move { connection.dispatch(json!({"type":"thread.merge_back","createdBy":"user","creationSource":"web","sourceThreadId":thread,"targetThreadId":parent,"sourcePoint":{"type":"run","runId":run}})).await }).await;
            let _ = this.update(cx, |root, cx| {
                root.side_chat_busy = false;
                root.conversation.message = Some(match result { Ok(_) => "Brought back\nThe main thread gets this side chat with your next message there.".into(), Err(error) => format!("Could not bring back the side chat: {error:#}") });
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    fn delete_side_chat(&mut self, cx: &mut Context<Self>) {
        let Some((environment, thread)) = self.current() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        self.side_chat_busy = true;
        let runtime = self.runtime.clone();
        self.spawn_pane(cx, async move |this, cx| {
            let id = thread.clone();
            let result = on_runtime(&runtime, async move {
                connection
                    .dispatch(json!({"type":"thread.delete","threadId":id}))
                    .await
            })
            .await;
            let _ = this.update(cx, |root, cx| {
                root.side_chat_busy = false;
                match result {
                    Ok(_) => root.close_panel_tab(&format!("side-chat:{thread}"), cx),
                    Err(error) => {
                        root.conversation.message =
                            Some(format!("Could not delete the side chat: {error:#}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_side_chat_header(&self, cx: &Context<Self>) -> AnyElement {
        let title = self
            .current()
            .and_then(|(environment, thread)| self.thread_value(&environment, &thread))
            .and_then(|thread| thread["title"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "Side chat".into());
        h_flex()
            .id(("side-chat-header", self.conversation.id))
            .h(px(36.))
            .pl_3()
            .pr_1()
            .gap_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .id(("side-chat-title", self.conversation.id))
                    .role(accesskit::Role::Label)
                    .aria_value(title.clone())
                    .child(title.clone()),
            )
            .child(
                Button::new(("bring-back", self.conversation.id))
                    .label("Bring back")
                    .xsmall()
                    .h(px(24.))
                    .ghost()
                    .compact()
                    .disabled(self.side_chat_busy || self.side_chat_bring_back_reason().is_some())
                    .tooltip(
                        self.side_chat_bring_back_reason()
                            .unwrap_or("Adds this side chat to the main thread's next message."),
                    )
                    .on_click(
                        self.pane_listener(cx, |root, _, _, cx| root.bring_back_side_chat(cx)),
                    ),
            )
            .child(
                Button::new(("delete-side-chat", self.conversation.id))
                    .xsmall()
                    .h(px(24.))
                    .icon(gpui_kit::assets::IconName::Trash)
                    .ghost()
                    .compact()
                    .accessibility_label("Delete side chat")
                    .disabled(self.side_chat_busy)
                    .on_click(self.pane_listener(cx, move |root, _, window, cx| {
                        if root.settings.values["confirmThreadDelete"] == false {
                            root.delete_side_chat(cx);
                            return;
                        }
                        let pane = root.pane_root(cx);
                        let title = title.clone();
                        window.open_alert_dialog(cx, move |dialog, _, _| {
                            let pane = pane.clone();
                            dialog
                                .title(format!("Delete side chat \"{title}\"?"))
                                .centered()
                                .child("This permanently clears its conversation history.")
                                .confirm()
                                .on_ok(move |_, _, cx| {
                                    let _ = pane.update(cx, |root, cx| root.delete_side_chat(cx));
                                    true
                                })
                        });
                    })),
            )
            .child(
                Button::new(("close-side-chat", self.conversation.id))
                    .xsmall()
                    .h(px(24.))
                    .icon(IconName::Close)
                    .ghost()
                    .compact()
                    .accessibility_label("Close side chat")
                    .on_click(self.pane_listener(cx, |root, _, _, cx| {
                        if let Some((_, thread)) = root.current() {
                            root.close_panel_tab(&format!("side-chat:{thread}"), cx);
                        }
                    })),
            )
            .into_any_element()
    }
}
