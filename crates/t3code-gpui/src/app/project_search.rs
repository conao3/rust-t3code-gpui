use super::*;

pub(super) struct ProjectSearch {
    pub input: Entity<InputState>,
    pub selected: usize,
    rows: Vec<SearchRow>,
    list: ListState,
    _subscription: Subscription,
    pub open: bool,
    pub query: String,
    pub options: [bool; 3],
    pub matches: Vec<Value>,
    pub request: u64,
    pub loading: bool,
    pub truncated: bool,
    pub error: Option<String>,
    pub pending_match: Option<(String, usize)>,
}

enum SearchRow {
    Header(String, usize),
    Match(usize),
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = t3code_gpui, no_json)]
pub(super) struct OpenMatch {
    path: String,
    line: usize,
}

impl ProjectSearch {
    pub fn new(window: &mut Window, cx: &mut Context<Root>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search project contents"));
        let subscription =
            cx.subscribe(&input, |root, input, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    root.search_project_contents(input.read(cx).value().to_string(), cx)
                }
                InputEvent::PressEnter { .. } => {
                    if let Some(item) = root
                        .project_search
                        .matches
                        .get(root.project_search.selected)
                    {
                        let action = OpenMatch {
                            path: item["path"].as_str().unwrap_or_default().into(),
                            line: item["lineNumber"].as_u64().unwrap_or(1) as usize,
                        };
                        root.open_project_match(&action, cx);
                    }
                }
                _ => {}
            });
        Self {
            input,
            selected: 0,
            rows: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(300.)),
            _subscription: subscription,
            open: false,
            query: String::new(),
            options: [false; 3],
            matches: vec![],
            request: 0,
            loading: false,
            truncated: false,
            error: None,
            pending_match: None,
        }
    }
}

