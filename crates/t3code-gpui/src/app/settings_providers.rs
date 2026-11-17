use super::*;
use gpui_kit::component::{
    select::{Select, SelectEvent},
    switch::Switch,
};

pub(super) struct Providers {
    error: Option<String>,
    saving: bool,
    root: WeakEntity<Root>,
    environment: String,
    editing: Option<String>,
    selected: String,
    original: Value,
    inputs: [Entity<InputState>; 4],
    driver: Entity<settings_controls::Choices>,
    creating: bool,
    enabled: bool,
    subscriptions: Vec<Subscription>,
}
impl Providers {
    pub(super) fn new(
        root: WeakEntity<Root>,
        environment: String,
        owner: &Root,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let config = owner.config_of(&environment).unwrap_or(Value::Null);
        let mut drivers = config["providers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|provider| provider["driver"].as_str())
            .map(|driver| (driver.to_string(), driver.to_string()))
            .collect::<Vec<_>>();
        drivers.sort();
        drivers.dedup();
        let driver = settings_controls::select(drivers, "codex", window, cx);
        let inputs = ["Instance ID", "Display name", "Binary path", "Home path"]
            .map(|label| cx.new(|cx| InputState::new(window, cx).placeholder(label)));
        let subscriptions = vec![cx.subscribe(&driver, |_, _, _: &SelectEvent<_>, cx| cx.notify())];
        Self {
            error: None,
            saving: false,
            root,
            environment,
            editing: None,
            selected: "codex".into(),
            original: Value::Null,
            inputs,
            driver,
            creating: false,
            enabled: true,
            subscriptions,
        }
    }
    fn request(&self, method: &'static str, payload: Value, cx: &mut Context<Self>) {
        let _ = self.root.update(cx, |root, cx| {
            root.settings_request(&self.environment, method, payload, cx)
        });
    }
    fn remove(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let owner = cx.entity().downgrade();
        window.open_alert_dialog(cx,move |dialog,_,_| {
            let owner=owner.clone();let id=id.clone();
            dialog.title("Remove provider instance?").description("Threads that use this instance will keep their history.").confirm().ok_text("Remove")
                .on_ok(move |_,_,cx| {let _=owner.update(cx,|view,cx|view.request("server.updateSettings",json!({"patch":{},"providerInstanceMutation":{"operation":"remove","instanceId":id}}),cx));true})
        });
    }
    fn edit(&mut self, id: String, instance: Value, window: &mut Window, cx: &mut Context<Self>) {
        self.creating = id.is_empty();
        let values = [
            id.clone(),
            instance["displayName"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            instance["config"]["binaryPath"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            instance["config"]["homePath"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        ];
        for (input, value) in self.inputs.iter().zip(values) {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        if let Some(driver) = instance["driver"].as_str() {
            self.driver.update(cx, |state, cx| {
                state.set_selected_value(&driver.to_string(), window, cx)
            });
        }
        self.enabled = instance["enabled"].as_bool().unwrap_or(true);
        self.original = instance;
        self.editing = Some(id);
        cx.notify();
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let [id, name, binary, home] = self
            .inputs
            .clone()
            .map(|input| input.read(cx).value().trim().to_string());
        let Some(driver) = self.driver.read(cx).selected_value().cloned() else {
            return;
        };
        let mut instance = if self.original.is_object() {
            self.original.clone()
        } else {
            json!({})
        };
        instance["driver"] = json!(driver);
        instance["enabled"] = json!(self.enabled);
        if name.is_empty() {
            instance
                .as_object_mut()
                .expect("instance")
                .remove("displayName");
        } else {
            instance["displayName"] = json!(name);
        }
        if !instance["config"].is_object() {
            instance["config"] = json!({});
        }
        for (key, value) in [("binaryPath", binary), ("homePath", home)] {
            if value.is_empty() {
                instance["config"]
                    .as_object_mut()
                    .expect("config")
                    .remove(key);
            } else {
                instance["config"][key] = json!(value);
            }
        }
        self.saving = true;
        settings_controls::request(
            &self.root,
            &self.environment,
            "server.updateSettings",
            json!({"patch":{},"providerInstanceMutation":{"operation":if self.creating {"create"} else {"upsert"},"instanceId":id,"instance":instance}}),
            cx,
            |view, result, _| {
                view.saving = false;
                match result {
                    Ok(_) => {
                        view.editing = None;
                        view.error = None;
                    }
                    Err(error) => view.error = Some(format!("Unable to save provider: {error:#}")),
                }
            },
        );
        cx.notify();
    }
}
impl Render for Providers {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = &self.subscriptions;
        let config = self
            .root
            .upgrade()
            .and_then(|root| root.read(cx).config_of(&self.environment))
            .unwrap_or(Value::Null);
        let mut instances = config["settings"]["providerInstances"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        for provider in config["providers"].as_array().into_iter().flatten() {
            if let Some(id) = provider["instanceId"].as_str() {
                instances.entry(id.to_string()).or_insert_with(
                    || json!({"driver":provider["driver"],"enabled":provider["enabled"]}),
                );
            }
        }
        let order = [
            "codex",
            "claudeAgent",
            "cursor",
            "grok",
            "fx",
            "opencode",
            "antigravity",
            "kiro",
            "muse",
            "pi",
            "acpRegistry",
        ];
        let mut ordered = instances.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|(id, instance)| {
            (
                order
                    .iter()
                    .position(|driver| instance["driver"] == *driver)
                    .unwrap_or(order.len()),
                (*id).clone(),
            )
        });
        let selected = instances
            .get(&self.selected)
            .map(|instance| (self.selected.clone(), instance.clone()))
            .or_else(|| {
                ordered
                    .first()
                    .map(|(id, instance)| ((*id).clone(), (*instance).clone()))
            });
        let mut details = v_flex().flex_1().min_w_0().gap_6().py_4();
        if let Some((id, instance)) = &selected {
            let provider = config["providers"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|provider| provider["instanceId"] == *id);
            let name = instance_name(id, instance);
            details = details
                .child(
                    settings_controls::text(name)
                        .px_4()
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .when(
                    instance["driver"].as_str() != Some(id.as_str()),
                    |details| {
                        let id = id.clone();
                        details.child(
                            Button::new("delete-instance")
                                .icon(gpui_kit::assets::IconName::Trash)
                                .accessibility_label(format!("Delete instance {id}"))
                                .ghost()
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.remove(id.clone(), window, cx)
                                })),
                        )
                    },
                )
                .child(
                    v_flex()
                        .px_4()
                        .gap_1()
                        .child(settings_controls::text("Status"))
                        .child(settings_controls::text(provider_summary(provider)).text_xs()),
                );
            let mut fields = vec![("displayName", "Display name", "")];
            if instance["driver"] == "codex" {
                fields.extend([
                    ("config/binaryPath", "Binary path", "Path to the Codex binary used by this instance."),
                    ("config/homePath", "CODEX_HOME path", "Custom Codex home and config directory."),
                    ("config/shadowHomePath", "Shadow home path", "Account-specific Codex home. Keeps auth.json separate while sharing state from CODEX_HOME."),
                    ("config/launchArgs", "Launch arguments", "Additional CLI arguments passed to codex app-server on session start."),
                ]);
            }
            for (path, label, description) in fields {
                if path == "config/binaryPath" {
                    details = details.child(
                        settings_controls::text("Runtime")
                            .px_4()
                            .font_weight(FontWeight::SEMIBOLD),
                    );
                }
                let value = instance
                    .pointer(&format!("/{path}"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let root = self.root.clone();
                let environment = self.environment.clone();
                let id = id.clone();
                let accessible_name = if path == "displayName" {
                    instance_name(&id, &json!({"driver":instance["driver"]}))
                } else {
                    label.to_string()
                };
                let field = window.use_keyed_state(
                    SharedString::from(format!("provider-field:{environment}:{id}:{path}:{value}")),
                    cx,
                    |window, cx| {
                        ProviderField::new(
                            root,
                            environment,
                            id,
                            path,
                            label,
                            accessible_name,
                            description,
                            value,
                            window,
                            cx,
                        )
                    },
                );
                details = details.child(field);
            }
            if instance["driver"] != "codex" {
                let id = id.clone();
                let instance = instance.clone();
                details = details.child(
                    Button::new("edit-runtime")
                        .label("Runtime")
                        .outline()
                        .mx_4()
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.edit(id.clone(), instance.clone(), window, cx)
                        })),
                );
            }
        }
        v_flex().gap_3().when_some(self.error.clone(),|view,error|view.child(div().id("provider-save-error").role(accesskit::Role::Alert).text_color(cx.theme().danger).child(error)))
            .child(h_flex().gap_2().child(Button::new("add-provider").label("Add provider").outline().on_click(cx.listener(|view,_,window,cx|view.edit(String::new(),json!({"driver":"codex"}),window,cx)))).child(Button::new("refresh-providers").label("Refresh provider status").ghost().on_click(cx.listener(|view,_,_,cx|view.request("server.refreshProviders",json!({}),cx)))))
            .when(self.editing.is_some(),|view|view.child(v_flex().gap_2().p_3().border_1().border_color(cx.theme().border)
                .child(Select::new(&self.driver).accessibility_label("Provider").disabled(!self.creating))
                .children(self.inputs.iter().zip(["Instance ID","Display name","Binary path","Home path"]).map(|(input,label)|Input::new(input).aria_label(label).disabled(label=="Instance ID"&&!self.creating)))
                .child(Switch::new("provider-enabled").accessibility_label("Enabled").label("Enabled").checked(self.enabled).on_change(cx.listener(|view,checked,_,cx| {view.enabled = *checked;cx.notify();})))
                .child(h_flex().gap_2().when(!self.creating, |row| row.child(Button::new("remove-provider").label("Remove").ghost().on_click(cx.listener(|view, _, window, cx| { if let Some(id) = view.editing.clone() { view.remove(id, window, cx); } })))).child(Button::new("save-provider").label("Save").disabled(self.saving).primary().on_click(cx.listener(|view,_,_,cx|view.save(cx)))).child(Button::new("cancel-provider").label("Cancel").ghost().on_click(cx.listener(|view,_,_,cx| {view.editing=None;cx.notify();}))))))
            .child(h_flex().items_start().border_1().border_color(cx.theme().border).rounded(px(12.)).overflow_hidden()
                .child(v_flex().w(px(272.)).flex_none().children(ordered.into_iter().enumerate().map(|(index, (id, instance))| {
                    let name = instance_name(id, instance);
                    let select_id = id.clone(); let enable_id = id.clone(); let enable_instance = instance.clone();
                    let provider = config["providers"].as_array().into_iter().flatten().find(|provider| provider["instanceId"] == *id);
                    let status = provider_summary(provider);
                    let version = provider.and_then(|provider| provider["version"].as_str()).map(|version| if version.starts_with(|ch: char| ch.is_ascii_digit()) { format!("v{version}") } else { version.to_string() });
                    h_flex().id(("provider", index)).gap_3().px_4().py_3().border_b_1().border_color(cx.theme().border)
                        .when(self.selected == *id, |view| view.bg(cx.theme().accent))
                        .when(order.contains(&instance["driver"].as_str().unwrap_or_default()), |row| row.child(gpui_kit::component::Icon::default().path(SharedString::from(format!("icons/providers/{}-{}.svg", instance["driver"].as_str().unwrap_or_default(), if !cx.theme().mode.is_dark() { "light" } else { "dark" }))).size(px(16.))))
                        .child(Button::new(("select-provider", index)).accessibility_label(format!("Select {name}")).ghost().flex_1().min_w_0().justify_start().h_auto().child(v_flex().w_full().items_start().child(h_flex().gap_1().child(settings_controls::text(name.clone()).text_sm()).when_some(version, |view, version| view.child(settings_controls::text(version).text_xs().text_color(cx.theme().muted_foreground)))).child(settings_controls::text(status).text_xs().text_color(cx.theme().muted_foreground)))
                            .on_click(cx.listener(move |view, _, _, cx| { view.selected = select_id.clone(); cx.notify(); })))
                        .child(Switch::new(("enabled", index)).accessibility_label(format!("Enable {name}")).checked(instance["enabled"].as_bool().unwrap_or(true)).on_change(cx.listener(move |view, checked, _, cx| {
                            let mut instance = enable_instance.clone(); instance["enabled"] = json!(*checked);
                            view.request("server.updateSettings", json!({"patch":{},"providerInstanceMutation":{"operation":"upsert","instanceId":enable_id,"instance":instance}}), cx);
                        })))
                })))
                .child(details))
    }
}

fn instance_name(id: &str, instance: &Value) -> String {
    crate::provider_display::display_name(
        &json!({"driver": instance["driver"], "instanceId":id, "displayName":instance["displayName"]}),
    )
}

fn provider_summary(provider: Option<&Value>) -> String {
    let Some(provider) = provider else {
        return "Checking provider status".into();
    };
    if provider["enabled"] == false || provider["status"] == "disabled" {
        return "Disabled".into();
    }
    if provider["installed"] == false {
        return "Not found".into();
    }
    if provider["status"] == "warning" {
        return "Needs attention".into();
    }
    if provider["status"] == "error" {
        return "Unavailable".into();
    }
    let auth = provider["auth"]["status"].as_str();
    if matches!(auth, Some("unauthenticated" | "authenticated")) {
        let status = if auth == Some("authenticated") {
            "Authenticated"
        } else {
            "Not authenticated"
        };
        return provider["auth"]["label"]
            .as_str()
            .or_else(|| provider["auth"]["type"].as_str())
            .map(|label| format!("{status} · {label}"))
            .unwrap_or_else(|| status.into());
    }
    match provider["status"].as_str() {
        Some("warning") => "Needs attention",
        Some("error") => "Unavailable",
        _ => "Available",
    }
    .into()
}

struct ProviderField {
    input: Entity<InputState>,
    label: &'static str,
    accessible_name: String,
    description: &'static str,
    _subscription: Subscription,
}

impl ProviderField {
    #[allow(clippy::too_many_arguments)]
    fn new(
        root: WeakEntity<Root>,
        environment: String,
        id: String,
        path: &'static str,
        label: &'static str,
        accessible_name: String,
        description: &'static str,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(if path == "displayName" {
                    accessible_name.clone()
                } else {
                    String::new()
                })
                .default_value(value)
        });
        let subscription = cx.subscribe(&input, move |_, input, event, cx| {
            if !matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) { return; }
            let text = input.read(cx).value().trim().to_string();
            let _ = root.update(cx, |root, cx| {
                let config = root.config_of(&environment).unwrap_or(Value::Null);
                let instance = config["settings"]["providerInstances"].get(&id).cloned().or_else(|| {
                    config["providers"].as_array()?.iter().find(|provider| provider["instanceId"] == id)
                        .map(|provider| json!({"driver":provider["driver"],"enabled":provider["enabled"]}))
                });
                let Some(instance) = instance else {
                    root.message = Some(format!("Provider instance {id} is no longer available"));
                    cx.notify();
                    return;
                };
                let instance = update_field(instance, path, &text);
                root.settings_request(&environment, "server.updateSettings", json!({"patch":{},"providerInstanceMutation":{"operation":"upsert","instanceId":id,"instance":instance}}), cx);
            });
        });
        Self {
            input,
            label,
            accessible_name,
            description,
            _subscription: subscription,
        }
    }
}

