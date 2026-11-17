use super::*;

impl Root {
    pub(super) fn toggle_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = !self.palette_open;
        if self.palette_open {
            self.palette.update(cx, |state, cx| {
                state.set_query("", window, cx);
                state.focus(window, cx);
            });
        } else {
            self.composer
                .update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    pub(super) fn render_palette(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let commands: [(&str, Box<dyn Action>); 17] = [
            ("Side chat", Box::new(OpenSideChat)),
            ("Split conversation right", Box::new(SplitConversation)),
            (
                "Split conversation down",
                Box::new(SplitConversationVertical),
            ),
            ("Close conversation pane", Box::new(CloseConversation)),
            ("Focus Next Workspace Area", Box::new(FocusNextWorkspace)),
            ("Reopen Closed Tab", Box::new(ReopenClosed)),
            ("New thread", Box::new(NewThread)),
            ("Next Done or waiting thread", Box::new(NextDone)),
            ("Settle the thread", Box::new(Settle)),
            ("Pin or unpin the thread", Box::new(TogglePin)),
            ("Find in thread", Box::new(Find)),
            ("Toggle the file explorer", Box::new(ToggleExplorer)),
            ("Pick a model", Box::new(ToggleModelPicker)),
            ("Cycle the runtime mode", Box::new(CycleRuntimeMode)),
            ("Toggle the sidebar", Box::new(ToggleSidebar)),
            ("Manage hosts", Box::new(ManageHosts)),
            ("Settings", Box::new(OpenSettings)),
        ];
        let threads = self.entries.iter().map(|entry| {
            CommandItem::new()
                .label(format!("{}  ·  {}", entry.title, entry.environment))
                .keywords([entry.project_title.clone()])
                .action(Box::new(OpenThreadAction {
                    environment: entry.environment.clone(),
                    thread_id: entry.thread_id.clone(),
                }))
        });
        let weak = cx.entity().downgrade();
        let closer = move |window: &mut Window, cx: &mut App| {
            let _ = weak.update(cx, |root, cx| {
                root.palette_open = false;
                root.composer
                    .update(cx, |state, cx| state.focus(window, cx));
                cx.notify();
            });
        };
        let close_on_cancel = closer.clone();
        div()
            .absolute()
            .top(px(80.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(640.))
                    .rounded_md()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .shadow_lg()
                    .child(
                        Command::new(&self.palette)
                            .placeholder("Search threads and commands")
                            .max_h(px(480.))
                            .items(
                                commands
                                    .into_iter()
                                    .map(|(label, action)| {
                                        CommandItem::new().label(label).action(action)
                                    })
                                    .chain(threads),
                            )
                            .on_confirm(move |_, window, cx| closer(window, cx))
                            .on_cancel(move |window, cx| close_on_cancel(window, cx)),
                    ),
            )
            .into_any_element()
    }
}
