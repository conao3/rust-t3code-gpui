use super::*;

pub(super) struct Minimap {
    pub items: Vec<(usize, String, String)>,
    pub list: ListState,
    pub active: Option<usize>,
    pub focus: FocusHandle,
    pub bounds: Bounds<Pixels>,
    preview_height: f32,
    _subscriptions: Vec<Subscription>,
}

fn height(count: usize, available: f32) -> f32 {
    ((count.saturating_sub(1) * 8).max(1) as f32).min((available - 236.).max(64.))
}

fn index_at(count: usize, top: f32, height: f32, pointer: f32) -> Option<usize> {
    if count == 0 || height <= 0. {
        return None;
    }
    Some(
        (((pointer - top) / height).clamp(0., 1.) * count.saturating_sub(1) as f32).round()
            as usize,
    )
}

impl Minimap {
    pub fn new(list: ListState, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let subscriptions = vec![
            cx.on_focus(&focus, window, |map, _, cx| {
                let current = map.list.logical_scroll_top().item_ix;
                map.active = map
                    .active
                    .or_else(|| map.items.iter().rposition(|(row, _, _)| *row <= current))
                    .or(Some(0));
                cx.notify();
            }),
            cx.on_blur(&focus, window, |map, _, cx| {
                map.active = None;
                cx.notify();
            }),
        ];
        Self {
            items: vec![],
            list,
            active: None,
            focus,
            bounds: Bounds::default(),
            preview_height: 0.,
            _subscriptions: subscriptions,
        }
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some((row, _, _)) = self.items.get(index) {
            self.active = Some(index);
            self.list.scroll_to(ListOffset {
                item_ix: *row,
                offset_in_item: px(0.),
            });
            cx.notify();
        }
    }
}

