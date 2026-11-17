use super::*;
use gpui_kit::component::button::ButtonCustomVariant;

impl Root {
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
                    .child(div().truncate().child(section.label()))
                    .child(div().flex_1())
                    .child(
                        div()
                            .font_weight(FontWeight::NORMAL)
                            .font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])))
                            .child(count.to_string()),
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
