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
            let variables = window.use_keyed_state(
                SharedString::from(format!("provider-environment:{}:{id}", self.environment)),
                cx,
                |window, cx| {
                    ProviderEnvironment::new(
                        self.root.clone(),
                        self.environment.clone(),
                        id.clone(),
                        instance["environment"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default(),
                        window,
                        cx,
                    )
                },
            );
            details = details.child(variables);
            let models = window.use_keyed_state(
                SharedString::from(format!("provider-models:{}:{id}", self.environment)),
                cx,
                |window, cx| {
                    ProviderModels::new(
                        self.root.clone(),
                        self.environment.clone(),
                        id.clone(),
                        window,
                        cx,
                    )
                },
            );
            details = details.child(models);
        }
        v_flex().gap_3().when_some(self.error.clone(),|view,error|view.child(div().id("provider-save-error").role(accesskit::Role::Alert).text_color(cx.theme().danger).child(error)))
            .child(h_flex().gap_2().child(Button::new("add-provider").label("Add provider").outline().on_click(cx.listener(|view,_,window,cx|view.edit(String::new(),json!({"driver":"codex"}),window,cx)))).child(Button::new("refresh-providers").label("Refresh provider status").ghost().on_click(cx.listener(|view,_,_,cx|view.request("server.refreshProviders",json!({"refreshModels":true,"reloadSecrets":true}),cx)))))
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

