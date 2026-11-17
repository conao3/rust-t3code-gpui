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
        let Some((environment, cwd)) = self.composer_cwd() else {
            self.message = Some("Open a thread first.".into());
            cx.notify();
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let runtime = self.runtime.clone();
        let target = self.current();
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
                if root.current() != target || root.right_panel.active.as_deref() != Some("diff") {
                    return;
                }
                match preview {
                    Ok((lines, files)) => {
                        let text = lines.iter().map(|l| l.text().to_string()).collect();
                        let state = cx.new(|cx| {
                            DiffState::new(files, cx)
                                .with_mode(DiffMode::Unified)
                                .with_syntax_max_line_length(500)
                        });
                        root.install_panel_file(
                            FilePanel::new(DIFF_TITLE.into(), PanelBody::Diff(state), None, text),
                            cx,
                        );
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
