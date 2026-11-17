use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TrySendError};
use serde_json::{Value, json};
use t3_client::rpc::{BoundedChunk, StreamLimits};
use t3_client::session::Connection;
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::{KeyboardEvent, Mouse, Point, Range, Scrollbar, Snapshot, Terminal, Theme};

#[derive(Clone, Debug)]
pub struct Target {
    pub thread_id: String,
    pub terminal_id: String,
    pub cwd: String,
    pub worktree_path: Option<String>,
    pub project_root: String,
    pub read_only: bool,
    pub restart_if_not_running: bool,
}

impl Target {
    pub fn identity(&self) -> Value {
        json!({"threadId": self.thread_id, "terminalId": self.terminal_id})
    }

    fn attach(&self, size: Grid, replied: bool) -> Value {
        let mut value = self.identity();
        if self.read_only {
            return value;
        }
        value["cwd"] = json!(self.cwd);
        value["worktreePath"] = json!(self.worktree_path);
        value["cols"] = json!(size.cols);
        value["rows"] = json!(size.rows);
        value["env"] = json!({"T3CODE_PROJECT_ROOT": self.project_root});
        if let Some(path) = &self.worktree_path {
            value["env"]["T3CODE_WORKTREE_PATH"] = json!(path);
        }
        if self.restart_if_not_running {
            value["restartIfNotRunning"] = json!(true);
        }
        if replied {
            value["resume"] = json!(true);
        }
        value
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

impl Grid {
    pub fn new(cols: u16, rows: u16, cell_width: u16, cell_height: u16) -> Self {
        Self {
            cols: cols.clamp(1, 1000),
            rows: rows.clamp(1, 500),
            cell_width: cell_width.max(1),
            cell_height: cell_height.max(1),
        }
    }
}

#[derive(Clone, Debug)]
pub struct State {
    pub snapshot: Option<Arc<Snapshot>>,
    pub scrollbar: Option<Arc<Scrollbar>>,
    pub selection: Option<Range>,
    pub status: String,
    pub label: String,
    pub error: Option<String>,
    pub connected: bool,
    pub has_running_subprocess: bool,
    pub grid: Grid,
    pub revision: u64,
    pub modes: Modes,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    pub alternate_screen: bool,
    pub mouse_tracking: bool,
    pub mouse_any_event: bool,
    pub application_cursor_keys: bool,
    pub synchronized_output: bool,
}

pub enum Input {
    Key(KeyboardEvent),
    KeyWithFallback(KeyboardEvent, Option<String>),
    Text(String),
    Paste(String),
    Mouse(Mouse),
    Resize(Grid),
    Theme(Theme),
    Visible(bool),
    Scroll(i32),
    ScrollToBottom,
    SelectAll,
    SelectWord(Point),
    SelectLine(Point),
    SelectRange(Point, Point),
    BeginSelection(Point, usize),
    ExtendSelection(Point),
    EndSelection,
    ScrollSelection(i32, Point),
    ClearSelection,
    CopySelection { clear: bool },
    ForgetKey(String),
    Clear,
}

#[derive(Debug)]
pub enum Event {
    Copy(String),
}

enum Command {
    Input(Input),
    Chunk(BoundedChunk),
    Disconnected(String),
    WriteFailed,
    Stop,
}

enum Write {
    Text(String),
    Resize(Grid),
    Clear,
}

pub struct Session {
    pub target: Target,
    commands: Sender<Command>,
    stopped: Arc<AtomicBool>,
    network: JoinHandle<()>,
    writer: JoinHandle<()>,
    worker: JoinHandle<()>,
}

impl Session {
    pub fn new(
        connection: Arc<Connection>,
        target: Target,
        grid: Grid,
        theme: Theme,
        runtime: &Handle,
    ) -> (Self, watch::Receiver<State>, async_channel::Receiver<Event>) {
        let state = State {
            snapshot: None,
            scrollbar: None,
            selection: None,
            status: "starting".into(),
            label: String::new(),
            error: None,
            connected: false,
            has_running_subprocess: false,
            grid,
            revision: 0,
            modes: Modes::default(),
        };
        let (updates, receiver) = watch::channel(state.clone());
        let (commands, inputs) = crossbeam_channel::bounded(64);
        let (writes, outgoing) = async_channel::bounded(64);
        let (events, event_receiver) = async_channel::unbounded();
        let stopped = Arc::new(AtomicBool::new(false));
        let write_error = Arc::new(Mutex::new(None));
        let writer = runtime.spawn(write_loop(
            connection.clone(),
            target.clone(),
            outgoing,
            commands.clone(),
            write_error.clone(),
        ));
        let network = runtime.spawn(stream_loop(
            connection,
            target.clone(),
            receiver.clone(),
            commands.clone(),
        ));
        let worker = runtime.spawn_blocking({
            let stopped = stopped.clone();
            let read_only = target.read_only;
            move || {
                let mut worker = Worker {
                    terminal: None,
                    state,
                    theme,
                    updates,
                    events,
                    writes,
                    stopped,
                    write_error,
                    visible: true,
                    dirty: true,
                    frame_due: None,
                    sync_due: None,
                    output_since: None,
                    last_output: None,
                    sequence: None,
                    encoded_keys: HashSet::new(),
                    mouse_motion: Vec::new(),
                    selection_drag: None,
                    read_only,
                };
                if let Err(error) = worker.run(inputs) {
                    worker.state.error = Some(format!("{error:#}"));
                    worker.state.connected = false;
                    worker.publish_metadata();
                }
            }
        });
        (
            Self {
                target,
                commands,
                stopped,
                network,
                writer,
                worker,
            },
            receiver,
            event_receiver,
        )
    }

    pub fn input(&self, input: Input) -> Result<()> {
        if self.target.read_only
            && matches!(
                input,
                Input::Text(_) | Input::Paste(_) | Input::Mouse(_) | Input::Clear
            )
        {
            bail!("Terminal is read-only");
        }
        self.commands
            .try_send(Command::Input(input))
            .map_err(|error| match error {
                TrySendError::Full(_) => anyhow::anyhow!("Terminal input queue is full"),
                TrySendError::Disconnected(_) => anyhow::anyhow!("Terminal session ended"),
            })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.network.abort();
        self.writer.abort();
        let _ = self.commands.try_send(Command::Stop);
        self.worker.abort();
    }
}

async fn deliver(sender: &Sender<Command>, command: Command) -> Result<()> {
    match sender.try_send(command) {
        Ok(()) => Ok(()),
        Err(TrySendError::Disconnected(_)) => bail!("Terminal worker ended"),
        Err(TrySendError::Full(command)) => {
            let sender = sender.clone();
            tokio::task::spawn_blocking(move || {
                sender
                    .send(command)
                    .map_err(|_| anyhow::anyhow!("Terminal worker ended"))
            })
            .await?
        }
    }
}

async fn stream_loop(
    connection: Arc<Connection>,
    target: Target,
    state: watch::Receiver<State>,
    commands: Sender<Command>,
) {
    let mut replied = false;
    let mut delay = Duration::from_secs(1);
    loop {
        let result: Result<()> = async {
            let grid = state.borrow().grid;
            let mut stream = connection
                .subscribe(
                    if target.read_only {
                        "terminal.observe"
                    } else {
                        "terminal.attach"
                    },
                    target.attach(grid, replied),
                    StreamLimits::new(64, 8 << 20).expect("nonzero terminal stream limits"),
                )
                .await?;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                replied = true;
                delay = Duration::from_secs(1);
                deliver(&commands, Command::Chunk(chunk)).await?;
            }
            bail!("Terminal stream ended")
        }
        .await;
        if let Err(error) = result
            && deliver(&commands, Command::Disconnected(format!("{error:#}")))
                .await
                .is_err()
        {
            return;
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(30));
    }
}

async fn write_loop(
    connection: Arc<Connection>,
    target: Target,
    outgoing: async_channel::Receiver<Write>,
    commands: Sender<Command>,
    error_slot: Arc<Mutex<Option<String>>>,
) {
    while let Ok(write) = outgoing.recv().await {
        if target.read_only {
            continue;
        }
        let result: Result<()> = async {
            match write {
                Write::Text(data) => {
                    let mut text = data.as_str();
                    while !text.is_empty() {
                        let end = packet_end(text);
                        let mut payload = target.identity();
                        payload["data"] = json!(&text[..end]);
                        connection.request("terminal.write", payload).await?;
                        text = &text[end..];
                    }
                }
                Write::Resize(grid) => {
                    let mut payload = target.identity();
                    payload["cols"] = json!(grid.cols);
                    payload["rows"] = json!(grid.rows);
                    connection.request("terminal.resize", payload).await?;
                }
                Write::Clear => {
                    connection
                        .request("terminal.clear", target.identity())
                        .await?;
                }
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            *error_slot.lock().expect("terminal write error lock") = Some(format!("{error:#}"));
            let _ = commands.try_send(Command::WriteFailed);
        }
    }
}

fn packet_end(text: &str) -> usize {
    let mut end = text.len().min(65536);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

struct Worker {
    terminal: Option<Terminal>,
    state: State,
    theme: Theme,
    updates: watch::Sender<State>,
    events: async_channel::Sender<Event>,
    writes: async_channel::Sender<Write>,
    stopped: Arc<AtomicBool>,
    write_error: Arc<Mutex<Option<String>>>,
    visible: bool,
    dirty: bool,
    frame_due: Option<Instant>,
    sync_due: Option<Instant>,
    output_since: Option<Instant>,
    last_output: Option<Instant>,
    sequence: Option<u64>,
    encoded_keys: HashSet<String>,
    mouse_motion: Vec<u8>,
    selection_drag: Option<SelectionDrag>,
    read_only: bool,
}

struct SelectionDrag {
    anchor: Point,
    base: Option<Range>,
    mode: usize,
    moved: bool,
}

impl Worker {
    fn run(&mut self, inputs: Receiver<Command>) -> Result<()> {
        let grid = self.state.grid;
        self.terminal = Some(Terminal::new(
            grid.cols,
            grid.rows,
            grid.cell_width,
            grid.cell_height,
            &self.theme,
        )?);
        self.render()?;
        while !self.stopped.load(Ordering::Acquire) {
            let due = self.frame_due.into_iter().chain(self.sync_due).min();
            let command = match due {
                Some(due) => {
                    match inputs.recv_timeout(due.saturating_duration_since(Instant::now())) {
                        Ok(command) => Some(command),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                None => Some(match inputs.recv() {
                    Ok(command) => command,
                    Err(_) => break,
                }),
            };
            match command {
                Some(Command::Stop) => break,
                Some(Command::Disconnected(error)) => {
                    self.state.connected = false;
                    self.state.error = Some(error);
                    self.publish_metadata();
                }
                Some(Command::Chunk(chunk)) => {
                    let before = self.metadata();
                    for event in chunk.values() {
                        self.apply(event)?;
                    }
                    chunk.accept();
                    self.refresh_modes()?;
                    if before != self.metadata() {
                        self.publish_metadata();
                    }
                }
                Some(Command::Input(input)) => self.input(input)?,
                Some(Command::WriteFailed) | None => {}
            }
            let error = self
                .write_error
                .lock()
                .expect("terminal write error lock")
                .take();
            if let Some(error) = error {
                self.state.error = Some(error);
                self.publish_metadata();
            }
            let now = Instant::now();
            if self.sync_due.is_some_and(|due| due <= now) {
                self.term().end_synchronized_output()?;
                self.sync_due = None;
                self.schedule(true)?;
                self.refresh_modes()?;
                self.publish_metadata();
            }
            if self.frame_due.is_some_and(|due| due <= now) {
                self.frame_due = None;
                if self.visible && !self.term().mode(2026)? {
                    self.render()?;
                }
            }
        }
        Ok(())
    }

    fn term(&mut self) -> &mut Terminal {
        self.terminal.as_mut().unwrap()
    }

    fn metadata(&self) -> (String, String, Option<String>, bool, bool, Grid, Modes) {
        (
            self.state.status.clone(),
            self.state.label.clone(),
            self.state.error.clone(),
            self.state.connected,
            self.state.has_running_subprocess,
            self.state.grid,
            self.state.modes,
        )
    }

    fn publish_metadata(&mut self) {
        self.state.revision = self.state.revision.wrapping_add(1);
        self.updates.send_replace(self.state.clone());
    }

    fn render(&mut self) -> Result<()> {
        if !self.visible || !self.dirty {
            return Ok(());
        }
        self.refresh_modes()?;
        self.state.snapshot = Some(Arc::new(self.term().snapshot()?));
        self.state.scrollbar = self.term().scrollbar()?.map(Arc::new);
        self.state.selection = self.term().selection_range()?;
        self.dirty = false;
        self.publish_metadata();
        Ok(())
    }

    fn refresh_modes(&mut self) -> Result<()> {
        let modes = Modes {
            alternate_screen: self.term().is_alternate_screen()?,
            mouse_tracking: self.term().mouse_tracking()?,
            mouse_any_event: self.term().mode(1003)?,
            application_cursor_keys: self.term().mode(1)?,
            synchronized_output: self.term().mode(2026)?,
        };
        if modes.mouse_any_event != self.state.modes.mouse_any_event {
            self.mouse_motion.clear();
        }
        self.state.modes = modes;
        Ok(())
    }

    fn schedule(&mut self, immediate: bool) -> Result<()> {
        self.dirty = true;
        if self.term().mode(2026)? {
            self.frame_due = None;
            self.sync_due
                .get_or_insert_with(|| Instant::now() + Duration::from_millis(500));
            return Ok(());
        }
        self.sync_due = None;
        if !self.visible {
            self.frame_due = None;
            return Ok(());
        }
        let now = Instant::now();
        let continuous = self
            .output_since
            .is_some_and(|since| now.duration_since(since) >= Duration::from_secs(1));
        let due = now
            + if immediate {
                Duration::ZERO
            } else if continuous {
                Duration::from_millis(250)
            } else {
                Duration::from_millis(33)
            };
        self.frame_due = Some(self.frame_due.map_or(due, |old| old.min(due)));
        Ok(())
    }

    fn write(&self, data: Vec<u8>) -> Result<()> {
        if data.is_empty() || self.read_only {
            return Ok(());
        }
        self.writes
            .send_blocking(Write::Text(
                String::from_utf8(data).context("Terminal input must be UTF-8")?,
            ))
            .context("Terminal writer ended")
    }

    fn apply(&mut self, event: &Value) -> Result<()> {
        self.state.connected = true;
        self.state.error = None;
        match event["type"].as_str() {
            Some("snapshot" | "restarted" | "started") => {
                let snapshot = &event["snapshot"];
                self.shared_grid(snapshot)?;
                self.term()
                    .reset_and_write(snapshot["history"].as_str().unwrap_or_default().as_bytes())?;
                self.state.status = snapshot["status"].as_str().unwrap_or("starting").to_owned();
                self.state.label = snapshot["label"].as_str().unwrap_or_default().to_owned();
                self.state.has_running_subprocess =
                    snapshot["hasRunningSubprocess"].as_bool().unwrap_or(false);
                self.sequence = snapshot["sequence"].as_u64();
                self.output_since = None;
                self.last_output = None;
                self.schedule(true)?;
            }
            Some("output") => {
                let sequence = event["sequence"].as_u64();
                if sequence
                    .zip(self.sequence)
                    .is_some_and(|(next, previous)| next <= previous)
                {
                    return Ok(());
                }
                let ended = self
                    .term()
                    .write_with_sync_end(event["data"].as_str().unwrap_or_default().as_bytes())?;
                if ended {
                    self.sync_due = None;
                }
                if sequence.is_some() {
                    self.sequence = sequence;
                }
                let now = Instant::now();
                if self
                    .last_output
                    .is_none_or(|last| now.duration_since(last) > Duration::from_millis(250))
                {
                    self.output_since = Some(now);
                }
                self.last_output = Some(now);
                if self.state.status == "closed" {
                    self.state.status = "running".into();
                }
                for reply in self.term().take_replies() {
                    self.write(reply)?;
                }
                self.schedule(false)?;
            }
            Some("activity") => {
                self.shared_grid(event)?;
                self.state.label = event["label"].as_str().unwrap_or_default().to_owned();
                self.state.has_running_subprocess =
                    event["hasRunningSubprocess"].as_bool().unwrap_or(false);
                self.schedule(false)?;
            }
            Some("cleared") => {
                self.term().reset_and_write(b"")?;
                self.schedule(true)?;
            }
            Some("exited") => self.state.status = "exited".into(),
            Some("closed") => self.state.status = "closed".into(),
            Some("error") => self.state.error = event["message"].as_str().map(str::to_owned),
            _ => {}
        }
        Ok(())
    }

    fn shared_grid(&mut self, value: &Value) -> Result<()> {
        if let (Some(cols), Some(rows)) = (value["cols"].as_u64(), value["rows"].as_u64()) {
            let grid = self.state.grid;
            let grid = Grid::new(
                cols.min(1000) as u16,
                rows.min(500) as u16,
                grid.cell_width,
                grid.cell_height,
            );
            self.term()
                .resize(grid.cols, grid.rows, grid.cell_width, grid.cell_height)?;
            self.state.grid = grid;
        }
        Ok(())
    }

    fn copy(&mut self, clear: bool) -> Result<()> {
        let text = self.term().selection_text()?;
        if !text.is_empty() {
            let _ = self.events.try_send(Event::Copy(text));
        }
        if clear {
            self.term().clear_selection()?;
            self.schedule(true)?;
        }
        Ok(())
    }

    fn input(&mut self, input: Input) -> Result<()> {
        match input {
            Input::Visible(visible) => {
                self.visible = visible;
                if visible {
                    self.schedule(true)?;
                } else {
                    self.frame_due = None;
                }
            }
            Input::Key(event) => {
                self.key(event)?;
            }
            Input::KeyWithFallback(event, text) => {
                let release = event.release;
                let composing = event.composing;
                if !self.key(event)?
                    && !release
                    && !composing
                    && let Some(text) = text
                {
                    self.write(text.into_bytes())?;
                    self.schedule(true)?;
                }
            }
            Input::Text(text) => self.write(text.into_bytes())?,
            Input::Paste(text) => {
                let bytes = self.term().encode_paste(&text)?;
                self.write(bytes)?;
            }
            Input::Mouse(mouse) => {
                let bytes = self.term().encode_mouse(&mouse)?;
                let motion = matches!(mouse.action, crate::MouseAction::Motion);
                let send = !bytes.is_empty() && (!motion || bytes != self.mouse_motion);
                if motion {
                    self.mouse_motion = bytes.clone();
                } else {
                    self.mouse_motion.clear();
                }
                if send {
                    self.write(bytes)?;
                }
            }
            Input::Resize(mut grid) => {
                if self.read_only {
                    grid.cols = self.state.grid.cols;
                    grid.rows = self.state.grid.rows;
                }
                if grid != self.state.grid {
                    self.term()
                        .resize(grid.cols, grid.rows, grid.cell_width, grid.cell_height)?;
                    self.state.grid = grid;
                    if !self.read_only {
                        self.writes
                            .send_blocking(Write::Resize(grid))
                            .context("Terminal writer ended")?;
                    }
                    self.schedule(true)?;
                }
            }
            Input::Theme(theme) => {
                self.term().set_theme(&theme)?;
                self.theme = theme;
                self.schedule(true)?;
            }
            Input::Scroll(rows) => {
                self.term().scroll(rows)?;
                self.schedule(true)?;
            }
            Input::ScrollToBottom => {
                self.term().scroll_to_bottom()?;
                self.schedule(true)?;
            }
            Input::SelectAll => {
                self.term().select_all()?;
                self.schedule(true)?;
            }
            Input::SelectWord(point) => {
                self.term().select_word(point)?;
                self.schedule(true)?;
            }
            Input::SelectLine(point) => {
                self.term().select_line(point)?;
                self.schedule(true)?;
            }
            Input::SelectRange(start, end) => {
                self.term().set_selection(start, end, 1, 1)?;
                self.schedule(true)?;
            }
            Input::BeginSelection(point, count) => self.begin_selection(point, count)?,
            Input::ExtendSelection(point) => self.extend_selection(point)?,
            Input::ScrollSelection(rows, point) => {
                self.term().scroll(rows)?;
                self.extend_selection(point)?;
            }
            Input::EndSelection => {
                if self
                    .selection_drag
                    .take()
                    .is_some_and(|drag| drag.mode == 1 && !drag.moved)
                {
                    self.term().clear_selection()?;
                    self.schedule(true)?;
                }
            }
            Input::ClearSelection => {
                self.selection_drag = None;
                self.term().clear_selection()?;
                self.schedule(true)?;
            }
            Input::CopySelection { clear } => self.copy(clear)?,
            Input::ForgetKey(code) => {
                self.encoded_keys.remove(&code);
            }
            Input::Clear => {
                ensure!(!self.read_only, "Terminal is read-only");
                self.writes
                    .send_blocking(Write::Clear)
                    .context("Terminal writer ended")?;
            }
        }
        Ok(())
    }

    fn begin_selection(&mut self, point: Point, count: usize) -> Result<()> {
        let mode = count.clamp(1, 3);
        let range = match mode {
            2 => self.term().select_word(point)?,
            3 => self.term().select_line(point)?,
            _ => None,
        };
        let anchor = if let Some(range) = &range {
            Some(range.screen.start)
        } else {
            self.term().convert_point(point, 1, 2)?
        };
        self.selection_drag = anchor.map(|anchor| SelectionDrag {
            anchor,
            base: range.map(|range| range.screen),
            mode,
            moved: false,
        });
        if mode == 1
            && let Some(anchor) = anchor
        {
            self.term().set_selection(anchor, anchor, 2, 2)?;
        }
        self.schedule(true)
    }

    fn extend_selection(&mut self, point: Point) -> Result<()> {
        let Some(mut drag) = self.selection_drag.take() else {
            return Ok(());
        };
        let Some(anchor) = self.term().selection_range()?.map(|range| range.start) else {
            return Ok(());
        };
        let shift = i64::from(anchor.y) - i64::from(drag.anchor.y);
        if let Some(base) = &mut drag.base {
            base.start.y = (i64::from(base.start.y) + shift).clamp(0, i64::from(u32::MAX)) as u32;
            base.end.y = (i64::from(base.end.y) + shift).clamp(0, i64::from(u32::MAX)) as u32;
        }
        let range = match drag.mode {
            2 => self.term().select_word(point)?,
            3 => self.term().select_line(point)?,
            _ => None,
        };
        if let Some(cell) = self.term().convert_point(point, 1, 2)? {
            let before = drag
                .base
                .as_ref()
                .is_some_and(|base| (cell.y, cell.x) < (base.start.y, base.start.x));
            let start = drag
                .base
                .as_ref()
                .map_or(anchor, |base| if before { base.end } else { base.start });
            let end = range.map_or(cell, |range| {
                if before {
                    range.screen.start
                } else {
                    range.screen.end
                }
            });
            self.term().set_selection(start, end, 2, 2)?;
            drag.anchor = start;
            drag.moved = true;
        }
        self.selection_drag = Some(drag);
        self.schedule(true)
    }

    fn key(&mut self, event: KeyboardEvent) -> Result<bool> {
        if event.release {
            if !self.encoded_keys.remove(&event.code) {
                return Ok(true);
            }
        } else if !event.repeat {
            self.encoded_keys.remove(&event.code);
        }
        if event.composing {
            return Ok(false);
        }
        if !event.release {
            if let Some(data) = event.local_data() {
                self.write(data.as_bytes().to_vec())?;
                return Ok(true);
            }
            let command = if event.mac {
                event.meta && !event.control
            } else {
                event.control && !event.meta
            };
            if command
                && !event.alt
                && event.key.eq_ignore_ascii_case("a")
                && if event.mac { !event.shift } else { event.shift }
            {
                self.term().select_all()?;
                self.schedule(true)?;
                return Ok(true);
            }
            let page = event.shift
                && !event.alt
                && !event.control
                && !event.meta
                && matches!(event.key.as_str(), "PageUp" | "PageDown");
            let jump = event.shift
                && !event.alt
                && command
                && matches!(event.key.as_str(), "Home" | "End");
            if (page || jump)
                && !self.term().is_alternate_screen()?
                && let Some(scrollbar) = self.term().scrollbar()?
            {
                let amount = match event.key.as_str() {
                    "Home" => -(scrollbar.offset.min(i32::MAX as u64) as i32),
                    "End" => scrollbar
                        .total
                        .saturating_sub(scrollbar.len)
                        .saturating_sub(scrollbar.offset)
                        .min(i32::MAX as u64) as i32,
                    "PageUp" => -(scrollbar.len.max(1).min(i32::MAX as u64) as i32),
                    _ => scrollbar.len.max(1).min(i32::MAX as u64) as i32,
                };
                self.term().scroll(amount)?;
                self.schedule(true)?;
                return Ok(true);
            }
            if event.is_copy() && self.term().selection_range()?.is_some() {
                self.copy(!event.mac && !event.shift && !event.key.eq_ignore_ascii_case("insert"))?;
                return Ok(true);
            }
        }
        if self.read_only {
            return Ok(true);
        }
        let bytes = self.term().encode_key(&event.encoded())?;
        if bytes.is_empty() {
            return Ok(false);
        }
        {
            if !event.release {
                self.encoded_keys.insert(event.code);
            }
            self.write(bytes)?;
            self.schedule(true)?;
        }
        Ok(true)
    }
}

pub async fn close(connection: &Connection, target: &Target) -> Result<()> {
    ensure!(!target.read_only, "Terminal is read-only");
    let mut payload = target.identity();
    payload["deleteHistory"] = json!(true);
    if connection.request("terminal.close", payload).await.is_err() {
        let mut payload = target.identity();
        payload["data"] = json!("exit\n");
        connection.request("terminal.write", payload).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker() -> (
        Worker,
        async_channel::Receiver<Write>,
        async_channel::Receiver<Event>,
    ) {
        let grid = Grid::new(12, 4, 8, 18);
        let theme = Theme {
            foreground: crate::Color {
                r: 245,
                g: 245,
                b: 245,
            },
            background: crate::Color::default(),
            cursor: crate::Color {
                r: 180,
                g: 203,
                b: 255,
            },
            palette: None,
        };
        let state = State {
            snapshot: None,
            scrollbar: None,
            selection: None,
            status: "starting".into(),
            label: String::new(),
            error: None,
            connected: false,
            has_running_subprocess: false,
            grid,
            revision: 0,
            modes: Modes::default(),
        };
        let (updates, _) = watch::channel(state.clone());
        let (writes, outgoing) = async_channel::bounded(64);
        let (events, receiver) = async_channel::unbounded();
        (
            Worker {
                terminal: Some(Terminal::new(12, 4, 8, 18, &theme).unwrap()),
                state,
                theme,
                updates,
                events,
                writes,
                stopped: Arc::new(AtomicBool::new(false)),
                write_error: Arc::new(Mutex::new(None)),
                visible: true,
                dirty: true,
                frame_due: None,
                sync_due: None,
                output_since: None,
                last_output: None,
                sequence: None,
                encoded_keys: HashSet::new(),
                mouse_motion: Vec::new(),
                selection_drag: None,
                read_only: false,
            },
            outgoing,
            receiver,
        )
    }

    #[test]
    fn history_replay_suppresses_queries_and_duplicate_output_is_not_applied_twice() -> Result<()> {
        let (mut worker, writes, _) = worker();
        worker.apply(&json!({"type":"snapshot","snapshot":{"history":"old\u{1b}[5n", "sequence":3, "status":"running","cols":12,"rows":4}}))?;
        assert!(writes.is_empty());
        worker.apply(&json!({"type":"output","sequence":4,"data":"new\u{1b}[5n"}))?;
        assert!(matches!(writes.try_recv()?, Write::Text(text) if text == "\u{1b}[0n"));
        worker.apply(&json!({"type":"output","sequence":4,"data":"duplicate"}))?;
        worker.render()?;
        assert_eq!(
            worker.state.snapshot.as_ref().unwrap().row_data[0]
                .text
                .trim_end(),
            "oldnew"
        );
        assert!(writes.is_empty());
        Ok(())
    }

    #[test]
    fn hidden_output_keeps_terminal_state_without_publishing_frames() -> Result<()> {
        let (mut worker, _, _) = worker();
        worker.render()?;
        let previous = worker.state.snapshot.clone().unwrap();
        let revision = worker.state.revision;
        worker.input(Input::Visible(false))?;
        worker.apply(&json!({"type":"output","data":"hidden"}))?;
        worker.render()?;
        assert!(worker.frame_due.is_none());
        assert!(Arc::ptr_eq(
            &previous,
            worker.state.snapshot.as_ref().unwrap()
        ));
        assert_eq!(worker.state.revision, revision);
        worker.input(Input::Visible(true))?;
        worker.render()?;
        assert_eq!(
            worker.state.snapshot.as_ref().unwrap().row_data[0]
                .text
                .trim_end(),
            "hidden"
        );
        Ok(())
    }

    #[test]
    fn synchronized_output_has_a_bounded_deadline_and_reopened_groups_get_a_new_one() -> Result<()>
    {
        let (mut worker, _, _) = worker();
        worker.apply(&json!({"type":"output","data":"\u{1b}[?2026hfirst"}))?;
        let deadline = worker.sync_due.unwrap();
        assert!(deadline.duration_since(Instant::now()) <= Duration::from_millis(500));
        assert!(worker.frame_due.is_none());
        worker.sync_due = Some(Instant::now() - Duration::from_millis(1));
        worker.apply(&json!({"type":"output","data":"\u{1b}[?2026l\u{1b}[?2026hsecond"}))?;
        assert!(worker.sync_due.unwrap() > Instant::now());
        worker.apply(&json!({"type":"output","data":"\u{1b}[?2026l"}))?;
        assert!(worker.sync_due.is_none());
        assert!(worker.frame_due.is_some());
        Ok(())
    }

    #[test]
    fn plain_control_c_copies_once_then_sends_sigint_and_read_only_selection_still_works()
    -> Result<()> {
        let (mut worker, writes, events) = worker();
        worker.term().write(b"alpha")?;
        worker.input(Input::SelectAll)?;
        let key = KeyboardEvent {
            key: "c".into(),
            code: "KeyC".into(),
            control: true,
            ..KeyboardEvent::default()
        };
        worker.key(key.clone())?;
        assert!(matches!(events.try_recv()?, Event::Copy(text) if text.contains("alpha")));
        assert!(writes.is_empty());
        assert!(worker.term().selection_range()?.is_none());
        worker.key(key.clone())?;
        assert!(matches!(writes.try_recv()?, Write::Text(text) if text == "\u{3}"));
        worker.read_only = true;
        worker.input(Input::SelectAll)?;
        worker.key(key)?;
        assert!(matches!(events.try_recv()?, Event::Copy(text) if text.contains("alpha")));
        worker.input(Input::Resize(Grid::new(90, 40, 9, 20)))?;
        assert_eq!(worker.state.grid, Grid::new(12, 4, 9, 20));
        assert!(writes.is_empty());
        Ok(())
    }

    #[test]
    fn kitty_release_requires_an_encoded_press_and_global_keyup_only_forgets_it() -> Result<()> {
        let (mut worker, writes, _) = worker();
        worker.term().write(b"\x1b[>11u")?;
        let key = KeyboardEvent {
            key: "a".into(),
            code: "KeyA".into(),
            ..KeyboardEvent::default()
        };
        worker.key(KeyboardEvent {
            release: true,
            ..key.clone()
        })?;
        assert!(writes.is_empty());
        worker.key(key.clone())?;
        assert!(matches!(writes.try_recv()?, Write::Text(text) if !text.is_empty()));
        worker.apply(&json!({"type":"snapshot","snapshot":{"history":"\u{1b}[>11u", "status":"running","cols":12,"rows":4}}))?;
        worker.key(KeyboardEvent {
            release: true,
            ..key.clone()
        })?;
        assert!(matches!(writes.try_recv()?, Write::Text(text) if text.contains(":3")));
        worker.key(key.clone())?;
        let _ = writes.try_recv()?;
        worker.input(Input::ForgetKey("KeyA".into()))?;
        worker.key(KeyboardEvent {
            release: true,
            ..key
        })?;
        assert!(writes.is_empty());
        Ok(())
    }

    #[test]
    fn large_inputs_split_on_utf8_boundaries_without_losing_a_suffix() {
        let original = format!("{}界🙂{}", "a".repeat(65535), "日本語".repeat(40000));
        let mut rest = original.as_str();
        let mut packets = Vec::new();
        while !rest.is_empty() {
            let end = packet_end(rest);
            assert!(end > 0 && end <= 65536);
            packets.push(&rest[..end]);
            rest = &rest[end..];
        }
        assert_eq!(packets.concat(), original);
        assert!(packets.len() > 5);
    }

    #[test]
    fn an_unanswered_attach_retries_unchanged_and_observe_never_mutates() {
        let mut target = Target {
            thread_id: "thread".into(),
            terminal_id: "term-1".into(),
            cwd: "/tmp".into(),
            worktree_path: None,
            project_root: "/tmp".into(),
            read_only: false,
            restart_if_not_running: true,
        };
        let grid = Grid::new(80, 24, 8, 18);
        let first = target.attach(grid, false);
        assert_eq!(first, target.attach(grid, false));
        assert!(first.get("resume").is_none());
        let mut resumed = first;
        resumed["resume"] = json!(true);
        assert_eq!(target.attach(grid, true), resumed);
        target.read_only = true;
        assert_eq!(target.attach(grid, false), target.identity());
        assert_eq!(target.attach(grid, true), target.identity());
    }

    #[test]
    fn keyboard_text_fallback_preserves_unicode_without_duplicating_encoded_input() -> Result<()> {
        for (mode, expected) in [
            ("", ["a", "界", "🙂"]),
            (
                "\u{1b}[>11u",
                ["\u{1b}[97u", "\u{1b}[30028u", "\u{1b}[128578u"],
            ),
        ] {
            let (mut worker, writes, _) = worker();
            worker.term().write(mode.as_bytes())?;
            for (key, expected) in ["a", "界", "🙂"].into_iter().zip(expected) {
                worker.input(Input::KeyWithFallback(
                    KeyboardEvent {
                        key: key.into(),
                        code: "Unidentified".into(),
                        ..KeyboardEvent::default()
                    },
                    Some(key.into()),
                ))?;
                assert!(matches!(writes.try_recv()?, Write::Text(text) if text == expected));
                assert!(writes.is_empty());
            }
            worker.input(Input::KeyWithFallback(
                KeyboardEvent {
                    key: "e".into(),
                    code: "KeyE".into(),
                    composing: true,
                    ..KeyboardEvent::default()
                },
                Some("é".into()),
            ))?;
            assert!(writes.is_empty());
        }
        Ok(())
    }

    #[test]
    fn drag_selection_tracks_history_and_extends_complete_words() -> Result<()> {
        let (mut worker, _, _) = worker();
        for row in 0..25 {
            worker
                .term()
                .write(format!("line{row:02}\r\n").as_bytes())?;
        }
        worker.input(Input::Scroll(-10))?;
        let first = worker.term().snapshot()?.row_data[0]
            .text
            .trim_end()
            .to_owned();
        assert_eq!(first, "line12");
        worker.input(Input::BeginSelection(Point { x: 2, y: 0 }, 2))?;
        worker.input(Input::ExtendSelection(Point { x: 2, y: 2 }))?;
        assert_eq!(worker.term().selection_text()?, "line12\nline13\nline14");
        worker.input(Input::ScrollSelection(2, Point { x: 2, y: 2 }))?;
        assert_eq!(
            worker.term().selection_text()?,
            "line12\nline13\nline14\nline15\nline16"
        );
        worker.input(Input::EndSelection)?;
        assert!(worker.term().selection_range()?.is_some());
        worker.input(Input::BeginSelection(Point { x: 2, y: 0 }, 1))?;
        worker.input(Input::EndSelection)?;
        assert!(worker.term().selection_range()?.is_none());
        Ok(())
    }

    #[test]
    fn mouse_motion_is_deduplicated_until_a_button_or_tracking_change() -> Result<()> {
        let (mut worker, writes, _) = worker();
        worker.term().write(b"\x1b[?1003h\x1b[?1006h")?;
        worker.refresh_modes()?;
        let mouse = crate::Mouse {
            action: crate::MouseAction::Motion,
            button: Some(1),
            mods: 0,
            x: 12.,
            y: 24.,
            screen_width: 104.,
            screen_height: 80.,
            cell_width: 8.,
            cell_height: 18.,
            padding_left: 4.,
            padding_right: 4.,
            padding_top: 4.,
            padding_bottom: 4.,
            any_button_pressed: true,
        };
        worker.input(Input::Mouse(mouse.clone()))?;
        assert!(matches!(writes.try_recv()?, Write::Text(text) if text == "\x1b[<32;2;2M"));
        worker.input(Input::Mouse(mouse.clone()))?;
        assert!(writes.is_empty());
        worker.input(Input::Mouse(crate::Mouse {
            action: crate::MouseAction::Press,
            ..mouse.clone()
        }))?;
        assert!(matches!(writes.try_recv()?, Write::Text(text) if text == "\x1b[<0;2;2M"));
        worker.input(Input::Mouse(mouse.clone()))?;
        assert!(matches!(writes.try_recv()?, Write::Text(text) if text == "\x1b[<32;2;2M"));
        worker.term().write(b"\x1b[?1003l")?;
        worker.refresh_modes()?;
        worker.term().write(b"\x1b[?1003h")?;
        worker.refresh_modes()?;
        worker.input(Input::Mouse(mouse))?;
        assert!(matches!(writes.try_recv()?, Write::Text(text) if text == "\x1b[<32;2;2M"));
        Ok(())
    }
}
