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
    fn select_settings_theme(&mut self, id: &str, cx: &mut Context<Self>) {
        self.change_setting("gpuiThemeLight", json!(id), cx);
        self.change_setting("gpuiThemeDark", json!(id), cx);
    }

    pub(super) fn render_appearance_library(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme().colors;
        let mode = self.settings.values["gpuiAppearance"]
            .as_str()
            .unwrap_or("dark");
        v_flex()
            .id("appearance-library")
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
                                                        self.settings.values["gpuiThemeLight"]
                                                            .as_str()
                                                            .unwrap_or("t3-code"),
                                                        self.settings.values["gpuiThemeDark"]
                                                            .as_str()
                                                            .unwrap_or("t3-code"),
                                                    ))
                                                    .child(
                                                        settings_controls::text(label)
                                                            .text_xs()
                                                            .text_center(),
                                                    ),
                                            )
                                            .on_click(cx.listener(move |root, _, _, cx| {
                                                root.change_setting(
                                                    "gpuiAppearance",
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
                        settings_controls::text("Themes")
                            .px_4()
                            .font_weight(FontWeight::SEMIBOLD),
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
                                                    let picked = self.settings.values[if light {
                                                        "gpuiThemeLight"
                                                    } else {
                                                        "gpuiThemeDark"
                                                    }]
                                                    .as_str()
                                                    .unwrap_or("t3-code")
                                                        == id;
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
                                                                root.change_setting(
                                                                    if light {
                                                                        "gpuiThemeLight"
                                                                    } else {
                                                                        "gpuiThemeDark"
                                                                    },
                                                                    json!(id),
                                                                    cx,
                                                                );
                                                                cx.stop_propagation();
                                                            },
                                                        ))
                                                },
                                            ),
                                        ))
                                        .child(
                                            Button::new("use-theme")
                                                .toggled(false)
                                                .accessibility_label(format!("Use {label} theme"))
                                                .ghost()
                                                .justify_start()
                                                .w_full()
                                                .h(px(20.))
                                                .p_0()
                                                .child(settings_controls::text(label).w_full().text_sm())
                                                .on_click(cx.listener(move |root, _, _, cx| {
                                                    root.select_settings_theme(id, cx);
                                                    cx.stop_propagation();
                                                })),
                                        )
                                }))
                        }),
                    ),
            )
    }
}
