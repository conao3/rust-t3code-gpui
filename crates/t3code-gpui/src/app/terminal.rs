use super::*;

impl Root {
    pub(super) fn terminal_key(&self) -> Option<String> {
        if let Some(draft) = &self.draft {
            let (environment, _) = self.draft_project()?;
            return Some(drafts::thread_key(&environment, &draft.id));
        }
        self.current()
            .map(|(environment, id)| drafts::thread_key(&environment, &id))
    }

    fn terminal_target(&self) -> Option<(String, t3_terminal::session::Target)> {
        let (environment, cwd) = self.composer_cwd()?;
        let (id, project_id) = if let Some(draft) = &self.draft {
            (draft.id.clone(), self.draft_project()?.1)
        } else {
            let (_, id) = self.current()?;
            let thread = self.thread_value(&environment, &id)?;
            (id, thread.get("projectId")?.as_str()?.to_owned())
        };
        let project = self.project_value(&environment, &project_id)?;
        let project_root = project.get("workspaceRoot")?.as_str()?.to_owned();
        let worktree_path = (cwd != project_root).then(|| cwd.clone());
        Some((
            environment,
            t3_terminal::session::Target {
                thread_id: id,
                terminal_id: String::new(),
                cwd,
                worktree_path,
                project_root,
                read_only: false,
                restart_if_not_running: false,
            },
        ))
    }

    fn save_terminal_layout(&mut self, cx: &mut Context<Self>) {
        if self.terminal_save_scheduled {
            return;
        }
        self.terminal_save_scheduled = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let Ok(store) = this.update(cx, |root, _| {
                root.terminal_save_scheduled = false;
                root.terminal_store.clone()
            }) else {
                return;
            };
            let result = cx
                .background_executor()
                .spawn(async move { store.save() })
                .await;
            if let Err(error) = result {
                let _ = this.update(cx, |root, cx| {
                    root.message = Some(format!("Cannot save terminal layout: {error:#}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn sync_terminal_drawers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.terminal_key();
        let stamp = (
            key.clone(),
            self.terminal_revision,
            self.entries_revision,
            self.settings.light_theme,
            self.settings.font_size_code.to_bits(),
        );
        if self.terminal_stamp.as_ref() == Some(&stamp) {
            return;
        }
        self.terminal_stamp = Some(stamp);
        let active = key
            .as_ref()
            .filter(|key| self.terminal_store.get(key).open)
            .cloned();
        let mut existing: HashSet<_> = self
            .entries
            .iter()
            .map(|entry| drafts::thread_key(&entry.environment, &entry.thread_id))
            .collect();
        if let Some(key) = &key {
            existing.insert(key.clone());
        }
        self.terminal_recent
            .retain(|key| existing.contains(key) && self.terminal_store.get(key).open);
        if let Some(key) = &active {
            self.terminal_recent.retain(|recent| recent != key);
            self.terminal_recent.insert(0, key.clone());
        }
        self.terminal_recent
            .truncate(10 + usize::from(active.is_some()));
        self.terminal_drawers
            .retain(|key, _| self.terminal_recent.contains(key));
        self.terminal_subscriptions
            .retain(|key, _| self.terminal_recent.contains(key));
        let terminal_configuration = crate::terminal::DrawerConfig {
            light: self.settings.light_theme,
            font_size: self.settings.font_size_code,
            shortcuts: [
                "terminal.split",
                "terminal.splitVertical",
                "terminal.new",
                "terminal.close",
            ]
            .map(|command| {
                keymap::command_label(&self.key_rules, command, &|name| {
                    matches!(name, "terminalFocus" | "terminalOpen")
                        || self.key_context(name, window, cx)
                })
            }),
        };
        if let Some(key) = &active
            && !self.terminal_drawers.contains_key(key)
            && let Some((environment, target)) = self.terminal_target()
            && let Some(connection) = self.connection(&environment)
        {
            let layout = self.terminal_store.get(key);
            let drawer = cx.new(|cx| {
                crate::terminal::Drawer::new(
                    layout,
                    target,
                    connection,
                    self.runtime.clone(),
                    terminal_configuration.clone(),
                    window,
                    cx,
                )
            });
            let key = key.clone();
            let saved_key = key.clone();
            let subscription = cx.subscribe(&drawer, move |root, _, event, cx| match event {
                crate::terminal::DrawerEvent::Changed(layout) => {
                    root.terminal_store.set(saved_key.clone(), layout);
                    root.terminal_revision = root.terminal_revision.wrapping_add(1);
                    root.save_terminal_layout(cx);
                    cx.notify();
                }
                crate::terminal::DrawerEvent::Focus => {
                    if root.jump_hints {
                        root.jump_hints = false;
                        cx.notify();
                    }
                }
            });
            self.terminal_subscriptions
                .insert(key.clone(), subscription);
            self.terminal_drawers.insert(key, drawer);
        }
        for (key, drawer) in &self.terminal_drawers {
            drawer.update(cx, |drawer, cx| {
                drawer.configure(
                    active.as_ref() == Some(key),
                    terminal_configuration.clone(),
                    window,
                    cx,
                )
            });
        }
    }

    pub(super) fn toggle_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.terminal_key() else {
            return;
        };
        let mut layout = self.terminal_store.get(&key);
        layout.open = !layout.open;
        if layout.open && layout.ids().is_empty() {
            layout.add("term-1".into(), None);
        }
        let open = layout.open;
        self.terminal_store.set(key.clone(), &layout);
        self.terminal_revision = self.terminal_revision.wrapping_add(1);
        self.sync_terminal_drawers(window, cx);
        if open {
            if let Some(drawer) = self.terminal_drawers.get(&key).cloned() {
                drawer.update(cx, |drawer, cx| drawer.focus(window, cx));
            }
        } else {
            window.focus(&self.composer.read(cx).focus_handle(cx), cx);
        }
        self.save_terminal_layout(cx);
        cx.notify();
    }

    pub(super) fn add_terminal(
        &mut self,
        split: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.terminal_key() else {
            return;
        };
        if !self.terminal_store.get(&key).open {
            self.toggle_terminal(window, cx);
            return;
        }
        if let Some(drawer) = self.terminal_drawers.get(&key).cloned() {
            drawer.update(cx, |drawer, cx| drawer.add(split, window, cx));
        }
    }
}
