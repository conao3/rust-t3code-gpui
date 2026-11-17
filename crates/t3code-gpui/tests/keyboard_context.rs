use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    App, Context, FocusHandle, InputEvent as _, InteractiveElement as _, IntoElement, KeyBinding,
    KeyDownEvent, KeyboardInputAction, KeyboardInputContext, Keystroke, Modifiers, Render,
    TestAppContext, Window, div,
};

gpui::actions!(keyboard_context, [Prefix]);

type Observed = Rc<RefCell<Vec<(String, Option<String>)>>>;

struct Probe {
    focus: FocusHandle,
    observed: Observed,
}

fn down(key: &'static str) -> gpui::PlatformInput {
    KeyDownEvent {
        keystroke: Keystroke::parse(key).unwrap(),
        is_held: false,
        prefer_character_input: false,
    }
    .to_platform_input()
}

fn context(physical: &'static str) -> KeyboardInputContext {
    KeyboardInputContext {
        physical_key: physical.into(),
        text: None,
        unshifted_codepoint: 0,
        modifiers: Modifiers::default(),
        caps_lock: false,
        num_lock: false,
        alt_graph: false,
        is_modifier: false,
        action: KeyboardInputAction::Press,
    }
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let observed = self.observed.clone();
        div()
            .key_context("KeyboardContext")
            .track_focus(&self.focus)
            .on_key_down(move |event, window, cx| {
                observed.borrow_mut().push((
                    event.keystroke.key.clone(),
                    window
                        .current_keyboard_input_context()
                        .map(|context| context.physical_key.to_string()),
                ));
                if event.keystroke.key == "z" {
                    window.dispatch_event(down("c"), cx);
                    assert_eq!(
                        window
                            .current_keyboard_input_context()
                            .unwrap()
                            .physical_key
                            .as_ref(),
                        "KeyZ"
                    );
                }
            })
    }
}

#[gpui::test]
fn synthetic_and_replayed_input_do_not_inherit_physical_context(cx: &mut TestAppContext) {
    cx.update(|cx: &mut App| {
        cx.bind_keys([KeyBinding::new("a x", Prefix, Some("KeyboardContext"))])
    });
    let observed: Observed = Rc::new(RefCell::new(Vec::new()));
    let (_, cx) = cx.add_window_view(|window, cx| {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Probe {
            focus,
            observed: observed.clone(),
        }
    });
    cx.update(|window, cx| {
        window.simulate_keyboard_input(down("a"), context("KeyA"), cx);
        assert!(window.has_pending_keystrokes());
        assert!(window.current_keyboard_input_context().is_none());
        window.simulate_keyboard_input(down("b"), context("KeyB"), cx);
        assert!(!window.has_pending_keystrokes());
        assert!(window.current_keyboard_input_context().is_none());
        window.simulate_keyboard_input(down("z"), context("KeyZ"), cx);
        assert!(window.current_keyboard_input_context().is_none());
    });
    assert_eq!(
        *observed.borrow(),
        vec![
            ("a".into(), None),
            ("b".into(), Some("KeyB".into())),
            ("z".into(), Some("KeyZ".into())),
            ("c".into(), None)
        ]
    );
}
