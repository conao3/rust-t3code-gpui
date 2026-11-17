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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        v_flex().gap_3().when_some(self.error.clone(),|view,error|view.child(div().id("provider-save-error").role(accesskit::Role::Alert).text_color(cx.theme().danger).child(error)))
            .child(h_flex().gap_2().child(Button::new("add-provider").label("Add provider").outline().on_click(cx.listener(|view,_,window,cx|view.edit(String::new(),json!({"driver":"codex"}),window,cx)))).child(Button::new("refresh-providers").label("Refresh").ghost().on_click(cx.listener(|view,_,_,cx|view.request("server.refreshProviders",json!({}),cx)))))
            .when(self.editing.is_some(),|view|view.child(v_flex().gap_2().p_3().border_1().border_color(cx.theme().border)
                .child(Select::new(&self.driver).accessibility_label("Provider").disabled(!self.creating))
                .children(self.inputs.iter().zip(["Instance ID","Display name","Binary path","Home path"]).map(|(input,label)|Input::new(input).aria_label(label).disabled(label=="Instance ID"&&!self.creating)))
                .child(Switch::new("provider-enabled").accessibility_label("Enabled").label("Enabled").checked(self.enabled).on_change(cx.listener(|view,checked,_,cx| {view.enabled = *checked;cx.notify();})))
                .child(h_flex().gap_2().child(Button::new("save-provider").label("Save").disabled(self.saving).primary().on_click(cx.listener(|view,_,_,cx|view.save(cx)))).child(Button::new("cancel-provider").label("Cancel").ghost().on_click(cx.listener(|view,_,_,cx| {view.editing=None;cx.notify();}))))))
            .children(instances.into_iter().enumerate().map(|(index,(id,instance))| {
                let edit_id=id.clone();let edit_instance=instance.clone();let enable_id=id.clone();let enabled=instance["enabled"].as_bool().unwrap_or(true);
                let provider=config["providers"].as_array().into_iter().flatten().find(|provider|provider["instanceId"]==id);
                let status=provider.and_then(|provider|provider["status"].as_str()).unwrap_or("unavailable").to_string();
                h_flex().id(("provider",index)).role(accesskit::Role::Group).aria_label(id.clone()).gap_3().p_3().border_1().border_color(cx.theme().border).rounded_md()
                    .child(v_flex().flex_1().child(instance["displayName"].as_str().unwrap_or(&id).to_string()).child(div().text_xs().text_color(cx.theme().muted_foreground).child(format!("{} · {status}",instance["driver"].as_str().unwrap_or_default()))))
                    .child(Switch::new(("enabled",index)).accessibility_label(format!("Enable {id}")).checked(enabled).on_change(cx.listener(move |view,checked,_,cx| {let mut instance=instance.clone();instance["enabled"]=json!(*checked);view.request("server.updateSettings",json!({"patch":{},"providerInstanceMutation":{"operation":"upsert","instanceId":enable_id,"instance":instance}}),cx);})))
                    .child(Button::new(("edit",index)).label("Edit").ghost().on_click(cx.listener(move |view,_,window,cx|view.edit(edit_id.clone(),edit_instance.clone(),window,cx))))
                    .child(Button::new(("remove",index)).label("Remove").ghost().on_click(cx.listener(move |view,_,window,cx|view.remove(id.clone(),window,cx))))
            }))
    }
}
