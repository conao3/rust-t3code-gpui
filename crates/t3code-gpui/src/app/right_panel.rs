use super::*;

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(super) struct State {
    pub open: bool,
    pub maximized: bool,
    pub launcher: bool,
    pub tabs: Vec<String>,
    pub active: Option<String>,
    pub pull_requests: HashMap<String, (String, Value)>,
    pub width: Option<f32>,
    #[serde(skip)]
    files: HashMap<String, FilePanel>,
    #[serde(skip)]
    tab_scroll: ScrollHandle,
    #[serde(skip)]
    terminals: HashMap<String, Entity<crate::terminal_view::TerminalView>>,
    #[serde(skip)]
    diff: diff_controls::State,
    #[serde(skip)]
    device: Option<Entity<device_panel::DevicePanel>>,
    #[serde(skip)]
    pub(super) file_search: Option<Entity<InputState>>,
    #[serde(skip)]
    file_search_subscription: Option<Subscription>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct Saved {
    pub threads: Vec<((String, String), State)>,
}

pub(super) fn load(path: &std::path::Path) -> anyhow::Result<Saved> {
    match std::fs::read(path.with_file_name("right-panels.json")) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Saved {
            threads: Vec::new(),
        }),
        Err(error) => Err(error.into()),
    }
}

pub(super) struct Pane {
    pub root: WeakEntity<Root>,
}

impl Render for Pane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.root
            .update(cx, |root, cx| {
                root.render_panel_contents(f32::from(window.viewport_size().width), cx)
            })
            .expect("panel root")
    }
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = t3code_gpui, no_json)]
pub(super) struct TabAction {
    pub id: String,
    pub kind: String,
}

impl Root {
    pub(super) fn panel_tab_action(&mut self, action: &TabAction, cx: &mut Context<Self>) {
        let Some(index) = self.right_panel.tabs.iter().position(|id| id == &action.id) else {
            return;
        };
        let closing = self
            .right_panel
            .tabs
            .iter()
            .enumerate()
            .filter(|(position, id)| match action.kind.as_str() {
                "close" => *id == &action.id,
                "others" => *id != &action.id,
                "right" => *position > index,
                "all" => true,
                _ => false,
            })
            .map(|(_, id)| id.clone())
            .collect::<Vec<_>>();
        for id in closing {
            self.close_panel_tab(&id, cx);
        }
    }

    pub(super) fn follow_right_panel(&mut self, cx: &mut Context<Self>) {
        let target = self.current().or_else(|| {
            self.conversation.draft.as_ref().map(|draft| {
                (
                    draft.environment.clone().unwrap_or_default(),
                    draft.id.clone(),
                )
            })
        });
        if target == self.right_panel_target {
            return;
        }
        self.park_panel_file();
        self.right_panel.width = Some(self.right_panel_width);
        self.right_panel.diff = std::mem::take(&mut self.diff_view);
        if let Some(previous) = self.right_panel_target.take() {
            self.right_panel_threads
                .insert(previous, std::mem::take(&mut self.right_panel));
        }
        self.right_panel = target
            .as_ref()
            .and_then(|target| self.right_panel_threads.remove(target))
            .unwrap_or_default();
        self.diff_view = std::mem::take(&mut self.right_panel.diff);
        self.right_panel_width = self.right_panel.width.unwrap_or(540.);
        self.diff_view.loading = false;
        self.diff_view.request += 1;
        self.right_panel_target = target;
        self.explorer = None;
        if let Some(active) = self.right_panel.active.clone() {
            self.file_panel = self.right_panel.files.remove(&active);
            if active == "files" && self.right_panel.open {
                self.toggle_explorer(cx);
            }
        }
        self.file_find_open = false;
        self.panel_revision += 1;
        self.load_active_surface(cx);
    }

