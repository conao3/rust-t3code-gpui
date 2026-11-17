use super::*;

impl Root {
    pub(super) fn toggle_hosts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.usage_page = None;
        self.settings_open = true;
        self.settings_page = "connections";
        self.hosts_open = false;
        self.settings_focus.focus(window, cx);
        cx.notify();
    }

    fn connect_host(&mut self, environment: &Environment) {
        let connection = Connection::new(environment, self.change_sender.clone());
        self.host_tasks.insert(
            environment.label.clone(),
            vec![
                connection.spawn_shell(&self.runtime),
                connection.spawn_config(&self.runtime),
            ],
        );
        self.connections.push(connection);
    }

    /// Adds a host from the dialog: a pairing URL, or a server URL with a pairing code (exchanged
    /// for a token) or a token. The token is checked against the server before it is saved.
    fn add_host(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let [label, url, secret] = self
            .host_inputs
            .clone()
            .map(|input| input.read(cx).value().trim().to_string());
        if label.is_empty() || self.connection(&label).is_some() {
            self.message = Some("Give the host a name no other host has.".into());
            cx.notify();
            return;
        }
        let (origin, code, is_token) = match config::pairing_url(&url) {
            Some((origin, code)) => (origin, code, false),
            None if secret.is_empty() => {
                self.message = Some(
                    "Paste a pairing URL, or a server URL with a pairing code or token.".into(),
                );
                cx.notify();
                return;
            }
            None => {
                let is_token = secret.len() > 32;
                (url.trim_end_matches('/').to_string(), secret, is_token)
            }
        };
        let runtime = self.runtime.clone();
        cx.spawn_in(window, async move |this, cx| {
            let token = on_runtime(&runtime, {
                let origin = origin.clone();
                async move {
                    let token = if is_token {
                        code
                    } else {
                        Api::new(&origin, "").pair(&code, APP_NAME).await?
                    };
                    Api::new(&origin, &token).get("/api/auth/session").await?;
                    Ok(token)
                }
            })
            .await;
            let _ = this.update_in(cx, |root, window, cx| {
                match token {
                    Ok(token) => {
                        let environment = Environment {
                            label,
                            http_base_url: origin,
                            token,
                        };
                        let mut next = root.config.clone();
                        next.environments.push(environment.clone());
                        match next.save(&root.config_path) {
                            Ok(()) => {
                                root.config = next;
                                root.connect_host(&environment);
                                for input in root.host_inputs.clone() {
                                    input.update(cx, |state, cx| state.set_value("", window, cx));
                                }
                                root.message = None;
                            }
                            Err(error) => {
                                root.message = Some(format!("Saving the hosts failed: {error:#}"))
                            }
                        }
                    }
                    Err(error) => root.message = Some(format!("Adding the host failed: {error:#}")),
                }
                root.shell_dirty = true;
                root.refresh(window, cx);
            });
        })
        .detach();
    }

    fn remove_host(&mut self, label: String, window: &mut Window, cx: &mut Context<Self>) {
        let mut next = self.config.clone();
        next.environments.retain(|e| e.label != label);
        if let Err(error) = next.save(&self.config_path) {
            self.message = Some(format!("Saving the hosts failed: {error:#}"));
            cx.notify();
            return;
        }
        self.config = next;
        for task in self.host_tasks.remove(&label).unwrap_or_default() {
            task.abort();
        }
        self.connections.retain(|c| c.label() != label);
        if self
            .current()
            .is_some_and(|(environment, _)| environment == label)
        {
            self.close_thread(cx);
        }
        if self.host_filter.as_deref() == Some(label.as_str()) {
            self.host_filter = None;
        }
        self.shell_dirty = true;
        self.refresh(window, cx);
    }

    pub(super) fn render_host_content(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let rows = self
            .config
            .environments
            .iter()
            .enumerate()
            .map(|(i, environment)| {
                let label = environment.label.clone();
                let removable = self.config.environments.len() > 1;
                h_flex()
                    .gap_2()
                    .child(div().w(px(120.)).text_sm().child(environment.label.clone()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(environment.http_base_url.clone()),
                    )
                    .when(removable, |row| {
                        row.child(
                            Button::new(("remove-host", i))
                                .label("Remove")
                                .compact()
                                .outline()
                                .on_click(cx.listener(move |root, _, window, cx| {
                                    root.remove_host(label.clone(), window, cx)
                                })),
                        )
                    })
            });
        v_flex()
            .gap_3()
            .child(v_flex().gap_1().children(rows))
            .child(div().text_xs().text_color(theme.muted_foreground).child(
                "Add an environment with a pairing URL, or its server URL and pairing code.",
            ))
            .children(
                self.host_inputs
                    .iter()
                    .zip(["Environment name", "Server URL", "Pairing code or token"])
                    .map(|(input, label)| Input::new(input).aria_label(label)),
            )
            .child(
                Button::new("add-host")
                    .label("Add environment")
                    .primary()
                    .on_click(cx.listener(|root, _, window, cx| root.add_host(window, cx))),
            )
            .into_any_element()
    }

    pub(super) fn render_hosts(&self, cx: &Context<Self>) -> AnyElement {
        self.render_host_content(cx)
    }
}
