use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui_kit::component::{
    ActiveTheme as _, Theme,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{InputEvent, Textarea, TextareaState},
    text::{TextView, TextViewState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use t3_client::config::Config;
use t3_client::session::{Change, Connection, ConnectionStatus, ThreadWatch};
use tokio::runtime::{Handle, Runtime};

use crate::sidebar::{self, ThreadEntry, ThreadStatus};
use crate::timeline::{self, Row, RowKind};

const REDRAW_INTERVAL: Duration = Duration::from_millis(250);
const UI_FONT: &str = "Noto Sans CJK JP";

actions!(t3code_gpui, [Send, NextDone]);

fn now_iso() -> String {
    humantime::format_rfc3339_millis(SystemTime::now()).to_string()
}

async fn on_runtime<T: std::marker::Send + 'static>(
    runtime: &Handle,
    future: impl std::future::Future<Output = anyhow::Result<T>> + std::marker::Send + 'static,
) -> anyhow::Result<T> {
    runtime.spawn(future).await?
}

struct RowView {
    row: Row,
    markdown: Option<Entity<TextViewState>>,
}

impl RowView {
    fn new(row: Row, cx: &mut Context<Self>) -> Self {
        let markdown = (row.kind == RowKind::Assistant)
            .then(|| cx.new(|cx| TextViewState::markdown(&row.text, cx)));
        Self { row, markdown }
    }

    fn update(&mut self, row: Row, cx: &mut Context<Self>) {
        if row == self.row {
            return;
        }
        if let Some(markdown) = &self.markdown {
            markdown.update(cx, |state, cx| state.set_text(&row.text, cx));
        }
        self.row = row;
        cx.notify();
    }
}

impl Render for RowView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let base = div().w_full().px_4();
        match self.row.kind {
            RowKind::Assistant => match &self.markdown {
                Some(state) => base
                    .py_1()
                    .child(TextView::new(state).selectable(true))
                    .into_any_element(),
                None => base.into_any_element(),
            },
            RowKind::User => base
                .py_2()
                .child(
                    div()
                        .p_3()
                        .rounded_md()
                        .bg(theme.secondary)
                        .child(self.row.text.clone()),
                )
                .into_any_element(),
            RowKind::Reasoning => base
                .py_1()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(format!("Thinking: {}", self.row.text))
                .into_any_element(),
            RowKind::Tool | RowKind::Notice => base
                .py_0p5()
                .text_sm()
                .font_family(theme.mono_font_family.clone())
                .text_color(theme.muted_foreground)
                .child(self.row.text.clone())
                .into_any_element(),
            RowKind::Request => base
                .py_1()
                .child(
                    div()
                        .p_2()
                        .rounded_md()
                        .border_1()
                        .border_color(if self.row.live {
                            theme.warning
                        } else {
                            theme.border
                        })
                        .child(self.row.text.clone()),
                )
                .into_any_element(),
            RowKind::Error => base
                .py_1()
                .text_color(theme.danger)
                .child(self.row.text.clone())
                .into_any_element(),
        }
    }
}

struct OpenThread {
    environment: String,
    watch: ThreadWatch,
}

pub struct Root {
    runtime: Handle,
    connections: Vec<Arc<Connection>>,
    entries: Vec<ThreadEntry>,
    open: Option<OpenThread>,
    rows: Vec<Row>,
    row_views: Vec<Entity<RowView>>,
    list_state: ListState,
    composer: Entity<TextareaState>,
    message: Option<String>,
    shell_dirty: bool,
    thread_dirty: bool,
    redraw_scheduled: bool,
    focus: FocusHandle,
    _shell_tasks: Vec<tokio::task::JoinHandle<()>>,
    _subscriptions: Vec<Subscription>,
    _changes: Task<()>,
}

