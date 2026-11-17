use super::*;

pub(super) struct DetailsPane {
    pub(super) root: WeakEntity<Root>,
}
impl Render for DetailsPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.root
            .update(cx, |root, cx| root.render_thread_details(cx))
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
        if self.workspace_watch.as_ref().map(|watch| &watch.target) == target.as_ref() {
            return;
        }
        let same_directory = self
            .workspace_watch
            .as_ref()
            .zip(target.as_ref())
            .is_some_and(|(watch, target)| {
                watch.target.environment == target.environment && watch.target.cwd == target.cwd
            });
        self.workspace_watch = None;
        if !same_directory {
            self.branch_request += 1;
            self.workspace_status = workspace::Status::default();
            self.branch_anchor = None;
        }
        if let Some(target) = target
            && let Some(connection) = self.connection(&target.environment)
        {
            self.workspace_watch = Some(workspace::Watch::new(
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
        let card_height = self.lineage_height()
            + self.thread_extra_height()
            + 52.
            + if self.workspace_editor().is_some() {
                32.
            } else {
                0.
            }
            + if self.workspace_status.is_repo() {
                180.
            } else {
                0.
            }
            + if self.workspace_status.error.is_some() {
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
                self.workspace_watch.is_some()
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
                            .child(if self.branch_anchor == Some("details-branch") {
                                self.details_pane.clone().into_any_element()
                            } else {
                                self.details_pane
                                    .clone()
                                    .cached(
                                        StyleRefinement::default().w(px(280.)).h(px(card_height)),
                                    )
                                    .into_any_element()
                            }),
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

    fn render_thread_details(&self, cx: &Context<Self>) -> AnyElement {
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
        let groups = self.project_groups();
        let members = self
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
                    .child(div().flex_1().text_size(px(13.)).child(label))
                    .child(Icon::new(IconName::ChevronDown).size(px(16.))),
            )
            .ghost()
            .w_full()
            .h(px(32.))
            .px_2p5()
            .rounded(px(8.))
            .accessibility_label("Environment")
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
                        .on_click(cx.listener(move |root, _, window, cx| {
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
                    .px_2()
                    .pt_2()
                    .pb_2p5()
                    .child(host)
                    .children(editor_button),
            )
            .when(self.workspace_status.is_repo(), |card| {
                let changes = &self.workspace_status.local["workingTree"];
                card.child(
                    v_flex()
                        .px_2()
                        .pt_2()
                        .pb_2p5()
                        .border_t_1()
                        .border_color(theme.border.opacity(0.65))
                        .child(self.render_branch_control("details-branch", cx))
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
                                        .child("Changes")
                                        .child(
                                            div()
                                                .flex_1()
                                                .text_xs()
                                                .text_color(theme.muted_foreground)
                                                .child("Uncommitted"),
                                        )
                                        .child(div().text_xs().text_color(theme.success).child(
                                            format!(
                                                "+{}",
                                                changes["insertions"].as_u64().unwrap_or(0)
                                            ),
                                        ))
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(theme.muted_foreground)
                                                .child(format!(
                                                    "-{}",
                                                    changes["deletions"].as_u64().unwrap_or(0)
                                                )),
                                        ),
                                )
                                .ghost()
                                .w_full()
                                .h(px(32.))
                                .px_2p5()
                                .rounded(px(8.))
                                .accessibility_label("Open changes")
                                .on_click(cx.listener(|root, _, _, cx| root.toggle_diff(cx))),
                        )
                        .child(self.render_git_controls(cx)),
                )
            })
            .when_some(self.workspace_status.error.clone(), |card, error| {
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
            })
            .child(self.render_thread_pull_requests(cx))
            .child(self.render_lineage(cx))
            .child(self.render_thread_usage(cx))
            .into_any_element()
    }

    pub(super) fn render_workspace_mode(&self, cx: &Context<Self>) -> AnyElement {
        self.render_workspace_select(cx)
    }

    pub(super) fn workspace_editor(&self) -> Option<String> {
        let environment = &self.workspace_watch.as_ref()?.target.environment;
        let config = &self.model_catalogs.get(environment)?.0;
        workspace::preferred_editor(
            config.get("availableEditors")?.as_array()?,
            self.settings.preferred_editor.as_deref(),
        )
    }

    pub(super) fn render_branch_control(&self, id: &'static str, cx: &Context<Self>) -> AnyElement {
        let branch = self
            .draft
            .as_ref()
            .and_then(|draft| {
                draft
                    .workspace_strategy
                    .get("baseRef")
                    .or_else(|| draft.workspace_strategy.get("branch"))
            })
            .and_then(Value::as_str)
            .or_else(|| self.workspace_status.branch())
            .unwrap_or("Select branch")
            .to_string();
        let button = Button::new(id)
            .child(
                h_flex()
                    .w_full()
                    .gap_2p5()
                    .child(Icon::new(gpui_kit::assets::IconName::GitBranch).size(px(16.)))
                    .child(div().flex_1().truncate().child(branch)),
            )
            .ghost()
            .w_full()
            .h(px(32.))
            .px_2p5()
            .rounded(px(8.))
            .accessibility_label("Select branch")
            .role(accesskit::Role::ComboBox)
            .on_click(
                cx.listener(move |root, _, window, cx| root.open_branch_picker(id, window, cx)),
            );
        let mut popup = gpui_kit::base::Popup::new(id, button)
            .anchor(if id == "details-branch" {
                Anchor::TopRight
            } else {
                Anchor::BottomRight
            })
            .offset(px(4.));
        if self.branch_anchor == Some(id) {
            let weak = cx.entity().downgrade();
            let search = weak.clone();
            let cancel = weak.clone();
            let mut items = self
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
            if self.branch_cursor.is_some() {
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
                        Command::new(&self.branch_picker)
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
                                    root.branch_anchor = None;
                                    root.composer
                                        .update(cx, |state, cx| state.focus(window, cx));
                                    cx.notify();
                                });
                            }),
                    )
                    .when(
                        self.draft.as_ref().is_some_and(|draft| {
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
                                            .child("Start from remote"),
                                    )
                                    .child(
                                        gpui_kit::component::switch::Switch::new(
                                            "branch-from-origin",
                                        )
                                        .small()
                                        .accessibility_label("Start worktree from remote")
                                        .checked(self.draft.as_ref().is_some_and(|draft| {
                                            draft.workspace_strategy["startFromOrigin"].as_bool()
                                                == Some(true)
                                        }))
                                        .on_change(
                                            cx.listener(|root, checked, _, cx| {
                                                if let Some(draft) = root.draft.as_mut() {
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
        if !self.workspace_status.is_repo() {
            return;
        }
        self.branch_anchor = (self.branch_anchor != Some(id)).then_some(id);
        if self.branch_anchor.is_some() {
            self.branch_request += 1;
            self.branch_loading = false;
            self.branch_picker.update(cx, |state, cx| {
                state.set_query("", window, cx);
                state.focus(window, cx);
            });
            self.search_branches(String::new(), false, cx);
        } else {
            self.composer
                .update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    fn search_branches(&mut self, query: String, more: bool, cx: &mut Context<Self>) {
        let Some((environment, cwd)) = self.composer_cwd() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        self.branch_query = query.clone();
        if self.branch_loading {
            cx.notify();
            return;
        }
        self.branch_request += 1;
        let request = self.branch_request;
        self.branch_loading = true;
        let cursor = if more { self.branch_cursor } else { None };
        if !more {
            self.branch_refs.clear();
            self.branch_cursor = None;
        }
        let runtime = self.runtime.clone();
        let requested_query = query.clone();
        cx.spawn(async move |this, cx| {
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
                if root.branch_request != request || root.branch_anchor.is_none() {
                    return;
                }
                root.branch_loading = false;
                if root.branch_query != requested_query {
                    root.search_branches(root.branch_query.clone(), false, cx);
                    return;
                }
                match result {
                    Ok(result) => {
                        root.branch_refs
                            .extend(result["refs"].as_array().cloned().unwrap_or_default());
                        root.branch_cursor = result["nextCursor"].as_u64();
                    }
                    Err(error) => {
                        root.message = Some(format!("Could not load branches: {error:#}"))
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
            "detach-agent" => {
                self.lineage_action("detach", window, cx);
                return;
            }
            "choose-editor" => {
                self.change_setting("gpuiPreferredEditor", action.value.clone(), cx);
                return;
            }
            "host" => {
                if let Some(draft) = self.draft.as_mut() {
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
                    cx.spawn(async move |this, cx| {
                        let result = on_runtime(&runtime, async move {
                            connection.request("shell.openInEditor", input).await
                        })
                        .await;
                        if let Err(error) = result {
                            let _ = this.update(cx, |root, cx| {
                                root.message = Some(format!("Could not open editor: {error:#}"));
                                cx.notify();
                            });
                        }
                    })
                    .detach();
                }
                return;
            }
            "mode" => {
                if let Some(draft) = self.draft.as_mut() {
                    draft.workspace_strategy = if action.value.as_str() == Some("worktree") {
                        json!({"type":"worktree","baseRef":self.workspace_status.branch().unwrap_or("HEAD"),"startFromOrigin":false})
                    } else {
                        json!({"type":"root"})
                    };
                }
            }
            "more-branches" => {
                self.search_branches(self.branch_query.clone(), true, cx);
                return;
            }
            "branch" => self.select_workspace_branch(action.value.clone(), cx),
            _ => return,
        }
        if action.kind != "more-branches" {
            self.branch_anchor = None;
            self.composer
                .update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    fn select_workspace_branch(&mut self, reference: Value, cx: &mut Context<Self>) {
        let Some(name) = reference["name"].as_str().map(str::to_string) else {
            return;
        };
        if let Some(draft) = self.draft.as_mut()
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
        let draft_id = self.draft.as_ref().map(|draft| draft.id.clone());
        let project = if let Some((_, project_id, ..)) =
            self.draft_target().filter(|_| self.draft.is_some())
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
            self.workspace_status
                .local
                .pointer("/workingTree/rootPath")
                .and_then(Value::as_str)
                .filter(|_| current_worktree.is_none()),
            current_worktree,
            &reference,
        );
        let sessions = if current_worktree != target.worktree.as_deref() {
            self.open
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
        cx.spawn(async move |this, cx| {
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
                    Ok((branch, existing)) => if let Some(draft) = root.draft.as_mut().filter(|draft| Some(&draft.id) == draft_id.as_ref()) {
                        draft.workspace_strategy = match existing {
                            Some(path) => json!({"type":"existing_worktree","worktreePath":path,"branch":branch}),
                            None => json!({"type":"root","branch":branch})
                        };
                    },
                    Err(error) => root.message = Some(format!("Could not switch branch: {error:#}")),
                }
                cx.notify();
            });
        }).detach();
    }
}
