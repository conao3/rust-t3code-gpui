use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SubmissionIntent {
    Foreground,
    Alternate,
    Background,
}

fn delivery(running: bool, alternate: bool, steer: bool, can_steer: bool) -> Delivery {
    if !running {
        Delivery::Start
    } else if can_steer && (steer != alternate) {
        Delivery::Steer
    } else {
        Delivery::Queue
    }
}

pub(super) struct ComposerControlsPane {
    pub(super) root: WeakEntity<Root>,
    pub(super) pane_id: u64,
}
impl Render for ComposerControlsPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.root
            .update(cx, |root, cx| {
                root.with_conversation(self.pane_id, |root| root.render_composer_controls(cx))
                    .unwrap_or_else(|| div().into_any_element())
            })
            .expect("composer root is alive")
    }
}

impl Root {
    /// Runs the keybinding of a typed chord, and submits the composer on the key the send shortcut
    /// names. Returns whether the keystroke was used.
    /// The directory a file mention searches: the open thread's worktree or its project, or the
    /// project the new thread starts in.
    pub(super) fn composer_cwd(&self) -> Option<(String, String)> {
        if self.conversation.draft.is_some() {
            let (environment, project_id) = self.draft_project()?;
            let project = self.project_value(&environment, &project_id)?;
            let root = self
                .conversation
                .draft
                .as_ref()
                .and_then(|draft| draft.workspace_strategy.get("worktreePath"))
                .and_then(Value::as_str)
                .or_else(|| project.get("workspaceRoot").and_then(Value::as_str))?
                .to_string();
            return Some((environment, root));
        }
        let (environment, thread_id) = self.current()?;
        let thread = self.thread_value(&environment, &thread_id)?;
        if let Some(worktree) = thread.get("worktreePath").and_then(Value::as_str) {
            return Some((environment, worktree.to_string()));
        }
        let project_id = thread.get("projectId")?.as_str()?;
        let project = self.project_value(&environment, project_id)?;
        let root = project.get("workspaceRoot")?.as_str()?.to_string();
        Some((environment, root))
    }