fn update_field(mut instance: Value, path: &str, value: &str) -> Value {
    let (object, key) = if let Some(key) = path.strip_prefix("config/") {
        if !instance["config"].is_object() {
            instance["config"] = json!({});
        }
        (
            instance["config"].as_object_mut().expect("provider config"),
            key,
        )
    } else {
        (instance.as_object_mut().expect("provider instance"), path)
    };
    if value.is_empty() && key != "launchArgs" {
        object.remove(key);
    } else {
        object.insert(key.into(), json!(value));
    }
    instance
}

impl Render for ProviderField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .px_4()
            .gap_4()
            .items_start()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(settings_controls::text(self.label).text_sm())
                    .when(!self.description.is_empty(), |view| {
                        view.child(
                            settings_controls::text(self.description)
                                .text_xs()
                                .text_color(cx.theme().muted_foreground),
                        )
                    }),
            )
            .child(
                Input::new(&self.input)
                    .aria_label(self.accessible_name.clone())
                    .w(px(224.))
                    .flex_none(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::update_field;
    use serde_json::json;
    #[test]
    fn editing_runtime_preserves_provider_auth_and_other_fields() {
        let instance = json!({"driver":"codex","enabled":false,"environment":[{"name":"KEY","value":"fixture","sensitive":true}],"config":{"homePath":"/fixture","launchArgs":"--foo"}});
        let next = update_field(instance.clone(), "config/homePath", "");
        assert!(next["config"].get("homePath").is_none());
        assert_eq!(next["environment"], instance["environment"]);
        assert_eq!(next["enabled"], false);
        assert_eq!(
            update_field(next, "config/launchArgs", "")["config"]["launchArgs"],
            ""
        );
    }
}
