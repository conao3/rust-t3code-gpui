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
    pub(super) fn git_action(
        &mut self,
        action: &GitAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action.kind.as_str() {
            "commit-dialog" => self.open_commit_dialog(window, cx),
            "commit" => self.run_git_stack("commit", None, false, cx),
            "publish" => self.open_publish_dialog(window, cx),
            "script" => self.run_project_script(action.value.clone(), window, cx),
            "refresh" | "pull" => self.run_git_request(&action.kind, cx),
            "push" | "commit_push" | "commit_push_pr" | "create_pr" => {
                let action = action.kind.clone();
                let on_default =
                    self.workspace_status.local["isDefaultRef"].as_bool() == Some(true);
                if on_default {
                    let weak = cx.entity().downgrade();
                    window.open_dialog(cx, move |dialog, _, _| {
                        let action = action.clone();
                        let weak = weak.clone();
                        dialog
                            .title("Continue on default branch?")
                            .child("This action publishes changes from the default branch.")
                            .on_ok(move |_, _, cx| {
                                let _ = weak.update(cx, |root, cx| {
                                    root.run_git_stack(&action, None, false, cx)
                                });
                                true
                            })
                    });
                } else {
                    self.run_git_stack(&action, None, false, cx);
                }
            }
            _ => {}
        }
    }

    fn open_commit_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Commit message (optional)"));
        let weak = cx.entity().downgrade();
        let changes = self.workspace_status.local["workingTree"].clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let submit = input.clone();
            let weak = weak.clone();
            dialog
                .title("Commit changes")
                .accessibility_label("Commit changes")
                .width(px(560.))
                .child(div().child(format!(
                    "+{}  -{}",
                    changes["insertions"], changes["deletions"]
                )))
                .child(Input::new(&input))
                .on_ok(move |_, _, cx| {
                    let message = submit.read(cx).value().trim().to_string();
                    let _ = weak.update(cx, |root, cx| {
                        root.run_git_stack(
                            "commit",
                            (!message.is_empty()).then_some(message),
                            false,
                            cx,
                        )
                    });
                    true
                })
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
            dialog.title("Publish repository")
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
        self.workspace_revision += 1;
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
                root.workspace_revision += 1;
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
        self.workspace_revision += 1;
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
                        root.workspace_revision += 1;
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
                root.workspace_revision += 1;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn current_project(&self) -> Option<(String, Value)> {
        if let Some((environment, id)) = self.draft_project().filter(|_| self.draft.is_some()) {
            return Some((environment.clone(), self.project_value(&environment, &id)?));
        }
        let (environment, id) = self.current()?;
        let thread = self.thread_value(&environment, &id)?;
        Some((
            environment.clone(),
            self.project_value(&environment, thread["projectId"].as_str()?)?,
        ))
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
                    Err(error) => root.message = Some(format!("Script failed: {error:#}")),
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
            &self.workspace_status.local,
            &self.workspace_status.remote,
            busy,
        );
        let scripts = self
            .current_project()
            .and_then(|(_, p)| p["scripts"].as_array().cloned())
            .unwrap_or_default();
        let script_focus = focus.clone();
        let primary_script = scripts.first().cloned();
        let local = self.workspace_status.local.clone();
        let remote = self.workspace_status.remote.clone();
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
                    .child(
                        Button::new("project-script-actions")
                            .icon(IconName::ChevronDown)
                            .accessibility_label("Script actions")
                            .ghost()
                            .compact()
                            .disabled(scripts.is_empty())
                            .dropdown_menu(move |mut menu, _, _| {
                                for script in &scripts {
                                    menu = menu.menu(
                                        script["name"].as_str().unwrap_or_default().to_string(),
                                        Box::new(GitAction {
                                            kind: "script".into(),
                                            value: script.clone(),
                                        }),
                                    );
                                }
                                menu.action_context(script_focus.clone())
                            }),
                    ),
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
