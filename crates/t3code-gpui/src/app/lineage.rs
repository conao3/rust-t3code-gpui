use super::*;
use crate::lineage::{self as model, Kind, Relation};

impl Root {
    pub(super) fn sync_lineage(&mut self) {
        let target = self.current();
        let next = self
            .open
            .as_ref()
            .and_then(|open| {
                let connection = self.connection(&open.environment)?;
                let shell = connection.shell();
                let shell = shell.lock().expect("shell lock");
                let state = open.watch.state();
                let state = state.lock().expect("thread lock");
                Some(model::Details::new(
                    &shell.threads,
                    Some(&state),
                    open.watch.thread_id(),
                ))
            })
            .unwrap_or_default();
        if target != self.lineage_target {
            self.lineage_target = target;
            self.lineage_counts = [6; 4];
            self.lineage_previous = false;
            self.lineage_busy = None;
            self.lineage_revision += 1;
        }
        if next != self.lineage_data {
            self.lineage_data = next;
            self.lineage_revision += 1;
        }
    }

    fn lineage_heading(&self) -> bool {
        self.lineage_data.can_detach
            || self.lineage_data.running > 0
            || self
                .lineage_data
                .rows
                .iter()
                .any(|row| matches!(model::group(row), 0 | 1))
    }

    pub(super) fn lineage_height(&self) -> f32 {
        let mut counts = [0; 4];
        for row in &self.lineage_data.rows {
            counts[model::group(row)] += 1;
        }
        let mut height = 0.;
        if counts[..3].iter().any(|count| *count > 0)
            || self.lineage_data.can_detach
            || self.lineage_data.running > 0
        {
            height += 19. + if self.lineage_heading() { 36. } else { 0. };
        }
        if counts[3] > 0 {
            height += 55.;
        }
        for (group, count) in counts.iter().enumerate() {
            if *count == 0 {
                continue;
            }
            if group == 2 {
                height += 32.;
                if !self.lineage_previous {
                    continue;
                }
            }
            height += ((*count).min(self.lineage_counts[group]) as f32 * 32.).min(216.);
            if *count > self.lineage_counts[group] {
                height += 32.;
            }
        }
        height
    }

    pub(super) fn render_lineage(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let mut content = v_flex().w_full();
        let groups: [Vec<&Relation>; 4] = std::array::from_fn(|group| {
            self.lineage_data
                .rows
                .iter()
                .filter(|row| model::group(row) == group)
                .collect()
        });
        if groups[..3].iter().any(|rows| !rows.is_empty())
            || self.lineage_data.can_detach
            || self.lineage_data.running > 0
        {
            let title = if self.lineage_data.running > 0 {
                format!("Lineage · {} running", self.lineage_data.running)
            } else {
                "Lineage".into()
            };
            let mut section = v_flex()
                .id("thread-details-lineage")
                .role(accesskit::Role::Region)
                .aria_label("Lineage")
                .px_2()
                .pt_2()
                .pb_2p5()
                .border_t_1()
                .border_color(theme.border.opacity(0.65));
            if self.lineage_heading() {
                let mut heading = h_flex().h(px(32.)).mb_1().px_2p5().gap_2().child(
                    div()
                        .id("thread-details-lineage-heading")
                        .role(accesskit::Role::Heading)
                        .aria_label(title.clone())
                        .flex_1()
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.muted_foreground)
                        .child(title),
                );
                if self.lineage_data.can_detach {
                    let focus = self.focus.clone();
                    heading = heading.child(
                        Button::new("lineage-actions")
                            .icon(IconName::Ellipsis)
                            .ghost()
                            .size(px(24.))
                            .px_0()
                            .accessibility_label("More thread actions")
                            .disabled(self.lineage_busy.is_some())
                            .dropdown_menu(move |menu, _, _| {
                                menu.menu(
                                    "Disconnect agent session",
                                    Box::new(WorkspaceAction {
                                        kind: "detach-agent".into(),
                                        value: Value::Null,
                                    }),
                                )
                                .min_w(px(240.))
                                .action_context(focus.clone())
                            }),
                    );
                }
                section = section.child(heading);
            }
            for (group, rows) in groups.iter().enumerate().take(3) {
                section = section.child(self.render_lineage_group(group, rows, cx));
            }
            content = content.child(section);
        }
        if !groups[3].is_empty() {
            let running = groups[3]
                .iter()
                .filter(|row| row.edge_status.as_deref() == Some("running"))
                .count();
            let title = if running > 0 {
                format!("Side chats · {running} running")
            } else {
                "Side chats".into()
            };
            content = content.child(
                v_flex()
                    .id("thread-details-side-chats")
                    .role(accesskit::Role::Region)
                    .aria_label("Side chats")
                    .px_2()
                    .pt_2()
                    .pb_2p5()
                    .border_t_1()
                    .border_color(theme.border.opacity(0.65))
                    .child(
                        h_flex()
                            .h(px(32.))
                            .mb_1()
                            .px_2p5()
                            .text_size(px(11.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.muted_foreground)
                            .child(title),
                    )
                    .child(self.render_lineage_group(3, &groups[3], cx)),
            );
        }
        content.into_any_element()
    }

