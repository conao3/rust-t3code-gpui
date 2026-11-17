use super::choice_select;
use super::*;
use crate::snooze::{self, Setting};
use chrono::Local;
use gpui_kit::component::input::{NumberInput, NumberInputEvent, StepAction};
use gpui_kit::component::select::{Select, SelectEvent};

#[derive(Clone, Debug, PartialEq)]
struct Choose {
    row: u64,
    field: String,
    value: String,
}

struct OptionRow {
    _subscriptions: Vec<Subscription>,
    key: u64,
    value: Setting,
    amount: Entity<InputState>,
    time: Entity<InputState>,
    unit: i64,
    when_select: Entity<choice_select::State>,
    unit_select: Entity<choice_select::State>,
    remembered_minutes: i64,
    remembered_time: String,
}

pub(super) struct SnoozeOptions {
    focus: FocusHandle,
    root: WeakEntity<Root>,
    environment: String,
    rows: Vec<OptionRow>,
    next_key: u64,
    pending: Vec<Vec<Setting>>,
    source: Vec<Setting>,
    error: Option<String>,
    needs_revert: bool,
    subscriptions: Vec<Subscription>,
}

impl SnoozeOptions {
    pub(super) fn new(
        root: WeakEntity<Root>,
        environment: String,
        saved: Vec<Setting>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            focus: cx.focus_handle(),
            root: root.clone(),
            environment,
            rows: vec![],
            next_key: 0,
            pending: vec![],
            source: saved.clone(),
            error: None,
            needs_revert: false,
            subscriptions: vec![],
        };
        view.adopt(saved, window, cx);
        if let Some(root) = root.upgrade() {
            view.subscriptions
                .push(cx.observe_in(&root, window, |view, root, window, cx| {
                    let saved = root.read(cx).snooze_settings(&view.environment);
                    if saved == view.source {
                        return;
                    }
                    view.source = saved.clone();
                    if let Some(index) = view.pending.iter().position(|snapshot| snapshot == &saved)
                    {
                        view.pending.drain(..=index);
                    } else {
                        view.pending.clear();
                        view.adopt(saved, window, cx);
                    }
                    cx.notify();
                }));
        }
        view
    }

    fn insert(&mut self, value: Setting, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.next_key;
        self.next_key += 1;
        let minutes = match &value {
            Setting::After { minutes } => *minutes,
            _ => 60,
        };
        let unit = if minutes % 1440 == 0 {
            1440
        } else if minutes % 60 == 0 {
            60
        } else {
            1
        };
        let time = match &value {
            Setting::At { time, .. } => time.clone(),
            _ => "09:00".into(),
        };
        let amount = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value((minutes / unit).to_string())
                .min(1.)
                .max((43200 / unit) as f64)
                .step(1.)
        });
        amount.update(cx, |input, cx| input.set_step(None, window, cx));
        let time_input = cx.new(|cx| InputState::new(window, cx).default_value(time.clone()));
        let when = match &value {
            Setting::After { .. } => "after",
            Setting::At { day, .. } => day.as_str(),
        };
        let when_select = choice_select::state(choice_select::WHEN, when, window, cx);
        let unit_name = match unit {
            1440 => "days",
            60 => "hours",
            _ => "minutes",
        };
        let unit_select = choice_select::state(
            &choice_select::units(minutes / unit == 1),
            unit_name,
            window,
            cx,
        );
        let mut subscriptions = vec![];
        for (select, field) in [(&when_select, "when"), (&unit_select, "unit")] {
            subscriptions.push(cx.subscribe_in(
                select,
                window,
                move |view, _, event: &SelectEvent<_>, window, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        view.choose(
                            &Choose {
                                row: key,
                                field: field.into(),
                                value: (*value).into(),
                            },
                            window,
                            cx,
                        );
                    }
                },
            ));
        }
        subscriptions.push(
            cx.subscribe_in(&amount, window, move |view, _, event, window, cx| {
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    view.commit_amount(key, window, cx);
                }
            }),
        );
        subscriptions.push(cx.subscribe_in(
            &amount,
            window,
            move |view, input, event: &NumberInputEvent, window, cx| {
                let NumberInputEvent::Step(action) = event;
                let Some(row) = view.rows.iter().find(|row| row.key == key) else {
                    return;
                };
                let current = input
                    .read(cx)
                    .value()
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .unwrap_or(0.);
                let next = (current
                    + if *action == StepAction::Increment {
                        1.
                    } else {
                        -1.
                    })
                .clamp(1., (43200 / row.unit) as f64);
                input.update(cx, |input, cx| {
                    input.set_value(next.to_string(), window, cx)
                });
                view.commit_amount(key, window, cx);
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &time_input,
            window,
            move |view, input, event, _, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let value = input.read(cx).value().to_string();
                if chrono::NaiveTime::parse_from_str(&value, "%H:%M").is_err() || value.len() != 5 {
                    return;
                }
                if let Some(row) = view.rows.iter_mut().find(|row| row.key == key)
                    && let Setting::At { time, .. } = &mut row.value
                    && *time != value
                {
                    *time = value;
                    view.save(cx);
                }
            },
        ));
        self.rows.push(OptionRow {
            _subscriptions: subscriptions,
            key,
            value,
            amount,
            time: time_input,
            unit,
            when_select,
            unit_select,
            remembered_minutes: 60,
            remembered_time: "09:00".into(),
        });
    }

    fn adopt(&mut self, mut values: Vec<Setting>, window: &mut Window, cx: &mut Context<Self>) {
        let now = Local::now();
        values.sort_by_key(|setting| {
            snooze::resolve(setting, &now, &|_| String::new(), &|_| String::new())
                .map(|value| value.snoozed_until)
                .unwrap_or_else(|| "~".into())
        });
        self.rows.clear();
        for value in values {
            self.insert(value, window, cx);
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let values = self
            .rows
            .iter()
            .map(|row| row.value.clone())
            .collect::<Vec<_>>();
        self.pending.push(values.clone());
        let Some(root) = self.root.upgrade() else {
            return;
        };
        let root = root.read(cx);
        let Some(connection) = root.connection(&self.environment) else {
            return;
        };
        let runtime = root.runtime.clone();
        self.error = None;
        cx.spawn(async move |this, cx| {
            let result = on_runtime(&runtime, async move {
                connection
                    .request(
                        "server.updateSettings",
                        json!({"patch":{"snoozePresets":values}}),
                    )
                    .await
            })
            .await;
            if let Err(error) = result {
                let _ = this.update(cx, |view, cx| {
                    view.pending.clear();
                    view.needs_revert = true;
                    view.error = Some(format!("Setting not saved: {error:#}"));
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    fn commit_amount(&mut self, key: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter_mut().find(|row| row.key == key) else {
            return;
        };
        let Setting::After { minutes } = &mut row.value else {
            return;
        };
        let current = *minutes / row.unit;
        let value = row
            .amount
            .read(cx)
            .value()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(|value| value.round().clamp(1., (43200 / row.unit) as f64) as i64)
            .unwrap_or(current);
        row.amount.update(cx, |input, cx| {
            input.set_value(value.to_string(), window, cx)
        });
        let next = value * row.unit;
        if *minutes != next {
            *minutes = next;
            Self::sync_selects(row, window, cx);
            self.save(cx);
        }
    }

    fn sync_selects(row: &mut OptionRow, window: &mut Window, cx: &mut Context<Self>) {
        let when = match &row.value {
            Setting::After { .. } => "after",
            Setting::At { day, .. } => day.as_str(),
        };
        let when = choice_select::WHEN
            .iter()
            .find(|(value, _)| *value == when)
            .map(|(value, _)| *value)
            .unwrap_or("after");
        row.when_select.update(cx, |select, cx| {
            select.set_selected_value(&when, window, cx)
        });
        let unit = match row.unit {
            1440 => "days",
            60 => "hours",
            _ => "minutes",
        };
        let singular = matches!(row.value,Setting::After{minutes} if minutes/row.unit==1);
        row.unit_select.update(cx, |select, cx| {
            select.set_items(
                choice_select::items(&choice_select::units(singular)),
                window,
                cx,
            );
            select.set_selected_value(&unit, window, cx);
        });
    }

    fn choose(&mut self, action: &Choose, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter_mut().find(|row| row.key == action.row) else {
            return;
        };
        let previous = row.value.clone();
        let unchanged = match &row.value {
            Setting::After { .. } => action.field == "when" && action.value == "after",
            Setting::At { day, .. } => action.field == "when" && day == &action.value,
        };
        if unchanged {
            return;
        }
        if action.field == "when" {
            row.value = if action.value == "after" {
                match &row.value {
                    Setting::After { .. } => row.value.clone(),
                    Setting::At { time, .. } => {
                        row.remembered_time = time.clone();
                        Setting::After {
                            minutes: row.remembered_minutes,
                        }
                    }
                }
            } else {
                let time = match &row.value {
                    Setting::At { time, .. } => time.clone(),
                    Setting::After { minutes } => {
                        row.remembered_minutes = *minutes;
                        row.remembered_time.clone()
                    }
                };
                Setting::At {
                    day: action.value.clone(),
                    time,
                }
            };
            match &row.value {
                Setting::After { minutes } => {
                    row.unit = if minutes % 1440 == 0 {
                        1440
                    } else if minutes % 60 == 0 {
                        60
                    } else {
                        1
                    };
                    let value = (minutes / row.unit).to_string();
                    row.amount
                        .update(cx, |input, cx| input.set_value(value, window, cx));
                }
                Setting::At { time, .. } => {
                    row.time
                        .update(cx, |input, cx| input.set_value(time.clone(), window, cx));
                }
            }
        } else if let Setting::After { minutes } = &mut row.value {
            let unit = match action.value.as_str() {
                "days" => 1440,
                "hours" => 60,
                _ => 1,
            };
            *minutes = (*minutes / row.unit * unit).min(43200);
            row.unit = unit;
            row.amount.update(cx, |input, cx| {
                input.set_value((*minutes / unit).to_string(), window, cx)
            });
        }
        Self::sync_selects(row, window, cx);
        if row.value == previous {
            return;
        }
        row.amount.update(cx, |input, cx| {
            input.set_max(Some((43200 / row.unit) as f64), window, cx)
        });
        self.save(cx);
    }
}

impl Render for SnoozeOptions {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.needs_revert) {
            self.adopt(self.source.clone(), window, cx);
        }
        let focus = self.focus.clone();
        let now = Local::now();
        let values = self
            .rows
            .iter()
            .map(|row| row.value.clone())
            .collect::<Vec<_>>();
        let menu = snooze::presets(
            &values,
            &now,
            &|time| crate::timestamp::short(time, cx),
            &|date| crate::timestamp::weekday(date, cx),
        );
        let mut seen_ids = HashSet::new();
        let mut seen_wakes = HashSet::new();
        v_flex().gap(px(6.)).text_sm().track_focus(&focus)
            .child(h_flex().child(div().flex_1().font_weight(FontWeight::MEDIUM).child("Snooze options"))
                .child(Button::new("reset-snooze-options").label("Reset snooze options").ghost().compact().on_click(cx.listener(|view,_,window,cx| {view.adopt(snooze::settings(None),window,cx);view.save(cx);}))))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Choices in every snooze menu, listed soonest first. Weekdays mean the next one after today."))
            .children(self.rows.iter().enumerate().map(|(index,row)| {
                let key=row.key;
                let resolved=snooze::resolve(&row.value,&now,&|time| crate::timestamp::short(time,cx),&|date| crate::timestamp::weekday(date,cx));
                let preview=match resolved {None=>"Passed for today".into(),Some(value) if seen_ids.contains(&value.id)=>"Duplicate of an option above".into(),Some(value) if seen_wakes.contains(&value.snoozed_until)=>"Same time as an option above".into(),Some(value)=>{seen_ids.insert(value.id);seen_wakes.insert(value.snoozed_until);format!("{}  {}",value.label,value.when_label)}};
                h_flex().id(("snooze-option",key)).role(accesskit::Role::Group).aria_label(format!("Snooze option {}",index+1)).gap_2()
                    .child(Select::new(&row.when_select).accessibility_label("When").h(px(32.)).w(px(128.)))
                    .child(match &row.value {
                        Setting::After{..}=>h_flex().gap_2()
                            .child(NumberInput::new(&row.amount).accessibility_label("Amount").step_labels("Decrease","Increase").h(px(32.)).w(px(128.)))
                            .child(Select::new(&row.unit_select).accessibility_label("Unit").h(px(32.)).w(px(112.))).into_any_element(),
                        Setting::At{..}=>h_flex().gap_2().child("at").child(Input::new(&row.time).aria_label("Time").h(px(32.)).w(px(128.))).into_any_element(),
                    })
                    .child(div().flex_1().min_w_0().truncate().text_xs().text_color(cx.theme().muted_foreground).child(preview))
                    .child(Button::new(("remove",key)).accessibility_label(format!("Remove snooze option {}",index+1)).icon(gpui_kit::assets::IconName::X).ghost().h(px(28.)).w(px(28.)).on_click(cx.listener(move |view,_,_,cx| {view.rows.retain(|row| row.key!=key);view.save(cx);})))
            }))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(if menu.is_empty() {"Menus offer only Custom…".into()} else {format!("Menu now: {}",menu.iter().map(|preset| preset.label.clone()).collect::<Vec<_>>().join(" · "))}))
            .child(Button::new("add-snooze-option").label("Add option").icon(gpui_kit::assets::IconName::Plus).ghost().compact().disabled(self.rows.len()>=8).on_click(cx.listener(|view,_,window,cx| {
                let used=view.rows.iter().map(|row| snooze::preset_id(&row.value)).collect::<HashSet<_>>();
                let candidate=snooze::settings(None).into_iter().chain([Setting::After{minutes:30},Setting::At{day:"saturday".into(),time:"09:00".into()},Setting::After{minutes:1440}]).find(|value| !used.contains(&snooze::preset_id(value))).unwrap_or(Setting::After{minutes:60});
                if view.rows.len()<8 {view.insert(candidate,window,cx);view.save(cx);}
            })))
            .when_some(self.error.clone(),|view,error| view.child(div().id("snooze-options-error").role(accesskit::Role::Alert).text_color(cx.theme().danger).child(error)))
    }
}
