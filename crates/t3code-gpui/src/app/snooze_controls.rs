use super::choice_select;
use super::*;
use crate::snooze::{self, Preset};
use chrono::{Duration as CalendarDuration, Local};
use gpui_kit::component::select::{Select, SelectEvent};
use gpui_kit::component::{
    button::ButtonCustomVariant,
    date_picker::{DatePicker, DatePickerEvent, DatePickerState},
    dialog::DialogDescription,
    input::NumberInput,
};

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace=t3code_gpui,no_json)]
pub(super) struct SnoozeChoice {
    environment: String,
    thread_id: String,
    shown: Vec<Preset>,
    id: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace=t3code_gpui,no_json)]
pub(super) struct CustomSnooze {
    environment: String,
    thread_id: String,
}

struct CustomSnoozeView {
    root: WeakEntity<Root>,
    target: CustomSnooze,
    duration: bool,
    date: Entity<DatePickerState>,
    time: Entity<InputState>,
    amount: Entity<InputState>,
    unit: String,
    unit_select: Entity<choice_select::State>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl CustomSnoozeView {
    fn new(
        root: WeakEntity<Root>,
        target: CustomSnooze,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial = Local::now() + CalendarDuration::hours(1);
        let date = cx.new(|cx| {
            let mut state = DatePickerState::new(window, cx)
                .date_format("%b %-d, %Y")
                .disabled_matcher(|date: &chrono::NaiveDate| *date < Local::now().date_naive());
            state.set_date(initial.date_naive(), window, cx);
            state
        });
        let time = cx.new(|cx| {
            InputState::new(window, cx).default_value(initial.format("%H:%M").to_string())
        });
        let amount = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value("2")
                .min(0.)
                .step(1.)
        });
        let unit_select = choice_select::state(
            &[("minutes", "Minutes"), ("hours", "Hours"), ("days", "Days")],
            "hours",
            window,
            cx,
        );
        let mut subscriptions = Vec::new();
        for input in [&time, &amount] {
            subscriptions.push(
                cx.subscribe_in(input, window, |view, _, event, window, cx| match event {
                    InputEvent::Change => {
                        view.error = None;
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } if view.submit(window, cx) => {
                        window.close_dialog(cx);
                    }
                    _ => {}
                }),
            );
        }
        subscriptions.push(
            cx.subscribe(&unit_select, |view, _, event: &SelectEvent<_>, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    view.unit = (*value).into();
                    view.error = None;
                    cx.notify();
                }
            }),
        );
        subscriptions.push(cx.subscribe(&date, |view, _, _: &DatePickerEvent, cx| {
            view.error = None;
            cx.notify();
        }));
        Self {
            root,
            target,
            duration: false,
            date,
            time,
            amount,
            unit: "hours".into(),
            unit_select,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn submit(&mut self, _: &mut Window, cx: &mut Context<Self>) -> bool {
        let input = if self.duration {
            json!({"mode":"duration","amount":self.amount.read(cx).value().as_str(),"unit":self.unit})
        } else {
            json!({"mode":"date","date":self.date.read(cx).date_time().start().map(|date| date.format("%Y-%m-%d").to_string()).unwrap_or_default(),"time":self.time.read(cx).value().as_str()})
        };
        let Some(until) = snooze::custom(&input, &Local::now()) else {
            self.error = Some(
                if self.duration {
                    "Enter a positive duration."
                } else {
                    "Choose a valid date and time in the future."
                }
                .into(),
            );
            cx.notify();
            return false;
        };
        let target = self.target.clone();
        let _ = self.root.update(cx, |root, cx| {
            root.thread_op(
                ThreadMenu {
                    environment: target.environment,
                    thread_id: target.thread_id,
                    op: ThreadOp::SnoozeUntil(until),
                },
                cx,
            )
        });
        true
    }
}