struct ProviderModels {
    root: WeakEntity<Root>,
    environment: String,
    instance: String,
    adding: bool,
    input: Entity<InputState>,
    filter: Entity<InputState>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl ProviderModels {
    fn new(
        root: WeakEntity<Root>,
        environment: String,
        instance: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("model-slug"));
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter models"));
        let subscriptions = vec![
            cx.subscribe(&filter, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe_in(&input, window, |view, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    view.add(window, cx);
                }
            }),
        ];
        Self {
            root,
            environment,
            instance,
            adding: false,
            input,
            filter,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn config(&self, cx: &App) -> Value {
        self.root
            .upgrade()
            .and_then(|root| root.read(cx).config_of(&self.environment))
            .unwrap_or(Value::Null)
    }

    fn save(&mut self, payload: Value, cx: &mut Context<Self>) {
        settings_controls::request(
            &self.root,
            &self.environment,
            "server.updateSettings",
            payload,
            cx,
            |view, result, _| {
                view.error = result
                    .err()
                    .map(|error| format!("Model preferences not saved: {error:#}"));
            },
        );
    }

    fn preferences(&mut self, mut patch: Value, cx: &mut Context<Self>) {
        let config = self.config(cx);
        if config["settings"]["favorites"].is_null()
            || config["settings"]["providerModelPreferences"].is_null()
        {
            let root = self.root.upgrade().expect("settings owner");
            let settings = &root.read(cx).settings.values;
            patch["migrateModelPreferences"] = json!({"favorites":settings["favorites"].as_array().cloned().unwrap_or_default(),"providerModelPreferences":settings["providerModelPreferences"].as_object().cloned().unwrap_or_default()});
        }
        self.save(json!({"patch":patch}), cx);
    }

    fn save_custom(&mut self, models: Vec<Value>, cx: &mut Context<Self>) {
        let config = self.config(cx);
        let mut instance = config["settings"]["providerInstances"][&self.instance].clone();
        if !instance.is_object() {
            self.error = Some("Provider instance is no longer available".to_string());
            cx.notify();
            return;
        }
        if !instance["config"].is_object() {
            instance["config"] = json!({});
        }
        instance["config"]["customModels"] = json!(models);
        self.save(json!({"patch":{},"providerInstanceMutation":{"operation":"upsert","instanceId":self.instance,"instance":instance}}), cx);
    }

    fn remove_custom(&mut self, slug: &str, cx: &mut Context<Self>) {
        let config = self.config(cx);
        let custom =
            config["settings"]["providerInstances"][&self.instance]["config"]["customModels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|model| model["slug"] != slug && **model != slug)
                .cloned()
                .collect::<Vec<_>>();
        let mut instance = config["settings"]["providerInstances"][&self.instance].clone();
        instance["config"]["customModels"] = json!(custom);
        let root = self.root.upgrade().expect("settings owner");
        let local = &root.read(cx).settings.values;
        let preferences = if config["settings"]["providerModelPreferences"].is_null() {
            &local["providerModelPreferences"][&self.instance]
        } else {
            &config["settings"]["providerModelPreferences"][&self.instance]
        };
        let order = preferences["modelOrder"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|model| **model != slug)
            .cloned()
            .collect::<Vec<_>>();
        let mut patch = json!({"setModelFavorites":[{"provider":self.instance,"model":slug,"favorite":false}],"setProviderModelOrder":{"provider":self.instance,"modelOrder":order}});
        if config["settings"]["favorites"].is_null()
            || config["settings"]["providerModelPreferences"].is_null()
        {
            patch["migrateModelPreferences"] = json!({"favorites":local["favorites"].as_array().cloned().unwrap_or_default(),"providerModelPreferences":local["providerModelPreferences"].as_object().cloned().unwrap_or_default()});
        }
        self.save(json!({"patch":patch,"providerInstanceMutation":{"operation":"upsert","instanceId":self.instance,"instance":instance}}), cx);
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let slug = self.input.read(cx).value().trim().to_string();
        let config = self.config(cx);
        let mut custom =
            config["settings"]["providerInstances"][&self.instance]["config"]["customModels"]
                .as_array()
                .cloned()
                .unwrap_or_default();
        let built_in = config["providers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|provider| provider["instanceId"] == self.instance)
            .flat_map(|provider| provider["models"].as_array().into_iter().flatten())
            .any(|model| model["isCustom"] != true && model["slug"] == slug);
        self.error = if slug.is_empty() {
            Some("Enter a model slug.".into())
        } else if built_in {
            Some("That model is already built in.".into())
        } else if slug.encode_utf16().count() > 256 {
            Some("Model slugs must be 256 characters or less.".into())
        } else if custom
            .iter()
            .any(|model| model["slug"] == slug || *model == slug)
        {
            Some("That custom model is already saved.".into())
        } else {
            None
        };
        if self.error.is_none() {
            custom.push(json!(slug));
            self.save_custom(custom, cx);
            self.adding = false;
            self.input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.filter
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        cx.notify();
    }
}

impl Render for ProviderModels {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let config = self.config(cx);
        let instance = &config["settings"]["providerInstances"][&self.instance];
        let mut models = config["providers"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|provider| provider["instanceId"] == self.instance)
            .and_then(|provider| provider["models"].as_array())
            .into_iter()
            .flatten()
            .filter(|model| model["isCustom"] != true)
            .cloned()
            .collect::<Vec<_>>();
        for model in instance["config"]["customModels"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let slug = model
                .as_str()
                .or_else(|| model["slug"].as_str())
                .unwrap_or_default();
            if models.iter().any(|existing| existing["slug"] == slug) {
                continue;
            }
            models.push(
                json!({"slug":slug,"name":model["name"].as_str().unwrap_or(slug),"isCustom":true}),
            );
        }
        let root = self.root.upgrade().expect("settings owner");
        let local = &root.read(cx).settings.values;
        let preferences = if config["settings"]["providerModelPreferences"].is_null() {
            &local["providerModelPreferences"][&self.instance]
        } else {
            &config["settings"]["providerModelPreferences"][&self.instance]
        };
        let hidden = preferences["hiddenModels"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let order = preferences["modelOrder"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let favorites = if config["settings"]["favorites"].is_null() {
            &local["favorites"]
        } else {
            &config["settings"]["favorites"]
        };
        let favorite =
            |model: &Value| {
                favorites.as_array().into_iter().flatten().any(|entry| {
                    entry["provider"] == self.instance && entry["model"] == model["slug"]
                })
            };
        let group = |model: &Value| {
            if favorite(model) {
                0
            } else if model["isCustom"] != true && hidden.contains(&model["slug"]) {
                2
            } else {
                1
            }
        };
        models.sort_by_key(|model| {
            (
                group(model),
                order
                    .iter()
                    .position(|slug| *slug == model["slug"])
                    .unwrap_or(usize::MAX),
            )
        });
        let built_in = models
            .iter()
            .filter(|model| model["isCustom"] != true)
            .map(|model| model["slug"].clone())
            .collect::<Vec<_>>();
        let all_hidden = !built_in.is_empty() && built_in.iter().all(|slug| hidden.contains(slug));
        let query = self.filter.read(cx).value().trim().to_lowercase();
        let filtering = models.len() > 8 && !query.is_empty();
        let favorite_count = models.iter().filter(|model| favorite(model)).count();
        let hidden_count = built_in.iter().filter(|slug| hidden.contains(slug)).count();
        let rows = models.iter().enumerate().filter(|(_, model)| !filtering || format!("{} {}", model["name"], model["slug"]).to_lowercase().contains(&query)).map(|(index, model)| {
            let slug = model["slug"].as_str().unwrap_or_default().to_string();
            let name = model["name"].as_str().unwrap_or(&slug).to_string();
            let is_favorite = favorite(model);
            let is_hidden = model["isCustom"] != true && hidden.contains(&model["slug"]);
            let favorite_slug = slug.clone();
            let toggle_slug = slug.clone();
            let mut row = h_flex().id(SharedString::from(format!("provider-model:{slug}"))).h(px(28.)).gap_2().px_2()
                .child(Button::new("favorite").icon(gpui_kit::assets::IconName::Star).accessibility_label(format!("{} {name} {} favorites", if is_favorite { "Remove" } else { "Add" }, if is_favorite { "from" } else { "to" })).ghost().compact().selected(is_favorite).on_click(cx.listener(move |view, _, _, cx| view.preferences(json!({"setModelFavorites":[{"provider":view.instance,"model":favorite_slug,"favorite":!is_favorite}]}),cx))))
                .child(settings_controls::text(name.clone()).text_xs())
                .when(name != slug, |row| row.child(settings_controls::text(slug.clone()).text_xs().text_color(cx.theme().muted_foreground)))
                .child(div().flex_1());
            if !is_hidden && !filtering {
                for (up, target) in [(true, index.checked_sub(1)), (false, (index+1 < models.len()).then_some(index+1))] {
                    let can_move = target.is_some_and(|target| group(&models[target]) == group(model));
                    let mut next = models.iter().map(|model| model["slug"].clone()).collect::<Vec<_>>();
                    if let Some(target) = target { next.swap(index, target); }
                    row = row.child(Button::new(if up {"move-up"} else {"move-down"}).icon(if up {IconName::ArrowUp} else {IconName::ArrowDown}).accessibility_label(format!("Move {name} {}", if up {"up"} else {"down"})).ghost().compact().disabled(!can_move).on_click(cx.listener(move |view, _, _, cx| view.preferences(json!({"setProviderModelOrder":{"provider":view.instance,"modelOrder":next}}),cx))));
                }
            }
            if model["isCustom"] != true {
                row = row.child(Switch::new("visible").accessibility_label(format!("Show {name} in the model picker")).checked(!is_hidden).on_change(cx.listener(move |view, checked: &bool, _, cx| view.preferences(json!({"setModelsHidden":[{"provider":view.instance,"model":toggle_slug,"hidden":!*checked}]}),cx))));
            } else {
                row = row.child(settings_controls::text("custom").text_xs())
                    .child(Button::new("remove-custom-model").icon(gpui_kit::assets::IconName::X).accessibility_label(format!("Remove {slug}")).ghost().compact().on_click(cx.listener(move |view, _, _, cx| view.remove_custom(&slug, cx))))
                    .child(Switch::new("visible").accessibility_label(format!("Show {name} in the model picker")).checked(true).disabled(true));
            }
            let starts_group = index == 0 || group(&models[index-1]) != group(model);
            v_flex().when(starts_group && (favorite_count > 0 || is_hidden), |view| view.child(settings_controls::text(match group(model) {0=>"Favorites",2=>"Hidden from picker",_=>"All"}).text_xs().px_2().pt_3())).child(row)
        }).collect::<Vec<_>>();
        v_flex().gap_3().px_4()
            .child(settings_controls::text("Models").font_weight(FontWeight::SEMIBOLD))
            .child(settings_controls::text("Model preferences and custom models are saved on the selected environment and shared across connected devices.").text_xs().text_color(cx.theme().muted_foreground))
            .when(models.len()>8, |view| view.child(Input::new(&self.filter).aria_label("Filter models")))
            .child(h_flex().gap_2()
                .when(!built_in.is_empty(), |row| row.child(Button::new("bulk-models").label(if all_hidden {"Enable all"} else {"Disable all"}).ghost().compact().on_click(cx.listener(move |view, _, _, cx| view.preferences(json!({"setModelsHidden":built_in.iter().map(|slug| json!({"provider":view.instance,"model":slug,"hidden":!all_hidden})).collect::<Vec<_>>()}),cx)))))
                .child(settings_controls::text(models.len().to_string()).text_xs())
                .child(settings_controls::text(if models.len()==1 {"model"} else {"models"}).text_xs())
                .when(favorite_count>0, |row| row.child(settings_controls::text(format!("· {favorite_count} favorite{}",if favorite_count==1 {""} else {"s"})).text_xs()))
                .when(hidden_count>0, |row| row.child(settings_controls::text(format!("· {hidden_count} hidden")).text_xs()))
                .child(div().flex_1())
                .when(!self.adding && instance["driver"]!="antigravity", |row| row.child(Button::new("add-custom-model").label("Add custom model").ghost().compact().on_click(cx.listener(|view, _, window, cx| {view.adding=true;view.input.update(cx,|input,cx|input.focus(window,cx));cx.notify();})))))
            .when(rows.is_empty(), |view| view.child(settings_controls::text(if filtering {"No models match."} else {"No models reported for this provider yet."}).text_xs()))
            .children(rows)
            .when(self.adding, |view| view.child(h_flex().gap_2().child(Input::new(&self.input).aria_label("Custom model").flex_1()).child(Button::new("save-custom-model").label("Add").outline().on_click(cx.listener(|view,_,window,cx|view.add(window,cx)))).child(Button::new("cancel-custom-model").label("Cancel").ghost().on_click(cx.listener(|view,_,_,cx|{view.adding=false;view.error=None;cx.notify();})))))
            .when_some(self.error.clone(), |view,error| view.child(settings_controls::text(error).text_color(cx.theme().danger)))
    }
}

struct EnvironmentVariable {
    id: usize,
    value: Value,
    name_input: Entity<InputState>,
    value_input: Entity<InputState>,
    acknowledged: bool,
}

struct ProviderEnvironment {
    root: WeakEntity<Root>,
    environment: String,
    instance: String,
    rows: Vec<EnvironmentVariable>,
    next_id: usize,
    subscriptions: Vec<Subscription>,
    error: Option<String>,
}

impl ProviderEnvironment {
    fn new(
        root: WeakEntity<Root>,
        environment: String,
        instance: String,
        values: Vec<Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            root,
            environment,
            instance,
            rows: vec![],
            next_id: 0,
            subscriptions: vec![],
            error: None,
        };
        for value in values {
            view.add(value, window, cx);
        }
        view
    }

    fn add(&mut self, value: Value, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.next_id;
        self.next_id += 1;
        let name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("VARIABLE_NAME")
                .default_value(value["name"].as_str().unwrap_or_default())
        });
        let value_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(if value["valueRedacted"] == true {
                    "Configured"
                } else {
                    "value"
                })
                .default_value(value["value"].as_str().unwrap_or_default())
                .masked(value["sensitive"] == true)
        });
        for (input, key) in [(&name_input, "name"), (&value_input, "value")] {
            self.subscriptions
                .push(cx.subscribe(input, move |view, input, event, cx| {
                    if !matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                        return;
                    }
                    let Some(row) = view.rows.iter_mut().find(|row| row.id == id) else {
                        return;
                    };
                    let text = input.read(cx).unmask_value().to_string();
                    if key == "value" && row.value["valueRedacted"] == true && text.is_empty() {
                        return;
                    }
                    row.value[key] = json!(if key == "name" {
                        text.trim().to_string()
                    } else {
                        text
                    });
                    if key == "value" {
                        row.value["valueRedacted"] = json!(false);
                    }
                    view.publish(cx);
                }));
        }
        self.rows.push(EnvironmentVariable {
            id,
            value,
            name_input,
            value_input,
            acknowledged: false,
        });
        cx.notify();
    }

    fn publish(&mut self, cx: &mut Context<Self>) {
        let mut variables = vec![];
        for row in &self.rows {
            let name = row.value["name"].as_str().unwrap_or_default();
            let valid = !name.is_empty()
                && name.chars().enumerate().all(|(index, ch)| {
                    ch == '_' || ch.is_ascii_alphabetic() || index > 0 && ch.is_ascii_digit()
                });
            if !valid {
                if name.is_empty() && row.value["value"] == "" && row.value["sensitive"] == true {
                    continue;
                }
                self.error = Some("Enter a valid environment variable name.".into());
                cx.notify();
                return;
            }
            variables.push(row.value.clone());
        }
        let config = self
            .root
            .upgrade()
            .and_then(|root| root.read(cx).config_of(&self.environment))
            .unwrap_or(Value::Null);
        let mut instance = config["settings"]["providerInstances"][&self.instance].clone();
        if !instance.is_object() {
            self.error = Some("Provider instance is no longer available".into());
            cx.notify();
            return;
        }
        instance["environment"] = json!(variables);
        settings_controls::request(
            &self.root,
            &self.environment,
            "server.updateSettings",
            json!({"patch":{},"providerInstanceMutation":{"operation":"upsert","instanceId":self.instance,"instance":instance}}),
            cx,
            move |view, result, _| match result {
                Ok(settings) => {
                    view.error = None;
                    for row in &mut view.rows {
                        if !variables.contains(&row.value) {
                            continue;
                        }
                        if let Some(saved) =
                            settings["providerInstances"][&view.instance]["environment"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .find(|saved| saved["name"] == row.value["name"])
                        {
                            row.value = saved.clone();
                            row.acknowledged = true;
                        }
                    }
                }
                Err(error) => view.error = Some(format!("Unable to save provider: {error:#}")),
            },
        );
    }
}

