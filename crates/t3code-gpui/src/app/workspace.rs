use super::*;

pub(super) struct DetailsPane {
    pub(super) root: WeakEntity<Root>,
    pub(super) services: Option<((String, String), Entity<ThreadServices>)>,
}
impl Render for DetailsPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let target = self.root.upgrade().and_then(|root| root.read(cx).current());
        if self.services.as_ref().map(|(key, _)| key) != target.as_ref() {
            self.services = target.and_then(|key| {
                let root = self.root.upgrade()?;
                let connection = root.read(cx).connection(&key.0)?;
                let runtime = root.read(cx).runtime.clone();
                let weak = self.root.clone();
                let entity =
                    cx.new(|cx| ThreadServices::new(weak, key.clone(), connection, runtime, cx));
                Some((key, entity))
            });
        }
        let services = self.services.as_ref().map(|(_, entity)| entity.clone());
        self.root
            .update(cx, |root, cx| {
                root.with_main_conversation(|root| root.render_thread_details(services, cx))
            })
            .expect("details root is alive")
    }
}

impl Root {
    pub(super) fn follow_workspace(&mut self) {
        let target = self
            .composer_cwd()
            .map(|(environment, cwd)| workspace::Target {
                environment,
                cwd,
                include_remote: self.details_open
                    && workspace::layout(self.canvas_width, true).details_inline
                    || self.details_popover,
            });
        if self
            .conversation
            .workspace_watch
            .as_ref()
            .map(|watch| &watch.target)
            == target.as_ref()
        {
            return;
        }
        let same_directory = self
            .conversation
            .workspace_watch
            .as_ref()
            .zip(target.as_ref())
            .is_some_and(|(watch, target)| {
                watch.target.environment == target.environment && watch.target.cwd == target.cwd
            });
        self.conversation.workspace_watch = None;
        if !same_directory {
            self.conversation.branch_request += 1;
            self.conversation.workspace_status = workspace::Status::default();
            self.conversation.branch_anchor = None;
        }
        if let Some(target) = target
            && let Some(connection) = self.connection(&target.environment)
        {
            self.conversation.workspace_watch = Some(workspace::Watch::new(
                connection,
                target,
                &self.runtime,
                self.workspace_sender.clone(),
            ));
        }
    }