impl Render for CustomSnoozeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let error = self.error.clone();
        let label =
            |value: &'static str| div().text_sm().font_weight(FontWeight::MEDIUM).child(value);
        v_flex()
            .gap_4()
            .child(
                DialogDescription::new()
                    .px(px(24.))
                    .child("Choose when snoozed threads return to your inbox."),
            )
            .child(
                h_flex()
                    .id("snooze-schedule-type")
                    .role(accesskit::Role::Group)
                    .aria_label("Schedule type")
                    .mx(px(24.))
                    .rounded(px(8.))
                    .bg(cx.theme().muted.opacity(0.3))
                    .gap_1()
                    .children(
                        [("Date and time", false), ("Duration", true)]
                            .into_iter()
                            .map(|(label, duration)| {
                                Button::new(label)
                                    .label(label)
                                    .ghost()
                                    .selected(self.duration == duration)
                                    .toggled(self.duration == duration)
                                    .h(px(28.))
                                    .flex_1()
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.duration = duration;
                                        view.error = None;
                                        cx.notify();
                                    }))
                            }),
                    ),
            )
            .child(if self.duration {
                h_flex()
                    .px(px(24.))
                    .pb(px(8.))
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(label("Snooze for"))
                            .child(
                                NumberInput::new(&self.amount)
                                    .accessibility_label("Snooze for")
                                    .step_labels("Decrease duration", "Increase duration")
                                    .h(px(32.)),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(label("Unit"))
                            .child(
                                Select::new(&self.unit_select)
                                    .accessibility_label("Unit")
                                    .h(px(32.))
                                    .w_full(),
                            ),
                    )
                    .into_any_element()
            } else {
                h_flex()
                    .px(px(24.))
                    .pb(px(8.))
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(label("Date"))
                            .child(
                                DatePicker::new(&self.date)
                                    .accessibility_label("Date")
                                    .h(px(32.)),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(label("Time"))
                            .child(Input::new(&self.time).aria_label("Time").h(px(32.))),
                    )
                    .into_any_element()
            })
            .when_some(error, |view, error| {
                view.child(
                    div()
                        .id("snooze-error")
                        .px(px(24.))
                        .role(accesskit::Role::Alert)
                        .aria_label(error.clone())
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
            .child(
                h_flex()
                    .px(px(24.))
                    .py(px(16.))
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted.opacity(0.72))
                    .rounded_b(px(16.))
                    .gap_2()
                    .child(
                        Button::new("edit-snooze-options")
                            .label("Edit snooze options")
                            .icon(gpui_kit::assets::IconName::Settings2)
                            .ghost()
                            .h(px(32.))
                            .on_click(cx.listener(|view, _, window, cx| {
                                window.close_dialog(cx);
                                let _ = view.root.update(cx, |root, cx| {
                                    root.settings_open = true;
                                    cx.notify();
                                });
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("cancel-snooze")
                            .label("Cancel")
                            .outline()
                            .h(px(32.))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("confirm-snooze")
                            .label("Snooze")
                            .primary()
                            .h(px(32.))
                            .on_click(cx.listener(|view, _, window, cx| {
                                if view.submit(window, cx) {
                                    window.close_dialog(cx);
                                }
                            })),
                    ),
            )
    }
}

impl Root {
    pub(super) fn snooze_settings(&self, environment: &str) -> Vec<snooze::Setting> {
        snooze::settings(
            self.model_catalogs
                .get(environment)
                .and_then(|(config, _)| config.pointer("/settings/snoozePresets")),
        )
    }

    pub(super) fn choose_snooze(&mut self, action: &SnoozeChoice, cx: &mut Context<Self>) {
        let settings = self.snooze_settings(&action.environment);
        let now = Local::now();
        let selected = snooze::pick(
            &action.shown,
            &action.id,
            &settings,
            &now,
            &|time| crate::timestamp::short(time, cx),
            &|date| crate::timestamp::weekday(date, cx),
        );
        if let Some(preset) = selected {
            self.thread_op(
                ThreadMenu {
                    environment: action.environment.clone(),
                    thread_id: action.thread_id.clone(),
                    op: ThreadOp::SnoozeUntil(preset.snoozed_until),
                },
                cx,
            );
        } else {
            self.message = Some(
                "Could not snooze thread: That snooze time has passed. Choose another time.".into(),
            );
            cx.notify();
        }
    }

    pub(super) fn open_custom_snooze(
        &mut self,
        target: &CustomSnooze,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = cx.entity().downgrade();
        let target = target.clone();
        let view = cx.new(|cx| CustomSnoozeView::new(root, target, window, cx));
        let confirm = view.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            dialog
                .title(
                    div()
                        .px(px(24.))
                        .pt(px(24.))
                        .text_size(px(20.))
                        .line_height(px(27.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Custom snooze"),
                )
                .accessibility_label("Custom snooze")
                .centered()
                .p_0()
                .rounded(px(16.))
                .gap_0()
                .width(px(448.))
                .child(view)
                .on_ok({
                    let confirm = confirm.clone();
                    move |_, window, cx| confirm.update(cx, |view, cx| view.submit(window, cx))
                })
        });
    }

    pub(super) fn render_snooze_button(
        &self,
        entry: &ThreadEntry,
        state: &Entity<sidebar_controls::RowFocus>,
        visible: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let target = CustomSnooze {
            environment: entry.environment.clone(),
            thread_id: entry.thread_id.clone(),
        };
        let root = cx.entity().downgrade();
        let dispatch_focus = self.focus.clone();
        let focus = state.read(cx).snooze.clone();
        let state = state.clone();
        Button::new(SharedString::from(format!(
            "snooze:{}:{}",
            entry.environment, entry.thread_id
        )))
        .accessibility_label("Snooze thread")
        .tooltip("Snooze thread")
        .custom(ButtonCustomVariant::new(cx).foreground(cx.theme().muted_foreground))
        .h(px(20.))
        .px(px(6.))
        .rounded(px(6.))
        .border_0()
        .track_focus(&focus)
        .text_size(px(12.))
        .line_height(px(16.))
        .opacity(if visible { 1. } else { 0. })
        .group_hover("sidebar-row", |button| button.opacity(1.))
        .focus_visible(|button| button.opacity(1.))
        .hover_style(StyleRefinement::default().text_color(cx.theme().foreground))
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(Icon::new(gpui_kit::assets::IconName::Clock).size(px(12.)))
        .dropdown_menu(move |mut menu, _, cx| {
            let settings = root
                .upgrade()
                .map(|root| root.read(cx).snooze_settings(&target.environment))
                .unwrap_or_else(|| snooze::settings(None));
            let shown = snooze::presets(
                &settings,
                &Local::now(),
                &|time| crate::timestamp::short(time, cx),
                &|date| crate::timestamp::weekday(date, cx),
            );
            for preset in &shown {
                let label = preset.label.clone();
                let detail = preset.when_label.clone();
                menu = menu.item(
                    PopupMenuItem::element(move |_, cx| {
                        h_flex()
                            .gap_4()
                            .w_full()
                            .text_sm()
                            .child(label.clone())
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(detail.clone()),
                            )
                    })
                    .accessibility_label(format!("{} {}", preset.label, preset.when_label))
                    .action(Box::new(SnoozeChoice {
                        environment: target.environment.clone(),
                        thread_id: target.thread_id.clone(),
                        shown: shown.clone(),
                        id: preset.id.clone(),
                    })),
                );
            }
            if !shown.is_empty() {
                menu = menu.separator();
            }
            menu.menu("Custom…", Box::new(target.clone()))
                .action_context(dispatch_focus.clone())
        })
        .on_open_change(move |open, window, cx| {
            state.update(cx, |state, _| state.menu_open = *open);
            window.refresh();
        })
        .into_any_element()
    }
}