    pub(super) fn save_right_panel(&mut self) {
        self.right_panel.width = Some(self.right_panel_width);
        let mut threads = self
            .right_panel_threads
            .iter()
            .map(|(key, state)| json!([key, state]))
            .collect::<Vec<_>>();
        if let Some(key) = &self.right_panel_target {
            threads.push(json!([key, self.right_panel]));
        }
        let path = self.drafts_path.with_file_name("right-panels.json");
        let result = (|| -> anyhow::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let temporary = path.with_extension("tmp");
            std::fs::write(
                &temporary,
                serde_json::to_vec(&json!({"threads": threads}))?,
            )?;
            std::fs::rename(temporary, path)?;
            Ok(())
        })();
        if let Err(error) = result {
            self.conversation.message = Some(format!("Could not save right panels: {error:#}"));
        }
    }

    fn load_active_surface(&mut self, cx: &mut Context<Self>) {
        if !self.right_panel.open || self.file_panel.is_some() {
            return;
        }
        match self.right_panel.active.clone().as_deref() {
            Some("files") if self.explorer.is_none() => self.toggle_explorer(cx),
            Some("diff") => self.refresh_diff(cx),
            Some("pull-requests") => self.open_pull_requests(false, cx),
            Some(id) if id.starts_with("pull-request:") => {
                if !self.pr_state.views.contains_key(id)
                    && let Some((environment, reference)) =
                        self.right_panel.pull_requests.get(id).cloned()
                {
                    self.pr_state.page = false;
                    self.open_pull_request(environment, reference, cx);
                }
            }
            Some(id) if id.starts_with("file:") => self.open_file(id[5..].into(), cx),
            _ => {}
        }
    }

    fn park_panel_file(&mut self) {
        if let Some(file) = self.file_panel.take() {
            let id = if file.path == DIFF_TITLE {
                "diff".into()
            } else {
                format!("file:{}", file.path)
            };
            self.right_panel.files.insert(id, file);
        }
    }

    pub(super) fn activate_panel(&mut self, id: String, cx: &mut Context<Self>) {
        self.park_panel_file();
        if !self.right_panel.tabs.contains(&id) {
            self.right_panel.tabs.push(id.clone());
        }
        self.file_panel = self.right_panel.files.remove(&id);
        self.right_panel.active = Some(id);
        self.right_panel.open = true;
        self.right_panel.launcher = false;
        self.file_find_open = false;
        self.panel_revision += 1;
        self.save_right_panel();
        cx.notify();
    }

    pub(super) fn install_panel_file(&mut self, panel: FilePanel, cx: &mut Context<Self>) {
        let id = if panel.path == DIFF_TITLE {
            "diff".into()
        } else {
            format!("file:{}", panel.path)
        };
        self.activate_panel(id, cx);
        self.file_panel = Some(panel);
    }

    pub(super) fn toggle_right_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.right_panel.open = !self.right_panel.open;
        if self.right_panel.open {
            self.load_active_surface(cx);
            self.panel_focus.focus(window, cx);
        } else {
            self.conversation
                .composer
                .update(cx, |state, cx| state.focus(window, cx));
        }
        self.panel_revision += 1;
        self.save_right_panel();
        cx.notify();
    }

    pub(super) fn new_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.right_panel.open = true;
        self.right_panel.launcher = true;
        self.panel_focus.focus(window, cx);
        self.panel_revision += 1;
        self.save_right_panel();
        cx.notify();
    }

    pub(super) fn close_panel_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(index) = self.right_panel.tabs.iter().position(|tab| tab == id) else {
            return;
        };
        self.right_panel.tabs.remove(index);
        self.right_panel.files.remove(id);
        if id == "device" {
            self.right_panel.device = None;
        }
        if let Some(terminal) = id.strip_prefix("terminal:") {
            self.right_panel.terminals.remove(id);
            if let Some((environment, thread)) = self.current()
                && let Some(connection) = self.connection(&environment)
            {
                let terminal = terminal.to_string();
                let runtime = self.runtime.clone();
                cx.spawn(async move |this, cx| {
                    let result = on_runtime(&runtime, async move { connection.request("terminal.close", json!({"threadId":thread,"terminalId":terminal,"deleteHistory":true})).await }).await;
                    let _ = this.update(cx, |root, cx| { if let Err(error) = result { root.message = Some(format!("Could not close terminal: {error:#}")); cx.notify(); } });
                }).detach();
            }
        }
        self.right_panel.pull_requests.remove(id);
        if self.right_panel.active.as_deref() == Some(id) {
            self.file_panel = None;
            self.right_panel.active = None;
            if let Some(next) = self
                .right_panel
                .tabs
                .get(index.min(self.right_panel.tabs.len().saturating_sub(1)))
                .cloned()
            {
                self.activate_panel(next, cx);
                self.load_active_surface(cx);
            } else {
                self.right_panel.open = false;
                self.right_panel.maximized = false;
            }
        }
        self.panel_revision += 1;
        self.save_right_panel();
        cx.notify();
    }

    pub(super) fn invalidate_panel_diff(&mut self, cx: &mut Context<Self>) {
        self.right_panel.files.remove("diff");
        if self.right_panel.active.as_deref() == Some("diff") {
            self.refresh_diff(cx);
        }
    }

    pub(super) fn panel_width(&self, viewport: f32) -> f32 {
        let sidebar = if self.sidebar_hidden {
            0.
        } else {
            self.settings.sidebar_width
        };
        if self.right_panel.maximized || viewport <= 980. {
            return (viewport - sidebar).max(360.);
        }
        self.right_panel_width.clamp(
            360.,
            (viewport * 0.7)
                .floor()
                .min(viewport - sidebar - 360.)
                .max(360.),
        )
    }

    pub(super) fn prepare_right_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.follow_right_panel(cx);
        if self.right_panel.file_search.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search files…"));
            self.right_panel.file_search_subscription =
                Some(cx.subscribe(&input, |root, input, event, cx| {
                    if matches!(event, InputEvent::Change) {
                        root.search_files(input.read(cx).value().to_string(), cx);
                        root.panel_revision += 1;
                        cx.notify();
                    }
                }));
            self.right_panel.file_search = Some(input);
        }
        if self.right_panel.active.as_deref() == Some("device")
            && self.right_panel.device.is_none()
            && let Some((environment, _)) = self.current()
            && let Some(connection) = self.connection(&environment)
        {
            let root = cx.entity().downgrade();
            let runtime = self.runtime.clone();
            self.right_panel.device =
                Some(cx.new(|cx| {
                    device_panel::DevicePanel::new(root, connection, runtime, window, cx)
                }));
            self.panel_revision += 1;
        }
        if let Some(id) = self
            .right_panel
            .active
            .clone()
            .filter(|id| id.starts_with("terminal:"))
            && !self.right_panel.terminals.contains_key(&id)
            && let Some((environment, cwd)) = self.composer_cwd()
            && let Some((_, thread_id)) = self.current()
            && let Some((_, project)) = self.current_project()
            && let Some(connection) = self.connection(&environment)
        {
            let target = t3_terminal::session::Target {
                thread_id,
                terminal_id: id[9..].into(),
                cwd: cwd.clone(),
                project_root: project["workspaceRoot"].as_str().unwrap_or_default().into(),
                worktree_path: (project["workspaceRoot"].as_str() != Some(&cwd)).then_some(cwd),
                read_only: false,
                restart_if_not_running: false,
            };
            let terminal = cx.new(|cx| {
                crate::terminal_view::TerminalView::new(
                    connection,
                    target,
                    self.settings.light_theme,
                    self.settings.font_size_code,
                    &self.runtime,
                    window,
                    cx,
                )
            });
            terminal.read(cx).focus.clone().focus(window, cx);
            self.right_panel.terminals.insert(id, terminal);
            self.panel_revision += 1;
        }
        for (id, terminal) in &self.right_panel.terminals {
            terminal.update(cx, |view, cx| {
                view.set_visible(
                    self.right_panel.open && self.right_panel.active.as_ref() == Some(id),
                    cx,
                );
                view.configure(
                    self.settings.light_theme,
                    self.settings.font_size_code,
                    window,
                    cx,
                );
            });
        }
        let file = self.file_panel.as_ref().map(|p| {
            (
                p.path.clone(),
                Rc::as_ptr(&p.lines) as usize,
                p.hit_at,
                p.hits.len(),
            )
        });
        let explorer = self
            .explorer
            .as_ref()
            .map(|e| (e.directories.len(), e.expanded.len(), e.error.clone()));
        let stamp = format!(
            "{}:{file:?}:{explorer:?}:{}:{}:{}:{:?}:{}:{}",
            self.panel_revision,
            self.file_find_open,
            self.file_find_input.read(cx).value(),
            self.settings.light_theme,
            self.branch_anchor,
            self.branch_refs.len(),
            self.branch_query
        );
        if self.panel_stamp != stamp {
            self.panel_stamp = stamp;
            window.mark_view_dirty(self.panel_pane.entity_id());
            window.mark_view_dirty(self.pr_pane.entity_id());
        }
    }

    pub(super) fn render_panel_tabs(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        h_flex()
            .id("panel-tabs")
            .h(px(40.))
            .min_w_0()
            .w_full()
            .overflow_hidden()
            .gap_1()
            .px_2()
            .child(
                h_flex()
                    .id("panel-tab-scroll")
                    .min_w_0()
                    .flex_1()
                    .overflow_x_scroll()
                    .track_scroll(&self.right_panel.tab_scroll)
                    .children(self.right_panel.tabs.iter().map(|id| {
                        let label = panel_title(id);
                        let select = id.clone();
                        let close = id.clone();
                        let menu_id = id.clone();
                        let focus = self.focus.clone();
                        let drop_id = id.clone();
                        let middle_id = id.clone();
                        h_flex()
                            .id(SharedString::from(format!("tab-context-{id}")))
                            .group("panel-tab")
                            .h(px(24.))
                            .rounded_md()
                            .when(self.right_panel.active.as_ref() == Some(id), |tab| {
                                tab.bg(theme.accent)
                            })
                            .on_drag(PanelTabDrag(id.clone()), |drag, _, _, cx| {
                                cx.new(|_| drag.clone())
                            })
                            .on_drop(cx.listener(move |root, drag: &PanelTabDrag, _, cx| {
                                if let (Some(from), Some(to)) = (
                                    root.right_panel.tabs.iter().position(|id| id == &drag.0),
                                    root.right_panel.tabs.iter().position(|id| id == &drop_id),
                                ) {
                                    let id = root.right_panel.tabs.remove(from);
                                    root.right_panel.tabs.insert(to, id);
                                    root.panel_revision += 1;
                                    root.save_right_panel();
                                    cx.notify();
                                }
                            }))
                            .on_mouse_up(
                                MouseButton::Middle,
                                cx.listener(move |root, _, _, cx| {
                                    root.close_panel_tab(&middle_id, cx)
                                }),
                            )
                            .min_w_0()
                            .child(
                                Button::new(SharedString::from(format!("close-{id}")))
                                    .child(
                                        div()
                                            .relative()
                                            .size(px(12.))
                                            .child(
                                                div()
                                                    .size(px(12.))
                                                    .group_hover("panel-tab", |icon| {
                                                        icon.opacity(0.)
                                                    })
                                                    .child(
                                                        Icon::new(surface_icon(id)).size(px(12.)),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .absolute()
                                                    .inset_0()
                                                    .opacity(0.)
                                                    .group_hover("panel-tab", |icon| {
                                                        icon.opacity(1.)
                                                    })
                                                    .child(
                                                        Icon::new(gpui_kit::assets::IconName::X)
                                                            .size(px(12.)),
                                                    ),
                                            ),
                                    )
                                    .ghost()
                                    .size(px(20.))
                                    .p_0()
                                    .accessibility_label(format!("Close {label}"))
                                    .on_click(cx.listener(move |root, _, _, cx| {
                                        root.close_panel_tab(&close, cx)
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(id.clone()))
                                    .label(label.clone())
                                    .ghost()
                                    .compact()
                                    .text_xs()
                                    .max_w(px(116.))
                                    .on_click(cx.listener(move |root, _, _, cx| {
                                        root.activate_panel(select.clone(), cx);
                                        root.load_active_surface(cx);
                                    })),
                            )
                            .context_menu(move |mut menu, _, _| {
                                for (label, kind) in [
                                    ("Close", "close"),
                                    ("Close others", "others"),
                                    ("Close to the right", "right"),
                                    ("Close all", "all"),
                                ] {
                                    menu = menu.menu(
                                        label,
                                        Box::new(TabAction {
                                            id: menu_id.clone(),
                                            kind: kind.into(),
                                        }),
                                    );
                                }
                                menu.action_context(focus.clone())
                            })
                    }))
                    .child(
                        Button::new("add-panel")
                            .icon(IconName::Plus)
                            .ghost()
                            .size(px(24.))
                            .accessibility_label("Add panel surface")
                            .on_click(
                                cx.listener(|root, _, window, cx| root.new_panel(window, cx)),
                            ),
                    ),
            )
            .children(
                [
                    (-1., "Scroll tabs left", IconName::ChevronLeft),
                    (1., "Scroll tabs right", IconName::ChevronRight),
                ]
                .into_iter()
                .filter(|_| self.right_panel.tab_scroll.max_offset().x > px(0.))
                .map(|(direction, label, icon)| {
                    let handle = self.right_panel.tab_scroll.clone();
                    Button::new(label)
                        .icon(icon)
                        .ghost()
                        .size(px(20.))
                        .accessibility_label(label)
                        .disabled(if direction < 0. {
                            handle.offset().x >= px(0.)
                        } else {
                            -handle.offset().x >= handle.max_offset().x
                        })
                        .on_click(move |_, _, cx| {
                            let mut offset = handle.offset();
                            offset.x += px(direction * -120.);
                            handle.set_offset(offset);
                            cx.refresh_windows();
                        })
                }),
            )
            .child(
                Button::new("maximize-panel")
                    .icon(if self.right_panel.maximized {
                        gpui_kit::assets::IconName::Minimize2
                    } else {
                        gpui_kit::assets::IconName::Maximize2
                    })
                    .accessibility_label(if self.right_panel.maximized {
                        "Restore panel size"
                    } else {
                        "Maximize panel"
                    })
                    .ghost()
                    .compact()
                    .on_click(cx.listener(|root, _, window, cx| {
                        root.panel_focus.focus(window, cx);
                        root.right_panel.maximized = !root.right_panel.maximized;
                        root.save_right_panel();
                        root.panel_revision += 1;
                        cx.notify();
                    })),
            )
            .child(self.render_panel_layout_controls(cx))
            .into_any_element()
    }

    pub(super) fn render_panel_layout_controls(&self, cx: &Context<Self>) -> AnyElement {
        h_flex()
            .gap_1()
            .child(
                Button::new("terminal-drawer-toggle")
                    .icon(gpui_kit::assets::IconName::PanelBottom)
                    .ghost()
                    .size(px(28.))
                    .accessibility_label("Toggle terminal drawer")
                    .on_click(cx.listener(|_, _, window, cx| {
                        window.dispatch_action(Box::new(ToggleTerminal), cx)
                    })),
            )
            .child(
                Button::new("right-panel-toggle")
                    .icon(gpui_kit::assets::IconName::PanelRight)
                    .ghost()
                    .size(px(28.))
                    .selected(self.right_panel.open)
                    .accessibility_label("Toggle right panel")
                    .on_click(
                        cx.listener(|root, _, window, cx| root.toggle_right_panel(window, cx)),
                    ),
            )
            .into_any_element()
    }

    fn render_panel_contents(&self, width: f32, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let body = if self.right_panel.active.is_none() {
            self.render_surface_launcher(cx)
        } else {
            match self.right_panel.active.as_deref() {
                Some(id) if id.starts_with("pull-request:") || id == "pull-requests" => {
                    self.render_pull_requests(width, cx)
                }
                Some(id) if id.starts_with("terminal:") => self
                    .right_panel
                    .terminals
                    .get(id)
                    .map(|terminal| terminal.clone().into_any_element())
                    .unwrap_or_else(|| div().child(text!("Loading…")).into_any_element()),
                Some("diff") => v_flex()
                    .h_full()
                    .min_h_0()
                    .child(self.render_diff_toolbar(cx))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_h_0()
                            .items_start()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .children(self.render_file_panel(cx)),
                            )
                            .when(self.diff_view.file_tree, |row| {
                                row.child(self.render_diff_file_tree(cx))
                            }),
                    )
                    .into_any_element(),
                Some("device") => self
                    .right_panel
                    .device
                    .as_ref()
                    .map(|device| {
                        if device.read(cx).setup {
                            div().into_any_element()
                        } else {
                            device.clone().into_any_element()
                        }
                    })
                    .unwrap_or_else(|| div().into_any_element()),
                Some("side-chat") => self.render_side_chat_surface(cx),
                Some(id) if id == "browser" => v_flex()
                    .p_4()
                    .gap_2()
                    .child(text!(panel_title(id)))
                    .child(text!(format!(
                        "{} panel is not implemented in T3 Code GPUI.",
                        panel_title(id)
                    )))
                    .into_any_element(),
                Some("files") => self
                    .render_explorer(cx)
                    .unwrap_or_else(|| div().child(text!("Loading files…")).into_any_element()),
                _ => self
                    .render_file_panel(cx)
                    .unwrap_or_else(|| div().child(text!("Loading…")).into_any_element()),
            }
        };
        v_flex()
            .id("workspace-panel")
            .role(accesskit::Role::Region)
            .aria_label(
                self.right_panel
                    .active
                    .as_deref()
                    .map(|id| format!("Workspace panel: {}", panel_title(id)))
                    .unwrap_or_else(|| "Workspace panel".into()),
            )
            .track_focus(&self.panel_focus)
            .on_key_down(cx.listener(|root, event: &KeyDownEvent, window, cx| {
                if root.right_panel.launcher || root.right_panel.active.is_none() {
                    match event.keystroke.key.as_str() {
                        "b" => root.open_panel_surface("Browser", cx),
                        "t" => root.open_panel_surface("Terminal", cx),
                        "m" => root.open_panel_surface("Device", cx),
                        "s" => root.open_panel_surface("Side chat", cx),
                        "f" => root.toggle_explorer(cx),
                        "d" => root.toggle_diff(cx),
                        "p" => root.open_current_pull_request(cx),
                        "l" => root.open_pull_requests(false, cx),
                        "escape" => {
                            root.right_panel.launcher = false;
                            root.panel_revision += 1;
                            root.conversation
                                .composer
                                .update(cx, |state, cx| state.focus(window, cx));
                            cx.notify();
                        }
                        _ => return,
                    }
                    cx.stop_propagation();
                }
            }))
            .relative()
            .size_full()
            .min_w_0()
            .bg(theme.background)
            .border_l_1()
            .border_color(theme.border)
            .child(div().flex_1().min_h_0().child(body))
            .when(
                self.right_panel.launcher && self.right_panel.active.is_some(),
                |panel| {
                    panel.child(
                        v_flex()
                            .id("add-surface-menu")
                            .role(accesskit::Role::Menu)
                            .aria_label("Add panel surface")
                            .absolute()
                            .top(px(0.))
                            .right(px(4.))
                            .w(px(240.))
                            .p_1()
                            .rounded_md()
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.popover)
                            .shadow_lg()
                            .children(
                                [
                                    ("Browser", "B"),
                                    ("Terminal", "T"),
                                    ("Files", "F"),
                                    ("Diff", "D"),
                                    ("Pull request", "P"),
                                    ("Linked pull requests", "L"),
                                    ("Device", "M"),
                                    ("Side chat", "S"),
                                ]
                                .into_iter()
                                .map(|(label, key)| {
                                    Button::new(label)
                                        .label(format!("{label} {key}"))
                                        .accessibility_label(label)
                                        .role(accesskit::Role::MenuItem)
                                        .ghost()
                                        .w_full()
                                        .on_click(cx.listener(move |root, _, _, cx| {
                                            if ["Browser", "Terminal", "Device", "Side chat"]
                                                .contains(&label)
                                            {
                                                root.open_panel_surface(label, cx);
                                            } else if label == "Files" {
                                                root.toggle_explorer(cx);
                                            } else if label == "Pull request" {
                                                root.open_current_pull_request(cx);
                                            } else if label == "Linked pull requests" {
                                                root.open_pull_requests(false, cx);
                                            } else {
                                                root.toggle_diff(cx);
                                            }
                                        }))
                                }),
                            ),
                    )
                },
            )
            .into_any_element()
    }

    fn render_diff_file_tree(&self, cx: &Context<Self>) -> AnyElement {
        let state = self
            .file_panel
            .as_ref()
            .and_then(|panel| match &panel.body {
                PanelBody::Diff(state) => Some(state.clone()),
                _ => None,
            });
        v_flex()
            .id("diff-file-tree")
            .role(accesskit::Role::Tree)
            .aria_label("Changes files")
            .w(px(self.right_panel_width * 0.4))
            .h_full()
            .border_l_1()
            .border_color(cx.theme().border)
            .when_some(state, |tree, state| {
                tree.children(state.read(cx).files().iter().map(|file| {
                    let path = file.path().to_string();
                    let state = state.clone();
                    Button::new(SharedString::from(path.clone()))
                        .label(path.clone())
                        .role(accesskit::Role::TreeItem)
                        .ghost()
                        .compact()
                        .on_click(move |_, _, cx| {
                            state.update(cx, |state, cx| {
                                state.set_file_collapsed(&path, false, cx);
                                state.scroll_to_file(&path, cx);
                            });
                        })
                }))
            })
            .into_any_element()
    }

    fn open_panel_surface(&mut self, label: &str, cx: &mut Context<Self>) {
        self.right_panel.launcher = false;
        if label == "Terminal" {
            let mut number = 1;
            let existing = self
                .terminal_key()
                .map(|key| self.terminal_store.get(&key).ids())
                .unwrap_or_default();
            while existing
                .iter()
                .any(|id| id.starts_with(&format!("term-{number}")))
                || self
                    .right_panel
                    .tabs
                    .iter()
                    .any(|id| id.starts_with(&format!("terminal:term-{number}")))
            {
                number += 1;
            }
            self.activate_panel(
                format!("terminal:term-{number}-{}", uuid::Uuid::new_v4()),
                cx,
            );
        } else {
            let id = match label {
                "Browser" => "browser",
                "Device" => "device",
                "Side chat" => "side-chat",
                _ => return,
            };
            self.activate_panel(id.into(), cx);
        }
        self.panel_revision += 1;
        cx.notify();
    }

    fn render_surface_launcher(&self, cx: &Context<Self>) -> AnyElement {
        v_flex()
            .id("surface-launcher")
            .aria_label("Open a surface")
            .h_full()
            .items_center()
            .flex_1()
            .justify_center()
            .px_6()
            .gap_1()
            .child(div().text_sm().mb_3().child(text!("Open a surface")))
            .children(
                [
                    ("Browser", "B"),
                    ("Terminal", "T"),
                    ("Files", "F"),
                    ("Diff", "D"),
                    ("Pull request", "P"),
                    ("Linked pull requests", "L"),
                    ("Device", "M"),
                    ("Side chat", "S"),
                ]
                .into_iter()
                .map(|(label, key)| {
                    Button::new(label)
                        .child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .child(
                                    Icon::new(match key {
                                        "B" => gpui_kit::assets::IconName::Globe,
                                        "T" => gpui_kit::assets::IconName::Terminal,
                                        "M" => gpui_kit::assets::IconName::Smartphone,
                                        "S" => gpui_kit::assets::IconName::MessagesSquare,
                                        "F" => gpui_kit::assets::IconName::Files,
                                        "D" => gpui_kit::assets::IconName::FileDiff,
                                        "P" => gpui_kit::assets::IconName::GitPullRequest,
                                        _ => gpui_kit::assets::IconName::Link2,
                                    })
                                    .size(px(14.)),
                                )
                                .child(label)
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .text_xs()
                                        .px_1()
                                        .rounded_sm()
                                        .bg(cx.theme().muted)
                                        .child(key),
                                ),
                        )
                        .accessibility_label(
                            if matches!(label, "Pull request" | "Linked pull requests") {
                                label.to_string()
                            } else {
                                format!("{label} {key}")
                            },
                        )
                        .ghost()
                        .w(px(320.))
                        .on_click(cx.listener(move |root, _, _, cx| {
                            if ["Browser", "Terminal", "Device", "Side chat"].contains(&label) {
                                root.open_panel_surface(label, cx);
                            } else if label == "Files" {
                                root.toggle_explorer(cx);
                            } else if label == "Pull request" {
                                root.open_current_pull_request(cx);
                            } else if label == "Linked pull requests" {
                                root.open_pull_requests(false, cx);
                            } else {
                                root.toggle_diff(cx);
                            }
                        }))
                }),
            )
            .into_any_element()
    }
}

