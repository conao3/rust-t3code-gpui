use super::*;

pub struct ConversationState {
    pub(super) id: u64,
    pub(super) revision: u64,
    pub(super) draft_text: String,
    view: Entity<ConversationPane>,
    stamp: Option<String>,
    persisted_target: Option<String>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
    pub(super) open: Option<OpenThread>,
    pub(super) draft: Option<NewThreadDraft>,
    pub(super) rows: Vec<Row>,
    pub(super) row_views: Vec<Entity<RowView>>,
    pub(super) timeline_pane: Entity<TimelinePane>,
    pub(super) row_subscriptions: HashMap<String, Subscription>,
    pub(super) list_state: ListState,
    pub(super) user_cursor: Option<usize>,
    pub(super) hovered_message: Option<String>,
    pub(super) composer: Entity<TextareaState>,
    pub(super) composer_controls: Entity<ComposerControlsPane>,
    pub(super) composer_controls_stamp: Option<ComposerControlsStamp>,
    pub(super) message: Option<String>,
    pub(super) picker_open: bool,
    pub(super) picker_instance: Option<String>,
    pub(super) permissions_menu: Option<Entity<permissions::Menu>>,
    pub(super) permissions_subscription: Option<Subscription>,
    pub(super) model_picker: Entity<CommandState>,
    pub(super) attachments: Vec<attachments::Pending>,
    pub(super) queued: Vec<timeline::Queued>,
    pub(super) thread_loading: bool,
    pub(super) completion: Option<Completion>,
    pub(super) completion_seq: u64,
    pub(super) find_input: Entity<InputState>,
    pub(super) find_open: bool,
    pub(super) find_hits: Vec<usize>,
    pub(super) find_at: usize,
    pub(super) pending_composer: Option<String>,
    pub(super) thread_dirty: bool,
}

impl std::ops::Deref for Root {
    type Target = ConversationState;

    fn deref(&self) -> &Self::Target {
        &self.conversation
    }
}

impl std::ops::DerefMut for Root {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.conversation
    }
}

impl ConversationState {
    pub(super) fn new(
        id: u64,
        message: Option<String>,
        window: &mut Window,
        cx: &mut Context<Root>,
    ) -> Self {
        let composer = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx)
                .placeholder("Ask anything. @ files, $ skills, / commands");
            state.set_auto_grow(1, 12, cx);
            state
        });
        let find_input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in thread"));
        let subscriptions = vec![
            cx.subscribe(&composer, move |root, _, event: &InputEvent, cx| {
                root.with_conversation(id, |root| {
                    if matches!(event, InputEvent::Change) && root.pending_composer.is_none() {
                        root.draft_text = root.composer.read(cx).value().to_string();
                        root.stash_composer(cx);
                        root.update_completion(cx);
                    }
                });
            }),
            cx.subscribe(&find_input, move |root, _, event: &InputEvent, cx| {
                root.with_conversation(id, |root| match event {
                    InputEvent::Change => root.find_again(true, cx),
                    InputEvent::PressEnter { shift, .. } => root.step_find(!*shift, cx),
                    _ => {}
                });
            }),
        ];
        let root = cx.entity().downgrade();
        let list_state = ListState::new(0, ListAlignment::Top, px(800.));
        list_state.set_follow_mode(FollowMode::Tail);
        Self {
            id,
            revision: 0,
            draft_text: String::new(),
            stamp: None,
            persisted_target: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
            view: cx.new(|_| ConversationPane {
                root: root.clone(),
                id,
            }),
            open: None,
            draft: None,
            rows: vec![],
            row_views: vec![],
            timeline_pane: cx.new(|_| TimelinePane {
                views: vec![],
                list: list_state.clone(),
            }),
            row_subscriptions: HashMap::new(),
            list_state,
            user_cursor: None,
            hovered_message: None,
            composer,
            composer_controls: cx.new(|_| ComposerControlsPane { root, pane_id: id }),
            composer_controls_stamp: None,
            message,
            picker_open: false,
            picker_instance: None,
            permissions_menu: None,
            permissions_subscription: None,
            model_picker: cx.new(|cx| CommandState::new(window, cx)),
            attachments: vec![],
            queued: vec![],
            thread_loading: false,
            completion: None,
            completion_seq: 0,
            find_input,
            find_open: false,
            find_hits: vec![],
            find_at: 0,
            pending_composer: None,
            thread_dirty: false,
        }
    }
}

