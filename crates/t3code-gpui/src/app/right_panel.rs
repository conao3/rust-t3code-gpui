use super::*;

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(super) struct State {
    pub open: bool,
    pub maximized: bool,
    pub launcher: bool,
    pub tabs: Vec<String>,
    pub active: Option<String>,
    pub pull_requests: HashMap<String, (String, Value)>,
    #[serde(skip)]
    files: HashMap<String, FilePanel>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct Saved {
    pub width: f32,
    pub threads: Vec<((String, String), State)>,
}

pub(super) fn load(path: &std::path::Path) -> anyhow::Result<Saved> {
    match std::fs::read(path.with_file_name("right-panels.json")) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Saved {
            width: 540.,
            threads: Vec::new(),
        }),
        Err(error) => Err(error.into()),
    }
}

pub(super) struct Pane {
    pub root: WeakEntity<Root>,
}

impl Render for Pane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.root
            .update(cx, |root, cx| root.render_panel_contents(cx))
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
            self.draft.as_ref().map(|draft| {
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
        if let Some(previous) = self.right_panel_target.take() {
            self.right_panel_threads
                .insert(previous, std::mem::take(&mut self.right_panel));
        }
        self.right_panel = target
            .as_ref()
            .and_then(|target| self.right_panel_threads.remove(target))
            .unwrap_or_default();
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
                serde_json::to_vec(&json!({"width": self.right_panel_width, "threads": threads}))?,
            )?;
            std::fs::rename(temporary, path)?;
            Ok(())
        })();
        if let Err(error) = result {
            self.message = Some(format!("Could not save right panels: {error:#}"));
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
            self.composer
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
            "{}:{file:?}:{explorer:?}:{}:{}:{}",
            self.panel_revision,
            self.file_find_open,
            self.file_find_input.read(cx).value(),
            self.settings.light_theme
        );
        if self.panel_stamp != stamp {
            self.panel_stamp = stamp;
            window.mark_view_dirty(self.panel_pane.entity_id());
            window.mark_view_dirty(self.pr_pane.entity_id());
        }
    }

    fn render_panel_contents(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let tabs = h_flex()
            .id("panel-tabs")
            .role(accesskit::Role::TabList)
            .aria_label("Panel tabs")
            .h(px(40.))
            .gap_1()
            .px_2()
            .children(self.right_panel.tabs.iter().map(|id| {
                let label = panel_title(id);
                let select = id.clone();
                let close = id.clone();
                let menu_id = id.clone();
                let focus = self.focus.clone();
                h_flex()
                    .id(SharedString::from(format!("tab-context-{id}")))
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
                    .min_w_0()
                    .child(
                        Button::new(SharedString::from(id.clone()))
                            .label(label.clone())
                            .ghost()
                            .compact()
                            .role(accesskit::Role::Tab)
                            .selected(self.right_panel.active.as_ref() == Some(id))
                            .on_click(cx.listener(move |root, _, _, cx| {
                                root.activate_panel(select.clone(), cx);
                                root.load_active_surface(cx);
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("close-{id}")))
                            .icon(IconName::Close)
                            .ghost()
                            .size(px(20.))
                            .accessibility_label(format!("Close {label}"))
                            .on_click(
                                cx.listener(move |root, _, _, cx| root.close_panel_tab(&close, cx)),
                            ),
                    )
            }))
            .child(
                Button::new("add-panel")
                    .icon(IconName::Plus)
                    .ghost()
                    .size(px(24.))
                    .accessibility_label("Add panel surface")
                    .on_click(cx.listener(|root, _, window, cx| root.new_panel(window, cx))),
            )
            .child(div().flex_1())
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
                    .on_click(cx.listener(|root, _, _, cx| {
                        root.right_panel.maximized = !root.right_panel.maximized;
                        root.save_right_panel();
                        root.panel_revision += 1;
                        cx.notify();
                    })),
            );
        let body = if self.right_panel.active.is_none() {
            self.render_surface_launcher(cx)
        } else {
            match self.right_panel.active.as_deref() {
                Some(id) if id.starts_with("pull-request:") || id == "pull-requests" => {
                    self.render_pull_requests(cx)
                }
                Some("diff") => v_flex()
                    .h_full()
                    .min_h_0()
                    .child(self.render_diff_toolbar(cx))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .children(self.render_file_panel(cx)),
                    )
                    .into_any_element(),
                Some("files") => self
                    .render_explorer(cx)
                    .unwrap_or_else(|| div().child("Loading files…").into_any_element()),
                _ => self
                    .render_file_panel(cx)
                    .unwrap_or_else(|| div().child("Loading…").into_any_element()),
            }
        };
        v_flex()
            .id("workspace-panel")
            .role(accesskit::Role::Region)
            .aria_label("Workspace panel")
            .track_focus(&self.panel_focus)
            .on_key_down(cx.listener(|root, event: &KeyDownEvent, window, cx| {
                if root.right_panel.launcher || root.right_panel.active.is_none() {
                    match event.keystroke.key.as_str() {
                        "f" => root.toggle_explorer(cx),
                        "d" => root.toggle_diff(cx),
                        "p" => root.open_current_pull_request(cx),
                        "l" => root.open_pull_requests(false, cx),
                        "escape" => {
                            root.right_panel.launcher = false;
                            root.panel_revision += 1;
                            root.composer
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
            .child(tabs)
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
                            .top(px(40.))
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
                                    ("Files", "F"),
                                    ("Diff", "D"),
                                    ("Pull request", "P"),
                                    ("Linked pull requests", "L"),
                                ]
                                .into_iter()
                                .map(|(label, key)| {
                                    Button::new(label)
                                        .label(format!("{label}    {key}"))
                                        .accessibility_label(label)
                                        .role(accesskit::Role::MenuItem)
                                        .ghost()
                                        .w_full()
                                        .on_click(cx.listener(move |root, _, _, cx| {
                                            if label == "Files" {
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

    fn render_surface_launcher(&self, cx: &Context<Self>) -> AnyElement {
        v_flex()
            .id("surface-launcher")
            .aria_label("Open a surface")
            .flex_1()
            .justify_center()
            .px_6()
            .gap_1()
            .child(div().text_sm().mb_3().child("Open a surface"))
            .children(
                [
                    ("Files", "F"),
                    ("Diff", "D"),
                    ("Pull request", "P"),
                    ("Linked pull requests", "L"),
                ]
                .into_iter()
                .map(|(label, key)| {
                    Button::new(label)
                        .label(format!("{label}    {key}"))
                        .accessibility_label(label)
                        .ghost()
                        .w_full()
                        .on_click(cx.listener(move |root, _, _, cx| {
                            if label == "Files" {
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
        "diff" => "Diff".into(),
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
