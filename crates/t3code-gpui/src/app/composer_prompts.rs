use super::*;
use crate::prompt_store::{self, Entry, SavedAttachment, Store};

#[derive(Default)]
pub(super) struct Prompts {
    pub(super) store: Store,
    pub(super) open: bool,
    pub(super) selected: usize,
    pub(super) history: Option<prompt_store::Position>,
    pub(super) thumbnails: HashMap<String, Vec<Arc<Image>>>,
    pub(super) delete_focus: HashMap<String, FocusHandle>,
}

impl Root {
    pub(super) fn sync_prompt_thumbnails(&mut self, cx: &mut Context<Self>) {
        let prompts = &mut self.conversation.prompts;
        prompts
            .delete_focus
            .retain(|id, _| prompts.store.entries.iter().any(|entry| &entry.id == id));
        prompts
            .thumbnails
            .retain(|id, _| prompts.store.entries.iter().any(|entry| &entry.id == id));
        for entry in &prompts.store.entries {
            prompts
                .delete_focus
                .entry(entry.id.clone())
                .or_insert_with(|| cx.focus_handle());
            prompts
                .thumbnails
                .entry(entry.id.clone())
                .or_insert_with(|| {
                    entry
                        .attachments
                        .iter()
                        .filter_map(|file| {
                            ImageFormat::from_mime_type(&file.mime_type).map(|format| {
                                Arc::new(Image::from_bytes(format, file.bytes.clone()))
                            })
                        })
                        .take(3)
                        .collect()
                });
        }
    }

