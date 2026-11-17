use super::*;

pub(super) struct State {
    pub search: Entity<InputState>,
    pub query: String,
    pub selected: usize,
    pub matches: HashSet<(String, String)>,
    pub request: u64,
    pub pending: bool,
    pub error: Option<String>,
    pub scope_open: bool,
    pub scope_search: Entity<InputState>,
    pub scope_query: String,
    pub scope_keys: Vec<String>,
    pub scope_index: usize,
    pub focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl State {
    pub fn new(window: &mut Window, cx: &mut Context<Root>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let scope_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search projects..."));
        let subscriptions = vec![
            cx.subscribe_in(
                &search,
                window,
                |root, input, event, window, cx| match event {
                    InputEvent::Change => {
                        root.search_sidebar(input.read(cx).value().to_string(), cx)
                    }
                    InputEvent::PressEnter { .. } if !root.sidebar_header.query.is_empty() => {
                        if let Some(entry) = root
                            .visible_entries()
                            .get(root.sidebar_header.selected)
                            .cloned()
                        {
                            root.sidebar_header
                                .search
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            root.open_thread(entry.environment, entry.thread_id, cx);
                            root.conversation
                                .composer
                                .update(cx, |input, cx| input.focus(window, cx));
                        }
                    }
                    _ => {}
                },
            ),
            cx.subscribe(&scope_search, |root, input, event, cx| {
                if matches!(event, InputEvent::Change) {
                    root.sidebar_header.scope_query = input.read(cx).value().to_string();
                    root.sidebar_header.scope_index = 0;
                    cx.notify();
                }
            }),
        ];
        Self {
            search,
            query: String::new(),
            selected: 0,
            matches: HashSet::new(),
            request: 0,
            pending: false,
            error: None,
            scope_open: false,
            scope_search,
            scope_query: String::new(),
            scope_keys: Vec::new(),
            scope_index: 0,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }
}

impl Root {
    fn search_sidebar(&mut self, query: String, cx: &mut Context<Self>) {
        self.sidebar_header.query = query.trim().chars().take(200).collect();
        self.sidebar_header.matches.clear();
        self.sidebar_header.selected = 0;
        self.sidebar_header.error = None;
        self.sidebar_header.request += 1;
        self.sidebar_header.pending = self.sidebar_header.query.chars().count() >= 2;
        cx.notify();
        if !self.sidebar_header.pending {
            return;
        }
        let request = self.sidebar_header.request;
        let query = self.sidebar_header.query.clone();
        let connections = self.connections.clone();
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                let mut matches = HashSet::new();
                for connection in connections {
                    let value = connection
                        .request(
                            "orchestration.searchThreads",
                            json!({"query":query,"limit":50}),
                        )
                        .await?;
                    for item in value["matches"]
                        .as_array()
                        .context("Thread search matches missing")?
                    {
                        matches.insert((
                            connection.label().to_string(),
                            item["threadId"]
                                .as_str()
                                .context("Search thread ID missing")?
                                .to_string(),
                        ));
                    }
                }
                Ok(matches)
            })
            .await;
            let _ = this.update(cx, |root, cx| {
                if root.sidebar_header.request != request {
                    return;
                }
                root.sidebar_header.pending = false;
                match result {
                    Ok(matches) => root.sidebar_header.matches = matches,
                    Err(error) => root.sidebar_header.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn sidebar_scope_items(&self) -> Vec<(String, String)> {
        std::iter::once(("all".into(), "All projects".into()))
            .chain(
                self.project_groups()
                    .into_iter()
                    .map(|group| (group.key, group.title)),
            )
            .chain(
                self.connections
                    .iter()
                    .filter(|_| self.connections.len() > 1)
                    .map(|connection| {
                        (
                            format!("host:{}", connection.label()),
                            self.host_label(connection.label()),
                        )
                    }),
            )
            .filter(|(_, label)| {
                label
                    .to_lowercase()
                    .contains(&self.sidebar_header.scope_query.to_lowercase())
            })
            .collect()
    }

    fn choose_sidebar_scope(
        &mut self,
        key: &str,
        multiple: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if key == "all" {
            self.sidebar_header.scope_keys.clear();
        } else if let Some(host) = key.strip_prefix("host:") {
            self.host_filter = (self.host_filter.as_deref() != Some(host)).then(|| host.into());
            cx.notify();
            return;
        } else if multiple {
            if self
                .sidebar_header
                .scope_keys
                .iter()
                .any(|value| value == key)
            {
                self.sidebar_header.scope_keys.retain(|value| value != key);
            } else {
                self.sidebar_header.scope_keys.push(key.into());
            }
            cx.notify();
            return;
        } else {
            self.sidebar_header.scope_keys = vec![key.into()];
        }
        self.sidebar_header.scope_open = false;
        self.sidebar_header.focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn render_sidebar_scope(&self, cx: &mut Context<Self>) -> AnyElement {
        let items = self.sidebar_scope_items();
        let keyboard_items = items.clone();
        div()
            .absolute()
            .inset_0()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|root, _, _, cx| {
                    root.sidebar_header.scope_open = false;
                    cx.notify();
                }),
            )
            .child(
                v_flex()
                    .id("sidebar-scope-popup")
                    .absolute()
                    .top(px(84.))
                    .left(px(8.))
                    .w(px(288.))
                    .bg(cx.theme().popover)
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded_md()
                    .shadow_lg()
                    .p_1()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_key_down(cx.listener(move |root, event: &KeyDownEvent, window, cx| {
                        let count = keyboard_items.len();
                        match event.keystroke.key.as_str() {
                            "escape" => {
                                root.sidebar_header.scope_open = false;
                                root.sidebar_header.focus.focus(window, cx);
                            }
                            "down" if count > 0 => {
                                root.sidebar_header.scope_index =
                                    (root.sidebar_header.scope_index + 1) % count
                            }
                            "up" if count > 0 => {
                                root.sidebar_header.scope_index =
                                    (root.sidebar_header.scope_index + count - 1) % count
                            }
                            "enter" if count > 0 => root.choose_sidebar_scope(
                                &keyboard_items[root.sidebar_header.scope_index.min(count - 1)].0,
                                event.keystroke.modifiers.shift,
                                window,
                                cx,
                            ),
                            _ => return,
                        }
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .child(
                        Input::new(&self.sidebar_header.scope_search)
                            .aria_label("Search projects")
                            .appearance(false),
                    )
                    .child(
                        v_flex()
                            .id("sidebar-project-options")
                            .role(accesskit::Role::ListBox)
                            .aria_label("Projects")
                            .children(items.into_iter().enumerate().map(
                                |(index, (key, title))| {
                                    let selected = if key == "all" {
                                        self.sidebar_header.scope_keys.is_empty()
                                    } else if let Some(host) = key.strip_prefix("host:") {
                                        self.host_filter.as_deref() == Some(host)
                                    } else {
                                        self.sidebar_header.scope_keys.contains(&key)
                                    };
                                    let icon = if selected {
                                        gpui_kit::assets::IconName::Check
                                    } else if key.starts_with("host:") {
                                        gpui_kit::assets::IconName::Monitor
                                    } else {
                                        gpui_kit::assets::IconName::Folder
                                    };
                                    Button::new(("sidebar-scope", index))
                                        .role(accesskit::Role::ListBoxOption)
                                        .accessibility_label(title.clone())
                                        .ghost()
                                        .w_full()
                                        .h(px(32.))
                                        .selected(selected)
                                        .when(index == self.sidebar_header.scope_index, |item| {
                                            item.bg(cx.theme().list_hover)
                                        })
                                        .child(
                                            h_flex()
                                                .w_full()
                                                .gap_2()
                                                .child(Icon::new(icon).size(px(16.)))
                                                .child(title),
                                        )
                                        .on_click(cx.listener(move |root, _, window, cx| {
                                            root.choose_sidebar_scope(
                                                &key,
                                                window.modifiers().shift,
                                                window,
                                                cx,
                                            )
                                        }))
                                },
                            )),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_sidebar_header(&self, cx: &Context<Self>) -> AnyElement {
        let groups = self.project_groups();
        let selected: Vec<_> = groups
            .iter()
            .filter(|group| self.sidebar_header.scope_keys.contains(&group.key))
            .map(|group| group.title.as_str())
            .collect();
        let label = if !selected.is_empty() {
            format!("Filter threads by project: {}", selected.join(", "))
        } else if let Some(host) = &self.host_filter {
            format!("Filter threads by host: {}", self.host_label(host))
        } else {
            "Filter threads by project".into()
        };
        h_flex()
            .px_2()
            .py_1()
            .gap_1()
            .child(
                h_flex()
                    .id("sidebar-search-input")
                    .min_w_0()
                    .flex_1()
                    .on_key_down(cx.listener(|root, event: &KeyDownEvent, window, cx| {
                        let count = root.visible_entries().len();
                        match event.keystroke.key.as_str() {
                            "escape" if !root.sidebar_header.query.is_empty() => {
                                root.sidebar_header
                                    .search
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                            }
                            "down" if count > 0 => {
                                root.sidebar_header.selected =
                                    (root.sidebar_header.selected + 1) % count
                            }
                            "up" if count > 0 => {
                                root.sidebar_header.selected =
                                    (root.sidebar_header.selected + count - 1) % count
                            }
                            _ => return,
                        }
                        window.prevent_default();
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .child(
                        Input::new(&self.sidebar_header.search)
                            .role(accesskit::Role::ComboBox)
                            .aria_label("Search threads")
                            .appearance(false)
                            .h(px(32.))
                            .prefix(Icon::new(IconName::Search).size(px(16.))),
                    )
                    .when(!self.sidebar_header.query.is_empty(), |row| {
                        row.child(
                            Button::new("clear-thread-search")
                                .icon(IconName::Close)
                                .ghost()
                                .size(px(20.))
                                .accessibility_label("Clear thread search")
                                .on_click(cx.listener(|root, _, window, cx| {
                                    root.sidebar_header.search.update(cx, |input, cx| {
                                        input.set_value("", window, cx);
                                        input.focus(window, cx);
                                    });
                                })),
                        )
                    }),
            )
            .when(!groups.is_empty(), |row| {
                row.child(
                    Button::new("sidebar-filter")
                        .role(accesskit::Role::ComboBox)
                        .icon(gpui_kit::assets::IconName::ListFilter)
                        .ghost()
                        .size(px(28.))
                        .accessibility_label(label.clone())
                        .tooltip(label)
                        .track_focus(&self.sidebar_header.focus)
                        .aria_expanded(self.sidebar_header.scope_open)
                        .on_click(cx.listener(|root, _, window, cx| {
                            root.sidebar_header.scope_open = !root.sidebar_header.scope_open;
                            root.sidebar_header.scope_search.update(cx, |input, cx| {
                                input.set_value("", window, cx);
                                input.focus(window, cx);
                            });
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("sidebar-add-project")
                        .icon(gpui_kit::assets::IconName::FolderPlus)
                        .ghost()
                        .size(px(28.))
                        .accessibility_label("Add project")
                        .tooltip("Add project")
                        .on_click(
                            cx.listener(|root, _, window, cx| root.open_add_project(window, cx)),
                        ),
                )
            })
            .child(
                Button::new("sidebar-new")
                    .icon(gpui_kit::assets::IconName::SquarePen)
                    .ghost()
                    .size(px(28.))
                    .accessibility_label("New thread")
                    .tooltip("New thread")
                    .on_click(cx.listener(|root, _, window, cx| root.new_thread(window, cx))),
            )
            .into_any_element()
    }
}
