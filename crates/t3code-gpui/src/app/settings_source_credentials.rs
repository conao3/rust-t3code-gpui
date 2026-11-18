use super::*;
use gpui_kit::component::input::InputContentType;

pub(super) struct Credentials {
    root: WeakEntity<Root>,
    environment: String,
    kind: &'static str,
    fields: Vec<(
        &'static str,
        &'static str,
        &'static str,
        bool,
        Entity<InputState>,
    )>,
    subscriptions: Vec<Subscription>,
    saving: bool,
    error: Option<String>,
    dirty: std::collections::HashSet<usize>,
    discovery: WeakEntity<settings_source_control::Discovery>,
    clear_inputs: Vec<usize>,
    configured: std::collections::HashMap<usize, bool>,
}

impl Credentials {
    pub(super) fn new(
        root: WeakEntity<Root>,
        environment: String,
        kind: &'static str,
        saved: Value,
        discovery: WeakEntity<settings_source_control::Discovery>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let definitions = match kind {
            "github" => vec![
                ("host", "Host", "github.com", false),
                ("token", "GitHub token", "", true),
            ],
            "gitcafe" => vec![(
                "token",
                "Token",
                "A git.cafe token, used before CAFE_TOKEN and the cafe login, so GitCafe works without the CLI signed in.",
                true,
            )],
            "bitbucket" => vec![
                (
                    "accessToken",
                    "Access token",
                    "Scoped to one repository, project, or workspace. Create it in that item's Bitbucket settings.",
                    true,
                ),
                (
                    "email",
                    "Atlassian account email",
                    "With an API token instead of an access token.",
                    false,
                ),
                (
                    "apiToken",
                    "API token",
                    "Reaches every repository your Atlassian account can. Give it read and write access to repositories and pull requests, and read:user:bitbucket.",
                    true,
                ),
            ],
            _ => unreachable!("supported source control settings"),
        };
        let fields = definitions
            .into_iter()
            .map(|(key, label, description, secret)| {
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .default_value(if key == "host" {
                            "github.com"
                        } else if secret {
                            ""
                        } else {
                            saved[key].as_str().unwrap_or_default()
                        })
                        .masked(secret)
                });
                (key, label, description, secret, input)
            })
            .collect::<Vec<_>>();
        let mut view = Self {
            root,
            environment,
            kind,
            fields,
            subscriptions: vec![],
            saving: false,
            error: None,
            dirty: Default::default(),
            discovery,
            clear_inputs: vec![],
            configured: Default::default(),
        };
        for index in 0..view.fields.len() {
            view.subscriptions.push(cx.subscribe(
                &view.fields[index].4,
                move |view, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        if view.kind == "github" && index == 0 {
                            view.clear_inputs.push(1);
                            view.dirty.remove(&1);
                        }
                        view.dirty.insert(index);
                        cx.notify();
                    }
                    if view.kind != "github"
                        && matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. })
                        && view.dirty.contains(&index)
                    {
                        if view.fields[index].3 && view.fields[index].4.read(cx).value().is_empty()
                        {
                            view.dirty.remove(&index);
                            return;
                        }
                        view.save(Some(index), false, cx);
                    }
                },
            ));
        }
        view
    }

    fn saved(&self, cx: &App) -> Value {
        self.root
            .upgrade()
            .and_then(|root| root.read(cx).config_of(&self.environment))
            .map(|config| config["settings"]["sourceControlHosts"][self.kind].clone())
            .unwrap_or(Value::Null)
    }

    fn save(&mut self, field: Option<usize>, clear: bool, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let fields = if self.kind == "github" {
            let host = self.fields[0].4.read(cx).value().trim().to_lowercase();
            if host.is_empty() {
                return;
            }
            let token = if clear {
                String::new()
            } else {
                self.fields[1].4.read(cx).value().trim().to_string()
            };
            if !clear && token.is_empty() {
                return;
            }
            let mut tokens = self.saved(cx)["tokens"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            tokens.insert(host, json!(token));
            json!({"tokens": tokens})
        } else {
            let mut values = serde_json::Map::new();
            for (index, (key, _, _, _, input)) in self.fields.iter().enumerate() {
                if field.is_none_or(|selected| selected == index) {
                    values.insert(
                        (*key).into(),
                        json!(if clear {
                            String::new()
                        } else {
                            input.read(cx).value().trim().to_string()
                        }),
                    );
                }
            }
            json!(values)
        };
        self.saving = true;
        self.error = None;
        settings_controls::request(
            &self.root,
            &self.environment,
            "server.updateSettings",
            json!({"patch":{"sourceControlHosts":{self.kind:fields}}}),
            cx,
            move |view, result, cx| {
                view.saving = false;
                match result {
                    Ok(_) => {
                        view.clear_inputs
                            .extend(view.fields.iter().enumerate().filter_map(
                                |(index, (_, _, _, secret, _))| {
                                    (field.is_none_or(|selected| selected == index)
                                        && (clear || *secret)
                                        && (view.kind != "github" || index == 1))
                                        .then_some(index)
                                },
                            ));
                        if let Some(index) = field {
                            view.dirty.remove(&index);
                        } else {
                            view.dirty.clear();
                        }
                        let _ = view
                            .discovery
                            .update(cx, |discovery, cx| discovery.refresh(cx));
                    }
                    Err(error) => view.error = Some(format!("{error:#}")),
                }
            },
        );
        cx.notify();
    }
}