    fn render_lineage_group(
        &self,
        group: usize,
        rows: &[&Relation],
        cx: &Context<Self>,
    ) -> AnyElement {
        if rows.is_empty() {
            return div().into_any_element();
        }
        let theme = cx.theme();
        let mut content = v_flex().w_full();
        if group == 2 {
            let failed = rows
                .iter()
                .filter(|row| matches!(row.edge_status.as_deref(), Some("failed" | "error")))
                .count();
            let expanded = self.lineage_previous;
            content = content.child(
                Button::new("lineage-previous")
                    .ghost()
                    .w_full()
                    .h(px(32.))
                    .px_2p5()
                    .rounded(px(8.))
                    .accessibility_label("Previous agents")
                    .aria_expanded(expanded)
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2p5()
                            .child(
                                Icon::new(gpui_kit::assets::IconName::Bot)
                                    .size(px(16.))
                                    .text_color(theme.muted_foreground),
                            )
                            .child(div().flex_1().text_size(px(13.)).child("Previous agents"))
                            .when(failed > 0, |row| {
                                row.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(theme.danger)
                                        .child(format!("{failed} failed")),
                                )
                            })
                            .when(!expanded, |row| {
                                row.child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(theme.muted_foreground)
                                        .child(rows.len().to_string()),
                                )
                            })
                            .child(
                                Icon::new(if expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .size(px(12.)),
                            ),
                    )
                    .on_click(cx.listener(|root, _, _, cx| {
                        root.lineage_previous = !root.lineage_previous;
                        root.lineage_revision += 1;
                        cx.notify();
                    })),
            );
            if !expanded {
                return content.into_any_element();
            }
        }
        let count = self.lineage_counts[group].min(rows.len());
        content = content.child(
            v_flex()
                .id(SharedString::from(format!("lineage-group-{group}")))
                .role(accesskit::Role::List)
                .aria_label("Related threads")
                .w_full()
                .max_h(px(216.))
                .overflow_y_scroll()
                .children(
                    rows.iter()
                        .take(count)
                        .map(|row| self.render_lineage_row(row, cx)),
                ),
        );
        if count < rows.len() {
            let label = format!("Show {} more", (rows.len() - count).min(12));
            content = content.child(
                Button::new(SharedString::from(format!("lineage-more-{group}")))
                    .ghost()
                    .w_full()
                    .h(px(32.))
                    .px_2p5()
                    .rounded(px(8.))
                    .accessibility_label(label.clone())
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2p5()
                            .text_size(px(14.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.muted_foreground.opacity(0.7))
                            .child(Icon::new(IconName::Plus).size(px(16.)))
                            .child(label),
                    )
                    .on_click(cx.listener(move |root, _, _, cx| {
                        root.lineage_counts[group] += 12;
                        root.lineage_revision += 1;
                        cx.notify();
                    })),
            );
        }
        content.into_any_element()
    }

    fn render_lineage_row(&self, row: &Relation, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let target = row.thread_id.clone();
        let environment = self
            .lineage_target
            .as_ref()
            .map(|(environment, _)| environment.clone())
            .unwrap_or_default();
        let merge = self.lineage_data.merge_target.as_deref() == Some(target.as_str());
        let status = model::status_label(row.status.as_deref());
        let icon = if row.parent {
            gpui_kit::assets::IconName::CornerLeftUp
        } else if row.kind == Kind::Subagent {
            gpui_kit::assets::IconName::Bot
        } else if row.side_chat {
            gpui_kit::assets::IconName::MessagesSquare
        } else {
            gpui_kit::assets::IconName::GitFork
        };
        let dot = match row.status.as_deref() {
            Some("running" | "in_progress" | "pending" | "waiting") => theme.info,
            Some("failed" | "error") => theme.danger,
            Some("completed") => theme.success,
            _ => theme.muted_foreground.opacity(0.45),
        };
        let label = format!("{} {status}", row.title);
        let button = Button::new(SharedString::from(format!("lineage-open-{target}")))
            .ghost()
            .flex_1()
            .min_w_0()
            .h(px(32.))
            .px_2p5()
            .rounded(px(8.))
            .accessibility_label(label)
            .tooltip(row.title.clone())
            .hover_style(StyleRefinement::default().bg(if self.settings.light_theme {
                hsla(0., 0., 0., 0.035)
            } else {
                hsla(0., 0., 1., 0.08)
            }))
            .child(
                h_flex()
                    .w_full()
                    .gap_2p5()
                    .text_size(px(14.))
                    .line_height(px(16.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.foreground.opacity(0.85))
                    .child(
                        div()
                            .relative()
                            .size(px(16.))
                            .flex_shrink_0()
                            .child(
                                Icon::new(icon)
                                    .size(px(16.))
                                    .text_color(theme.muted_foreground),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .bottom(px(-4.))
                                    .right(px(-4.))
                                    .size(px(8.))
                                    .rounded_full()
                                    .border_2()
                                    .border_color(theme.popover)
                                    .bg(dot),
                            ),
                    )
                    .child(div().flex_1().truncate().child(row.title.clone()))
                    .when(!merge, |content| {
                        content.child(
                            div()
                                .text_size(px(11.))
                                .font_weight(FontWeight::NORMAL)
                                .text_color(theme.muted_foreground)
                                .child(status),
                        )
                    }),
            )
            .on_click(cx.listener(move |root, _, _, cx| {
                root.open_thread(environment.clone(), target.clone(), cx)
            }));
        let mut content = h_flex().w_full().h(px(32.)).flex_shrink_0().child(button);
        if merge {
            let title = row.title.clone();
            content = content
                .child(div().w(px(1.)).h(px(16.)).bg(theme.border.opacity(0.55)))
                .child(
                    Button::new("lineage-merge")
                        .ghost()
                        .w(px(32.))
                        .h(px(32.))
                        .px_0()
                        .rounded(px(8.))
                        .accessibility_label(format!("Merge back to {title}"))
                        .disabled(
                            self.lineage_data.merge_run.is_none() || self.lineage_busy.is_some(),
                        )
                        .tooltip(if self.lineage_data.merge_run.is_none() {
                            "Complete a run in this fork before merging it back".into()
                        } else {
                            format!("Merge this conversation back into {title}")
                        })
                        .child(Icon::new(gpui_kit::assets::IconName::GitMerge).size(px(12.)))
                        .on_click(cx.listener(|root, _, window, cx| {
                            root.lineage_action("merge", window, cx)
                        })),
                )
                .child(
                    div()
                        .pl_1()
                        .pr_2p5()
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.muted_foreground)
                        .child(status),
                );
        }
        content.into_any_element()
    }

    pub(super) fn lineage_action(
        &mut self,
        action: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.lineage_busy.is_some() {
            return;
        }
        let Some((environment, thread_id)) = self.current() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let merge = self
            .lineage_data
            .merge_target
            .clone()
            .zip(self.lineage_data.merge_run.clone());
        if action == "merge" && merge.is_none()
            || action == "detach" && !self.lineage_data.can_detach
        {
            return;
        }
        let busy = (thread_id.clone(), action.to_string());
        self.lineage_busy = Some(busy.clone());
        self.lineage_revision += 1;
        cx.notify();
        let runtime = self.runtime.clone();
        cx.spawn_in(window, async move |this, cx| {
            let next = merge.as_ref().map(|(target,_)| target.clone());
            let result = on_runtime(&runtime, async move {
                if action == "merge" {
                    let (target,run) = merge.expect("validated merge");
                    connection.dispatch(json!({"type":"thread.merge_back","createdBy":"user","creationSource":"web","sourceThreadId":thread_id,"targetThreadId":target,"sourcePoint":{"type":"run","runId":run}})).await?;
                } else {
                    let state = connection.request("orchestration.getThreadProjection",json!({"threadId":thread_id})).await?;
                    for session in state["providerSessions"].as_array().into_iter().flatten() {
                        let Some(id) = session["id"].as_str() else {continue;};
                        connection.dispatch(json!({"type":"provider-session.detach","threadId":thread_id,"providerSessionId":id,"reason":"client-requested"})).await?;
                    }
                }
                Ok::<_,anyhow::Error>(())
            }).await;
            let _ = this.update_in(cx, |root, _, cx| {
                if root.lineage_busy.as_ref() == Some(&busy) {root.lineage_busy = None; root.lineage_revision += 1;}
                match result {
                    Ok(()) if action == "merge" => {if let Some(target) = next {root.open_thread(environment,target,cx);}},
                    Ok(()) => {},
                    Err(error) => root.message = Some(format!("Could not {action} agent session: {error:#}")),
                }
                cx.notify();
            });
        }).detach();
    }
}
