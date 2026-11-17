use super::*;
use gpui_kit::component::select::Select;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = t3code_gpui, no_json)]
pub(super) struct GitAction {
    pub kind: String,
    pub value: Value,
}

#[derive(Default)]
pub(super) struct State {
    pub busy: bool,
    pub result: Option<String>,
    pub progress: Vec<String>,
}

impl Root {
    pub(super) fn project_script_key(
        &mut self,
        typed: &keymap::Chord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.palette_open || self.settings_open || self.pr_state.page {
            return false;
        }
        let context = |name: &str| self.key_context(name, window, cx);
        if keymap::resolve(&self.key_rules, typed, &context)
            .is_some_and(|command| !command.starts_with("script.") || !command.ends_with(".run"))
        {
            return false;
        }
        let Some((environment, _)) = self.current_project() else {
            return false;
        };
        let Some(config) = self.config_of(&environment) else {
            return false;
        };
        let rules = keymap::rules(&config);
        let Some(id) = keymap::resolve(&rules, typed, &context)
            .and_then(|command| command.strip_prefix("script."))
            .and_then(|command| command.strip_suffix(".run"))
        else {
            return false;
        };
        let Some(script) = self
            .project_scripts()
            .into_iter()
            .find(|script| script["id"] == id)
        else {
            return false;
        };
        self.run_project_script(script, window, cx);
        true
    }

    pub(super) fn git_action(
        &mut self,
        action: &GitAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action.kind.as_str() {
            "commit-dialog" => self.open_commit_dialog(window, cx),
            "commit" => self.run_git_stack("commit", None, false, None, cx),
            "publish" => self.open_publish_dialog(window, cx),
            "script" => self.run_project_script(action.value.clone(), window, cx),
            "edit-script" => self.open_script_editor(action.value.clone(), window, cx),
            "refresh" | "pull" => self.run_git_request(&action.kind, cx),
            "push" | "commit_push" | "commit_push_pr" | "create_pr" => {
                let action = action.kind.clone();
                let on_default = self.conversation.workspace_status.local["isDefaultRef"].as_bool()
                    == Some(true);
                if on_default {
                    let weak = cx.entity().downgrade();
                    window.open_dialog(cx, move |dialog, _, _| {
                        let action = action.clone();
                        let weak = weak.clone();
                        dialog
                            .title("Continue on default branch?")
                            .footer(dialog_footer("Continue"))
                            .child(text!(
                                "This action publishes changes from the default branch."
                            ))
                            .on_ok(move |_, _, cx| {
                                let _ = weak.update(cx, |root, cx| {
                                    root.run_git_stack(&action, None, false, None, cx)
                                });
                                true
                            })
                    });
                } else {
                    self.run_git_stack(&action, None, false, None, cx);
                }
            }
            _ => {}
        }
    }

