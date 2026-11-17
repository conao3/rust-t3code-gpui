use super::*;

pub(super) struct Notice {
    title: &'static str,
    message: String,
    copied: bool,
    hovered: bool,
    remaining: Duration,
}

struct Dismiss;
impl EventEmitter<Dismiss> for Notice {}

impl Render for Notice {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id("sidebar-notice")
            .role(accesskit::Role::Dialog)
            .aria_label(self.title)
            .relative()
            .w_full()
            .py_3()
            .pl(px(14.))
            .pr_6()
            .gap(px(6.))
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .rounded(px(8.))
            .shadow_lg()
            .text_size(px(14.))
            .line_height(px(20.))
            .on_hover(cx.listener(|notice, hovered, _, _| notice.hovered = *hovered))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Icon::new(gpui_kit::assets::IconName::CircleAlert)
                                    .size(px(16.))
                                    .text_color(cx.theme().danger),
                            )
                            .child(
                                div()
                                    .id("title")
                                    .role(accesskit::Role::Heading)
                                    .aria_label(self.title)
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(self.title),
                            ),
                    )
                    .child(
                        div()
                            .id("description")
                            .role(accesskit::Role::Label)
                            .aria_value(self.message.clone())
                            .text_color(cx.theme().muted_foreground)
                            .child(self.message.replace('-', "-\u{200b}")),
                    ),
            )
            .child(
                Button::new("copy-error")
                    .icon(if self.copied {
                        IconName::Check
                    } else {
                        IconName::Copy
                    })
                    .accessibility_label(if self.copied {
                        "Copied error"
                    } else {
                        "Copy error"
                    })
                    .ghost()
                    .size(px(24.))
                    .on_click(cx.listener(|notice, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(notice.message.clone()));
                        notice.copied = true;
                        cx.notify();
                        cx.spawn(async move |this, cx| {
                            cx.background_executor().timer(Duration::from_secs(2)).await;
                            let _ = this.update(cx, |notice, cx| {
                                notice.copied = false;
                                cx.notify();
                            });
                        })
                        .detach();
                    })),
            )
            .child(
                Button::new("dismiss-notification")
                    .icon(IconName::Close)
                    .accessibility_label("Dismiss notification")
                    .ghost()
                    .size(px(24.))
                    .absolute()
                    .top(px(-6.))
                    .right(px(-6.))
                    .rounded_full()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().popover)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(Dismiss))),
            )
    }
}

impl Root {
    pub(super) fn show_sidebar_error(
        &mut self,
        title: &'static str,
        message: String,
        cx: &mut Context<Self>,
    ) {
        let notice = cx.new(|_| Notice {
            title,
            message,
            copied: false,
            hovered: false,
            remaining: Duration::from_secs(5),
        });
        cx.subscribe(&notice, |root, notice, _: &Dismiss, cx| {
            root.sidebar_notices.retain(|entry| entry != &notice);
            cx.notify();
        })
        .detach();
        notice.update(cx, |_, cx| {
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(100))
                        .await;
                    let expired = this.update(cx, |notice, cx| {
                        if !notice.hovered {
                            notice.remaining =
                                notice.remaining.saturating_sub(Duration::from_millis(100));
                        }
                        if notice.remaining.is_zero() {
                            cx.emit(Dismiss);
                        }
                        notice.remaining.is_zero()
                    });
                    if expired.unwrap_or(true) {
                        break;
                    }
                }
            })
            .detach();
        });
        self.sidebar_notices.push(notice);
        cx.notify();
    }

    pub(super) fn render_sidebar_notices(&self, cx: &Context<Self>) -> AnyElement {
        v_flex()
            .id("sidebar-notifications")
            .role(accesskit::Role::Region)
            .aria_label("Notifications")
            .absolute()
            .top(px(72.))
            .right(px(32.))
            .w(px(360.))
            .gap_3()
            .children(
                self.sidebar_notices
                    .iter()
                    .rev()
                    .map(|notice| notice.clone().into_any_element()),
            )
            .text_color(cx.theme().foreground)
            .into_any_element()
    }
}
