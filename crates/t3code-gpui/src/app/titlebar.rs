use super::*;

impl Root {
    fn window_drag_area(&self, id: &'static str, cx: &Context<Self>) -> Stateful<Div> {
        div()
            .id(id)
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|root, event: &MouseDownEvent, window, cx| {
                    root.window_drag_pending = event.click_count == 1;
                    if event.click_count == 2 {
                        root.window_drag_pending = false;
                        window.zoom_window();
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(|root, event: &MouseMoveEvent, window, _| {
                if root.window_drag_pending && event.pressed_button == Some(MouseButton::Left) {
                    root.window_drag_pending = false;
                    window.start_window_move();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|root, _, _, _| root.window_drag_pending = false),
            )
            .on_mouse_down(MouseButton::Right, |event, window, _| {
                window.show_window_menu(event.position)
            })
    }

    pub(super) fn render_titlebar(
        &self,
        window: &Window,
        target: Option<String>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let content = if self.pr_state.page {
            self.window_drag_area("pr-page-window-drag", cx)
                .flex_1()
                .h_full()
                .px_5()
                .flex()
                .items_center()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(text!("Pull requests"))
                .into_any_element()
        } else if self.draft.is_some() || self.open.is_none() {
            self.window_drag_area("empty-window-drag", cx)
                .role(accesskit::Role::Navigation)
                .aria_label("Thread breadcrumb")
                .flex_1()
                .min_w_0()
                .h_full()
                .px_4()
                .flex()
                .items_center()
                .gap_3()
                .text_sm()
                .children(target.map(|label| {
                    h_flex()
                        .gap_3()
                        .text_color(theme.muted_foreground)
                        .child(
                            Button::new("draft-breadcrumb-project")
                                .ghost()
                                .h(px(28.))
                                .px_0()
                                .accessibility_label(format!("New thread in {label}"))
                                .child(settings_controls::text(label))
                                .on_click(
                                    cx.listener(|root, _, window, cx| root.new_thread(window, cx)),
                                ),
                        )
                        .child("/")
                }))
                .child(div().flex_1().min_w_0().truncate().child(
                    if self.conversation.draft.is_some() {
                        "New thread"
                    } else {
                        APP_NAME
                    },
                ))
                .into_any_element()
        } else {
            self.render_thread_header(cx)
        };
        let custom_controls = !cfg!(target_os = "macos")
            && matches!(window.window_decorations(), Decorations::Client { .. });
        h_flex()
            .h(px(40.))
            .when(cfg!(target_os = "macos"), |bar| bar.pl(px(80.)))
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(theme.border)
            .when(!self.sidebar_hidden, |bar| {
                bar.child(
                    div()
                        .w(px(self.settings.sidebar_width))
                        .flex_none()
                        .bg(theme.sidebar)
                        .text_color(theme.sidebar_foreground)
                        .child(self.render_sidebar_title(cx)),
                )
            })
            .when(self.sidebar_hidden, |bar| {
                bar.child(
                    Button::new("show-sidebar")
                        .icon(IconName::PanelLeftOpen)
                        .ghost()
                        .size(px(24.))
                        .tooltip("Toggle Sidebar")
                        .accessibility_label("Toggle main sidebar")
                        .on_click(cx.listener(|root, _, _, cx| {
                            root.sidebar_hidden = false;
                            cx.notify();
                        })),
                )
            })
            .when(
                !self.right_panel.open || !self.right_panel.maximized || self.pr_state.page,
                |bar| bar.child(content),
            )
            .when(
                !self.pr_state.page && self.workspace_watch.is_some(),
                |bar| {
                    bar.child(
                        Button::new("thread-details-toggle")
                            .icon(gpui_kit::assets::IconName::PanelRight)
                            .ghost()
                            .size(px(28.))
                            .accessibility_label("Toggle thread details panel")
                            .tooltip("Toggle thread details")
                            .on_click(cx.listener(|_, _, window, cx| {
                                window.dispatch_action(Box::new(ToggleThreadDetails), cx)
                            })),
                    )
                },
            )
            .when(self.pr_state.page, |bar| {
                bar.child(
                    Button::new("pr-panel-toggle")
                        .icon(gpui_kit::assets::IconName::PanelRight)
                        .ghost()
                        .size(px(28.))
                        .accessibility_label("Toggle right panel")
                        .disabled(self.pr_state.selected.is_none())
                        .on_click(cx.listener(|root, _, _, cx| {
                            root.pr_state.page_panel_open = !root.pr_state.page_panel_open;
                            root.panel_revision += 1;
                            cx.notify();
                        })),
                )
            })
            .when(self.right_panel.open && !self.pr_state.page, |bar| {
                bar.child(
                    div()
                        .flex_none()
                        .w(px((self
                            .panel_width(f32::from(window.viewport_size().width))
                            - if custom_controls { 104. } else { 0. })
                        .max(256.)))
                        .child(self.render_panel_tabs(cx)),
                )
            })
            .when(custom_controls, |bar| {
                bar.child(
                    h_flex()
                        .gap_1()
                        .px_2()
                        .child(
                            Button::new("window-minimize")
                                .icon(IconName::Minus)
                                .size(px(24.))
                                .rounded_full()
                                .ghost()
                                .bg(theme.foreground.opacity(0.07))
                                .tooltip("Minimize")
                                .accessibility_label("Minimize window")
                                .on_click(|_, window, _| window.minimize_window()),
                        )
                        .child(
                            Button::new("window-maximize")
                                .icon(if window.is_maximized() {
                                    IconName::WindowRestore
                                } else {
                                    IconName::WindowMaximize
                                })
                                .size(px(24.))
                                .rounded_full()
                                .ghost()
                                .bg(theme.foreground.opacity(0.07))
                                .tooltip(if window.is_maximized() {
                                    "Restore"
                                } else {
                                    "Maximize"
                                })
                                .accessibility_label("Maximize or restore window")
                                .on_click(|_, window, _| {
                                    window.zoom_window();
                                    window.refresh();
                                }),
                        )
                        .child(
                            Button::new("window-close")
                                .icon(IconName::WindowClose)
                                .size(px(24.))
                                .rounded_full()
                                .ghost()
                                .bg(theme.foreground.opacity(0.07))
                                .tooltip("Close")
                                .accessibility_label("Close window")
                                .on_click(|_, window, _| window.remove_window()),
                        ),
                )
            })
            .into_any_element()
    }

    fn render_sidebar_title(&self, cx: &Context<Self>) -> AnyElement {
        h_flex()
            .h(px(40.))
            .px_2()
            .gap_1()
            .child(
                Button::new("collapse-sidebar")
                    .icon(gpui_kit::assets::IconName::PanelLeftClose)
                    .ghost()
                    .size(px(24.))
                    .tooltip("Toggle Sidebar")
                    .accessibility_label("Toggle main sidebar")
                    .on_click(cx.listener(|root, _, _, cx| {
                        root.sidebar_hidden = true;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("sidebar-home")
                    .role(accesskit::Role::Link)
                    .accessibility_label("T3 Code home")
                    .ghost()
                    .h(px(28.))
                    .px_1()
                    .on_click(cx.listener(|root, _, window, cx| root.new_thread(window, cx)))
                    .ml_4()
                    .child(
                        h_flex().gap(px(3.)).child("T3 Code").child(
                            div()
                                .relative()
                                .top(px(-4.))
                                .text_size(px(10.))
                                .line_height(px(12.))
                                .child("GPUI"),
                        ),
                    ),
            )
            .child(
                Button::new("history-back")
                    .icon(IconName::ArrowLeft)
                    .ghost()
                    .size(px(24.))
                    .disabled(!self.history.can_back())
                    .tooltip("Go back")
                    .accessibility_label("Go back")
                    .on_click(cx.listener(|_, _, window, cx| {
                        window.dispatch_action(Box::new(HistoryBack), cx)
                    })),
            )
            .child(
                Button::new("history-forward")
                    .icon(IconName::ArrowRight)
                    .ghost()
                    .size(px(24.))
                    .disabled(!self.history.can_forward())
                    .tooltip("Go forward")
                    .accessibility_label("Go forward")
                    .on_click(cx.listener(|_, _, window, cx| {
                        window.dispatch_action(Box::new(HistoryForward), cx)
                    })),
            )
            .into_any_element()
    }

    fn render_thread_header(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let entry = self.open_entry();
        let project_title = entry.map(|e| e.project_title.clone()).unwrap_or_default();
        let title = entry.map(|e| e.title.clone()).unwrap_or_default();
        let status = entry.map(|e| e.status).unwrap_or(ThreadStatus::Idle);
        let menu_root = cx.entity().downgrade();
        h_flex()
            .flex_1()
            .min_w_0()
            .h(px(40.))
            .px_4()
            .gap_2()
            .child(
                self.window_drag_area("thread-window-drag", cx)
                    .role(accesskit::Role::Navigation)
                    .aria_label("Thread breadcrumb")
                    .flex()
                    .items_center()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .text_sm()
                    .child(
                        div().text_color(theme.muted_foreground).child(
                            Button::new("breadcrumb-project")
                                .ghost()
                                .h(px(28.))
                                .px_0()
                                .accessibility_label(format!("New thread in {project_title}"))
                                .child(settings_controls::text(project_title))
                                .on_click(
                                    cx.listener(|root, _, window, cx| root.new_thread(window, cx)),
                                ),
                        ),
                    )
                    .child(div().text_color(theme.muted_foreground).child("/"))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(settings_controls::text(title.clone())),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(status.label()),
            )
            .child(
                Button::new("files")
                    .icon(gpui_kit::assets::IconName::FolderOpen)
                    .size(px(24.))
                    .tooltip("Toggle right panel")
                    .accessibility_label("Toggle right panel")
                    .ghost()
                    .compact()
                    .selected(self.right_panel.open)
                    .on_click(
                        cx.listener(|root, _, window, cx| root.toggle_right_panel(window, cx)),
                    ),
            )
            .child(
                Button::new("diff")
                    .icon(gpui_kit::assets::IconName::PanelRight)
                    .size(px(24.))
                    .tooltip("Changes")
                    .accessibility_label("Changes")
                    .ghost()
                    .compact()
                    .on_click(cx.listener(|root, _, _, cx| root.toggle_diff(cx))),
            )
            .child(
                Button::new("thread-actions")
                    .icon(gpui_kit::assets::IconName::Ellipsis)
                    .ghost()
                    .size(px(24.))
                    .tooltip("Thread actions")
                    .accessibility_label(format!("Thread actions for {title}"))
                    .dropdown_menu(move |menu, _, _| {
                        menu.menu(
                            if status == ThreadStatus::Settled {
                                "Unsettle"
                            } else {
                                "Settle"
                            },
                            Box::new(Settle),
                        )
                        .item(
                            gpui_kit::component::menu::PopupMenuItem::new("Open in browser")
                                .on_click({
                                    let root = menu_root.clone();
                                    move |_, _, cx| {
                                        let _ =
                                            root.update(cx, |root, cx| root.open_in_browser(cx));
                                    }
                                }),
                        )
                    }),
            )
            .into_any_element()
    }
}
