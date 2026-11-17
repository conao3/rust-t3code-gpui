use super::*;
use gpui_kit::component::{
    IndexPath,
    select::{SearchableVec, SelectItem, SelectState},
};

#[derive(Clone)]
pub(super) struct Choice {
    value: &'static str,
    label: SharedString,
}

impl SelectItem for Choice {
    type Value = &'static str;
    fn title(&self) -> SharedString {
        self.label.clone()
    }
    fn value(&self) -> &Self::Value {
        &self.value
    }
}

pub(super) type State = SelectState<SearchableVec<Choice>>;

pub(super) fn items(values: &[(&'static str, &'static str)]) -> SearchableVec<Choice> {
    SearchableVec::new(
        values
            .iter()
            .map(|(value, label)| Choice {
                value,
                label: (*label).into(),
            })
            .collect::<Vec<_>>(),
    )
}

pub(super) fn state(
    values: &[(&'static str, &'static str)],
    selected: &str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<State> {
    let index = values
        .iter()
        .position(|(value, _)| *value == selected)
        .map(IndexPath::new);
    let values = items(values);
    cx.new(|cx| SelectState::new(values, index, window, cx))
}

pub(super) const WHEN: &[(&str, &str)] = &[
    ("after", "In"),
    ("today", "Today"),
    ("tomorrow", "Tomorrow"),
    ("monday", "Monday"),
    ("tuesday", "Tuesday"),
    ("wednesday", "Wednesday"),
    ("thursday", "Thursday"),
    ("friday", "Friday"),
    ("saturday", "Saturday"),
    ("sunday", "Sunday"),
];

pub(super) fn units(singular: bool) -> [(&'static str, &'static str); 3] {
    if singular {
        [("minutes", "minute"), ("hours", "hour"), ("days", "day")]
    } else {
        [("minutes", "minutes"), ("hours", "hours"), ("days", "days")]
    }
}
