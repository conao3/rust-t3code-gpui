use super::*;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = t3code_gpui, no_json)]
pub(super) struct SelectDiff {
    pub scope: String,
    pub commit: Option<String>,
}

pub(super) struct State {
    pub scope: String,
    pub commit: Option<String>,
    pub split: bool,
    pub wrap: bool,
    pub ignore_whitespace: bool,
    pub collapsed: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub request: u64,
    pub commits: Vec<Value>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            scope: "branch-range".into(),
            commit: None,
            split: false,
            wrap: true,
            ignore_whitespace: true,
            collapsed: true,
            loading: false,
            error: None,
            request: 0,
            commits: Vec::new(),
        }
    }
}

impl Root {
    pub(super) fn select_diff(&mut self, selection: &SelectDiff, cx: &mut Context<Self>) {
        self.diff_view.scope = selection.scope.clone();
        self.diff_view.commit = selection.commit.clone();
        self.refresh_diff(cx);
    }

    pub(super) fn refresh_diff(&mut self, cx: &mut Context<Self>) {
        let Some((environment, cwd)) = self.composer_cwd() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        self.diff_view.request += 1;
        let request = self.diff_view.request;
        self.diff_view.loading = true;
        self.diff_view.error = None;
        let target = self.current();
        let mut input = json!({"cwd":cwd,"ignoreWhitespace":self.diff_view.ignore_whitespace});
        if let Some(commit) = &self.diff_view.commit {
            input["commitSha"] = json!(commit);
        } else {
            input["sourceKind"] = json!(self.diff_view.scope);
        }
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                let (preview, commits) = tokio::join!(
                    connection.request("review.getDiffPreview", input),
                    connection.request("review.getBranchCommits", json!({"cwd":cwd}))
                );
                Ok((preview?, commits))
            })
            .await;
            let result = match result {
                Ok((preview, commits)) => {
                    cx.background_spawn(async move {
                        let mut files = Vec::new();
                        for source in preview["sources"].as_array().into_iter().flatten() {
                            files.extend(
                                DiffFile::parse_with(
                                    source["diff"].as_str().unwrap_or_default(),
                                    DiffParseOptions::new().truncated_hunks(true),
                                )
                                .map_err(|error| anyhow::anyhow!("Diff parse error: {error:?}"))?,
                            );
                        }
                        let lines = completion::diff_lines(&preview)
                            .iter()
                            .map(|line| line.text().to_string())
                            .collect();
                        Ok((
                            files,
                            lines,
                            commits,
                            preview["incomplete"].as_bool() == Some(true),
                        ))
                    })
                    .await
                }
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |root, cx| {
                if root.diff_view.request != request || root.current() != target {
                    return;
                }
                root.diff_view.loading = false;
                match result {
                    Ok((files, lines, commits, incomplete)) => {
                        match commits {
                            Ok(value) => {
                                root.diff_view.commits =
                                    value["commits"].as_array().cloned().unwrap_or_default()
                            }
                            Err(error) => {
                                root.diff_view.error =
                                    Some(format!("Could not list commits: {error:#}"))
                            }
                        }
                        let mode = if root.diff_view.split {
                            DiffMode::Split
                        } else {
                            DiffMode::Unified
                        };
                        let state = cx.new(|cx| {
                            let mut state = DiffState::new(files, cx)
                                .with_mode(mode)
                                .with_syntax_max_line_length(500);
                            if root.diff_view.collapsed {
                                let paths = state
                                    .files()
                                    .iter()
                                    .map(|file| file.path().to_string())
                                    .collect::<Vec<_>>();
                                for path in paths {
                                    state.set_file_collapsed(&path, true, cx);
                                }
                            }
                            state
                        });
                        let panel = FilePanel::new(
                            DIFF_TITLE.into(),
                            PanelBody::Diff(state),
                            incomplete.then(|| "The server returned an incomplete diff.".into()),
                            lines,
                        );
                        if root.right_panel.active.as_deref() == Some("diff") {
                            root.install_panel_file(panel, cx);
                        }
                    }
                    Err(error) => root.diff_view.error = Some(format!("{error:#}")),
                }
                root.panel_revision += 1;
                cx.notify();
            });
        })
        .detach();
        self.panel_revision += 1;
        cx.notify();
    }

    pub(super) fn render_diff_toolbar(&self, cx: &Context<Self>) -> AnyElement {
        let focus = self.focus.clone();
        let commits = self.diff_view.commits.clone();
        let scope = self.diff_view.scope.clone();
        let label = if let Some(commit) = &self.diff_view.commit {
            commit.chars().take(7).collect::<String>()
        } else if scope == "branch-range" {
            "Changes".into()
        } else {
            "Uncommitted".into()
        };
        v_flex()
            .child(
                h_flex()
                    .gap_1()
                    .p_2()
                    .child(
                        Button::new("diff-scope")
                            .label(label.clone())
                            .accessibility_label(format!("Diff scope: {label}"))
                            .ghost()
                            .compact()
                            .dropdown_menu(move |mut menu, _, _| {
                                for (label, selected) in
                                    [("Uncommitted", "working-tree"), ("Changes", "branch-range")]
                                {
                                    menu = menu.menu_with_check(
                                        label,
                                        scope == selected,
                                        Box::new(SelectDiff {
                                            scope: selected.into(),
                                            commit: None,
                                        }),
                                    );
                                }
                                for commit in &commits {
                                    menu = menu.menu(
                                        commit["subject"].as_str().unwrap_or_default().to_string(),
                                        Box::new(SelectDiff {
                                            scope: "commit".into(),
                                            commit: commit["sha"].as_str().map(str::to_string),
                                        }),
                                    );
                                }
                                menu.action_context(focus.clone())
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("refresh-diff")
                            .icon(IconName::RefreshCw)
                            .accessibility_label("Refresh diff")
                            .ghost()
                            .compact()
                            .disabled(self.diff_view.loading)
                            .on_click(cx.listener(|root, _, _, cx| root.refresh_diff(cx))),
                    )
                    .child(
                        h_flex()
                            .id("diff-layout")
                            .role(accesskit::Role::Group)
                            .aria_label("Diff layout")
                            .children(
                                [(false, "Stacked diff view"), (true, "Split diff view")]
                                    .into_iter()
                                    .map(|(split, name)| {
                                        Button::new(name)
                                            .icon(if split {
                                                gpui_kit::assets::IconName::Columns2
                                            } else {
                                                gpui_kit::assets::IconName::Rows3
                                            })
                                            .accessibility_label(name)
                                            .ghost()
                                            .compact()
                                            .selected(self.diff_view.split == split)
                                            .toggled(self.diff_view.split == split)
                                            .on_click(cx.listener(move |root, _, _, cx| {
                                                root.diff_view.split = split;
                                                if let Some(FilePanel {
                                                    body: PanelBody::Diff(state),
                                                    ..
                                                }) = &root.file_panel
                                                {
                                                    state.update(cx, |state, cx| {
                                                        state.set_mode(
                                                            if split {
                                                                DiffMode::Split
                                                            } else {
                                                                DiffMode::Unified
                                                            },
                                                            cx,
                                                        )
                                                    });
                                                }
                                                root.panel_revision += 1;
                                                cx.notify();
                                            }))
                                    }),
                            ),
                    )
                    .child(
                        Button::new("diff-wrap")
                            .icon(gpui_kit::assets::IconName::TextWrap)
                            .accessibility_label(if self.diff_view.wrap {
                                "Disable diff line wrapping"
                            } else {
                                "Enable diff line wrapping"
                            })
                            .ghost()
                            .compact()
                            .selected(self.diff_view.wrap)
                            .toggled(self.diff_view.wrap)
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.diff_view.wrap = !root.diff_view.wrap;
                                root.panel_revision += 1;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("ignore-diff-whitespace")
                            .label("¶")
                            .accessibility_label(if self.diff_view.ignore_whitespace {
                                "Show whitespace changes"
                            } else {
                                "Hide whitespace changes"
                            })
                            .ghost()
                            .compact()
                            .selected(self.diff_view.ignore_whitespace)
                            .toggled(self.diff_view.ignore_whitespace)
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.diff_view.ignore_whitespace =
                                    !root.diff_view.ignore_whitespace;
                                root.refresh_diff(cx);
                            })),
                    )
                    .child(
                        Button::new("collapse-diff-files")
                            .icon(gpui_kit::assets::IconName::ChevronsUpDown)
                            .accessibility_label(if self.diff_view.collapsed {
                                "Expand all files"
                            } else {
                                "Collapse all files"
                            })
                            .ghost()
                            .compact()
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.diff_view.collapsed = !root.diff_view.collapsed;
                                let collapsed = root.diff_view.collapsed;
                                if let Some(FilePanel {
                                    body: PanelBody::Diff(state),
                                    ..
                                }) = &root.file_panel
                                {
                                    state.update(cx, |state, cx| {
                                        let paths = state
                                            .files()
                                            .iter()
                                            .map(|file| file.path().to_string())
                                            .collect::<Vec<_>>();
                                        for path in paths {
                                            state.set_file_collapsed(&path, collapsed, cx);
                                        }
                                    });
                                }
                                root.panel_revision += 1;
                                cx.notify();
                            })),
                    ),
            )
            .when(self.diff_view.loading, |toolbar| {
                toolbar.child(div().text_xs().px_2().child("Refreshing diff…"))
            })
            .children(self.diff_view.error.clone().map(|error| {
                div()
                    .text_xs()
                    .px_2()
                    .text_color(cx.theme().danger)
                    .child(error)
            }))
            .into_any_element()
    }
}