struct ConversationPane {
    root: WeakEntity<Root>,
    id: u64,
}

impl Render for ConversationPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *TRACE.get_or_init(|| std::env::var_os("T3CODE_GPUI_TRACE_CONVERSATIONS").is_some()) {
            eprintln!("T3CODE_GPUI_CONVERSATION_RENDER {}", self.id);
        }

        self.root
            .update(cx, |root, cx| {
                root.with_conversation(self.id, |root| {
                    let body = if let Some(draft) = &root.draft {
                        root.render_draft(draft, &root.project_groups(), cx)
                    } else {
                        root.render_thread(cx)
                    };
                    let id = root.id;
                    let multiple = root.pane_layout.ids().len() > 1;
                    let title = root
                        .current()
                        .and_then(|(environment, thread)| root.thread_value(&environment, &thread))
                        .and_then(|thread| thread["title"].as_str().map(str::to_owned))
                        .unwrap_or_else(|| "New thread".into());
                    v_flex()
                        .id(("conversation", id))
                        .role(accesskit::Role::Region)
                        .aria_label("Conversation")
                        .size_full()
                        .min_w_0()
                        .min_h_0()
                        .track_focus(&root.conversation.focus)
                        .capture_any_mouse_down(cx.listener(move |root, _, window, cx| {
                            root.activate_conversation(id, window, cx, false);
                        }))
                        .child(
                            h_flex()
                                .h(px(36.))
                                .px_2()
                                .gap_1()
                                .border_b_1()
                                .border_color(cx.theme().border)
                                .child(div().flex_1().text_xs().truncate().child(title))
                                .child(
                                    Button::new("split-conversation")
                                        .label("Split")
                                        .ghost()
                                        .compact()
                                        .accessibility_label("Split conversation horizontally")
                                        .on_click(cx.listener(move |root, _, window, cx| {
                                            root.activate_conversation(id, window, cx, false);
                                            root.split_conversation(false, window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("split-conversation-vertical")
                                        .label("Split vertically")
                                        .ghost()
                                        .compact()
                                        .accessibility_label("Split conversation vertically")
                                        .on_click(cx.listener(move |root, _, window, cx| {
                                            root.activate_conversation(id, window, cx, false);
                                            root.split_conversation(true, window, cx);
                                        })),
                                )
                                .when(multiple, |bar| {
                                    bar.child(
                                        Button::new("close-conversation")
                                            .icon(IconName::Close)
                                            .ghost()
                                            .compact()
                                            .accessibility_label("Close side chat")
                                            .on_click(cx.listener(move |root, _, window, cx| {
                                                root.activate_conversation(id, window, cx, false);
                                                root.close_conversation(window, cx);
                                            })),
                                    )
                                }),
                        )
                        .child(body)
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element())
            })
            .expect("conversation root is alive")
    }
}

impl Root {
    pub(super) fn with_conversation<R>(
        &mut self,
        id: u64,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        if self.id == id {
            return Some(f(self));
        }
        let mut pane = self.panes.remove(&id)?;
        std::mem::swap(&mut self.conversation, &mut pane);
        let result = f(self);
        std::mem::swap(&mut self.conversation, &mut pane);
        self.panes.insert(id, pane);
        if std::mem::take(&mut self.pane_save_pending) {
            self.save_conversations();
        }
        Some(result)
    }

    pub(super) fn activate_conversation(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
        focus: bool,
    ) {
        if self.id != id
            && let Some(mut pane) = self.panes.remove(&id)
        {
            std::mem::swap(&mut self.conversation, &mut pane);
            self.panes.insert(pane.id, pane);
            self.sync_lineage();
            self.save_conversations();
            cx.notify();
        }
        if focus {
            self.composer
                .update(cx, |state, cx| state.focus(window, cx));
        }
    }

    pub(super) fn save_conversations(&mut self) {
        if self.pane_layout.ids().len() != self.panes.len() + 1 {
            self.pane_save_pending = true;
            return;
        }
        self.pane_save_pending = false;
        let saved = pane_layout::SavedLayout {
            layout: self.pane_layout.clone(),
            active: self.id,
            panes: std::iter::once(&self.conversation)
                .chain(self.panes.values())
                .map(|pane| pane_layout::SavedPane {
                    id: pane.id,
                    draft: pane.draft.clone(),
                    composer: pane.draft_text.clone(),
                    thread: pane
                        .open
                        .as_ref()
                        .map(|open| (open.environment.clone(), open.watch.thread_id().to_string())),
                })
                .collect(),
            closed: self.pane_closed.clone(),
        };
        let path = self.drafts_path.with_file_name("conversations.json");
        let result = (|| -> anyhow::Result<()> {
            std::fs::create_dir_all(path.parent().context("conversation state directory")?)?;
            let temporary = path.with_extension("tmp");
            std::fs::write(&temporary, serde_json::to_vec(&saved)?)?;
            std::fs::rename(temporary, path)?;
            Ok(())
        })();
        if let Err(error) = result {
            self.message = Some(format!("Cannot save conversation layout: {error:#}"));
        }
    }

    fn restore_conversations(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pane_restored {
            return;
        }
        self.pane_restored = true;
        let path = self.drafts_path.with_file_name("conversations.json");
        let saved = match std::fs::read(&path) {
            Ok(data) => serde_json::from_slice::<pane_layout::SavedLayout>(&data)
                .map_err(anyhow::Error::from),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => Err(error.into()),
        }
        .and_then(|saved| {
            saved.validate()?;
            Ok(saved)
        });
        match saved {
            Ok(saved) => {
                self.pane_next_id = saved
                    .panes
                    .iter()
                    .chain(&saved.closed)
                    .map(|pane| pane.id)
                    .max()
                    .unwrap_or(0)
                    + 1;
                self.conversation = ConversationState::new(saved.active, None, window, cx);
                for pane in &saved.panes {
                    if pane.id != saved.active {
                        self.panes
                            .insert(pane.id, ConversationState::new(pane.id, None, window, cx));
                    }
                }
                self.pane_layout = saved.layout;
                self.pane_closed = saved.closed;
                for pane in saved.panes {
                    self.with_conversation(pane.id, |root| {
                        if let Some((environment, thread)) = pane.thread {
                            root.open_thread(environment, thread, cx);
                        }
                        root.draft = pane.draft;
                        root.pending_composer = Some(pane.composer);
                    });
                }
                self.composer
                    .update(cx, |state, cx| state.focus(window, cx));
            }
            Err(error) => {
                self.message = Some(format!("Cannot restore conversation layout: {error:#}"))
            }
        }
    }

    pub(super) fn split_conversation(
        &mut self,
        vertical: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let thread = self.current();
        let draft = self.draft.clone();
        let id = self.pane_next_id;
        self.pane_next_id += 1;
        let active = self.id;
        self.pane_layout.split(active, id, vertical);
        self.panes
            .insert(id, ConversationState::new(id, None, window, cx));
        self.activate_conversation(id, window, cx, false);
        if let Some((environment, thread)) = thread {
            self.open_thread(environment, thread, cx);
        }
        if let Some(mut draft) = draft {
            draft.id = uuid::Uuid::new_v4().to_string();
            self.draft = Some(draft);
        }
        self.save_conversations();
        self.composer
            .update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub(super) fn close_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.panes.is_empty() {
            return;
        }
        self.stash_composer(cx);
        let id = self.id;
        self.pane_closed.push(pane_layout::SavedPane {
            id,
            draft: self.draft.clone(),
            thread: self.current(),
            composer: self.composer.read(cx).value().to_string(),
        });
        if self.pane_closed.len() > 20 {
            self.pane_closed.remove(0);
        }
        self.pane_layout = self
            .pane_layout
            .clone()
            .remove(id)
            .expect("another conversation is open");
        let next = self.pane_layout.ids()[0];
        self.activate_conversation(next, window, cx, true);
        self.panes.remove(&id);
        self.save_conversations();
        cx.notify();
    }

    pub(super) fn reopen_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pane) = self.pane_closed.pop() {
            self.split_conversation(false, window, cx);
            if let Some((environment, thread)) = pane.thread {
                self.open_thread(environment, thread, cx);
            }
            if pane.draft.is_some() {
                self.close_thread(cx);
            }
            self.draft = pane.draft;
            self.pending_composer = Some(pane.composer);
            self.save_conversations();
        }
    }

    pub(super) fn focus_workspace(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids = self.pane_layout.ids();
        let mut targets = vec![];
        if !self.sidebar_hidden {
            targets.push((self.id, self.focus.clone()));
        }
        for id in ids {
            self.with_conversation(id, |root| {
                targets.push((id, root.conversation.focus.clone()));
                targets.push((id, root.composer.read(cx).focus_handle(cx)));
            });
        }
        if self.file_panel.is_some() || self.explorer.is_some() {
            targets.push((self.id, self.panel_focus.clone()));
        }
        let terminal = self
            .terminal_key()
            .filter(|key| self.terminal_store.get(key).open)
            .and_then(|key| self.terminal_drawers.get(&key))
            .cloned();
        if let Some(terminal) = &terminal
            && terminal.read(cx).is_focused(window, cx)
        {
            let (id, handle) = if forward {
                &targets[0]
            } else {
                &targets[targets.len() - 1]
            };
            self.activate_conversation(*id, window, cx, false);
            handle.focus(window, cx);
            return;
        }
        let current = targets
            .iter()
            .position(|(_, focus)| focus.is_focused(window))
            .unwrap_or(0);
        if let Some(terminal) = terminal
            && ((forward && current + 1 == targets.len()) || (!forward && current == 0))
        {
            terminal.update(cx, |drawer, cx| drawer.focus(window, cx));
            return;
        }
        let next = if forward {
            (current + 1) % targets.len()
        } else {
            (current + targets.len() - 1) % targets.len()
        };
        let (id, handle) = &targets[next];
        self.activate_conversation(*id, window, cx, false);
        handle.focus(window, cx);
    }

    pub(super) fn render_conversations(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.restore_conversations(window, cx);
        let mut save = false;
        for id in self.pane_layout.ids() {
            self.with_conversation(id, |root| {
                if let Some(text) = root.pending_composer.take() {
                    root.draft_text = text.clone();
                    root.composer
                        .update(cx, |state, cx| state.set_value(text, window, cx));
                }
                let target = format!(
                    "{:?}:{}",
                    root.current(),
                    serde_json::to_string(&root.draft).expect("serialize conversation draft")
                );
                if root.persisted_target.as_ref() != Some(&target) {
                    root.persisted_target = Some(target);
                    save = true;
                }
                let controls = ComposerControlsStamp {
                    target: root.composer_target(),
                    current: root.current(),
                    draft: root.draft.is_some(),
                    status: root.open_entry().map(|entry| entry.status),
                    question: root.pending_question().is_some(),
                    empty: root.composer.read(cx).value().trim().is_empty(),
                    attachments: !root.attachments.is_empty(),
                    picker: root.picker_open,
                    permissions: root.permissions_menu.is_some(),
                    picker_instance: root.picker_instance.clone(),
                    catalog_revision: root.model_catalog_revision,
                    light: root.settings.light_theme,
                    font_size: root.settings.font_size_interface,
                    plan_enabled: root.settings.plan_mode_enabled,
                };
                let controls_changed = root.composer_controls_stamp.as_ref() != Some(&controls);
                if controls_changed {
                    root.composer_controls_stamp = Some(controls);
                    window.mark_view_dirty(root.composer_controls.entity_id());
                }
                let stamp = format!(
                    "{:?}:{:?}:{}:{}:{}:{:?}:{}:{}:{:?}:{}:{}",
                    root.current(),
                    root.draft.as_ref().map(|draft| (&draft.id, &draft.group)),
                    root.revision,
                    root.find_open,
                    root.find_at,
                    root.message,
                    root.thread_loading,
                    root.completion_seq,
                    root.picker_instance,
                    root.settings.light_theme,
                    root.settings.font_size_interface
                );
                let stamp = format!(
                    "{stamp}:{}:{}:{}:{:?}",
                    root.pane_layout.ids().len(),
                    root.attachments.len(),
                    root.queued.len(),
                    root.completion
                        .as_ref()
                        .map(|completion| completion.selected)
                );
                let stamp = format!(
                    "{stamp}:{:?}:{:?}",
                    root.open_entry().map(|entry| &entry.title),
                    root.draft.as_ref().map(|_| root
                        .project_groups()
                        .into_iter()
                        .map(|group| (group.key, group.title))
                        .collect::<Vec<_>>())
                );
                let stamp = format!(
                    "{stamp}:{}:{}",
                    root.find_input.read(cx).value(),
                    root.find_hits.len()
                );
                if controls_changed || root.stamp.as_ref() != Some(&stamp) {
                    root.stamp = Some(stamp);
                    window.mark_view_dirty(root.view.entity_id());
                }
            });
        }
        if save {
            self.save_conversations();
        }
        self.render_conversation_layout(&self.pane_layout.clone(), vec![], cx)
    }

    fn render_conversation_layout(
        &self,
        layout: &pane_layout::Layout,
        path: Vec<bool>,
        cx: &Context<Self>,
    ) -> AnyElement {
        match layout {
            pane_layout::Layout::Pane(id) => {
                let pane = if *id == self.id {
                    &self.conversation
                } else {
                    &self.panes[id]
                };
                pane.view
                    .clone()
                    .cached(StyleRefinement::default().size_full())
                    .into_any_element()
            }
            pane_layout::Layout::Split {
                vertical,
                fraction,
                first,
                second,
            } => {
                let vertical = *vertical;
                let mut left_path = path.clone();
                left_path.push(false);
                let mut right_path = path.clone();
                right_path.push(true);
                let bounds = Rc::new(std::cell::Cell::new(Bounds::default()));
                let measured = bounds.clone();
                let first = self.render_conversation_layout(first, left_path, cx);
                let second = self.render_conversation_layout(second, right_path, cx);
                let handle = div()
                    .id(SharedString::from(format!("conversation-resize-{path:?}")))
                    .role(accesskit::Role::Splitter)
                    .aria_label("Resize conversation panes")
                    .flex_none()
                    .bg(cx.theme().border)
                    .when(vertical, |el| el.h(px(4.)).w_full().cursor_row_resize())
                    .when(!vertical, |el| el.w(px(4.)).h_full().cursor_col_resize())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |root, _, _, cx| {
                            root.pane_resize = Some((path.clone(), vertical, bounds.get()));
                            cx.notify();
                        }),
                    );
                div()
                    .relative()
                    .flex()
                    .size_full()
                    .min_h_0()
                    .min_w_0()
                    .when(vertical, |el| el.flex_col())
                    .child(
                        canvas(move |bounds, _, _| measured.set(bounds), |_, _, _, _| {})
                            .absolute()
                            .size_full(),
                    )
                    .child(
                        div()
                            .min_h_0()
                            .min_w_0()
                            .overflow_hidden()
                            .flex_none()
                            .when(vertical, |el| el.h(relative(*fraction)).w_full())
                            .when(!vertical, |el| el.w(relative(*fraction)).h_full())
                            .child(first),
                    )
                    .child(handle)
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .overflow_hidden()
                            .child(second),
                    )
                    .into_any_element()
            }
        }
    }
}

