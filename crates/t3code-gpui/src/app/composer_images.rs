use super::*;

impl Root {
    pub(super) fn load_timeline_images(&mut self, cx: &mut Context<Self>) {
        let Some(open) = &self.conversation.open else {
            return;
        };
        let Some(connection) = self.connection(&open.environment) else {
            return;
        };
        let state = open.watch.state();
        let sources: HashMap<String, Vec<Value>> = state
            .lock()
            .expect("thread")
            .items()
            .into_iter()
            .filter(|item| item["type"] == "user_message")
            .filter_map(|item| {
                Some((
                    item["id"].as_str()?.to_string(),
                    item["attachments"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|a| a["type"] == "image")
                        .cloned()
                        .collect(),
                ))
            })
            .collect();
        for (row, view) in self
            .conversation
            .rows
            .iter()
            .zip(&self.conversation.row_views)
        {
            let source = sources.get(&row.key).cloned().unwrap_or_default();
            view.update(cx, |view, cx| {
                view.load_images(source, connection.clone(), self.runtime.clone(), cx)
            });
        }
    }
}
