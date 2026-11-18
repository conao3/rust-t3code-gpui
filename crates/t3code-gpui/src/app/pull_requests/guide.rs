use super::*;

pub(super) fn parse_diffs(
    files: &[DiffFile],
    patch: &str,
    guide: &Value,
) -> HashMap<String, guide_excerpt::Excerpt> {
    let mut result = HashMap::new();
    for file in files {
        result.insert(
            format!("file:{}", file.path()),
            guide_excerpt::Excerpt {
                files: vec![file.clone()],
                shown: 0,
                total: 0,
            },
        );
    }
    for stop in guide["guide"]["stops"].as_array().into_iter().flatten() {
        for excerpt in stop["excerpts"].as_array().into_iter().flatten() {
            if let Some(parsed) = guide_excerpt::extract(
                patch,
                excerpt["path"].as_str().unwrap_or_default(),
                excerpt["side"] == "old",
                excerpt["start"].as_u64().unwrap_or_default() as usize,
                excerpt["end"].as_u64().unwrap_or_default() as usize,
            ) {
                result.insert(excerpt_key(excerpt), parsed);
            }
        }
    }
    result
}

fn excerpt_key(excerpt: &Value) -> String {
    format!(
        "excerpt:{}:{}:{}:{}",
        excerpt["path"], excerpt["side"], excerpt["start"], excerpt["end"]
    )
}

impl Root {
    fn open_guide_file(&mut self, path: &str, cx: &mut Context<Self>) {
        self.select_pr_tab("code", cx);
        if let Some(view) = self
            .pull_request_id()
            .and_then(|id| self.pr_state.views.get(&id))
            && let Some(diff) = &view.diff
        {
            diff.update(cx, |diff, cx| diff.scroll_to_file(path, cx));
        }
    }

    pub(super) fn render_guide_materials(
        &self,
        view: &Detail,
        stop: &Value,
        cx: &Context<Self>,
    ) -> AnyElement {
        let mut body = v_flex().gap_4().min_w_0();
        let excerpts = stop["excerpts"].as_array().cloned().unwrap_or_default();
        for (index, excerpt) in excerpts.iter().enumerate() {
            let path = excerpt["path"].as_str().unwrap_or_default().to_string();
            let mut figure = v_flex().gap_2().min_w_0();
            if let Some(caption) = excerpt["caption"]
                .as_str()
                .filter(|text| !text.trim().is_empty())
            {
                figure = figure.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(text!(caption.to_string())),
                );
            }
            let target = path.clone();
            let open = Button::new(("open-guide-excerpt", index))
                .ghost()
                .small()
                .accessibility_label(format!("Open {path} in Code"))
                .icon(gpui_kit::assets::IconName::ArrowUpRight)
                .on_click(cx.listener(move |root, _, _, cx| root.open_guide_file(&target, cx)));
            if let Some((diff, shown, total)) = view.guide_diffs.get(&excerpt_key(excerpt)) {
                figure = figure
                    .child(
                        h_flex()
                            .gap_2()
                            .child(text!(path.clone()))
                            .child(div().flex_1())
                            .child(text!(format!(
                                "{}showing {shown} of {total} diff lines",
                                if excerpt["side"] == "old" {
                                    "removed code, "
                                } else {
                                    ""
                                }
                            )))
                            .child(open),
                    )
                    .child(
                        div()
                            .h(px((*shown as f32 * 22. + 40.).clamp(80., 360.)))
                            .child(Diff::new(diff).header_visible(false).size_full()),
                    );
            } else if view.diff_loading {
                figure = figure.child(text!(format!("Loading {path}...")));
            } else {
                figure = figure.child(
                    h_flex()
                        .gap_3()
                        .child(text!(format!(
                            "{path} lines {}-{} are not in the loaded diff.",
                            excerpt["start"], excerpt["end"]
                        )))
                        .child(
                            open.label("Open in Code")
                                .accessibility_label("Open in Code"),
                        ),
                );
            }
            body = body.child(figure);
        }
        if let Some(notes) = stop["notes"].as_array().filter(|notes| !notes.is_empty()) {
            body = body.child(
                h_flex()
                    .justify_between()
                    .child(text!("Notes on this stop"))
                    .child(text!(format!(
                        "{} {}",
                        notes.len(),
                        if notes.len() == 1 { "note" } else { "notes" }
                    ))),
            );
            for note in notes {
                let path = note["path"]
                    .as_str()
                    .unwrap_or_default()
                    .rsplit('/')
                    .next()
                    .unwrap_or_default();
                body = body.child(
                    v_flex()
                        .gap_1()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(text!(
                                    note["title"].as_str().unwrap_or_default().to_string()
                                ))
                                .child(text!(format!("{path}:{}", note["line"]))),
                        )
                        .child(text!(note["body"].as_str().unwrap_or_default().to_string())),
                );
            }
        }
        let files = stop["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|file| {
                !excerpts
                    .iter()
                    .any(|excerpt| excerpt["path"] == file["path"])
            })
            .collect::<Vec<_>>();
        if !files.is_empty() {
            body = body.child(text!(if excerpts.is_empty() {
                "Files in this stop"
            } else {
                "Also in this stop"
            }));
        }
        for (index, file) in files.into_iter().enumerate() {
            let path = file["path"].as_str().unwrap_or_default().to_string();
            let key = format!("{}:{path}", view.guide_position);
            let expanded = view.guide_open_files.contains(&key);
            let diff = view.guide_diffs.get(&format!("file:{path}"));
            let mut label = path.clone();
            if let Some(note) = file["note"].as_str().filter(|note| !note.trim().is_empty()) {
                label.push(' ');
                label.push_str(note);
            }
            if let Some((diff, _, _)) = diff {
                let file = &diff.read(cx).files()[0];
                label.push_str(&format!(" +{} -{}", file.additions(), file.deletions()));
            }
            body = body.child(
                Button::new(("guide-file", index))
                    .ghost()
                    .w_full()
                    .justify_start()
                    .label(label)
                    .icon(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .aria_expanded(expanded)
                    .on_click(cx.listener(move |root, _, _, cx| {
                        if let Some(view) = root
                            .pull_request_id()
                            .and_then(|id| root.pr_state.views.get_mut(&id))
                            && !view.guide_open_files.remove(&key)
                        {
                            view.guide_open_files.insert(key.clone());
                        }
                        root.panel_revision += 1;
                        cx.notify();
                    })),
            );
            if expanded {
                body = body.child(if let Some((diff, _, _)) = diff {
                    div()
                        .h(px(240.))
                        .child(Diff::new(diff).header_visible(false).size_full())
                        .into_any_element()
                } else {
                    div()
                        .text_xs()
                        .child(text!(if view.diff_loading {
                            "Loading the diff..."
                        } else {
                            "This file is not in the loaded diff."
                        }))
                        .into_any_element()
                });
            }
        }
        if let Some(questions) = stop["questions"]
            .as_array()
            .filter(|items| !items.is_empty())
        {
            body = body.child(text!("Questions to consider"));
            for question in questions {
                body = body.child(text!(question.as_str().unwrap_or_default().to_string()));
            }
        }
        body.into_any_element()
    }
}
