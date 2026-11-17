use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, StyleRefinement,
    Styled as _, TestAppContext, Window, div, px,
};

struct Pane {
    value: Rc<Cell<usize>>,
    painted: Rc<RefCell<Vec<usize>>>,
}

impl Render for Pane {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let value = self.value.get();
        self.painted.borrow_mut().push(value);
        div().child(value.to_string())
    }
}

struct Parent {
    value: Rc<Cell<usize>>,
    stamp: Option<usize>,
    pane: Entity<Pane>,
}

impl Render for Parent {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let value = self.value.get();
        if self.stamp != Some(value) {
            self.stamp = Some(value);
            window.mark_view_dirty(self.pane.entity_id());
        }
        div().size_full().child(
            self.pane
                .clone()
                .cached(StyleRefinement::default().w(px(100.)).h(px(40.))),
        )
    }
}

#[gpui::test]
fn a_parent_change_updates_a_cached_pane_in_the_same_frame_and_unchanged_frames_reuse_it(
    cx: &mut TestAppContext,
) {
    let value = Rc::new(Cell::new(0));
    let painted = Rc::new(RefCell::new(Vec::new()));
    let (parent, cx) = cx.add_window_view(|_, cx| Parent {
        value: value.clone(),
        stamp: None,
        pane: cx.new(|_| Pane {
            value: value.clone(),
            painted: painted.clone(),
        }),
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(painted.borrow().last(), Some(&0));
    let initial = painted.borrow().len();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(painted.borrow().len(), initial);
    cx.update(|window, cx| {
        parent.update(cx, |_, cx| {
            value.set(1);
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert_eq!(painted.borrow().last(), Some(&1));
    });
    let changed = painted.borrow().len();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(painted.borrow().len(), changed);
    cx.update(|window, cx| {
        parent.update(cx, |_, cx| {
            value.set(0);
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert_eq!(painted.borrow().last(), Some(&0));
    });
}
