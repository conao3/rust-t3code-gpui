use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Theme,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{InputEvent, Textarea, TextareaState},
    menu::ContextMenuExt as _,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;
use t3_client::config::Config;
use t3_client::session::{Change, Connection, ConnectionStatus, Delivery, Launch, ThreadWatch};
use tokio::runtime::{Handle, Runtime};

use crate::client_settings::ClientSettings;
use crate::drafts::{self, Drafts};
use crate::keymap;
use crate::models;
use crate::projects::{self, ProjectGroup};
use crate::row_view::{RowEvent, RowView};
use crate::sidebar::{self, ThreadEntry, ThreadStatus};
use crate::timeline::{self, Pending, Row, RowKind};

type CommandFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<u64>> + std::marker::Send>>;

const REDRAW_INTERVAL: Duration = Duration::from_millis(250);
const UI_FONT: &str = "Noto Sans CJK JP";

actions!(
    t3code_gpui,
    [
        Send,
        NextDone,
        PreviousDone,
        ToggleSidebar,
        TogglePin,
        CycleRuntimeMode,
        NextUserMessage,
        PreviousUserMessage,
        NextThread,
        PreviousThread,
        Settle,
        NewThread,
        Cancel,
        ToggleModelPicker,
    ]
);

/// The action of a T3 keybinding command this app implements.
fn action_for(command: &str) -> Option<Box<dyn Action>> {
    Some(match command {
        "thread.nextDone" => Box::new(NextDone),
        "thread.previousDone" => Box::new(PreviousDone),
        "chat.nextUserMessage" => Box::new(NextUserMessage),
        "chat.previousUserMessage" => Box::new(PreviousUserMessage),
        "thread.next" => Box::new(NextThread),
        "thread.previous" => Box::new(PreviousThread),
        "thread.settle" => Box::new(Settle),
        "chat.new" | "chat.newLocal" => Box::new(NewThread),
        "modelPicker.toggle" => Box::new(ToggleModelPicker),
        "composer.mode" => Box::new(CycleRuntimeMode),
        "sidebar.toggle" => Box::new(ToggleSidebar),
        "thread.pin" => Box::new(TogglePin),
        _ => return None,
    })
}

#[derive(Clone, Debug, PartialEq)]
enum ThreadOp {
    Pin,
    Unpin,
    Snooze(u64),
    Unsnooze,
    Settle,
    Unsettle,
    Archive,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = t3code_gpui, no_json)]
