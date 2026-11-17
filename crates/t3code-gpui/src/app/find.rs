use super::*;

impl Root {
    pub(super) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.conversation.open.is_none() {
            return;
        }
        self.conversation.find_open = true;
        self.conversation
            .find_input
            .update(cx, |state, cx| state.focus(window, cx));
        self.find_again(true, cx);
    }

    pub(super) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.conversation.find_open = false;
        self.conversation.find_hits.clear();
        self.mark_found(cx);
        self.conversation
            .composer
            .update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// Finds the query in the rows again, and with `jump` scrolls to the first hit.
    pub(super) fn find_again(&mut self, jump: bool, cx: &mut Context<Self>) {
        if !self.conversation.find_open {
            return;
        }
        let query = self.conversation.find_input.read(cx).value().to_string();
        self.conversation.find_hits = timeline::find_rows(&self.conversation.rows, &query);
        if jump {
            self.conversation.find_at = 0;
        }
        self.conversation.find_at = self
            .conversation
            .find_at
            .min(self.conversation.find_hits.len().saturating_sub(1));
        if jump {
            self.reveal_find();
        }
        self.mark_found(cx);
        cx.notify();
    }

    pub(super) fn step_find(&mut self, forward: bool, cx: &mut Context<Self>) {
        let n = self.conversation.find_hits.len();
        if n == 0 {
            return;
        }
        self.conversation.find_at = if forward {
            (self.conversation.find_at + 1) % n
        } else {
            (self.conversation.find_at + n - 1) % n
        };
        self.reveal_find();
        self.mark_found(cx);
        cx.notify();
    }

    fn reveal_find(&self) {
        if let Some(&item_ix) = self.conversation.find_hits.get(self.conversation.find_at) {
            self.conversation.list_state.scroll_to(ListOffset {
                item_ix,
                offset_in_item: px(0.),
            });
        }
    }

    fn mark_found(&self, cx: &mut Context<Self>) {
        let current = self
            .conversation
            .find_hits
            .get(self.conversation.find_at)
            .copied();
        for (i, view) in self.conversation.row_views.iter().enumerate() {
            view.update(cx, |view, cx| view.set_found(Some(i) == current, cx));
        }
    }
}
