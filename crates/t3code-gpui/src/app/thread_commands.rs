use super::*;

pub(super) struct DiscardedDraft {
    pane: u64,
    key: String,
    text: String,
    attachments: Vec<attachments::Pending>,
}

pub(super) struct UndoEntry {
    pub environment: String,
    pub thread: String,
    pub kind: &'static str,
    pub commands: Vec<Value>,
    pub drafts: Vec<DiscardedDraft>,
}

pub(super) fn undo_plan(
    op: &ThreadOp,
    thread: &Value,
    thread_id: &str,
) -> Option<(&'static str, Vec<Value>)> {
    let mut commands = vec![];
    let kind = match op {
        ThreadOp::Settle => {
            commands.push(json!({"type":"thread.unsettle","threadId":thread_id,"reason":"user"}));
            "Settled"
        }
        ThreadOp::Snooze(_) | ThreadOp::SnoozeUntil(_) => {
            return Some((
                "Snoozed",
                vec![json!({"type":"thread.unsnooze","threadId":thread_id,"reason":"user"})],
            ));
        }
        ThreadOp::Unpin => "Unpinned",
        ThreadOp::Archive => {
            return Some((
                "Archived",
                vec![json!({"type":"thread.unarchive","threadId":thread_id})],
            ));
        }
        _ => return None,
    };
    if matches!(op, ThreadOp::Unpin) || thread["pinnedAt"].as_str().is_some() {
        let mut pin = json!({"type":"thread.pin","threadId":thread_id});
        if let Some(order) = thread["pinOrderKey"].as_str() {
            pin["orderKey"] = json!(order);
        }
        commands.push(pin);
    }
    if let Some(until) = thread["snoozedUntil"].as_str() {
        commands.push(json!({"type":"thread.snooze","threadId":thread_id,"snoozedUntil":until}));
    }
    Some((kind, commands))
}

