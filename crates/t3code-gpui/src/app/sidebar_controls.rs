use super::*;
use gpui_kit::component::button::ButtonCustomVariant;

#[derive(Clone)]
pub(super) struct RowFocus {
    pub row: FocusHandle,
    pub snooze: FocusHandle,
    pub settle: FocusHandle,
    pub menu_open: bool,
}

impl RowFocus {
    pub fn new(cx: &mut App) -> Self {
        Self {
            row: cx.focus_handle(),
            snooze: cx.focus_handle(),
            settle: cx.focus_handle(),
            menu_open: false,
        }
    }
}

impl Root {
    pub(super) fn render_row_metadata(
        &self,
        entry: &ThreadEntry,
        section: sidebar::Section,
        state: &Entity<RowFocus>,
        visible: bool,
        jump: Option<String>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let now = now_iso();
        let shell = &entry.presentation;
        let woke = crate::snooze::woke_at(shell, &now);
        let unseen = crate::snooze::woke_unseen(
            woke.as_deref(),
            shell["settledOverride"] == "settled",
            shell["lastVisitedAt"].as_str(),
            None,
        );
        let snoozed = section == sidebar::Section::Snoozed;
        let slim = snoozed || section == sidebar::Section::Settled;
        let unseen =
            unseen && (slim || sidebar::display_status(shell) == sidebar::DisplayStatus::Ready);
        let timestamp = if section == sidebar::Section::Settled {
            shell["settledAt"].as_str()
        } else {
            entry.activity_timestamp.as_deref()
        };
        let status = sidebar::display_status(shell);
        let top = if status == sidebar::DisplayStatus::Working
            && shell.pointer("/goal/status").and_then(Value::as_str) == Some("active")
        {
            "Goal"
        } else if status != sidebar::DisplayStatus::Ready {
            status.label()
        } else if unseen {
            "Woke"
        } else if sidebar::unseen_completion(shell) {
            "Done"
        } else {
            ""
        };
        let age = if snoozed {
            crate::snooze::wake_label(entry.snoozed_until.as_deref().unwrap_or_default(), &now)
        } else if unseen {
            "Woke".into()
        } else if !slim && !top.is_empty() {
            top.into()
        } else {
            timestamp
                .map(|time| sidebar::relative_label(time, SystemTime::now()))
                .unwrap_or_default()
        };
        let icon = if snoozed || unseen {
            Some(gpui_kit::assets::IconName::AlarmClock)
        } else if !slim {
            use gpui_kit::assets::IconName as I;
            Some(match status {
                sidebar::DisplayStatus::Preparing | sidebar::DisplayStatus::Working => {
                    I::CircleDashed
                }
                sidebar::DisplayStatus::Waiting => I::CircleDotDashed,
                sidebar::DisplayStatus::Approval => I::ShieldQuestionMark,
                sidebar::DisplayStatus::Input => I::MessageCircleQuestionMark,
                sidebar::DisplayStatus::Request => I::UserRound,
                sidebar::DisplayStatus::Limited | sidebar::DisplayStatus::Failed => I::CircleAlert,
                sidebar::DisplayStatus::Ready if !top.is_empty() => I::CircleCheck,
                _ => I::Clock,
            })
        } else {
            None
        };
        let color = if !slim && status != sidebar::DisplayStatus::Ready {
            match status {
                sidebar::DisplayStatus::Preparing | sidebar::DisplayStatus::Working => {
                    cx.theme().info_foreground
                }
                sidebar::DisplayStatus::Approval | sidebar::DisplayStatus::Limited => {
                    cx.theme().warning_foreground
                }
                sidebar::DisplayStatus::Failed => cx.theme().danger,
                sidebar::DisplayStatus::Input => rgb(if self.settings.light_theme {
                    0x4f46e5
                } else {
                    0xa5b4fc
                })
                .into(),
                _ => cx.theme().muted_foreground,
            }
        } else if unseen {
            cx.theme().warning_foreground
        } else if snoozed {
            cx.theme().info_foreground
        } else {
            crate::appearance::secondary_label(cx)
        };
        let height = if slim { 24. } else { 20. };
        let mut slot = h_flex()
            .relative()
            .h(px(height))
            .min_w(px(32.))
            .flex_none()
            .justify_end();
        if let Some(label) = jump {
            return slot
                .child(
                    div()
                        .px_1()
                        .h(px(20.))
                        .min_w(px(20.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_sm()
                        .bg(cx.theme().foreground.opacity(0.07))
                        .font_weight(FontWeight::MEDIUM)
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(label),
                )
                .into_any_element();
        }
        let duration = if !slim {
            match sidebar::display_status(shell) {
                sidebar::DisplayStatus::Preparing | sidebar::DisplayStatus::Working => {
                    sidebar::working_started_at(shell)
                }
                sidebar::DisplayStatus::Waiting => shell
                    .pointer("/latestRun/completedAt")
                    .and_then(Value::as_str),
                _ => None,
            }
        } else {
            None
        };
        let duration = duration
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|start| {
                sidebar::duration_label(
                    (chrono::Utc::now().timestamp_millis() - start.timestamp_millis()) as f64,
                )
            });
        let status = h_flex()
            .gap_1()
            .text_xs()
            .text_color(color)
            .when_some(
                icon.filter(|_| snoozed || unseen || !top.is_empty()),
                |row, icon| row.child(Icon::new(icon).size(px(12.))),
            )
            .child(settings_controls::text(age))
            .when_some(duration, |row, duration| row.child(duration));
        let content = if unseen {
            let acknowledged_at = woke.clone().expect("unseen wake timestamp");
            let environment = entry.environment.clone();
            let thread = entry.thread_id.clone();
            Button::new(SharedString::from(format!("woke:{environment}:{thread}")))
                .accessibility_label("Dismiss Woke notification")
                .custom(ButtonCustomVariant::new(cx).foreground(color))
                .p_0()
                .h(px(height))
                .border_0()
                .child(status)
                .on_click(cx.listener(move |root, _, _, cx| {
                    cx.stop_propagation();
                    let Some(connection) = root.connection(&environment) else {
                        return;
                    };
                    let thread = thread.clone();
                    let visited = acknowledged_at.clone();
                    let runtime = root.runtime.clone();
                    cx.spawn(async move |this, cx| {
                        let result = on_runtime(&runtime, async move {
                            connection.visit_thread(&thread, &visited).await
                        })
                        .await;
                        if let Err(error) = result {
                            let _ = this.update(cx, |root, cx| {
                                root.conversation.message =
                                    Some(format!("Could not dismiss Woke notification: {error:#}"));
                                cx.notify();
                            });
                        }
                    })
                    .detach();
                }))
                .into_any_element()
        } else {
            status.into_any_element()
        };
        slot = slot.child(
            div()
                .when(visible, |status| status.absolute().opacity(0.))
                .when(!unseen, |status| {
                    status.group_hover("sidebar-row", |status| status.absolute().opacity(0.))
                })
                .child(content),
        );
        if self.sidebar_catalogs.contains_key(&entry.environment) {
            slot = slot.child(self.render_row_actions(entry, section, state, visible, cx));
        }
        slot.into_any_element()
    }

    pub(super) fn render_row_actions(
        &self,
        entry: &ThreadEntry,
        section: sidebar::Section,
        state: &Entity<RowFocus>,
        visible: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let focus = state.read(cx).settle.clone();
        let (label, icon, op) = match section {
            sidebar::Section::Snoozed => (
                "Wake thread now",
                gpui_kit::assets::IconName::AlarmClockOff,
                ThreadOp::Unsnooze,
            ),
            sidebar::Section::Settled => (
                "Un-settle thread",
                gpui_kit::assets::IconName::Undo2,
                ThreadOp::Unsettle,
            ),
            _ => (
                "Settle thread",
                gpui_kit::assets::IconName::Check,
                ThreadOp::Settle,
            ),
        };
        let slim = matches!(
            section,
            sidebar::Section::Snoozed | sidebar::Section::Settled
        );
        let height = if slim { 24. } else { 20. };
        let target = ThreadMenu {
            environment: entry.environment.clone(),
            thread_id: entry.thread_id.clone(),
            op,
        };
        let discard_target = (entry.environment.clone(), entry.thread_id.clone());
        let has_draft = self.current().as_ref() != Some(&discard_target)
            && self.has_thread_draft(&entry.environment, &entry.thread_id);
        h_flex()
            .h(px(height))
            .flex_none()
            .when(!visible, |actions| actions.absolute())
            .right_0()
            .top_0()
            .group_hover("sidebar-row", |actions| actions.relative())
            .when(has_draft, |actions| {
                actions.child(
                    Button::new(SharedString::from(format!(
                        "discard-draft:{}:{}",
                        entry.environment, entry.thread_id
                    )))
                    .icon(IconName::Close)
                    .accessibility_label("Discard draft")
                    .tooltip("Discard draft")
                    .ghost()
                    .compact()
                    .h(px(height))
                    .opacity(if visible { 1. } else { 0. })
                    .group_hover("sidebar-row", |button| button.opacity(1.))
                    .focus_visible(|button| button.opacity(1.))
                    .on_click(cx.listener(move |root, _, _, cx| {
                        cx.stop_propagation();
                        root.discard_thread_draft(
                            discard_target.0.clone(),
                            discard_target.1.clone(),
                            cx,
                        );
                    })),
                )
            })
            .when(
                !slim && crate::snooze::can_snooze(&entry.presentation, &now_iso()),
                |actions| actions.child(self.render_snooze_button(entry, state, visible, cx)),
            )
            .child(
                Button::new(SharedString::from(format!(
                    "row-action:{}:{}",
                    entry.environment, entry.thread_id
                )))
                .accessibility_label(label)
                .tooltip(label)
                .track_focus(&focus)
                .custom(ButtonCustomVariant::new(cx).foreground(cx.theme().muted_foreground))
                .h(px(height))
                .px(px(6.))
                .mr(px(-4.))
                .rounded(px(6.))
                .border_0()
                .text_size(px(12.))
                .line_height(px(16.))
                .opacity(if visible { 1. } else { 0. })
                .group_hover("sidebar-row", |button| button.opacity(1.))
                .focus_visible(|button| button.opacity(1.))
                .hover_style(StyleRefinement::default().text_color(cx.theme().foreground))
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Icon::new(icon).size(px(if section == sidebar::Section::Snoozed {
                                12.
                            } else {
                                14.
                            })),
                        )
                        .when(!slim, |row| row.child(settings_controls::text("Settle"))),
                )
                .on_click(cx.listener(move |root, _, _, cx| {
                    cx.stop_propagation();
                    root.thread_op(target.clone(), cx);
                })),
            )
            .into_any_element()
    }

    pub(super) fn render_shelf_toggle(
        &self,
        section: sidebar::Section,
        count: usize,
        bottom: bool,
        child_index: usize,
        cx: &Context<Self>,
    ) -> AnyElement {
        let expanded = !self.collapsed.contains(&section);
        let color = crate::appearance::sidebar_muted(cx);
        let scroll = self.sidebar_scroll.clone();
        Button::new(SharedString::from(format!("shelf-{}", section.label())))
            .custom(
                ButtonCustomVariant::new(cx)
                    .color(transparent_black())
                    .foreground(color)
                    .hover(transparent_black())
                    .active(transparent_black()),
            )
            .h(px(32.))
            .mx(px(2.))
            .px_2()
            .rounded(px(6.))
            .cursor_pointer()
            .when(bottom, |button| button.mt_auto())
            .accessibility_label(format!("{} {count}", section.label()))
            .aria_expanded(expanded)
            .inset_focus_ring(px(2.), cx.theme().ring.opacity(0.7))
            .on_a11y_action(accesskit::Action::ScrollIntoView, move |_, window, _| {
                scroll.scroll_to_item(child_index);
                window.refresh();
            })
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(color)
                    .child(
                        div()
                            .truncate()
                            .child(settings_controls::text(section.label())),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .font_weight(FontWeight::NORMAL)
                            .font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])))
                            .child(settings_controls::text(count.to_string())),
                    )
                    .child(
                        Icon::new(if expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(px(12.)),
                    ),
            )
            .on_click(cx.listener(move |root, _, _, cx| {
                if !root.collapsed.remove(&section) {
                    root.collapsed.insert(section);
                }
                let sections = [
                    sidebar::Section::Working,
                    sidebar::Section::Snoozed,
                    sidebar::Section::Settled,
                ];
                let flags = sections.map(|section| !root.collapsed.contains(&section));
                root.change_setting(
                    "gpuiSidebarShelves",
                    json!({"working":flags[0],"snoozed":flags[1],"settled":flags[2]}),
                    cx,
                );
            }))
            .into_any_element()
    }
}
