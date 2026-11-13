use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Theme,
    button::{Button, ButtonVariants as _},
    command::{Command, CommandGroup, CommandItem, CommandState},
    diff::{Diff, DiffFile, DiffMode, DiffParseOptions, DiffState},
    h_flex,
    input::{CursorBlink, Input, InputEvent, InputState, Position, Textarea, TextareaState},
    menu::{ContextMenuExt as _, DropdownMenu as _},
    text::{TextView, TextViewState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;
use t3_client::config::{self, Config, Environment};
use t3_client::http::Api;
use t3_client::session::{Change, Connection, ConnectionStatus, Delivery, Launch, ThreadWatch};
use tokio::runtime::{Handle, Runtime};

use crate::attachments;
use crate::client_settings::{self, ClientSettings};
use crate::completion;
use crate::drafts::{self, Drafts};
use crate::explorer;
use crate::keymap;
use crate::models;
use crate::projects::{self, ProjectGroup};
use crate::row_view::{RowEvent, RowView};
use crate::sidebar::{self, ThreadEntry, ThreadStatus};
use crate::tables;
use crate::timeline::{self, Pending, Row, RowKind};

type CommandFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<u64>> + std::marker::Send>>;

const REDRAW_INTERVAL: Duration = Duration::from_millis(250);
const UI_FONT: &str = "Inter Variable";
const DIFF_TITLE: &str = "Changes";
const CURSOR_BLINK: Duration = Duration::from_millis(500);
const CURSOR_BLINK_IDLE: Duration = Duration::from_secs(5);

actions!(
    t3code_gpui,
    [
        Send,
        NextDone,
        PreviousDone,
        ToggleSidebar,
        TogglePin,
        Find,
        TogglePalette,
        ToggleFilePicker,
        ToggleDiff,
        ToggleExplorer,
        ManageHosts,
        OpenSettings,
        CloseFilePanel,
        CycleRuntimeMode,
        NextUserMessage,
        PreviousUserMessage,
        NextThread,
        PreviousThread,
        Settle,
        NewThread,
        Cancel,
        ToggleModelPicker,
        HistoryBack,
        HistoryForward,
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
        "chat.find" => Box::new(Find),
        "commandPalette.toggle" => Box::new(TogglePalette),
        "filePicker.toggle" => Box::new(ToggleFilePicker),
        "diff.toggle" => Box::new(ToggleDiff),
        "settings.connections" => Box::new(ManageHosts),
        "settings.open" => Box::new(OpenSettings),
        "rightPanel.toggle" | "rightPanel.close" => Box::new(CloseFilePanel),
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
struct OpenFile {
    path: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = t3code_gpui, no_json)]
struct OpenThreadAction {
    environment: String,
    thread_id: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = t3code_gpui, no_json)]
struct ThreadMenu {
    environment: String,
    thread_id: String,
    op: ThreadOp,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = t3code_gpui, no_json)]
struct PickModelAction {
    instance_id: String,
    slug: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = t3code_gpui, no_json)]
struct SetRuntimeModeAction {
    mode: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = t3code_gpui, no_json)]
struct SetModelOptionAction {
    id: String,
    value: Value,
}

fn now_iso() -> String {
    humantime::format_rfc3339_millis(SystemTime::now()).to_string()
}

/// Uploads the composer's attachments in order and returns what a message refers to them by.
async fn upload_all(
    connection: &Connection,
    pending: &[attachments::Pending],
) -> anyhow::Result<Vec<Value>> {
    let mut uploaded = Vec::with_capacity(pending.len());
    for file in pending {
        uploaded.push(
            connection
                .upload_attachment(&file.name, &file.mime_type, file.bytes.as_ref().clone())
                .await?,
        );
    }
    Ok(uploaded)
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

enum PanelPart {
    Text(Entity<TextViewState>),
    Table {
        rows: Rc<Vec<Vec<String>>>,
        header_rows: usize,
        sort: Option<tables::Sort>,
    },
}

enum PanelBody {
    Document(Vec<PanelPart>),
    Diff(Entity<DiffState>),
}

type FindHits = Rc<Vec<(usize, std::ops::Range<usize>)>>;

struct Explorer {
    environment: String,
    cwd: String,
    directories: HashMap<String, Vec<explorer::Entry>>,
    expanded: HashSet<String>,
    error: Option<String>,
}

struct FilePanel {
    path: String,
    body: PanelBody,
    note: Option<String>,
    lines: Rc<Vec<String>>,
    hits: FindHits,
    hit_at: usize,
    scroll: UniformListScrollHandle,
}

impl FilePanel {
    fn new(path: String, body: PanelBody, note: Option<String>, lines: Vec<String>) -> Self {
        Self {
            path,
            body,
            note,
            lines: Rc::new(lines),
            hits: Rc::default(),
            hit_at: 0,
            scroll: UniformListScrollHandle::new(),
        }
    }
}

/// The parts the file preview shows: a markdown file with its tables taken out so they can be
/// sorted, anything else as one fenced block.
fn document_parts(path: &str, contents: &str, cx: &mut App) -> Vec<PanelPart> {
    let markdown = completion::file_markdown(path, contents);
    if markdown != contents {
        return vec![PanelPart::Text(
            cx.new(|cx| TextViewState::markdown(&markdown, cx)),
        )];
    }
    tables::split(contents)
        .into_iter()
        .map(|segment| match segment {
            tables::Segment::Markdown(text) => {
                PanelPart::Text(cx.new(|cx| TextViewState::markdown(&text, cx)))
            }
            tables::Segment::Table(rows) => PanelPart::Table {
                rows: Rc::new(rows),
                header_rows: 1,
                sort: None,
            },
        })
        .collect()
}

struct Completion {
    trigger: completion::Trigger,
    suggestions: Vec<completion::Suggestion>,
    selected: usize,
}

struct NewThreadDraft {
    group: Option<String>,
    carry: Option<Value>,
    model: Option<Value>,
    runtime_mode: Option<String>,
    interaction_mode: String,
}

struct TimelinePane {
    views: Vec<Entity<RowView>>,
    list: ListState,
}

impl Render for TimelinePane {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let views = self.views.clone();
        div()
            .relative()
            .flex_1()
            .w_full()
            .h_full()
            .child(
                list(self.list.clone(), move |ix, _, _| {
                    views
                        .get(ix)
                        .map(|view| view.clone().into_any_element())
                        .unwrap_or_else(|| div().into_any_element())
                })
                .w_full()
                .h_full(),
            )
            .child(
                gpui_kit::base::Scrollbar::vertical(&self.list)
                    .mode(gpui_kit::base::ScrollbarMode::Always),
            )
    }
}

#[derive(PartialEq)]
struct SidebarStamp {
    history_available: (bool, bool),
    entries_revision: u64,
    open: Option<(String, String)>,
    filter: Option<String>,
    collapsed: HashSet<sidebar::Section>,
    families: HashSet<(String, String)>,
    connections: Vec<(String, Option<String>, ConnectionStatus)>,
    width: f32,
    light: bool,
    font_size: f32,
    minute: u64,
}

struct SidebarPane {
    root: WeakEntity<Root>,
}

impl Render for SidebarPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.root
            .update(cx, |root, cx| root.render_sidebar(cx).into_any_element())
            .expect("sidebar root is alive")
    }
}

#[derive(PartialEq)]
struct ComposerControlsStamp {
    target: Option<(String, Option<Value>, String, String)>,
    current: Option<(String, String)>,
    draft: bool,
    status: Option<ThreadStatus>,
    question: bool,
    empty: bool,
    attachments: bool,
    picker: bool,
    picker_instance: Option<String>,
    catalog_revision: u64,
    light: bool,
    font_size: f32,
    plan_enabled: bool,
}

struct ComposerControlsPane {
    root: WeakEntity<Root>,
}
impl Render for ComposerControlsPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.root
            .update(cx, |root, cx| root.render_composer_controls(cx))
            .expect("composer root is alive")
    }
}

pub struct Root {
    sidebar_resizing: bool,
    history: crate::navigation::History,
    sidebar_pane: Entity<SidebarPane>,
    sidebar_stamp: Option<SidebarStamp>,
    runtime: Handle,
    connections: Vec<Arc<Connection>>,
    model_catalogs: HashMap<String, (Value, Vec<models::Instance>)>,
    model_catalog_revision: u64,
    composer_controls: Entity<ComposerControlsPane>,
    composer_controls_stamp: Option<ComposerControlsStamp>,
    entries: Vec<ThreadEntry>,
    entries_revision: u64,
    statuses: HashMap<(String, String), ThreadStatus>,
    host_filter: Option<String>,
    open: Option<OpenThread>,
    draft: Option<NewThreadDraft>,
    rows: Vec<Row>,
    row_views: Vec<Entity<RowView>>,
    timeline_pane: Entity<TimelinePane>,
    row_subscriptions: HashMap<String, Subscription>,
    list_state: ListState,
    user_cursor: Option<usize>,
    composer: Entity<TextareaState>,
    message: Option<String>,
    picker_open: bool,
    picker_instance: Option<String>,
    model_picker: Entity<CommandState>,
    attachments: Vec<attachments::Pending>,
    queued: Vec<timeline::Queued>,
    thread_loading: bool,
    completion: Option<Completion>,
    completion_seq: u64,
    find_input: Entity<InputState>,
    palette: Entity<CommandState>,
    palette_open: bool,
    file_picker: Entity<CommandState>,
    file_picker_open: bool,
    file_hits: Vec<String>,
    file_panel: Option<FilePanel>,
    explorer: Option<Explorer>,
    file_find_input: Entity<InputState>,
    file_find_open: bool,
    panel_focus: FocusHandle,
    find_open: bool,
    find_hits: Vec<usize>,
    find_at: usize,
    drafts: Drafts,
    drafts_path: std::path::PathBuf,
    drafts_save_scheduled: bool,
    pending_composer: Option<String>,
    key_rules: Vec<keymap::Rule>,
    settings: ClientSettings,
    settings_path: std::path::PathBuf,
    settings_open: bool,
    sidebar_hidden: bool,
    wake_scheduled: bool,
    collapsed: HashSet<sidebar::Section>,
    collapsed_families: HashSet<(String, String)>,
    window_active: bool,
    shell_dirty: bool,
    thread_dirty: bool,
    redraw_scheduled: bool,
    focus: FocusHandle,
    host_tasks: HashMap<String, Vec<tokio::task::JoinHandle<()>>>,
    config: Config,
    config_path: std::path::PathBuf,
    change_sender: async_channel::Sender<Change>,
    hosts_open: bool,
    host_inputs: [Entity<InputState>; 3],
    _subscriptions: Vec<Subscription>,
    _changes: Task<()>,
}

