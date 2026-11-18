use super::*;
use gpui_kit::component::switch::Switch;

pub(super) struct DevicePanel {
    root: WeakEntity<Root>,
    connection: Arc<Connection>,
    runtime: Handle,
    state: Value,
    step: usize,
    pub(super) setup: bool,
    busy: bool,
    keep_open: Rc<std::cell::Cell<bool>>,
    error: Option<String>,
    worker: tokio::task::JoinHandle<()>,
}

impl Drop for DevicePanel {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

impl DevicePanel {
    pub(super) fn new(
        root: WeakEntity<Root>,
        connection: Arc<Connection>,
        runtime: Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        use t3_client::rpc::StreamLimits;
        let (sender, receiver) = async_channel::bounded(8);
        let stream_connection = connection.clone();
        let worker = runtime.spawn(async move {
            let result: anyhow::Result<()> = async {
                let mut stream = stream_connection
                    .subscribe(
                        "subscribeDeviceState",
                        json!({}),
                        StreamLimits::new(8, 4 << 20).expect("positive stream limits"),
                    )
                    .await?;
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk?;
                    for value in chunk.values() {
                        if sender.send(Ok(value.clone())).await.is_err() {
                            return Ok(());
                        }
                    }
                    chunk.accept();
                }
                Ok(())
            }
            .await;
            if let Err(error) = result {
                let _ = sender.send(Err(format!("{error:#}"))).await;
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(mut result) = receiver.recv().await {
                cx.background_executor().timer(REDRAW_INTERVAL).await;
                while let Ok(next) = receiver.try_recv() { result = next; }
                if this.update_in(cx, |view, window, cx| {
                    let initial = view.state.is_null();
                    match result { Ok(state) => view.state = state, Err(error) => view.error = Some(error) }
                    if initial && !view.state.is_null() && (view.state["onboardingCompleted"] != true || view.state["hostStatus"] == "disabled") {
                        view.setup = true;
                        let _ = view.root.update(cx, |root, cx| { root.panel_revision += 1; cx.notify(); });
                        let keep_open = view.keep_open.clone();
                        let entity = cx.entity();
                        let root = view.root.clone();
                        window.open_dialog(cx, move |dialog, _, cx| {
                            let root = root.clone();
                            let keep_open = keep_open.clone();
                            dialog.title(text!("Set up devices")).accessibility_label("Set up devices").width(px(576.)).centered()
                                .child(div().text_sm().text_color(cx.theme().muted_foreground).child(text!("Review what runs on this environment before using simulators and emulators.")))
                                .child(entity.clone())
                                .on_close(move |_, _, cx| { if !keep_open.get() { let _ = root.update(cx, |root, cx| root.close_panel_tab("device", cx)); } })
                        });
                    }
                    cx.notify();
                }).is_err() { break; }
            }
        }).detach();
        Self {
            root,
            connection,
            runtime,
            state: Value::Null,
            step: 0,
            setup: false,
            busy: false,
            keep_open: Rc::new(std::cell::Cell::new(false)),
            error: None,
            worker,
        }
    }

