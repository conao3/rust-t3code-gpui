use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

pub enum Event {
    Select(String),
    Dismiss,
}

pub fn presentation(mode: &str) -> (&'static str, &'static str, IconName) {
    match mode {
        "approval-required" => (
            "Supervised",
            "Ask before commands and file changes.",
            IconName::Lock,
        ),
        "auto-accept-edits" => (
            "Auto-accept edits",
            "Auto-approve edits, ask before other actions.",
            IconName::PenLine,
        ),
        "auto" => (
            "Auto",
            "Supported providers approve routine actions; others still ask.",
            IconName::ShieldCheck,
        ),
        "full-access" => (
            "Full access",
            "Allow commands and edits without prompts.",
            IconName::ShieldOff,
        ),
        _ => (
            "Supervised",
            "Ask before commands and file changes.",
            IconName::Lock,
        ),
    }
}

pub struct Menu {
    modes: Vec<String>,
    current: String,
    preview: Option<usize>,
    focused: Option<usize>,
    focus: FocusHandle,
    buttons: Vec<FocusHandle>,
    _focus_subscriptions: Vec<Subscription>,
}

impl EventEmitter<Event> for Menu {}

impl Menu {
    pub fn new(
        modes: Vec<String>,
        current: String,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focused = keyboard.then(|| modes.iter().position(|mode| mode == &current).unwrap_or(0));
        let buttons: Vec<FocusHandle> = modes.iter().map(|_| cx.focus_handle()).collect();
        let focus_subscriptions = buttons
            .iter()
            .enumerate()
            .map(|(index, button)| {
                cx.on_focus(button, window, move |menu, _, cx| {
                    menu.focused = Some(index);
                    menu.preview = Some(index);
                    cx.notify();
                })
            })
            .collect();
        Self {
            modes,
            current,
            preview: None,
            focused,
            focus: cx.focus_handle(),
            buttons,
            _focus_subscriptions: focus_subscriptions,
        }
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.modes.is_empty() {
            return;
        }
        let next = match event.keystroke.key.as_str() {
            "escape" => {
                cx.emit(Event::Dismiss);
                None
            }
            "enter" | "space" => {
                if let Some(index) = self.focused {
                    cx.emit(Event::Select(self.modes[index].clone()));
                }
                None
            }
            "left" => self.focused.map(|index| index.saturating_sub(1)),
            "right" => self
                .focused
                .map(|index| (index + 1).min(self.modes.len() - 1)),
            "up" => Some(self.focused.map_or(self.modes.len() - 1, |index| {
                (index + self.modes.len() - 1) % self.modes.len()
            })),
            "down" => Some(
                self.focused
                    .map_or(0, |index| (index + 1) % self.modes.len()),
            ),
            _ => return,
        };
        if let Some(index) = next {
            self.focused = Some(index);
            self.preview = Some(index);
            self.buttons[index].focus(window, cx);
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Focusable for Menu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.buttons
            .get(self.focused.unwrap_or(usize::MAX))
            .unwrap_or(&self.focus)
            .clone()
    }
}

impl Render for Menu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let shown = self
            .preview
            .and_then(|index| self.modes.get(index))
            .unwrap_or(&self.current);
        let (label, description, _) = presentation(shown);
        v_flex()
            .id("permissions-menu")
            .role(accesskit::Role::Menu)
            .aria_label(format!("Permissions: {}", presentation(&self.current).0))
            .track_focus(&self.focus)
            .w(px(298.))
            .p_1()
            .rounded(px(8.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .shadow_lg()
            .occlude()
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(Event::Dismiss)))
            .child(
                v_flex()
                    .w_full()
                    .px_2()
                    .py_1p5()
                    .gap_1p5()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.muted_foreground)
                            .child("Permissions"),
                    )
                    .child(
                        h_flex()
                            .id("permission-modes")
                            .role(accesskit::Role::RadioGroup)
                            .aria_label("Permissions")
                            .w_full()
                            .p_0p5()
                            .rounded(px(8.))
                            .bg(theme.input.opacity(0.4))
                            .children(self.modes.iter().enumerate().map(|(index, mode)| {
                                let (label, description, icon) = presentation(mode);
                                let selected = mode == &self.current;
                                let value = mode.clone();
                                h_flex()
                                    .id(("permission-mode", index))
                                    .role(accesskit::Role::MenuItemRadio)
                                    .aria_label(label)
                                    .aria_description(description)
                                    .aria_toggled(if selected {
                                        accesskit::Toggled::True
                                    } else {
                                        accesskit::Toggled::False
                                    })
                                    .justify_center()
                                    .cursor_pointer()
                                    .tab_index(0)
                                    .border_2()
                                    .border_color(transparent_black())
                                    .focus_visible(|style| style.border_color(theme.ring))
                                    .track_focus(&self.buttons[index])
                                    .h(px(24.))
                                    .flex_1()
                                    .rounded(px(6.))
                                    .text_color(if selected {
                                        theme.foreground
                                    } else {
                                        theme.muted_foreground
                                    })
                                    .when(selected, |button| button.bg(theme.input.opacity(0.72)))
                                    .when(!selected, |button| {
                                        button.hover(|style| {
                                            style
                                                .bg(theme.foreground.opacity(0.04))
                                                .text_color(theme.foreground)
                                        })
                                    })
                                    .on_hover(cx.listener(move |menu, hovered: &bool, _, cx| {
                                        menu.preview = hovered.then_some(index);
                                        cx.notify();
                                    }))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |menu, _, window, cx| {
                                            menu.focused = Some(index);
                                            menu.buttons[index].focus(window, cx);
                                        }),
                                    )
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(Event::Select(value.clone()))
                                    }))
                                    .child(Icon::new(icon).size(px(14.)))
                            })),
                    )
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(if shown == &self.current {
                                        theme.foreground
                                    } else {
                                        theme.muted_foreground
                                    })
                                    .child(label),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .line_height(px(16.))
                                    .min_h(px(32.))
                                    .text_color(theme.muted_foreground)
                                    .child(description),
                            ),
                    ),
            )
    }
}