fn hosted_segment(id: &str) -> String {
    id.bytes()
        .map(|byte| match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' => (byte as char).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn thread_context_label(title: &str) -> String {
    let normalized = title
        .replace(['[', ']', '\\', '\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut units = 0;
    let label: String = normalized
        .chars()
        .take_while(|ch| {
            units += ch.len_utf16();
            units <= 200
        })
        .collect();
    if label.is_empty() {
        "thread".into()
    } else {
        label
    }
}

fn current_pull_request(links: &[Value]) -> Option<&Value> {
    let visible: Vec<_> = links
        .iter()
        .filter(|link| link["source"] != "stack-dismissed")
        .collect();
    let is_open = |index: usize| {
        visible[index]["snapshot"].is_null() || visible[index]["snapshot"]["state"] == "open"
    };
    let open: Vec<_> = (0..visible.len()).filter(|index| is_open(*index)).collect();
    if open.len() == 1 {
        return Some(visible[open[0]]);
    }
    let repository = |index: usize| {
        let link = visible[index];
        format!(
            "{}/{}",
            link["authority"]
                .as_str()
                .or(link["host"].as_str())
                .unwrap_or_default()
                .trim()
                .to_lowercase(),
            link["repository"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_lowercase()
        )
    };
    let mut native: Vec<(String, Vec<usize>)> = vec![];
    for (index, link) in visible.iter().enumerate() {
        if link["stack"].is_null() {
            continue;
        }
        let key = format!("{}#{}", repository(index), link["stack"]["id"]);
        if let Some((_, members)) = native.iter_mut().find(|(old, _)| *old == key) {
            members.push(index);
        } else {
            native.push((key, vec![index]));
        }
    }
    let mut chains: Vec<Vec<(usize, usize)>> = vec![];
    let mut placed: HashSet<usize> = HashSet::new();
    for (_, mut members) in native {
        let mut layers: Vec<Value> = vec![];
        let mut best = (0, 0);
        for &index in &members {
            let candidate = visible[index]["stack"]["layers"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let score = (
                candidate
                    .iter()
                    .filter(|layer| {
                        members
                            .iter()
                            .any(|index| visible[*index]["number"] == layer["number"])
                    })
                    .count(),
                candidate.len(),
            );
            if score > best {
                layers = candidate;
                best = score;
            }
        }
        members.sort_by_key(|index| {
            layers
                .iter()
                .position(|layer| layer["number"] == visible[*index]["number"])
                .unwrap_or(layers.len())
        });
        placed.extend(&members);
        chains.push(
            members
                .into_iter()
                .enumerate()
                .map(|(depth, index)| (index, depth))
                .collect(),
        );
    }
    let remaining: Vec<_> = (0..visible.len())
        .filter(|index| !placed.contains(index))
        .collect();
    let parent = |index: usize| {
        let snapshot = &visible[index]["snapshot"];
        if snapshot.is_null() {
            return None;
        }
        let matches: Vec<_> = remaining
            .iter()
            .copied()
            .filter(|other| {
                repository(*other) == repository(index)
                    && !visible[*other]["snapshot"].is_null()
                    && visible[*other]["snapshot"]["headBranch"] == snapshot["baseBranch"]
            })
            .collect();
        (matches.len() == 1 && matches[0] != index).then(|| matches[0])
    };
    for &base in &remaining {
        if parent(base).is_some() {
            continue;
        }
        let mut chain = vec![];
        let mut pending = vec![(base, 0)];
        while let Some((index, depth)) = pending.pop() {
            if !placed.insert(index) {
                continue;
            }
            chain.push((index, depth));
            pending.extend(
                remaining
                    .iter()
                    .rev()
                    .filter(|other| parent(**other) == Some(index))
                    .map(|other| (*other, depth + 1)),
            );
        }
        if !chain.is_empty() {
            chains.push(chain);
        }
    }
    for index in remaining {
        if !placed.contains(&index) {
            chains.push(vec![(index, 0)]);
        }
    }
    for chain in &mut chains {
        chain.reverse();
        chain.sort_by_key(|(_, depth)| std::cmp::Reverse(*depth));
    }
    let timestamp = |value: &Value| {
        value
            .as_str()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map_or(0, |date| date.timestamp_millis())
    };
    if !open.is_empty() {
        let mut open_chains: Vec<_> = chains
            .into_iter()
            .map(|chain| {
                chain
                    .into_iter()
                    .filter(|(index, _)| is_open(*index))
                    .collect::<Vec<_>>()
            })
            .filter(|chain| !chain.is_empty())
            .collect();
        open_chains.sort_by_key(|chain| {
            std::cmp::Reverse(
                chain
                    .iter()
                    .map(|(index, _)| timestamp(&visible[*index]["linkedAt"]))
                    .max()
                    .unwrap_or(0),
            )
        });
        return open_chains
            .first()
            .and_then(|chain| chain.first())
            .map(|(index, _)| visible[*index]);
    }
    if chains.len() == 1 {
        return chains[0].first().map(|(index, _)| visible[*index]);
    }
    visible
        .into_iter()
        .enumerate()
        .max_by_key(|(index, link)| {
            (
                timestamp(
                    link.pointer("/snapshot/updatedAt")
                        .filter(|value| !value.is_null())
                        .unwrap_or(&link["linkedAt"]),
                ),
                std::cmp::Reverse(*index),
            )
        })
        .map(|(_, link)| link)
}

impl Root {
    fn panel_pull_request_url(&self) -> Option<Option<String>> {
        if !self.right_panel.open {
            return None;
        }
        let id = self
            .right_panel
            .active
            .as_ref()
            .filter(|id| id.starts_with("pull-request:"))?;
        Some(
            self.pr_state
                .views
                .get(id)
                .and_then(|detail| detail.value["url"].as_str().map(str::to_owned)),
        )
    }

    pub(super) fn has_thread_draft(&self, environment: &str, thread: &str) -> bool {
        let suffix = drafts::thread_key(environment, thread);
        std::iter::once(&self.conversation)
            .chain(self.panes.values())
            .any(|pane| {
                let key = format!("pane:{}:{suffix}", pane.id);
                self.drafts.has(&key)
                    || pane
                        .composer_attachments
                        .get(&key)
                        .is_some_and(|items| !items.is_empty())
            })
    }

    pub(super) fn discard_thread_draft(
        &mut self,
        environment: String,
        thread: String,
        cx: &mut Context<Self>,
    ) {
        let suffix = drafts::thread_key(&environment, &thread);
        let mut discarded = Vec::new();
        for pane in std::iter::once(&mut self.conversation).chain(self.panes.values_mut()) {
            let key = format!("pane:{}:{suffix}", pane.id);
            let text = self.drafts.get(&key).unwrap_or_default().to_owned();
            let attachments = pane.composer_attachments.remove(&key).unwrap_or_default();
            if text.is_empty() && attachments.is_empty() {
                continue;
            }
            self.drafts.set(&key, "");
            pane.attachments_dirty = true;
            if pane.open.as_ref().is_some_and(|open| {
                open.environment == environment && open.watch.thread_id() == thread
            }) {
                pane.attachments.clear();
                pane.pending_composer = Some(String::new());
                pane.draft_text.clear();
            }
            discarded.push(DiscardedDraft {
                pane: pane.id,
                key,
                text,
                attachments,
            });
        }
        if discarded.is_empty() {
            return;
        }
        if let Err(error) = self.drafts.save(&self.drafts_path) {
            self.conversation.message = Some(format!("Failed to discard draft: {error:#}"));
        }
        self.remember_thread_undo(
            UndoEntry {
                environment,
                thread,
                kind: "Discarded",
                commands: vec![],
                drafts: discarded,
            },
            cx,
        );
        cx.notify();
    }

    fn restore_discarded_drafts(&mut self, entry: &UndoEntry, cx: &mut Context<Self>) {
        if entry.drafts.iter().any(|draft| {
            self.drafts.has(&draft.key)
                || std::iter::once(&self.conversation)
                    .chain(self.panes.values())
                    .find(|pane| pane.id == draft.pane)
                    .is_some_and(|pane| {
                        pane.composer_attachments
                            .get(&draft.key)
                            .is_some_and(|items| !items.is_empty())
                    })
        }) {
            self.conversation.message =
                Some("Failed to restore draft: The draft has new content.".into());
            return;
        }
        for draft in &entry.drafts {
            self.drafts.set(&draft.key, &draft.text);
            self.with_conversation(draft.pane, |root| {
                root.composer_attachments
                    .insert(draft.key.clone(), draft.attachments.clone());
                root.attachments_dirty = true;
                if root.current() == Some((entry.environment.clone(), entry.thread.clone())) {
                    root.conversation.attachments = draft.attachments.clone();
                    root.pending_composer = Some(draft.text.clone());
                }
            });
        }
        if let Err(error) = self.drafts.save(&self.drafts_path) {
            self.conversation.message = Some(format!("Failed to restore draft: {error:#}"));
        }
        cx.notify();
    }

    pub(super) fn reference_label(&self) -> &'static str {
        if self.panel_pull_request_url().is_some()
            || self
                .current()
                .and_then(|(environment, thread)| self.thread_value(&environment, &thread))
                .is_some_and(|value| {
                    value["pullRequests"]
                        .as_array()
                        .and_then(|links| current_pull_request(links))
                        .is_some()
                        || value
                            .pointer("/linkedPullRequest/url")
                            .and_then(Value::as_str)
                            .is_some()
                        || value
                            .pointer("/branchPullRequest/url")
                            .and_then(Value::as_str)
                            .is_some()
                })
        {
            "Copy PR link"
        } else {
            "Copy thread"
        }
    }

    pub(super) fn copy_thread_reference(&mut self, cx: &mut Context<Self>) {
        let Some((environment, thread)) = self.current() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let Some(environment_id) = connection.environment_id() else {
            return;
        };
        let value = self
            .thread_value(&environment, &thread)
            .unwrap_or(Value::Null);
        let panel_url = self.panel_pull_request_url();
        if panel_url == Some(None) {
            return;
        }
        let link = panel_url
            .as_ref()
            .and_then(|url| url.as_deref())
            .or_else(|| {
                value["pullRequests"]
                    .as_array()
                    .and_then(|links| current_pull_request(links))
                    .and_then(|link| link["url"].as_str())
                    .or_else(|| {
                        value
                            .pointer("/linkedPullRequest/url")
                            .or_else(|| value.pointer("/branchPullRequest/url"))
                            .and_then(Value::as_str)
                    })
            });
        let (text, notice) = if let Some(url) = link {
            (url.to_string(), "PR link copied")
        } else {
            (
                format!(
                    "https://app.t3.codes/threads/{}/{}",
                    hosted_segment(&environment_id),
                    hosted_segment(&thread)
                ),
                "Thread copied",
            )
        };
        let mut item = ClipboardItem::new_string(text.clone());
        if link.is_none() {
            let title = value["title"].as_str().unwrap_or_default();
            let label = thread_context_label(title);
            let fragment = json!({"version":1,"source":{"environmentId":environment_id},"records":[{"version":1,"kind":"thread","contextId":format!("thread_{thread}"),"label":label,"title":label,"environmentId":environment_id,"threadId":thread}]}).to_string();
            let html = format!(
                "<pre data-t3-context-fragment=\"{}\">{}</pre>",
                hosted_segment(&fragment),
                html_escape::encode_safe(&text)
            );
            item = item.with_mime_type("web application/x-t3-context-fragment+json", fragment.as_bytes().to_vec())
                .with_mime_type("application/web;type=\"custom/format0\"", fragment.into_bytes())
                .with_mime_type("application/web;type=\"custom/formatmap\"", br#"{"application/x-t3-context-fragment+json":"application/web;type=\"custom/format0\""}"#.to_vec())
                .with_mime_type("text/html", html.into_bytes());
        }
        cx.write_to_clipboard(item);
        self.conversation.message = Some(notice.into());
        cx.notify();
    }

    pub(super) fn regenerate_thread_title(&mut self, cx: &mut Context<Self>) {
        let Some((environment, thread)) = self.current() else {
            return;
        };
        if self
            .thread_value(&environment, &thread)
            .is_some_and(|value| !value["titleRegeneration"].is_null())
        {
            return;
        }
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let runtime = self.runtime.clone();
        self.spawn_pane(cx, async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                connection.dispatch(json!({"type":"thread.metadata.update","threadId":thread,"regenerateTitle":true})).await
            }).await;
            if let Err(error) = result {
                let _ = this.update(cx, |root, cx| { root.conversation.message = Some(format!("Failed to regenerate thread title: {error:#}")); cx.notify(); });
            }
        }).detach();
    }

    pub(super) fn remember_thread_undo(&mut self, entry: UndoEntry, cx: &mut Context<Self>) {
        self.thread_undos.retain(|old| {
            old.environment != entry.environment
                || old.thread != entry.thread
                || old.kind != entry.kind
        });
        self.thread_undos.push(entry);
        self.thread_undo_claimed = true;
        self.thread_undo_epoch += 1;
        let epoch = self.thread_undo_epoch;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            let _ = this.update(cx, |root, cx| {
                if root.thread_undo_epoch == epoch {
                    root.thread_undos.clear();
                    root.thread_undo_claimed = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn undo_thread_action(&mut self, cx: &mut Context<Self>) {
        let Some(kind) = self.thread_undos.last().map(|undo| undo.kind) else {
            return;
        };
        let mut group = vec![];
        while self
            .thread_undos
            .last()
            .is_some_and(|entry| entry.kind == kind)
        {
            group.push(self.thread_undos.pop().unwrap());
        }
        for entry in group {
            if entry.kind == "Discarded" {
                self.restore_discarded_drafts(&entry, cx);
                continue;
            }
            let Some(connection) = self.connection(&entry.environment) else {
                continue;
            };
            let runtime = self.runtime.clone();
            cx.spawn(async move |this, cx| {
                let result = on_runtime(&runtime, async move {
                    for command in entry.commands {
                        if let Some(until) = command["snoozedUntil"].as_str()
                            && chrono::DateTime::parse_from_rfc3339(until)
                                .is_ok_and(|until| until <= chrono::Utc::now())
                        {
                            continue;
                        }
                        connection.dispatch(command).await?;
                    }
                    Ok(())
                })
                .await;
                if let Err(error) = result {
                    let _ = this.update(cx, |root, cx| {
                        root.conversation.message =
                            Some(format!("Failed to undo thread action: {error:#}"));
                        cx.notify();
                    });
                }
            })
            .detach();
        }
        cx.notify();
    }

    pub(super) fn render_thread_undo(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let last = self.thread_undos.last()?;
        let count = self
            .thread_undos
            .iter()
            .rev()
            .take_while(|entry| entry.kind == last.kind)
            .count();
        let label = format!(
            "{} {count} {}{}",
            last.kind,
            if last.kind == "Discarded" {
                "draft"
            } else {
                "thread"
            },
            if count == 1 { "" } else { "s" }
        );
        let undo_label = self
            .key_rules
            .iter()
            .find(|rule| rule.command == "thread.undo")
            .map(|rule| {
                let chord = &rule.chord;
                let mut parts = Vec::new();
                if chord.control {
                    parts.push("Ctrl".to_string());
                }
                if chord.platform {
                    parts.push(
                        if cfg!(target_os = "macos") {
                            "⌘"
                        } else {
                            "Super"
                        }
                        .to_string(),
                    );
                }
                if chord.alt {
                    parts.push("Alt".to_string());
                }
                if chord.shift {
                    parts.push("Shift".to_string());
                }
                parts.push(chord.key.to_uppercase());
                format!("{} to undo", parts.join("+"))
            })
            .unwrap_or_else(|| "Undo".into());
        Some(
            h_flex()
                .id("thread-undo-notice")
                .absolute()
                .left(px(80.))
                .bottom(px(4.))
                .w(px(self.settings.sidebar_width - 88.))
                .h(px(24.))
                .justify_end()
                .overflow_hidden()
                .text_color(crate::appearance::sidebar_muted(cx))
                .gap_1()
                .role(accesskit::Role::Alert)
                .aria_label(label.clone())
                .child(div().text_size(px(10.)).child(if count == 1 {
                    format!("{},", last.kind)
                } else {
                    format!("{} {count},", last.kind)
                }))
                .child(
                    Button::new("undo-thread")
                        .label(undo_label)
                        .text_size(px(10.))
                        .compact()
                        .ghost()
                        .on_click(cx.listener(|root, _, _, cx| root.undo_thread_action(cx))),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{ThreadOp, current_pull_request, hosted_segment, thread_context_label, undo_plan};
    use serde_json::json;

    #[test]
    fn context_labels_follow_utf16_limits_without_splitting_a_character() {
        assert_eq!(thread_context_label(" [A]\\\n B "), "A B");
        assert_eq!(thread_context_label("[ ]"), "thread");
        assert_eq!(
            thread_context_label(&format!("{}🦀", "x".repeat(199))),
            "x".repeat(199)
        );
        assert_eq!(
            thread_context_label(&format!("{}🦀tail", "x".repeat(198))),
            format!("{}🦀", "x".repeat(198))
        );
    }

    #[test]
    fn reference_uses_the_top_open_layer_and_ignores_dismissed_links() {
        let links = vec![
            json!({"number":1,"host":"github.com","repository":"a/b","source":"user","stack":null,"snapshot":{"state":"open","headBranch":"base","baseBranch":"main"}}),
            json!({"number":2,"host":"github.com","repository":"a/b","source":"user","stack":null,"snapshot":{"state":"open","headBranch":"top","baseBranch":"base"}}),
            json!({"number":3,"source":"stack-dismissed","snapshot":null}),
        ];
        assert_eq!(current_pull_request(&links).unwrap()["number"], 2);
        assert!(current_pull_request(&[]).is_none());
        let links = vec![
            json!({"number":1,"snapshot":{"state":"closed","updatedAt":"2026-01-02T00:00:00Z"}}),
            json!({"number":2,"snapshot":{"state":"merged","updatedAt":"2026-01-01T00:00:00Z"}}),
        ];
        assert_eq!(current_pull_request(&links).unwrap()["number"], 1);
    }

    #[test]
    fn undo_settle_restores_pin_before_snooze() {
        let (_, plan) = undo_plan(
            &ThreadOp::Settle,
            &json!({"pinnedAt":"present","pinOrderKey":"a1","snoozedUntil":"future"}),
            "thread",
        )
        .unwrap();
        assert_eq!(
            plan,
            vec![
                json!({"type":"thread.unsettle","threadId":"thread","reason":"user"}),
                json!({"type":"thread.pin","threadId":"thread","orderKey":"a1"}),
                json!({"type":"thread.snooze","threadId":"thread","snoozedUntil":"future"})
            ]
        );
        assert_eq!(hosted_segment("a.日!"), "a%2E%E6%97%A5%21");
    }
}