impl Root {
    pub(super) fn toggle_project_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.project_search.open = !self.project_search.open;
        self.palette_open = false;
        if self.project_search.open {
            let title = self
                .open_entry()
                .map(|entry| entry.project_title.clone())
                .or_else(|| {
                    self.draft_project()
                        .and_then(|(environment, project)| {
                            self.project_value(&environment, &project)
                        })
                        .and_then(|project| project["title"].as_str().map(str::to_owned))
                })
                .unwrap_or_else(|| "project".into());
            self.project_search.input.update(cx, |state, cx| {
                state.set_placeholder(format!("Search in {title}"), window, cx)
            });
            self.project_search
                .input
                .update(cx, |state, cx| state.focus(window, cx));
        } else {
            self.composer
                .update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    fn search_project_contents(&mut self, query: String, cx: &mut Context<Self>) {
        self.project_search.query = query.clone();
        self.project_search.request += 1;
        let request = self.project_search.request;
        self.project_search.matches.clear();
        self.project_search.rows.clear();
        self.project_search.list.reset(0);
        self.project_search.selected = 0;
        self.project_search.error = None;
        self.project_search.loading = false;
        if query.is_empty() {
            cx.notify();
            return;
        }
        let Some((environment, cwd)) = self.composer_cwd() else {
            self.project_search.error = Some("Open a project to search its files.".into());
            cx.notify();
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        self.project_search.loading = true;
        let [case_sensitive, whole_word, use_regex] = self.project_search.options;
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(150)).await;
            if this.read_with(cx, |root, _| root.project_search.request != request).unwrap_or(true) { return; }
            let result = on_runtime(&runtime, async move {
                connection.request("projects.searchContents", json!({"cwd":cwd,"query":query,"limit":500,"caseSensitive":case_sensitive,"wholeWord":whole_word,"useRegex":use_regex})).await
            }).await;
            let _ = this.update(cx, |root, cx| {
                if root.project_search.request != request { return; }
                root.project_search.loading = false;
                match result {
                    Ok(result) => {
                        root.project_search.matches = result["matches"].as_array().cloned().unwrap_or_default();
                        let mut groups: Vec<(String, Vec<usize>)> = vec![];
                        for (index, item) in root.project_search.matches.iter().enumerate() {
                            let path = item["path"].as_str().unwrap_or_default();
                            if let Some((_, indices)) = groups.iter_mut().find(|(name, _)| name == path) { indices.push(index); } else { groups.push((path.to_string(), vec![index])); }
                        }
                        root.project_search.rows = groups.into_iter().flat_map(|(path, indices)| std::iter::once(SearchRow::Header(path, indices.len())).chain(indices.into_iter().map(SearchRow::Match))).collect();
                        root.project_search.list.reset(root.project_search.rows.len());
                        root.project_search.truncated = result["truncated"].as_bool() == Some(true);
                        root.project_search.error = result["regexFallbackError"].as_str().map(|error| format!("Invalid regular expression: {error}"));
                    }
                    Err(error) => root.project_search.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub(super) fn open_project_match(&mut self, action: &OpenMatch, cx: &mut Context<Self>) {
        self.project_search.open = false;
        self.open_file(action.path.clone(), cx);
        self.project_search.pending_match =
            Some((action.path.clone(), action.line.saturating_sub(1)));
        cx.notify();
    }

    pub(super) fn reveal_project_match(&mut self) {
        if let Some((path, line)) = &self.project_search.pending_match
            && let Some(panel) = &self.file_panel
            && panel.path == *path
        {
            panel.scroll.scroll_to_item(*line, ScrollStrategy::Top);
            self.project_search.pending_match = None;
        }
    }

    fn move_project_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = self.project_search.matches.len();
        if count == 0 {
            return;
        }
        self.project_search.selected = if forward {
            (self.project_search.selected + 1).min(count - 1)
        } else {
            self.project_search.selected.saturating_sub(1)
        };
        let selected = self.project_search.selected;
        if let Some(index) = self
            .project_search
            .rows
            .iter()
            .position(|row| matches!(row, SearchRow::Match(value) if *value == selected))
        {
            self.project_search.list.scroll_to(ListOffset {
                item_ix: index,
                offset_in_item: px(0.),
            });
        }
        cx.notify();
    }

    fn render_project_search_row(&self, index: usize, cx: &Context<Self>) -> AnyElement {
        match self.project_search.rows.get(index) {
            Some(SearchRow::Header(path, count)) => h_flex()
                .w_full()
                .h(px(32.))
                .px_3()
                .gap_2()
                .bg(cx.theme().popover)
                .text_xs()
                .child(div().flex_1().truncate().child(path.clone()))
                .child(count.to_string())
                .into_any_element(),
            Some(SearchRow::Match(index)) => {
                let item = &self.project_search.matches[*index];
                let action = OpenMatch {
                    path: item["path"].as_str().unwrap_or_default().into(),
                    line: item["lineNumber"].as_u64().unwrap_or(1) as usize,
                };
                let content = item["lineContent"].as_str().unwrap_or_default();
                let label = format!("{} {content}", action.line);
                let byte_offset = |offset: usize| {
                    let mut units = 0;
                    for (byte, ch) in content.char_indices() {
                        if units >= offset {
                            return byte;
                        }
                        units += ch.len_utf16();
                    }
                    content.len()
                };
                let highlights: Vec<_> = item["matchRanges"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|range| {
                        let start = byte_offset(range["start"].as_u64().unwrap_or(0) as usize);
                        let end = byte_offset(range["end"].as_u64().unwrap_or(0) as usize);
                        (
                            start..end,
                            HighlightStyle {
                                background_color: Some(cx.theme().primary.opacity(0.3)),
                                ..Default::default()
                            },
                        )
                    })
                    .collect();
                Button::new(("project-search-match", *index))
                    .accessibility_label(label)
                    .ghost()
                    .compact()
                    .h(px(28.))
                    .w_full()
                    .px_3()
                    .rounded_none()
                    .when(*index == self.project_search.selected, |button| {
                        button.bg(cx.theme().accent)
                    })
                    .child(
                        h_flex()
                            .w_full()
                            .gap_3()
                            .font_family("monospace")
                            .text_xs()
                            .child(
                                div()
                                    .w(px(40.))
                                    .text_right()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(action.line.to_string()),
                            )
                            .child(div().flex_1().min_w_0().truncate().text_left().child(
                                StyledText::new(content.to_string()).with_highlights(highlights),
                            )),
                    )
                    .on_click(
                        cx.listener(move |root, _, _, cx| root.open_project_match(&action, cx)),
                    )
                    .into_any_element()
            }
            None => div().into_any_element(),
        }
    }

    pub(super) fn render_project_search(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let state = &self.project_search;
        let project = self
            .open_entry()
            .map(|entry| entry.project_title.clone())
            .or_else(|| {
                self.draft_project()
                    .and_then(|(environment, project)| self.project_value(&environment, &project))
                    .and_then(|project| project["title"].as_str().map(str::to_owned))
            })
            .unwrap_or_else(|| "project".into());
        let status = if state.loading {
            "Searching…".into()
        } else if let Some(error) = &state.error {
            error.clone()
        } else {
            let files: HashSet<_> = state
                .matches
                .iter()
                .filter_map(|item| item["path"].as_str())
                .collect();
            format!(
                "{}{} results in {} files",
                state.matches.len(),
                if state.truncated { "+" } else { "" },
                files.len()
            )
        };
        let root = cx.entity().downgrade();
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(black().opacity(0.5))
            .flex()
            .justify_center()
            .child(
                v_flex()
                    .id("project-content-search")
                    .w(px(576.))
                    .h(px(420.))
                    .mt(window.viewport_size().height * 0.1)
                    .rounded(px(16.))
                    .overflow_hidden()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .shadow_lg()
                    .role(accesskit::Role::Dialog)
                    .aria_label("Search project contents")
                    .capture_key_down(cx.listener(|root, event: &KeyDownEvent, window, cx| {
                        match event.keystroke.key.as_str() {
                            "up" => root.move_project_match(false, cx),
                            "down" => root.move_project_match(true, cx),
                            "escape" => root.toggle_project_search(window, cx),
                            _ => return,
                        }
                        cx.stop_propagation();
                    }))
                    .child(
                        h_flex()
                            .px_3()
                            .py_2()
                            .gap_2()
                            .child(
                                div().flex_1().child(
                                    Input::new(&state.input)
                                        .appearance(false)
                                        .focus_bordered(false)
                                        .prefix(Icon::new(IconName::Search).size(px(14.)))
                                        .role(accesskit::Role::ComboBox)
                                        .aria_label(format!("Search in {project}")),
                                ),
                            )
                            .children(
                                [
                                    ("Aa", "Match case"),
                                    ("ab", "Match whole word"),
                                    (".*", "Use regular expression"),
                                ]
                                .into_iter()
                                .enumerate()
                                .map(|(index, (label, name))| {
                                    Button::new(("search-option", index))
                                        .label(label)
                                        .accessibility_label(name)
                                        .selected(state.options[index])
                                        .ghost()
                                        .compact()
                                        .on_click(cx.listener(move |root, _, _, cx| {
                                            root.project_search.options[index] =
                                                !root.project_search.options[index];
                                            root.search_project_contents(
                                                root.project_search.query.clone(),
                                                cx,
                                            );
                                        }))
                                }),
                            ),
                    )
                    .child(
                        h_flex()
                            .h(px(36.))
                            .flex_none()
                            .px_3()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(status),
                    )
                    .child(
                        list(state.list.clone(), move |index, _, cx| {
                            root.update(cx, |root, cx| root.render_project_search_row(index, cx))
                                .unwrap_or_else(|_| div().into_any_element())
                        })
                        .flex_1()
                        .min_h_0()
                        .w_full(),
                    )
                    .child(
                        h_flex()
                            .h(px(40.))
                            .flex_none()
                            .px_3()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("↑ ↓ Navigate   Enter Open file   Esc Back"),
                    ),
            )
            .into_any_element()
    }
}
