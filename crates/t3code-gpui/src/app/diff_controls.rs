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
    pub base_ref: Option<String>,
    pub resolved_base: Option<String>,
    pub head_ref: Option<String>,
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
            base_ref: None,
            resolved_base: None,
            head_ref: None,
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
        self.diff_view.scope = if selection.scope == "latest" {
            let Some(count) = self.diff_turns().first().copied() else {
                return;
            };
            format!("turn:{count}")
        } else {
            selection.scope.clone()
        };
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
        if let Some(base) = &self.diff_view.base_ref {
            input["baseRef"] = json!(base);
        }
        let mut commit_input = json!({"cwd":cwd});
        if let Some(base) = &self.diff_view.base_ref {
            commit_input["baseRef"] = json!(base);
        }
        if let Some(commit) = &self.diff_view.commit {
            input["commitSha"] = json!(commit);
        } else {
            input["sourceKind"] = json!(self.diff_view.scope);
        }
        let turn_input = self.diff_view.scope.strip_prefix("turn:").and_then(|count| count.parse::<u64>().ok()).and_then(|count| target.as_ref().map(|(_, thread)| json!({"threadId":thread,"fromTurnCount":count.saturating_sub(1),"toTurnCount":count,"ignoreWhitespace":self.diff_view.ignore_whitespace})));
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                let (preview, commits) = tokio::join!(
                    async {
                        if let Some(input) = turn_input {
                            let result = connection
                                .request("orchestration.getTurnDiff", input)
                                .await?;
                            Ok(json!({"sources":[{"diff":result["diff"]}]}))
                        } else {
                            connection.request("review.getDiffPreview", input).await
                        }
                    },
                    connection.request("review.getBranchCommits", commit_input)
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
                            preview["sources"][0]["headRef"]
                                .as_str()
                                .map(str::to_string),
                            preview["sources"][0]["baseRef"]
                                .as_str()
                                .map(str::to_string),
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
                    Ok((files, lines, commits, incomplete, head, base)) => {
                        root.diff_view.head_ref = head;
                        root.diff_view.resolved_base = base;
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

    fn diff_turns(&self) -> Vec<u64> {
        let Some((environment, thread)) = self.current() else {
            return Vec::new();
        };
        let Some(thread) = self.thread_value(&environment, &thread) else {
            return Vec::new();
        };
        let mut turns = thread["checkpoints"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|checkpoint| checkpoint["runId"].is_string())
            .filter_map(|checkpoint| checkpoint["appRunOrdinal"].as_u64())
            .collect::<Vec<_>>();
        turns.sort_unstable_by(|a, b| b.cmp(a));
        turns
    }

    fn render_diff_base(&self, cx: &Context<Self>) -> AnyElement {
        let base = self
            .diff_view
            .resolved_base
            .as_deref()
            .or(self.diff_view.base_ref.as_deref())
            .unwrap_or("Automatic");
        let head = self
            .diff_view
            .head_ref
            .as_deref()
            .or(self.workspace_status.branch())
            .unwrap_or("HEAD");
        let button = Button::new("diff-base")
            .child(
                h_flex()
                    .gap_1()
                    .child(base.to_string())
                    .child(Icon::new(IconName::ChevronDown).size(px(12.))),
            )
            .ghost()
            .compact()
            .role(accesskit::Role::ComboBox)
            .accessibility_label(format!("Change comparison target. Currently {base}"))
            .on_click(cx.listener(|root, _, window, cx| {
                root.open_branch_picker("diff-base", window, cx);
                root.panel_revision += 1;
            }));
        let mut popup = gpui_kit::base::Popup::new("diff-base-popup", button)
            .anchor(Anchor::TopLeft)
            .offset(px(4.));
        if self.branch_anchor == Some("diff-base") {
            let weak = cx.entity().downgrade();
            let cancel = weak.clone();
            let mut items = vec![
                CommandItem::new()
                    .label("Automatic")
                    .checked(self.diff_view.base_ref.is_none())
                    .action(Box::new(WorkspaceAction {
                        kind: "diff-base".into(),
                        value: Value::Null,
                    })),
            ];
            items.extend(self.branch_refs.iter().map(|reference| {
                CommandItem::new()
                    .label(reference["name"].as_str().unwrap_or_default().to_string())
                    .checked(reference["name"].as_str() == self.diff_view.base_ref.as_deref())
                    .action(Box::new(WorkspaceAction {
                        kind: "diff-base".into(),
                        value: reference["name"].clone(),
                    }))
            }));
            popup = popup.content(
                v_flex()
                    .w(px(288.))
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().popover)
                    .child(
                        Command::new(&self.branch_picker)
                            .placeholder("Search refs...")
                            .filterable(false)
                            .items(items)
                            .on_query(move |query, _, cx| {
                                let _ = weak.update(cx, |root, cx| {
                                    root.search_branches(query.into(), false, cx)
                                });
                            })
                            .on_cancel(move |_, cx| {
                                let _ = cancel.update(cx, |root, cx| {
                                    root.branch_anchor = None;
                                    root.panel_revision += 1;
                                    cx.notify();
                                });
                            }),
                    ),
            );
        }
        h_flex()
            .gap_2()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(text!(head.to_string()))
            .child(Icon::new(IconName::ArrowRight).size(px(12.)))
            .child(popup)
            .into_any_element()
    }

    pub(super) fn render_diff_toolbar(&self, cx: &Context<Self>) -> AnyElement {
        let focus = self.focus.clone();
        let commits = self.diff_view.commits.clone();
        let turns = self.diff_turns();
        let stats = self
            .file_panel
            .as_ref()
            .and_then(|panel| match &panel.body {
                PanelBody::Diff(state) => Some(state.read(cx).files().iter().fold(
                    (0, 0),
                    |(additions, deletions), file| {
                        (additions + file.additions(), deletions + file.deletions())
                    },
                )),
                _ => None,
            });
        let scope = self.diff_view.scope.clone();
        let label = if let Some(commit) = &self.diff_view.commit {
            commit.chars().take(7).collect::<String>()
        } else if let Some(count) = scope.strip_prefix("turn:") {
            if turns
                .first()
                .is_some_and(|latest| latest.to_string() == count)
            {
                "Latest turn".into()
            } else {
                format!("Turn {count}")
            }
        } else if scope == "branch-range" {
            "Changes".into()
        } else {
            "Uncommitted".into()
        };
        v_flex()
            .child(
                h_flex()
                    .gap_1()
                    .px_2()
                    .h(px(40.))
                    .child(
                        Button::new("diff-scope")
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(label.clone())
                                    .child(Icon::new(IconName::ChevronDown).size(px(12.))),
                            )
                            .accessibility_label(format!("Diff scope: {label}"))
                            .ghost()
                            .compact()
                            .dropdown_menu(move |mut menu, window, cx| {
                                for (label, selected) in [
                                    ("Changes", "branch-range"),
                                    ("Uncommitted", "working-tree"),
                                    ("Latest turn", "latest"),
                                ] {
                                    menu = menu.menu_with_check(
                                        label,
                                        scope == selected,
                                        Box::new(SelectDiff {
                                            scope: selected.into(),
                                            commit: None,
                                        }),
                                    );
                                }
                                let turn_focus = focus.clone();
                                let turns = turns.clone();
                                menu = menu.submenu("Turn", window, cx, move |mut menu, _, _| {
                                    for count in &turns {
                                        menu = menu.menu(
                                            format!("Turn {count}"),
                                            Box::new(SelectDiff {
                                                scope: format!("turn:{count}"),
                                                commit: None,
                                            }),
                                        );
                                    }
                                    menu.radio_group().action_context(turn_focus.clone())
                                });
                                let commit_focus = focus.clone();
                                let commits = commits.clone();
                                menu = menu.submenu("Commit", window, cx, move |mut menu, _, _| {
                                    for commit in &commits {
                                        menu = menu.menu(
                                            commit["subject"]
                                                .as_str()
                                                .unwrap_or_default()
                                                .to_string(),
                                            Box::new(SelectDiff {
                                                scope: "commit".into(),
                                                commit: commit["sha"].as_str().map(str::to_string),
                                            }),
                                        );
                                    }
                                    if commits.is_empty() {
                                        menu = menu.item(
                                            PopupMenuItem::new("No commits on this branch.")
                                                .disabled(true),
                                        );
                                    }
                                    menu.radio_group().action_context(commit_focus.clone())
                                });
                                menu.radio_group().action_context(focus.clone())
                            }),
                    )
                    .when(
                        self.diff_view.scope == "branch-range" && self.diff_view.commit.is_none(),
                        |row| row.child(self.render_diff_base(cx)),
                    )
                    .child(div().flex_1())
                    .when_some(stats, |row, (additions, deletions)| {
                        row.child(
                            h_flex()
                                .gap_1()
                                .text_xs()
                                .child(
                                    div()
                                        .text_color(cx.theme().success)
                                        .child(text!(format!("+{additions}"))),
                                )
                                .child(
                                    div()
                                        .text_color(cx.theme().danger)
                                        .child(text!(format!("-{deletions}"))),
                                ),
                        )
                    })
                    .child(
                        Button::new("refresh-diff")
                            .icon(IconName::RefreshCw)
                            .accessibility_label(if self.diff_view.loading {
                                "Refreshing diff"
                            } else {
                                "Refresh diff"
                            })
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
                            .when(
                                self.file_panel.as_ref().is_none_or(|p| p.lines.is_empty()),
                                |button| button.invisible(),
                            )
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
