use super::*;

impl Root {
    pub(super) fn toggle_diff(&mut self, cx: &mut Context<Self>) {
        if self
            .file_panel
            .as_ref()
            .is_some_and(|p| p.path == DIFF_TITLE)
        {
            self.file_panel = None;
            cx.notify();
            return;
        }
        let Some((environment, cwd)) = self.composer_cwd() else {
            self.message = Some("Open a thread first.".into());
            cx.notify();
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let preview =
                on_runtime(&runtime, async move { connection.diff_preview(&cwd).await }).await;
            let preview = match preview {
                Ok(preview) => {
                    cx.background_spawn(async move {
                        let lines = completion::diff_lines(&preview);
                        let mut files = Vec::new();
                        if let Some(sources) = preview["sources"].as_array() {
                            for source in sources {
                                let patch = source["diff"].as_str().unwrap_or_default();
                                files.extend(
                                    DiffFile::parse_with(
                                        patch,
                                        DiffParseOptions::new().truncated_hunks(true),
                                    )
                                    .map_err(|error| {
                                        anyhow::anyhow!("Diff parse error: {error:?}")
                                    })?,
                                );
                            }
                        }
                        Ok((lines, files))
                    })
                    .await
                }
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |root, cx| {
                match preview {
                    Ok((lines, files)) => {
                        let text = lines.iter().map(|l| l.text().to_string()).collect();
                        let state = cx.new(|cx| {
                            DiffState::new(files, cx)
                                .with_mode(DiffMode::Unified)
                                .with_syntax_max_line_length(500)
                        });
                        root.file_panel = Some(FilePanel::new(
                            DIFF_TITLE.into(),
                            PanelBody::Diff(state),
                            None,
                            text,
                        ));
                        root.file_find_again(cx);
                    }
                    Err(error) => root.message = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