    fn open_commit_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input =
            cx.new(|cx| TextareaState::new(window, cx).placeholder("Leave empty to auto-generate"));
        let root = cx.entity().downgrade();
        let editor = cx.new(|_| CommitEditor {
            root,
            input,
            files: self.workspace_status.local["workingTree"]["files"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
            branch: self
                .workspace_status
                .branch()
                .unwrap_or("(detached HEAD)")
                .to_string(),
            default_branch: self.workspace_status.local["isDefaultRef"].as_bool() == Some(true),
            editing: false,
            excluded: HashSet::new(),
        });
        window.open_dialog(cx, move |dialog, _, cx| {
            use gpui_kit::component::dialog::{DialogAction, DialogFooter};
            let submit = editor.clone();
            let branch_submit = editor.clone();
            let empty = editor.read(cx).selected().is_empty();
            dialog.title(text!("Commit changes")).accessibility_label("Commit changes").width(px(512.)).centered()
                .child(div().text_sm().text_color(cx.theme().muted_foreground)
                    .child(text!("Review and confirm your commit. Leave the message blank to auto-generate one.")))
                .child(editor.clone())
                .footer(DialogFooter::new()
                    .child(Button::new("cancel-commit").label("Cancel").outline().small().on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(Button::new("commit-new-branch").label("Commit on new branch").outline().small().disabled(empty)
                        .on_click(move |_, window, cx| { if branch_submit.update(cx, |view, cx| view.submit(true, cx)) { window.close_dialog(cx); } }))
                    .child(div().w(px(72.)).child(DialogAction::new().child(Button::new("commit-selected").label("Commit").primary().small().disabled(empty)))))
                .on_ok(move |_, _, cx| submit.update(cx, |view, cx| view.submit(false, cx)))
        });
    }

    fn open_publish_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let repository = cx.new(|cx| InputState::new(window, cx).placeholder("owner/repository"));
        let provider = choice_select::state(
            &[("github", "GitHub"), ("gitlab", "GitLab")],
            "github",
            window,
            cx,
        );
        let visibility = choice_select::state(
            &[("private", "Private"), ("public", "Public")],
            "private",
            window,
            cx,
        );
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let repository_submit = repository.clone();
            let provider_submit = provider.clone();
            let visibility_submit = visibility.clone();
            let weak = weak.clone();
            dialog.title("Publish repository").footer(dialog_footer("Publish repository"))
                .child(Select::new(&provider).title_prefix("Provider"))
                .child(Input::new(&repository))
                .child(Select::new(&visibility).title_prefix("Visibility"))
                .on_ok(move |_, _, cx| {
                    let repository = repository_submit.read(cx).value().trim().to_string();
                    if repository.is_empty() { return false; }
                    let provider = provider_submit.read(cx).selected_value().copied().unwrap_or("github");
                    let visibility = visibility_submit.read(cx).selected_value().copied().unwrap_or("private");
                    let _ = weak.update(cx, |root, cx| {
                        root.run_git_rpc("sourceControl.publishRepository", json!({"provider":provider,"repository":repository,"visibility":visibility}), cx);
                    });
                    true
                })
        });
    }

    fn run_git_request(&mut self, kind: &str, cx: &mut Context<Self>) {
        self.run_git_rpc(
            if kind == "pull" {
                "vcs.pull"
            } else {
                "vcs.refreshStatus"
            },
            json!({}),
            cx,
        );
    }

    fn run_git_rpc(&mut self, method: &'static str, mut input: Value, cx: &mut Context<Self>) {
        if self.git_state.busy {
            return;
        }
        let Some((environment, cwd)) = self.composer_cwd() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        input["cwd"] = json!(cwd);
        self.git_state.busy = true;
        self.conversation.workspace_revision += 1;
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let result = on_runtime(
                &runtime,
                async move { connection.request(method, input).await },
            )
            .await;
            let _ = this.update(cx, |root, cx| {
                root.git_state.busy = false;
                root.git_state.result = Some(match result {
                    Ok(value) => value["status"].as_str().unwrap_or("Refreshed").to_string(),
                    Err(error) => format!("{error:#}"),
                });
                root.conversation.workspace_revision += 1;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn run_git_stack(
        &mut self,
        action: &str,
        message: Option<String>,
        feature_branch: bool,
        files: Option<Vec<String>>,
        cx: &mut Context<Self>,
    ) {
        if self.git_state.busy {
            return;
        }
        let Some((environment, cwd)) = self.composer_cwd() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let mut input =
            json!({"actionId":uuid::Uuid::new_v4().to_string(), "cwd":cwd, "action":action});
        if let Some(files) = files {
            input["filePaths"] = json!(files);
        }
        if let Some(message) = message {
            input["commitMessage"] = json!(message);
        }
        if feature_branch {
            input["featureBranch"] = json!(true);
        }
        if let Some((_, id)) = self.current() {
            input["threadId"] = json!(id);
        } else if let Some((_, project_id)) = self.draft_project() {
            input["projectId"] = json!(project_id);
        }
        self.git_state.busy = true;
        self.git_state.result = None;
        self.git_state.progress.clear();
        self.conversation.workspace_revision += 1;
        let runtime = self.runtime.clone();
        let (sender, receiver) = async_channel::bounded(32);
        let task = runtime.spawn(async move {
            let result: anyhow::Result<()> = async {
                let mut stream = connection
                    .subscribe(
                        "git.runStackedAction",
                        input,
                        t3_client::rpc::StreamLimits::new(64, 8 << 20)
                            .expect("bounded git action stream"),
                    )
                    .await?;
                let mut finished = false;
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk?;
                    for event in chunk.values() {
                        finished |= matches!(
                            event["kind"].as_str(),
                            Some("action_finished" | "action_failed")
                        );
                        sender.send(Ok(event.clone())).await?;
                    }
                    chunk.accept();
                }
                anyhow::ensure!(finished, "Source control action ended without a result");
                Ok(())
            }
            .await;
            if let Err(error) = result {
                let _ = sender.send(Err(format!("{error:#}"))).await;
            }
        });
        cx.spawn(async move |this, cx| {
            while let Ok(event) = receiver.recv().await {
                if this
                    .update(cx, |root, cx| {
                        match event {
                            Ok(event) => match event["kind"].as_str() {
                                Some("action_finished") => {
                                    root.invalidate_panel_diff(cx);
                                    root.git_state.result = Some(
                                        event
                                            .pointer("/result/toast/title")
                                            .and_then(Value::as_str)
                                            .unwrap_or("Completed")
                                            .into(),
                                    );
                                }
                                Some("action_failed") => {
                                    root.git_state.result = Some(
                                        event["message"].as_str().unwrap_or("Action failed").into(),
                                    );
                                }
                                _ => {
                                    if let Some(text) =
                                        event["text"].as_str().or_else(|| event["label"].as_str())
                                    {
                                        if root.git_state.progress.len() == 200 {
                                            root.git_state.progress.remove(0);
                                        }
                                        root.git_state.progress.push(text.into());
                                    }
                                }
                            },
                            Err(error) => root.git_state.result = Some(error),
                        }
                        root.conversation.workspace_revision += 1;
                        cx.notify();
                    })
                    .is_err()
                {
                    task.abort();
                    return;
                }
            }
            let _ = this.update(cx, |root, cx| {
                root.git_state.busy = false;
                root.conversation.workspace_revision += 1;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn current_project(&self) -> Option<(String, Value)> {
        if let Some((environment, id)) = self
            .draft_project()
            .filter(|_| self.conversation.draft.is_some())
        {
            return Some((environment.clone(), self.project_value(&environment, &id)?));
        }
        let (environment, id) = self.current()?;
        let thread = self.thread_value(&environment, &id)?;
        Some((
            environment.clone(),
            self.project_value(&environment, thread["projectId"].as_str()?)?,
        ))
    }

    fn project_scripts(&self) -> Vec<Value> {
        let Some((environment, project)) = self.current_project() else {
            return Vec::new();
        };
        let config = self.config_of(&environment).unwrap_or(Value::Null);
        let settings = &config["settings"];
        let id = project["id"].as_str().unwrap_or_default();
        if let Some(scripts) =
            settings["projectSettingsOverrides"][id]["defaultProjectScripts"].as_array()
        {
            return scripts.clone();
        }
        if settings["projectSettingsFolded"] == true {
            return settings["defaultProjectScripts"]
                .as_array()
                .cloned()
                .unwrap_or_default();
        }
        if let Some(scripts) = settings["projectScriptOverrides"][id].as_array() {
            return scripts.clone();
        }
        if settings["projectScriptOverrides"].get(id) != Some(&Value::Null)
            && let Some(scripts) = project["scripts"]
                .as_array()
                .filter(|scripts| !scripts.is_empty())
        {
            return scripts.clone();
        }
        settings["defaultProjectScripts"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    fn open_script_editor(&mut self, original: Value, window: &mut Window, cx: &mut Context<Self>) {
        let Some((environment, project)) = self.current_project() else {
            return;
        };
        let name = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Test");
            input.set_value(original["name"].as_str().unwrap_or_default(), window, cx);
            input
        });
        let command = cx.new(|cx| {
            let mut input = TextareaState::new(window, cx).placeholder("bun test");
            input.set_value(original["command"].as_str().unwrap_or_default(), window, cx);
            input
        });
        let preview = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("http://localhost:5173");
            input.set_value(
                original["previewUrl"].as_str().unwrap_or_default(),
                window,
                cx,
            );
            input
        });
        let script_command = original["id"].as_str().map(|id| format!("script.{id}.run"));
        let config = self.config_of(&environment).unwrap_or(Value::Null);
        let previous_key = config["keybindings"]
            .as_array()
            .into_iter()
            .flatten()
            .rev()
            .find(|binding| binding["command"].as_str() == script_command.as_deref())
            .map(script_keybinding)
            .unwrap_or_default();
        let keybinding = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Press shortcut");
            input.set_value(previous_key, window, cx);
            input
        });
        let editing = original["id"].is_string();
        let root = cx.entity().downgrade();
        let editor = cx.new(|_| ScriptEditor {
            root,
            environment,
            project: project["id"].as_str().unwrap_or_default().into(),
            name,
            command,
            preview,
            keybinding,
            icon: match original["icon"].as_str() {
                Some("test") => "test",
                Some("lint") => "lint",
                Some("configure") => "configure",
                Some("build") => "build",
                Some("debug") => "debug",
                _ => "play",
            },
            flags: [
                original["runOnWorktreeCreate"] == true,
                original["async"] == false,
                original["runOnSettle"] == true,
                original["stopOnSettle"] == true,
                original["autoOpenPreview"] == true,
            ],
            original,
            error: None,
            saving: false,
        });
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title(text!(if editing { "Edit Action" } else { "Add Action" }))
                .accessibility_label(if editing { "Edit Action" } else { "Add Action" })
                .width(px(512.))
                .child(editor.clone())
        });
    }

    fn run_project_script(&mut self, script: Value, window: &mut Window, cx: &mut Context<Self>) {
        let Some((environment, cwd)) = self.composer_cwd() else {
            return;
        };
        let Some((_, thread_id)) = self.current() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let Some(key) = self.terminal_key() else {
            return;
        };
        let id = format!("script-{}", uuid::Uuid::new_v4());
        let mut layout = self.terminal_store.get(&key);
        layout.open = true;
        layout.add(id.clone(), None);
        let runtime = self.runtime.clone();
        let command = script["command"].as_str().unwrap_or_default().to_string();
        let worktree = self
            .thread_value(&environment, &thread_id)
            .and_then(|thread| thread["worktreePath"].as_str().map(str::to_string));
        cx.spawn_in(window, async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                connection.request("terminal.open", json!({"threadId":thread_id,"terminalId":id,"cwd":cwd,"worktreePath":worktree,"cols":120,"rows":30,"stopOnSettle":script["stopOnSettle"].as_bool().unwrap_or(false)})).await?;
                connection.request("terminal.write", json!({"threadId":thread_id,"terminalId":id,"data":format!(" {command}\r")})).await
            }).await;
            let _ = this.update_in(cx, |root, window, cx| {
                match result {
                    Ok(_) => {
                        root.terminal_store.set(key, &layout);
                        root.terminal_revision += 1;
                        root.save_terminal_layout(cx);
                        root.sync_terminal_drawers(window, cx);
                    }
                    Err(error) => root.conversation.message = Some(format!("Script failed: {error:#}")),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub(super) fn render_git_controls(&self, cx: &Context<Self>) -> AnyElement {
        let focus = self.focus.clone();
        let busy = self.git_state.busy;
        let (label, action, enabled) = quick_action(
            &self.conversation.workspace_status.local,
            &self.conversation.workspace_status.remote,
            busy,
        );
        let scripts = self.project_scripts();
        let script_focus = focus.clone();
        let script_root = cx.entity().downgrade();
        let no_scripts = scripts.is_empty();
        let primary_script = scripts
            .iter()
            .find(|script| script["runOnWorktreeCreate"] != true && script["runOnSettle"] != true)
            .or_else(|| scripts.iter().find(|script| script["runOnSettle"] != true))
            .cloned();
        let local = self.conversation.workspace_status.local.clone();
        let remote = self.conversation.workspace_status.remote.clone();
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .id("git-controls")
                    .role(accesskit::Role::Group)
                    .aria_label("Git actions")
                    .child(
                        Button::new("git-commit")
                            .label(label)
                            .ghost()
                            .flex_1()
                            .disabled(!enabled)
                            .on_click(cx.listener(move |root, _, window, cx| {
                                root.git_action(
                                    &GitAction {
                                        kind: action.into(),
                                        value: Value::Null,
                                    },
                                    window,
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new("git-actions")
                            .icon(IconName::ChevronDown)
                            .accessibility_label("Git action options")
                            .ghost()
                            .compact()
                            .disabled(busy)
                            .dropdown_menu(move |menu, _, _| {
                                let mut menu = menu;
                                for (label, kind) in [
                                    ("Commit", "commit-dialog"),
                                    ("Push", "push"),
                                    ("Pull", "pull"),
                                    ("Commit and push", "commit_push"),
                                    ("Create pull request", "create_pr"),
                                    ("Commit, push, and create pull request", "commit_push_pr"),
                                    ("Refresh", "refresh"),
                                ] {
                                    let changes =
                                        local["hasWorkingTreeChanges"].as_bool() == Some(true);
                                    let behind = remote["behindCount"].as_u64().unwrap_or(0) > 0;
                                    let can_push = local["refName"].is_string()
                                        && !behind
                                        && (remote["hasUpstream"].as_bool() == Some(true)
                                            || local["hasPrimaryRemote"].as_bool() == Some(true));
                                    let disabled = match kind {
                                        "commit-dialog" => !changes,
                                        "push" => {
                                            changes
                                                || !can_push
                                                || remote["aheadCount"].as_u64().unwrap_or(0) == 0
                                        }
                                        "pull" => {
                                            remote["hasUpstream"].as_bool() != Some(true)
                                                || remote["behindCount"].as_u64().unwrap_or(0) == 0
                                        }
                                        "commit_push" | "commit_push_pr" => !changes || !can_push,
                                        "create_pr" => {
                                            !can_push
                                                || remote["pr"]["state"].as_str() == Some("open")
                                        }
                                        _ => false,
                                    };
                                    menu = menu.menu_with_disabled(
                                        label,
                                        Box::new(GitAction {
                                            kind: kind.into(),
                                            value: Value::Null,
                                        }),
                                        disabled,
                                    );
                                }
                                menu.action_context(focus.clone())
                            }),
                    ),
            )
            .child(
                h_flex()
                    .id("project-scripts")
                    .role(accesskit::Role::Group)
                    .aria_label("Project scripts")
                    .when_some(primary_script, |row, script| {
                        let name = script["name"].as_str().unwrap_or_default().to_string();
                        row.child(
                            Button::new("run-project-script")
                                .label(name.clone())
                                .accessibility_label(format!("Run {name}"))
                                .ghost()
                                .flex_1()
                                .on_click(cx.listener(move |root, _, window, cx| {
                                    root.run_project_script(script.clone(), window, cx)
                                })),
                        )
                    })
                    .when(!no_scripts, |row| {
                        row.child(
                            Button::new("project-script-actions")
                                .icon(IconName::ChevronDown)
                                .accessibility_label("Script actions")
                                .ghost()
                                .compact()
                                .dropdown_menu(move |mut menu, _, cx| {
                                    for script in &scripts {
                                        let name =
                                            script["name"].as_str().unwrap_or_default().to_string();
                                        let edit = script.clone();
                                        let root = script_root.clone();
                                        let popup = cx.entity().downgrade();
                                        menu =
                                            menu.item(
                                                PopupMenuItem::element(move |_, _| {
                                                    let edit = edit.clone();
                                                    let root = root.clone();
                                                    let popup = popup.clone();
                                                    h_flex()
                                                    .w_full()
                                                    .child(name.clone())
                                                    .child(div().flex_1())
                                                    .child(
                                                        Button::new(SharedString::from(format!(
                                                            "edit-{}",
                                                            edit["id"]
                                                        )))
                                                        .icon(gpui_kit::assets::IconName::Settings)
                                                        .accessibility_label(format!("Edit {name}"))
                                                        .ghost()
                                                        .size(px(24.))
                                                        .on_click(move |_, window, cx| {
                                                            cx.stop_propagation();
                                                            let _ = popup.update(cx, |_, cx| {
                                                                cx.emit(gpui::DismissEvent)
                                                            });
                                                            let _ = root.update(cx, |root, cx| {
                                                                root.open_script_editor(
                                                                    edit.clone(),
                                                                    window,
                                                                    cx,
                                                                )
                                                            });
                                                        }),
                                                    )
                                                })
                                                .accessibility_label(
                                                    script["name"]
                                                        .as_str()
                                                        .unwrap_or_default()
                                                        .to_string(),
                                                )
                                                .action(Box::new(GitAction {
                                                    kind: "script".into(),
                                                    value: script.clone(),
                                                })),
                                            );
                                    }
                                    menu.menu(
                                        "Add project script",
                                        Box::new(GitAction {
                                            kind: "edit-script".into(),
                                            value: Value::Null,
                                        }),
                                    )
                                    .action_context(script_focus.clone())
                                }),
                        )
                    })
                    .when(no_scripts, |row| {
                        row.child(
                            Button::new("add-project-script")
                                .label("Add project script")
                                .accessibility_label("Add project script")
                                .ghost()
                                .w_full()
                                .on_click(cx.listener(|root, _, window, cx| {
                                    root.open_script_editor(Value::Null, window, cx)
                                })),
                        )
                    }),
            )
            .children(
                self.git_state
                    .progress
                    .last()
                    .filter(|_| busy)
                    .cloned()
                    .map(|progress| div().text_xs().px_2().truncate().child(progress)),
            )
            .children(
                self.git_state
                    .result
                    .clone()
                    .map(|result| div().text_xs().px_2().child(result)),
            )
            .into_any_element()
    }
}

fn quick_action(local: &Value, remote: &Value, busy: bool) -> (String, &'static str, bool) {
    let disabled = ("Commit".into(), "commit", false);
    if busy || !local["refName"].is_string() {
        return disabled;
    }
    let upstream = remote["hasUpstream"].as_bool() == Some(true);
    let primary = local["hasPrimaryRemote"].as_bool() == Some(true);
    let default = local["isDefaultRef"].as_bool() == Some(true);
    let open = remote["pr"]["state"].as_str() == Some("open");
    let ahead = remote["aheadCount"].as_u64().unwrap_or(0) > 0;
    let behind = remote["behindCount"].as_u64().unwrap_or(0) > 0;
    let delta = remote["aheadOfDefaultCount"]
        .as_u64()
        .or_else(|| remote["aheadCount"].as_u64())
        .unwrap_or(0)
        > 0;
    let term = if local["sourceControlProvider"]["kind"].as_str() == Some("gitlab") {
        "MR"
    } else {
        "PR"
    };
    if local["hasWorkingTreeChanges"].as_bool() == Some(true) {
        if behind || (!upstream && !primary) {
            return ("Commit".into(), "commit", true);
        }
        if open || default {
            return ("Commit & push".into(), "commit_push", true);
        }
        return (format!("Commit, push & {term}"), "commit_push_pr", true);
    }
    if !upstream {
        if !primary {
            return ("Publish repository".into(), "publish", true);
        }
        if !ahead {
            return (if open { "Commit" } else { "Push" }.into(), "push", false);
        }
    } else {
        if ahead && behind {
            return ("Sync ref".into(), "pull", false);
        }
        if behind {
            return ("Pull".into(), "pull", true);
        }
    }
    if ahead {
        if open || default {
            return (
                "Push".into(),
                if default { "commit_push" } else { "push" },
                true,
            );
        }
        return (format!("Push & create {term}"), "create_pr", true);
    }
    if !open && delta && !default {
        return (format!("Create {term}"), "create_pr", true);
    }
    disabled
}

pub(super) fn dialog_footer(label: impl Into<SharedString>) -> AnyElement {
    use gpui_kit::component::dialog::{DialogAction, DialogFooter};
    DialogFooter::new()
        .child(
            Button::new("cancel-action")
                .label("Cancel")
                .outline()
                .on_click(|_, window, cx| window.close_dialog(cx)),
        )
        .child(
            div().w(px(180.)).h(px(36.)).child(
                DialogAction::new().child(
                    Button::new("confirm-action")
                        .label(label)
                        .primary()
                        .w_full(),
                ),
            ),
        )
        .into_any_element()
}

struct CommitEditor {
    root: WeakEntity<Root>,
    input: Entity<TextareaState>,
    files: Vec<Value>,
    branch: String,
    default_branch: bool,
    editing: bool,
    excluded: HashSet<String>,
}

impl CommitEditor {
    fn selected(&self) -> Vec<String> {
        self.files
            .iter()
            .filter_map(|file| file["path"].as_str())
            .filter(|path| !self.excluded.contains(*path))
            .map(str::to_string)
            .collect()
    }

    fn submit(&self, branch: bool, cx: &mut App) -> bool {
        let files = self.selected();
        if files.is_empty() {
            return false;
        }
        let message = self.input.read(cx).value().trim().to_string();
        let _ = self.root.update(cx, |root, cx| {
            root.run_git_stack(
                "commit",
                (!message.is_empty()).then_some(message),
                branch,
                (!self.excluded.is_empty()).then_some(files),
                cx,
            )
        });
        true
    }
}

impl Render for CommitEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::checkbox::Checkbox;
        let selected = self.selected();
        let additions: u64 = self
            .files
            .iter()
            .filter(|file| {
                file["path"]
                    .as_str()
                    .is_some_and(|path| !self.excluded.contains(path))
            })
            .map(|file| file["insertions"].as_u64().unwrap_or(0))
            .sum();
        let deletions: u64 = self
            .files
            .iter()
            .filter(|file| {
                file["path"]
                    .as_str()
                    .is_some_and(|path| !self.excluded.contains(path))
            })
            .map(|file| file["deletions"].as_u64().unwrap_or(0))
            .sum();
        v_flex().gap_3().child(v_flex().p_3().gap_2().rounded_xl().bg(cx.theme().foreground.opacity(0.035))
            .child(h_flex().gap_2().child(div().text_color(cx.theme().muted_foreground).child(text!("Branch")))
                .child(div().flex_1().child(text!(self.branch.clone())))
                .when(self.default_branch, |row| row.child(div().text_color(cx.theme().warning).child(text!("Default branch")))))
            .child(h_flex().gap_2()
                .when(self.editing, |row| row.child(Checkbox::new("commit-all-files").checked(self.excluded.is_empty()).on_click(cx.listener(|view, _, window, cx| {
                    if view.excluded.is_empty() { view.excluded = view.files.iter().filter_map(|file| file["path"].as_str().map(str::to_string)).collect(); }
                    else { view.excluded.clear(); }
                    window.refresh(); cx.notify();
                }))))
                .child(text!("Files"))
                .when(!self.excluded.is_empty() && !self.editing, |row| row.child(h_flex().child(text!("(")).child(text!(selected.len().to_string())).child(text!(" of ")).child(text!(self.files.len().to_string())).child(text!(")"))))
                .child(div().flex_1())
                .child(Button::new("commit-edit-files").label(if self.editing { "Done" } else { "Edit" }).ghost().small().on_click(cx.listener(|view, _, window, cx| { view.editing = !view.editing; window.refresh(); cx.notify(); }))))
            .child(v_flex().id("commit-files").h(px(176.)).overflow_y_scroll().gap_1().rounded_lg().bg(cx.theme().foreground.opacity(0.025)).children(self.files.iter().map(|file| {
                let path = file["path"].as_str().unwrap_or_default().to_string();
                let check_path = path.clone();
                let open_path = path.clone();
                let excluded = self.excluded.contains(&path);
                h_flex().gap_2().px_2().py_1()
                    .when(self.editing, |row| row.child(Checkbox::new(SharedString::from(format!("commit-file-{path}"))).checked(!excluded).on_click(cx.listener(move |view, _, window, cx| {
                        if !view.excluded.remove(&check_path) { view.excluded.insert(check_path.clone()); }
                        window.refresh(); cx.notify();
                    }))))
                    .child(Button::new(SharedString::from(format!("commit-open-{path}"))).child(div().w_full().child(path.clone())).accessibility_label(format!("Open {path} in editor")).ghost().compact().flex_1()
                        .on_click(cx.listener(move |view, _, window, cx| { let _ = view.root.update(cx, |root, cx| {
                            if let Some((_, cwd)) = root.composer_cwd() { root.workspace_action(&WorkspaceAction { kind: "editor".into(), value: json!({"path":format!("{cwd}/{open_path}"),"editor":root.workspace_editor()}) },window,cx); }
                        }); })))
                    .child(div().text_xs().child(if excluded { "Excluded".into() } else { format!("+{} / -{}",file["insertions"].as_u64().unwrap_or(0),file["deletions"].as_u64().unwrap_or(0)) }))
            })))
            .child(h_flex().justify_end().child(text!("+")).child(text!(additions.to_string())).child(text!(" / ")).child(text!("-")).child(text!(deletions.to_string()))))
            .child(text!("Commit message (optional)")).child(Textarea::new(&self.input).h(px(68.)))
    }
}

struct ScriptEditor {
    root: WeakEntity<Root>,
    environment: String,
    project: String,
    original: Value,
    name: Entity<InputState>,
    command: Entity<TextareaState>,
    preview: Entity<InputState>,
    keybinding: Entity<InputState>,
    icon: &'static str,
    flags: [bool; 5],
    error: Option<String>,
    saving: bool,
}

impl ScriptEditor {
    fn submit(&mut self, delete: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let name = self.name.read(cx).value().trim().to_string();
        let command = self.command.read(cx).value().trim().to_string();
        if !delete && (name.is_empty() || command.is_empty()) {
            self.error = Some(
                if name.is_empty() {
                    "Name is required."
                } else {
                    "Command is required."
                }
                .into(),
            );
            cx.notify();
            return;
        }
        let preview = self.preview.read(cx).value().trim().to_string();
        let Ok((connection, runtime, mut scripts, config)) = self.root.update(cx, |root, _| {
            (
                root.connection(&self.environment),
                root.runtime.clone(),
                root.project_scripts(),
                root.config_of(&self.environment).unwrap_or(Value::Null),
            )
        }) else {
            return;
        };
        let Some(connection) = connection else {
            self.error = Some("This connection cannot change project actions.".into());
            cx.notify();
            return;
        };
        let id = self.original["id"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| {
                let base = name
                    .to_ascii_lowercase()
                    .split(|ch: char| !ch.is_ascii_alphanumeric())
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join("-");
                let base = if base.is_empty() { "script" } else { &base };
                let mut id = base.to_string();
                let mut suffix = 2;
                while scripts.iter().any(|script| script["id"] == id) {
                    id = format!("{base}-{suffix}");
                    suffix += 1;
                }
                id
            });
        if delete {
            scripts.retain(|script| script["id"] != id);
        } else {
            let mut value = json!({"id":id,"name":name,"command":command,"icon":self.icon,"runOnWorktreeCreate":self.flags[0]});
            if self.flags[0] && self.flags[1] {
                value["async"] = json!(false);
            }
            if self.flags[2] {
                value["runOnSettle"] = json!(true);
            }
            if self.flags[3] {
                value["stopOnSettle"] = json!(true);
            }
            if !preview.is_empty() {
                value["previewUrl"] = json!(preview);
                value["autoOpenPreview"] = json!(self.flags[4]);
            }
            for script in &mut scripts {
                if script["id"] != id {
                    if self.flags[0] {
                        script["runOnWorktreeCreate"] = json!(false);
                    }
                    if self.flags[2] {
                        script["runOnSettle"] = json!(false);
                    }
                }
            }
            if let Some(existing) = scripts.iter_mut().find(|script| script["id"] == id) {
                *existing = value;
            } else {
                scripts.push(value);
            }
        }
        let mut overrides = config["settings"]["projectSettingsOverrides"][&self.project]
            .as_object()
            .cloned()
            .unwrap_or_default();
        overrides.insert("defaultProjectScripts".into(), json!(scripts));
        let patch = json!({"projectSettingsOverrides":{&self.project:overrides}});
        let command_id = format!("script.{id}.run");
        let key = if delete {
            String::new()
        } else {
            self.keybinding.read(cx).value().trim().to_string()
        };
        let previous = config["keybindings"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|binding| binding["command"] == command_id && binding["whenAst"].is_null())
            .map(|binding| json!({"command": command_id, "key": script_keybinding(binding)}))
            .collect::<Vec<_>>();
        let retained_elsewhere = delete
            && (config["settings"]["defaultProjectScripts"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|script| script["id"] == id)
                || config["settings"]["projectSettingsOverrides"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .any(|(project, entry)| {
                        project != &self.project
                            && entry["defaultProjectScripts"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .any(|script| script["id"] == id)
                    }));
        let reconcile_key = !retained_elsewhere
            && previous
                .last()
                .and_then(|value| value["key"].as_str())
                .unwrap_or_default()
                != key;
        self.saving = true;
        self.error = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                connection
                    .request("server.updateSettings", json!({"patch":patch}))
                    .await?;
                if reconcile_key {
                    let remove_count = if key.is_empty() {
                        previous.len()
                    } else {
                        previous.len().saturating_sub(1)
                    };
                    for rule in previous.iter().take(remove_count) {
                        connection
                            .request("server.removeKeybinding", rule.clone())
                            .await?;
                    }
                    if !key.is_empty() {
                        let mut input = json!({"command": command_id, "key": key});
                        if let Some(previous) = previous.last() {
                            input["replace"] = previous.clone();
                        }
                        connection.request("server.upsertKeybinding", input).await?;
                    }
                }
                Ok(())
            })
            .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.saving = false;
                match result {
                    Ok(_) => window.close_dialog(cx),
                    Err(error) => view.error = Some(format!("{error:#}")),
                }
                let _ = view.root.update(cx, |root, cx| {
                    root.workspace_revision += 1;
                    cx.notify();
                });
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for ScriptEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::switch::Switch;
        let weak = cx.entity().downgrade();
        let editing = self.original["id"].is_string();
        v_flex()
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                if !view.keybinding.read(cx).focus_handle(cx).is_focused(window) || event.keystroke.key == "tab" { return; }
                cx.stop_propagation();
                let stroke = &event.keystroke;
                let key = if stroke.key == "backspace" || stroke.key == "delete" { String::new() } else {
                    let mut parts = Vec::new();
                    if stroke.modifiers.control { parts.push(if cfg!(target_os = "macos") { "ctrl" } else { "mod" }); }
                    if stroke.modifiers.platform { parts.push(if cfg!(target_os = "macos") { "mod" } else { "meta" }); }
                    if stroke.modifiers.alt { parts.push("alt"); }
                    if stroke.modifiers.shift { parts.push("shift"); }
                    parts.push(match stroke.key.as_str() { "escape" => "esc", "up" => "arrowup", "down" => "arrowdown", "left" => "arrowleft", "right" => "arrowright", key => key });
                    parts.join("+")
                };
                view.keybinding.update(cx, |input, cx| input.set_value(key, window, cx));
                cx.notify();
            }))
            .gap_3()
            .child(text!(
                "Actions are project-scoped commands you can run from the top bar or keybindings."
            ))
            .child(text!("Name"))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("script-icon")
                            .icon(gpui_kit::assets::IconName::Play)
                            .accessibility_label("Choose icon")
                            .dropdown_menu(move |mut menu, _, _| {
                                for (id, label) in [
                                    ("play", "Play"),
                                    ("test", "Test"),
                                    ("lint", "Lint"),
                                    ("configure", "Configure"),
                                    ("build", "Build"),
                                    ("debug", "Debug"),
                                ] {
                                    let weak = weak.clone();
                                    menu = menu.item(PopupMenuItem::new(label).on_click(
                                        move |_, _, cx| {
                                            let _ = weak.update(cx, |view, cx| {
                                                view.icon = id;
                                                cx.notify();
                                            });
                                        },
                                    ));
                                }
                                menu
                            }),
                    )
                    .child(Input::new(&self.name).aria_label("Name")),
            )
            .child(text!("Keybinding"))
            .child(Input::new(&self.keybinding).aria_label("Keybinding").readonly(true))
            .child(div().text_xs().child("Press a shortcut. Use Backspace to clear. Shortcuts are environment-wide. Projects using the same action share its shortcut."))
            .child(text!("Command"))
            .child(
                Textarea::new(&self.command)
                    .aria_label("Command")
                    .h(px(64.)),
            )
            .child(text!("Preview URL (optional)"))
            .child(Input::new(&self.preview).aria_label("Preview URL (optional)"))
            .child(text!(
                "Open this URL in the in-app preview when this action runs."
            ))
            .children(
                [
                    "Run automatically on worktree creation",
                    "Wait for it to finish before the agent starts",
                    "Run in the thread's worktree when the thread settles",
                    "Stop this action when the thread settles",
                    "Open preview automatically when this action runs",
                ]
                .into_iter()
                .enumerate()
                .map(|(index, label)| {
                    Switch::new(label)
                        .label(label)
                        .checked(self.flags[index])
                        .disabled(
                            self.saving
                                || index == 1 && !self.flags[0]
                                || index == 4 && self.preview.read(cx).value().trim().is_empty(),
                        )
                        .on_click(cx.listener(move |view, checked, _, cx| {
                            view.flags[index] = *checked;
                            cx.notify();
                        }))
                }),
            )
            .children(
                self.error
                    .clone()
                    .map(|error| div().text_color(cx.theme().danger).child(text!(error))),
            )
            .child(
                h_flex()
                    .gap_2()
                    .when(editing, |row| {
                        row.child(
                            Button::new("delete-script")
                                .label("Delete")
                                .disabled(self.saving)
                                .on_click(cx.listener(|view, _, window, cx| {
                                    let editor = cx.entity().downgrade();
                                    let name = view.name.read(cx).value().to_string();
                                    window.open_dialog(cx, move |dialog, _, _| {
                                        let editor = editor.clone();
                                        dialog
                                            .title(text!(format!("Delete action \"{name}\"?")))
                                            .footer(dialog_footer("Delete"))
                                            .on_ok(move |_, window, cx| {
                                                let _ = editor.update(cx, |view, cx| {
                                                    view.submit(true, window, cx)
                                                });
                                                true
                                            })
                                    });
                                })),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        Button::new("cancel-script")
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("save-script")
                            .label(if self.saving {
                                "Saving…"
                            } else if editing {
                                "Save changes"
                            } else {
                                "Save action"
                            })
                            .disabled(self.saving)
                            .primary()
                            .on_click(
                                cx.listener(|view, _, window, cx| view.submit(false, window, cx)),
                            ),
                    ),
            )
    }
}

fn script_keybinding(binding: &Value) -> String {
    let shortcut = &binding["shortcut"];
    let mut parts = Vec::new();
    for (field, label) in [
        ("modKey", "mod"),
        ("ctrlKey", "ctrl"),
        ("metaKey", "meta"),
        ("altKey", "alt"),
        ("shiftKey", "shift"),
    ] {
        if shortcut[field] == true {
            parts.push(label);
        }
    }
    parts.push(match shortcut["key"].as_str().unwrap_or_default() {
        " " => "space",
        "escape" => "esc",
        key => key,
    });
    parts.join("+")
}

#[cfg(test)]
mod tests {
    use super::quick_action;
    use serde_json::json;
    #[test]
    fn quick_actions_match_reference_transitions() {
        let local = json!({"refName":"feature/test","hasPrimaryRemote":true,"isDefaultRef":false});
        for (remote, changes, label, action, enabled) in [
            (
                json!({"hasUpstream":true,"aheadCount":1,"behindCount":0}),
                false,
                "Push & create PR",
                "create_pr",
                true,
            ),
            (
                json!({"hasUpstream":true,"aheadCount":1,"behindCount":1}),
                false,
                "Sync ref",
                "pull",
                false,
            ),
            (
                json!({"hasUpstream":true,"aheadCount":0,"behindCount":1}),
                false,
                "Pull",
                "pull",
                true,
            ),
            (
                json!({"hasUpstream":true,"aheadCount":0,"behindCount":1}),
                true,
                "Commit",
                "commit",
                true,
            ),
            (
                json!({"hasUpstream":true,"pr":{"state":"open"}}),
                true,
                "Commit & push",
                "commit_push",
                true,
            ),
            (
                json!({"hasUpstream":true}),
                true,
                "Commit, push & PR",
                "commit_push_pr",
                true,
            ),
            (
                json!({"hasUpstream":true}),
                false,
                "Commit",
                "commit",
                false,
            ),
        ] {
            let mut local = local.clone();
            local["hasWorkingTreeChanges"] = json!(changes);
            assert_eq!(
                quick_action(&local, &remote, false),
                (label.into(), action, enabled)
            );
        }
    }
}
