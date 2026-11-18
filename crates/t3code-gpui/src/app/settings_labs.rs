use super::*;

pub(super) fn description(key: &str, root: &WeakEntity<Root>, cx: &App) -> Option<AnyElement> {
    let parts: &'static [(&'static str, &'static str)] = match key {
        "guestSharingEnabled" => &[
            (
                "Share a thread with a guest who watches it live in a browser and sends messages you approve. Needs T3 Connect: sign in and link this machine in",
                "",
            ),
            ("Sharing & access", "access"),
            (". Turning it off stops every guest link.", ""),
        ],
        "linkedEnvironmentsEnabled" => &[
            (
                "Let agents here work on your other machines, and theirs here. Turn it on for each machine, then link them in",
                "",
            ),
            ("Connections", "connections"),
            (
                ". While off, nothing new starts over a link; running tasks finish.",
                "",
            ),
        ],
        "simlockDeviceHostsEnabled" => &[
            (
                "Let the Device panel lease simulators and emulators from your own",
                "",
            ),
            ("Simlock", "https://incubator.callstack.com/simlock/"),
            ("pool, added in", ""),
            ("Devices", "devices"),
            (
                ". An alpha that may change or break. T3 Code never installs Simlock.",
                "",
            ),
        ],
        _ => return None,
    };
    Some(
        h_flex()
            .id("labs-description")
            .w_full()
            .min_w_0()
            .flex_wrap()
            .gap_x_1()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .children(parts.iter().enumerate().map(|(index, &(label, target))| {
                if target.is_empty() {
                    return settings_controls::text(label)
                        .max_w_full()
                        .into_any_element();
                }
                let root = root.clone();
                gpui_kit::base::Link::new(("setting-link", index))
                    .href(target)
                    .accessibility_label(label)
                    .text_decoration_1()
                    .child(label)
                    .open_with(move |_, _, window, cx| {
                        if target.starts_with("https://") {
                            cx.open_url(target);
                        } else {
                            let _ = root.update(cx, |root, cx| {
                                root.settings_page = target;
                                root.settings_focus.focus(window, cx);
                                cx.notify();
                            });
                        }
                    })
                    .into_any_element()
            }))
            .into_any_element(),
    )
}

pub(super) fn guest_links_denial(root: &Root, environment: &str) -> Option<String> {
    root.connections
        .iter()
        .filter(|connection| {
            root.settings_environment.is_none() || connection.label() == environment
        })
        .find_map(|connection| {
            let config = root.config_of(connection.label())?;
            let policy = &config["managedPolicy"];
            (policy["limits"]["connect"] == false).then(|| {
                format!(
                    "{}'s policy does not allow T3 Connect.",
                    policy["organization"]
                        .as_str()
                        .expect("managed organization")
                )
            })
        })
}
