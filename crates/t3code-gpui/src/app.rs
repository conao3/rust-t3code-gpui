use anyhow::Context as _;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _, Theme,
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    command::{Command, CommandGroup, CommandItem, CommandState},
    diff::{Diff, DiffFile, DiffMode, DiffParseOptions, DiffState},
    h_flex,
    input::{CursorBlink, Input, InputEvent, InputState, Position, Textarea, TextareaState},
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem},
    text::{TextView, TextViewState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};
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
use crate::permissions;
use crate::projects::{self, ProjectGroup};
use crate::row_view::{RowEvent, RowView};
use crate::sidebar::{self, ThreadEntry, ThreadStatus};
use crate::tables;
use crate::timeline::{self, Pending, Row, RowKind};
use crate::workspace;

mod choice_select;
mod composer;
mod composer_controls;
mod composer_dictation;
mod composer_feedback;
mod composer_images;
mod composer_prompts;
mod composer_queue;
mod composer_scratch;
mod composer_tokens;
mod conversation;
mod device_panel;
mod diff;
mod diff_controls;
mod files;
mod find;
mod git_controls;
mod hosts;
mod lineage;
mod minimap;
mod palette;
mod pane_layout;
mod project_search;
mod pull_requests;
mod right_panel;
mod settings;
mod settings_archive;
mod settings_catalog;
mod settings_controls;
mod settings_keybindings;
mod settings_providers;
mod settings_tasks;
mod sidebar_branding;
mod sidebar_controls;
mod sidebar_header;
mod sidebar_projects;
mod sidebar_view;
mod snooze_controls;
mod snooze_options;
mod terminal;
mod thread_commands;
mod thread_details;
#[path = "app/timeline.rs"]
mod timeline_view;
mod titlebar;
mod usage;
mod usage_data;
#[path = "app/workspace.rs"]
mod workspace_view;

use composer::ComposerControlsPane;
use conversation::ConversationState;
use sidebar_view::SidebarPane;
use timeline_view::TimelinePane;
use workspace_view::DetailsPane;

type CommandFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<u64>> + std::marker::Send>>;

const REDRAW_INTERVAL: Duration = Duration::from_millis(250);
const APP_NAME: &str = "T3 Code GPUI (Unofficial)";
const UI_FONT: &str = "Inter Variable";
const DIFF_TITLE: &str = "Changes";
const CURSOR_BLINK: Duration = Duration::from_millis(500);
const CURSOR_BLINK_IDLE: Duration = Duration::from_secs(5);

actions!(
    t3code_gpui,
    [
        Send,
        Dictate,
        NewWithoutProject,
        SteerQueuedMessage,
        EditQueuedMessage,
        StashPrompt,
        PreviousWorktree,
        OpenComposerWorkspace,
        DecreaseEffort,
        IncreaseEffort,
        OpenEffort,
        CycleComposerHost,
        OpenComposerHost,
        SendAlternate,
        SendBackground,
        SendAndNewThread,
        NextDone,
        PreviousDone,
        ToggleSidebar,
        TogglePin,
        Find,
        TogglePalette,
        ToggleProjectSearch,
        ToggleFilePicker,
        ToggleDiff,
        ToggleExplorer,
        ToggleRightPanel,
        CopyPullRequestNumber,
        NewRightPanel,
        MaximizeRightPanel,
        ManageHosts,
        OpenSettings,
        OpenUsage,
        UsageCost,
        UsageTokens,
        UsageLimits,
        UsageDay,
        UsageWeek,
        UsageMonth,
        UsageQuarter,
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
        Stop,
        CycleAppearance,
        OpenPermissions,
        ToggleThreadDetails,
        OpenBranchPicker,
        OpenFavoriteEditor,
        ToggleTerminal,
        SplitTerminal,
        SplitTerminalVertical,
        NewTerminal,
        CloseTerminal,
        OpenSideChat,
        SplitConversation,
        SplitConversationVertical,
        CloseConversation,
        FocusNextWorkspace,
        FocusPreviousWorkspace,
        ReopenClosed,
        CopyThreadReference,
        RegenerateTitle,
        UndoThreadAction,
    ]
);

/// The action of a T3 keybinding command this app implements.
fn action_for(command: &str) -> Option<Box<dyn Action>> {
    Some(match command {
        "composer.dictation" => Box::new(Dictate),
        "chat.newWithoutProject" => Box::new(NewWithoutProject),
        "thread.steerQueuedMessage" => Box::new(SteerQueuedMessage),
        "thread.editQueuedMessage" => Box::new(EditQueuedMessage),
        "composer.stash" => Box::new(StashPrompt),
        "composer.previousWorktree" => Box::new(PreviousWorktree),
        "composer.workspace" => Box::new(OpenComposerWorkspace),
        "composer.decreaseEffort" => Box::new(DecreaseEffort),
        "composer.increaseEffort" => Box::new(IncreaseEffort),
        "composer.effort" => Box::new(OpenEffort),
        "composer.cycleHost" => Box::new(CycleComposerHost),
        "composer.host" => Box::new(OpenComposerHost),
        "workspace.focusNext" => Box::new(FocusNextWorkspace),
        "workspace.focusPrevious" => Box::new(FocusPreviousWorkspace),
        "thread.copyReference" => Box::new(CopyThreadReference),
        "thread.regenerateTitle" => Box::new(RegenerateTitle),
        "thread.undo" => Box::new(UndoThreadAction),
        "view.reopenClosed" => Box::new(ReopenClosed),
        "terminal.toggle" => Box::new(ToggleTerminal),
        "terminal.split" => Box::new(SplitTerminal),
        "terminal.splitVertical" => Box::new(SplitTerminalVertical),
        "terminal.new" => Box::new(NewTerminal),
        "terminal.close" => Box::new(CloseTerminal),
        "thread.nextDone" => Box::new(NextDone),
        "thread.previousDone" => Box::new(PreviousDone),
        "chat.nextUserMessage" => Box::new(NextUserMessage),
        "chat.previousUserMessage" => Box::new(PreviousUserMessage),
        "thread.next" => Box::new(NextThread),
        "thread.previous" => Box::new(PreviousThread),
        "thread.settle" => Box::new(Settle),
        "chat.new" | "chat.newLocal" => Box::new(NewThread),
        "thread.stop" => Box::new(Stop),
        "navigation.back" => Box::new(HistoryBack),
        "navigation.forward" => Box::new(HistoryForward),
        "modelPicker.toggle" => Box::new(ToggleModelPicker),
        "composer.sendAlternate" => Box::new(SendAlternate),
        "composer.sendBackground" => Box::new(SendBackground),
        "composer.sendAndNewThread" => Box::new(SendAndNewThread),
        "composer.mode" => Box::new(OpenPermissions),
        "sidebar.toggle" => Box::new(ToggleSidebar),
        "thread.pin" => Box::new(TogglePin),
        "chat.find" => Box::new(Find),
        "projectSearch.toggle" => Box::new(ToggleProjectSearch),
        "commandPalette.toggle" => Box::new(TogglePalette),
        "filePicker.toggle" => Box::new(ToggleFilePicker),
        "diff.toggle" => Box::new(ToggleDiff),
        "threadPanel.toggle" => Box::new(ToggleThreadDetails),
        "composer.branch" => Box::new(OpenBranchPicker),
        "editor.openFavorite" => Box::new(OpenFavoriteEditor),
        "settings.connections" => Box::new(ManageHosts),
        "settings.open" => Box::new(OpenSettings),
        "usage.open" => Box::new(OpenUsage),
        "usage.cost" => Box::new(UsageCost),
        "usage.tokens" => Box::new(UsageTokens),
        "usage.limits" => Box::new(UsageLimits),
        "usage.period.day" => Box::new(UsageDay),
        "usage.period.week" => Box::new(UsageWeek),
        "usage.period.month" => Box::new(UsageMonth),
        "usage.period.quarter" => Box::new(UsageQuarter),
        "pullRequest.copyNumber" => Box::new(CopyPullRequestNumber),
        "rightPanel.toggle" => Box::new(ToggleRightPanel),
        "rightPanel.new" => Box::new(NewRightPanel),
        "rightPanel.toggleMaximized" => Box::new(MaximizeRightPanel),
        "rightPanel.close" => Box::new(CloseFilePanel),
        "appearance.cycle" => Box::new(CycleAppearance),
        "theme.select" => Box::new(OpenSettings),
        _ => return None,
    })
}