    pub(super) fn stash_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer_composing(window, cx) {
            return;
        }
        if self.pending_question().is_some() {
            self.conversation.prompts.open = !self.conversation.prompts.open;
            cx.notify();
            return;
        }
        let prompt = self
            .conversation
            .composer
            .read(cx)
            .value()
            .trim()
            .to_string();
        if prompt.is_empty() && self.conversation.attachments.is_empty() {
            if self.conversation.prompts.store.entries.len() == 1 {
                let id = self.conversation.prompts.store.entries[0].id.clone();
                self.restore_prompt(&id, window, cx);
            } else {
                self.conversation.prompts.open = !self.conversation.prompts.open;
                self.conversation.prompts.selected = 0;
                cx.notify();
            }
            return;
        }
        let mut used = 0;
        let mut attachments = vec![];
        let mut dropped = vec![];
        for file in &self.conversation.attachments {
            let chars = file.bytes.len().div_ceil(3) * 4 + file.mime_type.len() + 13;
            if file.mime_type.starts_with("image/") && used + chars > 2_700_000 {
                dropped.push(file.name.clone());
                continue;
            }
            if file.mime_type.starts_with("image/") {
                used += chars;
            }
            attachments.push(SavedAttachment {
                name: file.name.clone(),
                mime_type: file.mime_type.clone(),
                bytes: file.bytes.as_ref().clone(),
            });
        }
        let entry = Entry {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("current time")
                .as_secs(),
            prompt,
            attachments,
            dropped,
        };
        let dropped = entry.dropped.clone();
        match self
            .conversation
            .prompts
            .store
            .push(&Store::path(&self.drafts_path, self.conversation.id), entry)
        {
            Ok(evicted) => {
                self.conversation
                    .composer
                    .update(cx, |state, cx| state.set_value("", window, cx));
                self.conversation.attachments.clear();
                self.conversation.completion = None;
                self.conversation.prompts.history = None;
                self.stash_composer(cx);
                if evicted {
                    self.conversation.message =
                        Some("Oldest stashed prompt discarded. The stash holds 20 prompts.".into());
                }
                if !dropped.is_empty() {
                    self.conversation.message =
                        Some(format!("Images too large to stash: {}", dropped.join(", ")));
                }
            }
            Err(error) => {
                self.conversation.message = Some(format!("Could not save prompt stash: {error:#}"))
            }
        }
        cx.notify();
    }

    fn restore_prompt(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer_composing(window, cx) || self.pending_question().is_some() {
            return;
        }
        match self
            .conversation
            .prompts
            .store
            .take(&Store::path(&self.drafts_path, self.conversation.id), id)
        {
            Ok(Some(entry)) => {
                let current = self.conversation.composer.read(cx).value().to_string();
                let text = if current.trim().is_empty() {
                    entry.prompt
                } else if entry.prompt.is_empty() {
                    current
                } else {
                    format!("{}\n\n{}", current.trim_end(), entry.prompt)
                };
                self.conversation.composer.update(cx, |state, cx| {
                    state.set_value(super::composer_tokens::content(text), window, cx);
                    state.focus(window, cx);
                });
                for file in entry.attachments {
                    match attachments::Pending::new(file.name, file.mime_type, file.bytes) {
                        Ok(file) => self.conversation.attachments.push(file),
                        Err(error) => {
                            self.conversation.message =
                                Some(format!("Could not restore attachment: {error:#}"))
                        }
                    }
                }
                if !entry.dropped.is_empty() {
                    self.conversation.message =
                        Some(format!("Re-attach images: {}", entry.dropped.join(", ")));
                }
                self.conversation.prompts.open = false;
                self.conversation.prompts.history = None;
                self.stash_composer(cx);
            }
            Ok(None) => {}
            Err(error) => {
                self.conversation.message = Some(format!("Could not restore prompt: {error:#}"))
            }
        }
        cx.notify();
    }

    fn delete_stash(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Err(error) = self
            .conversation
            .prompts
            .store
            .take(&Store::path(&self.drafts_path, self.conversation.id), id)
        {
            self.conversation.message = Some(format!("Could not delete stashed prompt: {error:#}"));
        }
        self.conversation.prompts.selected = self.prompts.selected.min(
            self.conversation
                .prompts
                .store
                .entries
                .len()
                .saturating_sub(1),
        );
        cx.notify();
    }

    pub(super) fn prompt_key(
        &mut self,
        key: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.conversation.prompts.open {
            match key.key.as_str() {
                "escape" => self.conversation.prompts.open = false,
                "up" => {
                    let n = self.conversation.prompts.store.entries.len();
                    if n > 0 {
                        self.conversation.prompts.selected =
                            (self.conversation.prompts.selected + n - 1) % n;
                    }
                }
                "down" => {
                    let n = self.conversation.prompts.store.entries.len();
                    if n > 0 {
                        self.conversation.prompts.selected =
                            (self.conversation.prompts.selected + 1) % n;
                    }
                }
                "enter" => {
                    if self
                        .conversation
                        .prompts
                        .delete_focus
                        .values()
                        .any(|focus| focus.is_focused(window))
                    {
                        return false;
                    }
                    if let Some(entry) = self
                        .conversation
                        .prompts
                        .store
                        .entries
                        .get(self.conversation.prompts.selected)
                    {
                        let id = entry.id.clone();
                        self.restore_prompt(&id, window, cx);
                    }
                }
                "backspace" if key.modifiers.control || key.modifiers.platform => {
                    if let Some(entry) = self
                        .conversation
                        .prompts
                        .store
                        .entries
                        .get(self.conversation.prompts.selected)
                    {
                        let id = entry.id.clone();
                        self.delete_stash(&id, cx);
                    }
                }
                _ => return false,
            }
            cx.notify();
            return true;
        }
        if !matches!(key.key.as_str(), "up" | "down")
            || key.modifiers != Modifiers::default()
            || !self.conversation.attachments.is_empty()
            || self.pending_question().is_some()
            || !self
                .conversation
                .composer
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        {
            return false;
        }
        let state = self.conversation.composer.read(cx);
        if !state.selected_range().is_empty() {
            return false;
        }
        let current = state.value().to_string();
        let backward = key.key == "up";
        let boundary = if backward { 0 } else { current.len() };
        let caret = state.range_to_bounds(&(state.cursor()..state.cursor()));
        let edge = state.range_to_bounds(&(boundary..boundary));
        if !caret
            .zip(edge)
            .is_some_and(|(caret, edge)| (caret.origin.y - edge.origin.y).abs() < px(1.))
        {
            return false;
        }
        let Some(target) = self.composer_key() else {
            return false;
        };
        let Some(open) = &self.conversation.open else {
            return false;
        };
        let state = open.watch.state();
        let entries = {
            let state = state.lock().expect("thread");
            let items = state
                .items()
                .into_iter()
                .map(|item| {
                    let mut item = item.clone();
                    if item["textOmitted"] == true
                        && let Some(message) = state
                            .list("messages")
                            .iter()
                            .find(|message| message["id"] == item["messageId"])
                    {
                        item["text"] = message["text"].clone();
                    }
                    item
                })
                .collect::<Vec<_>>();
            prompt_store::history(&items)
        };
        let Some((position, prompt)) = prompt_store::step(
            &entries,
            self.conversation.prompts.history.as_ref(),
            &target,
            &current,
            backward,
        ) else {
            return false;
        };
        self.conversation.prompts.history = position;
        let (line, column) = completion::line_column(&prompt, prompt.len());
        let position = Position::new(line, column);
        self.conversation.composer.update(cx, |state, cx| {
            state.set_value(super::composer_tokens::content(prompt), window, cx);
            state.set_cursor_position(position, window, cx);
        });
        self.stash_composer(cx);
        true
    }

    pub(super) fn render_prompt_stash(
        &self,
        drawer: bool,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        if (drawer && !self.conversation.prompts.open)
            || (!drawer && self.conversation.prompts.store.entries.is_empty())
        {
            return None;
        }
        let open = drawer;
        let count = self.conversation.prompts.store.entries.len();
        Some(
            v_flex()
                .absolute()
                .bottom_full()
                .right_5()
                .when(!open, |view| view.rounded_t_lg().border_1().border_color(cx.theme().border).bg(crate::appearance::composer(cx)))
                .when(open, |view| {
                    view.left_4()
                        .right_4()
                        .px_1()
                        .py_1()
                        .rounded_md()
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(crate::appearance::composer(cx).opacity(1.))
                        .shadow_md()
                })
                .gap_1()
                .child(
                    Button::new("stash-toggle")
                        .child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .text_xs()
                                .child(
                                    Icon::new(gpui_kit::assets::IconName::Bookmark).size(px(12.)),
                                )
                                .child(text!("Stash"))
                                .child(div().flex_1())
                                .child(text!(count.to_string())),
                        )
                        .small()
                        .accessibility_label(if open {
                            "Close stash".to_string()
                        } else {
                            format!("Stashed prompts: {count}. Open stash.")
                        })
                        .ghost()
                        .compact()
                        .on_click(self.pane_listener(cx, |root, _, _, cx| {
                            root.conversation.prompts.open = !root.conversation.prompts.open;
                            cx.notify();
                        })),
                )
                .when(open, |view| {
                    view.child(
                        v_flex()
                            .id("stashed-prompts")
                            .role(accesskit::Role::List)
                            .aria_label("Stashed prompts")
                            .max_h(px(220.))
                            .overflow_y_scroll()
                            .when(count == 0, |list| list.child("Nothing stashed yet."))
                            .children(
                                self.conversation
                                    .prompts
                                    .store
                                    .entries
                                    .iter()
                                    .enumerate()
                                    .map(|(index, entry)| {
                                        let id = entry.id.clone();
                                        let delete = id.clone();
                                        let snippet = stash_snippet(entry);
                                        h_flex()
                                            .gap_1()
                                            .when(
                                                index == self.conversation.prompts.selected,
                                                |row| row.bg(cx.theme().list_active),
                                            )
                                            .child(
                                                Button::new(SharedString::from(format!(
                                                    "restore-{id}"
                                                )))
                                                .child(
                                                    h_flex()
                                                        .w_full()
                                                        .gap_2()
                                                        .text_xs()
                                                        .child(
                                                            Icon::new(IconName::FileText)
                                                                .size(px(12.)),
                                                        )
                                                        .child(text!(id = ("stash-snippet", index), snippet.clone()))
                                                        .child(div().flex_1())
                                                        .children(self.conversation.prompts.thumbnails.get(&entry.id).into_iter().flatten().take(3).map(|image| img(image.clone()).size(px(16.)).object_fit(ObjectFit::Cover).rounded_sm()))
                                                        .child(div().text_color(cx.theme().muted_foreground).child(text!(id = ("stash-time", index), prompt_store::relative_time(entry.created_at, SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).expect("current time").as_secs())))),
                                                )
                                                .small()
                                                .accessibility_label(format!(
                                                    "Restore stashed prompt: {snippet}"
                                                ))
                                                .ghost()
                                                .compact()
                                                .flex_1()
                                                .on_click(self.pane_listener(
                                                    cx,
                                                    move |root, _, window, cx| {
                                                        root.restore_prompt(&id, window, cx)
                                                    },
                                                )),
                                            )
                                            .child(
                                                Button::new(SharedString::from(format!(
                                                    "delete-{delete}"
                                                )))
                                                .icon(gpui_kit::assets::IconName::X)
                                                .track_focus(self.conversation.prompts.delete_focus.get(&entry.id).expect("stash delete focus"))
                                                .size(px(24.))
                                                .accessibility_label("Delete stashed prompt")
                                                .ghost()
                                                .compact()
                                                .on_click(self.pane_listener(
                                                    cx,
                                                    move |root, _, _, cx| {
                                                        root.delete_stash(&delete, cx)
                                                    },
                                                )),
                                            )
                                    }),
                            ),
                    )
                })
                .into_any_element(),
        )
    }
}

fn stash_snippet(entry: &Entry) -> String {
    let text = entry
        .prompt
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if !text.is_empty() {
        let utf16: Vec<_> = text.encode_utf16().collect();
        return if utf16.len() > 90 {
            format!("{}…", String::from_utf16_lossy(&utf16[..90]))
        } else {
            text
        };
    }
    let images = entry
        .attachments
        .iter()
        .filter(|file| file.mime_type.starts_with("image/"))
        .count()
        + entry.dropped.len();
    let files = entry
        .attachments
        .iter()
        .filter(|file| !file.mime_type.starts_with("image/"))
        .count();
    let count = images + files;
    if count == 0 {
        return "(empty)".into();
    }
    let label = if images > 0 && files > 0 {
        "attachment"
    } else if files > 0 {
        "file"
    } else {
        "image"
    };
    format!("({count} {label}{})", if count == 1 { "" } else { "s" })
}