#[derive(Clone)]
pub(super) struct PaneRoot {
    root: WeakEntity<Root>,
    id: u64,
}

impl PaneRoot {
    pub fn update<C: AppContext, R>(
        &self,
        cx: &mut C,
        update: impl FnOnce(&mut Root, &mut Context<Root>) -> R,
    ) -> anyhow::Result<R> {
        self.root
            .update(cx, |root, cx| {
                root.with_conversation(self.id, |root| update(root, cx))
            })?
            .context("conversation pane is closed")
    }

    pub fn update_in<C: AppContext, R>(
        &self,
        cx: &mut C,
        update: impl FnOnce(&mut Root, &mut Window, &mut Context<Root>) -> R,
    ) -> anyhow::Result<R> {
        self.root
            .update_in(cx, |root, window, cx| {
                root.with_conversation(self.id, |root| update(root, window, cx))
            })?
            .context("conversation pane is closed")
    }

    pub fn read_with<C: AppContext, R>(
        &self,
        cx: &mut C,
        read: impl FnOnce(&Root, &App) -> R,
    ) -> anyhow::Result<R> {
        self.update(cx, |root, cx| read(root, cx))
    }
}

impl Root {
    pub(super) fn spawn_pane<F, R: 'static>(&self, cx: &Context<Self>, f: F) -> Task<R>
    where
        F: AsyncFnOnce(PaneRoot, &mut AsyncApp) -> R + 'static,
    {
        let pane = PaneRoot {
            root: cx.entity().downgrade(),
            id: self.id,
        };
        cx.spawn(async move |_, cx| f(pane, cx).await)
    }

