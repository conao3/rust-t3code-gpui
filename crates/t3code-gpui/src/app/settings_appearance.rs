use super::*;

fn pane(id: &str, light: bool) -> Div {
    let color = |role| {
        if id == "t3-code" {
            match (role, light) {
                ("canvas", false) => rgb(0x0a0a0a).into(),
                ("sidebar", false) => rgb(0x0f0f10).into(),
                ("surface", false) => rgb(0x121212).into(),
                ("accentSurface" | "messageSurface", false) => rgb(0x27272a).into(),
                ("messageSurface", true) => rgb(0xe4e4e7).into(),
                ("messageAction", true) => rgb(0x4f46e5).into(),
                ("messageAction", false) => rgb(0x8b9cff).into(),
                _ => crate::appearance::preset_color(id, role, light),
            }
        } else {
            crate::appearance::preset_color(id, role, light)
        }
    };
    let line = Hsla::from(rgb(0x7f7f7f)).opacity(0.25);
    let rect = |x, y, w, h, color: Hsla| {
        div()
            .absolute()
            .left(relative(x))
            .top(relative(y))
            .w(relative(w))
            .h(relative(h))
            .rounded_sm()
            .bg(color)
    };
    div()
        .absolute()
        .inset_0()
        .bg(color("canvas"))
        .child(
            rect(0., 0., 0.22, 1., color("sidebar"))
                .rounded_none()
                .border_r_1()
                .border_color(line),
        )
        .child(
            rect(0.03, 0.08, 0.16, 0.08, color("surface"))
                .border_1()
                .border_color(line),
        )
        .child(rect(0.03, 0.22, 0.16, 0.07, color("accentSurface")))
        .child(rect(
            0.03,
            0.32,
            0.16,
            0.07,
            color("messageSurface").opacity(0.7),
        ))
        .child(rect(
            0.03,
            0.42,
            0.16,
            0.07,
            color("messageSurface").opacity(0.5),
        ))
        .child(rect(0.48, 0.11, 0.24, 0.09, color("messageSurface")))
        .child(rect(0.27, 0.28, 0.34, 0.05, line))
        .child(rect(0.27, 0.38, 0.26, 0.05, line))
        .child(
            rect(0.26, 0.77, 0.68, 0.15, color("surface"))
                .border_1()
                .border_color(line)
                .child(rect(0.03, 0.37, 0.34, 0.26, line.opacity(0.7)).rounded_full())
                .child(
                    div()
                        .absolute()
                        .right(px(4.))
                        .top(relative(0.21))
                        .size(px(12.))
                        .rounded_full()
                        .bg(color("messageAction")),
                ),
        )
        .child(
            rect(0.75, 0.08, 0.2, 0.46, color("surface"))
                .border_1()
                .border_color(line)
                .shadow_sm()
                .children((0..3).map(|row| {
                    h_flex()
                        .absolute()
                        .left(relative(0.11))
                        .right(relative(0.11))
                        .top(relative(0.1 + row as f32 * 0.3))
                        .h(relative(0.2))
                        .gap_1()
                        .child(
                            div().size(px(3.)).rounded_full().bg(match row {
                                0 => Hsla::from(rgb(0x34d399)),
                                1 => color("messageAction"),
                                _ => rgb(0xfbbf24).into(),
                            }
                            .opacity(0.55)),
                        )
                        .child(
                            div()
                                .h(relative(0.3))
                                .w(relative(0.52))
                                .rounded_sm()
                                .bg(line),
                        )
                })),
        )
}

fn preview(mode: &str, light: &str, dark: &str) -> Div {
    div()
        .relative()
        .h(px(140.))
        .w_full()
        .rounded_lg()
        .overflow_hidden()
        .child(if mode == "system" {
            h_flex()
                .size_full()
                .gap(px(2.))
                .child(
                    div()
                        .relative()
                        .w(relative(0.5))
                        .h_full()
                        .overflow_hidden()
                        .child(
                            div()
                                .relative()
                                .w(relative(2.))
                                .h_full()
                                .child(pane(light, true)),
                        ),
                )
                .child(
                    div()
                        .relative()
                        .w(relative(0.5))
                        .h_full()
                        .overflow_hidden()
                        .child(
                            div()
                                .relative()
                                .left(relative(-1.))
                                .w(relative(2.))
                                .h_full()
                                .child(pane(dark, false)),
                        ),
                )
                .into_any_element()
        } else {
            pane(if mode == "light" { light } else { dark }, mode == "light").into_any_element()
        })
}

