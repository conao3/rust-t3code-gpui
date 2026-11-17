use super::*;

impl Root {
    pub(super) fn toggle_file_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.file_picker_open = !self.file_picker_open;
        if self.file_picker_open {
            self.file_hits.clear();
            self.file_picker.update(cx, |state, cx| {
                state.set_query("", window, cx);
                state.focus(window, cx);
            });
        } else {
            self.composer
                .update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    fn search_files(&mut self, query: String, cx: &mut Context<Self>) {
        let Some((environment, cwd)) = self.composer_cwd() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let runtime = self.runtime.clone();
        let target = self.current();
        cx.spawn(async move |this, cx| {
            let found = on_runtime(&runtime, async move {
                connection.search_entries(&cwd, &query, 50).await
            })
            .await;
            let _ = this.update(cx, |root, cx| {
                if root.current() != target {
                    return;
                }
                root.file_hits = found
                    .ok()
                    .and_then(|r| r.get("entries").and_then(Value::as_array).cloned())
                    .unwrap_or_default()
                    .iter()
                    .filter(|e| e.get("kind").and_then(Value::as_str) == Some("file"))
                    .filter_map(|e| e.get("path").and_then(Value::as_str).map(str::to_string))
                    .collect();
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn open_file(&mut self, path: String, cx: &mut Context<Self>) {
        let Some((environment, cwd)) = self.composer_cwd() else {
            self.message = Some("Open a thread or a new thread first.".into());
            cx.notify();
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let runtime = self.runtime.clone();
        let target = self.current();
        cx.spawn(async move |this, cx| {
            let read = on_runtime(&runtime, {
                let path = path.clone();
                async move { connection.read_file(&cwd, &path).await }
            })
            .await;
            let _ = this.update(cx, |root, cx| {
                if root.current() != target {
                    return;
                }
                match read {
                    Ok(file) => {
                        let contents = file
                            .get("contents")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let note = (file.get("truncated").and_then(Value::as_bool) == Some(true))
                            .then(|| {
                                "The file is longer than T3 sends; the rest is left out."
                                    .to_string()
                            });
                        let parts = document_parts(&path, contents, cx);
                        let lines = contents.lines().map(str::to_string).collect();
                        root.install_panel_file(
                            FilePanel::new(path, PanelBody::Document(parts), note, lines),
                            cx,
                        );
                        root.file_find_again(cx);
                    }
                    Err(error) => root.message = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_file_picker(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let weak = cx.entity().downgrade();
        let search = weak.clone();
        let closer = move |window: &mut Window, cx: &mut App| {
            let _ = weak.update(cx, |root, cx| {
                root.file_picker_open = false;
                root.composer
                    .update(cx, |state, cx| state.focus(window, cx));
                cx.notify();
            });
        };
        let close_on_cancel = closer.clone();
        div()
            .absolute()
            .top(px(80.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(640.))
                    .rounded_md()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .shadow_lg()
                    .child(
                        Command::new(&self.file_picker)
                            .placeholder("Open a file in the project")
                            .filterable(false)
                            .max_h(px(480.))
                            .items(self.file_hits.iter().map(|path| {
                                CommandItem::new()
                                    .label(path.clone())
                                    .action(Box::new(OpenFile { path: path.clone() }))
                            }))
                            .on_query(move |query, _, cx| {
                                let query = query.to_string();
                                let _ = search.update(cx, |root, cx| root.search_files(query, cx));
                            })
                            .on_confirm(move |_, window, cx| closer(window, cx))
                            .on_cancel(move |window, cx| close_on_cancel(window, cx)),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn toggle_explorer(&mut self, cx: &mut Context<Self>) {
        self.activate_panel("files".into(), cx);
        if self.explorer.is_some() {
            return;
        }
        let Some((environment, cwd)) = self.composer_cwd() else {
            self.message = Some("Open a thread or a new thread first.".into());
            cx.notify();
            return;
        };
        self.start_explorer(environment, cwd, cx);
    }

    fn start_explorer(&mut self, environment: String, cwd: String, cx: &mut Context<Self>) {
        self.explorer = Some(Explorer {
            environment,
            cwd,
            directories: HashMap::new(),
            expanded: HashSet::new(),
            error: None,
        });
        self.load_directory(String::new(), cx);
        cx.notify();
    }

    /// Keeps the open explorer on the checkout of the thread or new thread in front.
    pub(super) fn follow_explorer(&mut self, cx: &mut Context<Self>) {
        let Some(open) = &self.explorer else { return };
        let Some((environment, cwd)) = self.composer_cwd() else {
            return;
        };
        if open.environment != environment || open.cwd != cwd {
            self.start_explorer(environment, cwd, cx);
        }
    }

    fn load_directory(&mut self, directory: String, cx: &mut Context<Self>) {
        let Some(open) = &self.explorer else { return };
        let Some(connection) = self.connection(&open.environment) else {
            return;
        };
        let (environment, cwd) = (open.environment.clone(), open.cwd.clone());
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let listed = on_runtime(&runtime, {
                let (cwd, directory) = (cwd.clone(), directory.clone());
                async move { connection.list_entries(&cwd, &directory).await }
            })
            .await;
            let _ = this.update(cx, |root, cx| {
                let Some(open) = root.explorer.as_mut() else {
                    return;
                };
                if open.environment != environment || open.cwd != cwd {
                    return;
                }
                match listed {
                    Ok(result) => {
                        let entries = explorer::children(&result, &directory);
                        open.directories.insert(directory, entries);
                        open.error = None;
                    }
                    Err(error) => {
                        open.expanded.remove(&directory);
                        open.error = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn click_entry(&mut self, entry: explorer::Entry, cx: &mut Context<Self>) {
        if !entry.directory {
            self.open_file(entry.path, cx);
            return;
        }
        let Some(open) = self.explorer.as_mut() else {
            return;
        };
        if !open.expanded.remove(&entry.path) {
            open.expanded.insert(entry.path.clone());
            if !open.directories.contains_key(&entry.path) {
                self.load_directory(entry.path, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn render_explorer(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let open = self.explorer.as_ref()?;
        let theme = cx.theme();
        let lines = Rc::new(explorer::lines(&open.directories, &open.expanded));
        let current = self.file_panel.as_ref().map(|p| p.path.clone());
        let (muted, selected) = (theme.muted_foreground, theme.muted);
        let weak = cx.entity().downgrade();
        let loading_root = !open.directories.contains_key("");
        Some(
            v_flex()
                .w_full()
                .h_full()
                .flex_none()
                .border_l_1()
                .border_color(theme.border)
                .child(
                    h_flex()
                        .px_3()
                        .py_2()
                        .gap_2()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(div().flex_1().text_sm().child("Files"))
                        .child(
                            Button::new("close-explorer")
                                .label("Close")
                                .compact()
                                .outline()
                                .on_click(cx.listener(|root, _, _, cx| {
                                    root.close_panel_tab("files", cx);
                                    cx.notify();
                                })),
                        ),
                )
                .children(open.error.clone().map(|error| {
                    div()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .text_color(theme.danger)
                        .child(error)
                }))
                .when(loading_root, |pane| {
                    pane.child(
                        div()
                            .px_3()
                            .py_2()
                            .text_xs()
                            .text_color(muted)
                            .child("Loading…"),
                    )
                })
                .child(
                    uniform_list("explorer-lines", lines.len(), move |range, _, _| {
                        range
                            .map(|i| {
                                let line = &lines[i];
                                let mark = match (line.entry.directory, line.expanded) {
                                    (true, true) => "▾ ",
                                    (true, false) => "▸ ",
                                    _ => "  ",
                                };
                                let label = format!(
                                    "{mark}{}{}",
                                    line.entry.name(),
                                    if line.loading { "  …" } else { "" }
                                );
                                let entry = line.entry.clone();
                                let weak = weak.clone();
                                div()
                                    .id(SharedString::from(format!("explorer-{}", line.entry.path)))
                                    .pl(px(12. + 14. * line.depth as f32))
                                    .pr_2()
                                    .text_sm()
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .cursor_pointer()
                                    .when(current.as_deref() == Some(&line.entry.path), |d| {
                                        d.bg(selected)
                                    })
                                    .hover(move |d| d.bg(selected))
                                    .on_click(move |_, _, cx| {
                                        let _ = weak.update(cx, |root, cx| {
                                            root.click_entry(entry.clone(), cx)
                                        });
                                    })
                                    .child(label)
                                    .into_any_element()
                            })
                            .collect()
                    })
                    .flex_1(),
                )
                .into_any_element(),
        )
    }

    pub(super) fn render_file_panel(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let panel = self.file_panel.as_ref()?;
        let theme = cx.theme();
        let finding = self.file_find_open && !self.file_find_input.read(cx).value().is_empty();
        let count = if panel.hits.is_empty() {
            "No match".to_string()
        } else {
            format!("{} / {}", panel.hit_at + 1, panel.hits.len())
        };
        let body = if finding {
            self.render_found_lines(panel, cx)
        } else {
            match &panel.body {
                PanelBody::Document(parts) => div()
                    .id("file-view")
                    .flex_1()
                    .overflow_y_scroll()
                    .p_3()
                    .children(
                        parts
                            .iter()
                            .enumerate()
                            .map(|(ix, part)| self.render_panel_part(ix, part, cx)),
                    )
                    .into_any_element(),
                PanelBody::Diff(state) => div().flex_1().child(Diff::new(state)).into_any_element(),
            }
        };
        Some(
            v_flex()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|root, _, window, cx| {
                        if !root.panel_focus.contains_focused(window, cx) {
                            root.panel_focus.focus(window, cx);
                        }
                    }),
                )
                .w_full()
                .h_full()
                .flex_none()
                .border_l_1()
                .border_color(theme.border)
                .child(
                    h_flex()
                        .px_3()
                        .py_2()
                        .gap_2()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .child(panel.path.clone()),
                        )
                        .child(
                            Button::new("find-file")
                                .label("Find")
                                .compact()
                                .outline()
                                .on_click(cx.listener(|root, _, window, cx| {
                                    root.open_file_find(window, cx)
                                })),
                        )
                        .child(
                            Button::new("close-file")
                                .label("Close")
                                .compact()
                                .outline()
                                .on_click(cx.listener(|root, _, window, cx| {
                                    root.close_file_panel(window, cx)
                                })),
                        ),
                )
                .when(self.file_find_open, |panel| {
                    panel.child(
                        h_flex()
                            .px_3()
                            .py_1()
                            .gap_2()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(div().flex_1().child(Input::new(&self.file_find_input)))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(count),
                            )
                            .child(
                                Button::new("file-find-previous")
                                    .label("Previous")
                                    .compact()
                                    .outline()
                                    .on_click(
                                        cx.listener(|root, _, _, cx| {
                                            root.step_file_find(false, cx)
                                        }),
                                    ),
                            )
                            .child(
                                Button::new("file-find-next")
                                    .label("Next")
                                    .compact()
                                    .outline()
                                    .on_click(
                                        cx.listener(|root, _, _, cx| root.step_file_find(true, cx)),
                                    ),
                            ),
                    )
                })
                .children(panel.note.clone().map(|note| {
                    div()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(note)
                }))
                .child(body)
                .into_any_element(),
        )
    }

    fn render_panel_part(&self, ix: usize, part: &PanelPart, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (rows, header_rows, sort) = match part {
            PanelPart::Text(view) => {
                return TextView::new(view).selectable(true).into_any_element();
            }
            PanelPart::Table {
                rows,
                header_rows,
                sort,
            } => (rows, *header_rows, *sort),
        };
        let columns = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let cell = |text: String, header: bool| {
            div()
                .px_2()
                .py_1()
                .border_b_1()
                .border_color(theme.border)
                .text_sm()
                .when(header, |d| d.font_weight(FontWeight::BOLD))
                .child(text)
        };
        let mut grid = div().grid().grid_cols(columns as u16).w_full();
        for row in rows.iter().take(header_rows) {
            for column in 0..columns {
                let text = row.get(column).cloned().unwrap_or_default();
                let mark = match sort {
                    Some(tables::Sort::Ascending(c)) if c == column => " ↑",
                    Some(tables::Sort::Descending(c)) if c == column => " ↓",
                    _ => "",
                };
                grid = grid.child(
                    cell(format!("{text}{mark}"), true)
                        .id(SharedString::from(format!("table-{ix}-head-{column}")))
                        .cursor_pointer()
                        .hover(|d| d.bg(theme.muted))
                        .on_click(
                            cx.listener(move |root, _, _, cx| root.sort_table(ix, column, cx)),
                        ),
                );
            }
        }
        for row in tables::sorted(rows, header_rows, sort) {
            for column in 0..columns {
                grid = grid.child(cell(row.get(column).cloned().unwrap_or_default(), false));
            }
        }
        v_flex()
            .my_2()
            .gap_1()
            .child(
                h_flex()
                    .gap_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("Header rows: {header_rows}"))
                    .child(
                        Button::new(SharedString::from(format!("table-{ix}-fewer")))
                            .label("−")
                            .compact()
                            .ghost()
                            .on_click(cx.listener(move |root, _, _, cx| {
                                root.change_header_rows(ix, false, cx)
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("table-{ix}-more")))
                            .label("+")
                            .compact()
                            .ghost()
                            .on_click(cx.listener(move |root, _, _, cx| {
                                root.change_header_rows(ix, true, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .border_1()
                    .border_color(theme.border)
                    .rounded_md()
                    .child(grid),
            )
            .into_any_element()
    }

    fn render_found_lines(&self, panel: &FilePanel, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let lines = panel.lines.clone();
        let hits = panel.hits.clone();
        let current = hits.get(panel.hit_at).cloned();
        let (plain, muted, mark, strong) = (
            theme.foreground,
            theme.muted_foreground,
            theme.warning.opacity(0.35),
            theme.warning,
        );
        let mono = theme.mono_font_family.clone();
        let width = lines.len().to_string().len();
        uniform_list("file-find-lines", lines.len(), move |range, _, _| {
            range
                .map(|n| {
                    let from = hits.partition_point(|(line, _)| *line < n);
                    let highlights: Vec<_> = hits[from..]
                        .iter()
                        .take_while(|(line, _)| *line == n)
                        .map(|(line, bytes)| {
                            let here = current.as_ref() == Some(&(*line, bytes.clone()));
                            (
                                bytes.clone(),
                                HighlightStyle {
                                    background_color: Some(if here { strong } else { mark }),
                                    ..Default::default()
                                },
                            )
                        })
                        .collect();
                    h_flex()
                        .px_3()
                        .gap_3()
                        .text_xs()
                        .whitespace_nowrap()
                        .child(div().text_color(muted).child(format!("{:>width$}", n + 1)))
                        .child(
                            div().text_color(plain).child(
                                StyledText::new(lines[n].clone()).with_highlights(highlights),
                            ),
                        )
                        .into_any_element()
                })
                .collect()
        })
        .track_scroll(&panel.scroll)
        .font_family(mono)
        .flex_1()
        .into_any_element()
    }

    fn sort_table(&mut self, part: usize, column: usize, cx: &mut Context<Self>) {
        if let Some(PanelPart::Table { sort, .. }) = self.panel_part(part) {
            *sort = tables::Sort::next(*sort, column);
            self.panel_revision += 1;
            cx.notify();
        }
    }

    fn change_header_rows(&mut self, part: usize, more: bool, cx: &mut Context<Self>) {
        if let Some(PanelPart::Table {
            rows, header_rows, ..
        }) = self.panel_part(part)
        {
            *header_rows = if more {
                (*header_rows + 1).min(rows.len().saturating_sub(1)).max(1)
            } else {
                header_rows.saturating_sub(1)
            };
            self.panel_revision += 1;
            cx.notify();
        }
    }

    fn panel_part(&mut self, part: usize) -> Option<&mut PanelPart> {
        match &mut self.file_panel.as_mut()?.body {
            PanelBody::Document(parts) => parts.get_mut(part),
            PanelBody::Diff(_) => None,
        }
    }

    pub(super) fn close_file_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.right_panel.active.clone() {
            self.close_panel_tab(&id, cx);
        }
        self.file_find_open = false;
        self.composer
            .update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub(super) fn open_file_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_panel.is_none() {
            return;
        }
        self.file_find_open = true;
        self.file_find_input
            .update(cx, |state, cx| state.focus(window, cx));
        self.file_find_again(cx);
    }

    pub(super) fn close_file_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.file_find_open = false;
        self.panel_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn file_find_again(&mut self, cx: &mut Context<Self>) {
        let query = self.file_find_input.read(cx).value().to_string();
        let Some(panel) = self.file_panel.as_mut() else {
            return;
        };
        panel.hits = Rc::new(completion::find_in_lines(&panel.lines, &query));
        panel.hit_at = 0;
        reveal_hit(panel);
        cx.notify();
    }

    pub(super) fn step_file_find(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(panel) = self.file_panel.as_mut() else {
            return;
        };
        let n = panel.hits.len();
        if n == 0 {
            return;
        }
        panel.hit_at = if forward {
            (panel.hit_at + 1) % n
        } else {
            (panel.hit_at + n - 1) % n
        };
        reveal_hit(panel);
        cx.notify();
    }
}

impl FilePanel {
    pub(super) fn new(
        path: String,
        body: PanelBody,
        note: Option<String>,
        lines: Vec<String>,
    ) -> Self {
        Self {
            path,
            body,
            note,
            lines: Rc::new(lines),
            hits: Rc::default(),
            hit_at: 0,
            scroll: UniformListScrollHandle::new(),
        }
    }
}

/// The parts the file preview shows: a markdown file with its tables taken out so they can be
/// sorted, anything else as one fenced block.
fn document_parts(path: &str, contents: &str, cx: &mut App) -> Vec<PanelPart> {
    let markdown = completion::file_markdown(path, contents);
    if markdown != contents {
        return vec![PanelPart::Text(
            cx.new(|cx| TextViewState::markdown(&markdown, cx)),
        )];
    }
    tables::split(contents)
        .into_iter()
        .map(|segment| match segment {
            tables::Segment::Markdown(text) => {
                PanelPart::Text(cx.new(|cx| TextViewState::markdown(&text, cx)))
            }
            tables::Segment::Table(rows) => PanelPart::Table {
                rows: Rc::new(rows),
                header_rows: 1,
                sort: None,
            },
        })
        .collect()
}

fn reveal_hit(panel: &FilePanel) {
    if let Some((line, _)) = panel.hits.get(panel.hit_at) {
        panel.scroll.scroll_to_item(*line, ScrollStrategy::Center);
    }
}