impl Root {
    fn new(
        (config, config_path): (Config, std::path::PathBuf),
        (settings, settings_path): (ClientSettings, std::path::PathBuf),
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
        let shell_tasks: HashMap<String, Vec<tokio::task::JoinHandle<()>>> = connections
            .iter()
            .map(|connection| {
                (
                    connection.label().to_string(),
                    vec![
                        connection.spawn_shell(&runtime),
                        connection.spawn_config(&runtime),
                    ],
                )
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
            let mut state = TextareaState::new(window, cx)
                .placeholder("Ask anything. @ files, $ skills, / commands");
            state.set_auto_grow(1, 12, cx);
            state
        });
        let find_input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in thread"));
        let file_find_input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in file"));
        let host_inputs = [
            cx.new(|cx| InputState::new(window, cx).placeholder("Name (eos, helios)")),
            cx.new(|cx| InputState::new(window, cx).placeholder("Server URL or pairing URL")),
            cx.new(|cx| InputState::new(window, cx).placeholder("Pairing code or token")),
        ];
        let palette = cx.new(|cx| CommandState::new(window, cx));
        let file_picker = cx.new(|cx| CommandState::new(window, cx));
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
            cx.subscribe(&find_input, |root, _, event: &InputEvent, cx| match event {
                InputEvent::Change => root.find_again(true, cx),
                InputEvent::PressEnter { shift, .. } => root.step_find(!*shift, cx),
                _ => {}
            }),
            cx.subscribe(
                &file_find_input,
                |root, _, event: &InputEvent, cx| match event {
                    InputEvent::Change => root.file_find_again(cx),
                    InputEvent::PressEnter { shift, .. } => root.step_file_find(!*shift, cx),
                    _ => {}
                },
            ),
            cx.subscribe(&composer, |root, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) && root.pending_composer.is_none() {
                    root.stash_composer(cx);
                    root.update_completion(cx);
                }
            }),
        ];
        let list_state = ListState::new(0, ListAlignment::Bottom, px(800.));
        list_state.set_follow_mode(FollowMode::Tail);
        let sidebar_root = cx.entity().downgrade();
        Self {
            sidebar_resizing: false,
            history: crate::navigation::History::default(),
            sidebar_pane: cx.new(|_| SidebarPane {
                root: sidebar_root.clone(),
            }),
            sidebar_stamp: None,
            runtime,
            connections,
            model_catalogs: HashMap::new(),
            model_catalog_revision: 0,
            composer_controls: cx.new(|_| ComposerControlsPane { root: sidebar_root }),
            composer_controls_stamp: None,
            entries: Vec::new(),
            entries_revision: 0,
            statuses: HashMap::new(),
            host_filter: None,
            open: None,
            draft: None,
            rows: Vec::new(),
            row_views: Vec::new(),
            timeline_pane: cx.new(|_| TimelinePane {
                views: Vec::new(),
                list: list_state.clone(),
            }),
            row_subscriptions: HashMap::new(),
            list_state,
            user_cursor: None,
            composer,
            message: None,
            picker_open: false,
            picker_instance: None,
            model_picker: cx.new(|cx| CommandState::new(window, cx)),
            attachments: Vec::new(),
            queued: Vec::new(),
            thread_loading: false,
            completion: None,
            completion_seq: 0,
            find_input,
            palette,
            palette_open: false,
            file_picker,
            file_picker_open: false,
            file_hits: Vec::new(),
            file_panel: None,
            explorer: None,
            file_find_input,
            file_find_open: false,
            panel_focus: cx.focus_handle(),
            find_open: false,
            find_hits: Vec::new(),
            find_at: 0,
            drafts,
            drafts_path,
            drafts_save_scheduled: false,
            pending_composer: None,
            key_rules: Vec::new(),
            settings,
            settings_path,
            settings_open: false,
            sidebar_hidden: false,
            wake_scheduled: false,
            collapsed: HashSet::from([sidebar::Section::Settled]),
            collapsed_families: HashSet::new(),
            window_active: window.is_window_active(),
            shell_dirty: false,
            thread_dirty: false,
            redraw_scheduled: false,
            focus: cx.focus_handle(),
            host_tasks: shell_tasks,
            config,
            config_path,
            change_sender: sender,
            hosts_open: false,
            host_inputs,
            _subscriptions: subscriptions,
            _changes: changes,
        }
    }

    /// Runs the keybinding of a typed chord, and submits the composer on the key the send shortcut
    /// names. Returns whether the keystroke was used.
    /// The directory a file mention searches: the open thread's worktree or its project, or the
    /// project the new thread starts in.
    fn composer_cwd(&self) -> Option<(String, String)> {
        if self.draft.is_some() {
            let (environment, project_id, ..) = self.draft_target()?;
            let project = self.project_value(&environment, &project_id)?;
            let root = project.get("workspaceRoot")?.as_str()?.to_string();
            return Some((environment, root));
        }
        let (environment, thread_id) = self.current()?;
        let thread = self.thread_value(&environment, &thread_id)?;
        if let Some(worktree) = thread.get("worktreePath").and_then(Value::as_str) {
            return Some((environment, worktree.to_string()));
        }
        let project_id = thread.get("projectId")?.as_str()?;
        let project = self.project_value(&environment, project_id)?;
        let root = project.get("workspaceRoot")?.as_str()?.to_string();
        Some((environment, root))
    }

    /// Attaches an image on the clipboard. Returns whether there was one, so text pastes as usual.
    fn paste_image(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let image = item.into_entries().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(image),
            _ => None,
        });
        let Some(image) = image else {
            return false;
        };
        let mime = image.format.mime_type().to_string();
        let extension = mime.rsplit('/').next().unwrap_or("png").to_string();
        let name = format!("pasted-{}.{extension}", self.attachments.len() + 1);
        match attachments::Pending::new(name, mime, image.bytes) {
            Ok(pending) => self.attachments.push(pending),
            Err(error) => self.message = Some(format!("{error:#}")),
        }
        cx.notify();
        true
    }

    fn attach_paths(&mut self, paths: &ExternalPaths, cx: &mut Context<Self>) {
        for path in paths.paths() {
            match attachments::Pending::from_path(path) {
                Ok(pending) => self.attachments.push(pending),
                Err(error) => self.message = Some(format!("{error:#}")),
            }
        }
        cx.notify();
    }

    fn update_completion(&mut self, cx: &mut Context<Self>) {
        let state = self.composer.read(cx);
        let text = state.value().to_string();
        let cursor = state.cursor();
        self.completion_seq += 1;
        let Some(trigger) = completion::trigger(&text, cursor) else {
            if self.completion.take().is_some() {
                cx.notify();
            }
            return;
        };
        if trigger.kind == completion::TriggerKind::Path {
            let Some((environment, cwd)) = self.composer_cwd() else {
                return;
            };
            let Some(connection) = self.connection(&environment) else {
                return;
            };
            let seq = self.completion_seq;
            let runtime = self.runtime.clone();
            let query = trigger.query.clone();
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                if this
                    .read_with(cx, |root, _| root.completion_seq != seq)
                    .unwrap_or(true)
                {
                    return;
                }
                let found = on_runtime(&runtime, async move {
                    connection.search_entries(&cwd, &query, 20).await
                })
                .await;
                let _ = this.update(cx, |root, cx| {
                    if root.completion_seq != seq {
                        return;
                    }
                    root.completion = found.ok().map(|result| Completion {
                        trigger,
                        suggestions: completion::path_suggestions(&result),
                        selected: 0,
                    });
                    cx.notify();
                });
            })
            .detach();
            return;
        }
        let provider = self.composer_target().and_then(|(environment, model, ..)| {
            let instance = model?.get("instanceId")?.as_str()?.to_string();
            let config = self.config_of(&environment)?;
            config
                .get("providers")?
                .as_array()?
                .iter()
                .find(|p| p.get("instanceId").and_then(Value::as_str) == Some(&instance))
                .cloned()
        });
        let suggestions = provider
            .map(|p| completion::provider_suggestions(&p, trigger.kind, &trigger.query))
            .unwrap_or_default();
        self.completion = Some(Completion {
            trigger,
            suggestions,
            selected: 0,
        });
        cx.notify();
    }

    /// Moves through and accepts the completion list while it is open. Returns whether the key
    /// was used.
    fn completion_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(open) = self
            .completion
            .as_mut()
            .filter(|c| !c.suggestions.is_empty())
        else {
            return false;
        };
        if !self.composer.read(cx).focus_handle(cx).is_focused(window) {
            return false;
        }
        let n = open.suggestions.len().min(8);
        match keystroke.key.as_str() {
            "down" => open.selected = (open.selected + 1) % n,
            "up" => open.selected = (open.selected + n - 1) % n,
            "escape" => self.completion = None,
            "enter" | "tab" if !keystroke.modifiers.control => {
                let text = self.composer.read(cx).value().to_string();
                let (value, at) =
                    completion::apply(&text, &open.trigger, &open.suggestions[open.selected]);
                let (line, column) = completion::line_column(&value, at);
                self.completion = None;
                self.composer.update(cx, |state, cx| {
                    state.set_value(value, window, cx);
                    state.set_cursor_position(Position::new(line, column), window, cx);
                });
                self.stash_composer(cx);
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    fn intercept(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if self.completion_key(keystroke, window, cx) {
            return true;
        }
        if keystroke.key == "v"
            && keystroke.modifiers.control
            && !keystroke.modifiers.alt
            && self.composer.read(cx).focus_handle(cx).is_focused(window)
            && self.paste_image(cx)
        {
            return true;
        }
        if keystroke.key == "escape"
            && self.find_open
            && self.find_input.read(cx).focus_handle(cx).is_focused(window)
        {
            self.close_find(window, cx);
            return true;
        }
        if keystroke.key == "escape"
            && self.file_find_open
            && self
                .file_find_input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        {
            self.close_file_find(window, cx);
            return true;
        }
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
            Change::Config { environment } => {
                self.model_catalog_revision = self.model_catalog_revision.wrapping_add(1);
                if let Some(config) = self.config_of(&environment) {
                    let instances = models::instances(&config);
                    self.model_catalogs.insert(environment, (config, instances));
                } else {
                    self.model_catalogs.remove(&environment);
                }
                self.shell_dirty = true;
            }
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
        self.entries_revision += 1;
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
        let (rows, queued, loading) = {
            let state = open.watch.state();
            let state = state.lock().expect("thread lock");
            (
                timeline::rows(&state),
                timeline::queued(&state),
                state.projection.is_empty(),
            )
        };
        self.queued = queued;
        self.thread_loading = loading;
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
                    if matches!(
                        row.kind,
                        RowKind::Request | RowKind::Plan | RowKind::Subagent
                    ) {
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
        self.timeline_pane.update(cx, |pane, cx| {
            pane.views = self.row_views.clone();
            cx.notify();
        });
        self.row_subscriptions = kept;
        self.find_again(false, cx);
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
            RowEvent::OpenChild { thread_id } => {
                if let Some((environment, _)) = self.current() {
                    self.open_thread(environment, thread_id.clone(), cx);
                }
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
        self.stash_composer(cx);
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
                    root.stash_composer(cx);
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
        self.history.visit((environment.clone(), thread_id.clone()));
        self.stash_composer(cx);
        let watch = connection.watch_thread(&self.runtime, &thread_id);
        self.open = Some(OpenThread { environment, watch });
        self.draft = None;
        self.rows.clear();
        self.row_views.clear();
        self.row_subscriptions.clear();
        self.list_state.reset(0);
        self.thread_loading = true;
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

    fn toggle_file_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.file_picker_open = !self.file_picker_open;
        if self.file_picker_open {
            self.file_hits.clear();
            self.file_picker.update(cx, |state, cx| {
                state.set_query("", window, cx);
                state.focus(window, cx);
            });
        } else {
            self.composer
                .update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    fn search_files(&mut self, query: String, cx: &mut Context<Self>) {
        let Some((environment, cwd)) = self.composer_cwd() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let found = on_runtime(&runtime, async move {
                connection.search_entries(&cwd, &query, 50).await
            })
            .await;
            let _ = this.update(cx, |root, cx| {
                root.file_hits = found
                    .ok()
                    .and_then(|r| r.get("entries").and_then(Value::as_array).cloned())
                    .unwrap_or_default()
                    .iter()
                    .filter(|e| e.get("kind").and_then(Value::as_str) == Some("file"))
                    .filter_map(|e| e.get("path").and_then(Value::as_str).map(str::to_string))
                    .collect();
                cx.notify();
            });
        })
        .detach();
    }

    /// Writes client settings to the official app's file and applies them at once.
    fn change_setting(&mut self, key: &'static str, value: Value, cx: &mut Context<Self>) {
        match client_settings::update(&self.settings_path, &[(key, value)]) {
            Ok(settings) => {
                let (interface, code) = (settings.font_size_interface, settings.font_size_code);
                self.settings = settings;
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

    fn render_settings(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
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

    fn toggle_hosts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hosts_open = !self.hosts_open;
        if self.hosts_open {
            self.host_inputs[0].update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    fn connect_host(&mut self, environment: &Environment) {
        let connection = Connection::new(environment, self.change_sender.clone());
        self.host_tasks.insert(
            environment.label.clone(),
            vec![
                connection.spawn_shell(&self.runtime),
                connection.spawn_config(&self.runtime),
            ],
        );
        self.connections.push(connection);
    }

    /// Adds a host from the dialog: a pairing URL, or a server URL with a pairing code (exchanged
    /// for a token) or a token. The token is checked against the server before it is saved.
    fn add_host(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let [label, url, secret] = self
            .host_inputs
            .clone()
            .map(|input| input.read(cx).value().trim().to_string());
        if label.is_empty() || self.connection(&label).is_some() {
            self.message = Some("Give the host a name no other host has.".into());
            cx.notify();
            return;
        }
        let (origin, code, is_token) = match config::pairing_url(&url) {
            Some((origin, code)) => (origin, code, false),
            None if secret.is_empty() => {
                self.message = Some(
                    "Paste a pairing URL, or a server URL with a pairing code or token.".into(),
                );
                cx.notify();
                return;
            }
            None => {
                let is_token = secret.len() > 32;
                (url.trim_end_matches('/').to_string(), secret, is_token)
            }
        };
        let runtime = self.runtime.clone();
        cx.spawn_in(window, async move |this, cx| {
            let token = on_runtime(&runtime, {
                let origin = origin.clone();
                async move {
                    let token = if is_token {
                        code
                    } else {
                        Api::new(&origin, "").pair(&code, "T3 Code (GPUI)").await?
                    };
                    Api::new(&origin, &token).get("/api/auth/session").await?;
                    Ok(token)
                }
            })
            .await;
            let _ = this.update_in(cx, |root, window, cx| {
                match token {
                    Ok(token) => {
                        let environment = Environment {
                            label,
                            http_base_url: origin,
                            token,
                        };
                        let mut next = root.config.clone();
                        next.environments.push(environment.clone());
                        match next.save(&root.config_path) {
                            Ok(()) => {
                                root.config = next;
                                root.connect_host(&environment);
                                for input in root.host_inputs.clone() {
                                    input.update(cx, |state, cx| state.set_value("", window, cx));
                                }
                                root.message = None;
                            }
                            Err(error) => {
                                root.message = Some(format!("Saving the hosts failed: {error:#}"))
                            }
                        }
                    }
                    Err(error) => root.message = Some(format!("Adding the host failed: {error:#}")),
                }
                root.shell_dirty = true;
                root.refresh(window, cx);
            });
        })
        .detach();
    }

    fn remove_host(&mut self, label: String, window: &mut Window, cx: &mut Context<Self>) {
        let mut next = self.config.clone();
        next.environments.retain(|e| e.label != label);
        if let Err(error) = next.save(&self.config_path) {
            self.message = Some(format!("Saving the hosts failed: {error:#}"));
            cx.notify();
            return;
        }
        self.config = next;
        for task in self.host_tasks.remove(&label).unwrap_or_default() {
            task.abort();
        }
        self.connections.retain(|c| c.label() != label);
        if self
            .current()
            .is_some_and(|(environment, _)| environment == label)
        {
            self.close_thread(cx);
        }
        if self.host_filter.as_deref() == Some(label.as_str()) {
            self.host_filter = None;
        }
        self.shell_dirty = true;
        self.refresh(window, cx);
    }

    fn render_hosts(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let rows = self
            .config
            .environments
            .iter()
            .enumerate()
            .map(|(i, environment)| {
                let label = environment.label.clone();
                let removable = self.config.environments.len() > 1;
                h_flex()
                    .gap_2()
                    .child(div().w(px(120.)).text_sm().child(environment.label.clone()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(environment.http_base_url.clone()),
                    )
                    .when(removable, |row| {
                        row.child(
                            Button::new(("remove-host", i))
                                .label("Remove")
                                .compact()
                                .outline()
                                .on_click(cx.listener(move |root, _, window, cx| {
                                    root.remove_host(label.clone(), window, cx)
                                })),
                        )
                    })
            });
        div()
            .absolute()
            .top(px(80.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                v_flex()
                    .w(px(640.))
                    .p_4()
                    .gap_3()
                    .rounded_md()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .shadow_lg()
                    .child(h_flex().child(div().flex_1().child("Hosts")).child(
                        Button::new("close-hosts")
                            .label("Close")
                            .compact()
                            .outline()
                            .on_click(cx.listener(|root, _, window, cx| root.toggle_hosts(window, cx))),
                    ))
                    .child(v_flex().gap_1().children(rows))
                    .child(div().text_xs().text_color(theme.muted_foreground).child(
                        "Add a host with the pairing URL from `t3 pair`, or its server URL and a pairing code or a token from `t3 auth session issue`. The first host is the primary one.",
                    ))
                    .children(self.host_inputs.iter().map(Input::new))
                    .child(
                        h_flex().justify_end().child(
                            Button::new("add-host")
                                .label("Add host")
                                .compact()
                                .primary()
                                .on_click(cx.listener(|root, _, window, cx| root.add_host(window, cx))),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn toggle_diff(&mut self, cx: &mut Context<Self>) {
        if self
            .file_panel
            .as_ref()
            .is_some_and(|p| p.path == DIFF_TITLE)
        {
            self.file_panel = None;
            cx.notify();
            return;
        }
        let Some((environment, cwd)) = self.composer_cwd() else {
            self.message = Some("Open a thread first.".into());
            cx.notify();
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let preview =
                on_runtime(&runtime, async move { connection.diff_preview(&cwd).await }).await;
            let preview = match preview {
                Ok(preview) => {
                    cx.background_spawn(async move {
                        let lines = completion::diff_lines(&preview);
                        let mut files = Vec::new();
                        if let Some(sources) = preview["sources"].as_array() {
                            for source in sources {
                                let patch = source["diff"].as_str().unwrap_or_default();
                                files.extend(
                                    DiffFile::parse_with(
                                        patch,
                                        DiffParseOptions::new().truncated_hunks(true),
                                    )
                                    .map_err(|error| {
                                        anyhow::anyhow!("Diff parse error: {error:?}")
                                    })?,
                                );
                            }
                        }
                        Ok((lines, files))
                    })
                    .await
                }
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |root, cx| {
                match preview {
                    Ok((lines, files)) => {
                        let text = lines.iter().map(|l| l.text().to_string()).collect();
                        let state = cx.new(|cx| {
                            DiffState::new(files, cx)
                                .with_mode(DiffMode::Unified)
                                .with_syntax_max_line_length(500)
                        });
                        root.file_panel = Some(FilePanel::new(
                            DIFF_TITLE.into(),
                            PanelBody::Diff(state),
                            None,
                            text,
                        ));
                        root.file_find_again(cx);
                    }
                    Err(error) => root.message = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_file(&mut self, path: String, cx: &mut Context<Self>) {
        let Some((environment, cwd)) = self.composer_cwd() else {
            self.message = Some("Open a thread or a new thread first.".into());
            cx.notify();
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let read = on_runtime(&runtime, {
                let path = path.clone();
                async move { connection.read_file(&cwd, &path).await }
            })
            .await;
            let _ = this.update(cx, |root, cx| {
                match read {
                    Ok(file) => {
                        let contents = file
                            .get("contents")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let note = (file.get("truncated").and_then(Value::as_bool) == Some(true))
                            .then(|| {
                                "The file is longer than T3 sends; the rest is left out."
                                    .to_string()
                            });
                        let parts = document_parts(&path, contents, cx);
                        let lines = contents.lines().map(str::to_string).collect();
                        root.file_panel = Some(FilePanel::new(
                            path,
                            PanelBody::Document(parts),
                            note,
                            lines,
                        ));
                        root.file_find_again(cx);
                    }
                    Err(error) => root.message = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_file_picker(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let weak = cx.entity().downgrade();
        let search = weak.clone();
        let closer = move |window: &mut Window, cx: &mut App| {
            let _ = weak.update(cx, |root, cx| {
                root.file_picker_open = false;
                root.composer
                    .update(cx, |state, cx| state.focus(window, cx));
                cx.notify();
            });
        };
        let close_on_cancel = closer.clone();
        div()
            .absolute()
            .top(px(80.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(640.))
                    .rounded_md()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .shadow_lg()
                    .child(
                        Command::new(&self.file_picker)
                            .placeholder("Open a file in the project")
                            .filterable(false)
                            .max_h(px(480.))
                            .items(self.file_hits.iter().map(|path| {
                                CommandItem::new()
                                    .label(path.clone())
                                    .action(Box::new(OpenFile { path: path.clone() }))
                            }))
                            .on_query(move |query, _, cx| {
                                let query = query.to_string();
                                let _ = search.update(cx, |root, cx| root.search_files(query, cx));
                            })
                            .on_confirm(move |_, window, cx| closer(window, cx))
                            .on_cancel(move |window, cx| close_on_cancel(window, cx)),
                    ),
            )
            .into_any_element()
    }

    fn toggle_explorer(&mut self, cx: &mut Context<Self>) {
        if self.explorer.take().is_some() {
            cx.notify();
            return;
        }
        let Some((environment, cwd)) = self.composer_cwd() else {
            self.message = Some("Open a thread or a new thread first.".into());
            cx.notify();
            return;
        };
        self.start_explorer(environment, cwd, cx);
    }

    fn start_explorer(&mut self, environment: String, cwd: String, cx: &mut Context<Self>) {
        self.explorer = Some(Explorer {
            environment,
            cwd,
            directories: HashMap::new(),
            expanded: HashSet::new(),
            error: None,
        });
        self.load_directory(String::new(), cx);
        cx.notify();
    }

    /// Keeps the open explorer on the checkout of the thread or new thread in front.
    fn follow_explorer(&mut self, cx: &mut Context<Self>) {
        let Some(open) = &self.explorer else { return };
        let Some((environment, cwd)) = self.composer_cwd() else {
            return;
        };
        if open.environment != environment || open.cwd != cwd {
            self.start_explorer(environment, cwd, cx);
        }
    }

    fn load_directory(&mut self, directory: String, cx: &mut Context<Self>) {
        let Some(open) = &self.explorer else { return };
        let Some(connection) = self.connection(&open.environment) else {
            return;
        };
        let (environment, cwd) = (open.environment.clone(), open.cwd.clone());
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let listed = on_runtime(&runtime, {
                let (cwd, directory) = (cwd.clone(), directory.clone());
                async move { connection.list_entries(&cwd, &directory).await }
            })
            .await;
            let _ = this.update(cx, |root, cx| {
                let Some(open) = root.explorer.as_mut() else {
                    return;
                };
                if open.environment != environment || open.cwd != cwd {
                    return;
                }
                match listed {
                    Ok(result) => {
                        let entries = explorer::children(&result, &directory);
                        open.directories.insert(directory, entries);
                        open.error = None;
                    }
                    Err(error) => {
                        open.expanded.remove(&directory);
                        open.error = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn click_entry(&mut self, entry: explorer::Entry, cx: &mut Context<Self>) {
        if !entry.directory {
            self.open_file(entry.path, cx);
            return;
        }
        let Some(open) = self.explorer.as_mut() else {
            return;
        };
        if !open.expanded.remove(&entry.path) {
            open.expanded.insert(entry.path.clone());
            if !open.directories.contains_key(&entry.path) {
                self.load_directory(entry.path, cx);
            }
        }
        cx.notify();
    }

    fn render_explorer(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let open = self.explorer.as_ref()?;
        let theme = cx.theme();
        let lines = Rc::new(explorer::lines(&open.directories, &open.expanded));
        let current = self.file_panel.as_ref().map(|p| p.path.clone());
        let (muted, selected) = (theme.muted_foreground, theme.muted);
        let weak = cx.entity().downgrade();
        let loading_root = !open.directories.contains_key("");
        Some(
            v_flex()
                .w(px(260.))
                .h_full()
                .flex_none()
                .border_l_1()
                .border_color(theme.border)
                .child(
                    h_flex()
                        .px_3()
                        .py_2()
                        .gap_2()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(div().flex_1().text_sm().child("Files"))
                        .child(
                            Button::new("close-explorer")
                                .label("Close")
                                .compact()
                                .outline()
                                .on_click(cx.listener(|root, _, _, cx| {
                                    root.explorer = None;
                                    cx.notify();
                                })),
                        ),
                )
                .children(open.error.clone().map(|error| {
                    div()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .text_color(theme.danger)
                        .child(error)
                }))
                .when(loading_root, |pane| {
                    pane.child(
                        div()
                            .px_3()
                            .py_2()
                            .text_xs()
                            .text_color(muted)
                            .child("Loading…"),
                    )
                })
                .child(
                    uniform_list("explorer-lines", lines.len(), move |range, _, _| {
                        range
                            .map(|i| {
                                let line = &lines[i];
                                let mark = match (line.entry.directory, line.expanded) {
                                    (true, true) => "▾ ",
                                    (true, false) => "▸ ",
                                    _ => "  ",
                                };
                                let label = format!(
                                    "{mark}{}{}",
                                    line.entry.name(),
                                    if line.loading { "  …" } else { "" }
                                );
                                let entry = line.entry.clone();
                                let weak = weak.clone();
                                div()
                                    .id(SharedString::from(format!("explorer-{}", line.entry.path)))
                                    .pl(px(12. + 14. * line.depth as f32))
                                    .pr_2()
                                    .text_sm()
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .cursor_pointer()
                                    .when(current.as_deref() == Some(&line.entry.path), |d| {
                                        d.bg(selected)
                                    })
                                    .hover(move |d| d.bg(selected))
                                    .on_click(move |_, _, cx| {
                                        let _ = weak.update(cx, |root, cx| {
                                            root.click_entry(entry.clone(), cx)
                                        });
                                    })
                                    .child(label)
                                    .into_any_element()
                            })
                            .collect()
                    })
                    .flex_1(),
                )
                .into_any_element(),
        )
    }

    fn render_file_panel(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let panel = self.file_panel.as_ref()?;
        let theme = cx.theme();
        let finding = self.file_find_open && !self.file_find_input.read(cx).value().is_empty();
        let count = if panel.hits.is_empty() {
            "No match".to_string()
        } else {
            format!("{} / {}", panel.hit_at + 1, panel.hits.len())
        };
        let body = if finding {
            self.render_found_lines(panel, cx)
        } else {
            match &panel.body {
                PanelBody::Document(parts) => div()
                    .id("file-view")
                    .flex_1()
                    .overflow_y_scroll()
                    .p_3()
                    .children(
                        parts
                            .iter()
                            .enumerate()
                            .map(|(ix, part)| self.render_panel_part(ix, part, cx)),
                    )
                    .into_any_element(),
                PanelBody::Diff(state) => div().flex_1().child(Diff::new(state)).into_any_element(),
            }
        };
        Some(
            v_flex()
                .track_focus(&self.panel_focus)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|root, _, window, cx| {
                        if !root.panel_focus.contains_focused(window, cx) {
                            root.panel_focus.focus(window, cx);
                        }
                    }),
                )
                .w(px(620.))
                .h_full()
                .flex_none()
                .border_l_1()
                .border_color(theme.border)
                .child(
                    h_flex()
                        .px_3()
                        .py_2()
                        .gap_2()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .child(panel.path.clone()),
                        )
                        .child(
                            Button::new("find-file")
                                .label("Find")
                                .compact()
                                .outline()
                                .on_click(cx.listener(|root, _, window, cx| {
                                    root.open_file_find(window, cx)
                                })),
                        )
                        .child(
                            Button::new("close-file")
                                .label("Close")
                                .compact()
                                .outline()
                                .on_click(cx.listener(|root, _, window, cx| {
                                    root.close_file_panel(window, cx)
                                })),
                        ),
                )
                .when(self.file_find_open, |panel| {
                    panel.child(
                        h_flex()
                            .px_3()
                            .py_1()
                            .gap_2()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(div().flex_1().child(Input::new(&self.file_find_input)))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(count),
                            )
                            .child(
                                Button::new("file-find-previous")
                                    .label("Previous")
                                    .compact()
                                    .outline()
                                    .on_click(
                                        cx.listener(|root, _, _, cx| {
                                            root.step_file_find(false, cx)
                                        }),
                                    ),
                            )
                            .child(
                                Button::new("file-find-next")
                                    .label("Next")
                                    .compact()
                                    .outline()
                                    .on_click(
                                        cx.listener(|root, _, _, cx| root.step_file_find(true, cx)),
                                    ),
                            ),
                    )
                })
                .children(panel.note.clone().map(|note| {
                    div()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(note)
                }))
                .child(body)
                .into_any_element(),
        )
    }

    fn render_panel_part(&self, ix: usize, part: &PanelPart, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (rows, header_rows, sort) = match part {
            PanelPart::Text(view) => {
                return TextView::new(view).selectable(true).into_any_element();
            }
            PanelPart::Table {
                rows,
                header_rows,
                sort,
            } => (rows, *header_rows, *sort),
        };
        let columns = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let cell = |text: String, header: bool| {
            div()
                .px_2()
                .py_1()
                .border_b_1()
                .border_color(theme.border)
                .text_sm()
                .when(header, |d| d.font_weight(FontWeight::BOLD))
                .child(text)
        };
        let mut grid = div().grid().grid_cols(columns as u16).w_full();
        for row in rows.iter().take(header_rows) {
            for column in 0..columns {
                let text = row.get(column).cloned().unwrap_or_default();
                let mark = match sort {
                    Some(tables::Sort::Ascending(c)) if c == column => " ↑",
                    Some(tables::Sort::Descending(c)) if c == column => " ↓",
                    _ => "",
                };
                grid = grid.child(
                    cell(format!("{text}{mark}"), true)
                        .id(SharedString::from(format!("table-{ix}-head-{column}")))
                        .cursor_pointer()
                        .hover(|d| d.bg(theme.muted))
                        .on_click(
                            cx.listener(move |root, _, _, cx| root.sort_table(ix, column, cx)),
                        ),
                );
            }
        }
        for row in tables::sorted(rows, header_rows, sort) {
            for column in 0..columns {
                grid = grid.child(cell(row.get(column).cloned().unwrap_or_default(), false));
            }
        }
        v_flex()
            .my_2()
            .gap_1()
            .child(
                h_flex()
                    .gap_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("Header rows: {header_rows}"))
                    .child(
                        Button::new(SharedString::from(format!("table-{ix}-fewer")))
                            .label("−")
                            .compact()
                            .ghost()
                            .on_click(cx.listener(move |root, _, _, cx| {
                                root.change_header_rows(ix, false, cx)
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("table-{ix}-more")))
                            .label("+")
                            .compact()
                            .ghost()
                            .on_click(cx.listener(move |root, _, _, cx| {
                                root.change_header_rows(ix, true, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .border_1()
                    .border_color(theme.border)
                    .rounded_md()
                    .child(grid),
            )
            .into_any_element()
    }

    fn render_found_lines(&self, panel: &FilePanel, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let lines = panel.lines.clone();
        let hits = panel.hits.clone();
        let current = hits.get(panel.hit_at).cloned();
        let (plain, muted, mark, strong) = (
            theme.foreground,
            theme.muted_foreground,
            theme.warning.opacity(0.35),
            theme.warning,
        );
        let mono = theme.mono_font_family.clone();
        let width = lines.len().to_string().len();
        uniform_list("file-find-lines", lines.len(), move |range, _, _| {
            range
                .map(|n| {
                    let from = hits.partition_point(|(line, _)| *line < n);
                    let highlights: Vec<_> = hits[from..]
                        .iter()
                        .take_while(|(line, _)| *line == n)
                        .map(|(line, bytes)| {
                            let here = current.as_ref() == Some(&(*line, bytes.clone()));
                            (
                                bytes.clone(),
                                HighlightStyle {
                                    background_color: Some(if here { strong } else { mark }),
                                    ..Default::default()
                                },
                            )
                        })
                        .collect();
                    h_flex()
                        .px_3()
                        .gap_3()
                        .text_xs()
                        .whitespace_nowrap()
                        .child(div().text_color(muted).child(format!("{:>width$}", n + 1)))
                        .child(
                            div().text_color(plain).child(
                                StyledText::new(lines[n].clone()).with_highlights(highlights),
                            ),
                        )
                        .into_any_element()
                })
                .collect()
        })
        .track_scroll(&panel.scroll)
        .font_family(mono)
        .flex_1()
        .into_any_element()
    }

    fn sort_table(&mut self, part: usize, column: usize, cx: &mut Context<Self>) {
        if let Some(PanelPart::Table { sort, .. }) = self.panel_part(part) {
            *sort = tables::Sort::next(*sort, column);
            cx.notify();
        }
    }

    fn change_header_rows(&mut self, part: usize, more: bool, cx: &mut Context<Self>) {
        if let Some(PanelPart::Table {
            rows, header_rows, ..
        }) = self.panel_part(part)
        {
            *header_rows = if more {
                (*header_rows + 1).min(rows.len().saturating_sub(1)).max(1)
            } else {
                header_rows.saturating_sub(1)
            };
            cx.notify();
        }
    }

    fn panel_part(&mut self, part: usize) -> Option<&mut PanelPart> {
        match &mut self.file_panel.as_mut()?.body {
            PanelBody::Document(parts) => parts.get_mut(part),
            PanelBody::Diff(_) => None,
        }
    }

    fn close_file_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.file_panel = None;
        self.file_find_open = false;
        self.composer
            .update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    fn open_file_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_panel.is_none() {
            return;
        }
        self.file_find_open = true;
        self.file_find_input
            .update(cx, |state, cx| state.focus(window, cx));
        self.file_find_again(cx);
    }

    fn close_file_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.file_find_open = false;
        self.panel_focus.focus(window, cx);
        cx.notify();
    }

    fn file_find_again(&mut self, cx: &mut Context<Self>) {
        let query = self.file_find_input.read(cx).value().to_string();
        let Some(panel) = self.file_panel.as_mut() else {
            return;
        };
        panel.hits = Rc::new(completion::find_in_lines(&panel.lines, &query));
        panel.hit_at = 0;
        reveal_hit(panel);
        cx.notify();
    }

    fn step_file_find(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(panel) = self.file_panel.as_mut() else {
            return;
        };
        let n = panel.hits.len();
        if n == 0 {
            return;
        }
        panel.hit_at = if forward {
            (panel.hit_at + 1) % n
        } else {
            (panel.hit_at + n - 1) % n
        };
        reveal_hit(panel);
        cx.notify();
    }

    fn toggle_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = !self.palette_open;
        if self.palette_open {
            self.palette.update(cx, |state, cx| {
                state.set_query("", window, cx);
                state.focus(window, cx);
            });
        } else {
            self.composer
                .update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    fn render_palette(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let commands: [(&str, Box<dyn Action>); 11] = [
            ("New thread", Box::new(NewThread)),
            ("Next Done or waiting thread", Box::new(NextDone)),
            ("Settle the thread", Box::new(Settle)),
            ("Pin or unpin the thread", Box::new(TogglePin)),
            ("Find in thread", Box::new(Find)),
            ("Toggle the file explorer", Box::new(ToggleExplorer)),
            ("Pick a model", Box::new(ToggleModelPicker)),
            ("Cycle the runtime mode", Box::new(CycleRuntimeMode)),
            ("Toggle the sidebar", Box::new(ToggleSidebar)),
            ("Manage hosts", Box::new(ManageHosts)),
            ("Settings", Box::new(OpenSettings)),
        ];
        let threads = self.entries.iter().map(|entry| {
            CommandItem::new()
                .label(format!("{}  ·  {}", entry.title, entry.environment))
                .keywords([entry.project_title.clone()])
                .action(Box::new(OpenThreadAction {
                    environment: entry.environment.clone(),
                    thread_id: entry.thread_id.clone(),
                }))
        });
        let weak = cx.entity().downgrade();
        let closer = move |window: &mut Window, cx: &mut App| {
            let _ = weak.update(cx, |root, cx| {
                root.palette_open = false;
                root.composer
                    .update(cx, |state, cx| state.focus(window, cx));
                cx.notify();
            });
        };
        let close_on_cancel = closer.clone();
        div()
            .absolute()
            .top(px(80.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(640.))
                    .rounded_md()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .shadow_lg()
                    .child(
                        Command::new(&self.palette)
                            .placeholder("Search threads and commands")
                            .max_h(px(480.))
                            .items(
                                commands
                                    .into_iter()
                                    .map(|(label, action)| {
                                        CommandItem::new().label(label).action(action)
                                    })
                                    .chain(threads),
                            )
                            .on_confirm(move |_, window, cx| closer(window, cx))
                            .on_cancel(move |window, cx| close_on_cancel(window, cx)),
                    ),
            )
            .into_any_element()
    }

    fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open.is_none() {
            return;
        }
        self.find_open = true;
        self.find_input
            .update(cx, |state, cx| state.focus(window, cx));
        self.find_again(true, cx);
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_open = false;
        self.find_hits.clear();
        self.mark_found(cx);
        self.composer
            .update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// Finds the query in the rows again, and with `jump` scrolls to the first hit.
    fn find_again(&mut self, jump: bool, cx: &mut Context<Self>) {
        if !self.find_open {
            return;
        }
        let query = self.find_input.read(cx).value().to_string();
        self.find_hits = timeline::find_rows(&self.rows, &query);
        if jump {
            self.find_at = 0;
        }
        self.find_at = self.find_at.min(self.find_hits.len().saturating_sub(1));
        if jump {
            self.reveal_find();
        }
        self.mark_found(cx);
        cx.notify();
    }

    fn step_find(&mut self, forward: bool, cx: &mut Context<Self>) {
        let n = self.find_hits.len();
        if n == 0 {
            return;
        }
        self.find_at = if forward {
            (self.find_at + 1) % n
        } else {
            (self.find_at + n - 1) % n
        };
        self.reveal_find();
        self.mark_found(cx);
        cx.notify();
    }

    fn reveal_find(&self) {
        if let Some(&item_ix) = self.find_hits.get(self.find_at) {
            self.list_state.scroll_to(ListOffset {
                item_ix,
                offset_in_item: px(0.),
            });
        }
    }

    fn mark_found(&self, cx: &mut Context<Self>) {
        let current = self.find_hits.get(self.find_at).copied();
        for (i, view) in self.row_views.iter().enumerate() {
            view.update(cx, |view, cx| view.set_found(Some(i) == current, cx));
        }
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
        if text.is_empty() && self.attachments.is_empty() {
            return;
        }
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let delivery = match self.open_entry().map(|e| e.status) {
            Some(ThreadStatus::Working) if self.settings.steer_follow_ups => Delivery::Steer,
            Some(ThreadStatus::Working | ThreadStatus::Input | ThreadStatus::Approval) => {
                Delivery::Queue
            }
            _ => Delivery::Start,
        };
        let runtime = self.runtime.clone();
        let pending = std::mem::take(&mut self.attachments);
        self.composer
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.stash_composer(cx);
        cx.spawn_in(window, async move |this, cx| {
            let sent = on_runtime(&runtime, {
                let text = text.clone();
                let pending = pending.clone();
                async move {
                    let attachments = upload_all(&connection, &pending).await?;
                    connection
                        .send_message(&thread_id, &text, attachments, delivery)
                        .await
                }
            })
            .await;
            let _ = this.update_in(cx, |root, window, cx| {
                root.message = sent.as_ref().err().map(|e| format!("Send failed: {e:#}"));
                if sent.is_err() {
                    root.composer
                        .update(cx, |state, cx| state.set_value(text, window, cx));
                    root.stash_composer(cx);
                    root.attachments = pending;
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
        self.stash_composer(cx);
        let runtime = self.runtime.clone();
        let pending = std::mem::take(&mut self.attachments);
        cx.spawn_in(window, async move |this, cx| {
            let launched = on_runtime(&runtime, {
                let text = text.clone();
                let pending = pending.clone();
                async move {
                    let attachments = upload_all(&connection, &pending).await?;
                    connection
                        .launch_thread(Launch {
                            thread_id: &thread_id,
                            project_id: &project_id,
                            title: &title,
                            model,
                            runtime_mode: &runtime_mode,
                            interaction_mode: &interaction_mode,
                            attachments,
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
                    root.stash_composer(cx);
                    root.attachments = pending;
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

    fn toggle_model_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picker_open = !self.picker_open;
        if self.picker_open {
            self.picker_instance =
                self.composer_target()
                    .and_then(|(_, m, _, _)| m)
                    .and_then(|m| {
                        m.get("instanceId")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    });
            self.model_picker.update(cx, |state, cx| {
                state.set_query("", window, cx);
                state.focus(window, cx);
            });
        } else {
            self.composer
                .update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    fn choose_attachments(&mut self, cx: &mut Context<Self>) {
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach files".into()),
        });
        cx.spawn(async move |this, cx| {
            let result = selected.await;
            let _ = this.update(cx, |root, cx| {
                match result {
                    Ok(Ok(Some(paths))) => {
                        root.attach_paths(&ExternalPaths(paths.into_iter().collect()), cx)
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => {
                        root.message = Some(format!("Choosing attachments failed: {error:#}"))
                    }
                    Err(error) => {
                        root.message = Some(format!("Choosing attachments failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_runtime_mode(&mut self, mode: String, cx: &mut Context<Self>) {
        if let Some(draft) = self.draft.as_mut() {
            draft.runtime_mode = Some(mode);
            cx.notify();
        } else {
            self.dispatch_on_open("Runtime mode change", cx, move |connection, thread_id| {
                Box::pin(async move { connection.set_runtime_mode(&thread_id, &mode).await })
            });
        }
    }

    fn set_model_option(&mut self, id: &str, value: Value, cx: &mut Context<Self>) {
        let Some((_, Some(current), _, _)) = self.composer_target() else {
            return;
        };
        let Some(chosen) = models::with_option(&current, id, value) else {
            self.message = Some("The model's option selection is malformed".into());
            cx.notify();
            return;
        };
        if let Some(draft) = self.draft.as_mut() {
            draft.model = Some(chosen);
            cx.notify();
        } else {
            self.dispatch_on_open("Model option change", cx, move |connection, thread_id| {
                Box::pin(async move { connection.set_model(&thread_id, chosen, false).await })
            });
        }
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
        let sidebar_active = crate::appearance::sidebar_active(cx);
        let sidebar_hover = crate::appearance::sidebar_hover(cx);
        let sidebar_muted = crate::appearance::sidebar_muted(cx);
        let statuses = v_flex()
            .gap_0p5()
            .p_2()
            .children(self.connections.iter().map(|c| {
                let (color, text) = match c.status() {
                    ConnectionStatus::Connected => (theme.success, "connected".to_string()),
                    ConnectionStatus::Connecting => (sidebar_muted, "connecting".to_string()),
                    ConnectionStatus::Disconnected(reason) => (theme.danger, reason),
                };
                h_flex()
                    .gap_2()
                    .text_xs()
                    .child(div().size_2().rounded_full().bg(color))
                    .child(c.label().to_string())
                    .child(div().text_color(sidebar_muted).truncate().child(text))
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
        let has_settled = shelves
            .iter()
            .any(|shelf| shelf.section == sidebar::Section::Settled);
        let mut lower_started = false;
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut index = 0usize;
        for shelf in shelves {
            let section = shelf.section;
            let lower = matches!(
                section,
                sidebar::Section::Working | sidebar::Section::Snoozed | sidebar::Section::Settled
            );
            let bottom = lower && !lower_started;
            lower_started |= lower;
            let collapsed = self.collapsed.contains(&section);
            if !matches!(section, sidebar::Section::Pinned | sidebar::Section::Active) {
                rows.push(
                    h_flex()
                        .id(SharedString::from(format!("shelf-{}", section.label())))
                        .px_2()
                        .pt_2()
                        .pb_1()
                        .gap_1()
                        .text_xs()
                        .text_color(sidebar_muted)
                        .cursor_pointer()
                        .on_click(cx.listener(move |root, _, _, cx| {
                            if !root.collapsed.remove(&section) {
                                root.collapsed.insert(section);
                            }
                            cx.notify();
                        }))
                        .when(bottom, |row| row.mt_auto())
                        .child(section.label())
                        .child(div().flex_1())
                        .child(format!("{}", shelf.rows.len()))
                        .child(if collapsed { "▸" } else { "▾" })
                        .into_any_element(),
                );
            }
            if collapsed && !matches!(section, sidebar::Section::Pinned | sidebar::Section::Active)
            {
                continue;
            }
            for (entry, depth, family) in
                sidebar::visible_rows(&shelf.rows, &self.collapsed_families)
            {
                index += 1;
                let family_key = entry.key();
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
                    v_flex()
                        .id(("thread", index))
                        .w_full()
                        .pl(px(crate::appearance::ROW_INSET + 16. * depth as f32))
                        .pr_2()
                        .py_2()
                        .gap_1()
                        .rounded_md()
                        .when(selected, |row| row.bg(sidebar_active))
                        .when(settled || snoozed, |row| row.text_color(sidebar_muted))
                        .hover(|row| {
                            row.bg(if selected {
                                sidebar_active
                            } else {
                                sidebar_hover
                            })
                        })
                        .on_click(cx.listener(move |root, _, _, cx| {
                            root.open_thread(environment.clone(), thread_id.clone(), cx)
                        }))
                        .child(
                            h_flex()
                                .w_full()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(sidebar_muted)
                                        .when(remote, |chip| {
                                            chip.px_1()
                                                .rounded_sm()
                                                .bg(theme.warning.opacity(0.15))
                                                .text_color(theme.warning)
                                        })
                                        .child(entry.environment.clone()),
                                )
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(dot.unwrap_or(sidebar_muted))
                                        .child(if entry.status == ThreadStatus::Idle {
                                            humantime::parse_rfc3339(
                                                entry
                                                    .latest_user_message_at
                                                    .as_deref()
                                                    .unwrap_or(&entry.updated_at),
                                            )
                                            .ok()
                                            .and_then(|time| {
                                                SystemTime::now().duration_since(time).ok()
                                            })
                                            .map(|age| {
                                                if age.as_secs() >= 3600 {
                                                    format!("{}h", age.as_secs() / 3600)
                                                } else {
                                                    format!("{}m", age.as_secs() / 60)
                                                }
                                            })
                                            .unwrap_or_default()
                                        } else {
                                            entry.status.label().to_string()
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .child(entry.title.clone()),
                        )
                        .when_some(family, |row, family| {
                            let label = match (family.collapsed, family.working) {
                                (true, working) if working > 0 => {
                                    format!("▸ {} · {working} working", family.children)
                                }
                                (true, _) => format!("▸ {}", family.children),
                                (false, _) => format!("▾ {}", family.children),
                            };
                            let key = family_key.clone();
                            row.child(
                                div()
                                    .id(("family", index))
                                    .flex_none()
                                    .px_1()
                                    .text_xs()
                                    .text_color(sidebar_muted)
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |root, _, _, cx| {
                                        cx.stop_propagation();
                                        if !root.collapsed_families.remove(&key) {
                                            root.collapsed_families.insert(key.clone());
                                        }
                                        cx.notify();
                                    }))
                                    .child(label),
                            )
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
        if !has_settled {
            rows.push(
                h_flex()
                    .px_2()
                    .py_2()
                    .gap_2()
                    .text_xs()
                    .text_color(sidebar_muted)
                    .when(!lower_started, |row| row.mt_auto())
                    .child("Settled")
                    .child(div().flex_1())
                    .child("0")
                    .child("▸")
                    .into_any_element(),
            );
        }
        v_flex()
            .w(px(self.settings.sidebar_width))
            .h_full()
            .flex_none()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .child(
                h_flex()
                    .h(px(40.))
                    .px_2()
                    .gap_1()
                    .child(
                        Button::new("collapse-sidebar")
                            .icon(gpui_kit::assets::IconName::PanelLeftClose)
                            .ghost()
                            .size(px(24.))
                            .tooltip("Toggle sidebar")
                            .accessibility_label("Toggle sidebar")
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.sidebar_hidden = true;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .ml_4()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("T3 Code"),
                    )
                    .child(
                        Button::new("history-back")
                            .icon(IconName::ArrowLeft)
                            .ghost()
                            .size(px(24.))
                            .disabled(!self.history.can_back())
                            .tooltip("Back")
                            .accessibility_label("Back")
                            .on_click(cx.listener(|_, _, window, cx| {
                                window.dispatch_action(Box::new(HistoryBack), cx)
                            })),
                    )
                    .child(
                        Button::new("history-forward")
                            .icon(IconName::ArrowRight)
                            .ghost()
                            .size(px(24.))
                            .disabled(!self.history.can_forward())
                            .tooltip("Forward")
                            .accessibility_label("Forward")
                            .on_click(cx.listener(|_, _, window, cx| {
                                window.dispatch_action(Box::new(HistoryForward), cx)
                            })),
                    ),
            )
            .child(
                h_flex()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .child(
                        h_flex()
                            .id("sidebar-search")
                            .h_8()
                            .px_2()
                            .gap_2()
                            .rounded_md()
                            .cursor_pointer()
                            .text_sm()
                            .text_color(sidebar_muted)
                            .hover(|row| row.bg(theme.list_hover))
                            .flex_1()
                            .child(IconName::Search)
                            .child("Search")
                            .on_click(
                                cx.listener(|root, _, window, cx| root.toggle_palette(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("sidebar-new")
                            .icon(gpui_kit::assets::IconName::SquarePen)
                            .ghost()
                            .size(px(24.))
                            .tooltip("New thread")
                            .accessibility_label("New thread")
                            .on_click(
                                cx.listener(|root, _, window, cx| root.new_thread(window, cx)),
                            ),
                    ),
            )
            .children(filters)
            .child(
                v_flex()
                    .id("threads")
                    .flex_1()
                    .overflow_y_scroll()
                    .p_2()
                    .children(rows),
            )
            .child(statuses)
            .child(
                h_flex()
                    .px_2()
                    .pt_1()
                    .gap_1()
                    .child(
                        Button::new("hosts")
                            .icon(IconName::Globe)
                            .tooltip("Connections")
                            .accessibility_label("Connections")
                            .compact()
                            .ghost()
                            .on_click(
                                cx.listener(|root, _, window, cx| root.toggle_hosts(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("settings")
                            .icon(IconName::Settings)
                            .tooltip("Settings")
                            .accessibility_label("Settings")
                            .compact()
                            .ghost()
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.settings_open = !root.settings_open;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn render_queue(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if self.queued.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let rows = self.queued.iter().enumerate().map(|(i, queued)| {
            let steer_run = queued.run_id.clone();
            let cancel_run = queued.run_id.clone();
            h_flex()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child("Queued"),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .child(queued.text.clone()),
                )
                .child(
                    Button::new(("queue-steer", i))
                        .label("Steer now")
                        .compact()
                        .outline()
                        .on_click(cx.listener(move |root, _, _, cx| {
                            root.steer_queued(steer_run.clone(), cx)
                        })),
                )
                .child(
                    Button::new(("queue-cancel", i))
                        .label("Cancel")
                        .compact()
                        .outline()
                        .on_click(cx.listener(move |root, _, _, cx| {
                            root.cancel_queued(cancel_run.clone(), cx)
                        })),
                )
        });
        Some(
            v_flex()
                .px_3()
                .pt_2()
                .gap_1()
                .children(rows)
                .into_any_element(),
        )
    }

    fn steer_queued(&mut self, run_id: String, cx: &mut Context<Self>) {
        let Some((environment, thread_id)) = self.current() else {
            return;
        };
        let Some(active) = self.thread_value(&environment, &thread_id).and_then(|t| {
            t.get("activeRunId")
                .and_then(Value::as_str)
                .map(str::to_string)
        }) else {
            self.message = Some("No run is active to steer.".into());
            cx.notify();
            return;
        };
        self.dispatch_on_open("Steering", cx, move |connection, thread_id| {
            Box::pin(async move { connection.steer_queued(&thread_id, &run_id, &active).await })
        });
    }

    fn cancel_queued(&mut self, run_id: String, cx: &mut Context<Self>) {
        self.dispatch_on_open(
            "Cancelling the queued message",
            cx,
            move |connection, thread_id| {
                Box::pin(async move { connection.cancel_queued(&thread_id, &run_id).await })
            },
        );
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

    fn composer_button(&self, cx: &Context<Self>) -> Button {
        if self.draft.is_some() {
            return Button::new("launch")
                .icon(IconName::ArrowUp)
                .size(px(32.))
                .rounded(px(16.))
                .disabled(
                    self.composer.read(cx).value().trim().is_empty() && self.attachments.is_empty(),
                )
                .when(
                    self.composer.read(cx).value().trim().is_empty() && self.attachments.is_empty(),
                    |button| {
                        button
                            .bg(cx.theme().foreground.opacity(0.12))
                            .text_color(cx.theme().muted_foreground)
                    },
                )
                .tooltip("Send")
                .accessibility_label("Send")
                .primary()
                .on_click(cx.listener(|root, _, window, cx| root.launch(window, cx)));
        }
        let working = self.open_entry().is_some_and(|entry| {
            matches!(
                entry.status,
                ThreadStatus::Working | ThreadStatus::Input | ThreadStatus::Approval
            )
        });
        if self.pending_question().is_some() {
            Button::new("answer")
                .label("Answer")
                .primary()
                .on_click(cx.listener(|root, _, window, cx| root.send(window, cx)))
        } else if working {
            Button::new("stop")
                .icon(Icon::empty().path("icons/t3-stop.svg"))
                .tooltip("Interrupt")
                .accessibility_label("Stop")
                .size(px(32.))
                .rounded(px(16.))
                .danger()
                .on_click(cx.listener(|root, _, _, cx| root.stop(cx)))
        } else {
            Button::new("send")
                .icon(IconName::ArrowUp)
                .disabled(
                    self.composer.read(cx).value().trim().is_empty() && self.attachments.is_empty(),
                )
                .when(
                    self.composer.read(cx).value().trim().is_empty() && self.attachments.is_empty(),
                    |button| {
                        button
                            .bg(cx.theme().foreground.opacity(0.12))
                            .text_color(cx.theme().muted_foreground)
                    },
                )
                .tooltip("Send")
                .accessibility_label("Send")
                .size(px(32.))
                .rounded(px(16.))
                .primary()
                .on_click(cx.listener(|root, _, window, cx| root.send(window, cx)))
        }
    }

    fn render_composer_controls(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let target = self.composer_target();
        // Provider catalogs change on config events, not on each keystroke.
        let catalog = target
            .as_ref()
            .and_then(|(env, ..)| self.model_catalogs.get(env));
        let config = catalog.map(|(config, _)| config);
        let instances = catalog
            .map(|(_, instances)| instances.as_slice())
            .unwrap_or_default();
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
        let selected_instance = instances.iter().find(|i| i.instance_id == current_instance);
        let selected_model = selected_instance.and_then(|instance| {
            instance
                .models
                .iter()
                .find(|model| model.slug == current_slug)
        });
        let label = selected_model
            .zip(current.as_ref())
            .map(|(model, selection)| models::trigger_label(model, selection))
            .unwrap_or_else(|| "Pick a model".into());
        let toolbar = h_flex()
            .px_2()
            .pb_2()
            .gap_1()
            .w_full()
            .child(
                Button::new("attach-files")
                    .icon(
                        Icon::new(gpui_kit::assets::IconName::Paperclip)
                            .text_color(theme.muted_foreground),
                    )
                    .ghost()
                    .size(px(32.))
                    .tooltip("Attach files")
                    .accessibility_label("Attach files")
                    .on_click(cx.listener(|root, _, _, cx| root.choose_attachments(cx))),
            )
            .child(div().flex_1())
            .child(
                Button::new("model")
                    .label(label)
                    .text_color(theme.foreground.opacity(0.8))
                    .icon(
                        Icon::empty()
                            .path(match selected_instance.map(|i| i.driver.as_str()) {
                                Some("codex") => "icons/t3-openai.svg",
                                Some("claudeAgent") => "icons/t3-claude.svg",
                                _ => "icons/bot.svg",
                            })
                            .text_color(theme.muted_foreground),
                    )
                    .compact()
                    .ghost()
                    .h(px(28.))
                    .text_sm()
                    .tooltip(
                        current
                            .as_ref()
                            .map(|selection| {
                                let mut label = models::label(instances, selection);
                                if let Some(config) = &config {
                                    for (window, percent) in
                                        models::usage(config, &current_instance)
                                    {
                                        label.push_str(&format!("\n{window}: {percent:.0}% used"));
                                    }
                                }
                                label
                            })
                            .unwrap_or_else(|| "Choose model and permissions".into()),
                    )
                    .selected(self.picker_open)
                    .on_click(
                        cx.listener(|root, _, window, cx| root.toggle_model_picker(window, cx)),
                    ),
            );
        let picker = self.picker_open.then(|| {
            let shown = instances
                .iter()
                .find(|i| Some(&i.instance_id) == self.picker_instance.as_ref())
                .or(selected_instance);
            let group = CommandGroup::new().items(shown.into_iter().flat_map(|instance| {
                instance.models.iter().map(|model| {
                    CommandItem::new()
                        .label(model.full_name.clone())
                        .checked(
                            instance.instance_id == current_instance && model.slug == current_slug,
                        )
                        .action(Box::new(PickModelAction {
                            instance_id: instance.instance_id.clone(),
                            slug: model.slug.clone(),
                        }))
                })
            }));
            let (runtime_mode, interaction_mode) = target
                .as_ref()
                .map(|(_, _, r, i)| (r.clone(), i.clone()))
                .unwrap_or_default();
            let modes = selected_instance
                .map(|i| i.runtime_modes.clone())
                .unwrap_or_default();
            let current_mode = runtime_mode.clone();
            let runtime_label = match runtime_mode.as_str() {
                "full-access" => "Full access",
                "approval-required" => "Supervised",
                "auto-accept-edits" => "Auto-accept edits",
                _ => "Auto",
            };
            let weak = cx.entity().downgrade();
            let close = move |window: &mut Window, cx: &mut App| {
                let _ = weak.update(cx, |root, cx| {
                    root.picker_open = false;
                    root.composer
                        .update(cx, |state, cx| state.focus(window, cx));
                    cx.notify();
                });
            };
            let cancel = close.clone();
            let reasoning = selected_model.and_then(|model| {
                model.option_descriptors.iter().find(|d| {
                    d.get("role").and_then(Value::as_str) == Some("reasoning")
                        || d.get("id").and_then(Value::as_str) == Some("reasoningEffort")
                })
            });
            let reasoning_column = reasoning.map(|descriptor| {
                let id = descriptor
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let selected = current
                    .as_ref()
                    .and_then(|selection| models::option_value(selection, descriptor));
                v_flex()
                    .w(px(192.))
                    .h_full()
                    .border_l_1()
                    .border_color(theme.border.opacity(0.7))
                    .bg(theme.muted.opacity(0.4))
                    .p_2()
                    .child(
                        div()
                            .h(px(34.))
                            .px_1()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child("Reasoning"),
                    )
                    .children(
                        descriptor
                            .get("options")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .map(|option| {
                                let value = option.get("id").cloned().unwrap_or(Value::Null);
                                let checked = Some(&value) == selected.as_ref();
                                let id = id.clone();
                                h_flex()
                                    .id(SharedString::from(format!("reasoning-{value}")))
                                    .w_full()
                                    .h(px(34.))
                                    .px_2()
                                    .rounded_sm()
                                    .text_sm()
                                    .cursor_pointer()
                                    .hover(|row| row.bg(theme.list_hover))
                                    .child(
                                        option
                                            .get("label")
                                            .and_then(Value::as_str)
                                            .unwrap_or_default()
                                            .to_string(),
                                    )
                                    .child(div().flex_1())
                                    .when(checked, |row| {
                                        row.child(Icon::new(IconName::Check).size(px(14.)))
                                    })
                                    .on_click(cx.listener(move |root, _, _, cx| {
                                        root.set_model_option(&id, value.clone(), cx)
                                    }))
                            }),
                    )
            });
            v_flex()
                .id("model-picker")
                .absolute()
                .bottom(px(40.))
                .right(px(48.))
                .mb_1()
                .w(px(if reasoning.is_some() { 554. } else { 362. }))
                .h(px(392.))
                .rounded_md()
                .border_1()
                .border_color(theme.border)
                .bg(theme.popover)
                .shadow_lg()
                .occlude()
                .child(
                    h_flex()
                        .flex_1()
                        .min_h_0()
                        .items_start()
                        .child(
                            v_flex()
                                .w(px(44.))
                                .h_full()
                                .border_r_1()
                                .border_color(theme.border)
                                .py_2()
                                .gap_1()
                                .children(instances.iter().map(|instance| {
                                    let id = instance.instance_id.clone();
                                    Button::new(SharedString::from(format!("picker-provider-{id}")))
                                        .icon(Icon::empty().path(match instance.driver.as_str() {
                                            "codex" => "icons/t3-openai.svg",
                                            "claudeAgent" => "icons/t3-claude.svg",
                                            _ => "icons/bot.svg",
                                        }))
                                        .ghost()
                                        .size(px(36.))
                                        .tooltip(instance.name.clone())
                                        .selected(shown.is_some_and(|i| i.instance_id == id))
                                        .on_click(cx.listener(move |root, _, window, cx| {
                                            root.picker_instance = Some(id.clone());
                                            root.model_picker.update(cx, |state, cx| {
                                                state.set_query("", window, cx);
                                                state.focus(window, cx);
                                            });
                                            cx.notify();
                                        }))
                                })),
                        )
                        .child(
                            div().flex_1().min_w_0().h_full().child(
                                Command::new(&self.model_picker)
                                    .placeholder("Search models...")
                                    .max_h(px(300.))
                                    .group(group)
                                    .on_confirm(move |_, window, cx| close(window, cx))
                                    .on_cancel(move |window, cx| cancel(window, cx)),
                            ),
                        )
                        .children(reasoning_column),
                )
                .child(
                    h_flex()
                        .h(px(44.))
                        .bg(theme.foreground.opacity(0.025))
                        .px_2()
                        .gap_1()
                        .border_t_1()
                        .border_color(theme.foreground.opacity(0.07))
                        .children(
                            selected_model
                                .into_iter()
                                .flat_map(|model| &model.option_descriptors)
                                .filter(|d| {
                                    d.get("role").and_then(Value::as_str) != Some("reasoning")
                                        && d.get("id").and_then(Value::as_str)
                                            != Some("reasoningEffort")
                                })
                                .filter_map(|descriptor| {
                                    let id = descriptor.get("id")?.as_str()?.to_string();
                                    let selected = current.as_ref().and_then(|selection| {
                                        models::option_value(selection, descriptor)
                                    });
                                    if descriptor.get("type").and_then(Value::as_str)
                                        == Some("boolean")
                                    {
                                        let checked =
                                            selected.and_then(|v| v.as_bool()).unwrap_or(false);
                                        return Some(
                                            h_flex().child(
                                                Button::new(SharedString::from(format!(
                                                    "option-{id}"
                                                )))
                                                .label(
                                                    descriptor.get("label")?.as_str()?.to_string(),
                                                )
                                                .ghost()
                                                .compact()
                                                .selected(checked)
                                                .on_click(cx.listener(move |root, _, _, cx| {
                                                    root.set_model_option(
                                                        &id,
                                                        Value::Bool(!checked),
                                                        cx,
                                                    )
                                                })),
                                            ),
                                        );
                                    }
                                    Some(
                                        h_flex()
                                            .rounded(px(6.))
                                            .bg(theme.foreground.opacity(0.05))
                                            .p(px(3.))
                                            .children(
                                                descriptor.get("options")?.as_array()?.iter().map(
                                                    |option| {
                                                        let id = id.clone();
                                                        let value = option
                                                            .get("id")
                                                            .cloned()
                                                            .unwrap_or(Value::Null);
                                                        Button::new(SharedString::from(format!(
                                                            "option-{id}-{value}"
                                                        )))
                                                        .label(
                                                            option
                                                                .get("label")
                                                                .and_then(Value::as_str)
                                                                .unwrap_or_default()
                                                                .to_string(),
                                                        )
                                                        .compact()
                                                        .ghost()
                                                        .text_size(px(12.))
                                                        .h(px(22.))
                                                        .px(px(8.))
                                                        .rounded(px(3.))
                                                        .when(
                                                            Some(&value) == selected.as_ref(),
                                                            |button| {
                                                                button.bg(theme
                                                                    .foreground
                                                                    .opacity(0.09))
                                                            },
                                                        )
                                                        .selected(Some(&value) == selected.as_ref())
                                                        .on_click(cx.listener(
                                                            move |root, _, _, cx| {
                                                                root.set_model_option(
                                                                    &id,
                                                                    value.clone(),
                                                                    cx,
                                                                )
                                                            },
                                                        ))
                                                    },
                                                ),
                                            ),
                                    )
                                }),
                        )
                        .child(div().flex_1())
                        .when(
                            self.settings.plan_mode_enabled
                                && selected_instance.is_some_and(|i| i.interaction_mode_toggle),
                            |row| {
                                row.child(
                                    Button::new("plan")
                                        .label(if interaction_mode == "plan" {
                                            "Plan"
                                        } else {
                                            "Build"
                                        })
                                        .compact()
                                        .ghost()
                                        .text_size(px(12.))
                                        .selected(interaction_mode == "plan")
                                        .on_click(
                                            cx.listener(|root, _, _, cx| root.toggle_plan(cx)),
                                        ),
                                )
                            },
                        )
                        .child(
                            Button::new("permissions")
                                .label(runtime_label)
                                .icon(match runtime_mode.as_str() {
                                    "full-access" => gpui_kit::assets::IconName::ShieldOff,
                                    "approval-required" => gpui_kit::assets::IconName::Lock,
                                    "auto-accept-edits" => gpui_kit::assets::IconName::PenLine,
                                    _ => gpui_kit::assets::IconName::Sparkles,
                                })
                                .compact()
                                .ghost()
                                .text_size(px(12.))
                                .text_color(theme.muted_foreground)
                                .dropdown_menu_with_anchor(
                                    Anchor::BottomRight,
                                    move |mut menu, _, _| {
                                        for mode in &modes {
                                            let label = match mode.as_str() {
                                                "full-access" => "Full access",
                                                "approval-required" => "Supervised",
                                                "auto-accept-edits" => "Auto-accept edits",
                                                _ => "Auto",
                                            };
                                            menu = menu.menu_with_check(
                                                label,
                                                *mode == current_mode,
                                                Box::new(SetRuntimeModeAction {
                                                    mode: mode.clone(),
                                                }),
                                            );
                                        }
                                        menu
                                    },
                                ),
                        ),
                )
        });
        div()
            .relative()
            .w_full()
            .child(toolbar.child(self.composer_button(cx)))
            .children(picker)
            .into_any_element()
    }

    fn render_composer(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let suggestions = self
            .completion
            .as_ref()
            .filter(|c| !c.suggestions.is_empty())
            .map(|c| {
                v_flex()
                    .mx_3()
                    .mt_2()
                    .p_1()
                    .rounded_md()
                    .border_1()
                    .border_color(theme.border)
                    .children(c.suggestions.iter().take(8).enumerate().map(|(i, s)| {
                        h_flex()
                            .px_2()
                            .py_0p5()
                            .gap_3()
                            .rounded_sm()
                            .when(i == c.selected, |row| row.bg(theme.list_active))
                            .child(div().text_sm().child(s.label.clone()))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(s.detail.clone()),
                            )
                    }))
            });
        let chips = (!self.attachments.is_empty()).then(|| {
            h_flex().px_3().pt_2().gap_1().flex_wrap().children(
                self.attachments.iter().enumerate().map(|(i, file)| {
                    Button::new(("attachment", i))
                        .label(format!("{} ×", file.name))
                        .compact()
                        .outline()
                        .tooltip("Remove")
                        .on_click(cx.listener(move |root, _, _, cx| {
                            if i < root.attachments.len() {
                                root.attachments.remove(i);
                            }
                            cx.notify();
                        }))
                }),
            )
        });
        let content = v_flex()
            .relative()
            .w_full()
            .max_w(px(736.))
            .mb_5()
            .rounded(px(16.))
            .border_1()
            .border_color(theme.border)
            .bg(crate::appearance::composer(cx))
            .on_drop(cx.listener(|root, paths: &ExternalPaths, _, cx| root.attach_paths(paths, cx)))
            .children(suggestions)
            .children(chips)
            .child(
                h_flex().px_1p5().pt_3().pb_5().gap_2().items_end().child(
                    div()
                        .flex_1()
                        .text_size(px(self.settings.font_size_prompt))
                        .child(
                            Textarea::new(&self.composer)
                                .appearance(false)
                                .bordered(false),
                        ),
                ),
            )
            .child(if self.picker_open {
                self.composer_controls.clone().into_any_element()
            } else {
                self.composer_controls
                    .clone()
                    .cached(StyleRefinement::default().w_full().h(px(40.)))
                    .into_any_element()
            })
            .into_any_element();
        h_flex()
            .w_full()
            .px_4()
            .justify_center()
            .child(content)
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
                div()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(self.render_composer(cx)),
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
        let project_title = entry.map(|e| e.project_title.clone()).unwrap_or_default();
        let title = entry.map(|e| e.title.clone()).unwrap_or_default();
        let status = entry.map(|e| e.status).unwrap_or(ThreadStatus::Idle);
        let menu_root = cx.entity().downgrade();
        let header = h_flex()
            .h(px(40.))
            .flex_none()
            .px_4()
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .when(self.sidebar_hidden, |header| {
                header.child(
                    Button::new("show-sidebar")
                        .icon(gpui_kit::assets::IconName::PanelLeftOpen)
                        .ghost()
                        .size(px(24.))
                        .tooltip("Show sidebar")
                        .accessibility_label("Show sidebar")
                        .on_click(cx.listener(|root, _, _, cx| {
                            root.sidebar_hidden = false;
                            cx.notify();
                        })),
                )
            })
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .text_sm()
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(project_title),
                    )
                    .child(div().text_color(theme.muted_foreground).child("/"))
                    .child(div().min_w_0().truncate().child(title)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(status.label()),
            )
            .child(
                Button::new("files")
                    .icon(gpui_kit::assets::IconName::FolderOpen)
                    .size(px(24.))
                    .tooltip("Files")
                    .accessibility_label("Files")
                    .ghost()
                    .compact()
                    .selected(self.explorer.is_some())
                    .on_click(cx.listener(|root, _, _, cx| root.toggle_explorer(cx))),
            )
            .child(
                Button::new("diff")
                    .icon(gpui_kit::assets::IconName::PanelRight)
                    .size(px(24.))
                    .tooltip("Changes")
                    .accessibility_label("Changes")
                    .ghost()
                    .compact()
                    .on_click(cx.listener(|root, _, _, cx| root.toggle_diff(cx))),
            )
            .child(
                Button::new("thread-actions")
                    .icon(gpui_kit::assets::IconName::Ellipsis)
                    .ghost()
                    .size(px(24.))
                    .tooltip("Thread actions")
                    .accessibility_label("Thread actions")
                    .dropdown_menu(move |menu, _, _| {
                        menu.menu(
                            if status == ThreadStatus::Settled {
                                "Unsettle"
                            } else {
                                "Settle"
                            },
                            Box::new(Settle),
                        )
                        .item(
                            gpui_kit::component::menu::PopupMenuItem::new("Open in browser")
                                .on_click({
                                    let root = menu_root.clone();
                                    move |_, _, cx| {
                                        let _ =
                                            root.update(cx, |root, cx| root.open_in_browser(cx));
                                    }
                                }),
                        )
                    }),
            );
        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(header)
            .when(self.find_open, |pane| {
                let count = if self.find_hits.is_empty() {
                    "No results".to_string()
                } else {
                    format!("{} / {}", self.find_at + 1, self.find_hits.len())
                };
                pane.child(
                    h_flex()
                        .px_4()
                        .py_1()
                        .gap_2()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(div().flex_1().child(Input::new(&self.find_input)))
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(count),
                        )
                        .child(
                            Button::new("find-previous")
                                .label("Previous")
                                .compact()
                                .outline()
                                .on_click(cx.listener(|root, _, _, cx| root.step_find(false, cx))),
                        )
                        .child(
                            Button::new("find-next")
                                .label("Next")
                                .compact()
                                .outline()
                                .on_click(cx.listener(|root, _, _, cx| root.step_find(true, cx))),
                        )
                        .child(
                            Button::new("find-close")
                                .label("Close")
                                .compact()
                                .outline()
                                .on_click(
                                    cx.listener(|root, _, window, cx| root.close_find(window, cx)),
                                ),
                        ),
                )
            })
            .when(self.thread_loading, |pane| {
                pane.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child("Loading thread…"),
                )
            })
            .when(!self.thread_loading, |pane| {
                pane.child(
                    self.timeline_pane
                        .clone()
                        .cached(StyleRefinement::default().flex_1().w_full()),
                )
            })
            .children(message)
            .child(
                div()
                    .pt_9()
                    .children(self.render_queue(cx))
                    .child(self.render_composer(cx)),
            )
            .into_any_element()
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.settings.sidebar_width = self.settings.sidebar_width.clamp(
            208.,
            (f32::from(window.viewport_size().width) - 640.).max(208.),
        );
        let sidebar_stamp = SidebarStamp {
            history_available: (self.history.can_back(), self.history.can_forward()),
            entries_revision: self.entries_revision,
            open: self.current(),
            filter: self.host_filter.clone(),
            collapsed: self.collapsed.clone(),
            families: self.collapsed_families.clone(),
            connections: self
                .connections
                .iter()
                .map(|connection| {
                    (
                        connection.label().to_string(),
                        connection.environment_id(),
                        connection.status(),
                    )
                })
                .collect(),
            width: self.settings.sidebar_width,
            light: self.settings.light_theme,
            font_size: self.settings.font_size_interface,
            minute: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                / 60,
        };
        if self.sidebar_stamp.as_ref() != Some(&sidebar_stamp) {
            self.sidebar_stamp = Some(sidebar_stamp);
            self.sidebar_pane.update(cx, |_, cx| cx.notify());
        }
        self.follow_explorer(cx);
        if let Some(text) = self.pending_composer.take() {
            self.composer
                .update(cx, |state, cx| state.set_value(text, window, cx));
        }
        let controls_stamp = ComposerControlsStamp {
            target: self.composer_target(),
            current: self.current(),
            draft: self.draft.is_some(),
            status: self.open_entry().map(|entry| entry.status),
            question: self.pending_question().is_some(),
            empty: self.composer.read(cx).value().trim().is_empty(),
            attachments: !self.attachments.is_empty(),
            picker: self.picker_open,
            picker_instance: self.picker_instance.clone(),
            catalog_revision: self.model_catalog_revision,
            light: self.settings.light_theme,
            font_size: self.settings.font_size_interface,
            plan_enabled: self.settings.plan_mode_enabled,
        };
        if self.composer_controls_stamp.as_ref() != Some(&controls_stamp) {
            self.composer_controls_stamp = Some(controls_stamp);
            self.composer_controls.update(cx, |_, cx| cx.notify());
        }
        if !self.focus.contains_focused(window, cx)
            && self.open.is_none()
            && self.draft.is_none()
            && !self.palette_open
            && !self.file_picker_open
            && !self.hosts_open
        {
            self.focus.focus(window, cx);
        }
        let pane = match &self.draft {
            Some(draft) => self.render_draft(draft, cx),
            None => self.render_thread(cx),
        };
        h_flex()
            .relative()
            .size_full()
            .track_focus(&self.focus)
            .key_context("Root")
            .on_action(cx.listener(|root, _: &NextDone, _, cx| root.next_done(true, cx)))
            .on_action(cx.listener(|root, _: &PreviousDone, _, cx| root.next_done(false, cx)))
            .on_action(
                cx.listener(|root, menu: &ThreadMenu, _, cx| root.thread_op(menu.clone(), cx)),
            )
            .on_action(cx.listener(|root, _: &Find, window, cx| {
                let in_file = root.panel_focus.contains_focused(window, cx)
                    || (root.open.is_none() && root.file_panel.is_some());
                if in_file {
                    root.open_file_find(window, cx)
                } else {
                    root.open_find(window, cx)
                }
            }))
            .on_action(
                cx.listener(|root, _: &TogglePalette, window, cx| root.toggle_palette(window, cx)),
            )
            .on_action(cx.listener(|root, _: &ToggleFilePicker, window, cx| {
                root.toggle_file_picker(window, cx)
            }))
            .on_action(cx.listener(|root, _: &ToggleDiff, _, cx| root.toggle_diff(cx)))
            .on_action(cx.listener(|root, _: &ToggleExplorer, _, cx| root.toggle_explorer(cx)))
            .on_action(
                cx.listener(|root, _: &ManageHosts, window, cx| root.toggle_hosts(window, cx)),
            )
            .on_action(cx.listener(|root, _: &OpenSettings, _, cx| {
                root.settings_open = !root.settings_open;
                cx.notify();
            }))
            .on_action(cx.listener(|root, _: &CloseFilePanel, window, cx| {
                root.close_file_panel(window, cx);
            }))
            .on_action(cx.listener(|root, open: &OpenFile, _, cx| {
                root.file_picker_open = false;
                root.open_file(open.path.clone(), cx)
            }))
            .on_action(cx.listener(|root, open: &OpenThreadAction, _, cx| {
                root.palette_open = false;
                root.open_thread(open.environment.clone(), open.thread_id.clone(), cx)
            }))
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
            .on_action(cx.listener(|root, _: &HistoryBack, _, cx| {
                if let Some((environment, thread_id)) = root.history.step(false) {
                    root.open_thread(environment, thread_id, cx);
                }
            }))
            .on_action(cx.listener(|root, _: &HistoryForward, _, cx| {
                if let Some((environment, thread_id)) = root.history.step(true) {
                    root.open_thread(environment, thread_id, cx);
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
            .on_action(cx.listener(|root, _: &ToggleModelPicker, window, cx| {
                root.toggle_model_picker(window, cx)
            }))
            .on_action(cx.listener(|root, action: &PickModelAction, _, cx| {
                root.pick_model(action.instance_id.clone(), action.slug.clone(), cx)
            }))
            .on_action(cx.listener(|root, action: &SetModelOptionAction, _, cx| {
                root.set_model_option(&action.id, action.value.clone(), cx)
            }))
            .on_action(cx.listener(|root, action: &SetRuntimeModeAction, _, cx| {
                root.set_runtime_mode(action.mode.clone(), cx)
            }))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(UI_FONT)
            .on_mouse_move(cx.listener(|root, event: &MouseMoveEvent, window, cx| {
                if root.sidebar_resizing && event.pressed_button == Some(MouseButton::Left) {
                    let max = (f32::from(window.viewport_size().width) - 640.).max(208.);
                    root.settings.sidebar_width = f32::from(event.position.x).clamp(208., max);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|root, _, _, cx| {
                    if root.sidebar_resizing {
                        root.sidebar_resizing = false;
                        if let Err(error) = client_settings::update(
                            &root.settings_path,
                            &[(
                                "gpuiSidebarWidth",
                                serde_json::json!(root.settings.sidebar_width),
                            )],
                        ) {
                            root.message = Some(format!("Cannot save sidebar width: {error:#}"));
                        }
                        cx.notify();
                    }
                }),
            )
            .when(!self.sidebar_hidden, |root| {
                root.child(
                    self.sidebar_pane.clone().cached(
                        StyleRefinement::default()
                            .w(px(self.settings.sidebar_width))
                            .h_full()
                            .flex_none(),
                    ),
                )
                .child(
                    div()
                        .ml(px(-4.))
                        .w(px(4.))
                        .h_full()
                        .flex_none()
                        .cursor_col_resize()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|root, _, _, cx| {
                                root.sidebar_resizing = true;
                                cx.notify();
                            }),
                        ),
                )
            })
            .child(pane)
            .children(self.render_explorer(cx))
            .children(self.render_file_panel(cx))
            .when(self.palette_open, |root_el| {
                root_el.child(self.render_palette(cx))
            })
            .when(self.hosts_open, |root_el| {
                root_el.child(self.render_hosts(cx))
            })
            .when(self.settings_open, |root_el| {
                root_el.child(self.render_settings(cx))
            })
            .when(self.file_picker_open, |root_el| {
                root_el.child(self.render_file_picker(cx))
            })
    }
}

pub fn run(
    config: Config,
    config_path: std::path::PathBuf,
    (settings, settings_path): (ClientSettings, std::path::PathBuf),
) -> anyhow::Result<()> {
    let drafts_path = drafts::default_path()?;
    let drafts = Drafts::load(&drafts_path)?;
    let runtime = Runtime::new()?;
    let handle = runtime.handle().clone();
    gpui_kit::application()
        .with_assets(crate::appearance::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            crate::appearance::apply(settings.light_theme, cx);
            Theme::update(cx, |theme| {
                theme.font_family = UI_FONT.into();
                theme.font_size = px(settings.font_size_interface);
                theme.mono_font_size = px(settings.font_size_code);
                theme.cursor_blink =
                    CursorBlink::blinking(CURSOR_BLINK).with_idle_timeout(CURSOR_BLINK_IDLE);
            });
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
                cx.new(|cx| {
                    Root::new(
                        (config, config_path),
                        (settings.clone(), settings_path),
                        (drafts, drafts_path),
                        handle,
                        window,
                        cx,
                    )
                })
            })
            .expect("open the window");
        });
    drop(runtime);
    Ok(())
}

fn reveal_hit(panel: &FilePanel) {
    if let Some((line, _)) = panel.hits.get(panel.hit_at) {
        panel.scroll.scroll_to_item(*line, ScrollStrategy::Center);
    }
}
