use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;
use t3_client::session::Connection;
use t3_terminal::session::{Event, Grid, Input, Session, State, Target};
use t3_terminal::{Color, KeyboardEvent, Mouse as TerminalMouse, MouseAction, Theme};
use tokio::runtime::Handle;

use crate::terminal_paint::{Cache, PADDING, PaintState, Typography};

pub struct TerminalView {
    session: Session,
    pub state: State,
    pub focus: FocusHandle,
    visible: bool,
    focused: bool,
    cursor_on: bool,
    blink: Option<Task<()>>,
    _focus_subscriptions: Vec<Subscription>,
    _updates: Task<()>,
    _events: Task<()>,
    typography: Typography,
    cache: Rc<RefCell<Cache>>,
    bounds: Bounds<Pixels>,
    preedit: String,
    ime_selection: Range<usize>,
    drag: Option<t3_terminal::Point>,
    drag_pointer: Point<Pixels>,
    reporting_button: Option<i32>,
    selection_scroll: Option<Task<()>>,
    wheel_remainder: f32,
    light: bool,
    requested_grid: Grid,
}

#[derive(Clone)]
pub enum ViewEvent {
    Focus,
    Metadata,
}

impl EventEmitter<ViewEvent> for TerminalView {}

fn mouse_button(button: MouseButton) -> Option<i32> {
    match button {
        MouseButton::Left => Some(1),
        MouseButton::Middle => Some(3),
        MouseButton::Right => Some(2),
        MouseButton::Navigate(_) => None,
    }
}

fn rgb_color(value: u32) -> Color {
    Color {
        r: (value >> 16) as u8,
        g: (value >> 8) as u8,
        b: value as u8,
    }
}

pub fn theme(light: bool) -> Theme {
    let palette = if light {
        [
            0x000000, 0xcd3131, 0x107c10, 0x949800, 0x0451a5, 0xbc05bc, 0x0598bc, 0x555555,
            0x666666, 0xcd3131, 0x14ce14, 0xb5ba00, 0x0451a5, 0xbc05bc, 0x0598bc, 0xa5a5a5,
        ]
    } else {
        [
            0x000000, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5,
            0x666666, 0xf14c4c, 0x23d18b, 0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xe5e5e5,
        ]
    };
    Theme {
        foreground: rgb_color(if light { 0x27272a } else { 0xf5f5f5 }),
        background: rgb_color(if light { 0xfcfcfc } else { 0x000000 }),
        cursor: rgb_color(if light { 0x26384e } else { 0xb4cbff }),
        palette: Some(palette.into_iter().map(rgb_color).collect()),
    }
}

fn logical_key(keystroke: &Keystroke, raw: Option<&KeyboardInputContext>) -> String {
    if keystroke.key.starts_with("dead_") {
        return "Dead".into();
    }
    if let Some(text) = raw.and_then(|raw| raw.text.as_ref()) {
        return text.to_string();
    }
    match keystroke.key.as_str() {
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "home" => "Home",
        "end" => "End",
        "backspace" => "Backspace",
        "delete" => "Delete",
        "insert" => "Insert",
        "escape" => "Escape",
        "enter" => "Enter",
        "tab" => "Tab",
        "space" => " ",
        _ => {
            return keystroke
                .key_char
                .clone()
                .unwrap_or_else(|| keystroke.key.clone());
        }
    }
    .into()
}

