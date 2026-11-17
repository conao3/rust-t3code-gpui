use gpui::{
    Context, FocusHandle, Focusable as _, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, TestAppContext,
    WeakEntity, Window, actions, div, point, px,
};
use gpui_kit::component::{
    button::Button,
    menu::{DropdownMenu as _, PopupMenu},
};
use std::{cell::RefCell, rc::Rc};
actions!(pr_menu_test, [CopyText]);
struct FocusedMenuRoot {
    focus: FocusHandle,
    menu: Rc<RefCell<Option<WeakEntity<PopupMenu>>>>,
}

impl Render for FocusedMenuRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let menu = self.menu.clone();
        div()
            .id("focused-root")
            .role(gpui::accesskit::Role::Group)
            .track_focus(&self.focus)
            .size_full()
            .child(
                Button::new("trigger")
                    .label("Sort")
                    .w(px(100.))
                    .h(px(30.))
                    .dropdown_menu(move |builder, _, cx| {
                        *menu.borrow_mut() = Some(cx.weak_entity());
                        builder.menu("Recently updated", Box::new(CopyText))
                    }),
            )
    }
}

#[gpui::test]
fn opening_from_focused_root_transfers_focus_after_prepaint(cx: &mut TestAppContext) {
    cx.update(gpui_kit::component::init);
    let menu = Rc::new(RefCell::new(None::<WeakEntity<PopupMenu>>));
    let (_, cx) = cx.add_window_view(|window, cx| {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        FocusedMenuRoot {
            focus,
            menu: menu.clone(),
        }
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_mouse_down(
        point(px(10.), px(10.)),
        MouseButton::Left,
        Default::default(),
    );
    cx.run_until_parked();
    let menu = menu.borrow().as_ref().unwrap().upgrade().unwrap();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(menu.focus_handle(cx).is_focused(window));
    });
}