fn panel_title(id: &str) -> String {
    match id {
        "files" => "Files".into(),
        "browser" => "Browser".into(),
        "device" => "Device".into(),
        "side-chat" => "Side chat".into(),
        "diff" => "Diff".into(),
        id if id.starts_with("terminal:") => {
            format!("Terminal {}", id.split('-').nth(1).unwrap_or_default())
        }
        "pull-requests" => "Linked pull requests".into(),
        id if id.starts_with("pull-request:") => {
            format!("PR #{}", id.rsplit(':').next().unwrap_or_default())
        }
        _ => id
            .strip_prefix("file:")
            .unwrap_or(id)
            .rsplit('/')
            .next()
            .unwrap_or(id)
            .into(),
    }
}

#[derive(Clone)]
struct PanelTabDrag(String);

impl Render for PanelTabDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(cx.theme().accent)
            .text_xs()
            .child(panel_title(&self.0))
    }
}

fn surface_icon(id: &str) -> gpui_kit::assets::IconName {
    use gpui_kit::assets::IconName;
    match id {
        "files" => IconName::Files,
        "diff" => IconName::FileDiff,
        "browser" => IconName::Globe,
        id if id.starts_with("terminal:") => IconName::Terminal,
        "device" => IconName::Smartphone,
        "side-chat" => IconName::MessagesSquare,
        _ if id.starts_with("pull-request") => IconName::GitPullRequest,
        _ => IconName::File,
    }
}

impl Root {
    fn render_side_chat_surface(&self, cx: &Context<Self>) -> AnyElement {
        let title = self
            .current()
            .and_then(|(environment, id)| self.thread_value(&environment, &id))
            .and_then(|thread| thread["title"].as_str().map(str::to_string))
            .unwrap_or_default();
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                h_flex()
                    .h(px(40.))
                    .px_3()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(text!("Side chat")),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("close-side-chat")
                            .icon(IconName::Close)
                            .ghost()
                            .compact()
                            .accessibility_label("Close side chat")
                            .on_click(
                                cx.listener(|root, _, _, cx| root.close_panel_tab("side-chat", cx)),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(text!("Context from"))
                    .child(text!("·"))
                    .child(text!(title)),
            )
            .child(
                div()
                    .id("side-chat-conversation")
                    .role(accesskit::Role::Region)
                    .aria_label("Side chat conversation")
                    .flex_1()
                    .min_h_0(),
            )
            .into_any_element()
    }
}
