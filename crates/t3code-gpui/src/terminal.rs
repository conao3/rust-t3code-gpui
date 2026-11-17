use crate::terminal_view::{TerminalView, ViewEvent};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use t3_client::session::Connection;
use t3_terminal::session::Target;
use tokio::runtime::Handle;

pub fn next_id(ids: impl IntoIterator<Item = String>, reusable: bool) -> String {
    let ids: HashSet<_> = ids
        .into_iter()
        .filter_map(|id| terminal_number(&id).map(str::to_string))
        .collect();
    let index = (1..)
        .find(|index| !ids.contains(&index.to_string()))
        .expect("terminal ID available");
    if reusable {
        format!("term-{index}")
    } else {
        format!("term-{index}-{}", uuid::Uuid::new_v4())
    }
}

pub fn label(id: &str, metadata: &str) -> String {
    if !metadata.trim().is_empty() {
        return metadata.trim().to_string();
    }
    let number = terminal_number(id);
    number
        .map(|number| format!("Terminal {number}"))
        .unwrap_or_else(|| id.to_string())
}

fn terminal_number(id: &str) -> Option<&str> {
    let rest = if id
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("term-"))
    {
        &id[5..]
    } else if id
        .get(..9)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("terminal-"))
    {
        &id[9..]
    } else {
        return None;
    };
    let (number, suffix) = rest
        .split_once('-')
        .map_or((rest, None), |(number, suffix)| (number, Some(suffix)));
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if let Some(suffix) = suffix
        && (suffix.len() != 36 || uuid::Uuid::parse_str(suffix).is_err())
    {
        return None;
    }
    Some(number)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub id: String,
    pub terminals: Vec<String>,
    pub vertical: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub open: bool,
    pub height: f32,
    pub terminal_ids: Vec<String>,
    pub groups: Vec<Group>,
    pub active: String,
    pub active_group: String,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            open: false,
            height: 280.,
            terminal_ids: Vec::new(),
            groups: Vec::new(),
            active: String::new(),
            active_group: String::new(),
        }
    }
}

