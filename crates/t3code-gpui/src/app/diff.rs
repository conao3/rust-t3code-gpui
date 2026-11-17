use super::*;

impl Root {
    pub(super) fn toggle_diff(&mut self, cx: &mut Context<Self>) {
        if self.right_panel.open
            && !self.right_panel.launcher
            && self.right_panel.active.as_deref() == Some("diff")
        {
            self.close_panel_tab("diff", cx);
            return;
        }
        self.activate_panel("diff".into(), cx);
        self.refresh_diff(cx);
    }
}
