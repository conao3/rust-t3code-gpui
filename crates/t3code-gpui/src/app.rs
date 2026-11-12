use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Theme,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{InputEvent, Textarea, TextareaState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use t3_client::config::Config;
use t3_client::session::{Change, Connection, ConnectionStatus, Launch, ThreadWatch};
use tokio::runtime::{Handle, Runtime};

use crate::projects::{self, ProjectGroup};
use crate::row_view::{RowEvent, RowView};
use crate::sidebar::{self, ThreadEntry, ThreadStatus};
use crate::timeline::{self, Pending, Row, RowKind};

const REDRAW_INTERVAL: Duration = Duration::from_millis(250);
const UI_FONT: &str = "Noto Sans CJK JP";

actions!(
    t3code_gpui,
    [
        Send,
        NextDone,
        NextUserMessage,
        PreviousUserMessage,
        NextThread,
        PreviousThread,
        Settle,
        NewThread,
        Cancel,
    ]
);

fn now_iso() -> String {
    humantime::format_rfc3339_millis(SystemTime::now()).to_string()
}

async fn on_runtime<T: std::marker::Send + 'static>(
    runtime: &Handle,
    future: impl std::future::Future<Output = anyhow::Result<T>> + std::marker::Send + 'static,
) -> anyhow::Result<T> {
    runtime.spawn(future).await?
}

struct OpenThread {
    environment: String,
    watch: ThreadWatch,
}

struct NewThreadDraft {
    group: Option<String>,
}