impl Layout {
    pub fn from_value(value: &Value) -> Self {
        let mut layout = Self {
            open: value["terminalOpen"].as_bool().unwrap_or(false),
            height: value["terminalHeight"]
                .as_f64()
                .filter(|height| height.is_finite() && *height > 0.)
                .unwrap_or(280.) as f32,
            ..Self::default()
        };
        let mut unique = HashSet::new();
        let ids: Vec<String> = value["terminalIds"]
            .as_array()
            .map(|ids| {
                ids.iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|id| !id.is_empty() && unique.insert(id.to_string()))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        layout.terminal_ids = ids.clone();
        let allowed: HashSet<String> = ids.iter().cloned().collect();
        let mut assigned = HashSet::new();
        if let Some(groups) = value["terminalGroups"].as_array() {
            for group in groups {
                let terminals: Vec<_> = group["terminalIds"]
                    .as_array()
                    .map(|ids| {
                        ids.iter()
                            .filter_map(Value::as_str)
                            .map(str::trim)
                            .filter(|id| allowed.contains(*id) && assigned.insert(id.to_string()))
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                if terminals.is_empty() {
                    continue;
                }
                let id = group["id"]
                    .as_str()
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("group-{}", terminals[0]));
                let id = layout.unique_group_id(&id);
                layout.groups.push(Group {
                    id,
                    terminals,
                    vertical: group["splitDirection"].as_str() == Some("vertical"),
                });
            }
        }
        for terminal in ids {
            if assigned.insert(terminal.clone()) {
                let id = layout.unique_group_id(&format!("group-{terminal}"));
                layout.groups.push(Group {
                    id,
                    terminals: vec![terminal],
                    vertical: false,
                });
            }
        }
        let active = value["activeTerminalId"].as_str().unwrap_or_default();
        if !layout.select(active)
            && let Some(first) = layout.terminal_ids.first().cloned()
        {
            layout.select(&first);
        }
        if let Some(group) = value["activeTerminalGroupId"]
            .as_str()
            .filter(|id| layout.groups.iter().any(|group| group.id == *id))
        {
            layout.active_group = group.to_string();
        }
        layout
    }

    fn unique_group_id(&self, base: &str) -> String {
        if !self.groups.iter().any(|group| group.id == base) {
            return base.to_string();
        }
        (2..)
            .map(|index| format!("{base}-{index}"))
            .find(|id| !self.groups.iter().any(|group| &group.id == id))
            .expect("terminal group ID available")
    }

    pub fn ids(&self) -> Vec<String> {
        self.terminal_ids.clone()
    }

    pub fn value(&self) -> Value {
        json!({"terminalOpen":self.open,"terminalHeight":self.height,"terminalIds":self.ids(),"activeTerminalId":self.active,"activeTerminalGroupId":self.active_group,"terminalGroups":self.groups.iter().map(|group|json!({"id":group.id,"terminalIds":group.terminals,"splitDirection":if group.vertical {"vertical"} else {"horizontal"}})).collect::<Vec<_>>()})
    }

    pub fn select(&mut self, terminal: &str) -> bool {
        let Some(group) = self
            .groups
            .iter()
            .find(|group| group.terminals.iter().any(|id| id == terminal))
        else {
            return false;
        };
        self.active = terminal.to_string();
        self.active_group = group.id.clone();
        true
    }

    pub fn add(&mut self, terminal: String, split: Option<bool>) -> bool {
        if self.ids().contains(&terminal) {
            return false;
        }
        if let Some(vertical) = split.filter(|_| !self.groups.is_empty()) {
            let Some(group) = self
                .groups
                .iter_mut()
                .find(|group| group.id == self.active_group)
            else {
                return false;
            };
            if group.terminals.len() >= 4 {
                return false;
            }
            group.vertical = vertical;
            let position = group
                .terminals
                .iter()
                .position(|id| id == &self.active)
                .map_or(group.terminals.len(), |index| index + 1);
            group.terminals.insert(position, terminal.clone());
        } else {
            let id = self.unique_group_id(&format!("group-{terminal}"));
            self.groups.push(Group {
                id,
                terminals: vec![terminal.clone()],
                vertical: false,
            });
        }
        self.terminal_ids.push(terminal.clone());
        self.select(&terminal);
        self.open = true;
        true
    }

    pub fn remove(&mut self, terminal: &str) {
        let Some(group_index) = self
            .groups
            .iter()
            .position(|group| group.terminals.iter().any(|id| id == terminal))
        else {
            return;
        };
        let global_index = self
            .terminal_ids
            .iter()
            .position(|id| id == terminal)
            .expect("terminal has an ID");
        self.terminal_ids.retain(|id| id != terminal);
        let group = &mut self.groups[group_index];
        let terminal_index = group
            .terminals
            .iter()
            .position(|id| id == terminal)
            .expect("terminal belongs to group");
        group.terminals.remove(terminal_index);
        let adjacent = group
            .terminals
            .get(terminal_index.min(group.terminals.len().saturating_sub(1)))
            .cloned();
        self.groups.retain(|group| !group.terminals.is_empty());
        if self.groups.is_empty() {
            self.open = false;
            self.active.clear();
            self.active_group.clear();
        } else if self.active == terminal {
            let next = adjacent
                .or_else(|| {
                    self.terminal_ids
                        .get(global_index.min(self.terminal_ids.len() - 1))
                        .cloned()
                })
                .expect("nonempty terminal group");
            self.select(&next);
        }
    }
}

#[derive(Clone)]
pub struct Store {
    pub path: PathBuf,
    pub error: Option<String>,
    values: serde_json::Map<String, Value>,
}

impl Store {
    pub fn load(path: PathBuf) -> Self {
        let mut store = Self {
            path,
            error: None,
            values: serde_json::Map::new(),
        };
        match std::fs::read(&store.path) {
            Ok(data) => match serde_json::from_slice(&data) {
                Ok(values) => store.values = values,
                Err(error) => store.error = Some(format!("Cannot load terminal layout: {error}")),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => store.error = Some(format!("Cannot load terminal layout: {error}")),
        }
        store
    }
    pub fn get(&self, key: &str) -> Layout {
        Layout::from_value(self.values.get(key).unwrap_or(&Value::Null))
    }
    pub fn set(&mut self, key: String, layout: &Layout) {
        self.values.insert(key, layout.value());
    }
    pub fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let partial = self.path.with_extension("json.partial");
        std::fs::write(&partial, serde_json::to_vec_pretty(&self.values)?)?;
        std::fs::rename(partial, &self.path)?;
        Ok(())
    }
}

#[derive(Clone)]
pub enum DrawerEvent {
    Changed(Layout),
    Focus,
}

pub struct Drawer {
    pub layout: Layout,
    views: HashMap<String, Entity<TerminalView>>,
    subscriptions: HashMap<String, Subscription>,
    target: Target,
    connection: Arc<Connection>,
    runtime: Handle,
    visible: bool,
    light: bool,
    font_size: f32,
    shortcuts: [Option<String>; 4],
    resizing: Option<(f32, f32)>,
    closing: bool,
    error: Option<String>,
}

impl EventEmitter<DrawerEvent> for Drawer {}

#[derive(Clone, PartialEq)]
pub struct DrawerConfig {
    pub light: bool,
    pub font_size: f32,
    pub shortcuts: [Option<String>; 4],
}

impl Drawer {
    pub fn new(
        layout: Layout,
        target: Target,
        connection: Arc<Connection>,
        runtime: Handle,
        configuration: DrawerConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut drawer = Self {
            layout,
            views: HashMap::new(),
            subscriptions: HashMap::new(),
            target,
            connection,
            runtime,
            visible: true,
            light: configuration.light,
            font_size: configuration.font_size,
            shortcuts: configuration.shortcuts,
            resizing: None,
            closing: false,
            error: None,
        };
        drawer.reconcile(window, cx);
        drawer
    }

    fn reconcile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self
            .layout
            .groups
            .iter()
            .find(|group| group.id == self.layout.active_group)
            .map(|group| group.terminals.clone())
            .unwrap_or_default();
        self.views.retain(|id, _| ids.contains(id));
        self.subscriptions.retain(|id, _| ids.contains(id));
        for id in ids {
            if self.views.contains_key(&id) {
                continue;
            }
            let mut target = self.target.clone();
            target.terminal_id = id.clone();
            let view = cx.new(|cx| {
                TerminalView::new(
                    self.connection.clone(),
                    target,
                    self.light,
                    self.font_size,
                    &self.runtime,
                    window,
                    cx,
                )
            });
            let owned = id.clone();
            let subscription = cx.subscribe(&view, move |drawer, _, event, cx| match event {
                ViewEvent::Focus => {
                    if drawer.layout.active != owned {
                        drawer.layout.select(&owned);
                        drawer.changed(cx);
                    }
                    cx.emit(DrawerEvent::Focus);
                }
                ViewEvent::Metadata => cx.notify(),
            });
            self.subscriptions.insert(id.clone(), subscription);
            self.views.insert(id, view);
        }
    }

    fn changed(&self, cx: &mut Context<Self>) {
        cx.emit(DrawerEvent::Changed(self.layout.clone()));
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.views.get(&self.layout.active) {
            let focus = view.read(cx).focus.clone();
            window.focus(&focus, cx);
        }
    }

    pub fn is_focused(&self, window: &Window, cx: &App) -> bool {
        self.visible
            && self
                .views
                .values()
                .any(|view| view.read(cx).focus.is_focused(window))
    }

    pub fn configure(
        &mut self,
        visible: bool,
        configuration: DrawerConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let light = configuration.light;
        let font_size = configuration.font_size;
        let changed = self.visible != visible
            || self.light != light
            || self.font_size != font_size
            || self.shortcuts != configuration.shortcuts;
        self.visible = visible;
        self.light = light;
        self.font_size = font_size;
        self.shortcuts = configuration.shortcuts;
        if changed {
            for view in self.views.values() {
                view.update(cx, |view, cx| {
                    view.set_visible(visible, cx);
                    view.configure(light, font_size, window, cx);
                });
            }
            if visible {
                cx.notify();
            }
        }
    }

    pub fn add(&mut self, split: Option<bool>, window: &mut Window, cx: &mut Context<Self>) {
        if self.target.read_only {
            return;
        }
        let id = next_id(self.layout.ids(), true);
        if self.layout.add(id, split) {
            self.reconcile(window, cx);
            self.focus(window, cx);
            self.changed(cx);
        }
    }

    pub fn select(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.layout.select(id) {
            self.reconcile(window, cx);
            self.focus(window, cx);
            self.changed(cx);
        }
    }

    pub fn close_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close(self.layout.active.clone(), window, cx);
    }