impl Render for Minimap {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.items.len() < 2 {
            return div().into_any_element();
        }
        let current_row = self.list.logical_scroll_top().item_ix;
        let current = self
            .items
            .iter()
            .rposition(|(row, _, _)| *row <= current_row)
            .unwrap_or(0);
        let count = self.items.len();
        let active = self.active;
        let label = active
            .and_then(|index| self.items.get(index))
            .map(|(_, text, _)| text.as_str())
            .unwrap_or("User message");
        let theme = cx.theme().colors;
        let measure = cx.entity().downgrade();
        let preview_measure = cx.entity().downgrade();
        let positions: Vec<_> = self.items.iter().map(|(row, _, _)| *row).collect();
        let list = self.list.clone();
        let rail_height = height(count, f32::from(self.bounds.size.height));
        let rail = div()
            .id("minimap-rail")
            .relative()
            .size_full()
            .cursor_pointer()
            .role(accesskit::Role::Button)
            .aria_label(format!("Jump to message: {label}"))
            .track_focus(&self.focus)
            .on_mouse_move(cx.listener(|map, event: &MouseMoveEvent, _, cx| {
                let h = height(map.items.len(), f32::from(map.bounds.size.height));
                let top =
                    f32::from(map.bounds.origin.y) + (f32::from(map.bounds.size.height) - h) / 2.;
                let active = index_at(map.items.len(), top, h, f32::from(event.position.y));
                if map.active != active {
                    map.active = active;
                    cx.notify();
                }
                if event.pressed_button == Some(MouseButton::Left)
                    && let Some(index) = active
                {
                    map.select(index, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|map, event: &MouseDownEvent, _, cx| {
                    let h = height(map.items.len(), f32::from(map.bounds.size.height));
                    let top = f32::from(map.bounds.origin.y)
                        + (f32::from(map.bounds.size.height) - h) / 2.;
                    if let Some(index) =
                        index_at(map.items.len(), top, h, f32::from(event.position.y))
                    {
                        map.select(index, cx);
                    }
                }),
            )
            .on_hover(cx.listener(|map, hovered, _, cx| {
                if !hovered {
                    map.active = None;
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(move |map, event: &KeyDownEvent, _, cx| {
                let index = map.active.unwrap_or(current);
                match event.keystroke.key.as_str() {
                    "up" => map.active = Some(index.saturating_sub(1)),
                    "down" => map.active = Some((index + 1).min(map.items.len() - 1)),
                    "home" => map.active = Some(0),
                    "end" => map.active = Some(map.items.len() - 1),
                    "enter" | "space" => map.select(index, cx),
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let _ = measure.update(cx, |map, _| map.bounds = bounds);
                        bounds
                    },
                    move |_, bounds, window, _| {
                        let h = height(count, f32::from(bounds.size.height));
                        let start = bounds.origin.y + (bounds.size.height - px(h)) / 2.;
                        let current = positions
                            .iter()
                            .rposition(|row| *row <= list.logical_scroll_top().item_ix)
                            .unwrap_or(0);
                        for index in 0..positions.len() {
                            let distance = active.map(|active| active.abs_diff(index));
                            let width = match distance {
                                Some(0) => 24.,
                                Some(1) => 16.,
                                Some(2) => 10.,
                                _ => 8.,
                            };
                            let in_view = index == current
                                || positions
                                    .get(index)
                                    .and_then(|row| list.bounds_for_item(*row))
                                    .is_some_and(|bounds| {
                                        bounds.top() < list.viewport_bounds().bottom()
                                            && bounds.bottom() > list.viewport_bounds().top()
                                    });
                            let color = if in_view {
                                theme.foreground.opacity(0.55)
                            } else {
                                theme.muted_foreground.opacity(if distance == Some(0) {
                                    0.6
                                } else {
                                    0.25
                                })
                            };
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(
                                        bounds.origin.x + px(12.),
                                        start + px(index as f32 / (count - 1) as f32 * h),
                                    ),
                                    size(px(width), px(2.)),
                                ),
                                color,
                            ));
                        }
                    },
                )
                .size_full(),
            );
        div()
            .relative()
            .w(px(48.))
            .h_full()
            .child(rail)
            .children([true, false].into_iter().map(|previous| {
                let index = if previous {
                    current.checked_sub(1)
                } else {
                    (current + 1 < count).then_some(current + 1)
                };
                div()
                    .absolute()
                    .left(px(4.))
                    .top(relative(0.5))
                    .mt(px(if previous {
                        -rail_height / 2. - 26.
                    } else {
                        rail_height / 2. + 6.
                    }))
                    .opacity(if active.is_some() { 1. } else { 0. })
                    .child(
                        Button::new(if previous {
                            "previous-turn"
                        } else {
                            "next-turn"
                        })
                        .icon(if previous {
                            IconName::ChevronUp
                        } else {
                            IconName::ChevronDown
                        })
                        .size(px(20.))
                        .ghost()
                        .accessibility_label(if previous {
                            "Previous turn"
                        } else {
                            "Next turn"
                        })
                        .disabled(index.is_none())
                        .on_click(cx.listener(move |map, _, _, cx| {
                            if let Some(index) = index {
                                map.select(index, cx);
                            }
                        })),
                    )
            }))
            .when_some(
                active.and_then(|index| self.items.get(index)),
                |el, (_, user, assistant)| {
                    el.child(
                        v_flex()
                            .absolute()
                            .left(px(44.))
                            .top(px((f32::from(self.bounds.size.height) - rail_height) / 2.
                                + active.unwrap_or(0) as f32 / (count - 1) as f32
                                    * rail_height
                                - self.preview_height / 2.))
                            .w(px(320.))
                            .p_3()
                            .gap_1()
                            .rounded_lg()
                            .bg(theme.popover)
                            .border_1()
                            .border_color(theme.border)
                            .shadow_lg()
                            .child(div().text_sm().truncate().child(user.clone()))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .max_h(px(60.))
                                    .overflow_hidden()
                                    .child(assistant.chars().take(1500).collect::<String>()),
                            )
                            .child(
                                canvas(
                                    move |bounds, _, cx| {
                                        let _ = preview_measure.update(cx, |map, cx| {
                                            let height = f32::from(bounds.size.height);
                                            if map.preview_height != height {
                                                map.preview_height = height;
                                                cx.notify();
                                            }
                                        });
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .size_full(),
                            ),
                    )
                },
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{height, index_at};

    #[test]
    fn reference_rail_geometry() {
        assert_eq!(height(2, 900.), 8.);
        assert_eq!(height(100, 900.), 664.);
        assert_eq!(height(100, 200.), 64.);
        for (pointer, expected) in [(0., 0), (20., 0), (24., 1), (36., 2), (100., 2)] {
            assert_eq!(index_at(3, 20., 16., pointer), Some(expected));
        }
        assert_eq!(index_at(0, 0., 20., 1.), None);
    }
}
