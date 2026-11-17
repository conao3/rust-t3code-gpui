use super::*;

impl Root {
    pub(super) fn is_scratch_draft(&self) -> bool {
        self.draft_project().is_some_and(|(environment, project)| {
            self.project_value(&environment, &project)
                .zip(self.config_of(&environment))
                .is_some_and(|(project, config)| {
                    project["workspaceRoot"]
                        .as_str()
                        .zip(config["scratchWorkspaceRoot"].as_str())
                        .is_some_and(|(path, scratch)| {
                            path.trim_end_matches('/') == scratch.trim_end_matches('/')
                        })
                })
        })
    }

    pub(super) fn can_start_without_project(&self) -> bool {
        self.connections.iter().any(|connection| {
            self.config_of(connection.label()).is_some_and(|config| {
                config["scratchWorkspaceRoot"]
                    .as_str()
                    .is_some_and(|root| !root.is_empty())
            })
        })
    }

    pub(super) fn new_without_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let available = |connection: &&Arc<Connection>| {
            self.config_of(connection.label()).is_some_and(|config| {
                config["scratchWorkspaceRoot"]
                    .as_str()
                    .is_some_and(|root| !root.is_empty())
            })
        };
        let current = self.composer_target().map(|(env, ..)| env);
        let connection = self
            .connections
            .first()
            .filter(available)
            .or_else(|| {
                self.connections
                    .iter()
                    .find(|c| Some(c.label()) == current.as_deref())
                    .filter(available)
            })
            .or_else(|| {
                let mut matching = self.connections.iter().filter(available);
                let one = matching.next()?;
                matching.next().is_none().then_some(one)
            })
            .cloned();
        let Some(connection) = connection else {
            return;
        };
        let environment = connection.label().to_string();
        let runtime = self.runtime.clone();
        self.spawn_pane_in(window, cx, async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                let result = connection
                    .request("projects.ensureScratch", json!({}))
                    .await?;
                let id = result["projectId"]
                    .as_str()
                    .context("Scratch project id missing")?
                    .to_string();
                for _ in 0..100 {
                    if connection
                        .shell()
                        .lock()
                        .expect("shell")
                        .projects
                        .iter()
                        .any(|p| p["id"].as_str() == Some(&id))
                    {
                        return Ok(id);
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                anyhow::bail!("Scratch project did not arrive in the shell")
            })
            .await;
            let _ = this.update_in(cx, |root, window, cx| {
                match result {
                    Ok(project_id) => {
                        let group = root
                            .project_groups()
                            .iter()
                            .find(|g| g.contains(&environment, &project_id))
                            .map(|g| g.key.clone());
                        root.new_thread(window, cx);
                        if let Some(draft) = root.conversation.draft.as_mut() {
                            draft.group = group;
                            draft.environment = Some(environment);
                        }
                    }
                    Err(error) => {
                        root.conversation.message =
                            Some(format!("Could not start without a project: {error:#}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
