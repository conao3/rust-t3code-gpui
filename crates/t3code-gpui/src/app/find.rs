use super::*;

impl Root {
    pub(super) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open.is_none() {
            return;
        }
        self.find_open = true;
        self.find_input
            .update(cx, |state, cx| state.focus(window, cx));
        self.find_again(true, cx);
    }

    pub(super) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_open = false;
        self.find_hits.clear();
        self.mark_found(cx);
        self.composer
            .update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// Finds the query in the rows again, and with `jump` scrolls to the first hit.
    pub(super) fn find_again(&mut self, jump: bool, cx: &mut Context<Self>) {
        if !self.find_open {
            return;
        }
        let query = self.find_input.read(cx).value().to_string();
        self.find_hits = timeline::find_rows(&self.rows, &query);
        if jump {
            self.find_at = 0;
        }
        self.find_at = self.find_at.min(self.find_hits.len().saturating_sub(1));
        if jump {
            self.reveal_find();
        }
        self.mark_found(cx);
        cx.notify();
    }

    pub(super) fn step_find(&mut self, forward: bool, cx: &mut Context<Self>) {
        let n = self.find_hits.len();
        if n == 0 {
            return;
        }
        self.find_at = if forward {
            (self.find_at + 1) % n
        } else {
            (self.find_at + n - 1) % n
        };
        self.reveal_find();
        self.mark_found(cx);
        cx.notify();
    }

    fn reveal_find(&self) {
        if let Some(&item_ix) = self.find_hits.get(self.find_at) {
            self.list_state.scroll_to(ListOffset {
                item_ix,
                offset_in_item: px(0.),
            });
        }
    }

    fn mark_found(&self, cx: &mut Context<Self>) {
        let current = self.find_hits.get(self.find_at).copied();
        for (i, view) in self.row_views.iter().enumerate() {
            view.update(cx, |view, cx| view.set_found(Some(i) == current, cx));
        }
    }
}