    pub(super) fn render_workspace_canvas(
        &self,
        pane: AnyElement,
        cx: &Context<Self>,
    ) -> AnyElement {
        let layout = workspace::layout(self.canvas_width, self.details_open);
        let card_height = if self.details_has_lineage() {
            self.lineage_height()
        } else {
            0.
        } + self
            .details_pane
            .read(cx)
            .services
            .as_ref()
            .map_or(0., |(_, services)| services.read(cx).height())
            + self.thread_extra_height()
            + 84.
            + if self.workspace_editor().is_some() {
                32.
            } else {
                0.
            }
            + if self.conversation.workspace_status.is_repo() {
                114.
            } else {
                0.
            }
            + if self.conversation.workspace_status.error.is_some() {
                32.
            } else {
                0.
            };
        let weak = cx.entity().downgrade();
        div()
            .relative()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(div().h_full().pr(px(layout.inset_end)).child(pane))
            .when(
                self.conversation.workspace_watch.is_some()
                    && (layout.details_inline && self.details_open
                        || !layout.details_inline && self.details_popover),
                |canvas| {
                    canvas.child(
                        div()
                            .absolute()
                            .top(px(12.))
                            .right(px(12.))
                            .w(px(280.))
                            .h(px(card_height))
                            .child(
                                if self.conversation.branch_anchor == Some("details-branch") {
                                    self.details_pane.clone().into_any_element()
                                } else {
                                    self.details_pane
                                        .clone()
                                        .cached(
                                            StyleRefinement::default()
                                                .w(px(280.))
                                                .h(px(card_height)),
                                        )
                                        .into_any_element()
                                },
                            ),
                    )
                },
            )
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let _ = weak.update(cx, |root, cx| {
                            let width = f32::from(bounds.size.width);
                            if (root.canvas_width - width).abs() >= 0.5 {
                                root.canvas_width = width;
                                cx.notify();
                            }
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full()
                .top_0()
                .left_0(),
            )
            .into_any_element()
    }

    fn details_has_lineage(&self) -> bool {
        !self.lineage_data.rows.is_empty()
            || self.lineage_data.running > 0
            || self
                .current()
                .and_then(|(environment, thread)| self.thread_value(&environment, &thread))
                .is_some_and(|thread| !thread["delegatedFrom"].is_null())
    }

    fn render_thread_details(
        &self,
        services: Option<Entity<ThreadServices>>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let Some((environment, cwd)) = self.composer_cwd() else {
            return div().into_any_element();
        };
        let config = self
            .model_catalogs
            .get(&environment)
            .map(|(config, _)| config)
            .unwrap_or(&Value::Null);
        let label = environment.clone();
        let workspace_label = if self
            .conversation
            .draft
            .as_ref()
            .is_some_and(|draft| draft.workspace_strategy["type"] == "worktree")
        {
            "New worktree"
        } else if self.conversation.draft.is_none() {
            "Local checkout"
        } else {
            "Current checkout"
        };
        let workspace_label = if self.conversation.draft.is_none()
            && self
                .current()
                .and_then(|(environment, thread)| self.thread_value(&environment, &thread))
                .and_then(|thread| thread["worktreePath"].as_str().map(str::to_owned))
                .is_some()
        {
            std::path::Path::new(&cwd)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&cwd)
        } else {
            workspace_label
        };
        let host_accessibility_label = format!("Run context: {label}, {workspace_label}");
        let groups = self.project_groups();
        let members = self
            .conversation
            .draft
            .as_ref()
            .and_then(|draft| {
                groups
                    .iter()
                    .find(|group| Some(&group.key) == draft.group.as_ref())
            })
            .map(|group| group.members.clone())
            .unwrap_or_default();
        let focus = self.focus.clone();
        let host = Button::new("workspace-host")
            .child(
                h_flex()
                    .w_full()
                    .gap_2p5()
                    .child(Icon::new(gpui_kit::assets::IconName::Monitor).size(px(16.)))
                    .child(div().flex_1().text_size(px(13.)).child(text!(label)))
                    .child(Icon::new(IconName::Folder).size(px(16.)))
                    .child(Icon::new(IconName::ChevronDown).size(px(16.))),
            )
            .ghost()
            .w_full()
            .h(px(32.))
            .px_2p5()
            .rounded(px(8.))
            .accessibility_label(host_accessibility_label)
            .tooltip(cwd.clone())
            .dropdown_menu(move |mut menu, _, _| {
                for member in &members {
                    menu = menu.menu_with_check(
                        member.environment.clone(),
                        member.environment == environment,
                        Box::new(WorkspaceAction {
                            kind: "host".into(),
                            value: json!(member.environment),
                        }),
                    );
                }
                menu.action_context(focus.clone())
            });
        let editors = config
            .get("availableEditors")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let preferred = self.workspace_editor();
        let focus = self.focus.clone();
        let weak = cx.entity().downgrade();
        let light = self.settings.light_theme;
        let editor_picker_focus = self.editor_picker_focus.clone();
        let editor_menu = Button::new("workspace-editor-picker")
            .track_focus(&editor_picker_focus)
            .icon(IconName::ChevronDown)
            .ghost()
            .w(px(32.))
            .h(px(32.))
            .px_0()
            .rounded(px(8.))
            .accessibility_label("Choose editor")
            .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, window, cx| {
                let shortcut = weak.upgrade().and_then(|entity| {
                    let root = entity.read(cx);
                    keymap::command_label(&root.key_rules, "editor.openFavorite", &|name| {
                        root.key_context(name, window, cx)
                    })
                });
                for editor in editors.iter().filter_map(Value::as_str) {
                    let editor = editor.to_string();
                    let label = workspace::editor_label(&editor).to_string();
                    let hint = shortcut
                        .clone()
                        .filter(|_| preferred.as_deref() == Some(&editor));
                    let accessible_label = label.clone();
                    let selected_editor = editor.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, cx| {
                            h_flex()
                                .w_full()
                                .gap_2()
                                .child(workspace::editor_icon(
                                    &editor,
                                    light,
                                    cx.theme().foreground,
                                    cx.theme().muted_foreground,
                                ))
                                .child(label.clone())
                                .child(div().flex_1())
                                .children(hint.clone().map(|hint| {
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(hint)
                                }))
                        })
                        .accessibility_label(accessible_label)
                        .action(Box::new(WorkspaceAction {
                            kind: "choose-editor".into(),
                            value: json!(selected_editor),
                        })),
                    );
                }
                menu.min_w(px(262.))
                    .max_w(px(262.))
                    .action_context(focus.clone())
            })
            .on_open_change(move |open, window, cx| {
                if !open {
                    editor_picker_focus.focus(window, cx);
                }
            });
        let editor_button = self.workspace_editor().map(|editor| {
            let label = format!("Open in {}", workspace::editor_label(&editor));
            h_flex()
                .w_full()
                .id("workspace-editor-group")
                .role(accesskit::Role::Group)
                .aria_label("Open in editor")
                .child(
                    Button::new("workspace-editor")
                        .child(
                            h_flex()
                                .w_full()
                                .gap_2p5()
                                .child(workspace::editor_icon(
                                    &editor,
                                    self.settings.light_theme,
                                    theme.foreground,
                                    theme.muted_foreground,
                                ))
                                .child(div().flex_1().text_size(px(13.)).child(label.clone())),
                        )
                        .ghost()
                        .flex_1()
                        .h(px(32.))
                        .px_2p5()
                        .rounded(px(8.))
                        .accessibility_label(label)
                        .on_click(self.pane_listener(cx, move |root, _, window, cx| {
                            root.workspace_action(
                                &WorkspaceAction {
                                    kind: "editor".into(),
                                    value: json!(editor),
                                },
                                window,
                                cx,
                            )
                        })),
                )
                .child(editor_menu)
        });
        v_flex()
            .size_full()
            .id("thread-details")
            .role(accesskit::Role::Region)
            .aria_label("Thread details")
            .rounded(px(16.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .text_color(theme.foreground.opacity(0.8))
            .text_size(px(13.))
            .child(
                v_flex()
                    .id("details-workspace-section")
                    .role(accesskit::Role::Region)
                    .aria_label("Workspace")
                    .px_2()
                    .pt_2()
                    .pb_2p5()
                    .child(host)
                    .children(editor_button)
                    .child(self.render_project_scripts(cx)),
            )
            .when(self.conversation.workspace_status.is_repo(), |card| {
                let changes = &self.conversation.workspace_status.local["workingTree"];
                card.child(
                    v_flex()
                        .id("details-git-section")
                        .role(accesskit::Role::Region)
                        .aria_label("Version Control")
                        .px_2()
                        .pt_2()
                        .pb_2p5()
                        .border_t_1()
                        .border_color(theme.border.opacity(0.65))
                        .child(self.render_branch_control("details-branch", cx))
                        .child(self.render_git_controls(cx))
                        .child(
                            Button::new("workspace-changes")
                                .child(
                                    h_flex()
                                        .gap_2p5()
                                        .w_full()
                                        .child(
                                            Icon::new(gpui_kit::assets::IconName::FileDiff)
                                                .size(px(16.)),
                                        )
                                        .child(text!("Changes"))
                                        .child(
                                            div()
                                                .flex_1()
                                                .text_xs()
                                                .text_color(theme.muted_foreground)
                                                .child(text!("Uncommitted")),
                                        )
                                        .child(
                                            h_flex()
                                                .text_xs()
                                                .text_color(theme.success)
                                                .child(text!("+"))
                                                .child(text!(
                                                    id = "changes-insertions",
                                                    changes["insertions"]
                                                        .as_u64()
                                                        .unwrap_or(0)
                                                        .to_string()
                                                )),
                                        )
                                        .child(
                                            h_flex()
                                                .text_xs()
                                                .text_color(theme.muted_foreground)
                                                .child(text!("-"))
                                                .child(text!(
                                                    id = "changes-deletions",
                                                    changes["deletions"]
                                                        .as_u64()
                                                        .unwrap_or(0)
                                                        .to_string()
                                                )),
                                        ),
                                )
                                .ghost()
                                .w_full()
                                .h(px(32.))
                                .px_2p5()
                                .rounded(px(8.))
                                .accessibility_label(format!(
                                    "Changes Uncommitted +{} -{}",
                                    changes["insertions"].as_u64().unwrap_or(0),
                                    changes["deletions"].as_u64().unwrap_or(0)
                                ))
                                .on_click(
                                    self.pane_listener(cx, |root, _, _, cx| root.toggle_diff(cx)),
                                ),
                        ),
                )
            })
            .when_some(
                self.conversation.workspace_status.error.clone(),
                |card, error| {
                    card.child(
                        div()
                            .h(px(32.))
                            .px_3()
                            .pb_2()
                            .text_xs()
                            .truncate()
                            .text_color(theme.danger)
                            .child(error),
                    )
                },
            )
            .child(self.render_thread_pull_requests(cx))
            .when(self.details_has_lineage(), |card| {
                card.child(self.render_lineage(cx))
            })
            .children(services)
            .child(self.render_thread_usage(cx))
            .into_any_element()
    }

    pub(super) fn render_workspace_mode(&self, cx: &Context<Self>) -> AnyElement {
        self.render_workspace_select(cx)
    }

    pub(super) fn workspace_editor(&self) -> Option<String> {
        let environment = &self
            .conversation
            .workspace_watch
            .as_ref()?
            .target
            .environment;
        let config = &self.model_catalogs.get(environment)?.0;
        workspace::preferred_editor(
            config.get("availableEditors")?.as_array()?,
            self.settings.preferred_editor.as_deref(),
        )
    }

    pub(super) fn render_branch_control(&self, id: &'static str, cx: &Context<Self>) -> AnyElement {
        let branch = self
            .conversation
            .draft
            .as_ref()
            .and_then(|draft| {
                draft
                    .workspace_strategy
                    .get("baseRef")
                    .or_else(|| draft.workspace_strategy.get("branch"))
            })
            .and_then(Value::as_str)
            .or_else(|| self.conversation.workspace_status.branch())
            .unwrap_or("Select branch")
            .to_string();
        let branch = if let Some(draft) = self
            .conversation
            .draft
            .as_ref()
            .filter(|draft| draft.workspace_strategy["type"] == "worktree")
        {
            if draft.workspace_strategy["startFromOrigin"] == true {
                format!("From {branch} (remote)")
            } else {
                format!("From {branch}")
            }
        } else {
            branch
        };
        let (head, tail) = split_branch_label(&branch);
        let accessible_label = if tail.is_empty() {
            branch.clone()
        } else {
            format!("{head} {tail}")
        };
        let button =
            Button::new(id)
                .child(
                    h_flex()
                        .when(id == "details-branch", |row| row.w_full())
                        .gap(px(if id == "details-branch" { 10. } else { 6. }))
                        .child(
                            Icon::new(gpui_kit::assets::IconName::GitBranch)
                                .size(px(if id == "details-branch" { 16. } else { 12. })),
                        )
                        .child(
                            h_flex()
                                .flex_1()
                                .min_w_0()
                                .child(div().truncate().child(text!(
                                    id = SharedString::from(format!("{id}-label")),
                                    head
                                )))
                                .when(!tail.is_empty(), |row| {
                                    row.child(div().flex_shrink_0().child(text!(
                                        id = SharedString::from(format!("{id}-tail")),
                                        tail
                                    )))
                                }),
                        )
                        .when(id == "details-branch", |row| {
                            row.child(Icon::new(IconName::ChevronDown).size(px(12.)))
                        }),
                )
                .ghost()
                .when(id == "details-branch", |button| button.w_full())
                .h(px(if id == "details-branch" { 32. } else { 24. }))
                .px(px(if id == "details-branch" { 10. } else { 7. }))
                .when(id != "details-branch", |button| {
                    button
                        .text_size(px(12.))
                        .text_color(cx.theme().muted_foreground)
                })
                .rounded(px(8.))
                .accessibility_label(accessible_label)
                .role(accesskit::Role::ComboBox)
                .on_click(self.pane_listener(cx, move |root, _, window, cx| {
                    root.open_branch_picker(id, window, cx)
                }));
        let mut popup = gpui_kit::base::Popup::new(id, button)
            .anchor(if id == "details-branch" {
                Anchor::TopRight
            } else {
                Anchor::BottomRight
            })
            .offset(px(4.));
        if self.conversation.branch_anchor == Some(id) {
            let weak = self.pane_root(cx);
            let search = weak.clone();
            let cancel = weak.clone();
            let mut items = self
                .conversation
                .branch_refs
                .iter()
                .map(|reference| {
                    CommandItem::new()
                        .label(reference["name"].as_str().unwrap_or_default().to_string())
                        .checked(reference["current"].as_bool() == Some(true))
                        .action(Box::new(WorkspaceAction {
                            kind: "branch".into(),
                            value: reference.clone(),
                        }))
                })
                .collect::<Vec<_>>();
            if self.conversation.branch_cursor.is_some() {
                items.push(
                    CommandItem::new()
                        .label("Load more branches")
                        .action(Box::new(WorkspaceAction {
                            kind: "more-branches".into(),
                            value: Value::Null,
                        })),
                );
            }
            popup = popup.content(
                div()
                    .w(px(320.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().popover)
                    .child(
                        Command::new(&self.conversation.branch_picker)
                            .placeholder("Search refs...")
                            .filterable(false)
                            .max_h(px(224.))
                            .items(items)
                            .on_query(move |query, _, cx| {
                                let _ = search.update(cx, |root, cx| {
                                    root.search_branches(query.to_string(), false, cx)
                                });
                            })
                            .on_cancel(move |window, cx| {
                                let _ = cancel.update(cx, |root, cx| {
                                    root.conversation.branch_anchor = None;
                                    root.conversation
                                        .composer
                                        .update(cx, |state, cx| state.focus(window, cx));
                                    cx.notify();
                                });
                            }),
                    )
                    .when(
                        self.conversation.draft.as_ref().is_some_and(|draft| {
                            draft.workspace_strategy["type"].as_str() == Some("worktree")
                        }),
                        |popup| {
                            popup.child(
                                h_flex()
                                    .justify_between()
                                    .px_3()
                                    .py_2()
                                    .border_t_1()
                                    .border_color(cx.theme().border.opacity(0.6))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(text!("Start from remote")),
                                    )
                                    .child(
                                        gpui_kit::component::switch::Switch::new(
                                            "branch-from-origin",
                                        )
                                        .small()
                                        .accessibility_label("Start worktree from remote")
                                        .checked(self.conversation.draft.as_ref().is_some_and(
                                            |draft| {
                                                draft.workspace_strategy["startFromOrigin"]
                                                    .as_bool()
                                                    == Some(true)
                                            },
                                        ))
                                        .on_change(
                                            self.pane_listener(cx, |root, checked, _, cx| {
                                                if let Some(draft) =
                                                    root.conversation.draft.as_mut()
                                                {
                                                    draft.workspace_strategy["startFromOrigin"] =
                                                        json!(*checked);
                                                }
                                                cx.notify();
                                            }),
                                        ),
                                    ),
                            )
                        },
                    ),
            )
        }
        popup.into_any_element()
    }

    pub(super) fn open_branch_picker(
        &mut self,
        id: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.conversation.workspace_status.is_repo() {
            return;
        }
        self.conversation.branch_anchor =
            (self.conversation.branch_anchor != Some(id)).then_some(id);
        if self.conversation.branch_anchor.is_some() {
            self.conversation.branch_request += 1;
            self.conversation.branch_loading = false;
            self.conversation.branch_picker.update(cx, |state, cx| {
                state.set_query("", window, cx);
                state.focus(window, cx);
            });
            self.search_branches(String::new(), false, cx);
        } else {
            self.conversation
                .composer
                .update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    pub(super) fn search_branches(&mut self, query: String, more: bool, cx: &mut Context<Self>) {
        let Some((environment, cwd)) = self.composer_cwd() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        self.conversation.branch_query = query.clone();
        if self.conversation.branch_loading {
            cx.notify();
            return;
        }
        self.conversation.branch_request += 1;
        let request = self.conversation.branch_request;
        self.conversation.branch_loading = true;
        let cursor = if more {
            self.conversation.branch_cursor
        } else {
            None
        };
        if !more {
            self.conversation.branch_refs.clear();
            self.conversation.branch_cursor = None;
        }
        let runtime = self.runtime.clone();
        let requested_query = query.clone();
        self.spawn_pane(cx, async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                let mut input = json!({"cwd":cwd,"limit":200});
                if query.is_empty() && !more {
                    connection
                        .request("vcs.refreshStatus", json!({"cwd":cwd,"localOnly":true}))
                        .await?;
                    input["refresh"] = json!(true);
                }
                if !query.trim().is_empty() {
                    input["query"] = json!(query.trim());
                }
                if let Some(cursor) = cursor {
                    input["cursor"] = json!(cursor);
                }
                connection.request("vcs.listRefs", input).await
            })
            .await;
            let _ = this.update(cx, |root, cx| {
                if root.conversation.branch_request != request
                    || root.conversation.branch_anchor.is_none()
                {
                    return;
                }
                root.conversation.branch_loading = false;
                if root.conversation.branch_query != requested_query {
                    root.search_branches(root.conversation.branch_query.clone(), false, cx);
                    return;
                }
                match result {
                    Ok(result) => {
                        root.conversation
                            .branch_refs
                            .extend(result["refs"].as_array().cloned().unwrap_or_default());
                        root.conversation.branch_cursor = result["nextCursor"].as_u64();
                    }
                    Err(error) => {
                        root.conversation.message =
                            Some(format!("Could not load branches: {error:#}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn workspace_action(
        &mut self,
        action: &WorkspaceAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action.kind.as_str() {
            "diff-base" => {
                self.diff_view.base_ref = action.value.as_str().map(str::to_string);
                self.diff_view.collapsed = true;
                self.file_panel = None;
                self.branch_anchor = None;
                self.refresh_diff(cx);
                self.panel_focus.focus(window, cx);
                return;
            }
            "detach-agent" => {
                self.lineage_action("detach", window, cx);
                return;
            }
            "choose-editor" => {
                self.change_setting("gpuiPreferredEditor", action.value.clone(), cx);
                return;
            }
            "host" => {
                if let Some(draft) = self.conversation.draft.as_mut() {
                    draft.environment = action.value.as_str().map(str::to_string);
                    draft.workspace_strategy = json!({"type":"root"});
                    draft.model = None;
                    draft.runtime_mode = None;
                }
            }
            "editor" => {
                if let Some((environment, cwd)) = self.composer_cwd()
                    && let Some(connection) = self.connection(&environment)
                {
                    let editor = action.value.clone();
                    let target = action
                        .value
                        .get("path")
                        .and_then(Value::as_str)
                        .unwrap_or(&cwd);
                    let editor = editor.get("editor").cloned().unwrap_or(editor);
                    let input = json!({"cwd":target,"editor":editor});
                    self.change_setting("gpuiPreferredEditor", editor, cx);
                    let runtime = self.runtime.clone();
                    self.spawn_pane(cx, async move |this, cx| {
                        let result = on_runtime(&runtime, async move {
                            connection.request("shell.openInEditor", input).await
                        })
                        .await;
                        if let Err(error) = result {
                            let _ = this.update(cx, |root, cx| {
                                root.conversation.message =
                                    Some(format!("Could not open editor: {error:#}"));
                                cx.notify();
                            });
                        }
                    })
                    .detach();
                }
                return;
            }
            "mode" => {
                if let Some(draft) = self.conversation.draft.as_mut() {
                    draft.workspace_strategy = if action.value.as_str() == Some("worktree") {
                        json!({"type":"worktree","baseRef":self.conversation.workspace_status.branch().unwrap_or("HEAD"),"startFromOrigin":true})
                    } else {
                        json!({"type":"root"})
                    };
                }
            }
            "more-branches" => {
                self.search_branches(self.conversation.branch_query.clone(), true, cx);
                return;
            }
            "branch" => self.select_workspace_branch(action.value.clone(), cx),
            _ => return,
        }
        if action.kind != "more-branches" {
            self.conversation.branch_anchor = None;
            self.conversation
                .composer
                .update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    fn select_workspace_branch(&mut self, reference: Value, cx: &mut Context<Self>) {
        let Some(name) = reference["name"].as_str().map(str::to_string) else {
            return;
        };
        if let Some(draft) = self.conversation.draft.as_mut()
            && draft.workspace_strategy["type"].as_str() == Some("worktree")
        {
            draft.workspace_strategy["baseRef"] = json!(name);
            return;
        }
        let Some((environment, _)) = self.composer_cwd() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let thread_id = self.current().map(|(_, id)| id);
        let draft_id = self
            .conversation
            .draft
            .as_ref()
            .map(|draft| draft.id.clone());
        let project = if let Some((_, project_id, ..)) = self
            .draft_target()
            .filter(|_| self.conversation.draft.is_some())
        {
            self.project_value(&environment, &project_id)
        } else {
            self.current()
                .and_then(|(_, id)| self.thread_value(&environment, &id))
                .and_then(|thread| {
                    thread["projectId"]
                        .as_str()
                        .and_then(|id| self.project_value(&environment, id))
                })
        };
        let Some(project_root) = project
            .as_ref()
            .and_then(|project| project["workspaceRoot"].as_str())
        else {
            return;
        };
        let current_worktree = self
            .conversation
            .draft
            .as_ref()
            .and_then(|draft| draft.workspace_strategy["worktreePath"].as_str());
        let thread = thread_id
            .as_ref()
            .and_then(|id| self.thread_value(&environment, id));
        let current_worktree = current_worktree.or_else(|| {
            thread
                .as_ref()
                .and_then(|thread| thread["worktreePath"].as_str())
        });
        let target = workspace::branch_target(
            project_root,
            self.conversation
                .workspace_status
                .local
                .pointer("/workingTree/rootPath")
                .and_then(Value::as_str)
                .filter(|_| current_worktree.is_none()),
            current_worktree,
            &reference,
        );
        let sessions = if current_worktree != target.worktree.as_deref() {
            self.conversation
                .open
                .as_ref()
                .map(|open| {
                    open.watch
                        .state()
                        .lock()
                        .expect("thread lock")
                        .projection
                        .get("providerSessions")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let runtime = self.runtime.clone();
        self.spawn_pane(cx,async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                let branch = if target.reuse {
                    name
                } else {
                    connection
                        .request("vcs.switchRef", json!({"cwd":target.cwd,"refName":name}))
                        .await?["refName"]
                        .as_str()
                        .unwrap_or(&name)
                        .to_string()
                };
                let existing = target.worktree;
                if let Some(thread_id) = thread_id {
                    for session in sessions {
                        connection.dispatch(json!({
                            "type":"provider-session.detach",
                            "threadId":thread_id,
                            "providerSessionId":session["id"],
                            "reason":"client-requested"
                        })).await?;
                    }
                    connection.dispatch(json!({
                        "type":"thread.metadata.update",
                        "threadId":thread_id,
                        "branch":branch,
                        "worktreePath":existing
                    })).await?;
                }
                Ok::<_, anyhow::Error>((branch, existing))
            }).await;
            let _ = this.update(cx, |root, cx| {
                match result {
                    Ok((branch, existing)) => if let Some(draft) = root.conversation.draft.as_mut().filter(|draft| Some(&draft.id) == draft_id.as_ref()) {
                        draft.workspace_strategy = match existing {
                            Some(path) => json!({"type":"existing_worktree","worktreePath":path,"branch":branch}),
                            None => json!({"type":"root","branch":branch})
                        };
                    },
                    Err(error) => root.conversation.message = Some(format!("Could not switch branch: {error:#}")),
                }
                cx.notify();
            });
        }).detach();
    }
}

pub(super) struct ThreadServices {
    root: WeakEntity<Root>,
    environment: String,
    thread: String,
    servers: Vec<Value>,
    tasks: Vec<Value>,
    visible: usize,
    busy: Option<String>,
    error: Option<String>,
    workers: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for ThreadServices {
    fn drop(&mut self) {
        for worker in &self.workers {
            worker.abort();
        }
    }
}

impl ThreadServices {
    fn height(&self) -> f32 {
        let servers = if self.servers.is_empty() {
            0.
        } else {
            40. + self.servers.len().min(self.visible) as f32 * 52.
                + if self.servers.len() > 4 { 32. } else { 0. }
        };
        let tasks = if self.tasks.is_empty() {
            0.
        } else {
            48. + self.tasks.len() as f32 * 52.
        };
        servers + tasks + if self.error.is_some() { 32. } else { 0. }
    }

    fn new(
        root: WeakEntity<Root>,
        target: (String, String),
        connection: Arc<Connection>,
        runtime: Handle,
        cx: &mut Context<Self>,
    ) -> Self {
        use t3_client::rpc::StreamLimits;
        let mut view = Self {
            root,
            environment: target.0,
            thread: target.1,
            servers: vec![],
            tasks: vec![],
            visible: 4,
            busy: None,
            error: None,
            workers: vec![],
        };
        for (method, field) in [
            ("subscribeDiscoveredLocalServers", "servers"),
            ("scheduledTasks.subscribe", "tasks"),
        ] {
            let connection = connection.clone();
            let (sender, receiver) = async_channel::bounded(8);
            view.workers.push(runtime.spawn(async move {
                let result: anyhow::Result<()> = async {
                    let mut stream = connection
                        .subscribe(
                            method,
                            json!({}),
                            StreamLimits::new(8, 4 << 20).expect("positive stream limits"),
                        )
                        .await?;
                    while let Some(chunk) = stream.next().await {
                        let chunk = chunk?;
                        for value in chunk.values() {
                            if sender.send(Ok(value.clone())).await.is_err() {
                                return Ok(());
                            }
                        }
                        chunk.accept();
                    }
                    Ok(())
                }
                .await;
                if let Err(error) = result {
                    let _ = sender.send(Err(format!("{error:#}"))).await;
                }
            }));
            cx.spawn(async move |this, cx| {
                while let Ok(mut value) = receiver.recv().await {
                    cx.background_executor().timer(REDRAW_INTERVAL).await;
                    while let Ok(next) = receiver.try_recv() {
                        value = next;
                    }
                    if this
                        .update(cx, |view, cx| {
                            match value {
                                Ok(value) => {
                                    let values =
                                        value[field].as_array().cloned().unwrap_or_default();
                                    if field == "servers" {
                                        view.servers = values
                                            .into_iter()
                                            .filter(|v| {
                                                v["terminal"]["threadId"] == view.thread
                                                    || v["agent"]["threadId"] == view.thread
                                            })
                                            .collect();
                                    } else {
                                        view.tasks = values
                                            .into_iter()
                                            .filter(|v| v["threadId"] == view.thread)
                                            .collect();
                                        view.error = None;
                                    }
                                }
                                Err(error) => {
                                    view.error = Some(format!(
                                        "Could not load {}: {error}",
                                        if field == "tasks" {
                                            "automations"
                                        } else {
                                            "servers"
                                        }
                                    ));
                                }
                            }
                            let _ = view.root.update(cx, |root, cx| {
                                root.details_stamp = None;
                                cx.notify();
                            });
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }
        view
    }

    fn mutate(&mut self, method: &'static str, input: Value, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Ok(Some((connection, runtime))) = self.root.update(cx, |root, _| {
            root.connection(&self.environment)
                .map(|connection| (connection, root.runtime.clone()))
        }) else {
            return;
        };
        self.busy = Some(method.into());
        self.error = None;
        cx.spawn(async move |this, cx| {
            let result = on_runtime(
                &runtime,
                async move { connection.request(method, input).await },
            )
            .await;
            let _ = this.update(cx, |view, cx| {
                view.busy = None;
                if let Err(error) = result {
                    view.error = Some(format!("{error:#}"));
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn manage_tasks(&self, cx: &mut Context<Self>) {
        let _ = self.root.update(cx, |root, cx| {
            root.settings_environment = Some(self.environment.clone());
            root.settings_page = "scheduled-tasks";
            root.settings_open = true;
            cx.notify();
        });
    }
}

impl Render for ThreadServices {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::switch::Switch;
        let mut body = v_flex();
        if !self.servers.is_empty() {
            let supports_stop = self.root.upgrade().is_some_and(|root| {
                root.read(cx)
                    .config_of(&self.environment)
                    .is_some_and(|config| {
                        config["environment"]["capabilities"]["localServerStop"] == true
                    })
            });
            let mut servers = v_flex()
                .px_2()
                .py_2()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(div().px_2p5().text_xs().child(text!("Servers")));
            for (index, server) in self.servers.iter().take(self.visible).enumerate() {
                let port = server["port"].as_u64().unwrap_or_default();
                let mut description = if server["terminal"].is_null() {
                    "Agent".to_string()
                } else {
                    "Terminal".to_string()
                };
                if let Some(process) = server["processName"].as_str() {
                    description.push_str(&format!(" · {process}"));
                }
                let url = server["url"].as_str().unwrap_or_default().to_string();
                let input = json!({"pid":server["pid"],"port":port});
                servers = servers.child(
                    h_flex()
                        .child(
                            Button::new(("server", index))
                                .ghost()
                                .flex_1()
                                .h_auto()
                                .py_2()
                                .child(
                                    v_flex()
                                        .items_start()
                                        .child(format!("localhost:{port}"))
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(description.clone()),
                                        ),
                                )
                                .accessibility_label(format!("localhost:{port} {description}"))
                                .on_click(cx.listener(move |view, event: &ClickEvent, _, cx| {
                                    if event.modifiers().control || event.modifiers().platform {
                                        cx.open_url(&url);
                                    } else {
                                        let _ = view.root.update(cx, |root, cx| {
                                            root.activate_panel("browser".into(), cx);
                                        });
                                    }
                                })),
                        )
                        .when(supports_stop && !server["pid"].is_null(), |row| {
                            row.child(
                                Button::new(("stop-server", index))
                                    .icon(gpui_kit::assets::IconName::Square)
                                    .accessibility_label(format!("Stop localhost:{port}"))
                                    .ghost()
                                    .small()
                                    .disabled(self.busy.is_some())
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.mutate("preview.stopLocalServer", input.clone(), cx)
                                    })),
                            )
                        }),
                );
            }
            if self.servers.len() > 4 {
                let hidden = self.servers.len().saturating_sub(self.visible);
                servers = servers.child(
                    Button::new("more-servers")
                        .ghost()
                        .label(if hidden > 0 {
                            format!("Show {} more", hidden.min(4))
                        } else {
                            "Show less".into()
                        })
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.visible = if hidden > 0 { view.visible + 4 } else { 4 };
                            let _ = view.root.update(cx, |_, cx| cx.notify());
                            cx.notify();
                        })),
                );
            }
            body = body.child(servers);
        }
        if !self.tasks.is_empty() {
            let mut tasks = v_flex()
                .px_2()
                .py_2()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(
                    h_flex()
                        .justify_between()
                        .child(div().px_2p5().text_xs().child(text!("Automations")))
                        .child(
                            Button::new("manage-tasks")
                                .icon(gpui_kit::assets::IconName::Settings2)
                                .accessibility_label("Manage scheduled tasks")
                                .ghost()
                                .small()
                                .on_click(cx.listener(|view, _, _, cx| view.manage_tasks(cx))),
                        ),
                );
            for (index, task) in self.tasks.iter().enumerate() {
                let title = task["title"].as_str().unwrap_or_default().to_string();
                let id = task["id"].clone();
                let toggle_id = id.clone();
                let edit_task = task.clone();
                let enabled = task["enabled"] == true;
                let schedule = task_schedule_label(&task["schedule"]);
                let suffix = if let Some(blocked) = task["blockedByPolicy"].as_str() {
                    format!("· {blocked}")
                } else if !enabled {
                    "· paused".into()
                } else if !task["nextRunAt"].is_null() {
                    "· next".into()
                } else {
                    String::new()
                };
                tasks = tasks.child(
                    h_flex()
                        .id(SharedString::from(format!("automation-{id}")))
                        .py_1p5()
                        .gap_2()
                        .child(
                            v_flex().flex_1().child(text!(title.clone())).child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .child(text!(schedule))
                                            .child(text!(suffix))
                                            .when(
                                                enabled
                                                    && task["blockedByPolicy"].is_null()
                                                    && !task["nextRunAt"].is_null(),
                                                |row| row.child(text!(task_next_label(task))),
                                            ),
                                    ),
                            ),
                        )
                        .child(
                            Button::new(("edit-automation", index))
                                .icon(gpui_kit::assets::IconName::Pencil)
                                .accessibility_label(format!("Edit {title}"))
                                .ghost()
                                .small()
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    let task = edit_task.clone();
                                    let _ = view.root.update(cx, |root, _| {
                                        *root.right_panel.task_to_edit.borrow_mut() = Some(task);
                                    });
                                    view.manage_tasks(cx);
                                })),
                        )
                        .when(task["schedule"]["type"] != "webhook", |row| {
                            row.child(
                                Button::new(("run-automation", index))
                                    .icon(gpui_kit::assets::IconName::Play)
                                    .accessibility_label(format!("Run {title} now"))
                                    .ghost()
                                    .small()
                                    .disabled(
                                        self.busy.is_some() || task["lastRunStatus"] == "running",
                                    )
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.mutate("scheduledTasks.runNow", json!({"id":id}), cx)
                                    })),
                            )
                        })
                        .child(
                            Switch::new(("toggle-automation", index))
                                .accessibility_label(format!(
                                    "{} {title}",
                                    if enabled { "Pause" } else { "Resume" }
                                ))
                                .checked(enabled)
                                .disabled(self.busy.is_some())
                                .on_click(cx.listener(move |view, enabled, _, cx| {
                                    view.mutate(
                                        "scheduledTasks.setEnabled",
                                        json!({"id":toggle_id,"enabled":enabled}),
                                        cx,
                                    )
                                })),
                        ),
                );
            }
            body = body.child(tasks);
        }
        body.children(self.error.clone().map(|error| {
            div()
                .px_3()
                .text_xs()
                .text_color(cx.theme().danger)
                .child(error)
        }))
    }
}

fn task_schedule_label(schedule: &Value) -> String {
    match schedule["type"].as_str() {
        Some("webhook") => "On webhook".into(),
        Some("once") => format!("Once, {}", schedule["runAt"].as_str().unwrap_or_default()),
        Some("interval") => {
            let ms = schedule["everyMs"].as_u64().unwrap_or_default();
            if ms.is_multiple_of(60000) {
                format!("Every {} min", ms / 60000)
            } else {
                format!("Every {} sec", (ms + 500) / 1000)
            }
        }
        _ => {
            let days = schedule["weekdays"].as_array().cloned().unwrap_or_default();
            let names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
            let label = if days.is_empty() {
                "Daily".into()
            } else if days.len() == 5
                && days
                    .iter()
                    .all(|day| day.as_u64().is_some_and(|day| (1..=5).contains(&day)))
            {
                "Weekdays".into()
            } else {
                days.iter()
                    .filter_map(|day| {
                        day.as_u64()
                            .and_then(|day| names.get(day as usize))
                            .copied()
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            format!(
                "{label} at {}",
                schedule["timeOfDay"].as_str().unwrap_or_default()
            )
        }
    }
}

fn split_branch_label(value: &str) -> (String, String) {
    let chars: Vec<char> = value.chars().collect();
    let mut keep = 10;
    if let Some(slash) = chars.iter().rposition(|c| *c == '/') {
        let segment = chars.len() - slash - 1;
        if slash > 0 && segment > 0 && segment <= 16 {
            keep = segment;
        }
    }
    if chars.len() <= keep + 4 {
        return (value.into(), String::new());
    }
    let mut cut = chars.len() - keep;
    let starts_word = |index: usize| matches!(chars[index - 1], '-' | '_' | '.' | '/');
    if !starts_word(cut)
        && let Some(index) = (cut + 1..=chars.len() - 4)
            .find(|index| starts_word(*index))
            .or_else(|| {
                (5.max(chars.len().saturating_sub(16))..cut)
                    .rev()
                    .find(|index| starts_word(*index))
            })
    {
        cut = index;
    }
    (chars[..cut].iter().collect(), chars[cut..].iter().collect())
}

fn task_next_label(task: &Value) -> String {
    let Some(time) = task["nextRunAt"]
        .as_str()
        .and_then(|time| humantime::parse_rfc3339(time).ok())
    else {
        return String::new();
    };
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 60
        * 60;
    let timestamp = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if timestamp <= now {
        let seconds = now - timestamp;
        return match seconds {
            0..60 => "just now".into(),
            60..3600 => format!("{}m ago", seconds / 60),
            3600..86400 => format!("{}h ago", seconds / 3600),
            _ => format!("{}d ago", seconds / 86400),
        };
    }
    let minutes = (timestamp - now).div_ceil(60);
    if minutes < 2 {
        "in under a minute".into()
    } else if minutes < 60 {
        format!("in {minutes}m")
    } else {
        let hours = (minutes + 30) / 60;
        if hours < 24 {
            format!("in {hours}h")
        } else {
            format!("in {}d", (hours + 12) / 24)
        }
    }
}