impl Render for Credentials {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for index in self.clear_inputs.drain(..) {
            self.fields[index]
                .4
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        let saved = self.saved(cx);
        let github = self.kind == "github";
        let mut panel = v_flex().id(self.kind).gap_3();
        if github {
            let host = self.fields[0].4.read(cx).value().trim().to_lowercase();
            panel = panel.child(settings_controls::text("A token saved here is used before GH_TOKEN and the gh login, so GitHub works without the GitHub CLI. Give it read and write access to pull requests and contents.").text_xs().text_color(cx.theme().muted_foreground))
                .child(Button::new("create-token").label("Create a token").link().on_click(move |_, _, cx| cx.open_url(&format!("https://{host}/settings/personal-access-tokens/new"))));
        }
        for (index, (key, label, description, secret, input)) in self.fields.iter().enumerate() {
            let stored = if github && *key == "token" {
                &saved["tokens"][self.fields[0].4.read(cx).value().trim().to_lowercase()]
            } else {
                &saved[*key]
            };
            let configured =
                stored.is_object() || stored.as_str().is_some_and(|value| !value.is_empty());
            if *secret {
                let placeholder = if configured {
                    "Stored secret, enter a new value to replace"
                } else {
                    "Not set"
                };
                if self.configured.insert(index, configured) != Some(configured) {
                    input.update(cx, |input, cx| {
                        input.set_placeholder(placeholder, window, cx)
                    });
                }
            }
            panel = panel.child(
                v_flex()
                    .id(*key)
                    .gap_1()
                    .child(
                        settings_controls::text(if github && *key == "token" {
                            "Token"
                        } else {
                            label
                        })
                        .text_sm(),
                    )
                    .when(!github, |row| {
                        row.child(
                            settings_controls::text(*description)
                                .text_xs()
                                .text_color(cx.theme().muted_foreground),
                        )
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Input::new(input)
                                    .aria_label(*label)
                                    .when(*secret, |input| {
                                        input.content_type(InputContentType::Password)
                                    })
                                    .disabled(self.saving)
                                    .flex_1(),
                            )
                            .when(!github && configured, |row| {
                                row.child(
                                    Button::new("clear")
                                        .icon(gpui_kit::assets::IconName::X)
                                        .accessibility_label(format!("Clear {label}"))
                                        .ghost()
                                        .disabled(self.saving)
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.save(Some(index), true, cx)
                                        })),
                                )
                            }),
                    ),
            );
        }
        if github {
            let host = self.fields[0].4.read(cx).value().trim().to_lowercase();
            let hosts = saved["tokens"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(_, token)| {
                    token.is_object() || token.as_str().is_some_and(|token| !token.is_empty())
                })
                .map(|(host, _)| host.as_str())
                .collect::<Vec<_>>();
            panel = panel.child(
                h_flex()
                    .gap_2()
                    .child(
                        settings_controls::text(if hosts.is_empty() {
                            "No token saved; the server uses GH_TOKEN or the gh login.".into()
                        } else {
                            format!("Saved for {}.", hosts.join(", "))
                        })
                        .text_xs()
                        .flex_1(),
                    )
                    .when(hosts.contains(&host.as_str()), |row| {
                        row.child(
                            Button::new("remove")
                                .label("Remove")
                                .outline()
                                .disabled(self.saving)
                                .on_click(cx.listener(|view, _, _, cx| view.save(None, true, cx))),
                        )
                    })
                    .child(
                        Button::new("save")
                            .label("Save")
                            .primary()
                            .disabled(
                                self.saving
                                    || self.dirty.is_empty()
                                    || host.is_empty()
                                    || self.fields[1].4.read(cx).value().trim().is_empty(),
                            )
                            .on_click(cx.listener(|view, _, _, cx| view.save(None, false, cx))),
                    ),
            );
        }
        if !github
            && saved.as_object().is_some_and(|fields| {
                fields.values().any(|value| {
                    value.is_object() || value.as_str().is_some_and(|value| !value.is_empty())
                })
            })
        {
            panel = panel.child(
                h_flex().justify_end().child(
                    Button::new("remove")
                        .label("Remove")
                        .outline()
                        .disabled(self.saving)
                        .on_click(cx.listener(|view, _, _, cx| view.save(None, true, cx))),
                ),
            );
        }
        panel.children(self.error.clone().map(|error| {
            div()
                .id("credential-error")
                .role(accesskit::Role::Alert)
                .text_color(cx.theme().danger)
                .child(error)
        }))
    }
}
