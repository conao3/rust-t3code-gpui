use super::*;

pub(super) struct Surface {
    child: AnyElement,
    hidden: bool,
}

impl Surface {
    pub(super) fn new(child: AnyElement) -> Self {
        Self {
            child,
            hidden: false,
        }
    }
}

impl IntoElement for Surface {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Surface {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some("conversation-surface".into())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        self.hidden = window.has_active_dialog(cx);
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }

    fn a11y_role(&self) -> Option<accesskit::Role> {
        Some(accesskit::Role::GenericContainer)
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        if self.hidden {
            node.set_hidden();
        }
    }
}
