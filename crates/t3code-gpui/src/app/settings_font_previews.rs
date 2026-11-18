use super::*;
use std::cell::RefCell;

const PROMPT: &str = "Use $frontend-design to fix the flaky test in [surface.test.ts](apps/web/src/terminal/ghostty/surface.test.ts) and align the header with [SettingsPanels.tsx](apps/web/src/components/settings/SettingsPanels.tsx) before shipping.";
const TERMINAL_PROMPT: &str = "\x1b[1;32m→\x1b[0m \x1b[1;36mt3code\x1b[0m \x1b[1;34mgit:(\x1b[1;31mmain\x1b[1;34m)\x1b[0m \x1b[1;33m✗\x1b[0m ";

pub(super) struct FontPreviews {
    root: WeakEntity<Root>,
    key: &'static str,
    prompt: Entity<TextareaState>,
    terminal: Option<t3_terminal::Terminal>,
    terminal_input: Entity<InputState>,
    terminal_text: String,
    light: bool,
    cache: Rc<RefCell<crate::terminal_paint::Cache>>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl FontPreviews {
    pub(super) fn new(
        root: WeakEntity<Root>,
        key: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let prompt = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx);
            state.set_auto_grow(2, 6, cx);
            let mut content = gpui_kit::base::input::InputContent::new(PROMPT.to_string());
            for (source, label) in [
                ("$frontend-design", "frontend-design"),
                (
                    "[surface.test.ts](apps/web/src/terminal/ghostty/surface.test.ts)",
                    "surface.test.ts",
                ),
                (
                    "[SettingsPanels.tsx](apps/web/src/components/settings/SettingsPanels.tsx)",
                    "SettingsPanels.tsx",
                ),
            ] {
                let start = PROMPT.find(source).expect("preview token");
                content = content
                    .with_token(
                        start..start + source.len(),
                        gpui_kit::base::input::InlineToken::new(source, source).with_label(label),
                    )
                    .expect("preview token range");
            }
            state.set_value(content, window, cx);
            state
        });
        let terminal_input = cx.new(|cx| InputState::new(window, cx));
        let subscriptions =
            vec![
                cx.subscribe_in(&terminal_input, window, |view, input, event, window, cx| {
                    match event {
                        InputEvent::Change => {
                            let next = input.read(cx).value().to_string();
                            let common = view
                                .terminal_text
                                .chars()
                                .zip(next.chars())
                                .take_while(|(a, b)| a == b)
                                .count();
                            let mut echo =
                                "\x08 \x08".repeat(view.terminal_text.chars().count() - common);
                            echo.extend(
                                next.chars()
                                    .skip(common)
                                    .filter(|c| *c >= ' ' && *c != '\x7f'),
                            );
                            view.terminal_text = next;
                            view.write_terminal(&echo, cx);
                        }
                        InputEvent::PressEnter { .. } => {
                            view.terminal_text.clear();
                            input.update(cx, |input, cx| input.set_value("", window, cx));
                            view.write_terminal(&format!("\r\n{TERMINAL_PROMPT}"), cx);
                        }
                        _ => {}
                    }
                }),
            ];
        let light = !cx.theme().mode.is_dark();
        let initialize = cx.background_executor().spawn(async move {
            let mut terminal = t3_terminal::Terminal::new(90, 13, 8, 17, &crate::terminal_view::theme(light))?;
            terminal.write(format!("{TERMINAL_PROMPT}vpr dev\r\n\r\n  \x1b[1;32mVITE\x1b[0m \x1b[32mv7.1.1\x1b[0m  \x1b[2mready in\x1b[0m \x1b[1m1.24s\x1b[0m\r\n\r\n  \x1b[32m→\x1b[0m  \x1b[2mLocal:\x1b[0m    \x1b[4;36mhttp://127.0.0.1:5173/\x1b[0m\r\n  \x1b[32m→\x1b[0m  \x1b[2mNetwork:\x1b[0m  \x1b[4;36mhttp://192.168.1.24:5173/\x1b[0m\r\n\r\n  \x1b[32m✓ 85 passed\x1b[0m   \x1b[33m△ 2 warnings\x1b[0m   \x1b[31m✗ 0 failed\x1b[0m\r\n\r\n  \x1b[42;30m READY \x1b[0m \x1b[2mwatching for changes, press\x1b[0m \x1b[1mq\x1b[0m \x1b[2mto quit\x1b[0m\r\n\r\n{TERMINAL_PROMPT}").as_bytes())?;
            Ok::<_, anyhow::Error>(terminal)
        });
        cx.spawn(async move |this, cx| {
            let result = initialize.await;
            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok(terminal) => view.terminal = Some(terminal),
                    Err(error) => view.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        Self {
            root,
            key,
            prompt,
            terminal: None,
            terminal_input,
            terminal_text: String::new(),
            light,
            cache: Rc::new(RefCell::new(crate::terminal_paint::Cache::default())),
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn write_terminal(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(terminal) = &mut self.terminal
            && let Err(error) = terminal.write(text.as_bytes())
        {
            self.error = Some(error.to_string());
        }
        cx.notify();
    }
}

impl Render for FontPreviews {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = self.root.upgrade().expect("settings owner");
        let settings = &root.read(cx).settings;
        let separate = settings.values["gpuiSeparateFonts"] == true;
        let prompt = self.key == "fontSizePrompt" || self.key == "fontSizeInterface" && !separate;
        let code = self.key == "fontSizeCode";
        let terminal = self.key == "fontSizeTerminal" || code && !separate;
        let prompt_family = settings.values[if separate {
            "fontFamilyComposer"
        } else {
            "fontFamilySans"
        }]
        .as_str()
        .filter(|family| !family.is_empty())
        .unwrap_or(UI_FONT)
        .to_string();
        let prompt_size = if separate {
            settings.font_size_prompt
        } else {
            settings.font_size_interface
        };
        let code_family = settings.values["fontFamilyCode"]
            .as_str()
            .filter(|family| !family.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| crate::appearance::default_code_family(cx));
        let terminal_family = if separate {
            settings.values["fontFamilyTerminal"]
                .as_str()
                .filter(|family| !family.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| crate::appearance::default_code_family(cx))
        } else {
            code_family.clone()
        };
        let terminal_size = if separate {
            settings.values["fontSizeTerminal"].as_f64().unwrap_or(12.) as f32
        } else {
            settings.font_size_code
        };
        let code_size = settings.font_size_code;
        let typography = crate::terminal_paint::Typography::measure(
            terminal_family.into(),
            terminal_size,
            window,
        );
        let light = !cx.theme().mode.is_dark();
        let snapshot = self.terminal.as_mut().and_then(|terminal| {
            let result = (|| {
                if self.light != light {
                    terminal.set_theme(&crate::terminal_view::theme(light))?;
                }
                terminal.snapshot()
            })();
            match result {
                Ok(snapshot) => Some(snapshot),
                Err(error) => {
                    self.error = Some(error.to_string());
                    None
                }
            }
        });
        self.light = light;
        let cache = self.cache.clone();
        let focused = self
            .terminal_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        v_flex()
            .gap_2()
            .px_4()
            .pb_3()
            .when(prompt, |view| {
                view.child(
                    div()
                        .border_1()
                        .border_color(cx.theme().border)
                        .rounded_lg()
                        .bg(cx.theme().background)
                        .p_3()
                        .font_family(prompt_family)
                        .text_size(px(prompt_size))
                        .child(
                            Textarea::new(&self.prompt)
                                .aria_label("Prompt font preview")
                                .appearance(false)
                                .bordered(false),
                        ),
                )
            })
            .when(code, |view| {
                view.child(code_preview(code_family, code_size, cx))
            })
            .when(terminal, |view| {
                view.child(
                    div()
                        .id("terminal-font-preview")
                        .role(accesskit::Role::Group)
                        .aria_label("Terminal font preview")
                        .relative()
                        .h(px(typography.height * 13. + 8.))
                        .border_1()
                        .border_color(cx.theme().border)
                        .rounded_lg()
                        .overflow_hidden()
                        .child(
                            canvas(
                                |_, _, _| (),
                                move |bounds, _, window, cx| {
                                    if let Some(snapshot) = &snapshot {
                                        cache.borrow_mut().paint(
                                            snapshot,
                                            &typography,
                                            bounds,
                                            crate::terminal_paint::PaintState {
                                                selection: cx.theme().accent,
                                                cursor_on: true,
                                                focused,
                                            },
                                            window,
                                            cx,
                                        );
                                    }
                                },
                            )
                            .size_full(),
                        )
                        .child(
                            div().absolute().inset_0().opacity(0.).child(
                                Input::new(&self.terminal_input)
                                    .aria_label("Terminal input")
                                    .appearance(false)
                                    .size_full(),
                            ),
                        ),
                )
            })
            .when_some(self.error.clone(), |view, error| {
                view.child(settings_controls::text(error).text_color(cx.theme().danger))
            })
    }
}

fn code_preview(family: String, size: f32, cx: &App) -> impl IntoElement {
    let theme = cx.theme().colors;
    let lines: [(usize, i8, &[&str]); 4] = [
        (
            1,
            0,
            &[
                "export",
                " ",
                "function",
                " ",
                "formatUser",
                "(",
                "user",
                ":",
                " ",
                "User",
                ") {",
            ],
        ),
        (
            2,
            -1,
            &[
                "  ",
                "return",
                " ",
                "user",
                ".",
                "name",
                ".",
                "toUpperCase",
                "()",
                ";",
            ],
        ),
        (
            2,
            1,
            &[
                "  ",
                "return",
                " ",
                "`",
                "${",
                "user",
                ".",
                "name",
                "}",
                " ",
                "<",
                "${",
                "user",
                ".",
                "email",
                "}",
                ">`",
                ";",
                " ",
                "// 0O 1lI",
            ],
        ),
        (3, 0, &["}"]),
    ];
    v_flex()
        .id("code-font-preview")
        .rounded_lg()
        .overflow_hidden()
        .border_1()
        .border_color(theme.border)
        .font_family(family)
        .text_size(px(size))
        .bg(theme.background)
        .child(
            h_flex()
                .h(px(32.))
                .px_3()
                .gap_2()
                .child(settings_controls::text("src/formatUser.ts").flex_1())
                .child(settings_controls::text("-1").text_color(theme.danger))
                .child(settings_controls::text("+1").text_color(theme.success)),
        )
        .children(
            lines
                .into_iter()
                .enumerate()
                .map(|(line, (number, kind, tokens))| {
                    h_flex()
                        .id(("preview-line", line))
                        .h(px(size * 1.5))
                        .items_center()
                        .when(kind < 0, |row| row.bg(theme.danger.opacity(0.1)))
                        .when(kind > 0, |row| row.bg(theme.success.opacity(0.1)))
                        .child(
                            settings_controls::text(number.to_string())
                                .w(px(36.))
                                .pr_2()
                                .text_right()
                                .text_color(theme.muted_foreground),
                        )
                        .child(div().w(px(18.)).child(if kind < 0 {
                            "−"
                        } else if kind > 0 {
                            "+"
                        } else {
                            ""
                        }))
                        .children(tokens.iter().enumerate().map(|(index, token)| {
                            settings_controls::text(*token)
                                .id(("preview-token", index))
                                .text_color(match *token {
                                    "export" | "function" | "return" => theme.info,
                                    "formatUser" | "toUpperCase" => theme.primary,
                                    "User" => theme.warning,
                                    "// 0O 1lI" => theme.muted_foreground,
                                    _ => theme.foreground,
                                })
                        }))
                }),
        )
}
