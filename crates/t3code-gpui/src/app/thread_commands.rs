use super::*;

pub(super) struct UndoEntry {
    pub environment: String,
    pub thread: String,
    pub kind: &'static str,
    pub commands: Vec<Value>,
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
    pub(super) fn reference_label(&self) -> &'static str {
        if self
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
        let link = value["pullRequests"]
            .as_array()
            .and_then(|links| current_pull_request(links))
            .and_then(|link| link["url"].as_str())
            .or_else(|| {
                value
                    .pointer("/linkedPullRequest/url")
                    .or_else(|| value.pointer("/branchPullRequest/url"))
                    .and_then(Value::as_str)
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
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.message = Some(notice.into());
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
                let _ = this.update(cx, |root, cx| { root.message = Some(format!("Failed to regenerate thread title: {error:#}")); cx.notify(); });
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
                        root.message = Some(format!("Failed to undo thread action: {error:#}"));
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
            "{} {count} thread{}",
            last.kind,
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
                .bottom(px(16.))
                .left(px(16.))
                .w(px(300.))
                .bg(cx.theme().popover)
                .border_1()
                .border_color(cx.theme().border)
                .rounded_md()
                .shadow_lg()
                .px_3()
                .py_2()
                .gap_2()
                .role(accesskit::Role::Alert)
                .aria_label(label.clone())
                .child(div().flex_1().text_xs().child(label))
                .child(
                    Button::new("undo-thread")
                        .label(undo_label)
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
    use super::{ThreadOp, current_pull_request, hosted_segment, undo_plan};
    use serde_json::json;

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