impl Render for ProviderEnvironment {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for row in &mut self.rows {
            if row.acknowledged {
                row.acknowledged = false;
                row.name_input.update(cx, |input, cx| {
                    if !input.focus_handle(cx).is_focused(window) {
                        input.set_value(row.value["name"].as_str().unwrap_or_default(), window, cx)
                    }
                });
                row.value_input.update(cx, |input, cx| {
                    if !input.focus_handle(cx).is_focused(window) {
                        input.set_value(
                            row.value["value"].as_str().unwrap_or_default(),
                            window,
                            cx,
                        );
                    }
                    input.set_masked(row.value["sensitive"] == true, window, cx);
                });
            }
        }
        let rows = self
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let id = row.id;
                let name = row.value["name"]
                    .as_str()
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| (index + 1).to_string());
                let redacted = row.value["valueRedacted"] == true;
                let sensitive = row.value["sensitive"] == true;
                v_flex()
                    .id(("variable", id))
                    .gap_2()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded_lg()
                    .p_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Input::new(&row.name_input)
                                    .aria_label(format!("Environment variable name {}", index + 1))
                                    .disabled(redacted)
                                    .flex_1(),
                            )
                            .child(
                                Button::new("remove")
                                    .icon(gpui_kit::assets::IconName::X)
                                    .accessibility_label(format!(
                                        "Remove environment variable {name}"
                                    ))
                                    .ghost()
                                    .compact()
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.rows.retain(|row| row.id != id);
                                        view.publish(cx);
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .when(row.value["secret"].is_null(), |view| {
                                view.child(
                                    Input::new(&row.value_input)
                                        .aria_label(format!(
                                            "Environment variable value {}",
                                            index + 1
                                        ))
                                        .flex_1(),
                                )
                            })
                            .when(row.value["secret"].is_object(), |view| {
                                view.child(
                                    settings_controls::text(format!(
                                        "External secret reference (editing not implemented): {}",
                                        row.value["secret"]
                                    ))
                                    .text_xs(),
                                )
                            })
                            .child(
                                Button::new("sensitive")
                                    .icon(gpui_kit::assets::IconName::Lock)
                                    .accessibility_label(format!(
                                        "Mark environment variable {name} as sensitive"
                                    ))
                                    .selected(sensitive)
                                    .toggled(sensitive)
                                    .disabled(redacted || row.value["secret"].is_object())
                                    .ghost()
                                    .compact()
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        if let Some(row) =
                                            view.rows.iter_mut().find(|row| row.id == id)
                                        {
                                            row.value["sensitive"] = json!(!sensitive);
                                            row.value_input.update(cx, |input, cx| {
                                                input.set_masked(!sensitive, window, cx)
                                            });
                                        }
                                        view.publish(cx);
                                        cx.notify();
                                    })),
                            ),
                    )
            })
            .collect::<Vec<_>>();
        v_flex()
            .gap_3()
            .px_4()
            .child(settings_controls::text("Environment").font_weight(FontWeight::SEMIBOLD))
            .child(
                h_flex()
                    .gap_4()
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(settings_controls::text("Variables").text_sm())
                            .child(
                                settings_controls::text(
                                    "API keys, base URLs, and other per-instance CLI settings.",
                                )
                                .text_xs()
                                .text_color(cx.theme().muted_foreground),
                            ),
                    )
                    .child(
                        Button::new("add-variable")
                            .label("Add variable")
                            .outline()
                            .compact()
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.add(json!({"name":"","value":"","sensitive":true}), window, cx)
                            })),
                    ),
            )
            .children(rows)
            .when(!self.rows.is_empty(), |view| {
                view.child(
                    settings_controls::text(
                        "Sensitive values are stored separately and never returned to the app.",
                    )
                    .text_xs(),
                )
            })
            .when(
                self.rows
                    .iter()
                    .any(|row| row.value["valueRedacted"] == true),
                |view| {
                    view.child(
                        settings_controls::text(
                            "Enter a new value to rename a saved one or make it plain text.",
                        )
                        .text_xs(),
                    )
                },
            )
            .when_some(self.error.clone(), |view, error| {
                view.child(settings_controls::text(error).text_color(cx.theme().danger))
            })
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