    pub(super) fn spawn_pane_in<F, R: 'static>(
        &self,
        window: &Window,
        cx: &Context<Self>,
        f: F,
    ) -> Task<R>
    where
        F: AsyncFnOnce(PaneRoot, &mut AsyncWindowContext) -> R + 'static,
    {
        let pane = PaneRoot {
            root: cx.entity().downgrade(),
            id: self.id,
        };
        cx.spawn_in(window, async move |_, cx| f(pane, cx).await)
    }
}

impl Root {
    pub(super) fn open_side_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((environment, source)) = self.current() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let target = uuid::Uuid::new_v4().to_string();
        let pane_id = self.id;
        let runtime = self.runtime.clone();
        cx.spawn_in(window, async move |this, cx| {
            let target_id = target.clone();
            let result = on_runtime(&runtime, async move {
                connection.dispatch(json!({"type":"thread.fork","createdBy":"user","creationSource":"web","sourceThreadId":source,"targetThreadId":target_id,"sourcePoint":{"type":"latest_stable"},"sideChat":true,"title":"Side chat"})).await
            }).await;
            let _ = this.update_in(cx, |root, window, cx| {
                match result {
                    Ok(_) => {
                        root.activate_conversation(pane_id, window, cx, false);
                        root.split_conversation(false, window, cx);
                        root.open_thread(environment, target, cx);
                    }
                    Err(error) => root.message = Some(format!("Could not start a side chat: {error:#}")),
                }
                cx.notify();
            });
        }).detach();
    }
}

impl Root {
    pub(super) fn pane_listener<E: ?Sized, L>(
        &self,
        cx: &Context<Self>,
        listener: L,
    ) -> impl Fn(&E, &mut Window, &mut App) + 'static + use<E, L>
    where
        L: Fn(&mut Self, &E, &mut Window, &mut Context<Self>) + 'static,
    {
        let id = self.id;
        let entity = cx.entity().downgrade();
        move |event, window, cx| {
            let _ = entity.update(cx, |root, cx| {
                root.with_conversation(id, |root| listener(root, event, window, cx));
            });
        }
    }
}