impl Root {
    fn save_theme_selection(&mut self, changes: &[(&str, Option<Value>)], cx: &mut Context<Self>) {
        match client_settings::update_entries(&self.settings_path, changes) {
            Ok(settings) => {
                self.settings = settings;
                crate::appearance::apply(&self.settings, cx);
                crate::appearance::apply_typography(&self.settings, cx);
                self.settings.light_theme = !cx.theme().mode.is_dark();
            }
            Err(error) => self.message = Some(format!("Couldn’t save theme selection: {error:#}")),
        }
        cx.notify();
    }

    fn select_settings_theme(&mut self, id: &str, cx: &mut Context<Self>) {
        let mode = self.settings.values["t3code:theme-appearance-mode"]
            .as_str()
            .unwrap_or("system");
        let theme = if id == "t3-code" { mode } else { id };
        self.save_theme_selection(
            &[
                ("t3code:theme-appearance-mode", Some(json!(mode))),
                ("t3code:theme", Some(json!(theme))),
                ("t3code:theme-halves:v1", None),
            ],
            cx,
        );
    }

    fn select_settings_theme_half(&mut self, id: &str, light: bool, cx: &mut Context<Self>) {
        let mode = if light { "light" } else { "dark" };
        let other = if light { "dark" } else { "light" };
        let mut halves = self.settings.values["t3code:theme-halves:v1"]
            .as_str()
            .map(|raw| serde_json::from_str::<Value>(raw).expect("valid saved theme halves"))
            .unwrap_or_else(|| json!({}));
        let base = self.settings.values["t3code:theme"]
            .as_str()
            .unwrap_or("system");
        let mut changes = vec![];
        if id == "t3-code" && !matches!(base, "system" | "light" | "dark") {
            let owner = halves[other].as_str().unwrap_or(base).to_string();
            halves = json!({other: owner});
            changes.push((
                "t3code:theme",
                Some(json!(
                    self.settings.values["t3code:theme-appearance-mode"]
                        .as_str()
                        .unwrap_or("system")
                )),
            ));
        } else if id == "t3-code" {
            halves
                .as_object_mut()
                .expect("theme halves object")
                .remove(mode);
        } else {
            halves[mode] = json!(id);
        }
        changes.push((
            "t3code:theme-halves:v1",
            if halves.as_object().expect("theme halves object").is_empty() {
                None
            } else {
                Some(json!(halves.to_string()))
            },
        ));
        self.save_theme_selection(&changes, cx);
    }