    fn close(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.target.read_only || self.closing || !self.layout.terminal_ids.contains(&id) {
            return;
        }
        let terminal_label = self
            .views
            .get(&id)
            .map(|view| label(&id, &view.read(cx).state.label))
            .unwrap_or_else(|| label(&id, ""));
        let prompt = window.prompt(
            PromptLevel::Warning,
            &format!("Close terminal \"{terminal_label}\"?"),
            Some("This stops the running process and clears its history."),
            &["Cancel", "Close"],
            cx,
        );
        self.closing = true;
        let runtime = self.runtime.clone();
        let connection = self.connection.clone();
        let mut target = self.target.clone();
        target.terminal_id = id.clone();
        cx.spawn_in(window, async move |this, cx| {
            if prompt.await != Ok(1) {
                let _ = this.update(cx, |drawer, _| drawer.closing = false);
                return;
            }
            let result = runtime
                .spawn(async move { t3_terminal::session::close(&connection, &target).await })
                .await;
            let _ = this.update(cx, |drawer, cx| {
                drawer.closing = false;
                match result {
                    Ok(Ok(())) => {
                        drawer.layout.remove(&id);
                        drawer.views.remove(&id);
                        drawer.subscriptions.remove(&id);
                        drawer.changed(cx);
                    }
                    Ok(Err(error)) => {
                        drawer.error = Some(format!("{error:#}"));
                        cx.notify();
                    }
                    Err(error) => {
                        drawer.error = Some(error.to_string());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    pub fn drag_resize(&mut self, position: f32, window_height: f32, cx: &mut Context<Self>) {
        if let Some((start, height)) = self.resizing {
            self.layout.height = (height + start - position)
                .round()
                .clamp(180., (window_height * 0.75).floor().max(180.));
            self.changed(cx);
        }
    }
    pub fn end_resize(&mut self, cx: &mut Context<Self>) {
        if self.resizing.take().is_some() {
            self.changed(cx);
        }
    }
}

impl Render for Drawer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.reconcile(window, cx);
        let active = self
            .layout
            .groups
            .iter()
            .find(|group| group.id == self.layout.active_group)
            .cloned();
        let sidebar = self.layout.ids().len() > 1;
        let mut panes = if active.as_ref().is_some_and(|group| group.vertical) {
            v_flex()
        } else {
            h_flex()
        };
        if let Some(group) = &active {
            for (index, id) in group.terminals.iter().enumerate() {
                if let Some(view) = self.views.get(id) {
                    panes =
                        panes.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .min_h_0()
                                .h_full()
                                .when(index > 0, |pane| {
                                    pane.when(group.vertical, |pane| pane.border_t_1())
                                        .when(!group.vertical, |pane| pane.border_l_1())
                                        .border_color(cx.theme().border.opacity(
                                            if id == &self.layout.active { 1. } else { 0.7 },
                                        ))
                                })
                                .child(view.clone().cached(StyleRefinement::default().size_full())),
                        );
                }
            }
        }
        let disabled = self.target.read_only;
        let limit = active
            .as_ref()
            .is_some_and(|group| group.terminals.len() >= 4);
        let mut labels = [
            "Split Terminal Horizontally",
            "Split Terminal Vertically",
            "New Terminal",
            "Close Terminal",
        ]
        .map(str::to_owned);
        for (index, label) in labels.iter_mut().enumerate() {
            if limit && index < 2 {
                label.push_str(" (max 4 per group)");
            } else if let Some(shortcut) = &self.shortcuts[index] {
                label.push_str(&format!(" ({shortcut})"));
            }
        }
        let actions = h_flex().h(px(22.)).children([
            Button::new("terminal-split-horizontal")
                .icon(Icon::new(IconName::SquareSplitHorizontal).size(px(13.)))
                .tooltip(labels[0].clone())
                .accessibility_label(labels[0].clone())
                .ghost()
                .p_0()
                .rounded(px(0.))
                .w(px(21.))
                .h(px(21.))
                .disabled(disabled)
                .on_click(cx.listener(|drawer, _, window, cx| drawer.add(Some(false), window, cx))),
            Button::new("terminal-split-vertical")
                .icon(Icon::new(IconName::SquareSplitVertical).size(px(13.)))
                .tooltip(labels[1].clone())
                .accessibility_label(labels[1].clone())
                .ghost()
                .p_0()
                .rounded(px(0.))
                .w(px(21.))
                .h(px(21.))
                .border_l_1()
                .border_color(cx.theme().border.opacity(0.8))
                .disabled(disabled)
                .on_click(cx.listener(|drawer, _, window, cx| drawer.add(Some(true), window, cx))),
            Button::new("terminal-new")
                .icon(Icon::new(IconName::Plus).size(px(13.)))
                .tooltip(labels[2].clone())
                .accessibility_label(labels[2].clone())
                .ghost()
                .p_0()
                .rounded(px(0.))
                .w(px(21.))
                .h(px(21.))
                .border_l_1()
                .border_color(cx.theme().border.opacity(0.8))
                .disabled(disabled)
                .on_click(cx.listener(|drawer, _, window, cx| drawer.add(None, window, cx))),
            Button::new("terminal-close")
                .icon(Icon::new(IconName::Trash).size(px(13.)))
                .tooltip(labels[3].clone())
                .accessibility_label(labels[3].clone())
                .ghost()
                .p_0()
                .rounded(px(0.))
                .w(px(21.))
                .h(px(21.))
                .border_l_1()
                .border_color(cx.theme().border.opacity(0.8))
                .disabled(disabled)
                .on_click(cx.listener(|drawer, _, window, cx| drawer.close_active(window, cx))),
        ]);
        let mut body = h_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .when(sidebar, |body| body.gap(px(6.)))
            .child(panes.flex_1().min_w_0().h_full());
        if sidebar {
            let mut list = v_flex()
                .w(px(144.))
                .h_full()
                .flex_none()
                .border_1()
                .border_color(cx.theme().border.opacity(0.7))
                .bg(cx.theme().muted.opacity(0.1))
                .child(
                    h_flex()
                        .h(px(22.))
                        .justify_end()
                        .border_b_1()
                        .border_color(cx.theme().border.opacity(0.7))
                        .child(actions),
                );
            let mut groups = v_flex()
                .id("terminal-groups")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p(px(4.));
            for group in &self.layout.groups {
                let group_active = group.terminals.contains(&self.layout.active);
                let target = if group_active {
                    self.layout.active.clone()
                } else {
                    group.terminals[0].clone()
                };
                let count = group.terminals.len();
                let (title, icon) = if count == 1 {
                    ("Single", IconName::Square)
                } else if group.vertical {
                    ("Stacked", IconName::SquareSplitVertical)
                } else {
                    ("Side by side", IconName::SquareSplitHorizontal)
                };
                let header =
                    Button::new(SharedString::from(format!("terminal-group-{}", group.id)))
                        .ghost()
                        .w_full()
                        .h(px(22.))
                        .p_0()
                        .px(px(6.))
                        .rounded(px(4.))
                        .accessibility_label(title)
                        .when(group_active, |button| {
                            button.bg(cx.theme().accent.opacity(0.5))
                        })
                        .child(
                            h_flex()
                                .w_full()
                                .gap(px(4.))
                                .text_size(px(11.))
                                .child(Icon::new(icon).size(px(12.)))
                                .child(div().flex_1().text_left().child(title))
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(cx.theme().muted_foreground.opacity(0.7))
                                        .child(count.to_string()),
                                ),
                        )
                        .on_click(cx.listener(move |drawer, _, window, cx| {
                            drawer.select(&target, window, cx)
                        }));
                let mut section = v_flex().pb(px(2.)).child(header);
                let mut rows = v_flex().gap(px(2.));
                for id in &group.terminals {
                    let selected = id == &self.layout.active;
                    let id = id.clone();
                    let click_id = id.clone();
                    let text = self
                        .views
                        .get(&id)
                        .map(|view| label(&id, &view.read(cx).state.label))
                        .unwrap_or_else(|| label(&id, ""));
                    let close_id = id.clone();
                    let close_label = format!(
                        "Close {text}{}",
                        if selected {
                            self.shortcuts[3]
                                .as_ref()
                                .map(|shortcut| format!(" ({shortcut})"))
                                .unwrap_or_default()
                        } else {
                            String::new()
                        }
                    );
                    let close = Button::new(SharedString::from(format!("terminal-tab-close-{id}")))
                        .ghost()
                        .p_0()
                        .size(px(12.))
                        .rounded(px(2.))
                        .accessibility_label(close_label.clone())
                        .tooltip(close_label)
                        .disabled(disabled)
                        .child(
                            div()
                                .relative()
                                .size(px(12.))
                                .child(
                                    div()
                                        .size(px(12.))
                                        .group_hover("terminal-tab", |icon| icon.opacity(0.))
                                        .child(Icon::new(IconName::SquareTerminal).size(px(12.))),
                                )
                                .child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .opacity(0.)
                                        .group_hover("terminal-tab", |icon| icon.opacity(1.))
                                        .child(Icon::new(IconName::X).size(px(12.))),
                                ),
                        )
                        .on_click(cx.listener(move |drawer, _, window, cx| {
                            drawer.close(close_id.clone(), window, cx)
                        }));
                    rows = rows.child(
                        h_flex()
                            .group("terminal-tab")
                            .h(px(24.))
                            .w_full()
                            .gap(px(2.))
                            .pl(px(6.))
                            .pr(px(8.))
                            .rounded(px(6.))
                            .text_color(if selected {
                                cx.theme().foreground
                            } else {
                                cx.theme().muted_foreground
                            })
                            .when(selected, |row| row.bg(cx.theme().accent))
                            .when(!selected, |row| {
                                row.hover(|row| {
                                    row.bg(cx.theme().accent.opacity(0.6))
                                        .text_color(cx.theme().foreground)
                                })
                            })
                            .child(close)
                            .child(
                                Button::new(SharedString::from(id))
                                    .accessibility_label(text.clone())
                                    .child(
                                        div()
                                            .w_full()
                                            .text_left()
                                            .text_size(px(12.))
                                            .truncate()
                                            .child(text),
                                    )
                                    .ghost()
                                    .flex_1()
                                    .min_w_0()
                                    .h(px(24.))
                                    .p_0()
                                    .text_size(px(12.))
                                    .justify_start()
                                    .on_click(cx.listener(move |drawer, _, window, cx| {
                                        drawer.select(&click_id, window, cx)
                                    })),
                            ),
                    );
                }
                section = section.child(rows);
                groups = groups.child(section);
            }
            list = list.child(groups);
            body = body.child(list);
        } else {
            body = body.child(
                div()
                    .absolute()
                    .top(px(8.))
                    .right(px(8.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(cx.theme().border.opacity(0.8))
                    .bg(cx.theme().background)
                    .child(actions),
            );
        }
        let height = self.layout.height.clamp(
            180.,
            (f32::from(window.viewport_size().height) * 0.75)
                .floor()
                .max(180.),
        );
        v_flex()
            .relative()
            .w_full()
            .h(px(height))
            .flex_none()
            .border_t_1()
            .border_color(cx.theme().border.opacity(0.8))
            .bg(cx.theme().background)
            .child(body)
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(6.))
                    .cursor_row_resize()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|drawer, event: &MouseDownEvent, _, _| {
                            drawer.resizing =
                                Some((f32::from(event.position.y), drawer.layout.height))
                        }),
                    ),
            )
            .children(
                self.error
                    .as_ref()
                    .map(|error| div().text_xs().child(error.clone())),
            )
    }
}