#[derive(Clone, Debug, PartialEq)]
enum ThreadOp {
    Pin,
    Unpin,
    Snooze(u64),
    SnoozeUntil(String),
    Unsnooze,
    Settle,
    Unsettle,
    Archive,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = t3code_gpui, no_json)]
struct WorkspaceAction {
    kind: String,
    value: Value,
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
struct PickProjectAction {
    key: String,
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

struct Completion {
    trigger: completion::Trigger,
    suggestions: Vec<completion::Suggestion>,
    selected: usize,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct NewThreadDraft {
    id: String,
    environment: Option<String>,
    workspace_strategy: Value,
    group: Option<String>,
    carry: Option<Value>,
    model: Option<Value>,
    runtime_mode: Option<String>,
    interaction_mode: String,
}

#[derive(PartialEq)]
struct SidebarStamp {
    search: (String, Vec<(String, String)>, bool),
    project_filter: Vec<String>,
    scope_open: bool,
    search_error: Option<String>,
    search_selected: usize,
    undo: Vec<(String, String, &'static str)>,
    catalog_revision: u64,
    pull_requests_page: bool,
    jump_labels: Vec<Option<String>>,
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

struct ParkPlan {
    target: (String, String),
    next: Option<(String, String)>,
    snooze: bool,
    sequence: u64,
}

#[derive(PartialEq)]
struct ComposerControlsStamp {
    width: u32,
    workspace_revision: u64,
    context_menu: (bool, usize),
    target: Option<(String, Option<Value>, String, String)>,
    current: Option<(String, String)>,
    draft: bool,
    status: Option<ThreadStatus>,
    delivery: Delivery,
    dictation: (bool, bool),
    effort_cursor: Option<Value>,
    queue_edit: Option<(String, bool)>,
    question: bool,
    empty: bool,
    attachments: bool,
    picker: bool,
    permissions: bool,
    picker_instance: Option<String>,
    catalog_revision: u64,
    light: bool,
    font_size: f32,
    plan_enabled: bool,
}

#[derive(PartialEq)]
struct DetailsStamp {
    usage_sequence: Option<(String, String, u64)>,
    pull_requests: Value,
    target: Option<workspace::Target>,
    status_revision: u64,
    catalog_revision: u64,
    draft_strategy: Option<Value>,
    branch_anchor: Option<&'static str>,
    branch_request: u64,
    branch_loading: bool,
    branch_count: usize,
    light: bool,
    preferred_editor: Option<String>,
    lineage_revision: u64,
}

pub struct Root {
    context_usage_stamp: Option<(String, String, u64)>,
    context_usage: Option<Value>,
    pr_state: pull_requests::State,
    pr_search: Entity<InputState>,
    pr_comment: Entity<TextareaState>,
    pr_pane: Entity<pull_requests::Pane>,
    diff_view: diff_controls::State,
    git_state: git_controls::State,
    right_panel: right_panel::State,
    right_panel_threads: HashMap<(String, String), right_panel::State>,
    right_panel_target: Option<(String, String)>,
    right_panel_width: f32,
    panel_resizing: bool,
    panel_revision: u64,
    panel_stamp: String,
    panel_pane: Entity<right_panel::Pane>,
    conversation: ConversationState,
    thread_undos: Vec<thread_commands::UndoEntry>,
    thread_undo_epoch: u64,
    thread_undo_claimed: bool,
    panes: HashMap<u64, ConversationState>,
    pane_layout: pane_layout::Layout,
    pane_closed: Vec<pane_layout::SavedPane>,
    pane_restored: bool,
    pane_save_pending: bool,
    pane_next_id: u64,
    pane_resize: Option<(Vec<bool>, bool, Bounds<Pixels>)>,
    lineage_data: crate::lineage::Details,
    lineage_target: Option<(String, String)>,
    lineage_revision: u64,
    lineage_counts: [usize; 4],
    lineage_previous: bool,
    lineage_busy: Option<(String, String)>,
    lineage_stopping: Option<(String, String)>,
    terminal_store: crate::terminal::Store,
    terminal_drawers: HashMap<String, Entity<crate::terminal::Drawer>>,
    terminal_subscriptions: HashMap<String, Subscription>,
    terminal_recent: Vec<String>,
    terminal_revision: u64,
    terminal_stamp: Option<(Option<String>, u64, u64, bool, u32)>,
    terminal_save_scheduled: bool,
    editor_picker_focus: FocusHandle,
    details_pane: Entity<DetailsPane>,
    details_stamp: Option<DetailsStamp>,
    workspace_sender: async_channel::Sender<workspace::Update>,
    _workspace_updates: Task<()>,
    canvas_width: f32,
    details_open: bool,
    details_popover: bool,
    sidebar_resizing: bool,
    window_drag_pending: bool,
    history: crate::navigation::History,
    sidebar_pane: Entity<SidebarPane>,
    sidebar_header: sidebar_header::State,
    sidebar_scroll: ScrollHandle,
    sidebar_stamp: Option<SidebarStamp>,
    runtime: Handle,
    connections: Vec<Arc<Connection>>,
    model_catalogs: HashMap<String, (Value, Vec<models::Instance>)>,
    sidebar_catalogs: HashMap<String, crate::provider_display::Catalog>,
    model_catalog_revision: u64,
    entries: Vec<ThreadEntry>,
    entries_revision: u64,
    statuses: HashMap<(String, String), ThreadStatus>,
    host_filter: Option<String>,
    palette: Entity<CommandState>,
    palette_open: bool,
    project_search: project_search::ProjectSearch,
    file_picker: Entity<CommandState>,
    file_picker_open: bool,
    file_hits: Vec<String>,
    file_panel: Option<FilePanel>,
    explorer: Option<Explorer>,
    file_find_input: Entity<InputState>,
    file_find_open: bool,
    panel_focus: FocusHandle,
    drafts: Drafts,
    drafts_path: std::path::PathBuf,
    drafts_save_scheduled: bool,
    key_rules: Vec<keymap::Rule>,
    jump_modifiers: Modifiers,
    jump_hints: bool,
    jump_hint_epoch: u64,
    settings: ClientSettings,
    settings_path: std::path::PathBuf,
    settings_open: bool,
    usage_page: Option<Entity<usage::UsagePage>>,
    settings_focus: FocusHandle,
    settings_page: &'static str,
    settings_environment: Option<String>,
    settings_project: Option<String>,
    sidebar_hidden: bool,
    wake_task: Option<Task<()>>,
    inbox: crate::inbox::Tracker,
    promoting: HashSet<(String, String)>,
    parking: HashSet<(String, String, bool)>,
    park_plans: Vec<ParkPlan>,
    focus_after_park: bool,
    collapsed: HashSet<sidebar::Section>,
    collapsed_families: HashSet<(String, String)>,
    window_active: bool,
    shell_dirty: bool,
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
        let (workspace_sender, workspace_receiver) = async_channel::bounded::<workspace::Update>(8);
        let workspace_updates = cx.spawn(async move |this, cx| {
            while let Ok(update) = workspace_receiver.recv().await {
                if this
                    .update(cx, |root, cx| {
                        for pane in
                            std::iter::once(&mut root.conversation).chain(root.panes.values_mut())
                        {
                            if pane
                                .workspace_watch
                                .as_ref()
                                .is_some_and(|watch| watch.target == update.target)
                                && pane.workspace_status.apply(update.event.clone())
                            {
                                pane.workspace_revision += 1;
                                cx.notify();
                            }
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let pr_search = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search pull requests, or label:bug")
        });
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
            cx.subscribe(&pr_search, |root, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change | InputEvent::PressEnter { .. }) {
                    root.load_pull_requests(false, cx);
                }
            }),
            cx.intercept_keystrokes(move |event, window, cx| {
                let handled = weak
                    .update(cx, |root, cx| root.intercept(event, window, cx))
                    .unwrap_or(false);
                if handled {
                    cx.stop_propagation();
                }
            }),
            cx.observe_window_activation(window, |root, window, cx| {
                root.window_active = window.is_window_active();
                if !root.window_active {
                    if root.pane_resize.take().is_some() {
                        root.save_conversations();
                    }
                    root.window_drag_pending = false;
                    root.jump_hint_epoch += 1;
                    root.jump_modifiers = Modifiers::default();
                    root.jump_hints = false;
                    cx.notify();
                }
            }),
            cx.subscribe(
                &file_find_input,
                |root, _, event: &InputEvent, cx| match event {
                    InputEvent::Change => root.file_find_again(cx),
                    InputEvent::PressEnter { shift, .. } => root.step_file_find(!*shift, cx),
                    _ => {}
                },
            ),
        ];
        let sidebar_root = cx.entity().downgrade();
        let mut terminal_store =
            crate::terminal::Store::load(drafts_path.with_file_name("terminals.json"));
        let terminal_error = terminal_store.error.take();
        let (saved_panels, panel_error) = match right_panel::load(&drafts_path) {
            Ok(saved) => (saved, None),
            Err(error) => (
                right_panel::Saved {
                    threads: Vec::new(),
                },
                Some(format!("Could not load right panels: {error:#}")),
            ),
        };
        let collapsed = [
            sidebar::Section::Working,
            sidebar::Section::Snoozed,
            sidebar::Section::Settled,
        ]
        .into_iter()
        .zip(settings.shelf_expanded)
        .filter_map(|(section, expanded)| (!expanded).then_some(section))
        .collect();
        Self {
            context_usage_stamp: None,
            context_usage: None,
            pr_state: pull_requests::State::default(),
            pr_search,
            pr_comment: cx.new(|cx| TextareaState::new(window, cx).placeholder("Leave a comment")),
            pr_pane: cx.new(|_| pull_requests::Pane {
                root: sidebar_root.clone(),
            }),
            diff_view: diff_controls::State::default(),
            git_state: git_controls::State::default(),
            right_panel: right_panel::State::default(),
            right_panel_threads: saved_panels.threads.into_iter().collect(),
            right_panel_target: None,
            right_panel_width: 540.,
            panel_resizing: false,
            panel_revision: 0,
            panel_stamp: String::new(),
            panel_pane: cx.new(|_| right_panel::Pane {
                root: sidebar_root.clone(),
            }),
            conversation: ConversationState::new(
                0,
                terminal_error.or(panel_error),
                &drafts_path,
                window,
                cx,
            ),
            thread_undos: Vec::new(),
            thread_undo_epoch: 0,
            thread_undo_claimed: false,
            panes: HashMap::new(),
            pane_layout: pane_layout::Layout::Pane(0),
            pane_closed: Vec::new(),
            pane_restored: false,
            pane_save_pending: false,
            pane_next_id: 1,
            pane_resize: None,
            lineage_data: crate::lineage::Details::default(),
            lineage_target: None,
            lineage_revision: 0,
            lineage_counts: [6; 4],
            lineage_previous: false,
            lineage_busy: None,
            lineage_stopping: None,
            terminal_store,
            terminal_drawers: HashMap::new(),
            terminal_subscriptions: HashMap::new(),
            terminal_recent: Vec::new(),
            terminal_revision: 0,
            terminal_stamp: None,
            terminal_save_scheduled: false,
            editor_picker_focus: cx.focus_handle(),
            details_pane: cx.new(|_| DetailsPane {
                root: sidebar_root.clone(),
                services: None,
            }),
            details_stamp: None,
            workspace_sender,
            _workspace_updates: workspace_updates,
            canvas_width: 0.,
            details_open: true,
            details_popover: false,
            sidebar_resizing: false,
            window_drag_pending: false,
            history: crate::navigation::History::default(),
            sidebar_pane: cx.new(|_| SidebarPane {
                root: sidebar_root.clone(),
            }),
            sidebar_header: sidebar_header::State::new(window, cx),
            sidebar_scroll: ScrollHandle::new(),
            sidebar_stamp: None,
            runtime,
            connections,
            model_catalogs: HashMap::new(),
            sidebar_catalogs: HashMap::new(),
            model_catalog_revision: 0,
            entries: Vec::new(),
            entries_revision: 0,
            statuses: HashMap::new(),
            host_filter: None,
            palette,
            palette_open: false,
            project_search: project_search::ProjectSearch::new(window, cx),
            file_picker,
            file_picker_open: false,
            file_hits: Vec::new(),
            file_panel: None,
            explorer: None,
            file_find_input,
            file_find_open: false,
            panel_focus: cx.focus_handle(),
            drafts,
            drafts_path,
            drafts_save_scheduled: false,
            key_rules: Vec::new(),
            jump_modifiers: Modifiers::default(),
            jump_hints: false,
            jump_hint_epoch: 0,
            settings,
            settings_path,
            settings_open: false,
            usage_page: None,
            settings_focus: cx.focus_handle(),
            settings_page: "general",
            settings_environment: None,
            settings_project: None,
            sidebar_hidden: false,
            wake_task: None,
            inbox: crate::inbox::Tracker::default(),
            promoting: HashSet::new(),
            parking: HashSet::new(),
            park_plans: vec![],
            focus_after_park: false,
            collapsed,
            collapsed_families: HashSet::new(),
            window_active: window.is_window_active(),
            shell_dirty: false,
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

    fn intercept(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if window.has_active_dialog(cx) {
            return false;
        }
        if let Some(id) = self.panes.iter().find_map(|(id, pane)| {
            pane.composer
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
                .then_some(*id)
        }) {
            self.activate_conversation(id, window, cx, false);
        }
        if self.composer_composing(window, cx) {
            return false;
        }
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if self.pr_guide_key(keystroke, window, cx) {
            return true;
        }
        if self.prompt_key(keystroke, window, cx) {
            return true;
        }
        if !((modifiers.control || modifiers.platform) && keystroke.key == "z") {
            self.thread_undo_claimed = false;
        }
        if self.completion_key(keystroke, window, cx) {
            return true;
        }
        if keystroke.key == "v"
            && keystroke.modifiers.control
            && !keystroke.modifiers.alt
            && self
                .conversation
                .composer
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
            && self.paste_image(cx)
        {
            return true;
        }
        if keystroke.key == "escape"
            && self.conversation.find_open
            && self
                .conversation
                .find_input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
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
            // GPUI drops Shift from symbol keystrokes; T3 keeps the physical modifier.
            shift: modifiers.shift || window.modifiers().shift,
            platform: modifiers.platform,
            key: keystroke.key.clone(),
        };
        if self.project_script_key(&typed, window, cx) {
            return true;
        }
        if let Some(command) = keymap::resolve(&self.key_rules, &typed, &|name| {
            self.key_context(name, window, cx)
        })
        .map(str::to_string)
        {
            if self.picker_shortcut(&command, window, cx) {
                return true;
            }
            if let Some(index) = keymap::jump_index(&command) {
                if self.palette_open {
                    return false;
                }
                if let Some(entry) = self.ordered_entries().get(index) {
                    self.open_thread(entry.environment.clone(), entry.thread_id.clone(), cx);
                }
                return true;
            }
            if let Some(action) = action_for(&command) {
                window.dispatch_action(action, cx);
                return true;
            }
        }
        if keystroke.key != "enter" || modifiers.alt || modifiers.shift || modifiers.platform {
            return false;
        }
        if !self
            .conversation
            .composer
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
        {
            return false;
        }
        let text = self.conversation.composer.read(cx).value().to_string();
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

    fn key_context(&self, name: &str, window: &Window, cx: &App) -> bool {
        match name {
            "terminalOpen" => self
                .terminal_key()
                .is_some_and(|key| self.terminal_store.get(&key).open),
            "terminalFocus" => self
                .terminal_key()
                .and_then(|key| self.terminal_drawers.get(&key))
                .is_some_and(|drawer| drawer.read(cx).is_focused(window, cx)),
            "isDesktop" => true,
            "modelPickerOpen" => self.conversation.picker_open,
            "usagePageOpen" => {
                self.usage_page.is_some()
                    && !self.key_context("editableFocus", window, cx)
                    && !window.context_stack().iter().any(|context| {
                        context.contains("PopupMenu")
                            || context.contains("Dialog")
                            || context.contains("Select")
                    })
            }
            "rightPanelOpen" => self.right_panel.open,
            "composerFocus" => self
                .conversation
                .composer
                .read(cx)
                .focus_handle(cx)
                .is_focused(window),
            "editableFocus"
                if self.thread_undo_claimed
                    && !self.sidebar_hidden
                    && !self.thread_undos.is_empty() =>
            {
                false
            }
            "editableFocus" => {
                window
                    .context_stack()
                    .iter()
                    .any(|context| context.contains("Input"))
                    || self.key_context("terminalFocus", window, cx)
                    || [
                        self.conversation.composer.read(cx).focus_handle(cx),
                        self.pr_search.read(cx).focus_handle(cx),
                        self.pr_comment.read(cx).focus_handle(cx),
                        self.conversation.find_input.read(cx).focus_handle(cx),
                        self.file_find_input.read(cx).focus_handle(cx),
                        self.palette.read(cx).focus_handle(cx),
                        self.file_picker.read(cx).focus_handle(cx),
                        self.conversation.branch_picker.read(cx).focus_handle(cx),
                        self.conversation.model_picker.read(cx).focus_handle(cx),
                    ]
                    .iter()
                    .any(|handle| handle.contains_focused(window, cx))
                    || self
                        .host_inputs
                        .iter()
                        .any(|input| input.read(cx).focus_handle(cx).is_focused(window))
            }
            "composerRequiresModifier" => self
                .settings
                .requires_modifier(&self.conversation.composer.read(cx).value()),
            "draftThreadRoute" => self.conversation.draft.is_some(),
            "turnRunning" => self
                .open_entry()
                .is_some_and(|entry| entry.status == ThreadStatus::Working),
            _ => false,
        }
    }

    fn modifiers_changed(
        &mut self,
        modifiers: Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if modifiers == self.jump_modifiers {
            return;
        }
        self.jump_modifiers = modifiers;
        self.jump_hint_epoch += 1;
        let was_visible = std::mem::take(&mut self.jump_hints);
        if was_visible {
            cx.notify();
        }
        let chord = keymap::Chord {
            control: modifiers.control,
            alt: modifiers.alt,
            shift: modifiers.shift,
            platform: modifiers.platform,
            key: String::new(),
        };
        if !keymap::jump_hints_held(&self.key_rules, &chord, &|name| {
            self.key_context(name, window, cx)
        }) {
            return;
        }
        let epoch = self.jump_hint_epoch;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            let _ = this.update_in(cx, |root, _, cx| {
                if root.jump_hint_epoch == epoch && root.window_active {
                    root.jump_hints = true;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn current(&self) -> Option<(String, String)> {
        self.conversation
            .open
            .as_ref()
            .map(|o| (o.environment.clone(), o.watch.thread_id().to_string()))
    }

    fn on_change(&mut self, change: Change, window: &mut Window, cx: &mut Context<Self>) {
        match change {
            Change::Config { environment } => {
                self.model_catalog_revision = self.model_catalog_revision.wrapping_add(1);
                if let Some(config) = self.config_of(&environment) {
                    self.sidebar_catalogs.insert(
                        environment.clone(),
                        crate::provider_display::Catalog::from_config(&config),
                    );
                    let instances = models::instances(&config);
                    self.model_catalogs.insert(environment, (config, instances));
                } else {
                    self.sidebar_catalogs.remove(&environment);
                    self.model_catalogs.remove(&environment);
                }
                self.shell_dirty = true;
            }
            Change::Status { .. } | Change::Shell { .. } => self.shell_dirty = true,
            Change::Thread {
                environment,
                thread_id,
            } => {
                let open = self.conversation.open.as_ref().is_some_and(|open| {
                    open.environment == environment && open.watch.thread_id() == thread_id
                });
                if open {
                    self.conversation.thread_dirty = true;
                }
                for pane in self.panes.values_mut() {
                    if pane.open.as_ref().is_some_and(|open| {
                        open.environment == environment && open.watch.thread_id() == thread_id
                    }) {
                        pane.thread_dirty = true;
                    }
                }
            }
        }
        if !self.redraw_scheduled
            && (self.shell_dirty
                || self.conversation.thread_dirty
                || self.panes.values().any(|pane| pane.thread_dirty))
        {
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
        if std::mem::take(&mut self.conversation.thread_dirty) {
            self.rebuild_rows(window, cx);
        }
        for id in self.pane_layout.ids() {
            self.with_conversation(id, |root| {
                if std::mem::take(&mut root.conversation.thread_dirty) {
                    root.rebuild_rows(window, cx);
                }
            });
        }
        self.finish_parks(cx);
        self.sync_lineage();
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

    fn label_of(&self, environment_id: &str) -> Option<String> {
        self.connections
            .iter()
            .find(|c| c.environment_id().as_deref() == Some(environment_id))
            .map(|c| c.label().to_string())
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
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_after_park) {
            self.conversation
                .composer
                .update(cx, |state, cx| state.focus(window, cx));
        }
        self.sync_terminal_drawers(window, cx);
        if self.conversation.draft.is_none() && !self.conversation.picker_open {
            self.conversation.permissions_menu = None;
            self.conversation.permissions_subscription = None;
        }
        self.settings.sidebar_width = self.settings.sidebar_width.clamp(
            208.,
            (f32::from(window.viewport_size().width) - 640.).max(208.),
        );
        let sidebar_stamp = SidebarStamp {
            search: (
                self.sidebar_header.query.clone(),
                {
                    let mut matches: Vec<_> = self.sidebar_header.matches.iter().cloned().collect();
                    matches.sort();
                    matches
                },
                self.sidebar_header.pending,
            ),
            project_filter: self.sidebar_header.scope_keys.clone(),
            scope_open: self.sidebar_header.scope_open,
            search_error: self.sidebar_header.error.clone(),
            search_selected: self.sidebar_header.selected,
            undo: self
                .thread_undos
                .iter()
                .map(|entry| (entry.environment.clone(), entry.thread.clone(), entry.kind))
                .collect(),
            catalog_revision: self.model_catalog_revision,
            pull_requests_page: self.pr_state.page,
            jump_labels: if self.jump_hints && !self.picker_open && !self.palette_open {
                keymap::jump_labels(&self.key_rules, &|name| self.key_context(name, window, cx))
            } else {
                Vec::new()
            },
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
            window.mark_view_dirty(self.sidebar_pane.entity_id());
        }
        if let Some(submitted) = self.pr_state.submitted_comment.take()
            && self.pr_comment.read(cx).value().as_ref() == submitted
        {
            self.pr_comment
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
        self.prepare_right_panel(window, cx);
        self.follow_explorer(cx);
        self.follow_workspace();
        self.follow_context_usage();
        let details_stamp = DetailsStamp {
            usage_sequence: self.context_usage_stamp.clone(),
            pull_requests: self
                .current()
                .and_then(|(environment, id)| self.thread_value(&environment, &id))
                .map(|thread| thread["pullRequests"].clone())
                .unwrap_or(Value::Null),
            target: self
                .conversation
                .workspace_watch
                .as_ref()
                .map(|watch| watch.target.clone()),
            status_revision: self.conversation.workspace_revision,
            catalog_revision: self.model_catalog_revision,
            draft_strategy: self
                .conversation
                .draft
                .as_ref()
                .map(|draft| draft.workspace_strategy.clone()),
            branch_anchor: self.conversation.branch_anchor,
            branch_request: self.conversation.branch_request,
            branch_loading: self.conversation.branch_loading,
            branch_count: self.conversation.branch_refs.len(),
            light: self.settings.light_theme,
            preferred_editor: self.settings.preferred_editor.clone(),
            lineage_revision: self.lineage_revision,
        };
        if self.details_stamp.as_ref() != Some(&details_stamp) {
            self.details_stamp = Some(details_stamp);
            window.mark_view_dirty(self.details_pane.entity_id());
        }
        if !self.focus.contains_focused(window, cx)
            && self.conversation.open.is_none()
            && self.conversation.draft.is_none()
            && !self.palette_open
            && !self.file_picker_open
            && !self.hosts_open
            && !self.settings_open
            && self.usage_page.is_none()
        {
            self.focus.focus(window, cx);
        }
        let draft_groups = self
            .conversation
            .draft
            .as_ref()
            .map(|_| self.project_groups());
        let target = self.conversation.draft.as_ref().map(|draft| {
            draft_groups
                .as_ref()
                .unwrap()
                .iter()
                .find(|group| draft.group.as_ref() == Some(&group.key))
                .map(|group| group.title.clone())
                .unwrap_or_else(|| "Pick a project".into())
        });
        self.reveal_project_match();
        let pane = self.render_conversations(window, cx);
        let terminal_drawer = self
            .terminal_key()
            .and_then(|key| self.terminal_drawers.get(&key))
            .cloned();
        let terminal_height = terminal_drawer.as_ref().map_or(0., |drawer| {
            drawer.read(cx).layout.height.clamp(
                180.,
                (f32::from(window.viewport_size().height) * 0.75)
                    .floor()
                    .max(180.),
            )
        });
        v_flex()
            .id("application-content")
            .role(accesskit::Role::Group)
            .aria_label("Application content")
            .relative()
            .size_full()
            .track_focus(&self.focus)
            .key_context("Root")
            .on_action(cx.listener(|root, _: &OpenSideChat, window, cx| root.open_side_chat(window, cx)))
            .on_action(cx.listener(|root, _: &SplitConversation, window, cx| root.split_conversation(false, window, cx)))
            .on_action(cx.listener(|root, _: &SplitConversationVertical, window, cx| root.split_conversation(true, window, cx)))
            .on_action(cx.listener(|root, _: &CloseConversation, window, cx| root.close_conversation(window, cx)))
            .on_action(cx.listener(|root, _: &FocusNextWorkspace, window, cx| root.focus_workspace(true, window, cx)))
            .on_action(cx.listener(|root, _: &FocusPreviousWorkspace, window, cx| root.focus_workspace(false, window, cx)))
            .on_action(cx.listener(|root, _: &CopyThreadReference, _, cx| root.copy_thread_reference(cx)))
            .on_action(cx.listener(|root, _: &RegenerateTitle, _, cx| root.regenerate_thread_title(cx)))
            .on_action(cx.listener(|root, _: &UndoThreadAction, _, cx| root.undo_thread_action(cx)))
            .on_action(cx.listener(|root, _: &ReopenClosed, window, cx| root.reopen_conversation(window, cx)))
            .on_action(cx.listener(|root, _: &NextDone, _, cx| root.next_done(true, cx)))
            .on_action(cx.listener(|root, _: &PreviousDone, _, cx| root.next_done(false, cx)))
            .on_action(
                cx.listener(|root, menu: &ThreadMenu, _, cx| root.thread_op(menu.clone(), cx)),
            )
            .on_action(cx.listener(|root,action:&snooze_controls::SnoozeChoice,_,cx| root.choose_snooze(action,cx)))
            .on_action(cx.listener(|root,action:&snooze_controls::CustomSnooze,window,cx| root.open_custom_snooze(action,window,cx)))
            .on_action(cx.listener(|root, _: &Find, window, cx| {
                let in_file = root.panel_focus.contains_focused(window, cx)
                    || (root.conversation.open.is_none() && root.file_panel.is_some());
                if in_file {
                    root.open_file_find(window, cx)
                } else {
                    root.open_find(window, cx)
                }
            }))
            .on_action(
                cx.listener(|root, _: &TogglePalette, window, cx| root.toggle_palette(window, cx)),
            )
            .on_action(cx.listener(|root, _: &ToggleProjectSearch, window, cx| root.toggle_project_search(window, cx)))
            .on_action(cx.listener(|root, action: &project_search::OpenMatch, _, cx| root.open_project_match(action, cx)))
            .on_action(cx.listener(|root, _: &ToggleFilePicker, window, cx| {
                root.toggle_file_picker(window, cx)
            }))
            .on_action(cx.listener(|root, _: &CopyPullRequestNumber, _, cx| root.copy_pull_request_number(cx)))
            .on_action(cx.listener(|root, action: &git_controls::GitAction, window, cx| root.git_action(action, window, cx)))
            .on_action(cx.listener(|root, action: &diff_controls::SelectDiff, _, cx| root.select_diff(action, cx)))
            .on_action(cx.listener(|root, action: &right_panel::TabAction, _, cx| root.panel_tab_action(action, cx)))
            .on_action(cx.listener(|root, _: &ToggleRightPanel, window, cx| root.toggle_right_panel(window, cx)))
            .on_action(cx.listener(|root, _: &NewRightPanel, window, cx| root.new_panel(window, cx)))
            .on_action(cx.listener(|root, _: &MaximizeRightPanel, window, cx| {
                if root.right_panel.open { root.panel_focus.focus(window, cx); root.right_panel.maximized = !root.right_panel.maximized; root.save_right_panel(); root.panel_revision += 1; cx.notify(); }
            }))
            .on_action(cx.listener(|root, _: &ToggleDiff, _, cx| root.toggle_diff(cx)))
            .on_action(cx.listener(|root, _: &OpenBranchPicker, window, cx| {
                root.open_branch_picker("composer-branch", window, cx)
            }))
            .on_action(cx.listener(|root, _: &OpenFavoriteEditor, window, cx| {
                if let Some(editor) = root.workspace_editor() {
                    let value = root.file_panel.as_ref().and_then(|panel| root.composer_cwd().map(|(_,cwd)| json!({"editor":editor,"path":std::path::Path::new(&cwd).join(&panel.path)}))).unwrap_or_else(|| json!(editor));
                    root.workspace_action(&WorkspaceAction {kind:"editor".into(),value},window,cx);
                }
            }))
            .on_action(cx.listener(|root, action: &WorkspaceAction, window, cx| {
                root.workspace_action(action, window, cx);
            }))
            .on_action(cx.listener(|root, _: &ToggleThreadDetails, _, cx| {
                if workspace::layout(root.canvas_width, root.details_open).details_inline {
                    root.details_open = !root.details_open;
                } else {
                    root.details_popover = !root.details_popover;
                }
                cx.notify();
            }))
            .on_action(cx.listener(|root, _: &ToggleExplorer, _, cx| root.toggle_explorer(cx)))
            .on_action(cx.listener(|root, _: &ToggleTerminal, window, cx| root.toggle_terminal(window, cx)))
            .on_action(cx.listener(|root, _: &SplitTerminal, window, cx| root.add_terminal(Some(false), window, cx)))
            .on_action(cx.listener(|root, _: &SplitTerminalVertical, window, cx| root.add_terminal(Some(true), window, cx)))
            .on_action(cx.listener(|root, _: &NewTerminal, window, cx| root.add_terminal(None, window, cx)))
            .on_action(cx.listener(|root, _: &CloseTerminal, window, cx| {
                if let Some(drawer) = root.terminal_key().and_then(|key| root.terminal_drawers.get(&key)).cloned() {
                    drawer.update(cx, |drawer, cx| drawer.close_active(window, cx));
                }
            }))
            .on_action(
                cx.listener(|root, _: &ManageHosts, window, cx| root.toggle_hosts(window, cx)),
            )
            .on_action(cx.listener(|root, _: &OpenSettings, _, cx| {
                root.usage_page = None;
                root.settings_open = !root.settings_open;
                cx.notify();
            }))
            .on_action(cx.listener(|root, _: &OpenUsage, window, cx| root.toggle_usage(window,cx)))
            .on_action(cx.listener(|root, _: &UsageCost, _, cx| root.select_usage(Some("cost"),None,cx)))
            .on_action(cx.listener(|root, _: &UsageTokens, _, cx| root.select_usage(Some("tokens"),None,cx)))
            .on_action(cx.listener(|root, _: &UsageLimits, _, cx| root.select_usage(Some("limits"),None,cx)))
            .on_action(cx.listener(|root, _: &UsageDay, _, cx| root.select_usage(None,Some(1),cx)))
            .on_action(cx.listener(|root, _: &UsageWeek, _, cx| root.select_usage(None,Some(7),cx)))
            .on_action(cx.listener(|root, _: &UsageMonth, _, cx| root.select_usage(None,Some(30),cx)))
            .on_action(cx.listener(|root, _: &UsageQuarter, _, cx| root.select_usage(None,Some(90),cx)))
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
            .on_action(cx.listener(|root, _: &OpenComposerHost, window, cx| root.open_composer_select(true, window, cx)))
            .on_action(cx.listener(|root, _: &CycleComposerHost, window, cx| root.cycle_composer_host(window, cx)))
            .on_action(cx.listener(|root, _: &OpenEffort, window, cx| root.open_effort(window, cx)))
            .on_action(cx.listener(|root, _: &IncreaseEffort, window, cx| root.step_effort(true, window, cx)))
            .on_action(cx.listener(|root, _: &DecreaseEffort, window, cx| root.step_effort(false, window, cx)))
            .on_action(cx.listener(|root, _: &OpenComposerWorkspace, window, cx| root.open_composer_select(false, window, cx)))
            .on_action(cx.listener(|root, _: &PreviousWorktree, window, cx| root.use_previous_worktree(window, cx)))
            .on_action(cx.listener(|root, _: &StashPrompt, window, cx| root.stash_prompt(window, cx)))
            .on_action(cx.listener(|root, _: &EditQueuedMessage, window, cx| root.edit_latest_queued(window, cx)))
            .on_action(cx.listener(|root, _: &SteerQueuedMessage, _, cx| root.steer_next_queued(cx)))
            .on_action(cx.listener(|root, _: &NewWithoutProject, window, cx| root.new_without_project(window, cx)))
            .on_action(cx.listener(|root, _: &Dictate, window, cx| root.toggle_dictation(window, cx)))
            .on_action(cx.listener(|root, _: &SendAlternate, window, cx| root.send_intent(composer::SubmissionIntent::Alternate, window, cx)))
            .on_action(cx.listener(|root, _: &SendBackground, window, cx| {
                if root.conversation.draft.is_some() { root.send_intent(composer::SubmissionIntent::Background, window, cx); }
            }))
            .on_action(cx.listener(|root, _: &SendAndNewThread, window, cx| {
                if root.conversation.draft.is_none() { root.send_intent(composer::SubmissionIntent::Background, window, cx); }
            }))
            .on_action(
                cx.listener(|root, _: &NextUserMessage, _, cx| root.jump_user_message(true, cx)),
            )
            .on_action(
                cx.listener(|root, _: &PreviousUserMessage, _, cx| {
                    root.jump_user_message(false, cx)
                }),
            )
            .on_modifiers_changed(
                cx.listener(|root, event: &ModifiersChangedEvent, window, cx| {
                    root.modifiers_changed(event.modifiers, window, cx);
                }),
            )
            .on_action(cx.listener(|root, _: &Stop, _, cx| root.stop(cx)))
            .on_action(cx.listener(|root, _: &OpenPermissions, window, cx| {
                root.open_permissions(true, window, cx)
            }))
            .on_action(cx.listener(|root, _: &CycleAppearance, _, cx| {
                root.change_setting(
                    "gpuiAppearance",
                    Value::from(if root.settings.light_theme {
                        "dark"
                    } else {
                        "light"
                    }),
                    cx,
                );
            }))
            .on_action(cx.listener(|root, _: &NextThread, _, cx| root.adjacent_thread(true, cx)))
            .on_action(
                cx.listener(|root, _: &PreviousThread, _, cx| root.adjacent_thread(false, cx)),
            )
            .on_action(cx.listener(|root, _: &Settle, _, cx| root.settle(cx)))
            .on_action(cx.listener(|root, _: &NewThread, window, cx| root.new_thread(window, cx)))
            .on_action(cx.listener(|root, _: &Cancel, window, cx| {if root.usage_page.is_some() {root.usage_page=None;root.conversation.composer.update(cx,|input,cx|input.focus(window,cx));cx.notify();} else if root.settings_open {root.settings_open=false;root.conversation.composer.update(cx,|input,cx|input.focus(window,cx));cx.notify();} else {root.cancel(window,cx);}}))
            .on_action(cx.listener(|root, _: &ToggleModelPicker, window, cx| {
                root.toggle_model_picker(window, cx)
            }))
            .on_action(cx.listener(|root, action: &PickProjectAction, _, cx| {
                if let Some(draft) = root.conversation.draft.as_mut() {
                    draft.group = Some(action.key.clone());
                    draft.environment = None;
                    draft.workspace_strategy = json!({"type":"root"});
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|root, action: &PickModelAction, _, cx| {
                root.pick_model(action.instance_id.clone(), action.slug.clone(), cx)
            }))
            .on_action(cx.listener(|root, action: &SetModelOptionAction, _, cx| {
                root.set_model_option(&action.id, action.value.clone(), cx)
            }))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(cx.theme().font_family.clone())
            .on_mouse_move(cx.listener(|root, event: &MouseMoveEvent, window, cx| {
                if event.pressed_button == Some(MouseButton::Left)
                    && let Some(drawer) = root.terminal_key().and_then(|key| root.terminal_drawers.get(&key)).cloned() {
                    drawer.update(cx, |drawer, cx| drawer.drag_resize(f32::from(event.position.y), f32::from(window.viewport_size().height), cx));
                }
                if root.panel_resizing && event.pressed_button == Some(MouseButton::Left) {
                    root.right_panel_width = (f32::from(window.viewport_size().width) - f32::from(event.position.x)).max(360.);
                    root.right_panel_width = root.panel_width(f32::from(window.viewport_size().width));
                    cx.notify();
                }
                if let Some((path, vertical, bounds)) = root.pane_resize.clone() && event.pressed_button == Some(MouseButton::Left) {
                    let (position, start, length) = if vertical { (event.position.y, bounds.origin.y, bounds.size.height) } else { (event.position.x, bounds.origin.x, bounds.size.width) };
                    if length > px(0.) { root.pane_layout.resize(&path, f32::from(position - start) / f32::from(length)); cx.notify(); }
                }
                if root.sidebar_resizing && event.pressed_button == Some(MouseButton::Left) {
                    let max = (f32::from(window.viewport_size().width) - 640.).max(208.);
                    root.settings.sidebar_width = f32::from(event.position.x).clamp(208., max);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|root, _, _, cx| {
                    if let Some(drawer) = root.terminal_key().and_then(|key| root.terminal_drawers.get(&key)).cloned() { drawer.update(cx, |drawer, cx| drawer.end_resize(cx)); }
                    root.window_drag_pending = false;
                    if root.panel_resizing { root.save_right_panel(); }
                    root.panel_resizing = false;
                    if root.pane_resize.take().is_some() { root.save_conversations(); }
                    if root.sidebar_resizing {
                        root.sidebar_resizing = false;
                        if let Err(error) = client_settings::update(
                            &root.settings_path,
                            &[(
                                "gpuiSidebarWidth",
                                serde_json::json!(root.settings.sidebar_width),
                            )],
                        ) {
                            root.conversation.message = Some(format!("Cannot save sidebar width: {error:#}"));
                        }
                        cx.notify();
                    }
                }),
            )
            .child(self.render_titlebar(window, target, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
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
                                .id("sidebar-resize")
                                .role(accesskit::Role::Button)
                                .aria_label("Resize Sidebar")
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
                    .when(!self.pr_state.page && !(self.right_panel.open && (self.right_panel.maximized || f32::from(window.viewport_size().width) <= 980.)), |row| row.child(v_flex().flex_1().h_full().min_w_0().min_h_0()
                        .child(div().flex_1().h_0().min_h_0().child(self.render_workspace_canvas(pane, cx)))
                        .children(terminal_drawer.map(|drawer| drawer.cached(StyleRefinement::default().w_full().h(px(terminal_height)).flex_none()).into_any_element()))))
                    .when(self.pr_state.page, |row| row.child(self.pr_pane.clone().cached(StyleRefinement::default().flex_1().h_full().min_w_0())))
                    .when(!self.pr_state.page && self.right_panel.open, |row| row
                        .child(div().w(px(4.)).h_full().cursor_col_resize().on_mouse_down(MouseButton::Left, cx.listener(|root, _, _, cx| { root.panel_resizing = true; cx.notify(); })))
                        .child(self.panel_pane.clone().cached(if self.right_panel.maximized || f32::from(window.viewport_size().width) <= 980. { StyleRefinement::default().flex_1().h_full().min_w_0() } else { StyleRefinement::default().w(px(self.panel_width(f32::from(window.viewport_size().width)))).h_full().flex_none() }))),
            )
            .when(!self.sidebar_hidden, |root_el| root_el.children(self.render_thread_undo(cx)))
            .when(self.sidebar_header.scope_open, |root_el| root_el.child(self.render_sidebar_scope(cx)))
            .when(self.project_search.open, |root_el| root_el.child(self.render_project_search(window, cx)))
            .when(self.palette_open, |root_el| {
                root_el.child(self.render_palette(cx))
            })
            .when(self.hosts_open, |root_el| {
                root_el.child(self.render_hosts(cx))
            })
            .children(self.usage_page.clone().map(|page| div().absolute().inset_0().p(px(40.)).flex().items_center().justify_center().bg(cx.theme().background.opacity(0.8)).child(div().size_full().max_w(px(1280.)).max_h(px(960.)).rounded(px(16.)).overflow_hidden().border_1().border_color(cx.theme().border).child(page.cached(StyleRefinement::default().size_full())))))
            .when(self.settings_open, |root_el| {
                root_el.child(self.render_settings(window,cx))
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
            crate::timestamp::initialize(settings.timestamp_format, cx)
                .expect("initialize timestamp formatting");
            crate::appearance::apply(settings.light_theme, cx);
            crate::appearance::apply_typography(&settings, cx);
            Theme::update(cx, |theme| {
                theme.cursor_blink =
                    CursorBlink::blinking(CURSOR_BLINK).with_idle_timeout(CURSOR_BLINK_IDLE);
            });
            let root = Some("Root");
            cx.bind_keys([KeyBinding::new("escape", Cancel, root)]);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1400.), px(900.)), cx)),
                titlebar: Some(TitlebarOptions {
                    title: Some(APP_NAME.into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(12.), px(12.))),
                }),
                window_decorations: Some(WindowDecorations::Client),
                window_background: WindowBackgroundAppearance::Transparent,
                app_owns_titlebar_drag: true,
                app_id: Some("t3code-gpui".into()),
                window_min_size: Some(size(px(840.), px(620.))),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                if std::env::var_os("T3CODE_GPUI_RENDERER_INFO").is_some() {
                    let renderer = window.gpu_specs().map(|gpu| {
                        json!({
                            "is_software_emulated": gpu.is_software_emulated,
                            "device_name": gpu.device_name,
                            "driver_name": gpu.driver_name,
                            "driver_info": gpu.driver_info,
                        })
                    });
                    eprintln!("T3CODE_GPUI_RENDERER_INFO {}", json!(renderer));
                }
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