struct ThreadMenu {
    environment: String,
    thread_id: String,
    op: ThreadOp,
}

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
    carry: Option<Value>,
    model: Option<Value>,
    runtime_mode: Option<String>,
    interaction_mode: String,
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
    user_cursor: Option<usize>,
    composer: Entity<TextareaState>,
    message: Option<String>,
    picker_open: bool,
    drafts: Drafts,
    drafts_path: std::path::PathBuf,
    drafts_save_scheduled: bool,
    pending_composer: Option<String>,
    key_rules: Vec<keymap::Rule>,
    settings: ClientSettings,
    sidebar_hidden: bool,
    wake_scheduled: bool,
    collapsed: HashSet<sidebar::Section>,
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
    fn new(
        config: Config,
        settings: ClientSettings,
        (drafts, drafts_path): (Drafts, std::path::PathBuf),
        runtime: Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (sender, receiver) = async_channel::unbounded::<Change>();
        let connections: Vec<Arc<Connection>> = config
            .environments
            .iter()
            .map(|environment| Connection::new(environment, sender.clone()))
            .collect();
        let shell_tasks = connections
            .iter()
            .flat_map(|connection| {
                [
                    connection.spawn_shell(&runtime),
                    connection.spawn_config(&runtime),
                ]
            })
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
            let mut state = TextareaState::new(window, cx).placeholder("Message");
            state.set_auto_grow(2, 12, cx);
            state
        });
        let weak = cx.entity().downgrade();
        let subscriptions = vec![
            cx.intercept_keystrokes(move |event, window, cx| {
                let handled = weak
                    .update(cx, |root, cx| root.intercept(event, window, cx))
                    .unwrap_or(false);
                if handled {
                    cx.stop_propagation();
                }
            }),
            cx.observe_window_activation(window, |root, window, _| {
                root.window_active = window.is_window_active();
            }),
            cx.subscribe(&composer, |root, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) && root.pending_composer.is_none() {
                    root.stash_composer(cx);
                }
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
            user_cursor: None,
            composer,
            message: None,
            picker_open: false,
            drafts,
            drafts_path,
            drafts_save_scheduled: false,
            pending_composer: None,
            key_rules: Vec::new(),
            settings,
            sidebar_hidden: false,
            wake_scheduled: false,
            collapsed: HashSet::from([sidebar::Section::Settled]),
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

    /// Runs the keybinding of a typed chord, and submits the composer on the key the send shortcut
    /// names. Returns whether the keystroke was used.
    fn intercept(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let typed = keymap::Chord {
            control: modifiers.control,
            alt: modifiers.alt,
            shift: modifiers.shift,
            platform: modifiers.platform,
            key: keystroke.key.clone(),
        };
        if let Some(action) =
            keymap::command_for(&self.key_rules, &typed, |c| action_for(c).is_some())
                .and_then(action_for)
        {
            window.dispatch_action(action, cx);
            return true;
        }
        if keystroke.key != "enter" || modifiers.alt || modifiers.shift || modifiers.platform {
            return false;
        }
        if !self.composer.read(cx).focus_handle(cx).is_focused(window) {
            return false;
        }
        let text = self.composer.read(cx).value().to_string();
        if !modifiers.control && self.settings.requires_modifier(&text) {
            return false;
        }
        self.send(window, cx);
        true
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
            Change::Status { .. } | Change::Shell { .. } | Change::Config { .. } => {
                self.shell_dirty = true
            }
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
            self.rebuild_key_rules();
            self.schedule_wake(window, cx);
            self.visit_if_done();
            cx.notify();
        }
        if std::mem::take(&mut self.thread_dirty) {
            self.rebuild_rows(window, cx);
        }
    }

    /// The keybindings of the primary host that has sent its configuration, as T3 uses the primary
    /// server's keybindings.
    fn rebuild_key_rules(&mut self) {
        let rules = self.connections.iter().find_map(|c| {
            c.config()
                .lock()
                .expect("config lock")
                .as_ref()
                .map(keymap::rules)
        });
        if let Some(rules) = rules {
            self.key_rules = rules;
        }
    }

    /// Rebuilds the shelves a minute later while a thread is snoozed, so it wakes on time.
    fn schedule_wake(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = now_iso();
        let snoozed = self.entries.iter().any(|e| {
            e.snoozed_until
                .as_deref()
                .is_some_and(|until| until > now.as_str())
        });
        if !snoozed || self.wake_scheduled {
            return;
        }
        self.wake_scheduled = true;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(60))
                .await;
            let _ = this.update_in(cx, |root, window, cx| {
                root.wake_scheduled = false;
                root.shell_dirty = true;
                root.refresh(window, cx);
            });
        })
        .detach();
    }

    fn label_of(&self, environment_id: &str) -> Option<String> {
        self.connections
            .iter()
            .find(|c| c.environment_id().as_deref() == Some(environment_id))
            .map(|c| c.label().to_string())
    }

    fn shelves_of(&self, entries: &[ThreadEntry]) -> Vec<sidebar::Shelf> {
        sidebar::shelves(entries, &now_iso(), |id| self.label_of(id))
    }

    fn rebuild_entries(&mut self) {
        let mut entries = Vec::new();
        for connection in &self.connections {
            let shell = connection.shell();
            let shell = shell.lock().expect("shell lock");
            entries.extend(sidebar::entries(connection.label(), &shell));
        }
        self.entries = self
            .shelves_of(&entries)
            .into_iter()
            .flat_map(|shelf| shelf.rows.into_iter().map(|(entry, _)| entry))
            .collect();
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
                        ThreadStatus::Approval => "Waiting for approval",
                        _ => "Done",
                    };
                    (format!("{what} · {}", e.environment), e.title.clone())
                })
                .collect();
        self.statuses = self.entries.iter().map(|e| (e.key(), e.status)).collect();
        if !self.settings.desktop_notifications {
            return;
        }
        let sound = self.settings.notification_sound;
        for (summary, body) in attention {
            self.runtime.spawn_blocking(move || {
                let mut notification = notify_rust::Notification::new();
                notification
                    .appname("T3 Code")
                    .summary(&summary)
                    .body(&body);
                if sound {
                    notification.sound_name("message-new-instant");
                }
                let _ = notification.show();
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
                    if matches!(row.kind, RowKind::Request | RowKind::Plan) {
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
                self.user_cursor = None;
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
            RowEvent::Implement { plan_id, markdown } => {
                let (plan_id, markdown) = (plan_id.clone(), markdown.clone());
                let view = view.clone();
                let Some((environment, thread_id)) = self.current() else {
                    return;
                };
                let Some(connection) = self.connection(&environment) else {
                    return;
                };
                let runtime = self.runtime.clone();
                cx.spawn(async move |this, cx| {
                    let result = on_runtime(&runtime, async move {
                        connection
                            .implement_plan(&thread_id, &plan_id, &markdown)
                            .await
                    })
                    .await;
                    if let Err(error) = result {
                        view.update(cx, |view, cx| view.set_responding(false, cx));
                        let _ = this.update(cx, |root, cx| {
                            root.message = Some(format!("Implementing the plan failed: {error:#}"));
                            cx.notify();
                        });
                    }
                })
                .detach();
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
        self.stash_composer(cx);
        let watch = connection.watch_thread(&self.runtime, &thread_id);
        self.open = Some(OpenThread { environment, watch });
        self.draft = None;
        self.rows.clear();
        self.row_views.clear();
        self.row_subscriptions.clear();
        self.list_state.reset(0);
        self.user_cursor = None;
        self.list_state.set_follow_mode(FollowMode::Tail);
        self.message = None;
        self.runtime.spawn({
            let thread_id = thread_id.clone();
            async move {
                let _ = connection.visit_thread(&thread_id, &now_iso()).await;
            }
        });
        self.load_composer();
        cx.notify();
    }

    fn composer_key(&self) -> Option<String> {
        if self.draft.is_some() {
            return Some(drafts::NEW_THREAD.to_string());
        }
        self.current()
            .map(|(environment, thread_id)| drafts::thread_key(&environment, &thread_id))
    }

    /// Keeps the composer text as the draft of what the composer is for now. Call before the
    /// composer changes what it is for.
    fn stash_composer(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.composer_key() else {
            return;
        };
        let text = self.composer.read(cx).value().to_string();
        if self.drafts.set(&key, &text) {
            self.schedule_drafts_save(cx);
        }
    }

    /// Puts the draft of what the composer is for now into it, at the next render.
    fn load_composer(&mut self) {
        let text = self
            .composer_key()
            .and_then(|key| self.drafts.get(&key).map(str::to_string))
            .unwrap_or_default();
        self.pending_composer = Some(text);
    }

    fn schedule_drafts_save(&mut self, cx: &mut Context<Self>) {
        if self.drafts_save_scheduled {
            return;
        }
        self.drafts_save_scheduled = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let _ = this.update(cx, |root, cx| {
                root.drafts_save_scheduled = false;
                if let Err(error) = root.drafts.save(&root.drafts_path) {
                    root.message = Some(format!("Saving drafts failed: {error:#}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn close_thread(&mut self, cx: &mut Context<Self>) {
        self.open = None;
        self.rows.clear();
        self.row_views.clear();
        self.row_subscriptions.clear();
        self.list_state.reset(0);
        self.user_cursor = None;
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
        let status = self.open_entry().map(|e| e.status);
        let active_run = self.thread_value(&environment, &thread_id).and_then(|t| {
            t.get("activeRunId")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
        let delivery = match (status, active_run) {
            (Some(ThreadStatus::Working), Some(run_id)) if self.settings.steer_follow_ups => {
                Delivery::Steer { run_id }
            }
            (Some(ThreadStatus::Working | ThreadStatus::Input | ThreadStatus::Approval), _) => {
                Delivery::Queue
            }
            _ => Delivery::Start,
        };
        let runtime = self.runtime.clone();
        self.composer
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.spawn_in(window, async move |this, cx| {
            let sent = on_runtime(&runtime, {
                let text = text.clone();
                async move { connection.send_message(&thread_id, &text, delivery).await }
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
                true,
                |e| e.status != ThreadStatus::Settled,
            )
            .map(ThreadEntry::key);
            match next {
                Some((environment, thread_id)) => self.open_thread(environment, thread_id, cx),
                None => {
                    self.stash_composer(cx);
                    self.close_thread(cx);
                    self.load_composer();
                }
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

    fn thread_op(&mut self, menu: ThreadMenu, cx: &mut Context<Self>) {
        let Some(connection) = self.connection(&menu.environment) else {
            return;
        };
        let until = match menu.op {
            ThreadOp::Snooze(hours) => Some(
                humantime::format_rfc3339_millis(
                    SystemTime::now() + Duration::from_secs(hours * 3600),
                )
                .to_string(),
            ),
            _ => None,
        };
        let runtime = self.runtime.clone();
        let thread_id = menu.thread_id.clone();
        let op = menu.op.clone();
        cx.spawn(async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                match op {
                    ThreadOp::Pin => connection.pin_thread(&thread_id, true).await,
                    ThreadOp::Unpin => connection.pin_thread(&thread_id, false).await,
                    ThreadOp::Snooze(_) => {
                        connection.snooze_thread(&thread_id, until.as_deref()).await
                    }
                    ThreadOp::Unsnooze => connection.snooze_thread(&thread_id, None).await,
                    ThreadOp::Settle => connection.settle_thread(&thread_id, true).await,
                    ThreadOp::Unsettle => connection.settle_thread(&thread_id, false).await,
                    ThreadOp::Archive => connection.archive_thread(&thread_id).await,
                }
            })
            .await;
            if let Err(error) = result {
                let _ = this.update(cx, |root, cx| {
                    root.message = Some(format!("Thread update failed: {error:#}"));
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

    fn next_done(&mut self, forward: bool, cx: &mut Context<Self>) {
        let current = self.current();
        let visible = self.visible_entries();
        let next = sidebar::next_with(
            &visible,
            current.as_ref().map(|(e, t)| (e.as_str(), t.as_str())),
            forward,
            |e| {
                sidebar::needs_attention(e.status)
                    || self
                        .drafts
                        .has(&drafts::thread_key(&e.environment, &e.thread_id))
            },
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
        let (from, inside) = if top.item_ix < self.rows.len() {
            (top.item_ix, top.offset_in_item > px(0.))
        } else {
            (self.user_cursor.unwrap_or(self.rows.len()), false)
        };
        let target = timeline::user_message_jump(&self.rows, from, inside, forward);
        self.user_cursor = target;
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

    fn thread_value(&self, environment: &str, thread_id: &str) -> Option<Value> {
        let connection = self.connection(environment)?;
        let shell = connection.shell();
        let shell = shell.lock().expect("shell lock");
        shell
            .threads
            .iter()
            .find(|t| t.get("id").and_then(Value::as_str) == Some(thread_id))
            .cloned()
    }

    fn project_value(&self, environment: &str, project_id: &str) -> Option<Value> {
        let connection = self.connection(environment)?;
        let shell = connection.shell();
        let shell = shell.lock().expect("shell lock");
        shell
            .projects
            .iter()
            .find(|p| p.get("id").and_then(Value::as_str) == Some(project_id))
            .cloned()
    }

    fn config_of(&self, environment: &str) -> Option<Value> {
        let connection = self.connection(environment)?;
        let config = connection.config();
        let config = config.lock().expect("config lock");
        config.clone()
    }

    /// Where the draft starts and with what: the host, the project, the model and the runtime mode.
    fn draft_target(&self) -> Option<(String, String, Option<Value>, String)> {
        let draft = self.draft.as_ref()?;
        let groups = self.project_groups();
        let group = groups
            .iter()
            .find(|g| Some(&g.key) == draft.group.as_ref())?;
        let member = group.target().clone();
        let project = self.project_value(&member.environment, &member.project_id)?;
        let config = self.config_of(&member.environment);
        let defaults = models::new_thread(config.as_ref(), &project, draft.carry.as_ref());
        let model = draft
            .model
            .clone()
            .or_else(|| defaults.as_ref().map(|(m, _)| m.clone()));
        let runtime_mode = draft
            .runtime_mode
            .clone()
            .or_else(|| defaults.map(|(_, r)| r))
            .unwrap_or_else(|| "full-access".into());
        Some((member.environment, member.project_id, model, runtime_mode))
    }

    fn new_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stash_composer(cx);
        let groups = self.project_groups();
        let from_open = self.open_entry().and_then(|entry| {
            groups
                .iter()
                .find(|g| g.contains(&entry.environment, &entry.project_id))
        });
        let group = from_open.or(groups.first()).map(|g| g.key.clone());
        let carry = self
            .current()
            .and_then(|(env, id)| self.thread_value(&env, &id))
            .and_then(|t| t.get("modelSelection").cloned())
            .filter(|m| !m.is_null());
        self.draft = Some(NewThreadDraft {
            group,
            carry,
            model: None,
            runtime_mode: None,
            interaction_mode: "default".into(),
        });
        self.message = None;
        self.picker_open = false;
        self.close_thread(cx);
        self.load_composer();
        self.composer
            .update(cx, |state, cx| state.focus(window, cx));
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.picker_open {
            self.picker_open = false;
            cx.notify();
            return;
        }
        if self.draft.is_some() {
            self.stash_composer(cx);
            self.draft = None;
            self.load_composer();
            self.message = None;
            cx.notify();
        }
    }

    fn launch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some((environment, project_id, model, runtime_mode)) = self.draft_target() else {
            self.message = Some("Pick a project first.".into());
            cx.notify();
            return;
        };
        let Some(model) = model else {
            self.message = Some("No model is available on this host. Pick one first.".into());
            cx.notify();
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let interaction_mode = self
            .draft
            .as_ref()
            .map(|d| d.interaction_mode.clone())
            .unwrap_or_else(|| "default".into());
        let thread_id = uuid::Uuid::new_v4().to_string();
        let title = projects::title_seed(&text);
        self.composer
            .update(cx, |state, cx| state.set_value("", window, cx));
        let runtime = self.runtime.clone();
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
                            interaction_mode: &interaction_mode,
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

    /// The host, model, runtime mode and interaction mode the composer acts on: the open thread's,
    /// or the draft's.
    fn composer_target(&self) -> Option<(String, Option<Value>, String, String)> {
        if let Some(draft) = &self.draft {
            let (environment, _, model, runtime_mode) = self.draft_target()?;
            return Some((
                environment,
                model,
                runtime_mode,
                draft.interaction_mode.clone(),
            ));
        }
        let (environment, thread_id) = self.current()?;
        let thread = self.thread_value(&environment, &thread_id)?;
        let get = |key: &str| {
            thread
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let model = thread
            .get("modelSelection")
            .cloned()
            .filter(|m| !m.is_null());
        Some((
            environment,
            model,
            get("runtimeMode"),
            get("interactionMode"),
        ))
    }

    fn dispatch_on_open(
        &mut self,
        what: &'static str,
        cx: &mut Context<Self>,
        command: impl FnOnce(Arc<Connection>, String) -> CommandFuture,
    ) {
        let Some((environment, thread_id)) = self.current() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let runtime = self.runtime.clone();
        let future = command(connection, thread_id);
        cx.spawn(async move |this, cx| {
            if let Err(error) = on_runtime(&runtime, future).await {
                let _ = this.update(cx, |root, cx| {
                    root.message = Some(format!("{what} failed: {error:#}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn pick_model(&mut self, instance_id: String, slug: String, cx: &mut Context<Self>) {
        self.picker_open = false;
        let chosen = models::selection(&instance_id, &slug);
        if let Some(draft) = self.draft.as_mut() {
            draft.model = Some(chosen);
            cx.notify();
            return;
        }
        let switch = self
            .composer_target()
            .and_then(|(_, model, _, _)| model)
            .is_some_and(|m| m.get("instanceId").and_then(Value::as_str) != Some(&instance_id));
        self.dispatch_on_open("Model change", cx, move |connection, thread_id| {
            Box::pin(async move { connection.set_model(&thread_id, chosen, switch).await })
        });
        cx.notify();
    }

    fn cycle_runtime_mode(&mut self, cx: &mut Context<Self>) {
        let Some((environment, model, current, _)) = self.composer_target() else {
            return;
        };
        let config = self.config_of(&environment);
        let instance_id = model
            .as_ref()
            .and_then(|m| m.get("instanceId").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string();
        let modes: Vec<String> = config
            .as_ref()
            .and_then(|c| {
                models::instances(c)
                    .into_iter()
                    .find(|i| i.instance_id == instance_id)
            })
            .map(|i| i.runtime_modes)
            .unwrap_or_else(|| {
                models::RUNTIME_MODES
                    .iter()
                    .map(|m| m.to_string())
                    .collect()
            });
        let next = modes
            .iter()
            .position(|m| *m == current)
            .map_or(0, |i| (i + 1) % modes.len());
        let Some(next) = modes.get(next).cloned() else {
            return;
        };
        if let Some(draft) = self.draft.as_mut() {
            draft.runtime_mode = Some(next);
            cx.notify();
            return;
        }
        self.dispatch_on_open("Runtime mode change", cx, move |connection, thread_id| {
            Box::pin(async move { connection.set_runtime_mode(&thread_id, &next).await })
        });
    }

    fn toggle_plan(&mut self, cx: &mut Context<Self>) {
        let Some((_, _, _, current)) = self.composer_target() else {
            return;
        };
        let next = if current == "plan" { "default" } else { "plan" };
        if let Some(draft) = self.draft.as_mut() {
            draft.interaction_mode = next.into();
            cx.notify();
            return;
        }
        self.dispatch_on_open("Plan mode change", cx, move |connection, thread_id| {
            Box::pin(async move { connection.set_interaction_mode(&thread_id, next).await })
        });
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
        let primary = self.connections.first().map(|c| c.label().to_string());
        let shelves = self.shelves_of(&self.visible_entries());
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut index = 0usize;
        for shelf in shelves {
            let section = shelf.section;
            let collapsed = self.collapsed.contains(&section);
            rows.push(
                h_flex()
                    .id(SharedString::from(format!("shelf-{}", section.label())))
                    .px_2()
                    .pt_2()
                    .pb_1()
                    .gap_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .cursor_pointer()
                    .on_click(cx.listener(move |root, _, _, cx| {
                        if !root.collapsed.remove(&section) {
                            root.collapsed.insert(section);
                        }
                        cx.notify();
                    }))
                    .child(if collapsed { "▸" } else { "▾" })
                    .child(section.label())
                    .child(format!("{}", shelf.rows.len()))
                    .into_any_element(),
            );
            if collapsed {
                continue;
            }
            for (entry, depth) in shelf.rows {
                index += 1;
                let selected = open.as_ref().is_some_and(|(env, id)| entry.is(env, id));
                let dot = match entry.status {
                    ThreadStatus::Approval | ThreadStatus::Input => Some(theme.warning),
                    ThreadStatus::Working => Some(theme.primary),
                    ThreadStatus::Failed => Some(theme.danger),
                    ThreadStatus::Done => Some(theme.success),
                    ThreadStatus::Idle | ThreadStatus::Settled => None,
                };
                let remote = primary.as_ref() != Some(&entry.environment);
                let (environment, thread_id) = entry.key();
                let menu_target = (environment.clone(), thread_id.clone());
                let pinned = entry.pin_order.is_some();
                let snoozed = section == sidebar::Section::Snoozed;
                let settled = entry.status == ThreadStatus::Settled;
                rows.push(
                    h_flex()
                        .id(("thread", index))
                        .w_full()
                        .pl(px(8. + 16. * depth as f32))
                        .pr_2()
                        .py_1()
                        .gap_2()
                        .rounded_md()
                        .when(selected, |row| row.bg(theme.list_active))
                        .when(settled || snoozed, |row| {
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
                                .when(remote, |chip| {
                                    chip.border_color(theme.warning).text_color(theme.warning)
                                })
                                .when(!remote, |chip| chip.border_color(theme.border))
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
                        .context_menu(move |menu, _, _| {
                            let (environment, thread_id) = menu_target.clone();
                            let op = |op: ThreadOp| -> Box<dyn Action> {
                                Box::new(ThreadMenu {
                                    environment: environment.clone(),
                                    thread_id: thread_id.clone(),
                                    op,
                                })
                            };
                            let menu = if pinned {
                                menu.menu("Unpin", op(ThreadOp::Unpin))
                            } else {
                                menu.menu("Pin", op(ThreadOp::Pin))
                            };
                            let menu = if snoozed {
                                menu.menu("Unsnooze", op(ThreadOp::Unsnooze))
                            } else {
                                menu.menu("Snooze 1 hour", op(ThreadOp::Snooze(1)))
                                    .menu("Snooze 4 hours", op(ThreadOp::Snooze(4)))
                                    .menu("Snooze 1 day", op(ThreadOp::Snooze(24)))
                            };
                            let menu = if settled {
                                menu.menu("Unsettle", op(ThreadOp::Unsettle))
                            } else {
                                menu.menu("Settle", op(ThreadOp::Settle))
                            };
                            menu.separator().menu("Archive", op(ThreadOp::Archive))
                        })
                        .into_any_element(),
                );
            }
        }
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

    fn render_composer(&self, button: Button, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let target = self.composer_target();
        let config = target.as_ref().and_then(|(env, ..)| self.config_of(env));
        let instances = config.as_ref().map(models::instances).unwrap_or_default();
        let current = target.as_ref().and_then(|(_, model, ..)| model.clone());
        let current_instance = current
            .as_ref()
            .and_then(|m| m.get("instanceId").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string();
        let current_slug = current
            .as_ref()
            .and_then(|m| m.get("model").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string();
        let toolbar = target
            .as_ref()
            .map(|(_, model, runtime_mode, interaction_mode)| {
                let label = model
                    .as_ref()
                    .map(|m| models::label(config.as_ref(), m))
                    .unwrap_or_else(|| "Pick a model".into());
                let plan = self.settings.plan_mode_enabled
                    && instances
                        .iter()
                        .any(|i| i.instance_id == current_instance && i.interaction_mode_toggle);
                h_flex()
                    .px_3()
                    .pt_2()
                    .gap_2()
                    .child(
                        Button::new("model")
                            .label(label)
                            .compact()
                            .outline()
                            .selected(self.picker_open)
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.picker_open = !root.picker_open;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("runtime-mode")
                            .label(runtime_mode.clone())
                            .compact()
                            .outline()
                            .on_click(cx.listener(|root, _, _, cx| root.cycle_runtime_mode(cx))),
                    )
                    .when(plan, |row| {
                        row.child(
                            Button::new("plan")
                                .label("Plan")
                                .compact()
                                .outline()
                                .selected(interaction_mode == "plan")
                                .on_click(cx.listener(|root, _, _, cx| root.toggle_plan(cx))),
                        )
                    })
            });
        let picker = self.picker_open.then(|| {
            let groups = instances.iter().enumerate().map(|(gi, instance)| {
                let buttons = instance.models.iter().enumerate().map(|(mi, model)| {
                    let chosen =
                        instance.instance_id == current_instance && model.slug == current_slug;
                    let instance_id = instance.instance_id.clone();
                    let slug = model.slug.clone();
                    Button::new(SharedString::from(format!("pick-{gi}-{mi}")))
                        .label(model.name.clone())
                        .compact()
                        .when(chosen, |b| b.primary())
                        .when(!chosen, |b| b.outline())
                        .on_click(cx.listener(move |root, _, _, cx| {
                            root.pick_model(instance_id.clone(), slug.clone(), cx)
                        }))
                });
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(instance.name.clone()),
                    )
                    .child(h_flex().gap_1().flex_wrap().children(buttons))
            });
            div()
                .id("model-picker")
                .mx_3()
                .mt_2()
                .p_2()
                .max_h(px(320.))
                .overflow_y_scroll()
                .rounded_md()
                .border_1()
                .border_color(theme.border)
                .child(v_flex().gap_2().children(groups))
        });
        v_flex()
            .children(picker)
            .children(toolbar)
            .child(
                h_flex()
                    .p_3()
                    .gap_2()
                    .items_end()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(self.settings.font_size_prompt))
                            .child(Textarea::new(&self.composer)),
                    )
                    .child(button),
            )
            .into_any_element()
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
                        cx,
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
        let working = matches!(
            status,
            ThreadStatus::Working | ThreadStatus::Input | ThreadStatus::Approval
        );
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
                    .child(self.render_composer(button, cx)),
            )
            .into_any_element()
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(text) = self.pending_composer.take() {
            self.composer
                .update(cx, |state, cx| state.set_value(text, window, cx));
        }
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
            .on_action(cx.listener(|root, _: &NextDone, _, cx| root.next_done(true, cx)))
            .on_action(cx.listener(|root, _: &PreviousDone, _, cx| root.next_done(false, cx)))
            .on_action(
                cx.listener(|root, menu: &ThreadMenu, _, cx| root.thread_op(menu.clone(), cx)),
            )
            .on_action(cx.listener(|root, _: &TogglePin, _, cx| {
                if let Some(entry) = root.open_entry().cloned() {
                    let op = if entry.pin_order.is_some() {
                        ThreadOp::Unpin
                    } else {
                        ThreadOp::Pin
                    };
                    root.thread_op(
                        ThreadMenu {
                            environment: entry.environment,
                            thread_id: entry.thread_id,
                            op,
                        },
                        cx,
                    );
                }
            }))
            .on_action(cx.listener(|root, _: &ToggleSidebar, _, cx| {
                root.sidebar_hidden = !root.sidebar_hidden;
                cx.notify();
            }))
            .on_action(cx.listener(|root, _: &CycleRuntimeMode, _, cx| root.cycle_runtime_mode(cx)))
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
            .on_action(cx.listener(|root, _: &ToggleModelPicker, _, cx| {
                root.picker_open = !root.picker_open;
                cx.notify();
            }))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(UI_FONT)
            .when(!self.sidebar_hidden, |root| {
                root.child(self.render_sidebar(cx))
            })
            .child(pane)
    }
}

pub fn run(config: Config, settings: ClientSettings) -> anyhow::Result<()> {
    let drafts_path = drafts::default_path()?;
    let drafts = Drafts::load(&drafts_path)?;
    let runtime = Runtime::new()?;
    let handle = runtime.handle().clone();
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        let theme = Theme::global_mut(cx);
        theme.font_family = UI_FONT.into();
        theme.font_size = px(settings.font_size_interface);
        theme.mono_font_size = px(settings.font_size_code);
        let root = Some("Root");
        cx.bind_keys([KeyBinding::new("escape", Cancel, root)]);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1400.), px(900.)), cx)),
            titlebar: Some(TitlebarOptions {
                title: Some("T3 Code".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Root::new(config, settings, (drafts, drafts_path), handle, window, cx))
        })
        .expect("open the window");
    });
    drop(runtime);
    Ok(())
}