    pub(super) fn render_appearance_library(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme().colors;
        let mode = self.settings.values["t3code:theme-appearance-mode"]
            .as_str()
            .unwrap_or("system");
        v_flex()
            .id("appearance-library")
            .role(accesskit::Role::Group)
            .aria_label("Colors & themes")
            .gap_6()
            .child(
                v_flex()
                    .gap_3()
                    .child(
                        settings_controls::text("Color scheme")
                            .px_4()
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        h_flex()
                            .id("appearance-mode")
                            .role(accesskit::Role::Group)
                            .aria_label("Appearance mode")
                            .gap_3()
                            .children(
                                [("system", "System"), ("light", "Light"), ("dark", "Dark")]
                                    .into_iter()
                                    .map(|(value, label)| {
                                        Button::new(value)
                                            .accessibility_label(if value == "system" {
                                                "Follow the system appearance".to_string()
                                            } else {
                                                format!("Use {value} mode")
                                            })
                                            .selected(value == mode)
                                            .toggled(value == mode)
                                            .outline()
                                            .border_1()
                                            .border_color(if value == mode {
                                                theme.primary
                                            } else {
                                                theme.border
                                            })
                                            .flex_1()
                                            .h_auto()
                                            .p_2()
                                            .child(
                                                v_flex()
                                                    .w_full()
                                                    .gap_2()
                                                    .child(preview(
                                                        value,
                                                        &crate::appearance::theme_half(&self.settings.values, "light"),
                                                        &crate::appearance::theme_half(&self.settings.values, "dark"),
                                                    ))
                                                    .child(
                                                        settings_controls::text(label)
                                                            .text_xs()
                                                            .text_center(),
                                                    ),
                                            )
                                            .on_click(cx.listener(move |root, _, _, cx| {
                                                root.change_setting(
                                                    "t3code:theme-appearance-mode",
                                                    json!(value),
                                                    cx,
                                                )
                                            }))
                                    }),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .gap_3()
                    .child(
                        h_flex().px_4().justify_between()
                            .child(settings_controls::text("Themes").font_weight(FontWeight::SEMIBOLD))
                            .child(h_flex().gap_2()
                                .child(Button::new("create-theme").label("Create theme").outline().compact().on_click(|_, window, cx| theme_editor_notice("Create theme", window, cx)))
                                .child(Button::new("add-theme").label("Add theme").outline().compact().on_click(|_, window, cx| theme_editor_notice("Add theme", window, cx)))),
                    )
                    .children(
                        [
                            [
                                ("t3-code", "T3 Code"),
                                ("t3-chat", "T3 Chat"),
                                ("grove", "Grove"),
                            ],
                            [("ocean", "Ocean"), ("ember", "Ember"), ("iris", "Iris")],
                        ]
                        .into_iter()
                        .map(|row| {
                            h_flex()
                                .gap_2()
                                .children(row.into_iter().map(|(id, label)| {
                                    v_flex()
                                        .id(id)
                                        .cursor_pointer()
                                        .on_click(cx.listener(move |root, _, _, cx| root.select_settings_theme(id, cx)))
                                        .flex_1()
                                        .gap_2()
                                        .p_3()
                                        .rounded_lg()
                                        .border_1()
                                        .border_color(theme.border)
                                        .bg(theme.muted.opacity(0.6))
                                        .child(h_flex().justify_center().gap_4().children(
                                            [("light", true), ("dark", false)].into_iter().map(
                                                |(mode, light)| {
                                                    let picked = crate::appearance::theme_half(&self.settings.values, mode) == id;
                                                    Button::new(mode)
                                                        .accessibility_label(format!(
                                                            "Use {label} {mode} mode"
                                                        ))
                                                        .ghost()
                                                        .size(px(68.))
                                                        .p_0()
                                                        .rounded_full()
                                                        .selected(picked)
                                                        .toggled(picked)
                                                        .border_2()
                                                        .border_color(if picked {
                                                            theme.primary
                                                        } else {
                                                            transparent_black()
                                                        })
                                                        .child(
                                                            div().relative().size(px(56.)).child(img(SharedString::from(format!("themes/{id}-{mode}.svg"))).size_full().rounded_full()).when(picked, |circle| circle.child(div().absolute().bottom(px(-4.)).right(px(-4.)).size(px(18.)).rounded_full().bg(theme.background).border_1().border_color(theme.border).flex().items_center().justify_center().child(Icon::new(if light { gpui_kit::assets::IconName::Sun } else { gpui_kit::assets::IconName::Moon }).size(px(12.)))))
                                                        )
                                                        .on_click(cx.listener(
                                                            move |root, _, _, cx| {
                                                                root.select_settings_theme_half(id, light, cx);
                                                                cx.stop_propagation();
                                                            },
                                                        ))
                                                },
                                            ),
                                        ))
                                        .child(h_flex().justify_between().child(
                                            Button::new("use-theme")
                                                .toggled(false)
                                                .accessibility_label(format!("Use {label} theme"))
                                                .ghost()
                                                .justify_start()
                                                .flex_1()
                                                .min_w_0()
                                                .h(px(20.))
                                                .p_0()
                                                .child(settings_controls::text(label).w_full().text_sm())
                                                .on_click(cx.listener(move |root, _, _, cx| {
                                                    root.select_settings_theme(id, cx);
                                                    cx.stop_propagation();
                                                }))
                                        ).child(Button::new("duplicate-theme")
                                            .icon(gpui_kit::assets::IconName::Copy)
                                            .accessibility_label(format!("Duplicate {label}"))
                                            .ghost().compact().on_click(move |_, window, cx| {
                                                cx.stop_propagation();
                                                theme_editor_notice(&format!("Duplicate {label}"), window, cx);
                                            })))
                                }))
                        }),
                    ),
            )
    }
}

fn theme_editor_notice(title: &str, window: &mut Window, cx: &mut App) {
    let title = title.to_string();
    window.open_alert_dialog(cx, move |dialog, _, _| {
        dialog
            .title(title.clone())
            .description("Theme editing and importing are not implemented in this client.")
    });
}

const DETAILS_ITEMS: [(&str, &str, bool); 12] = [
    ("environment", "Environment and worktree", true),
    ("openIn", "Open in editor", true),
    ("scripts", "Project scripts", true),
    ("branch", "Branch and pull request", true),
    ("gitActions", "Git actions", true),
    ("changes", "Changes", true),
    ("servers", "Servers", true),
    ("automations", "Automations", true),
    ("lineage", "Lineage", true),
    ("sideChats", "Side chats", true),
    ("usage", "Usage limits", false),
    ("context", "Context window", false),
];

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = t3code_gpui, no_json)]
pub(super) struct SetThreadDetails {
    item: String,
    visible: bool,
}

