use super::*;
use gpui_kit::component::notification::Notification;

impl Root {
    pub(super) fn background_started(
        &self,
        environment: String,
        thread_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = cx.entity().downgrade();
        window.push_notification(
            Notification::success("Started in background")
                .placement(Anchor::TopRight)
                .action(move |_, _, cx| {
                    let root = root.clone();
                    let environment = environment.clone();
                    let thread_id = thread_id.clone();
                    Button::new("open-background-thread")
                        .label("Open")
                        .primary()
                        .compact()
                        .on_click(cx.listener(move |note, _, window, cx| {
                            let _ = root.update(cx, |root, cx| {
                                root.open_thread(environment.clone(), thread_id.clone(), cx)
                            });
                            note.dismiss(window, cx);
                        }))
                })
                .autohide(true),
            cx,
        );
    }
}
