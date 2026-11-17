use super::*;
use crate::prompt_store::{self, Entry, SavedAttachment, Store};

#[derive(Default)]
pub(super) struct Prompts {
    pub(super) store: Store,
    pub(super) open: bool,
    pub(super) selected: usize,
    pub(super) history: Option<prompt_store::Position>,
}

impl Root {
    pub(super) fn stash_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer_composing(window, cx) {
            return;
        }
        if self.pending_question().is_some() {
            self.prompts.open = !self.prompts.open;
            cx.notify();
            return;
        }
        let prompt = self.composer.read(cx).value().trim().to_string();
        if prompt.is_empty() && self.attachments.is_empty() {
            if self.prompts.store.entries.len() == 1 {
                let id = self.prompts.store.entries[0].id.clone();
                self.restore_prompt(&id, window, cx);
            } else {
                self.prompts.open = !self.prompts.open;
                self.prompts.selected = 0;
                cx.notify();
            }
            return;
        }
        let mut used = 0;
        let mut attachments = vec![];
        let mut dropped = vec![];
        for file in &self.attachments {
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
            prompt,
            attachments,
            dropped,
        };
        let dropped = entry.dropped.clone();
        match self
            .prompts
            .store
            .push(&Store::path(&self.drafts_path), entry)
        {
            Ok(evicted) => {
                self.composer
                    .update(cx, |state, cx| state.set_value("", window, cx));
                self.attachments.clear();
                self.completion = None;
                self.prompts.history = None;
                self.stash_composer(cx);
                if evicted {
                    self.message =
                        Some("Oldest stashed prompt discarded. The stash holds 20 prompts.".into());
                }
                if !dropped.is_empty() {
                    self.message =
                        Some(format!("Images too large to stash: {}", dropped.join(", ")));
                }
            }
            Err(error) => self.message = Some(format!("Could not save prompt stash: {error:#}")),
        }
        cx.notify();
    }

    fn restore_prompt(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer_composing(window, cx) || self.pending_question().is_some() {
            return;
        }
        match self.prompts.store.take(&Store::path(&self.drafts_path), id) {
            Ok(Some(entry)) => {
                let current = self.composer.read(cx).value().to_string();
                let text = if current.trim().is_empty() {
                    entry.prompt
                } else if entry.prompt.is_empty() {
                    current
                } else {
                    format!("{}\n\n{}", current.trim_end(), entry.prompt)
                };
                self.composer.update(cx, |state, cx| {
                    state.set_value(super::composer_tokens::content(text), window, cx);
                    state.focus(window, cx);
                });
                for file in entry.attachments {
                    match attachments::Pending::new(file.name, file.mime_type, file.bytes) {
                        Ok(file) => self.attachments.push(file),
                        Err(error) => {
                            self.message = Some(format!("Could not restore attachment: {error:#}"))
                        }
                    }
                }
                if !entry.dropped.is_empty() {
                    self.message = Some(format!("Re-attach images: {}", entry.dropped.join(", ")));
                }
                self.prompts.open = false;
                self.prompts.history = None;
                self.stash_composer(cx);
            }
            Ok(None) => {}
            Err(error) => self.message = Some(format!("Could not restore prompt: {error:#}")),
        }
        cx.notify();
    }

    fn delete_stash(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Err(error) = self.prompts.store.take(&Store::path(&self.drafts_path), id) {
            self.message = Some(format!("Could not delete stashed prompt: {error:#}"));
        }
        self.prompts.selected = self
            .prompts
            .selected
            .min(self.prompts.store.entries.len().saturating_sub(1));
        cx.notify();
    }

    pub(super) fn prompt_key(
        &mut self,
        key: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.prompts.open {
            match key.key.as_str() {
                "escape" => self.prompts.open = false,
                "up" => {
                    let n = self.prompts.store.entries.len();
                    if n > 0 {
                        self.prompts.selected = (self.prompts.selected + n - 1) % n;
                    }
                }
                "down" => {
                    let n = self.prompts.store.entries.len();
                    if n > 0 {
                        self.prompts.selected = (self.prompts.selected + 1) % n;
                    }
                }
                "enter" => {
                    if let Some(entry) = self.prompts.store.entries.get(self.prompts.selected) {
                        let id = entry.id.clone();
                        self.restore_prompt(&id, window, cx);
                    }
                }
                "backspace" if key.modifiers.control || key.modifiers.platform => {
                    if let Some(entry) = self.prompts.store.entries.get(self.prompts.selected) {
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
            || !self.attachments.is_empty()
            || self.pending_question().is_some()
            || !self.composer.read(cx).focus_handle(cx).is_focused(window)
        {
            return false;
        }
        let state = self.composer.read(cx);
        if !state.selected_range().is_empty() {
            return false;
        }
        let current = state.value().to_string();
        let backward = key.key == "up";
        if (backward && current[..state.cursor()].contains('\n'))
            || (!backward && current[state.cursor()..].contains('\n'))
        {
            return false;
        }
        let Some(target) = self.composer_key() else {
            return false;
        };
        let Some(open) = &self.open else {
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
            self.prompts.history.as_ref(),
            &target,
            &current,
            backward,
        ) else {
            return false;
        };
        self.prompts.history = position;
        let (line, column) = completion::line_column(&prompt, prompt.len());
        let position = Position::new(line, column);
        self.composer.update(cx, |state, cx| {
            state.set_value(super::composer_tokens::content(prompt), window, cx);
            state.set_cursor_position(position, window, cx);
        });
        self.stash_composer(cx);
        true
    }

    pub(super) fn render_prompt_stash(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.prompts.open && self.prompts.store.entries.is_empty() {
            return None;
        }
        let open = self.prompts.open;
        let count = self.prompts.store.entries.len();
        Some(
            v_flex()
                .px_3()
                .pt_2()
                .gap_1()
                .child(
                    Button::new("stash-toggle")
                        .label(format!("Stash {count}"))
                        .accessibility_label(if open { "Close stash" } else { "Open stash" })
                        .ghost()
                        .compact()
                        .on_click(cx.listener(|root, _, _, cx| {
                            root.prompts.open = !root.prompts.open;
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
                            .children(self.prompts.store.entries.iter().enumerate().map(
                                |(index, entry)| {
                                    let id = entry.id.clone();
                                    let delete = id.clone();
                                    let snippet =
                                        entry.prompt.chars().take(100).collect::<String>();
                                    h_flex()
                                        .gap_1()
                                        .when(index == self.prompts.selected, |row| {
                                            row.bg(cx.theme().list_active)
                                        })
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "restore-{id}"
                                            )))
                                            .label(snippet.clone())
                                            .accessibility_label(format!(
                                                "Restore stashed prompt: {snippet}"
                                            ))
                                            .ghost()
                                            .compact()
                                            .flex_1()
                                            .on_click(cx.listener(move |root, _, window, cx| {
                                                root.restore_prompt(&id, window, cx)
                                            })),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "delete-{delete}"
                                            )))
                                            .label("Delete")
                                            .accessibility_label("Delete stashed prompt")
                                            .ghost()
                                            .compact()
                                            .on_click(cx.listener(move |root, _, _, cx| {
                                                root.delete_stash(&delete, cx)
                                            })),
                                        )
                                },
                            )),
                    )
                })
                .into_any_element(),
        )
    }
}