impl TerminalView {
    pub fn new(
        connection: Arc<Connection>,
        target: Target,
        light: bool,
        font_size: f32,
        runtime: &Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let typography = Typography::measure("Liberation Mono".into(), font_size, window);
        let grid = Grid::new(
            80,
            24,
            typography.width.round() as u16,
            typography.height as u16,
        );
        let (session, mut updates, events) =
            Session::new(connection, target, grid, theme(light), runtime);
        let state = updates.borrow().clone();
        let focus = cx.focus_handle();
        let focus_subscriptions = vec![
            cx.observe_window_activation(window, |view, window, cx| {
                view.focused = view.focus.is_focused(window) && window.is_window_active();
                view.update_blink(cx);
                cx.notify();
            }),
            cx.on_focus(&focus, window, |view, _, cx| {
                view.focused = true;
                view.cursor_on = true;
                view.update_blink(cx);
                cx.emit(ViewEvent::Focus);
                cx.notify();
            }),
            cx.on_blur(&focus, window, |view, _, cx| {
                view.focused = false;
                view.cursor_on = true;
                view.blink = None;
                cx.notify();
            }),
        ];
        let update_task = cx.spawn(async move |this, cx| {
            while updates.changed().await.is_ok() {
                let state = updates.borrow_and_update().clone();
                if this
                    .update(cx, |view, cx| {
                        let metadata = view.state.label != state.label
                            || view.state.status != state.status
                            || view.state.error != state.error;
                        view.state = state;
                        view.cursor_on = true;
                        view.update_blink(cx);
                        if metadata {
                            cx.emit(ViewEvent::Metadata);
                        }
                        if view.visible {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let event_task = cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if this
                    .update(cx, |_, cx| match event {
                        Event::Copy(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            session,
            state,
            focus,
            visible: true,
            focused: false,
            cursor_on: true,
            blink: None,
            _focus_subscriptions: focus_subscriptions,
            _updates: update_task,
            _events: event_task,
            typography,
            cache: Rc::new(RefCell::new(Cache::default())),
            bounds: Bounds::default(),
            preedit: String::new(),
            ime_selection: 0..0,
            drag: None,
            drag_pointer: Point::default(),
            reporting_button: None,
            selection_scroll: None,
            wheel_remainder: 0.,
            light,
            requested_grid: grid,
        }
    }

    fn update_blink(&mut self, cx: &mut Context<Self>) {
        let enabled = self.visible
            && self.focused
            && !self.state.modes.synchronized_output
            && self
                .state
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.cursor_blinking && snapshot.cursor_visible);
        if !enabled {
            self.blink = None;
            self.cursor_on = true;
            return;
        }
        if self.blink.is_none() {
            self.blink = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(500))
                        .await;
                    if this
                        .update(cx, |view, cx| {
                            view.cursor_on = !view.cursor_on;
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }
    }

    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        self.input(Input::Visible(visible), cx);
        self.update_blink(cx);
        if visible {
            cx.notify();
        }
    }

    pub fn configure(
        &mut self,
        light: bool,
        size: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.light != light {
            self.light = light;
            self.input(Input::Theme(theme(light)), cx);
        }
        if self.typography.size != size || self.typography.scale != window.scale_factor() {
            self.typography = Typography::measure(self.typography.family.clone(), size, window);
            self.resize(self.bounds, cx);
            cx.notify();
        }
    }

    pub fn input(&mut self, input: Input, cx: &mut Context<Self>) {
        if let Err(error) = self.session.input(input) {
            self.state.error = Some(error.to_string());
            cx.notify();
        }
    }

    fn resize(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        self.bounds = bounds;
        let cols = ((f32::from(bounds.size.width) - PADDING * 2.) / self.typography.width)
            .floor()
            .clamp(1., 1000.) as u16;
        let rows = ((f32::from(bounds.size.height) - PADDING * 2.) / self.typography.height)
            .floor()
            .clamp(1., 500.) as u16;
        let grid = Grid::new(
            cols,
            rows,
            self.typography.width.round() as u16,
            self.typography.height as u16,
        );
        if grid != self.requested_grid {
            self.requested_grid = grid;
            self.input(Input::Resize(grid), cx);
        }
    }

    fn key(
        &mut self,
        keystroke: &Keystroke,
        held: bool,
        release: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let raw = window.current_keyboard_input_context();
        let modifiers = raw.map_or(keystroke.modifiers, |raw| raw.modifiers);
        let mut event = KeyboardEvent {
            key: logical_key(keystroke, raw),
            code: raw
                .map(|raw| raw.physical_key.to_string())
                .unwrap_or_else(|| logical_key(keystroke, None)),
            shift: modifiers.shift,
            control: modifiers.control,
            alt: modifiers.alt,
            meta: modifiers.platform,
            caps_lock: raw.is_some_and(|raw| raw.caps_lock),
            num_lock: raw.is_some_and(|raw| raw.num_lock),
            mac: cfg!(target_os = "macos"),
            repeat: held,
            release,
            composing: !self.preedit.is_empty(),
            layout_character: raw
                .and_then(|raw| {
                    char::from_u32(raw.unshifted_codepoint).filter(|character| *character != '\0')
                })
                .map(|character| character.to_string()),
        };
        event.composing |= matches!(event.key.as_str(), "Dead" | "Process");
        if (!release && raw.is_some_and(|raw| raw.alt_graph)) || event.composing {
            if release || !held {
                self.input(Input::ForgetKey(event.code), cx);
            }
            return;
        }
        if !release && event.is_paste() {
            self.input(Input::ForgetKey(event.code), cx);
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.input(Input::Paste(text), cx);
            }
        } else {
            self.input(
                Input::KeyWithFallback(event, keystroke.key_char.clone()),
                cx,
            );
        }
        cx.stop_propagation();
    }

    fn cell(&self, position: Point<Pixels>) -> t3_terminal::Point {
        t3_terminal::Point {
            x: ((f32::from(position.x - self.bounds.origin.x) - PADDING) / self.typography.width)
                .floor()
                .clamp(0., f32::from(self.state.grid.cols - 1)) as u32,
            y: ((f32::from(position.y - self.bounds.origin.y) - PADDING) / self.typography.height)
                .floor()
                .clamp(0., f32::from(self.state.grid.rows - 1)) as u32,
        }
    }

    fn reports_mouse(&self, modifiers: Modifiers) -> bool {
        self.state.modes.mouse_tracking
            && !modifiers.shift
            && !modifiers.control
            && !modifiers.platform
    }

    fn drag_rows(&self) -> i32 {
        let y = f32::from(self.drag_pointer.y);
        let top = f32::from(self.bounds.top()) + PADDING;
        let bottom = f32::from(self.bounds.bottom()) - PADDING;
        let distance = if y < top {
            y - top
        } else if y > bottom {
            y - bottom
        } else {
            0.
        };
        if distance == 0. {
            return 0;
        }
        let ramp = (distance / 50.).clamp(-1., 1.);
        ramp.signum() as i32 + (ramp * 14.).round() as i32
    }

    fn extend_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.drag.is_none() {
            return;
        }
        self.drag_pointer = position;
        let end = self.cell(position);
        if self.drag != Some(end) {
            self.drag = Some(end);
            self.input(Input::ExtendSelection(end), cx);
        }
        if self.drag_rows() == 0 {
            self.selection_scroll = None;
        } else if self.selection_scroll.is_none() {
            self.selection_scroll = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(80))
                        .await;
                    if this
                        .update(cx, |view, cx| {
                            if view.drag.is_some() {
                                view.input(
                                    Input::ScrollSelection(
                                        view.drag_rows(),
                                        view.cell(view.drag_pointer),
                                    ),
                                    cx,
                                );
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            self.input(Input::EndSelection, cx);
        }
        self.selection_scroll = None;
    }

    fn mouse(
        &mut self,
        action: MouseAction,
        button: Option<i32>,
        position: Point<Pixels>,
        modifiers: Modifiers,
        pressed: bool,
        cx: &mut Context<Self>,
    ) {
        match action {
            MouseAction::Press if matches!(button, Some(1..=3)) => self.reporting_button = button,
            MouseAction::Release => {
                if self.reporting_button != button {
                    return;
                }
                self.reporting_button = None;
            }
            _ => {}
        }
        self.input(
            Input::Mouse(TerminalMouse {
                action,
                button,
                mods: i32::from(modifiers.shift)
                    | (i32::from(modifiers.control) << 1)
                    | (i32::from(modifiers.alt) << 2)
                    | (i32::from(modifiers.platform) << 3),
                x: f32::from(position.x - self.bounds.origin.x).max(0.),
                y: f32::from(position.y - self.bounds.origin.y).max(0.),
                screen_width: f32::from(self.bounds.size.width),
                screen_height: f32::from(self.bounds.size.height),
                cell_width: self.typography.width,
                cell_height: self.typography.height,
                padding_left: PADDING,
                padding_right: PADDING,
                padding_top: PADDING,
                padding_bottom: (f32::from(self.bounds.size.height)
                    - PADDING
                    - f32::from(self.state.grid.rows) * self.typography.height)
                    .max(0.),
                any_button_pressed: pressed,
            }),
            cx,
        );
    }

    fn cursor_bounds(&self) -> Bounds<Pixels> {
        let (x, y) = self.state.snapshot.as_ref().map_or((0, 0), |snapshot| {
            (snapshot.cursor_x.max(0), snapshot.cursor_y.max(0))
        });
        Bounds::new(
            self.bounds.origin
                + point(
                    px(PADDING + x as f32 * self.typography.width),
                    px(PADDING + y as f32 * self.typography.height),
                ),
            size(px(1.), px(self.typography.height)),
        )
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let weak = view.downgrade();
        let focus = self.focus.clone();
        let snapshot = self.state.snapshot.clone();
        let typography = self.typography.clone();
        let cache = self.cache.clone();
        let cursor_on = self.cursor_on;
        let focused = self.focused && window.is_window_active();
        let selection: Hsla = if self.light {
            rgba(0x253f6333)
        } else {
            rgba(0xb4cbff40)
        }
        .into();
        let label = if self.state.label.is_empty() {
            "Terminal".to_owned()
        } else {
            self.state.label.clone()
        };
        let text = snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .row_data
                    .iter()
                    .map(|row| row.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let foreground = cx.theme().foreground;
        div()
            .id("terminal-surface")
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .track_focus(&self.focus)
            .key_context("Terminal")
            .role(accesskit::Role::Terminal)
            .aria_label(label)
            .a11y_synthetic_children(move |builder| {
                let mut run = accesskit::Node::new(accesskit::Role::TextRun);
                run.set_value(text.clone());
                run.set_character_lengths(
                    text.chars()
                        .map(|character| character.len_utf8() as u8)
                        .collect::<Vec<_>>(),
                );
                let id = builder.synthetic_node_id("terminal-text");
                builder.push_child(id, run);
            })
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                view.key(&event.keystroke, event.is_held, false, window, cx)
            }))
            .on_key_up(cx.listener(|view, event: &KeyUpEvent, window, cx| {
                view.key(&event.keystroke, false, true, window, cx)
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &MouseDownEvent, window, cx| {
                    window.focus(&view.focus, cx);
                    if view.reports_mouse(event.modifiers) {
                        view.mouse(
                            MouseAction::Press,
                            Some(1),
                            event.position,
                            event.modifiers,
                            true,
                            cx,
                        );
                        cx.stop_propagation();
                        return;
                    }
                    let point = view.cell(event.position);
                    view.drag = Some(point);
                    view.drag_pointer = event.position;
                    view.input(Input::BeginSelection(point, event.click_count), cx);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, _, cx| {
                if view.reporting_button.is_some()
                    || (event.pressed_button.is_none()
                        && view.state.modes.mouse_any_event
                        && view.reports_mouse(event.modifiers))
                {
                    view.mouse(
                        MouseAction::Motion,
                        event.pressed_button.and_then(mouse_button),
                        event.position,
                        event.modifiers,
                        event.pressed_button.is_some(),
                        cx,
                    );
                    return;
                }
                if event.dragging() {
                    view.extend_drag(event.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, event: &MouseUpEvent, _, cx| {
                    if view.reporting_button.is_some() {
                        view.mouse(
                            MouseAction::Release,
                            Some(1),
                            event.position,
                            event.modifiers,
                            false,
                            cx,
                        );
                    }
                    view.end_drag(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, event: &MouseUpEvent, _, cx| {
                    if view.reporting_button.is_some() {
                        view.mouse(
                            MouseAction::Release,
                            Some(1),
                            event.position,
                            event.modifiers,
                            false,
                            cx,
                        );
                    }
                    view.end_drag(cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|view, event: &MouseDownEvent, window, cx| {
                    window.focus(&view.focus, cx);
                    if view.reports_mouse(event.modifiers) {
                        view.mouse(
                            MouseAction::Press,
                            Some(3),
                            event.position,
                            event.modifiers,
                            true,
                            cx,
                        );
                        cx.stop_propagation();
                        return;
                    }
                    if let Some(text) = cx.read_from_primary().and_then(|item| item.text()) {
                        view.input(Input::Paste(text), cx);
                    }
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|view, event: &MouseDownEvent, window, cx| {
                    window.focus(&view.focus, cx);
                    if view.reports_mouse(event.modifiers) {
                        view.mouse(
                            MouseAction::Press,
                            Some(2),
                            event.position,
                            event.modifiers,
                            true,
                            cx,
                        );
                        cx.stop_propagation();
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|view, event: &MouseUpEvent, _, cx| {
                    if view.reporting_button.is_some() {
                        view.mouse(
                            MouseAction::Release,
                            Some(3),
                            event.position,
                            event.modifiers,
                            false,
                            cx,
                        );
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|view, event: &MouseUpEvent, _, cx| {
                    if view.reporting_button.is_some() {
                        view.mouse(
                            MouseAction::Release,
                            Some(2),
                            event.position,
                            event.modifiers,
                            false,
                            cx,
                        );
                    }
                }),
            )
            .on_scroll(cx.listener(|view, event: &ScrollEvent, _, cx| {
                let delta = match event.delta {
                    ScrollDelta::Pixels(delta) => -f32::from(delta.y) / view.typography.height,
                    ScrollDelta::Lines(delta) => -delta.y,
                };
                view.wheel_remainder += delta;
                let rows = view.wheel_remainder.trunc() as i32;
                view.wheel_remainder -= rows as f32;
                if rows != 0 {
                    if view.reports_mouse(event.modifiers) {
                        let button = if rows < 0 { 4 } else { 5 };
                        for _ in 0..rows.unsigned_abs() {
                            view.mouse(
                                MouseAction::Press,
                                Some(button),
                                event.position,
                                event.modifiers,
                                false,
                                cx,
                            );
                        }
                    } else if view.state.modes.alternate_screen {
                        let sequence = if rows < 0 {
                            if view.state.modes.application_cursor_keys {
                                "\u{1b}OA"
                            } else {
                                "\u{1b}[A"
                            }
                        } else if view.state.modes.application_cursor_keys {
                            "\u{1b}OB"
                        } else {
                            "\u{1b}[B"
                        };
                        view.input(
                            Input::Text(sequence.repeat(rows.unsigned_abs() as usize)),
                            cx,
                        );
                    } else {
                        view.input(Input::Scroll(rows), cx);
                    }
                }
                cx.stop_propagation();
            }))
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let _ = weak.update(cx, |view, cx| view.resize(bounds, cx));
                    },
                    move |bounds, _, window, cx| {
                        let drag_view = view.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble
                                && event.dragging()
                                && !bounds.contains(&event.position)
                            {
                                drag_view.update(cx, |view, cx| {
                                    if let Some(button) = view.reporting_button {
                                        view.mouse(
                                            MouseAction::Motion,
                                            Some(button),
                                            event.position,
                                            event.modifiers,
                                            true,
                                            cx,
                                        );
                                    } else {
                                        view.extend_drag(event.position, cx);
                                    }
                                });
                            }
                        });
                        let release_view = view.clone();
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble && !bounds.contains(&event.position) {
                                release_view.update(cx, |view, cx| {
                                    if let Some(button) = view.reporting_button {
                                        view.mouse(
                                            MouseAction::Release,
                                            Some(button),
                                            event.position,
                                            event.modifiers,
                                            false,
                                            cx,
                                        );
                                    }
                                    view.end_drag(cx);
                                });
                            }
                        });
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, view.clone()),
                            cx,
                        );
                        if let Some(snapshot) = &snapshot {
                            cache.borrow_mut().paint(
                                snapshot,
                                &typography,
                                bounds,
                                PaintState {
                                    selection,
                                    cursor_on,
                                    focused,
                                },
                                window,
                                cx,
                            );
                        } else {
                            window.paint_quad(fill(bounds, cx.theme().background));
                        }
                    },
                )
                .size_full(),
            )
            .children(self.state.error.as_ref().map(|error| {
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .p_1()
                    .text_xs()
                    .text_color(foreground)
                    .bg(cx.theme().background)
                    .child(error.clone())
            }))
    }
}

fn utf8_offset(text: &str, utf16: usize) -> usize {
    let mut offset = 0;
    let mut units = 0;
    for character in text.chars() {
        if units + character.len_utf16() > utf16 {
            break;
        }
        units += character.len_utf16();
        offset += character.len_utf8();
    }
    offset
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let start = utf8_offset(&self.preedit, range.start);
        let end = utf8_offset(&self.preedit, range.end.max(range.start));
        *adjusted = Some(
            self.preedit[..start].encode_utf16().count()
                ..self.preedit[..end].encode_utf16().count(),
        );
        Some(self.preedit[start..end].to_owned())
    }
    fn selected_text_range(
        &mut self,
        ignore_disabled: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        (!self.session.target.read_only || ignore_disabled).then(|| UTF16Selection {
            range: self.ime_selection.clone(),
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.preedit.is_empty()).then(|| 0..self.preedit.encode_utf16().count())
    }
    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.preedit.clear();
        self.ime_selection = 0..0;
        window.notify_text_input_changed(&self.focus);
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !text.is_empty() {
            self.input(Input::Text(text.to_owned()), cx);
        }
        self.preedit.clear();
        self.ime_selection = 0..0;
        window.notify_text_input_changed(&self.focus);
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range.unwrap_or(0..self.preedit.encode_utf16().count());
        let start = utf8_offset(&self.preedit, range.start);
        let end = utf8_offset(&self.preedit, range.end.max(range.start));
        self.preedit.replace_range(start..end, text);
        let length = self.preedit.encode_utf16().count();
        self.ime_selection = selected
            .map(|range| range.start.min(length)..range.end.min(length))
            .unwrap_or(length..length);
        window.notify_text_input_changed(&self.focus);
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(self.cursor_bounds())
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.ime_selection.end)
    }
    fn set_selected_text_range(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        let length = self.preedit.encode_utf16().count();
        self.ime_selection = range.start.min(length)..range.end.min(length);
        window.notify_text_selection_changed(&self.focus);
    }
    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.preedit.encode_utf16().count())
    }
    fn accepts_text_input(&self, _: &mut Window, _: &mut Context<Self>) -> bool {
        !self.session.target.read_only
    }
    fn paste(&mut self, item: ClipboardItem, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = item.text() {
            self.input(Input::Paste(text), cx);
        }
    }
}