    /// Attaches an image on the clipboard. Returns whether there was one, so text pastes as usual.
    pub(super) fn paste_image(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let image = item.into_entries().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(image),
            _ => None,
        });
        let Some(image) = image else {
            return false;
        };
        let mime = image.format.mime_type().to_string();
        let extension = mime.rsplit('/').next().unwrap_or("png").to_string();
        let name = format!(
            "pasted-{}.{extension}",
            self.conversation.attachments.len() + 1
        );
        match attachments::Pending::new(name, mime, image.bytes) {
            Ok(pending) => self.conversation.attachments.push(pending),
            Err(error) => self.conversation.message = Some(format!("{error:#}")),
        }
        self.stash_composer(cx);
        cx.notify();
        true
    }

    fn attach_paths(&mut self, paths: &ExternalPaths, cx: &mut Context<Self>) {
        for path in paths.paths() {
            match attachments::Pending::from_path(path) {
                Ok(pending) => self.conversation.attachments.push(pending),
                Err(error) => self.conversation.message = Some(format!("{error:#}")),
            }
        }
        self.stash_composer(cx);
        cx.notify();
    }

    pub(super) fn update_completion(&mut self, cx: &mut Context<Self>) {
        let state = self.conversation.composer.read(cx);
        let text = state.value().to_string();
        let cursor = state.cursor();
        self.conversation.completion_seq += 1;
        let Some(trigger) = completion::trigger(&text, cursor) else {
            if self.conversation.completion.take().is_some() {
                cx.notify();
            }
            return;
        };
        if trigger.kind == completion::TriggerKind::Path {
            let Some((environment, cwd)) = self.composer_cwd() else {
                return;
            };
            let Some(connection) = self.connection(&environment) else {
                return;
            };
            let seq = self.conversation.completion_seq;
            let runtime = self.runtime.clone();
            let query = trigger.query.clone();
            self.spawn_pane(cx, async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                if this
                    .read_with(cx, |root, _| root.conversation.completion_seq != seq)
                    .unwrap_or(true)
                {
                    return;
                }
                let found = on_runtime(&runtime, async move {
                    connection.search_entries(&cwd, &query, 20).await
                })
                .await;
                let _ = this.update(cx, |root, cx| {
                    if root.conversation.completion_seq != seq {
                        return;
                    }
                    root.conversation.completion = found.ok().map(|result| Completion {
                        trigger,
                        suggestions: completion::path_suggestions(&result),
                        selected: 0,
                    });
                    cx.notify();
                });
            })
            .detach();
            return;
        }
        let provider = self.composer_target().and_then(|(environment, model, ..)| {
            let instance = model?.get("instanceId")?.as_str()?.to_string();
            let config = self.config_of(&environment)?;
            config
                .get("providers")?
                .as_array()?
                .iter()
                .find(|p| p.get("instanceId").and_then(Value::as_str) == Some(&instance))
                .cloned()
        });
        let suggestions = provider
            .map(|p| completion::provider_suggestions(&p, trigger.kind, &trigger.query))
            .unwrap_or_default();
        self.conversation.completion = Some(Completion {
            trigger,
            suggestions,
            selected: 0,
        });
        cx.notify();
    }

    /// Moves through and accepts the completion list while it is open. Returns whether the key
    /// was used.
    pub(super) fn completion_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(open) = self
            .conversation
            .completion
            .as_mut()
            .filter(|c| !c.suggestions.is_empty())
        else {
            return false;
        };
        if !self
            .conversation
            .composer
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
        {
            return false;
        }
        let n = open.suggestions.len().min(8);
        match keystroke.key.as_str() {
            "down" => open.selected = (open.selected + 1) % n,
            "up" => open.selected = (open.selected + n - 1) % n,
            "escape" => self.conversation.completion = None,
            "enter" | "tab" if !keystroke.modifiers.control => {
                let trigger = open.trigger.clone();
                let suggestion = open.suggestions[open.selected].clone();
                self.conversation.completion = None;
                if self.accept_token(&trigger, &suggestion, window, cx) {
                    self.stash_composer(cx);
                    cx.notify();
                    return true;
                }
                let text = self.conversation.composer.read(cx).value().to_string();
                let (value, at) = completion::apply(&text, &trigger, &suggestion);
                let (line, column) = completion::line_column(&value, at);
                self.conversation.completion = None;
                self.conversation.composer.update(cx, |state, cx| {
                    state.set_value(value, window, cx);
                    state.set_cursor_position(Position::new(line, column), window, cx);
                });
                self.stash_composer(cx);
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    pub(super) fn picker_shortcut(
        &mut self,
        command: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.conversation.picker_open {
            return false;
        }
        let jump = command
            .strip_prefix("modelPicker.jump.")
            .and_then(|index| index.parse::<usize>().ok())
            .filter(|index| (1..=9).contains(index))
            .map(|index| index - 1);
        let direction = match command {
            "modelPicker.previousProvider" => Some(false),
            "modelPicker.nextProvider" => Some(true),
            _ => None,
        };
        if jump.is_none() && direction.is_none() {
            return false;
        }
        let Some((environment, selected, ..)) = self.composer_target() else {
            return true;
        };
        let Some((config, instances)) = self.model_catalogs.get(&environment) else {
            return true;
        };
        let current = self.conversation.picker_instance.as_deref().or_else(|| {
            selected
                .as_ref()
                .and_then(|model| model.get("instanceId")?.as_str())
        });
        let tabs: Vec<&str> = std::iter::once("favorites")
            .chain(
                instances
                    .iter()
                    .map(|instance| instance.instance_id.as_str()),
            )
            .collect();
        let at = tabs
            .iter()
            .position(|tab| Some(*tab) == current)
            .unwrap_or(0);
        if let Some(forward) = direction {
            let at = if forward {
                (at + 1) % tabs.len()
            } else {
                (at + tabs.len() - 1) % tabs.len()
            };
            self.conversation.picker_instance = Some(tabs[at].to_string());
            self.conversation
                .model_picker
                .update(cx, |state, cx| state.set_query("", window, cx));
            cx.notify();
        } else if let Some(index) = jump {
            let query = self
                .conversation
                .model_picker
                .read(cx)
                .query(cx)
                .to_lowercase();
            let favorites = config["settings"]["favorites"].as_array();
            let choice = instances
                .iter()
                .flat_map(|instance| instance.models.iter().map(move |model| (instance, model)))
                .filter(|(instance, model)| {
                    if tabs[at] == "favorites" {
                        favorites.is_some_and(|items| {
                            items.iter().any(|item| {
                                item["provider"] == instance.instance_id
                                    && item["model"] == model.slug
                            })
                        })
                    } else {
                        instance.instance_id == tabs[at]
                    }
                })
                .filter(|(_, model)| model.full_name.to_lowercase().contains(&query))
                .nth(index)
                .map(|(instance, model)| (instance.instance_id.clone(), model.slug.clone()));
            if let Some((instance, model)) = choice {
                self.pick_model(instance, model, cx);
                self.conversation
                    .composer
                    .update(cx, |state, cx| state.focus(window, cx));
            }
        }
        true
    }

    pub(super) fn composer_key(&self) -> Option<String> {
        if self.conversation.draft.is_some() {
            return Some(format!(
                "pane:{}:{}",
                self.conversation.id,
                drafts::NEW_THREAD
            ));
        }
        self.current().map(|(environment, thread_id)| {
            format!(
                "pane:{}:{}",
                self.conversation.id,
                drafts::thread_key(&environment, &thread_id)
            )
        })
    }

    /// Keeps the composer text as the draft of what the composer is for now. Call before the
    /// composer changes what it is for.
    pub(super) fn stash_composer(&mut self, cx: &mut Context<Self>) {
        if let Some(draft) = &self.conversation.draft {
            self.history_drafts.insert(draft.id.clone(), draft.clone());
        }
        if self.conversation.queue_edit.is_some() {
            return;
        }
        let Some(key) = self.composer_key() else {
            return;
        };
        let text = self.conversation.composer.read(cx).value().to_string();
        self.conversation.draft_text = text.clone();
        let attachments_changed = self
            .conversation
            .composer_attachments
            .get(&key)
            .map(Vec::as_slice)
            .unwrap_or_default()
            != self.conversation.attachments.as_slice();
        self.conversation.attachments_dirty |= attachments_changed;
        if self.conversation.attachments.is_empty() {
            self.conversation.composer_attachments.remove(&key);
        } else {
            self.conversation
                .composer_attachments
                .insert(key.clone(), self.conversation.attachments.clone());
        }
        if self.drafts.set(&key, &text) || attachments_changed {
            self.schedule_drafts_save(cx);
        }
    }

    /// Puts the draft of what the composer is for now into it, at the next render.
    pub(super) fn load_composer(&mut self) {
        self.conversation.picker_open = false;
        self.abandon_queue_edit();
        self.conversation.attachments = self
            .composer_key()
            .and_then(|key| self.conversation.composer_attachments.get(&key))
            .cloned()
            .unwrap_or_default();
        self.conversation.prompts.history = None;
        self.conversation.queue_expanded = false;
        self.conversation.prompts.open = false;
        self.conversation.hovered_message = None;
        let text = self
            .composer_key()
            .and_then(|key| self.drafts.get(&key).map(str::to_string))
            .unwrap_or_default();
        self.conversation.pending_composer = Some(text);
    }

    fn schedule_drafts_save(&mut self, cx: &mut Context<Self>) {
        if self.drafts_save_scheduled {
            return;
        }
        self.drafts_save_scheduled = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let Ok(snapshots) = this.update(cx, |root, _| {
                std::iter::once(&mut root.conversation)
                    .chain(root.panes.values_mut())
                    .filter(|pane| pane.attachments_dirty)
                    .map(|pane| {
                        pane.attachments_dirty = false;
                        (
                            pane.id,
                            pane.attachment_writer.request(
                                composer_images::attachment_drafts_path(&root.drafts_path, pane.id),
                                pane.composer_attachments.clone(),
                            ),
                        )
                    })
                    .collect::<Vec<_>>()
            }) else {
                return;
            };
            let failures = cx
                .background_executor()
                .spawn(async move {
                    snapshots
                        .into_iter()
                        .filter_map(|(id, request)| {
                            request.save().err().map(|error| {
                                (id, format!("Saving draft attachments failed: {error:#}"))
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |root, cx| {
                root.drafts_save_scheduled = false;
                root.save_conversations();
                let failed = !failures.is_empty();
                for (id, message) in failures {
                    root.with_conversation(id, |root| {
                        root.conversation.message = Some(message);
                        root.conversation.attachments_dirty = true;
                    });
                    cx.notify();
                }
                if let Err(error) = root.drafts.save(&root.drafts_path) {
                    root.conversation.message = Some(format!("Saving drafts failed: {error:#}"));
                    cx.notify();
                }
                if !failed
                    && std::iter::once(&root.conversation)
                        .chain(root.panes.values())
                        .any(|pane| pane.attachments_dirty)
                {
                    root.schedule_drafts_save(cx);
                }
            });
        })
        .detach();
    }

    pub(super) fn composer_composing(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.conversation.composer.update(cx, |state, cx| {
            state.marked_text_range(window, cx).is_some()
        })
    }

    pub(super) fn composer_delivery(&self, alternate: bool) -> Delivery {
        let running = self.open_entry().is_some_and(|entry| {
            matches!(
                entry.status,
                ThreadStatus::Working | ThreadStatus::Input | ThreadStatus::Approval
            )
        });
        let approval = self
            .open_entry()
            .is_some_and(|entry| entry.status == ThreadStatus::Approval);
        let compacting = self.conversation.open.as_ref().is_some_and(|open| {
            let state = open.watch.state();
            let state = state.lock().expect("thread state");
            state
                .projection
                .get("turnItems")
                .and_then(Value::as_array)
                .is_some_and(|items| {
                    items
                        .iter()
                        .any(|item| item["type"] == "compaction" && item["status"] == "running")
                })
        });
        delivery(
            running,
            alternate,
            approval || self.settings.steer_follow_ups,
            !compacting,
        )
    }

    pub(super) fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.send_intent(SubmissionIntent::Foreground, window, cx);
    }

    pub(super) fn send_intent(
        &mut self,
        intent: SubmissionIntent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composer_composing(window, cx) {
            return;
        }
        if self.conversation.queue_edit.is_some() {
            self.save_queue_edit(window, cx);
            return;
        }
        if self.conversation.draft.is_some() {
            self.launch(intent, window, cx);
            return;
        }
        if let Some((view, request_id)) = self.pending_question() {
            self.submit_answers(view, request_id, window, cx);
            return;
        }
        let Some((environment, thread_id)) = self.current() else {
            return;
        };
        let text = self
            .conversation
            .composer
            .read(cx)
            .value()
            .trim()
            .to_string();
        if text.is_empty() && self.conversation.attachments.is_empty() {
            self.resume_composer(window, cx);
            return;
        }
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let background_thread = thread_id.clone();
        let delivery = self.composer_delivery(intent == SubmissionIntent::Alternate);
        let approvals: Vec<String> = self
            .conversation
            .rows
            .iter()
            .filter_map(|row| match &row.pending {
                Some(Pending::Approval { request_id, .. }) => Some(request_id.clone()),
                _ => None,
            })
            .collect();
        let Some(origin) = self.composer_key() else {
            return;
        };
        if !self.conversation.composer_sends.insert(origin.clone()) {
            return;
        }
        let runtime = self.runtime.clone();
        let pending = std::mem::take(&mut self.conversation.attachments);
        self.conversation
            .composer
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.stash_composer(cx);
        self.spawn_pane_in(window, cx, async move |this, cx| {
            let sent = on_runtime(&runtime, {
                let text = text.clone();
                let pending = pending.clone();
                async move {
                    let attachments = upload_all(&connection, &pending).await?;
                    for request_id in approvals {
                        connection
                            .respond(&thread_id, &request_id, "decline")
                            .await?;
                    }
                    connection
                        .send_message(&thread_id, &text, attachments, delivery)
                        .await
                }
            })
            .await;
            let _ = this.update_in(cx, |root, window, cx| {
                root.conversation.composer_sends.remove(&origin);
                root.conversation.message =
                    sent.as_ref().err().map(|e| format!("Send failed: {e:#}"));
                if sent.is_ok()
                    && intent == SubmissionIntent::Background
                    && root.composer_key().as_ref() == Some(&origin)
                {
                    root.new_thread(window, cx);
                    root.background_started(environment.clone(), background_thread, window, cx);
                }
                if sent.is_err() {
                    root.conversation
                        .composer
                        .update(cx, |state, cx| state.set_value(text, window, cx));
                    root.conversation.attachments = pending;
                    root.stash_composer(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn stop(&mut self, cx: &mut Context<Self>) {
        let Some((environment, thread_id)) = self.current() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let runtime = self.runtime.clone();
        self.spawn_pane(cx, async move |this, cx| {
            let stopped = on_runtime(
                &runtime,
                async move { connection.stop_thread(&thread_id).await },
            )
            .await;
            let _ = this.update(cx, |root, cx| {
                root.conversation.message = stopped.err().map(|e| format!("Stop failed: {e:#}"));
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn project_groups(&self) -> Vec<ProjectGroup> {
        let shells: Vec<_> = self.connections.iter().map(|c| c.shell()).collect();
        let guards: Vec<_> = shells
            .iter()
            .map(|s| s.lock().expect("shell lock"))
            .collect();
        projects::groups(
            self.connections
                .iter()
                .zip(&guards)
                .map(|(c, shell)| (c.label(), &**shell)),
        )
    }

    /// The environment and project of the new thread.
    pub(super) fn draft_project(&self) -> Option<(String, String)> {
        let draft = self.conversation.draft.as_ref()?;
        let groups = self.project_groups();
        let group = groups
            .iter()
            .find(|g| Some(&g.key) == draft.group.as_ref())?;
        let member = draft
            .environment
            .as_ref()
            .and_then(|environment| {
                group
                    .members
                    .iter()
                    .find(|member| &member.environment == environment)
            })
            .unwrap_or_else(|| group.target())
            .clone();
        Some((member.environment, member.project_id))
    }

    pub(super) fn draft_target(&self) -> Option<(String, String, Option<Value>, String)> {
        let draft = self.conversation.draft.as_ref()?;
        let (environment, project_id) = self.draft_project()?;
        let project = self.project_value(&environment, &project_id)?;
        let config = self.config_of(&environment);
        let defaults = models::new_thread(config.as_ref(), &project, draft.carry.as_ref());
        let model = draft
            .model
            .clone()
            .or_else(|| defaults.as_ref().map(|(m, _)| m.clone()))
            .and_then(|selected| {
                config.as_ref().and_then(|config| {
                    models::runnable_selection(
                        &models::project_config(config, &project_id),
                        &selected,
                    )
                })
            });
        let runtime_mode = draft
            .runtime_mode
            .clone()
            .or_else(|| defaults.map(|(_, r)| r))
            .unwrap_or_else(|| "full-access".into());
        Some((environment, project_id, model, runtime_mode))
    }

    pub(super) fn new_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.conversation.draft.is_none() {
            let groups = self.project_groups();
            let group = self
                .open_entry()
                .and_then(|entry| {
                    groups
                        .iter()
                        .find(|group| group.contains(&entry.environment, &entry.project_id))
                })
                .or_else(|| groups.first());
            let saved = self
                .history_drafts
                .values()
                .find(|draft| draft.group.as_deref() == group.map(|group| group.key.as_str()))
                .cloned();
            if let Some(draft) = saved {
                self.stash_composer(cx);
                self.history
                    .visit(crate::navigation::Target::Draft(draft.id.clone()));
                self.conversation.draft = Some(draft);
                self.pr_state.page = false;
                self.close_thread(cx);
                self.load_composer();
                cx.notify();
            } else {
                self.begin_new_thread(cx);
            }
        }
        self.conversation
            .composer
            .update(cx, |state, cx| state.focus(window, cx));
    }

    pub(super) fn begin_new_thread(&mut self, cx: &mut Context<Self>) {
        self.begin_new_thread_with_history(false, cx);
    }

    pub(super) fn begin_new_thread_with_history(&mut self, replace: bool, cx: &mut Context<Self>) {
        self.stash_composer(cx);
        let groups = self.project_groups();
        let from_open = self.open_entry().and_then(|entry| {
            groups
                .iter()
                .find(|g| g.contains(&entry.environment, &entry.project_id))
        });
        let group = from_open.or(groups.first()).map(|g| g.key.clone());
        let carry = self
            .current()
            .and_then(|(env, id)| self.thread_value(&env, &id))
            .and_then(|t| t.get("modelSelection").cloned())
            .filter(|m| !m.is_null());
        self.conversation.draft = Some(NewThreadDraft {
            id: uuid::Uuid::new_v4().to_string(),
            environment: None,
            workspace_strategy: json!({"type":"root"}),
            group,
            carry,
            model: None,
            runtime_mode: None,
            interaction_mode: "default".into(),
        });
        let target =
            crate::navigation::Target::Draft(self.conversation.draft.as_ref().unwrap().id.clone());
        if replace {
            self.history.replace(target);
        } else {
            self.history.visit(target);
        }
        self.conversation.message = None;
        self.conversation.picker_open = false;
        self.conversation.branch_anchor = None;
        self.conversation.branch_request += 1;
        self.close_thread(cx);
        self.load_composer();
        cx.notify();
    }

    pub(super) fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.conversation.queue_edit.is_some() {
            self.cancel_queue_edit(window, cx);
            return;
        }
        if self.conversation.branch_anchor.take().is_some() {
            self.conversation
                .composer
                .update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
            return;
        }
        if self.conversation.permissions_menu.take().is_some() {
            self.conversation.permissions_subscription = None;
            if self.conversation.picker_open {
                self.conversation
                    .model_picker
                    .update(cx, |state, cx| state.focus(window, cx));
            } else {
                self.conversation
                    .composer
                    .update(cx, |state, cx| state.focus(window, cx));
            }
            cx.notify();
            return;
        }
        if self.conversation.picker_open {
            self.conversation.picker_open = false;
            self.conversation
                .composer
                .update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
        }
    }

    fn launch(&mut self, intent: SubmissionIntent, window: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .conversation
            .composer
            .read(cx)
            .value()
            .trim()
            .to_string();
        if text.is_empty() && self.conversation.attachments.is_empty() {
            return;
        }
        let Some((environment, project_id, model, runtime_mode)) = self.draft_target() else {
            self.conversation.message = Some("Pick a project first.".into());
            cx.notify();
            return;
        };
        let Some(model) = model else {
            self.conversation.message =
                Some("No model is available on this host. Pick one first.".into());
            cx.notify();
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let interaction_mode = self
            .conversation
            .draft
            .as_ref()
            .map(|d| d.interaction_mode.clone())
            .unwrap_or_else(|| "default".into());
        let workspace_strategy = self
            .conversation
            .draft
            .as_ref()
            .map(|draft| draft.workspace_strategy.clone())
            .unwrap_or_else(|| json!({"type":"root"}));
        let thread_id = self
            .conversation
            .draft
            .as_ref()
            .map(|draft| draft.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let promoting = (environment.clone(), thread_id.clone());
        if !self.promoting.insert(promoting.clone()) {
            return;
        }
        self.rebuild_entries();
        self.schedule_wake(window, cx);
        let title = projects::title_seed(&text);
        self.conversation
            .composer
            .update(cx, |state, cx| state.set_value("", window, cx));
        let pending = std::mem::take(&mut self.conversation.attachments);
        self.stash_composer(cx);
        let runtime = self.runtime.clone();
        let root = cx.entity().downgrade();
        self.spawn_pane_in(window, cx, async move |this, cx| {
            let launched = on_runtime(&runtime, {
                let text = text.clone();
                let pending = pending.clone();
                async move {
                    let attachments = upload_all(&connection, &pending).await?;
                    connection
                        .launch_thread(Launch {
                            thread_id: &thread_id,
                            project_id: &project_id,
                            title: &title,
                            workspace_strategy,
                            model,
                            runtime_mode: &runtime_mode,
                            interaction_mode: &interaction_mode,
                            attachments,
                            text: &text,
                        })
                        .await
                }
            })
            .await;
            let _ = root.update_in(cx, |root, window, cx| {
                root.promoting.remove(&promoting);
                root.rebuild_entries();
                root.schedule_wake(window, cx);
                cx.notify();
            });
            let _ = this.update_in(cx, |root, window, cx| match launched {
                Ok(thread_id) => {
                    if root.conversation.draft.as_ref().map(|draft| &draft.id) != Some(&promoting.1)
                    {
                        return;
                    }
                    if intent == SubmissionIntent::Background {
                        root.new_thread(window, cx);
                        root.background_started(environment, thread_id, window, cx);
                    } else {
                        root.open_thread(environment, thread_id, cx);
                    }
                }
                Err(error) => {
                    root.conversation.message = Some(format!("New thread failed: {error:#}"));
                    root.conversation
                        .composer
                        .update(cx, |state, cx| state.set_value(text, window, cx));
                    root.conversation.attachments = pending;
                    root.stash_composer(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The host, model, runtime mode and interaction mode the composer acts on: the open thread's,
    /// or the draft's.
    pub(super) fn composer_target(&self) -> Option<(String, Option<Value>, String, String)> {
        if let Some(draft) = &self.conversation.draft {
            let (environment, _, model, runtime_mode) = self.draft_target()?;
            return Some((
                environment,
                model,
                runtime_mode,
                draft.interaction_mode.clone(),
            ));
        }
        let (environment, thread_id) = self.current()?;
        let thread = self.thread_value(&environment, &thread_id)?;
        let get = |key: &str| {
            thread
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let model = thread
            .get("modelSelection")
            .cloned()
            .filter(|m| !m.is_null())
            .or_else(|| {
                let project = self.project_value(&environment, &get("projectId"))?;
                models::new_thread(self.config_of(&environment).as_ref(), &project, None)
                    .map(|(model, _)| model)
            })
            .and_then(|selected| {
                self.config_of(&environment)
                    .and_then(|config| models::effective_selection(&config, &selected))
            });
        Some((
            environment,
            model,
            get("runtimeMode"),
            get("interactionMode"),
        ))
    }

    pub(super) fn dispatch_on_open(
        &mut self,
        what: &'static str,
        cx: &mut Context<Self>,
        command: impl FnOnce(Arc<Connection>, String) -> CommandFuture,
    ) {
        let Some((environment, thread_id)) = self.current() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let runtime = self.runtime.clone();
        let future = command(connection, thread_id);
        self.spawn_pane(cx, async move |this, cx| {
            if let Err(error) = on_runtime(&runtime, future).await {
                let _ = this.update(cx, |root, cx| {
                    root.conversation.message = Some(format!("{what} failed: {error:#}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn toggle_model_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.conversation.picker_open = !self.conversation.picker_open;
        if self.conversation.picker_open {
            self.conversation.picker_instance = self
                .composer_target()
                .and_then(|(_, m, _, _)| m)
                .and_then(|m| {
                    m.get("instanceId")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                });
            self.conversation.model_picker.update(cx, |state, cx| {
                state.set_query("", window, cx);
                state.focus(window, cx);
            });
        } else {
            self.conversation
                .composer
                .update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    fn ensure_permissions_menu(
        &mut self,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<permissions::Menu>> {
        if let Some(menu) = &self.conversation.permissions_menu {
            return Some(menu.clone());
        }
        let (environment, model, current, _) = self.composer_target()?;
        let instance_id = model.as_ref()?.get("instanceId")?.as_str()?;
        let modes = self
            .model_catalogs
            .get(&environment)?
            .1
            .iter()
            .find(|instance| instance.instance_id == instance_id)?
            .runtime_modes
            .clone();
        let menu = cx.new(|cx| permissions::Menu::new(modes, current, keyboard, window, cx));
        let pane_id = self.conversation.id;
        self.conversation.permissions_subscription = Some(cx.subscribe_in(
            &menu,
            window,
            move |root, _, event: &permissions::Event, window, cx| {
                root.with_conversation(pane_id, |root| {
                    if let permissions::Event::Select(mode) = event
                        && root
                            .composer_target()
                            .is_some_and(|(_, _, current, _)| current != *mode)
                    {
                        root.set_runtime_mode(mode.clone(), cx);
                    }
                    root.conversation.permissions_menu = None;
                    root.conversation.permissions_subscription = None;
                    if root.conversation.picker_open {
                        root.conversation
                            .model_picker
                            .update(cx, |state, cx| state.focus(window, cx));
                    } else {
                        root.conversation
                            .composer
                            .update(cx, |state, cx| state.focus(window, cx));
                    }
                    cx.notify();
                });
            },
        ));
        self.conversation.permissions_menu = Some(menu.clone());
        let focus = menu.focus_handle(cx);
        window.on_next_frame(move |window, cx| focus.focus(window, cx));
        Some(menu)
    }

    pub(super) fn open_permissions(
        &mut self,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composer_target().is_none() {
            return;
        }
        if self.conversation.permissions_menu.take().is_some() {
            self.conversation.permissions_subscription = None;
            if self.conversation.picker_open {
                self.conversation
                    .model_picker
                    .update(cx, |state, cx| state.focus(window, cx));
            } else {
                self.conversation
                    .composer
                    .update(cx, |state, cx| state.focus(window, cx));
            }
        } else {
            if keyboard && !self.conversation.picker_open {
                self.toggle_model_picker(window, cx);
            }
            self.ensure_permissions_menu(keyboard, window, cx);
        }
        cx.notify();
    }

    fn choose_attachments(&mut self, cx: &mut Context<Self>) {
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach files".into()),
        });
        self.spawn_pane(cx, async move |this, cx| {
            let result = selected.await;
            let _ = this.update(cx, |root, cx| {
                match result {
                    Ok(Ok(Some(paths))) => {
                        root.attach_paths(&ExternalPaths(paths.into_iter().collect()), cx)
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => {
                        root.conversation.message =
                            Some(format!("Choosing attachments failed: {error:#}"))
                    }
                    Err(error) => {
                        root.conversation.message =
                            Some(format!("Choosing attachments failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_runtime_mode(&mut self, mode: String, cx: &mut Context<Self>) {
        if let Some(draft) = self.conversation.draft.as_mut() {
            draft.runtime_mode = Some(mode);
            cx.notify();
        } else {
            self.dispatch_on_open("Runtime mode change", cx, move |connection, thread_id| {
                Box::pin(async move { connection.set_runtime_mode(&thread_id, &mode).await })
            });
        }
    }

    pub(super) fn set_model_option(&mut self, id: &str, value: Value, cx: &mut Context<Self>) {
        let Some((_, Some(current), _, _)) = self.composer_target() else {
            return;
        };
        let Some(chosen) = models::with_option(&current, id, value) else {
            self.conversation.message = Some("The model's option selection is malformed".into());
            cx.notify();
            return;
        };
        if let Some(draft) = self.conversation.draft.as_mut() {
            draft.model = Some(chosen);
            cx.notify();
        } else {
            self.dispatch_on_open("Model option change", cx, move |connection, thread_id| {
                Box::pin(async move { connection.set_model(&thread_id, chosen, false).await })
            });
        }
    }

    pub(super) fn pick_model(&mut self, instance_id: String, slug: String, cx: &mut Context<Self>) {
        self.conversation.picker_open = false;
        let chosen = models::selection(&instance_id, &slug);
        if let Some(draft) = self.conversation.draft.as_mut() {
            draft.model = Some(chosen);
            cx.notify();
            return;
        }
        let switch = self
            .composer_target()
            .and_then(|(_, model, _, _)| model)
            .is_some_and(|m| m.get("instanceId").and_then(Value::as_str) != Some(&instance_id));
        self.dispatch_on_open("Model change", cx, move |connection, thread_id| {
            Box::pin(async move { connection.set_model(&thread_id, chosen, switch).await })
        });
        cx.notify();
    }

    pub(super) fn cycle_runtime_mode(&mut self, cx: &mut Context<Self>) {
        let Some((environment, model, current, _)) = self.composer_target() else {
            return;
        };
        let config = self.config_of(&environment);
        let instance_id = model
            .as_ref()
            .and_then(|m| m.get("instanceId").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string();
        let modes: Vec<String> = config
            .as_ref()
            .and_then(|c| {
                models::instances(c)
                    .into_iter()
                    .find(|i| i.instance_id == instance_id)
            })
            .map(|i| i.runtime_modes)
            .unwrap_or_else(|| {
                models::RUNTIME_MODES
                    .iter()
                    .map(|m| m.to_string())
                    .collect()
            });
        let next = modes
            .iter()
            .position(|m| *m == current)
            .map_or(0, |i| (i + 1) % modes.len());
        let Some(next) = modes.get(next).cloned() else {
            return;
        };
        if let Some(draft) = self.conversation.draft.as_mut() {
            draft.runtime_mode = Some(next);
            cx.notify();
            return;
        }
        self.dispatch_on_open("Runtime mode change", cx, move |connection, thread_id| {
            Box::pin(async move { connection.set_runtime_mode(&thread_id, &next).await })
        });
    }

    fn toggle_plan(&mut self, cx: &mut Context<Self>) {
        let Some((_, _, _, current)) = self.composer_target() else {
            return;
        };
        let next = if current == "plan" { "default" } else { "plan" };
        if let Some(draft) = self.conversation.draft.as_mut() {
            draft.interaction_mode = next.into();
            cx.notify();
            return;
        }
        self.dispatch_on_open("Plan mode change", cx, move |connection, thread_id| {
            Box::pin(async move { connection.set_interaction_mode(&thread_id, next).await })
        });
    }

    pub(super) fn render_queue(&self, cx: &Context<Self>) -> Option<AnyElement> {
        static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *TRACE.get_or_init(|| std::env::var_os("T3CODE_GPUI_TRACE_CONVERSATIONS").is_some()) {
            eprintln!("T3CODE_GPUI_QUEUE_RENDER {}", self.conversation.id);
        }
        let count = self.conversation.queued.len();
        if count == 0 {
            return None;
        }
        let theme = cx.theme();
        let visible = if count > 3 && !self.conversation.queue_expanded {
            2
        } else {
            count
        };
        Some(
            h_flex()
                .w_full()
                .px_4()
                .justify_center()
                .child(
                    v_flex()
                        .id("queued-messages")
                        .role(accesskit::Role::Region)
                        .aria_label(format!(
                            "{count} queued message{}",
                            if count == 1 { "" } else { "s" }
                        ))
                        .w_full()
                        .max_w(px(736.))
                        .items_end()
                        .gap_3()
                        .pb_4()
                        .child(
                            div()
                                .id("queue-live-summary")
                                .role(accesskit::Role::Status)
                                .absolute()
                                .child(
                                    div()
                                        .id("queue-live-count")
                                        .role(accesskit::Role::Label)
                                        .aria_value(count.to_string()),
                                )
                                .child(
                                    div()
                                        .id("queue-live-label")
                                        .role(accesskit::Role::Label)
                                        .aria_value("queued"),
                                ),
                        )
                        .children(
                            self.conversation
                                .queued
                                .iter()
                                .take(visible)
                                .enumerate()
                                .map(|(i, queued)| {
                                    let steer_run = queued.run_id.clone();
                                    let cancel_run = queued.run_id.clone();
                                    let edit_run = queued.run_id.clone();
                                    let editing = self
                                        .conversation
                                        .queue_edit
                                        .as_ref()
                                        .is_some_and(|edit| edit.run_id == queued.run_id);
                                    v_flex()
                                        .id(("queued-message", i))
                                        .group("queued-message")
                                        .role(accesskit::Role::ListItem)
                                        .aria_label(format!(
                                            "Queued message {} of {count}, not sent",
                                            i + 1
                                        ))
                                        .w_full()
                                        .items_end()
                                        .gap_1()
                                        .child(
                                            div()
                                                .max_w(relative(0.8))
                                                .px_3()
                                                .py_2()
                                                .rounded(px(16.))
                                                .border_1()
                                                .border_dashed()
                                                .border_color(if editing {
                                                    theme.primary.opacity(0.6)
                                                } else {
                                                    theme.muted_foreground.opacity(0.4)
                                                })
                                                .text_sm()
                                                .text_color(if editing {
                                                    theme.foreground
                                                } else {
                                                    theme.muted_foreground
                                                })
                                                .child(text!(
                                                    id = ("queued-text", i),
                                                    queued.text.clone()
                                                )),
                                        )
                                        .child(
                                            h_flex()
                                                .gap_0p5()
                                                .pr_1()
                                                .text_xs()
                                                .text_color(theme.muted_foreground)
                                                .when_some(
                                                    queued.created_at.as_deref().map(|time| {
                                                        crate::timestamp::display(time, cx)
                                                    }),
                                                    |row, (timestamp, tooltip)| {
                                                        row.child(
                                                            div()
                                                                .id(("queued-timestamp", i))
                                                                .px_1()
                                                                .mr_1()
                                                                .opacity(0.)
                                                                .group_hover(
                                                                    "queued-message",
                                                                    |time| time.opacity(1.),
                                                                )
                                                                .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx))
                                                                .child(text!(
                                                                    id = (
                                                                        "queued-timestamp-label",
                                                                        i
                                                                    ),
                                                                    timestamp
                                                                )),
                                                        )
                                                    },
                                                )
                                                .when(!editing, |row| {
                                                    row.child(
                                                        Button::new(("queue-steer", i))
                                                            .label("Send now")
                                                            .icon(IconName::ArrowUp)
                                                            .compact()
                                                            .ghost()
                                                            .text_size(px(12.))
                                                            .on_click(self.pane_listener(
                                                                cx,
                                                                move |root, _, _, cx| {
                                                                    root.steer_queued(
                                                                        steer_run.clone(),
                                                                        cx,
                                                                    )
                                                                },
                                                            )),
                                                    )
                                                })
                                                .when(editing, |row| {
                                                    row.child(text!(
                                                        id = ("queue-editing-label", i),
                                                        "Editing in the composer"
                                                    ))
                                                })
                                                .child(
                                                    Button::new(("queue-edit", i))
                                                        .when(!editing, |button| {
                                                            button.icon(
                                                                gpui_kit::assets::IconName::Pencil,
                                                            )
                                                        })
                                                        .when(editing, |button| {
                                                            button.child(text!(
                                                                id = ("queue-cancel-label", i),
                                                                "Cancel"
                                                            ))
                                                        })
                                                        .ghost()
                                                        .compact()
                                                        .text_size(px(12.))
                                                        .accessibility_label(if editing {
                                                            "Cancel editing queued message"
                                                        } else {
                                                            "Edit queued message"
                                                        })
                                                        .on_click(self.pane_listener(
                                                            cx,
                                                            move |root, _, window, cx| {
                                                                if editing {
                                                                    root.cancel_queue_edit(
                                                                        window, cx,
                                                                    );
                                                                } else {
                                                                    root.edit_queued(
                                                                        edit_run.clone(),
                                                                        window,
                                                                        cx,
                                                                    );
                                                                }
                                                            },
                                                        )),
                                                )
                                                .when(!editing, |row| {
                                                    row.child(
                                                        Button::new(("queue-cancel", i))
                                                            .icon(gpui_kit::assets::IconName::X)
                                                            .accessibility_label(
                                                                "Remove queued message",
                                                            )
                                                            .compact()
                                                            .ghost()
                                                            .on_click(self.pane_listener(
                                                                cx,
                                                                move |root, _, _, cx| {
                                                                    root.cancel_queued(
                                                                        cancel_run.clone(),
                                                                        cx,
                                                                    )
                                                                },
                                                            )),
                                                    )
                                                }),
                                        )
                                }),
                        )
                        .when(count > 3, |view| {
                            view.child(
                                Button::new("queue-expand")
                                    .label(if self.conversation.queue_expanded {
                                        "Show fewer".into()
                                    } else {
                                        format!("Show all {count} queued")
                                    })
                                    .ghost()
                                    .compact()
                                    .text_size(px(12.))
                                    .on_click(self.pane_listener(cx, |root, _, _, cx| {
                                        root.conversation.queue_expanded =
                                            !root.conversation.queue_expanded;
                                        cx.notify();
                                    })),
                            )
                        }),
                )
                .into_any_element(),
        )
    }

    pub(super) fn steer_queued(&mut self, run_id: String, cx: &mut Context<Self>) {
        let Some((environment, thread_id)) = self.current() else {
            return;
        };
        let Some(active) = self.thread_value(&environment, &thread_id).and_then(|t| {
            t.get("activeRunId")
                .and_then(Value::as_str)
                .map(str::to_string)
        }) else {
            self.dispatch_on_open("Starting queued message",cx,move |connection,thread_id| Box::pin(async move {
                connection.dispatch(json!({"type":"queued-run.start","threadId":thread_id,"runId":run_id})).await
            }));
            return;
        };
        self.dispatch_on_open("Steering", cx, move |connection, thread_id| {
            Box::pin(async move { connection.steer_queued(&thread_id, &run_id, &active).await })
        });
    }

    fn cancel_queued(&mut self, run_id: String, cx: &mut Context<Self>) {
        self.dispatch_on_open(
            "Cancelling the queued message",
            cx,
            move |connection, thread_id| {
                Box::pin(async move { connection.cancel_queued(&thread_id, &run_id).await })
            },
        );
    }

    pub(super) fn render_message(&self, cx: &App) -> Option<Div> {
        self.conversation.message.clone().map(|message| {
            div()
                .px_4()
                .py_1()
                .text_sm()
                .text_color(cx.theme().danger)
                .child(message)
        })
    }

    fn composer_button(&self, cx: &Context<Self>) -> Button {
        if let Some(edit) = &self.conversation.queue_edit {
            return Button::new("save-queued-message")
                .icon(IconName::Check)
                .size(px(32.))
                .rounded(px(16.))
                .accessibility_label("Update queued message")
                .tooltip("Update queued message")
                .disabled(edit.saving)
                .on_click(
                    self.pane_listener(cx, |root, _, window, cx| root.save_queue_edit(window, cx)),
                );
        }
        if self.conversation.draft.is_some() {
            return Button::new("launch")
                .icon(IconName::ArrowUp)
                .size(px(32.))
                .rounded(px(16.))
                .disabled(
                    self.conversation
                        .composer
                        .read(cx)
                        .value()
                        .trim()
                        .is_empty()
                        && self.conversation.attachments.is_empty(),
                )
                .when(
                    self.conversation
                        .composer
                        .read(cx)
                        .value()
                        .trim()
                        .is_empty()
                        && self.conversation.attachments.is_empty(),
                    |button| {
                        button
                            .bg(cx.theme().foreground.opacity(0.12))
                            .text_color(cx.theme().muted_foreground)
                    },
                )
                .tooltip("Send")
                .accessibility_label("Submit message")
                .primary()
                .on_click(self.pane_listener(cx, |root, _, window, cx| {
                    root.launch(SubmissionIntent::Foreground, window, cx)
                }));
        }
        let working = self.open_entry().is_some_and(|entry| {
            matches!(
                entry.status,
                ThreadStatus::Working | ThreadStatus::Input | ThreadStatus::Approval
            )
        });
        if self.pending_question().is_some() {
            Button::new("answer")
                .label("Answer")
                .primary()
                .on_click(self.pane_listener(cx, |root, _, window, cx| {
                    let alternate = window.modifiers().control || window.modifiers().platform;
                    root.send_intent(
                        if alternate {
                            SubmissionIntent::Alternate
                        } else {
                            SubmissionIntent::Foreground
                        },
                        window,
                        cx,
                    );
                }))
        } else if working
            && self
                .conversation
                .composer
                .read(cx)
                .value()
                .trim()
                .is_empty()
            && self.conversation.attachments.is_empty()
        {
            Button::new("stop")
                .icon(Icon::empty().path("icons/t3-stop.svg"))
                .tooltip("Interrupt")
                .accessibility_label("Stop generation")
                .size(px(32.))
                .rounded(px(16.))
                .danger()
                .on_click(self.pane_listener(cx, |root, _, _, cx| root.stop(cx)))
        } else if self
            .conversation
            .composer
            .read(cx)
            .value()
            .trim()
            .is_empty()
            && self.conversation.attachments.is_empty()
            && self.can_resume_composer()
        {
            Button::new("resume-thread")
                .icon(gpui_kit::assets::IconName::Play)
                .accessibility_label("Resume thread")
                .tooltip("Resume thread")
                .size(px(32.))
                .rounded(px(16.))
                .primary()
                .disabled(
                    self.composer_key()
                        .is_some_and(|key| self.conversation.composer_sends.contains(&key)),
                )
                .on_click(
                    self.pane_listener(cx, |root, _, window, cx| root.resume_composer(window, cx)),
                )
        } else {
            Button::new("send")
                .icon(Icon::empty().path(match self.composer_delivery(false) {
                    Delivery::Start => "icons/t3-submit.svg",
                    Delivery::Steer => "icons/t3-steer.svg",
                    Delivery::Queue => "icons/t3-queue.svg",
                }))
                .disabled(
                    self.conversation
                        .composer
                        .read(cx)
                        .value()
                        .trim()
                        .is_empty()
                        && self.conversation.attachments.is_empty(),
                )
                .when(
                    self.conversation
                        .composer
                        .read(cx)
                        .value()
                        .trim()
                        .is_empty()
                        && self.conversation.attachments.is_empty(),
                    |button| {
                        button
                            .bg(cx.theme().foreground.opacity(0.12))
                            .text_color(cx.theme().muted_foreground)
                    },
                )
                .tooltip("Send")
                .accessibility_label(match self.composer_delivery(false) {
                    Delivery::Start => "Submit message",
                    Delivery::Steer => "Steer message",
                    Delivery::Queue => "Queue message",
                })
                .size(px(32.))
                .rounded(px(16.))
                .primary()
                .on_click(self.pane_listener(cx, |root, _, window, cx| {
                    let alternate = window.modifiers().control || window.modifiers().platform;
                    root.send_intent(
                        if alternate {
                            SubmissionIntent::Alternate
                        } else {
                            SubmissionIntent::Foreground
                        },
                        window,
                        cx,
                    );
                }))
        }
    }

    fn render_composer_controls(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let target = self.composer_target();
        // Provider catalogs change on config events, not on each keystroke.
        let catalog = target
            .as_ref()
            .and_then(|(env, ..)| self.model_catalogs.get(env));
        let config = catalog.map(|(config, _)| config);
        let instances = catalog
            .map(|(_, instances)| instances.as_slice())
            .unwrap_or_default();
        let current = target.as_ref().and_then(|(_, model, ..)| model.clone());
        let current_instance = current
            .as_ref()
            .and_then(|m| m.get("instanceId").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string();
        let current_slug = current
            .as_ref()
            .and_then(|m| m.get("model").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string();
        let selected_instance = instances.iter().find(|i| i.instance_id == current_instance);
        let selected_model = selected_instance.and_then(|instance| {
            instance
                .models
                .iter()
                .find(|model| model.slug == current_slug)
        });
        let label = selected_model
            .zip(current.as_ref())
            .map(|(model, selection)| models::trigger_label(model, selection))
            .unwrap_or_else(|| "Pick a model".into());
        let toolbar = h_flex()
            .px_2()
            .pb_2()
            .gap_1()
            .w_full()
            .child(
                Button::new("attach-files")
                    .icon(
                        Icon::new(gpui_kit::assets::IconName::Paperclip)
                            .text_color(theme.muted_foreground),
                    )
                    .ghost()
                    .size(px(32.))
                    .tooltip("Attach files")
                    .accessibility_label("Attach files")
                    .on_click(self.pane_listener(cx, |root, _, _, cx| root.choose_attachments(cx))),
            )
            .child(div().flex_1())
            .child(
                Button::new("model")
                    .accessibility_label(
                        selected_model
                            .map(|model| model.full_name.clone())
                            .unwrap_or_else(|| "Pick a model".into()),
                    )
                    .child(text!(id = "model-trigger-label", label))
                    .text_color(theme.foreground.opacity(0.8))
                    .icon(
                        Icon::empty()
                            .path(match selected_instance.map(|i| i.driver.as_str()) {
                                Some("codex") => "icons/t3-openai.svg",
                                Some("claudeAgent") => "icons/t3-claude.svg",
                                _ => "icons/bot.svg",
                            })
                            .text_color(theme.muted_foreground),
                    )
                    .compact()
                    .ghost()
                    .h(px(28.))
                    .text_sm()
                    .tooltip(
                        current
                            .as_ref()
                            .map(|selection| {
                                let mut label = models::label(instances, selection);
                                if let Some(config) = &config {
                                    for (window, percent) in
                                        models::usage(config, &current_instance)
                                    {
                                        label.push_str(&format!("\n{window}: {percent:.0}% used"));
                                    }
                                }
                                label
                            })
                            .unwrap_or_else(|| "Choose model and permissions".into()),
                    )
                    .selected(self.conversation.picker_open)
                    .on_click(self.pane_listener(cx, |root, _, window, cx| {
                        root.toggle_model_picker(window, cx)
                    })),
            );
        let picker = self.conversation.picker_open.then(|| {
            let shown = instances
                .iter()
                .find(|i| Some(&i.instance_id) == self.conversation.picker_instance.as_ref())
                .or(selected_instance);
            let favorites = config.and_then(|config| config["settings"]["favorites"].as_array());
            let favorite_tab = self.conversation.picker_instance.as_deref() == Some("favorites");
            let mut group = CommandGroup::new();
            let mut index = 0usize;
            for instance in instances {
                for model in &instance.models {
                    let favorite = favorites.is_some_and(|favorites| {
                        favorites.iter().any(|favorite| {
                            favorite["provider"] == instance.instance_id
                                && favorite["model"] == model.slug
                        })
                    });
                    if if favorite_tab {
                        !favorite
                    } else {
                        !shown.is_some_and(|shown| shown.instance_id == instance.instance_id)
                    } {
                        continue;
                    }
                    let name = model.full_name.clone();
                    let provider = instance.name.clone();
                    let favorite_label = if favorite {
                        "Remove from favorites"
                    } else {
                        "Add to favorites"
                    };
                    let jump = if index < 9 {
                        format!("Ctrl+{}", index + 1)
                    } else {
                        String::new()
                    };
                    let label = format!("{name} , {provider} {favorite_label} {jump}");
                    let instance_id = instance.instance_id.clone();
                    let slug = model.slug.clone();
                    let root = self.pane_root(cx);
                    let icon = match instance.driver.as_str() {
                        "codex" => "icons/t3-openai.svg",
                        "claudeAgent" => "icons/t3-claude.svg",
                        _ => "icons/bot.svg",
                    };
                    group = group.item(
                        CommandItem::new()
                            .label(name.clone())
                            .accessibility_label(label)
                            .checked(
                                instance.instance_id == current_instance
                                    && model.slug == current_slug,
                            )
                            .action(Box::new(PickModelAction {
                                instance_id: instance_id.clone(),
                                slug: slug.clone(),
                            }))
                            .child(move |_, _| {
                                let root = root.clone();
                                let instance_id = instance_id.clone();
                                let slug = slug.clone();
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(Icon::empty().path(icon).size(px(16.)))
                                    .child(text!(id = ("picker-model-label", index), name.clone()))
                                    .child(
                                        div()
                                            .id(("picker-provider-description", index))
                                            .role(accesskit::Role::Label)
                                            .aria_value(provider.clone()),
                                    )
                                    .child(
                                        div()
                                            .id(("picker-provider-separator", index))
                                            .role(accesskit::Role::Label)
                                            .aria_value(","),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        Button::new(("model-favorite", index))
                                            .icon(gpui_kit::assets::IconName::Star)
                                            .accessibility_label(favorite_label)
                                            .ghost()
                                            .size(px(24.))
                                            .selected(favorite)
                                            .on_click(move |_, _, cx| {
                                                cx.stop_propagation();
                                                let _ = root.update(cx, |root, cx| {
                                                    root.set_model_favorite(
                                                        instance_id.clone(),
                                                        slug.clone(),
                                                        !favorite,
                                                        cx,
                                                    )
                                                });
                                            }),
                                    )
                                    .child(div().text_xs().child(text!(
                                        id = ("picker-model-shortcut", index),
                                        jump.clone()
                                    )))
                            }),
                    );
                    index += 1;
                }
            }
            let interaction_mode = target
                .as_ref()
                .map(|(_, _, _, mode)| mode.as_str())
                .unwrap_or_default();
            let weak = self.pane_root(cx);
            let close = move |window: &mut Window, cx: &mut App| {
                let _ = weak.update(cx, |root, cx| {
                    root.conversation.picker_open = false;
                    root.conversation
                        .composer
                        .update(cx, |state, cx| state.focus(window, cx));
                    cx.notify();
                });
            };
            let cancel = close.clone();
            let reasoning = selected_model.and_then(|model| {
                model.option_descriptors.iter().find(|d| {
                    d.get("role").and_then(Value::as_str) == Some("reasoning")
                        || d.get("id").and_then(Value::as_str) == Some("reasoningEffort")
                })
            });
            let reasoning_column = reasoning.map(|descriptor| {
                let id = descriptor
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let selected = current
                    .as_ref()
                    .and_then(|selection| models::option_value(selection, descriptor));
                let options: Vec<Value> = descriptor["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|option| option["id"].clone())
                    .collect();
                let focused = self
                    .conversation
                    .composer_choices
                    .effort_cursor
                    .as_ref()
                    .filter(|value| options.contains(value))
                    .or(selected.as_ref())
                    .or(options.first());
                v_flex()
                    .id("composer-reasoning")
                    .role(accesskit::Role::RadioGroup)
                    .aria_label(descriptor["label"].as_str().unwrap_or("Effort").to_string())
                    .w(px(192.))
                    .h_full()
                    .border_l_1()
                    .border_color(theme.border.opacity(0.7))
                    .bg(theme.muted.opacity(0.4))
                    .p_2()
                    .child(
                        h_flex()
                            .h(px(34.))
                            .px_1()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(text!(
                                descriptor["label"].as_str().unwrap_or("Effort").to_string()
                            ))
                            .child(div().flex_1())
                            .child(div().text_xs().child(text!(
                                selected_model.expect("reasoning model").name.clone()
                            ))),
                    )
                    .children(
                        descriptor
                            .get("options")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .map(|option| {
                                let value = option.get("id").cloned().unwrap_or(Value::Null);
                                let checked = Some(&value) == selected.as_ref();
                                let is_default = option["isDefault"].as_bool() == Some(true);
                                let label =
                                    option["label"].as_str().unwrap_or_default().to_string();
                                let tracked = Some(&value) == focused;
                                let choices = options.clone();
                                let key_id = id.clone();
                                let key_value = value.clone();
                                let id = id.clone();
                                h_flex()
                                    .id(SharedString::from(format!("reasoning-{value}")))
                                    .role(accesskit::Role::RadioButton)
                                    .aria_label(if is_default {
                                        format!("{label} Default")
                                    } else {
                                        label.clone()
                                    })
                                    .aria_toggled(if checked {
                                        accesskit::Toggled::True
                                    } else {
                                        accesskit::Toggled::False
                                    })
                                    .when(tracked, |row| {
                                        row.track_focus(
                                            &self.conversation.composer_choices.effort_focus,
                                        )
                                    })
                                    .on_key_down(self.pane_listener(
                                        cx,
                                        move |root, event: &KeyDownEvent, window, cx| {
                                            match event.keystroke.key.as_str() {
                                                "up" | "down" => {
                                                    if let Some(index) = choices
                                                        .iter()
                                                        .position(|value| value == &key_value)
                                                    {
                                                        let next = if event.keystroke.key == "down"
                                                        {
                                                            (index + 1) % choices.len()
                                                        } else {
                                                            (index + choices.len() - 1)
                                                                % choices.len()
                                                        };
                                                        root.conversation
                                                            .composer_choices
                                                            .effort_cursor =
                                                            Some(choices[next].clone());
                                                        cx.notify();
                                                    }
                                                }
                                                "enter" | "space" => root.set_model_option(
                                                    &key_id,
                                                    key_value.clone(),
                                                    cx,
                                                ),
                                                "left" => root
                                                    .conversation
                                                    .model_picker
                                                    .update(cx, |state, cx| {
                                                        state.focus(window, cx)
                                                    }),
                                                "escape" => {
                                                    root.conversation.picker_open = false;
                                                    root.conversation
                                                        .composer
                                                        .update(cx, |state, cx| {
                                                            state.focus(window, cx)
                                                        });
                                                    cx.notify();
                                                }
                                                _ => return,
                                            }
                                            cx.stop_propagation();
                                        },
                                    ))
                                    .w_full()
                                    .h(px(34.))
                                    .px_2()
                                    .rounded_sm()
                                    .text_sm()
                                    .cursor_pointer()
                                    .hover(|row| row.bg(theme.list_hover))
                                    .when(checked, |row| row.bg(theme.list_hover))
                                    .child(text!(label))
                                    .child(div().flex_1())
                                    .when(is_default && !checked, |row| {
                                        row.child(
                                            div()
                                                .text_xs()
                                                .text_color(theme.muted_foreground)
                                                .child(text!("Default")),
                                        )
                                    })
                                    .when(checked, |row| {
                                        row.child(Icon::new(IconName::Check).size(px(14.)))
                                    })
                                    .on_click(self.pane_listener(cx, move |root, _, _, cx| {
                                        root.set_model_option(&id, value.clone(), cx)
                                    }))
                            }),
                    )
            });
            v_flex()
                .id("model-picker")
                .role(accesskit::Role::Dialog)
                .absolute()
                .bottom(px(40.))
                .right(px(48.))
                .mb_1()
                .w(px(554.))
                .h(px(392.))
                .rounded_md()
                .border_1()
                .border_color(theme.border)
                .bg(theme.popover)
                .shadow_lg()
                .occlude()
                .child(
                    h_flex()
                        .flex_1()
                        .min_h_0()
                        .items_start()
                        .child(
                            v_flex()
                                .id("model-picker-providers")
                                .role(accesskit::Role::Toolbar)
                                .aria_label("Providers")
                                .w(px(44.))
                                .h_full()
                                .border_r_1()
                                .border_color(theme.border)
                                .py_2()
                                .gap_1()
                                .child(
                                    Button::new("model-picker-favorites")
                                        .icon(gpui_kit::assets::IconName::Star)
                                        .accessibility_label("Favorites")
                                        .ghost()
                                        .size(px(36.))
                                        .selected(favorite_tab)
                                        .on_click(self.pane_listener(cx, |root, _, window, cx| {
                                            root.conversation.picker_instance =
                                                Some("favorites".into());
                                            root.conversation.model_picker.update(
                                                cx,
                                                |state, cx| {
                                                    state.set_query("", window, cx);
                                                    state.focus(window, cx);
                                                },
                                            );
                                            cx.notify();
                                        })),
                                )
                                .children(instances.iter().map(|instance| {
                                    let id = instance.instance_id.clone();
                                    Button::new(SharedString::from(format!("picker-provider-{id}")))
                                        .icon(Icon::empty().path(match instance.driver.as_str() {
                                            "codex" => "icons/t3-openai.svg",
                                            "claudeAgent" => "icons/t3-claude.svg",
                                            _ => "icons/bot.svg",
                                        }))
                                        .ghost()
                                        .size(px(36.))
                                        .tooltip(instance.name.clone())
                                        .accessibility_label(instance.name.clone())
                                        .selected(
                                            !favorite_tab
                                                && shown.is_some_and(|i| i.instance_id == id),
                                        )
                                        .on_click(self.pane_listener(
                                            cx,
                                            move |root, _, window, cx| {
                                                root.conversation.picker_instance =
                                                    Some(id.clone());
                                                root.conversation.model_picker.update(
                                                    cx,
                                                    |state, cx| {
                                                        state.set_query("", window, cx);
                                                        state.focus(window, cx);
                                                    },
                                                );
                                                cx.notify();
                                            },
                                        ))
                                })),
                        )
                        .child(
                            div().flex_1().min_w_0().h_full().child(
                                Command::new(&self.conversation.model_picker)
                                    .placeholder("Search models...")
                                    .max_h(px(300.))
                                    .group(group)
                                    .on_confirm(move |_, window, cx| close(window, cx))
                                    .on_cancel(move |window, cx| cancel(window, cx)),
                            ),
                        )
                        .child(reasoning_column.unwrap_or_else(|| {
                            v_flex()
                                .id("composer-no-effort")
                                .w(px(192.))
                                .h_full()
                                .border_l_1()
                                .border_color(theme.border.opacity(0.7))
                                .p_2()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(
                                    h_flex()
                                        .h(px(34.))
                                        .px_1()
                                        .child(text!("Effort"))
                                        .child(div().flex_1())
                                        .when_some(selected_model, |header, model| {
                                            header.child(
                                                div().text_xs().child(text!(model.name.clone())),
                                            )
                                        }),
                                )
                                .child(
                                    div()
                                        .id("no-effort-levels")
                                        .role(accesskit::Role::Label)
                                        .aria_label("No effort levels")
                                        .h(px(32.))
                                        .px_2()
                                        .child(text!("No effort levels")),
                                )
                        })),
                )
                .child(
                    h_flex()
                        .h(px(44.))
                        .bg(theme.foreground.opacity(0.025))
                        .px_2()
                        .gap_1()
                        .border_t_1()
                        .border_color(theme.foreground.opacity(0.07))
                        .children(
                            selected_model
                                .into_iter()
                                .flat_map(|model| &model.option_descriptors)
                                .filter(|d| {
                                    d.get("role").and_then(Value::as_str) != Some("reasoning")
                                        && d.get("id").and_then(Value::as_str)
                                            != Some("reasoningEffort")
                                })
                                .filter_map(|descriptor| {
                                    let id = descriptor.get("id")?.as_str()?.to_string();
                                    let selected = current.as_ref().and_then(|selection| {
                                        models::option_value(selection, descriptor)
                                    });
                                    if descriptor.get("type").and_then(Value::as_str)
                                        == Some("boolean")
                                    {
                                        let checked =
                                            selected.and_then(|v| v.as_bool()).unwrap_or(false);
                                        return Some(
                                            h_flex().child(
                                                Button::new(SharedString::from(format!(
                                                    "option-{id}"
                                                )))
                                                .label(
                                                    descriptor.get("label")?.as_str()?.to_string(),
                                                )
                                                .ghost()
                                                .compact()
                                                .selected(checked)
                                                .on_click(self.pane_listener(
                                                    cx,
                                                    move |root, _, _, cx| {
                                                        root.set_model_option(
                                                            &id,
                                                            Value::Bool(!checked),
                                                            cx,
                                                        )
                                                    },
                                                )),
                                            ),
                                        );
                                    }
                                    Some(
                                        h_flex()
                                            .rounded(px(6.))
                                            .bg(theme.foreground.opacity(0.05))
                                            .p(px(3.))
                                            .children(
                                                descriptor.get("options")?.as_array()?.iter().map(
                                                    |option| {
                                                        let id = id.clone();
                                                        let value = option
                                                            .get("id")
                                                            .cloned()
                                                            .unwrap_or(Value::Null);
                                                        Button::new(SharedString::from(format!(
                                                            "option-{id}-{value}"
                                                        )))
                                                        .label(
                                                            option
                                                                .get("label")
                                                                .and_then(Value::as_str)
                                                                .unwrap_or_default()
                                                                .to_string(),
                                                        )
                                                        .compact()
                                                        .ghost()
                                                        .text_size(px(12.))
                                                        .h(px(22.))
                                                        .px(px(8.))
                                                        .rounded(px(3.))
                                                        .when(
                                                            Some(&value) == selected.as_ref(),
                                                            |button| {
                                                                button.bg(theme
                                                                    .foreground
                                                                    .opacity(0.09))
                                                            },
                                                        )
                                                        .selected(Some(&value) == selected.as_ref())
                                                        .on_click(self.pane_listener(
                                                            cx,
                                                            move |root, _, _, cx| {
                                                                root.set_model_option(
                                                                    &id,
                                                                    value.clone(),
                                                                    cx,
                                                                )
                                                            },
                                                        ))
                                                    },
                                                ),
                                            ),
                                    )
                                }),
                        )
                        .child(div().flex_1())
                        .when(
                            self.settings.plan_mode_enabled
                                && selected_instance.is_some_and(|i| i.interaction_mode_toggle),
                            |row| {
                                row.child(
                                    Button::new("plan")
                                        .label(if interaction_mode == "plan" {
                                            "Plan"
                                        } else {
                                            "Build"
                                        })
                                        .compact()
                                        .ghost()
                                        .text_size(px(12.))
                                        .selected(interaction_mode == "plan")
                                        .on_click(self.pane_listener(cx, |root, _, _, cx| {
                                            root.toggle_plan(cx)
                                        })),
                                )
                            },
                        )
                        .child(self.render_permissions_control(true, cx)),
                )
        });
        div()
            .relative()
            .w_full()
            .child(
                toolbar
                    .child(
                        Button::new("dictation")
                            .icon(IconName::Mic)
                            .ghost()
                            .size(px(28.))
                            .accessibility_label(
                                if self.conversation.dictation.recording.is_some() {
                                    "Stop dictation"
                                } else {
                                    "Start voice input"
                                },
                            )
                            .tooltip(if self.conversation.dictation.recording.is_some() {
                                "Stop dictation"
                            } else {
                                "Start voice input"
                            })
                            .disabled(self.conversation.dictation.busy)
                            .on_click(self.pane_listener(cx, |root, _, window, cx| {
                                root.toggle_dictation(window, cx)
                            })),
                    )
                    .child(self.composer_button(cx)),
            )
            .children(picker)
            .into_any_element()
    }

    fn render_permissions_control(&self, inside_picker: bool, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let runtime_mode = self
            .composer_target()
            .map(|(_, _, mode, _)| mode)
            .unwrap_or_default();
        let (runtime_label, description, icon) = permissions::presentation(&runtime_mode);
        let trigger = Button::new("permissions")
            .child(
                h_flex()
                    .gap(px(6.))
                    .child(Icon::new(icon).size(px(12.)))
                    .child(div().text_size(px(12.)).child(text!(runtime_label))),
            )
            .accessibility_label(format!("Permissions: {runtime_label}"))
            .tooltip(format!("Permissions: {runtime_label}. {description}"))
            .compact()
            .ghost()
            .h(px(24.))
            .px(px(7.))
            .text_size(px(12.))
            .text_color(theme.muted_foreground)
            .selected(self.conversation.permissions_menu.is_some())
            .on_click(self.pane_listener(cx, |root, _, window, cx| {
                root.open_permissions(false, window, cx)
            }));
        gpui_kit::base::Popup::new("permissions-popover", trigger)
            .anchor(Anchor::BottomRight)
            .offset(px(4.))
            .when_some(
                self.conversation
                    .permissions_menu
                    .clone()
                    .filter(|_| inside_picker == self.conversation.picker_open),
                |popup, menu| popup.content(menu),
            )
            .into_any_element()
    }

    pub(super) fn render_composer(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let suggestions = self
            .conversation
            .completion
            .as_ref()
            .filter(|c| !c.suggestions.is_empty())
            .map(|c| {
                v_flex()
                    .id("composer-completions")
                    .role(accesskit::Role::ListBox)
                    .mx_3()
                    .mt_2()
                    .p_1()
                    .rounded_md()
                    .border_1()
                    .border_color(theme.border)
                    .children(c.suggestions.iter().take(8).enumerate().map(|(i, s)| {
                        let label = if c.trigger.kind == completion::TriggerKind::Path {
                            s.label.trim_start_matches('@').to_string()
                        } else {
                            s.label.clone()
                        };
                        h_flex()
                            .id(("composer-completion", i))
                            .role(accesskit::Role::ListBoxOption)
                            .aria_label(label.clone())
                            .cursor_pointer()
                            .on_click(self.pane_listener(cx, move |root, _, window, cx| {
                                if let Some(completion) = root.conversation.completion.as_mut() {
                                    completion.selected = i;
                                }
                                root.conversation.composer.update(cx, |state, cx| {
                                    state.focus(window, cx);
                                });
                                root.completion_key(
                                    &Keystroke::parse("enter").expect("completion key"),
                                    window,
                                    cx,
                                );
                                cx.stop_propagation();
                            }))
                            .px_2()
                            .py_0p5()
                            .gap_3()
                            .rounded_sm()
                            .when(i == c.selected, |row| row.bg(theme.list_active))
                            .child(div().text_sm().child(text!(label)))
                            .when(c.trigger.kind != completion::TriggerKind::Path, |row| {
                                row.child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(text!(s.detail.clone())),
                                )
                            })
                    }))
            });
        let chips = (!self.conversation.attachments.is_empty()).then(|| {
            h_flex().px_3().pt_2().gap_1().flex_wrap().children(
                self.conversation
                    .attachments
                    .iter()
                    .enumerate()
                    .map(|(i, file)| {
                        Button::new(("attachment", i))
                            .label(format!("{} ×", file.name))
                            .compact()
                            .outline()
                            .tooltip("Remove")
                            .on_click(self.pane_listener(cx, move |root, _, _, cx| {
                                if i < root.conversation.attachments.len() {
                                    root.conversation.attachments.remove(i);
                                    root.stash_composer(cx);
                                }
                                cx.notify();
                            }))
                    }),
            )
        });
        let content = v_flex()
            .id("composer-input")
            .role(accesskit::Role::Region)
            .aria_label("Message input")
            .children(self.render_queue_attachments(cx))
            .children(self.render_force_stop(cx))
            .children(self.render_prompt_stash(false, cx))
            .relative()
            .w_full()
            .max_w(px(736.))
            .when(self.conversation.draft.is_none(), |content| content.mb_5())
            .rounded(px(16.))
            .border_1()
            .border_color(theme.border)
            .bg(crate::appearance::composer(cx))
            .on_drop(
                self.pane_listener(cx, |root, paths: &ExternalPaths, _, cx| {
                    root.attach_paths(paths, cx)
                }),
            )
            .when(
                self.conversation.composer.read(cx).value().is_empty(),
                |content| {
                    content.child(
                        div()
                            .id("composer-placeholder-text")
                            .role(accesskit::Role::Label)
                            .aria_value("Ask anything. @ files, $ skills, / commands")
                            .absolute(),
                    )
                },
            )
            .children(suggestions)
            .children(chips)
            .child(
                h_flex().px_1p5().pt_2().pb_6().gap_2().items_end().child(
                    div()
                        .flex_1()
                        .text_size(px(self.settings.font_size_prompt))
                        .child(
                            Textarea::new(&self.conversation.composer)
                                .aria_label(format!(
                                    "Message {}",
                                    self.open_entry()
                                        .map(|entry| entry.title.as_str())
                                        .unwrap_or("New thread")
                                ))
                                .appearance(false)
                                .bordered(false),
                        ),
                ),
            )
            .child(if self.conversation.picker_open {
                self.conversation
                    .composer_controls
                    .clone()
                    .into_any_element()
            } else {
                self.conversation
                    .composer_controls
                    .clone()
                    .cached(StyleRefinement::default().w_full().h(px(40.)))
                    .into_any_element()
            })
            .into_any_element();
        h_flex()
            .w_full()
            .px_4()
            .justify_center()
            .child(
                v_flex()
                    .relative()
                    .w_full()
                    .max_w(px(736.))
                    .child(content)
                    .children(self.render_prompt_stash(true, cx)),
            )
            .into_any_element()
    }

    pub(super) fn render_draft(
        &self,
        draft: &NewThreadDraft,
        groups: &[ProjectGroup],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = groups
            .iter()
            .find(|group| draft.group.as_ref() == Some(&group.key));
        let scratch = self.is_scratch_draft();
        let title = selected
            .map(|group| group.title.clone())
            .unwrap_or_else(|| "Choose a project".into());
        let title = if scratch { "No project".into() } else { title };
        let heading_label = if scratch {
            "What should we work on?".to_string()
        } else if selected.is_some() {
            format!("What should we build in {title} ?")
        } else {
            format!("{title} to start")
        };
        let choices = groups.to_vec();
        let selected_key = draft.group.clone();
        let focus = self.focus.clone();
        let project = Button::new("draft-project")
            .child(
                div()
                    .text_size(px(if scratch { 14. } else { 30. }))
                    .line_height(px(if scratch { 24. } else { 36. }))
                    .font_weight(FontWeight::MEDIUM)
                    .child(if scratch {
                        div().child(title.clone()).into_any_element()
                    } else {
                        DraftHeadingText(title.clone().into()).into_any_element()
                    }),
            )
            .accessibility_label(title)
            .role(accesskit::Role::ComboBox)
            .ghost()
            .compact()
            .h(px(if scratch { 24. } else { 36. }))
            .px_0()
            .text_size(px(30.))
            .font_weight(FontWeight::MEDIUM)
            .dropdown_menu(move |mut menu, _, _| {
                for group in &choices {
                    menu = menu.menu_with_check(
                        group.title.clone(),
                        selected_key.as_ref() == Some(&group.key),
                        Box::new(PickProjectAction {
                            key: group.key.clone(),
                        }),
                    );
                }
                menu.action_context(focus.clone())
            });
        let mut project = Some(project);
        let heading = h_flex()
            .id("draft-heading")
            .role(accesskit::Role::Heading)
            .aria_label(heading_label)
            .justify_center()
            .items_center()
            .flex_wrap()
            .gap_2()
            .text_size(px(30.))
            .line_height(px(36.))
            .child(if scratch {
                div().child(DraftHeadingText("What should we work on?".into()))
            } else if selected.is_some() {
                h_flex()
                    .gap(px(7.))
                    .child(DraftHeadingText("What should we build".into()))
                    .child(DraftHeadingText("in".into()))
            } else {
                div()
            })
            .when(!scratch, |heading| {
                heading.child(
                    h_flex()
                        .items_center()
                        .child(project.take().expect("project selector"))
                        .child(DraftHeadingText(
                            if selected.is_some() { "?" } else { " to start" }.into(),
                        )),
                )
            });
        let heading = v_flex().items_center().child(heading).when(
            self.can_start_without_project(),
            |heading| {
                heading.child(
                    h_flex()
                        .mt_2()
                        .h(px(24.))
                        .justify_center()
                        .child(if scratch {
                            project
                                .take()
                                .expect("scratch project selector")
                                .into_any_element()
                        } else {
                            Button::new("draft-without-project")
                                .label("or start without a project")
                                .ghost()
                                .compact()
                                .h(px(24.))
                                .text_size(px(14.))
                                .text_color(cx.theme().muted_foreground)
                                .on_click(self.pane_listener(cx, |root, _, window, cx| {
                                    root.new_without_project(window, cx);
                                }))
                                .into_any_element()
                        }),
                )
            },
        );
        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .justify_center()
            .child(
                v_flex()
                    .relative()
                    .when(self.pane_layout.ids().len() == 1, |body| body.top(px(-18.)))
                    .when(
                        !self.conversation.prompts.store.entries.is_empty(),
                        |body| body.pt_8(),
                    )
                    .w_full()
                    .child(
                        div()
                            .absolute()
                            .bottom_full()
                            .w_full()
                            .pb(px(if self.conversation.prompts.store.entries.is_empty() {
                                16.
                            } else {
                                0.
                            }))
                            .px_4()
                            .child(heading),
                    )
                    .children(self.render_message(cx))
                    .child(self.render_composer(cx))
                    .child(
                        h_flex().w_full().justify_center().px_4().child(
                            h_flex()
                                .id("composer-context")
                                .role(accesskit::Role::Region)
                                .aria_label("Message input")
                                .w_full()
                                .max_w(px(736.))
                                .h(px(52.))
                                .px_2()
                                .items_start()
                                .pt_2()
                                .child(self.render_composer_context(cx))
                                .child(div().flex_1())
                                .when(self.conversation.workspace_status.is_repo(), |row| {
                                    row.child(self.render_branch_control("composer-branch", cx))
                                })
                                .child(self.render_permissions_control(false, cx)),
                        ),
                    ),
            )
            .into_any_element()
    }
}

/// Uploads the composer's attachments in order and returns what a message refers to them by.
pub(super) async fn upload_all(
    connection: &Connection,
    pending: &[attachments::Pending],
) -> anyhow::Result<Vec<Value>> {
    let mut uploaded = Vec::with_capacity(pending.len());
    for file in pending {
        uploaded.push(
            connection
                .upload_attachment(&file.name, &file.mime_type, file.bytes.as_ref().clone())
                .await?,
        );
    }
    Ok(uploaded)
}

#[cfg(test)]
mod send_tests {
    use super::delivery;
    use serde_json::Value;
    use t3_client::session::Delivery;
    #[test]
    fn dispatch_modes_match_reference() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/composer-dispatch.json"))
                .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let expected = match case["result"].as_str().unwrap() {
                "auto" => Delivery::Start,
                "steer" => Delivery::Steer,
                "queue" => Delivery::Queue,
                _ => unreachable!(),
            };
            assert_eq!(
                delivery(
                    case["running"].as_bool().unwrap(),
                    case["alternateModifier"].as_bool().unwrap(),
                    case["activeTurnDefault"] == "steer",
                    case["canSteer"].as_bool().unwrap()
                ),
                expected,
                "{case}"
            );
        }
    }
}

#[derive(IntoElement)]
struct DraftHeadingText(SharedString);

impl RenderOnce for DraftHeadingText {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = style.line_height_in_pixels(window.rem_size());
        let mut line = window.text_system().shape_line(
            self.0.clone(),
            font_size,
            &[style.to_run(self.0.len())],
            None,
        );
        let spacing = font_size * -0.025;
        let mut runs = line.runs.clone();
        for run in &mut runs {
            for glyph in &mut run.glyphs {
                glyph.position.x += spacing * self.0[..glyph.index].chars().count() as f32;
            }
        }
        let width = line.width + spacing * self.0.chars().count() as f32;
        *line = Arc::new(LineLayout {
            font_size: line.font_size,
            width,
            ascent: line.ascent,
            descent: line.descent,
            runs,
            len: line.len(),
        });
        div()
            .id(self.0.clone())
            .role(accesskit::Role::Label)
            .aria_value(self.0)
            .child(
                canvas(
                    move |_, _, _| line,
                    move |bounds, line, window, cx| {
                        line.paint(
                            bounds.origin,
                            line_height,
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        )
                        .expect("draft heading paint");
                    },
                )
                .w(width)
                .h(line_height),
            )
    }
}