impl Root {
    pub(super) fn set_thread_details(
        &mut self,
        action: &SetThreadDetails,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if action.item.is_empty() {
            self.change_setting("threadDetailsItems", json!({}), cx);
            self.change_setting("threadDetailsUsageEnabled", json!(false), cx);
            return;
        }
        let Some((id, _, default)) = DETAILS_ITEMS.iter().find(|(id, _, _)| *id == action.item)
        else {
            return;
        };
        let default = if matches!(*id, "usage" | "context") {
            self.settings.values["threadDetailsUsageEnabled"] == true
        } else {
            *default
        };
        let mut items = self.settings.values["threadDetailsItems"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        if action.visible == default {
            items.remove(*id);
        } else {
            items.insert(id.to_string(), json!(action.visible));
        }
        self.change_setting("threadDetailsItems", json!(items), cx);
    }

    pub(super) fn render_thread_details_setting(&self, cx: &Context<Self>) -> impl IntoElement {
        let values = self.settings.values.clone();
        let visibility = DETAILS_ITEMS.map(|(id, _, default)| {
            values["threadDetailsItems"][id].as_bool().unwrap_or(
                if matches!(id, "usage" | "context") {
                    values["threadDetailsUsageEnabled"] == true
                } else {
                    default
                },
            )
        });
        let count = visibility.iter().filter(|visible| **visible).count();
        let changed = DETAILS_ITEMS
            .iter()
            .zip(visibility)
            .any(|((_, _, default), visible)| *default != visible);
        h_flex().id("thread-details-setting").px_4().py_3().gap_4().border_b_1().border_color(cx.theme().border)
            .child(v_flex().flex_1().gap_1()
                .child(settings_controls::text("Thread details").text_sm())
                .child(settings_controls::text("Choose what the thread details card shows. You can also right-click the card.").text_xs().text_color(cx.theme().muted_foreground)))
            .when(changed, |row| row.child(Button::new("reset-thread-details").icon(gpui_kit::assets::IconName::Undo2).accessibility_label("Reset thread details to default").ghost().compact().on_click(cx.listener(|root, _, window, cx| root.set_thread_details(&SetThreadDetails { item: String::new(), visible: false }, window, cx)))))
            .child(Button::new("thread-details-items").accessibility_label(format!("Thread details: {count} of 12 on"))
                .outline().w(px(160.)).h(px(28.)).child(h_flex().gap_1().children([count.to_string(), "of".into(), "12".into(), "on".into()].into_iter().enumerate().map(|(index, value)| settings_controls::text(value).id(("thread-details-count", index)))))
                .dropdown_menu(move |mut menu, _, _| {
                    menu = menu.label("Show in thread details");
                    for ((id, label, _), visible) in DETAILS_ITEMS.into_iter().zip(visibility) {
                        menu = menu.menu_with_check(label, visible, Box::new(SetThreadDetails { item: id.to_string(), visible: !visible }));
                    }
                    menu.separator().item(PopupMenuItem::new("Reset to default").disabled(!changed).on_click(|_, window, cx| window.dispatch_action(Box::new(SetThreadDetails { item: String::new(), visible: false }), cx)))
                }))
    }
}
