use super::*;

impl Root {
    /// Writes client settings to the official app's file and applies them at once.
    pub(super) fn change_setting(
        &mut self,
        key: &'static str,
        value: Value,
        cx: &mut Context<Self>,
    ) {
        match client_settings::update(&self.settings_path, &[(key, value)]) {
            Ok(settings) => {
                let (interface, code) = (settings.font_size_interface, settings.font_size_code);
                self.settings = settings;
                if key == "sidebarWorkingShelfEnabled" {
                    self.rebuild_entries();
                }
                if key == "timestampFormat"
                    && let Err(error) =
                        crate::timestamp::change_format(self.settings.timestamp_format, cx)
                {
                    self.message = Some(format!("Timestamp format failed: {error:#}"));
                }
                if key == "gpuiAppearance" {
                    crate::appearance::apply(self.settings.light_theme, cx);
                }
                Theme::update(cx, |theme| {
                    theme.font_size = px(interface);
                    theme.mono_font_size = px(code);
                });
            }
            Err(error) => self.message = Some(format!("Saving the settings failed: {error:#}")),
        }
        cx.notify();
    }

    pub(super) fn render_settings(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = cx.entity().downgrade();
        let environment = self
            .current()
            .map(|(environment, _)| environment)
            .or_else(|| {
                self.connections
                    .first()
                    .map(|connection| connection.label().to_string())
            })
            .unwrap_or_default();
        let saved = self.snooze_settings(&environment);
        let snooze_options = window.use_keyed_state(
            SharedString::from(format!("settings-snooze-options:{environment}")),
            cx,
            |window, cx| snooze_options::SnoozeOptions::new(root, environment, saved, window, cx),
        );
        let theme = cx.theme().colors;
        let cx: &Context<Self> = cx;
        let s = &self.settings;
        let row =
            |name: &'static str, key: &'static str, options: Vec<(SharedString, Value, bool)>| {
                h_flex()
                    .gap_2()
                    .child(div().w(px(200.)).text_sm().child(name))
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .children(options.into_iter().enumerate().map(
                                move |(i, (label, value, chosen))| {
                                    Button::new(SharedString::from(format!("setting-{key}-{i}")))
                                        .label(label)
                                        .compact()
                                        .when(chosen, |b| b.primary())
                                        .when(!chosen, |b| b.outline())
                                        .on_click(cx.listener(move |root, _, _, cx| {
                                            root.change_setting(key, value.clone(), cx)
                                        }))
                                },
                            )),
                    )
            };
        let size = |name: &'static str, key: &'static str, current: f32| {
            row(
                name,
                key,
                vec![
                    ("−".into(), Value::from((current - 1.).max(10.)), false),
                    (format!("{current:.0}").into(), Value::from(current), true),
                    ("+".into(), Value::from((current + 1.).min(28.)), false),
                ],
            )
        };
        use client_settings::SendShortcut::*;
        let notification_mode = match (s.desktop_notifications, s.notification_sound) {
            (false, false) => "off",
            (true, false) => "notifications",
            (false, true) => "sound",
            (true, true) => "notifications-and-sound",
        };
        div()
            .absolute()
            .top(px(80.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                v_flex()
                    .id("settings-scroll")
                    .max_h(px(
                        (f32::from(window.viewport_size().height) - 120.).max(200.)
                    ))
                    .overflow_y_scroll()
                    .w(px(720.))
                    .p_4()
                    .gap_3()
                    .rounded_md()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .shadow_lg()
                    .child(
                        h_flex().child(div().flex_1().child("Settings")).child(
                            Button::new("close-settings")
                                .label("Close")
                                .compact()
                                .outline()
                                .on_click(cx.listener(|root, _, _, cx| {
                                    root.settings_open = false;
                                    cx.notify();
                                })),
                        ),
                    )
                    .child(row(
                        "Appearance",
                        "gpuiAppearance",
                        vec![
                            ("Dark".into(), Value::from("dark"), !s.light_theme),
                            ("Light".into(), Value::from("light"), s.light_theme),
                        ],
                    ))
                    .child(row(
                        "Send with",
                        "sendShortcut",
                        vec![
                            (
                                "Enter".into(),
                                Value::from("enter"),
                                s.send_shortcut == Enter,
                            ),
                            (
                                "Ctrl+Enter for multiline".into(),
                                Value::from("mod-enter-multiline"),
                                s.send_shortcut == ModEnterMultiline,
                            ),
                            (
                                "Ctrl+Enter always".into(),
                                Value::from("mod-enter"),
                                s.send_shortcut == ModEnter,
                            ),
                        ],
                    ))
                    .child(row(
                        "While a turn runs",
                        "followUpBehavior",
                        vec![
                            ("Steer".into(), Value::from("steer"), s.steer_follow_ups),
                            ("Queue".into(), Value::from("queue"), !s.steer_follow_ups),
                        ],
                    ))
                    .child(row(
                        "Notifications",
                        "notificationMode",
                        vec![
                            ("Off".into(), Value::from("off"), notification_mode == "off"),
                            (
                                "Notifications".into(),
                                Value::from("notifications"),
                                notification_mode == "notifications",
                            ),
                            (
                                "Sound".into(),
                                Value::from("sound"),
                                notification_mode == "sound",
                            ),
                            (
                                "Both".into(),
                                Value::from("notifications-and-sound"),
                                notification_mode == "notifications-and-sound",
                            ),
                        ],
                    ))
                    .child(row(
                        "Plan mode",
                        "planModeEnabled",
                        vec![
                            ("On".into(), Value::from(true), s.plan_mode_enabled),
                            ("Off".into(), Value::from(false), !s.plan_mode_enabled),
                        ],
                    ))
                    .child(size(
                        "Interface text size",
                        "fontSizeInterface",
                        s.font_size_interface,
                    ))
                    .child(size(
                        "Message text size",
                        "fontSizePrompt",
                        s.font_size_prompt,
                    ))
                    .child(size("Code text size", "fontSizeCode", s.font_size_code))
                    .child(h_flex().gap_2().child(v_flex().flex_1().child("Working section").child(div().text_xs().text_color(theme.muted_foreground).child("Fold working and monitoring threads into a Working section. They return to the top of the inbox when they need you. The boring sidebar in Labs has no Working section."))).child(gpui_kit::component::switch::Switch::new("working-shelf").accessibility_label("Working section").checked(s.working_shelf).on_change(cx.listener(|root,checked,_,cx|root.change_setting("sidebarWorkingShelfEnabled",json!(*checked),cx)))))
                    .child(snooze_options)
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!(
                                "Saved to {}, the file the official app reads too.",
                                self.settings_path.display()
                            )),
                    ),
            )
            .into_any_element()
    }
}