    fn request(
        &mut self,
        method: &'static str,
        input: Value,
        finish: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let connection = self.connection.clone();
        let runtime = self.runtime.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = on_runtime(
                &runtime,
                async move { connection.request(method, input).await },
            )
            .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.busy = false;
                match result {
                    Ok(state) => {
                        if state["hostStatus"].is_string() {
                            view.state = state;
                        }
                        if finish {
                            view.setup = false;
                            view.keep_open.set(true);
                            let _ = view.root.update(cx, |root, cx| {
                                root.panel_revision += 1;
                                cx.notify();
                            });
                            window.close_dialog(cx);
                        }
                    }
                    Err(error) => view.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn ready(&self) -> bool {
        self.state["hostStatus"] == "ready"
            || self.state["hostStatuses"]
                .as_object()
                .is_some_and(|hosts| hosts.values().any(|host| host["status"] == "ready"))
    }

    fn platform_message(&self, platform: &str) -> String {
        let hosts = self.state["hosts"].as_array().cloned().unwrap_or_default();
        let available = hosts.iter().find(|host| {
            host["platforms"].as_array().is_some_and(|entries| {
                entries
                    .iter()
                    .any(|entry| entry["platform"] == platform && entry["available"] == true)
            })
        });
        let name = if platform == "ios" { "iOS" } else { "Android" };
        let Some(host) = available else {
            return hosts
                .iter()
                .flat_map(|host| host["platforms"].as_array().into_iter().flatten())
                .find(|entry| entry["platform"] == platform)
                .and_then(|entry| entry["reason"].as_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("{name} support was not detected."));
        };
        let devices = self.state["devices"]
            .as_array()
            .is_some_and(|devices| devices.iter().any(|device| device["platform"] == platform));
        if self.ready() && !devices {
            return if platform == "ios" { "Xcode is installed, but no iOS Simulator is available. Install a runtime in Xcode Settings → Components." } else { "The Android SDK is installed, but no virtual device exists. Create one in Android Studio → Device Manager." }.into();
        }
        let location = if host["kind"] == "local" {
            String::new()
        } else {
            format!(" on {}", host["label"].as_str().unwrap_or_default())
        };
        if platform == "ios" {
            format!("Xcode and iOS Simulator are available{location}.")
        } else {
            format!("The Android SDK and Emulator are available{location}.")
        }
    }
}

impl Render for DevicePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = self.state["hostStatus"] != "disabled" && !self.state.is_null();
        let busy = self.busy
            || matches!(
                self.state["hostStatus"].as_str(),
                Some("installing" | "starting")
            );
        let ready = self.ready();
        let mut body = v_flex().w_full().min_w_0().gap_4().text_sm();
        if self.setup {
            body = body.child(
                h_flex().w_full().gap_1().children(
                    ["Device hub", "Simulators", "Agent access"]
                        .into_iter()
                        .enumerate()
                        .map(|(index, label)| {
                            Button::new(("device-step", index))
                                .label(format!("{} {label}", index + 1))
                                .flex_1()
                                .accessibility_label(format!("{label}, step {}", index + 1))
                                .ghost()
                                .selected(index == self.step)
                                .disabled(busy || index > self.step)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.step = index;
                                    cx.notify();
                                }))
                        }),
                ),
            );
            if self.step == 0 {
                body = body.child(text!("Enable the device hub")).child(h_flex().items_start().gap_4()
                    .child(div().flex_1().min_w_0().whitespace_normal().text_color(cx.theme().muted_foreground).child(text!("Enable this environment to open simulators and emulators, whether they run here or on a remote device host.")))
                    .child(Switch::new("enable-device-hub").accessibility_label("Enable device hub").checked(enabled).disabled(busy).on_click(cx.listener(|view, enabled, window, cx| { let mut input = json!({"enabled":enabled}); if !enabled { input["agentAccessEnabled"] = json!(false); } view.request("device.configure", input, false, window, cx); }))));
                if busy || ready {
                    body = body.child(text!(if busy {
                        match self.state["hostStatus"].as_str() {
                            Some("installing") => "Installing device hub…",
                            Some("starting") => "Starting device hub…",
                            _ => "Updating device hub…",
                        }
                    } else {
                        "Device hub is ready."
                    }));
                }
            }
            let unavailable = self.state["hosts"].as_array().is_some_and(|hosts| {
                hosts.iter().any(|host| {
                    host["kind"] == "local"
                        && host["platforms"].as_array().is_some_and(|platforms| {
                            !platforms
                                .iter()
                                .any(|platform| platform["available"] == true)
                        })
                })
            });
            if self.step == 1 || self.step == 0 && enabled && unavailable {
                body = body.child(text!("Check simulator support"));
                for (platform, label) in [("ios", "iOS"), ("android", "Android")] {
                    body = body.child(
                        v_flex()
                            .id(platform)
                            .gap_1()
                            .p_3()
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded_md()
                            .child(text!(label))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(text!(self.platform_message(platform))),
                            ),
                    );
                }
                body = body.child(div().text_xs().child(text!("You can use either platform. Fixing a missing platform does not block the other one.")))
                    .child(Button::new("check-devices").label(if self.busy { "Checking…" } else { "Check again" }).outline().disabled(!enabled || busy).on_click(cx.listener(|view, _, window, cx| view.request("device.list", json!({}), false, window, cx))));
            }
            if self.step == 2 {
                body = body.child(text!("Allow agent control")).child(h_flex().items_start().gap_4()
                    .child(div().flex_1().min_w_0().whitespace_normal().text_color(cx.theme().muted_foreground).child(text!("Allow new agent sessions in this environment to start and control local and remote devices, with required tools set up automatically.")))
                    .child(Switch::new("agent-device-access").accessibility_label("Allow agents to control devices").checked(self.state["agentAccessEnabled"] == true).disabled(!enabled || busy).on_click(cx.listener(|view, enabled, window, cx| view.request("device.configure", json!({"agentAccessEnabled":enabled}), false, window, cx)))))
                    .child(div().text_xs().child(text!("Leave this off to keep manual device controls without giving agents access.")));
            }
            body = body.child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("device-back")
                            .label(if self.step == 0 { "Cancel" } else { "Back" })
                            .outline()
                            .disabled(busy)
                            .on_click(cx.listener(|view, _, window, cx| {
                                if view.step == 0 {
                                    window.close_dialog(cx);
                                    let _ = view
                                        .root
                                        .update(cx, |root, cx| root.close_panel_tab("device", cx));
                                } else {
                                    view.step -= 1;
                                    cx.notify();
                                }
                            })),
                    )
                    .child(
                        Button::new("device-next")
                            .label(if self.step < 2 {
                                "Continue"
                            } else if self.busy {
                                "Saving…"
                            } else {
                                "Done"
                            })
                            .primary()
                            .disabled(!ready || busy)
                            .on_click(cx.listener(|view, _, window, cx| {
                                if view.step < 2 {
                                    view.step += 1;
                                    cx.notify();
                                } else {
                                    view.request(
                                        "device.configure",
                                        json!({"onboardingCompleted":true}),
                                        true,
                                        window,
                                        cx,
                                    );
                                }
                            })),
                    ),
            );
        } else {
            body = body.p_5().size_full().justify_center().items_center();
            let devices = self.state["devices"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if devices.is_empty() {
                body = body
                    .child(Icon::new(gpui_kit::assets::IconName::Smartphone))
                    .child(text!(if self.state.is_null() {
                        "Finding devices…"
                    } else {
                        "No simulators or emulators were found on this environment."
                    }));
            }
            for (index, device) in devices.iter().enumerate() {
                let name = device["name"].as_str().unwrap_or_default();
                let thread = self
                    .root
                    .update(cx, |root, _| root.current().map(|(_, id)| id))
                    .ok()
                    .flatten();
                let input = json!({"hostId":device["hostId"],"deviceId":device["id"],"platform":device["platform"],"threadId":thread});
                body = body.child(
                    h_flex()
                        .id(("device-row", index))
                        .gap_3()
                        .child(text!(name.to_string()))
                        .child(
                            Button::new(("open-device", index))
                                .label(if device["booted"] == true {
                                    "Open"
                                } else {
                                    "Start"
                                })
                                .accessibility_label(format!(
                                    "{} {name}",
                                    if device["booted"] == true {
                                        "Open"
                                    } else {
                                        "Start"
                                    }
                                ))
                                .disabled(busy)
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.request("device.open", input.clone(), false, window, cx)
                                })),
                        ),
                );
            }
            body = body.child(
                Button::new("refresh-devices")
                    .label("Refresh devices")
                    .outline()
                    .disabled(busy)
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.request("device.list", json!({}), false, window, cx)
                    })),
            );
        }
        if let Some(error) = self.error.as_deref().or(self.state["hostStatusDetail"]
            .as_str()
            .filter(|_| self.state["hostStatus"] == "failed"))
        {
            body = body.child(
                div()
                    .text_color(cx.theme().danger)
                    .child(text!(error.to_string())),
            );
        }
        body
    }
}