pub struct Root {
    runtime: Handle,
    connections: Vec<Arc<Connection>>,
    entries: Vec<ThreadEntry>,
    statuses: HashMap<(String, String), ThreadStatus>,
    host_filter: Option<String>,
    open: Option<OpenThread>,
    draft: Option<NewThreadDraft>,
    rows: Vec<Row>,
    row_views: Vec<Entity<RowView>>,
    row_subscriptions: HashMap<String, Subscription>,
    list_state: ListState,
    composer: Entity<TextareaState>,
    message: Option<String>,
    window_active: bool,
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
        let changes = cx.spawn_in(window, async move |this, cx| {
            while let Ok(change) = receiver.recv().await {
                if this
                    .update_in(cx, |root, window, cx| root.on_change(change, window, cx))
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
        let subscriptions = vec![
            cx.subscribe_in(
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
            ),
            cx.observe_window_activation(window, |root, window, _| {
                root.window_active = window.is_window_active();
            }),
        ];
        let list_state = ListState::new(0, ListAlignment::Bottom, px(800.));
        list_state.set_follow_mode(FollowMode::Tail);
        Self {
            runtime,
            connections,
            entries: Vec::new(),
            statuses: HashMap::new(),
            host_filter: None,
            open: None,
            draft: None,
            rows: Vec::new(),
            row_views: Vec::new(),
            row_subscriptions: HashMap::new(),
            list_state,
            composer,
            message: None,
            window_active: window.is_window_active(),
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

    fn current(&self) -> Option<(String, String)> {
        self.open
            .as_ref()
            .map(|o| (o.environment.clone(), o.watch.thread_id().to_string()))
    }

    fn visible_entries(&self) -> Vec<ThreadEntry> {
        self.entries
            .iter()
            .filter(|e| {
                self.host_filter
                    .as_ref()
                    .is_none_or(|h| *h == e.environment)
            })
            .cloned()
            .collect()
    }

    fn on_change(&mut self, change: Change, window: &mut Window, cx: &mut Context<Self>) {
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
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(REDRAW_INTERVAL).await;
                let _ = this.update_in(cx, |root, window, cx| root.refresh(window, cx));
            })
            .detach();
        }
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.redraw_scheduled = false;
        if std::mem::take(&mut self.shell_dirty) {
            self.rebuild_entries();
            self.notify_attention();
            self.visit_if_done();
            cx.notify();
        }
        if std::mem::take(&mut self.thread_dirty) {
            self.rebuild_rows(window, cx);
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

    fn notify_attention(&mut self) {
        let current = self.current();
        let attention: Vec<(String, String)> =
            sidebar::newly_needing_attention(&self.statuses, &self.entries)
                .into_iter()
                .filter(|e| !(self.window_active && current.as_ref() == Some(&e.key())))
                .map(|e| {
                    let what = match e.status {
                        ThreadStatus::Input => "Waiting for input",
                        _ => "Done",
                    };
                    (format!("{what} · {}", e.environment), e.title.clone())
                })
                .collect();
        self.statuses = self.entries.iter().map(|e| (e.key(), e.status)).collect();
        for (summary, body) in attention {
            self.runtime.spawn_blocking(move || {
                let _ = notify_rust::Notification::new()
                    .appname("T3 Code")
                    .summary(&summary)
                    .body(&body)
                    .show();
            });
        }
    }

    fn visit_if_done(&self) {
        let Some(open) = &self.open else { return };
        let done = self.entries.iter().any(|e| {
            e.is(&open.environment, open.watch.thread_id()) && e.status == ThreadStatus::Done
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

    fn rebuild_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        let mut subscriptions = std::mem::take(&mut self.row_subscriptions);
        let mut kept = HashMap::new();
        let views: Vec<Entity<RowView>> = rows
            .iter()
            .map(|row| match existing.remove(&row.key) {
                Some(view) => {
                    view.update(cx, |view, cx| view.update(row.clone(), cx));
                    if let Some(subscription) = subscriptions.remove(&row.key) {
                        kept.insert(row.key.clone(), subscription);
                    }
                    view
                }
                None => {
                    let view = {
                        let row = row.clone();
                        cx.new(|cx| RowView::new(row, cx))
                    };
                    if row.kind == RowKind::Request {
                        kept.insert(
                            row.key.clone(),
                            cx.subscribe_in(&view, window, Self::on_row_event),
                        );
                    }
                    view
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
        self.row_subscriptions = kept;
        cx.notify();
    }

    fn on_row_event(
        &mut self,
        view: &Entity<RowView>,
        event: &RowEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            RowEvent::Respond {
                request_id,
                decision,
            } => self.respond(view.clone(), request_id.clone(), decision.clone(), cx),
            RowEvent::Submit { request_id } => {
                self.submit_answers(view.clone(), request_id.clone(), window, cx)
            }
        }
    }

    fn respond(
        &mut self,
        view: Entity<RowView>,
        request_id: String,
        decision: String,
        cx: &mut Context<Self>,
    ) {
        let Some((environment, thread_id)) = self.current() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                connection.respond(&thread_id, &request_id, &decision).await
            })
            .await;
            if let Err(error) = result {
                view.update(cx, |view, cx| view.set_responding(false, cx));
                let _ = this.update(cx, |root, cx| {
                    root.message = Some(format!("Reply failed: {error:#}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn pending_question(&self) -> Option<(Entity<RowView>, String)> {
        self.rows
            .iter()
            .zip(&self.row_views)
            .rev()
            .find_map(|(row, view)| match &row.pending {
                Some(Pending::Question { request_id, .. }) => {
                    Some((view.clone(), request_id.clone()))
                }
                _ => None,
            })
    }

    fn submit_answers(
        &mut self,
        view: Entity<RowView>,
        request_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((environment, thread_id)) = self.current() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let questions = self.rows.iter().find_map(|row| match &row.pending {
            Some(Pending::Question {
                request_id: id,
                questions,
            }) if *id == request_id => Some(questions.clone()),
            _ => None,
        });
        let Some(questions) = questions else { return };
        let typed = self.composer.read(cx).value().to_string();
        let Some(answers) = timeline::answers_with_text(&questions, view.read(cx).drafts(), &typed)
        else {
            self.message = Some("Answer every question first.".into());
            cx.notify();
            return;
        };
        self.message = None;
        view.update(cx, |view, cx| view.set_responding(true, cx));
        self.composer
            .update(cx, |state, cx| state.set_value("", window, cx));
        let runtime = self.runtime.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                connection.answer(&thread_id, &request_id, answers).await
            })
            .await;
            if let Err(error) = result {
                view.update(cx, |view, cx| view.set_responding(false, cx));
                let _ = this.update_in(cx, |root, window, cx| {
                    root.message = Some(format!("Answer failed: {error:#}"));
                    root.composer
                        .update(cx, |state, cx| state.set_value(typed, window, cx));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn open_thread(&mut self, environment: String, thread_id: String, cx: &mut Context<Self>) {
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let watch = connection.watch_thread(&self.runtime, &thread_id);
        self.open = Some(OpenThread { environment, watch });
        self.draft = None;
        self.rows.clear();
        self.row_views.clear();
        self.row_subscriptions.clear();
        self.list_state.reset(0);
        self.list_state.set_follow_mode(FollowMode::Tail);
        self.message = None;
        self.runtime.spawn({
            let thread_id = thread_id.clone();
            async move {
                let _ = connection.visit_thread(&thread_id, &now_iso()).await;
            }
        });
        cx.notify();
    }

    fn close_thread(&mut self, cx: &mut Context<Self>) {
        self.open = None;
        self.rows.clear();
        self.row_views.clear();
        self.row_subscriptions.clear();
        self.list_state.reset(0);
        cx.notify();
    }

    fn open_entry(&self) -> Option<&ThreadEntry> {
        let open = self.open.as_ref()?;
        self.entries
            .iter()
            .find(|e| e.is(&open.environment, open.watch.thread_id()))
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.draft.is_some() {
            self.launch(window, cx);
            return;
        }
        if let Some((view, request_id)) = self.pending_question() {
            self.submit_answers(view, request_id, window, cx);
            return;
        }
        let Some((environment, thread_id)) = self.current() else {
            return;
        };
        let text = self.composer.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let queue = self
            .open_entry()
            .is_some_and(|e| matches!(e.status, ThreadStatus::Working | ThreadStatus::Input));
        let runtime = self.runtime.clone();
        self.composer
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.spawn_in(window, async move |this, cx| {
            let sent = on_runtime(&runtime, {
                let text = text.clone();
                async move { connection.send_message(&thread_id, &text, queue).await }
            })
            .await;
            let _ = this.update_in(cx, |root, window, cx| {
                root.message = sent.as_ref().err().map(|e| format!("Send failed: {e:#}"));
                if sent.is_err() {
                    root.composer
                        .update(cx, |state, cx| state.set_value(text, window, cx));
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        let Some((environment, thread_id)) = self.current() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
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

    fn settle(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.open_entry().cloned() else {
            return;
        };
        let Some(connection) = self.connection(&entry.environment) else {
            return;
        };
        let settled = entry.status != ThreadStatus::Settled;
        if settled {
            let visible = self.visible_entries();
            let next = sidebar::next_with(
                &visible,
                Some((&entry.environment, &entry.thread_id)),
                |e| e.status != ThreadStatus::Settled,
            )
            .map(ThreadEntry::key);
            match next {
                Some((environment, thread_id)) => self.open_thread(environment, thread_id, cx),
                None => self.close_thread(cx),
            }
        }
        let runtime = self.runtime.clone();
        let thread_id = entry.thread_id.clone();
        cx.spawn(async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                connection.settle_thread(&thread_id, settled).await
            })
            .await;
            if let Err(error) = result {
                let _ = this.update(cx, |root, cx| {
                    root.message = Some(format!("Settle failed: {error:#}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn open_in_browser(&mut self, cx: &mut Context<Self>) {
        let Some((environment, thread_id)) = self.current() else {
            return;
        };
        let url = self
            .connection(&environment)
            .and_then(|c| c.thread_url(&thread_id));
        match url {
            Some(url) => cx.open_url(&url),
            None => {
                self.message = Some("The environment id is not known yet.".into());
                cx.notify();
            }
        }
    }

    fn next_done(&mut self, cx: &mut Context<Self>) {
        let current = self.current();
        let visible = self.visible_entries();
        let next = sidebar::next_with(
            &visible,
            current.as_ref().map(|(e, t)| (e.as_str(), t.as_str())),
            |e| sidebar::needs_attention(e.status),
        )
        .map(ThreadEntry::key);
        if let Some((environment, thread_id)) = next {
            self.open_thread(environment, thread_id, cx);
        }
    }

    fn adjacent_thread(&mut self, forward: bool, cx: &mut Context<Self>) {
        let current = self.current();
        let visible = self.visible_entries();
        let next = sidebar::adjacent(
            &visible,
            current.as_ref().map(|(e, t)| (e.as_str(), t.as_str())),
            forward,
        )
        .map(ThreadEntry::key);
        if let Some((environment, thread_id)) = next {
            self.open_thread(environment, thread_id, cx);
        }
    }

    fn jump_user_message(&mut self, forward: bool, cx: &mut Context<Self>) {
        let top = self.list_state.logical_scroll_top();
        let users = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kind == RowKind::User)
            .map(|(i, _)| i);
        let target = if forward {
            users.into_iter().find(|i| *i > top.item_ix)
        } else {
            users
                .rev()
                .find(|i| *i < top.item_ix || (*i == top.item_ix && top.offset_in_item > px(0.)))
        };
        match target {
            Some(item_ix) => self.list_state.scroll_to(ListOffset {
                item_ix,
                offset_in_item: px(0.),
            }),
            None if forward => self.list_state.set_follow_mode(FollowMode::Tail),
            None => self.list_state.scroll_to(ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            }),
        }
        cx.notify();
    }

    fn project_groups(&self) -> Vec<ProjectGroup> {
        let shells: Vec<_> = self.connections.iter().map(|c| c.shell()).collect();
        let guards: Vec<_> = shells
            .iter()
            .map(|s| s.lock().expect("shell lock"))
            .collect();
        projects::groups(
            self.connections
                .iter()
                .zip(&guards)
                .map(|(c, shell)| (c.label(), &**shell)),
        )
    }

    fn new_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let groups = self.project_groups();
        let from_open = self.open_entry().and_then(|entry| {
            groups
                .iter()
                .find(|g| g.contains(&entry.environment, &entry.project_id))
        });
        let group = from_open.or(groups.first()).map(|g| g.key.clone());
        self.draft = Some(NewThreadDraft { group });
        self.message = None;
        self.close_thread(cx);
        self.composer
            .update(cx, |state, cx| state.focus(window, cx));
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.draft.take().is_some() {
            self.message = None;
            cx.notify();
        }
    }

    fn launch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let groups = self.project_groups();
        let selected = self.draft.as_ref().and_then(|d| d.group.clone());
        let Some(group) = groups.iter().find(|g| Some(&g.key) == selected.as_ref()) else {
            self.message = Some("Pick a project first.".into());
            cx.notify();
            return;
        };
        let member = group.target().clone();
        let Some(connection) = self.connection(&member.environment) else {
            return;
        };
        let settings = {
            let shell = connection.shell();
            let shell = shell.lock().expect("shell lock");
            projects::launch_settings(&shell, &member.project_id)
        };
        let Some((model, runtime_mode)) = settings else {
            self.message =
                Some("This project has no default model and no thread to take one from.".into());
            cx.notify();
            return;
        };
        let thread_id = uuid::Uuid::new_v4().to_string();
        let project_id = member.project_id.clone();
        let title = projects::title_seed(&text);
        self.composer
            .update(cx, |state, cx| state.set_value("", window, cx));
        let runtime = self.runtime.clone();
        let environment = member.environment.clone();
        cx.spawn_in(window, async move |this, cx| {
            let launched = on_runtime(&runtime, {
                let text = text.clone();
                async move {
                    connection
                        .launch_thread(Launch {
                            thread_id: &thread_id,
                            project_id: &project_id,
                            title: &title,
                            model,
                            runtime_mode: &runtime_mode,
                            text: &text,
                        })
                        .await
                }
            })
            .await;
            let _ = this.update_in(cx, |root, window, cx| match launched {
                Ok(thread_id) => root.open_thread(environment, thread_id, cx),
                Err(error) => {
                    root.message = Some(format!("New thread failed: {error:#}"));
                    root.composer
                        .update(cx, |state, cx| state.set_value(text, window, cx));
                    cx.notify();
                }
            });
        })
        .detach();
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
        let filters = (self.connections.len() > 1).then(|| {
            let chip = |id: SharedString, label: String, value: Option<String>, selected: bool| {
                Button::new(id)
                    .label(label)
                    .compact()
                    .outline()
                    .selected(selected)
                    .on_click(cx.listener(move |root, _, _, cx| {
                        root.host_filter = value.clone();
                        cx.notify();
                    }))
            };
            h_flex()
                .px_2()
                .gap_1()
                .flex_wrap()
                .child(chip(
                    "host-all".into(),
                    "All".into(),
                    None,
                    self.host_filter.is_none(),
                ))
                .children(self.connections.iter().map(|c| {
                    let label = c.label().to_string();
                    chip(
                        SharedString::from(format!("host-{label}")),
                        label.clone(),
                        Some(label.clone()),
                        self.host_filter.as_ref() == Some(&label),
                    )
                }))
        });
        let open = self.current();
        let rows = self
            .visible_entries()
            .into_iter()
            .enumerate()
            .map(|(i, entry)| {
                let selected = open.as_ref().is_some_and(|(env, id)| entry.is(env, id));
                let dot = match entry.status {
                    ThreadStatus::Input => Some(theme.warning),
                    ThreadStatus::Working => Some(theme.primary),
                    ThreadStatus::Failed => Some(theme.danger),
                    ThreadStatus::Done => Some(theme.success),
                    ThreadStatus::Idle | ThreadStatus::Settled => None,
                };
                let (environment, thread_id) = entry.key();
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
            .children(filters)
            .child(
                div().px_2().pt_2().child(
                    Button::new("new-thread")
                        .label("New thread")
                        .outline()
                        .compact()
                        .w_full()
                        .on_click(cx.listener(|root, _, window, cx| root.new_thread(window, cx))),
                ),
            )
            .child(
                div()
                    .id("threads")
                    .flex_1()
                    .overflow_y_scroll()
                    .p_1()
                    .children(rows),
            )
    }

    fn render_message(&self, cx: &App) -> Option<Div> {
        self.message.clone().map(|message| {
            div()
                .px_4()
                .py_1()
                .text_sm()
                .text_color(cx.theme().danger)
                .child(message)
        })
    }

    fn render_composer(&self, button: Button) -> impl IntoElement {
        h_flex()
            .p_3()
            .gap_2()
            .items_end()
            .child(div().flex_1().child(Textarea::new(&self.composer)))
            .child(button)
    }

    fn render_draft(&self, draft: &NewThreadDraft, cx: &mut Context<Self>) -> AnyElement {
        let message = self.render_message(cx);
        let theme = cx.theme();
        let groups = self.project_groups();
        let choices = groups.iter().enumerate().map(|(i, group)| {
            let key = group.key.clone();
            let hosts: Vec<&str> = group
                .members
                .iter()
                .map(|m| m.environment.as_str())
                .collect();
            Button::new(("project", i))
                .label(format!("{} · {}", group.title, hosts.join(", ")))
                .compact()
                .outline()
                .selected(draft.group.as_ref() == Some(&group.key))
                .on_click(cx.listener(move |root, _, _, cx| {
                    if let Some(draft) = root.draft.as_mut() {
                        draft.group = Some(key.clone());
                    }
                    cx.notify();
                }))
        });
        let target = groups
            .iter()
            .find(|g| draft.group.as_ref() == Some(&g.key))
            .map(|g| format!("Starts on {}", g.target().environment))
            .unwrap_or_else(|| "Pick a project".into());
        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(
                h_flex()
                    .px_4()
                    .py_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().flex_1().child("New thread"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(target),
                    ),
            )
            .child(
                div()
                    .id("projects")
                    .flex_1()
                    .overflow_y_scroll()
                    .p_4()
                    .child(h_flex().gap_1().flex_wrap().children(choices)),
            )
            .children(message)
            .child(
                div().border_t_1().border_color(theme.border).child(
                    self.render_composer(
                        Button::new("launch")
                            .label("Start")
                            .primary()
                            .on_click(cx.listener(|root, _, window, cx| root.launch(window, cx))),
                    ),
                ),
            )
            .into_any_element()
    }

    fn render_thread(&self, cx: &mut Context<Self>) -> AnyElement {
        let message = self.render_message(cx);
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
                Button::new("settle")
                    .label(if status == ThreadStatus::Settled {
                        "Unsettle"
                    } else {
                        "Settle"
                    })
                    .outline()
                    .compact()
                    .on_click(cx.listener(|root, _, _, cx| root.settle(cx))),
            )
            .child(
                Button::new("browser")
                    .label("Open in browser")
                    .outline()
                    .compact()
                    .on_click(cx.listener(|root, _, _, cx| root.open_in_browser(cx))),
            );
        let button = if self.pending_question().is_some() {
            Button::new("answer")
                .label("Answer")
                .primary()
                .on_click(cx.listener(|root, _, window, cx| root.send(window, cx)))
        } else if working {
            Button::new("stop")
                .label("Stop")
                .danger()
                .on_click(cx.listener(|root, _, _, cx| root.stop(cx)))
        } else {
            Button::new("send")
                .label("Send")
                .primary()
                .on_click(cx.listener(|root, _, window, cx| root.send(window, cx)))
        };
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
            .children(message)
            .child(
                div()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(self.render_composer(button)),
            )
            .into_any_element()
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focus.contains_focused(window, cx) && self.open.is_none() && self.draft.is_none() {
            self.focus.focus(window, cx);
        }
        let pane = match &self.draft {
            Some(draft) => self.render_draft(draft, cx),
            None => self.render_thread(cx),
        };
        h_flex()
            .size_full()
            .track_focus(&self.focus)
            .key_context("Root")
            .on_action(cx.listener(|root, _: &NextDone, _, cx| root.next_done(cx)))
            .on_action(cx.listener(|root, _: &Send, window, cx| root.send(window, cx)))
            .on_action(
                cx.listener(|root, _: &NextUserMessage, _, cx| root.jump_user_message(true, cx)),
            )
            .on_action(
                cx.listener(|root, _: &PreviousUserMessage, _, cx| {
                    root.jump_user_message(false, cx)
                }),
            )
            .on_action(cx.listener(|root, _: &NextThread, _, cx| root.adjacent_thread(true, cx)))
            .on_action(
                cx.listener(|root, _: &PreviousThread, _, cx| root.adjacent_thread(false, cx)),
            )
            .on_action(cx.listener(|root, _: &Settle, _, cx| root.settle(cx)))
            .on_action(cx.listener(|root, _: &NewThread, window, cx| root.new_thread(window, cx)))
            .on_action(cx.listener(|root, _: &Cancel, _, cx| root.cancel(cx)))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(UI_FONT)
            .child(self.render_sidebar(cx))
            .child(pane)
    }
}

pub fn run(config: Config) -> anyhow::Result<()> {
    let runtime = Runtime::new()?;
    let handle = runtime.handle().clone();
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        Theme::global_mut(cx).font_family = UI_FONT.into();
        let root = Some("Root");
        cx.bind_keys([
            KeyBinding::new("alt-o", NextDone, root),
            KeyBinding::new("alt-n", NextUserMessage, root),
            KeyBinding::new("alt-p", PreviousUserMessage, root),
            KeyBinding::new("ctrl-shift-]", NextThread, root),
            KeyBinding::new("ctrl-shift-[", PreviousThread, root),
            KeyBinding::new("ctrl-}", NextThread, root),
            KeyBinding::new("ctrl-{", PreviousThread, root),
            KeyBinding::new("ctrl-shift-s", Settle, root),
            KeyBinding::new("alt-s", Settle, root),
            KeyBinding::new("ctrl-n", NewThread, root),
            KeyBinding::new("ctrl-shift-o", NewThread, root),
            KeyBinding::new("escape", Cancel, root),
            KeyBinding::new("ctrl-enter", Send, root),
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
