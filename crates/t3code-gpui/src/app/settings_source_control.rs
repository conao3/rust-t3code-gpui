use super::*;
use gpui_kit::component::switch::Switch;

pub(super) struct Discovery {
    root: WeakEntity<Root>,
    environment: String,
    result: Value,
    error: Option<String>,
    loading: bool,
    expanded: std::collections::HashSet<String>,
}

impl Discovery {
    pub(super) fn new(root: WeakEntity<Root>, environment: String, cx: &mut Context<Self>) -> Self {
        let view = cx.entity().downgrade();
        cx.defer(move |cx| {
            let _ = view.update(cx, |view, cx| view.refresh(cx));
        });
        Self {
            root,
            environment,
            result: Value::Null,
            error: None,
            loading: false,
            expanded: Default::default(),
        }
    }

    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        settings_controls::request(
            &self.root,
            &self.environment,
            "server.discoverSourceControl",
            json!({}),
            cx,
            |view, result, _| {
                view.loading = false;
                match result {
                    Ok(result) => view.result = result,
                    Err(error) => view.error = Some(format!("{error:#}")),
                }
            },
        );
        cx.notify();
    }
}

fn option(value: &Value) -> Option<&str> {
    if value["_tag"] == "Some" {
        value["value"].as_str()
    } else {
        None
    }
}

impl Render for Discovery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut content = v_flex().id("source-control-discovery").gap_8();
        for (key, title) in [
            ("versionControlSystems", "Version Control"),
            ("sourceControlProviders", "Source control providers"),
        ] {
            let heading = h_flex()
                .gap_2()
                .px_4()
                .child(settings_controls::text(title).font_weight(FontWeight::SEMIBOLD))
                .child(div().flex_1())
                .when(key == "versionControlSystems", |heading| {
                    heading.child(
                        Button::new("rescan")
                            .accessibility_label("Rescan server environment")
                            .icon(gpui_kit::assets::IconName::RefreshCw)
                            .ghost()
                            .compact()
                            .disabled(self.loading)
                            .on_click(cx.listener(|view, _, _, cx| view.refresh(cx))),
                    )
                });
            let mut rows = v_flex()
                .id(key)
                .rounded_lg()
                .overflow_hidden()
                .border_1()
                .border_color(cx.theme().border);
            for item in self.result[key].as_array().cloned().unwrap_or_default() {
                let label = item["label"].as_str().expect("discovery label");
                let kind = item["kind"].as_str().expect("discovery kind");
                let provider = item.get("auth").is_some();
                let not_ready = !provider && item["implemented"] == false;
                let available = item["status"] == "available";
                let authenticated = item["auth"]["status"] == "authenticated";
                let mut name = h_flex()
                    .gap_2()
                    .child(settings_controls::text(label).text_sm());
                if let Some(version) = option(&item["version"]) {
                    name = name.child(
                        settings_controls::text(version)
                            .text_xs()
                            .font_family(crate::appearance::default_code_family(cx))
                            .text_color(cx.theme().muted_foreground),
                    );
                }
                if not_ready {
                    name = name.child(
                        settings_controls::text("Coming Soon")
                            .text_xs()
                            .text_color(cx.theme().warning),
                    );
                }
                if provider && item["auth"]["status"] == "unauthenticated" {
                    name = name.child(settings_controls::text("Not authenticated").text_xs());
                }
                let mut description = h_flex()
                    .flex_wrap()
                    .gap_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground);
                if not_ready {
                    description = description.children(
                        ["Support for", label, "is coming soon."].map(settings_controls::text),
                    );
                } else if !available {
                    description = description.children(
                        [
                            "Not available on this server:",
                            item["installHint"].as_str().unwrap_or_default(),
                        ]
                        .map(settings_controls::text),
                    );
                } else if authenticated {
                    description = description.child(settings_controls::text("Authenticated"));
                    if let Some(detail) = option(&item["auth"]["detail"]) {
                        description =
                            description.child(settings_controls::text(format!("· {detail}")));
                    }
                } else if provider && item["auth"]["status"] == "unauthenticated" {
                    if let Some(executable) = item["executable"].as_str() {
                        description = description.children([label, "is not authenticated on this server. Sign in or configure credentials using the", executable, "tool on the server host to enable change request features."].map(settings_controls::text));
                    } else {
                        description = description.children(
                            [
                                "Available.",
                                item["installHint"].as_str().unwrap_or_default(),
                            ]
                            .map(settings_controls::text),
                        );
                    }
                } else if provider {
                    description = description.child(settings_controls::text(format!(
                        "Could not verify {label}. {}",
                        option(&item["auth"]["detail"])
                            .or_else(|| item["installHint"].as_str())
                            .unwrap_or_default()
                    )));
                } else {
                    description = description.child(settings_controls::text("Available"));
                }
                let row = h_flex()
                    .id(SharedString::from(kind.to_string()))
                    .px_4()
                    .py_3()
                    .gap_4()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(name)
                            .child(div().id("summary").child(description)),
                    )
                    .when(matches!(kind, "github" | "gitcafe" | "bitbucket"), |row| {
                        let kind = kind.to_string();
                        row.child(
                            Button::new("details")
                                .accessibility_label(format!("Toggle {label} details"))
                                .aria_expanded(self.expanded.contains(&kind))
                                .icon(gpui_kit::assets::IconName::ChevronDown)
                                .ghost()
                                .compact()
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    if !view.expanded.remove(&kind) {
                                        view.expanded.insert(kind.clone());
                                    }
                                    cx.notify();
                                })),
                        )
                    })
                    .when(!not_ready, |row| {
                        row.child(
                            Switch::new("available")
                                .accessibility_label(format!("{label} availability"))
                                .checked(available && (!provider || authenticated))
                                .disabled(true),
                        )
                    });
                let mut item_view = v_flex()
                    .id(SharedString::from(format!("host:{kind}")))
                    .child(row);
                if self.expanded.contains(kind) {
                    let kind = match kind {
                        "github" => "github",
                        "gitcafe" => "gitcafe",
                        "bitbucket" => "bitbucket",
                        _ => unreachable!(),
                    };
                    let root = self.root.clone();
                    let environment = self.environment.clone();
                    let discovery = cx.entity().downgrade();
                    let saved = self
                        .root
                        .upgrade()
                        .and_then(|root| root.read(cx).config_of(&environment))
                        .map(|config| config["settings"]["sourceControlHosts"][kind].clone())
                        .unwrap_or(Value::Null);
                    let credentials = window.use_keyed_state(
                        SharedString::from(format!(
                            "source-control-credentials:{environment}:{kind}"
                        )),
                        cx,
                        |window, cx| {
                            settings_source_credentials::Credentials::new(
                                root,
                                environment,
                                kind,
                                saved,
                                discovery,
                                window,
                                cx,
                            )
                        },
                    );
                    item_view = item_view.child(div().px_4().pb_4().child(credentials));
                }
                rows = rows.child(item_view);
            }
            content = content.child(v_flex().gap_3().child(heading).child(rows));
        }
        content.children(self.error.clone().map(|error| {
            div()
                .id("discovery-error")
                .role(accesskit::Role::Alert)
                .text_color(cx.theme().danger)
                .child(error)
        }))
    }
}