impl Root {
    fn new(config: Config, runtime: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (sender, receiver) = async_channel::unbounded::<Change>();
        let connections: Vec<Arc<Connection>> = config
            .environments
            .iter()
            .map(|environment| Connection::new(environment, sender.clone()))
            .collect();
        let shell_tasks = connections
            .iter()
            .map(|connection| connection.spawn_shell(&runtime))
            .collect();
        let changes = cx.spawn(async move |this, cx| {
            while let Ok(change) = receiver.recv().await {
                if this
                    .update(cx, |root, cx| root.on_change(change, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let composer = cx.new(|cx| {
            let mut state =
                TextareaState::new(window, cx).placeholder("Message (Ctrl+Enter to send)");
            state.set_auto_grow(2, 12, cx);
            state
        });
        let subscriptions = vec![cx.subscribe_in(
            &composer,
            window,
            |root, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter {
                    secondary: true, ..
                } = event
                {
                    root.send(window, cx);
                }
            },
        )];
        let list_state = ListState::new(0, ListAlignment::Bottom, px(800.));
        list_state.set_follow_mode(FollowMode::Tail);
        Self {
            runtime,
            connections,
            entries: Vec::new(),
            open: None,
            rows: Vec::new(),
            row_views: Vec::new(),
            list_state,
            composer,
            message: None,
            shell_dirty: false,
            thread_dirty: false,
            redraw_scheduled: false,
            focus: cx.focus_handle(),
            _shell_tasks: shell_tasks,
            _subscriptions: subscriptions,
            _changes: changes,
        }
    }

    fn connection(&self, label: &str) -> Option<Arc<Connection>> {
        self.connections
            .iter()
            .find(|c| c.label() == label)
            .cloned()
    }

    fn on_change(&mut self, change: Change, cx: &mut Context<Self>) {
        match change {
            Change::Status { .. } | Change::Shell { .. } => self.shell_dirty = true,
            Change::Thread {
                environment,
                thread_id,
            } => {
                let open = self.open.as_ref().is_some_and(|open| {
                    open.environment == environment && open.watch.thread_id() == thread_id
                });
                if open {
                    self.thread_dirty = true;
                }
            }
        }
        if !self.redraw_scheduled && (self.shell_dirty || self.thread_dirty) {
            self.redraw_scheduled = true;
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(REDRAW_INTERVAL).await;
                let _ = this.update(cx, |root, cx| root.refresh(cx));
            })
            .detach();
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.redraw_scheduled = false;
        if std::mem::take(&mut self.shell_dirty) {
            self.rebuild_entries();
            self.visit_if_done();
            cx.notify();
        }
        if std::mem::take(&mut self.thread_dirty) {
            self.rebuild_rows(cx);
        }
    }

    fn rebuild_entries(&mut self) {
        let mut entries = Vec::new();
        for connection in &self.connections {
            let shell = connection.shell();
            let shell = shell.lock().expect("shell lock");
            entries.extend(sidebar::entries(connection.label(), &shell));
        }
        sidebar::sort_entries(&mut entries);
        self.entries = entries;
    }

    fn visit_if_done(&self) {
        let Some(open) = &self.open else { return };
        let done = self.entries.iter().any(|e| {
            e.environment == open.environment
                && e.thread_id == open.watch.thread_id()
                && e.status == ThreadStatus::Done
        });
        if !done {
            return;
        }
        if let Some(connection) = self.connection(&open.environment) {
            let thread_id = open.watch.thread_id().to_string();
            self.runtime.spawn(async move {
                let _ = connection.visit_thread(&thread_id, &now_iso()).await;
            });
        }
    }

    fn rebuild_rows(&mut self, cx: &mut Context<Self>) {
        let Some(open) = &self.open else { return };
        let rows = {
            let state = open.watch.state();
            let state = state.lock().expect("thread lock");
            timeline::rows(&state)
        };
        let mut existing: HashMap<String, Entity<RowView>> = self
            .rows
            .iter()
            .map(|r| r.key.clone())
            .zip(self.row_views.iter().cloned())
            .collect();
        let old_keys: Vec<String> = self.rows.iter().map(|r| r.key.clone()).collect();
        let views: Vec<Entity<RowView>> = rows
            .iter()
            .map(|row| match existing.remove(&row.key) {
                Some(view) => {
                    view.update(cx, |view, cx| view.update(row.clone(), cx));
                    view
                }
                None => {
                    let row = row.clone();
                    cx.new(|cx| RowView::new(row, cx))
                }
            })
            .collect();
        let appended = rows.len() >= old_keys.len()
            && old_keys.iter().zip(&rows).all(|(old, new)| *old == new.key);
        if appended {
            let old = old_keys.len();
            if rows.len() > old {
                self.list_state.splice(old..old, rows.len() - old);
            }
        } else {
            self.list_state.reset(rows.len());
        }
        self.rows = rows;
        self.row_views = views;
        cx.notify();
    }

    fn open_thread(&mut self, environment: String, thread_id: String, cx: &mut Context<Self>) {
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let watch = connection.watch_thread(&self.runtime, &thread_id);
        self.open = Some(OpenThread { environment, watch });
        self.rows.clear();
        self.row_views.clear();
        self.list_state.reset(0);
        self.message = None;
        self.runtime.spawn({
            let thread_id = thread_id.clone();
            async move {
                let _ = connection.visit_thread(&thread_id, &now_iso()).await;
            }
        });
        cx.notify();
    }

    fn open_entry(&self) -> Option<&ThreadEntry> {
        let open = self.open.as_ref()?;
        self.entries
            .iter()
            .find(|e| e.environment == open.environment && e.thread_id == open.watch.thread_id())
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &self.open else { return };
        let text = self.composer.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(connection) = self.connection(&open.environment) else {
            return;
        };
        let thread_id = open.watch.thread_id().to_string();
        let queue = self
            .open_entry()
            .is_some_and(|e| matches!(e.status, ThreadStatus::Working | ThreadStatus::Input));
        let runtime = self.runtime.clone();
        self.composer
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.spawn(async move |this, cx| {
            let sent = on_runtime(&runtime, async move {
                connection.send_message(&thread_id, &text, queue).await
            })
            .await;
            let _ = this.update(cx, |root, cx| {
                root.message = sent.err().map(|e| format!("Send failed: {e:#}"));
                cx.notify();
            });
        })
        .detach();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        let Some(open) = &self.open else { return };
        let Some(connection) = self.connection(&open.environment) else {
            return;
        };
        let thread_id = open.watch.thread_id().to_string();
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let stopped = on_runtime(
                &runtime,
                async move { connection.stop_thread(&thread_id).await },
            )
            .await;
            let _ = this.update(cx, |root, cx| {
                root.message = stopped.err().map(|e| format!("Stop failed: {e:#}"));
                cx.notify();
            });
        })
        .detach();
    }

    fn open_in_browser(&mut self, cx: &mut Context<Self>) {
        let Some(open) = &self.open else { return };
        let url = self
            .connection(&open.environment)
            .and_then(|c| c.thread_url(open.watch.thread_id()));
        match url {
            Some(url) => cx.open_url(&url),
            None => {
                self.message = Some("The environment id is not known yet.".into());
                cx.notify();
            }
        }
    }

    fn next_done(&mut self, cx: &mut Context<Self>) {
        let current = self
            .open
            .as_ref()
            .map(|o| (o.environment.clone(), o.watch.thread_id().to_string()));
        let wanted = |e: &ThreadEntry| matches!(e.status, ThreadStatus::Done | ThreadStatus::Input);
        let start = current
            .as_ref()
            .and_then(|(env, id)| {
                self.entries
                    .iter()
                    .position(|e| &e.environment == env && &e.thread_id == id)
            })
            .map_or(0, |i| i + 1);
        let n = self.entries.len();
        let next = (0..n)
            .map(|k| &self.entries[(start + k) % n.max(1)])
            .find(|e| wanted(e) && Some((e.environment.clone(), e.thread_id.clone())) != current)
            .map(|e| (e.environment.clone(), e.thread_id.clone()));
        if let Some((environment, thread_id)) = next {
            self.open_thread(environment, thread_id, cx);
        }
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let statuses = v_flex()
            .gap_0p5()
            .p_2()
            .children(self.connections.iter().map(|c| {
                let (color, text) = match c.status() {
                    ConnectionStatus::Connected => (theme.success, "connected".to_string()),
                    ConnectionStatus::Connecting => {
                        (theme.muted_foreground, "connecting".to_string())
                    }
                    ConnectionStatus::Disconnected(reason) => (theme.danger, reason),
                };
                h_flex()
                    .gap_2()
                    .text_xs()
                    .child(div().size_2().rounded_full().bg(color))
                    .child(c.label().to_string())
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(text),
                    )
            }));
        let open = self
            .open
            .as_ref()
            .map(|o| (o.environment.clone(), o.watch.thread_id().to_string()));
        let rows = self.entries.iter().enumerate().map(|(i, entry)| {
            let selected = open
                .as_ref()
                .is_some_and(|(env, id)| *env == entry.environment && *id == entry.thread_id);
            let dot = match entry.status {
                ThreadStatus::Input => Some(theme.warning),
                ThreadStatus::Working => Some(theme.primary),
                ThreadStatus::Failed => Some(theme.danger),
                ThreadStatus::Done => Some(theme.success),
                ThreadStatus::Idle | ThreadStatus::Settled => None,
            };
            let environment = entry.environment.clone();
            let thread_id = entry.thread_id.clone();
            h_flex()
                .id(("thread", i))
                .w_full()
                .px_2()
                .py_1()
                .gap_2()
                .rounded_md()
                .when(selected, |row| row.bg(theme.list_active))
                .when(entry.status == ThreadStatus::Settled, |row| {
                    row.text_color(theme.muted_foreground)
                })
                .hover(|row| row.bg(theme.secondary))
                .on_click(cx.listener(move |root, _, _, cx| {
                    root.open_thread(environment.clone(), thread_id.clone(), cx)
                }))
                .child(
                    div()
                        .flex_none()
                        .px_1()
                        .rounded_sm()
                        .text_xs()
                        .border_1()
                        .border_color(theme.border)
                        .child(entry.environment.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .child(entry.title.clone()),
                )
                .when_some(dot, |row, color| {
                    row.child(div().flex_none().size_2().rounded_full().bg(color))
                })
        });
        v_flex()
            .w(px(320.))
            .h_full()
            .flex_none()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .child(statuses)
            .child(
                div()
                    .id("threads")
                    .flex_1()
                    .overflow_y_scroll()
                    .p_1()
                    .children(rows),
            )
    }

    fn render_thread(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(_) = &self.open else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.muted_foreground)
                .child("Select a thread")
                .into_any_element();
        };
        let entry = self.open_entry();
        let title = entry.map(|e| e.title.clone()).unwrap_or_default();
        let status = entry.map(|e| e.status).unwrap_or(ThreadStatus::Idle);
        let working = matches!(status, ThreadStatus::Working | ThreadStatus::Input);
        let views = self.row_views.clone();
        let header = h_flex()
            .px_4()
            .py_2()
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .child(div().flex_1().min_w_0().truncate().child(title))
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(status.label()),
            )
            .child(
                Button::new("browser")
                    .label("Open in browser")
                    .outline()
                    .compact()
                    .on_click(cx.listener(|root, _, _, cx| root.open_in_browser(cx))),
            );
        let composer = h_flex()
            .p_3()
            .gap_2()
            .items_end()
            .border_t_1()
            .border_color(theme.border)
            .child(div().flex_1().child(Textarea::new(&self.composer)))
            .child(if working {
                Button::new("stop")
                    .label("Stop")
                    .danger()
                    .on_click(cx.listener(|root, _, _, cx| root.stop(cx)))
            } else {
                Button::new("send")
                    .label("Send")
                    .primary()
                    .on_click(cx.listener(|root, _, window, cx| root.send(window, cx)))
            });
        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(header)
            .child(
                list(self.list_state.clone(), move |ix, _, _| {
                    views
                        .get(ix)
                        .map(|v| v.clone().into_any_element())
                        .unwrap_or_else(|| div().into_any_element())
                })
                .flex_1()
                .w_full(),
            )
            .when_some(self.message.clone(), |pane, message| {
                pane.child(
                    div()
                        .px_4()
                        .py_1()
                        .text_sm()
                        .text_color(theme.danger)
                        .child(message),
                )
            })
            .child(composer)
            .into_any_element()
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focus.contains_focused(window, cx) && self.open.is_none() {
            self.focus.focus(window, cx);
        }
        h_flex()
            .size_full()
            .track_focus(&self.focus)
            .key_context("Root")
            .on_action(cx.listener(|root, _: &NextDone, _, cx| root.next_done(cx)))
            .on_action(cx.listener(|root, _: &Send, window, cx| root.send(window, cx)))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(UI_FONT)
            .child(self.render_sidebar(cx))
            .child(self.render_thread(cx))
    }
}

pub fn run(config: Config) -> anyhow::Result<()> {
    let runtime = Runtime::new()?;
    let handle = runtime.handle().clone();
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        Theme::global_mut(cx).font_family = UI_FONT.into();
        cx.bind_keys([
            KeyBinding::new("alt-o", NextDone, Some("Root")),
            KeyBinding::new("ctrl-enter", Send, Some("Root")),
        ]);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1400.), px(900.)), cx)),
            titlebar: Some(TitlebarOptions {
                title: Some("T3 Code".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Root::new(config, handle, window, cx))
        })
        .expect("open the window");
    });
    drop(runtime);
    Ok(())
}
