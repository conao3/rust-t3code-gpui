use gpui_kit::assets::IconName;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon,
    button::{Button, ButtonVariants as _},
    h_flex,
    text::{RenderedText, TextView, TextViewState, TextViewStyle},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::timeline::{AssistantMetadata, Draft, Pending, Question, Row, RowKind};

const DETAIL_LINES: usize = 400;

#[derive(Clone)]
enum BodySource {
    Markdown(RenderedText),
    Plain(SharedString),
}

#[derive(Clone)]
struct AccessibleBody {
    source: BodySource,
    character_lengths: Arc<OnceLock<Vec<u8>>>,
}

impl AccessibleBody {
    fn text(&self) -> &str {
        match &self.source {
            BodySource::Markdown(snapshot) => snapshot.as_str(),
            BodySource::Plain(text) => text.as_ref(),
        }
    }

    fn append(&self, builder: &mut A11ySubtreeBuilder) {
        let mut run = accesskit::Node::new(accesskit::Role::TextRun);
        run.set_value(self.text());
        run.set_character_lengths(
            self.character_lengths
                .get_or_init(|| {
                    self.text()
                        .chars()
                        .map(|character| character.len_utf8() as u8)
                        .collect()
                })
                .clone(),
        );
        let id = builder.synthetic_node_id("body");
        builder.push_child(id, run);
    }
}

#[derive(Clone)]
pub enum RowEvent {
    CopyMessage {
        message_id: String,
    },
    MessageHover {
        message_id: String,
        hovered: bool,
    },
    ForkResponse {
        source_thread_id: String,
        run_id: String,
    },
    Respond {
        request_id: String,
        decision: String,
    },
    Submit {
        request_id: String,
    },
    Implement {
        plan_id: String,
        markdown: String,
    },
    OpenChild {
        thread_id: String,
    },
}

pub struct RowView {
    row: Row,
    summary: String,
    markdown: Option<Entity<TextViewState>>,
    accessible_body: Option<AccessibleBody>,
    _markdown_observer: Option<Subscription>,
    found: bool,
    expanded: bool,
    drafts: HashMap<String, Draft>,
    responding: bool,
    message_hovered: bool,
    copied: bool,
    forking: bool,
    meta_focus: FocusHandle,
    children: Vec<Entity<RowView>>,
    _timer: Option<Task<()>>,
}

impl EventEmitter<RowEvent> for RowView {}

impl RowView {
    pub fn new(row: Row, cx: &mut Context<Self>) -> Self {
        let timer = (row.kind == RowKind::Working).then(|| {
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(1))
                        .await;
                    if this
                        .update(cx, |view, cx| {
                            if view.row.kind != RowKind::Working || !view.row.live {
                                return false;
                            }
                            cx.notify();
                            true
                        })
                        .ok()
                        != Some(true)
                    {
                        break;
                    }
                }
            })
        });
        let markdown = (row.kind == RowKind::Assistant)
            .then(|| cx.new(|cx| TextViewState::markdown(&row.text, cx)));
        let summary = if row.kind == RowKind::Reasoning {
            crate::timeline::thought_preview(&row.text)
        } else {
            row.text.clone()
        };
        let observer = markdown.as_ref().map(|markdown| {
            cx.observe(markdown, |view, _, cx| {
                if view.refresh_accessible_body(cx) {
                    cx.notify();
                }
            })
        });
        let mut view = Self {
            row,
            summary,
            markdown,
            accessible_body: None,
            _markdown_observer: observer,
            found: false,
            expanded: false,
            drafts: HashMap::new(),
            responding: false,
            message_hovered: false,
            copied: false,
            forking: false,
            meta_focus: cx.focus_handle(),
            children: Vec::new(),
            _timer: timer,
        };
        view.refresh_accessible_body(cx);
        view
    }

    fn refresh_accessible_body(&mut self, cx: &Context<Self>) -> bool {
        let source = match self.row.kind {
            RowKind::Assistant => {
                let Some(markdown) = &self.markdown else {
                    return false;
                };
                let snapshot = markdown.read(cx).rendered_text();
                if self.accessible_body.as_ref().is_some_and(|body| {
                    matches!(&body.source, BodySource::Markdown(previous) if previous == &snapshot)
                }) {
                    return false;
                }
                BodySource::Markdown(snapshot)
            }
            RowKind::User => {
                if self.accessible_body.as_ref().is_some_and(|body| {
                    matches!(&body.source, BodySource::Plain(previous) if previous.as_ref() == self.row.text)
                }) {
                    return false;
                }
                BodySource::Plain(self.row.text.clone().into())
            }
            _ => return self.accessible_body.take().is_some(),
        };
        self.accessible_body = Some(AccessibleBody {
            source,
            character_lengths: Arc::new(OnceLock::new()),
        });
        true
    }

    pub fn set_message_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        if self.message_hovered != hovered {
            self.message_hovered = hovered;
            cx.notify();
        }
    }

    pub fn set_forking(&mut self, forking: bool, cx: &mut Context<Self>) {
        if self.forking != forking {
            self.forking = forking;
            cx.notify();
        }
    }

    pub fn copied(&mut self, cx: &mut Context<Self>) {
        self.copied = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(1000))
                .await;
            let _ = this.update(cx, |view, cx| {
                view.copied = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn render_assistant_meta(
        &self,
        metadata: &AssistantMetadata,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let (timestamp, tooltip) = crate::timestamp::display(&metadata.updated_at, cx);
        let id = metadata.message_id.clone();
        let copy = Button::new("copy-message")
            .accessibility_label("Copy message")
            .icon(
                Icon::new(if self.copied {
                    IconName::Check
                } else {
                    IconName::Copy
                })
                .size(px(12.)),
            )
            .ghost()
            .h(px(24.))
            .w(px(28.))
            .p_0()
            .rounded(px(6.))
            .disabled(self.copied)
            .tooltip("Copy message")
            .on_click(cx.listener(move |_, _, _, cx| {
                cx.emit(RowEvent::CopyMessage {
                    message_id: id.clone(),
                });
            }));
        h_flex()
            .id("assistant-meta")
            .relative()
            .track_focus(&self.meta_focus)
            .tab_index(-1)
            .mt(px(6.))
            .gap_2()
            .text_size(px(12.))
            .line_height(px(16.))
            .font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])))
            .text_color(theme.muted_foreground)
            .opacity(if self.message_hovered || self.copied {
                1.
            } else {
                0.
            })
            .in_focus(|style| style.opacity(1.))
            .when(
                !metadata.status.is_empty() && metadata.status != "completed",
                |row| {
                    row.child(
                        div()
                            .px(px(6.))
                            .py(px(2.))
                            .rounded_full()
                            .border_1()
                            .border_color(theme.border.opacity(0.7))
                            .text_size(px(10.))
                            .line_height(px(14.))
                            .child(metadata.status.clone()),
                    )
                },
            )
            .when(metadata.can_copy, |row| row.child(copy))
            .when(metadata.can_fork, |row| {
                let source = metadata.source_thread_id.clone();
                let run = metadata
                    .run_id
                    .clone()
                    .expect("forkable response has a run");
                row.child(
                    Button::new("fork-response")
                        .accessibility_label("Fork from this response")
                        .icon(Icon::new(IconName::GitFork).size(px(12.)))
                        .ghost()
                        .h(px(24.))
                        .w(px(28.))
                        .p_0()
                        .rounded(px(6.))
                        .disabled(self.forking)
                        .tooltip("Fork from this response")
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.forking = true;
                            cx.emit(RowEvent::ForkResponse {
                                source_thread_id: source.clone(),
                                run_id: run.clone(),
                            });
                            cx.notify();
                        })),
                )
            })
            .child(
                div()
                    .id("message-timestamp")
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(tooltip.clone())
                            .build(window, cx)
                    })
                    .child(timestamp),
            )
            .when(self.copied, |row| {
                row.child(
                    div()
                        .absolute()
                        .bottom_full()
                        .left_0()
                        .mb_1()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(theme.popover)
                        .text_color(theme.popover_foreground)
                        .child("Copied!"),
                )
            })
            .into_any_element()
    }

    pub fn update(&mut self, row: Row, cx: &mut Context<Self>) {
        if row == self.row {
            return;
        }
        if row.text != self.row.text || row.kind != self.row.kind {
            self.summary = if row.kind == RowKind::Reasoning {
                crate::timeline::thought_preview(&row.text)
            } else {
                row.text.clone()
            };
        }
        if let Some(markdown) = &self.markdown {
            let body = if row.kind == RowKind::Reasoning {
                &row.detail
            } else {
                &row.text
            };
            markdown.update(cx, |state, cx| state.set_text(body, cx));
        }
        if row.pending != self.row.pending {
            self.responding = false;
            if row.pending.is_none() {
                self.drafts.clear();
            }
        }
        self.row = row;
        self.refresh_accessible_body(cx);
        if self.expanded {
            self.sync_children(cx);
        }
        cx.notify();
    }

    fn sync_children(&mut self, cx: &mut Context<Self>) {
        for (index, row) in self.row.children.iter().enumerate() {
            if let Some(child) = self.children.get(index) {
                child.update(cx, |child, cx| child.update(row.clone(), cx));
            } else {
                self.children.push(cx.new(|cx| Self::new(row.clone(), cx)));
            }
        }
        self.children.truncate(self.row.children.len());
    }

    pub fn set_found(&mut self, found: bool, cx: &mut Context<Self>) {
        if self.found != found {
            self.found = found;
            if found && !self.row.children.is_empty() {
                self.expanded = true;
                self.sync_children(cx);
            }
            cx.notify();
        }
    }

    pub fn drafts(&self) -> &HashMap<String, Draft> {
        &self.drafts
    }

    pub fn set_responding(&mut self, responding: bool, cx: &mut Context<Self>) {
        self.responding = responding;
        cx.notify();
    }

    fn render_question(
        &self,
        qi: usize,
        question: &Question,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let draft = self.drafts.get(&question.id);
        let options = question.options.iter().enumerate().map(|(oi, option)| {
            let selected = draft.is_some_and(|d| d.selected.contains(&option.value));
            let value = option.value.clone();
            let question = question.clone();
            Button::new(SharedString::from(format!("option-{qi}-{oi}")))
                .label(option.label.clone())
                .compact()
                .when(selected, |b| b.primary())
                .when(!selected, |b| b.outline())
                .disabled(self.responding)
                .when(!option.description.is_empty(), |b| {
                    b.tooltip(option.description.clone())
                })
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.drafts
                        .entry(question.id.clone())
                        .or_default()
                        .toggle(&question, &value);
                    cx.notify();
                }))
        });
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .when(!question.header.is_empty(), |row| {
                        row.child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(question.header.clone()),
                        )
                    })
                    .when(question.multi_select, |row| {
                        row.child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child("(multiple)"),
                        )
                    }),
            )
            .child(question.question.clone())
            .child(h_flex().gap_1().flex_wrap().children(options))
    }

    fn render_pending(&self, pending: &Pending, cx: &mut Context<Self>) -> AnyElement {
        let warning = cx.theme().warning;
        let muted = cx.theme().muted_foreground;
        let mono = cx.theme().mono_font_family.clone();
        let card = v_flex()
            .p_2()
            .gap_2()
            .rounded_md()
            .border_1()
            .border_color(warning)
            .child(self.row.text.clone());
        match pending {
            Pending::Approval {
                request_id,
                subject,
                options,
            } => {
                let buttons = options.iter().enumerate().map(|(i, option)| {
                    let event = RowEvent::Respond {
                        request_id: request_id.clone(),
                        decision: option.decision.clone(),
                    };
                    let button = Button::new(("decision", i))
                        .label(option.label.clone())
                        .compact()
                        .disabled(self.responding)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.responding = true;
                            cx.emit(event.clone());
                            cx.notify();
                        }));
                    if option.decision == "accept" {
                        button.primary()
                    } else {
                        button.outline()
                    }
                });
                card.when(!subject.is_empty(), |card| {
                    card.child(
                        div()
                            .text_sm()
                            .font_family(mono)
                            .text_color(muted)
                            .child(subject.clone()),
                    )
                })
                .child(h_flex().gap_2().flex_wrap().children(buttons))
                .into_any_element()
            }
            Pending::Plan { plan_id, markdown } => {
                let event = RowEvent::Implement {
                    plan_id: plan_id.clone(),
                    markdown: markdown.clone(),
                };
                card.child(
                    h_flex().gap_2().child(
                        Button::new("implement")
                            .label("Implement plan")
                            .compact()
                            .primary()
                            .disabled(self.responding)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.responding = true;
                                cx.emit(event.clone());
                                cx.notify();
                            })),
                    ),
                )
                .into_any_element()
            }
            Pending::Question {
                request_id,
                questions,
            } => {
                let request_id = request_id.clone();
                let custom = questions.iter().any(|q| q.allow_custom);
                let items: Vec<AnyElement> = questions
                    .iter()
                    .enumerate()
                    .map(|(qi, q)| self.render_question(qi, q, cx).into_any_element())
                    .collect();
                card.children(items)
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("submit")
                                    .label("Submit")
                                    .compact()
                                    .primary()
                                    .disabled(self.responding)
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(RowEvent::Submit {
                                            request_id: request_id.clone(),
                                        });
                                    })),
                            )
                            .when(custom, |row| {
                                row.child(div().text_xs().text_color(muted).child(
                                    "Text in the message box answers the first open question.",
                                ))
                            }),
                    )
                    .into_any_element()
            }
        }
    }
}

impl Render for RowView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let base = div()
            .w_full()
            .max_w(px(736.))
            .mx_auto()
            .text_sm()
            .px_0()
            .when(self.found, |row| {
                row.border_l_2().border_color(theme.warning)
            });
        let content = match self.row.kind {
            RowKind::Worked => base
                .id("worked")
                .pt_1()
                .pb_2()
                .mb_3()
                .border_b_1()
                .border_color(theme.border.opacity(0.6))
                .text_color(theme.muted_foreground)
                .child(
                    h_flex()
                        .id("worked-toggle")
                        .gap_1()
                        .px_1()
                        .cursor_pointer()
                        .child(self.row.text.clone())
                        .child(
                            Icon::new(if self.expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size(px(14.)),
                        )
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.expanded = !view.expanded;
                            if view.expanded {
                                view.sync_children(cx);
                            }
                            cx.notify();
                        })),
                )
                .when(self.expanded, |row| {
                    row.child(v_flex().mt_2().children(self.children.iter().cloned()))
                })
                .into_any_element(),
            RowKind::Working => {
                let elapsed = humantime::parse_rfc3339(&self.row.detail)
                    .ok()
                    .and_then(|start| std::time::SystemTime::now().duration_since(start).ok());
                let label = elapsed
                    .map(|elapsed| {
                        format!(
                            "Working for {}",
                            humantime::format_duration(std::time::Duration::from_secs(
                                elapsed.as_secs()
                            ))
                        )
                    })
                    .unwrap_or_else(|| "Working...".into());
                base.mt_6()
                    .px_1()
                    .pt_1()
                    .pb_2()
                    .mb_2()
                    .border_b_1()
                    .border_color(theme.border.opacity(0.6))
                    .text_color(theme.muted_foreground)
                    .child(label)
                    .into_any_element()
            }
            RowKind::Assistant => match &self.markdown {
                Some(state) => base
                    .id("assistant-message")
                    .role(accesskit::Role::Document)
                    .aria_label("Assistant message")
                    .accessibility_id(self.row.key.clone())
                    .a11y_synthetic_children({
                        let body = self.accessible_body.clone();
                        move |builder| {
                            if let Some(body) = body {
                                body.append(builder);
                            }
                        }
                    })
                    .on_hover(cx.listener(|view, hovered, _, cx| {
                        if let Some((id, _)) = view.row.key.rsplit_once('#') {
                            cx.emit(RowEvent::MessageHover {
                                message_id: id.to_string(),
                                hovered: *hovered,
                            });
                        }
                    }))
                    .px_1()
                    .pt(px(
                        if self.row.message_part.as_ref().is_none_or(|part| part.first) {
                            2.
                        } else {
                            0.
                        },
                    ))
                    .pb(
                        if self.row.message_part.as_ref().is_none_or(|part| part.last) {
                            px(
                                if self
                                    .row
                                    .message_part
                                    .as_ref()
                                    .is_some_and(|part| part.metadata.is_some())
                                {
                                    18.
                                } else {
                                    10.
                                },
                            )
                        } else {
                            rems(0.65).to_pixels(window.rem_size())
                        },
                    )
                    .text_color(theme.foreground.opacity(0.9))
                    .line_height(relative(1.625))
                    .child(
                        TextView::new(state).selectable(true).style(
                            TextViewStyle::default()
                                .paragraph_gap(rems(0.65))
                                .code_block(
                                    StyleRefinement::default()
                                        .line_height(relative(1.375))
                                        .bg(if theme.mode.is_dark() {
                                            theme.input.opacity(0.32)
                                        } else {
                                            crate::appearance::code_background(cx)
                                        })
                                        .border_color(if theme.mode.is_dark() {
                                            transparent_black()
                                        } else {
                                            theme.border.opacity(0.7)
                                        }),
                                )
                                .table(
                                    StyleRefinement::default()
                                        .bg(theme.background)
                                        .border_0()
                                        .rounded_none(),
                                )
                                .table_head(
                                    StyleRefinement::default()
                                        .bg(theme.background)
                                        .font_weight(FontWeight::SEMIBOLD),
                                )
                                .table_cell(
                                    StyleRefinement::default()
                                        .border_0()
                                        .border_color(transparent_black())
                                        .px_0()
                                        .py_2()
                                        .text_sm(),
                                ),
                        ),
                    )
                    .when_some(
                        self.row
                            .message_part
                            .as_ref()
                            .and_then(|part| part.metadata.as_ref()),
                        |row, metadata| row.child(self.render_assistant_meta(metadata, cx)),
                    )
                    .into_any_element(),
                None => base.into_any_element(),
            },
            RowKind::User => base
                .py_2()
                .flex()
                .justify_end()
                .child(
                    div()
                        .id("user-bubble")
                        .role(accesskit::Role::Document)
                        .aria_label("User message")
                        .accessibility_id(self.row.key.clone())
                        .a11y_synthetic_children({
                            let body = self.accessible_body.clone();
                            move |builder| {
                                if let Some(body) = body {
                                    body.append(builder);
                                }
                            }
                        })
                        .max_w(relative(0.8))
                        .p_3()
                        .rounded(px(16.))
                        .bg(crate::appearance::message(cx))
                        .when(self.row.detail == "agent", |bubble| {
                            bubble
                                .cursor_pointer()
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.expanded = !view.expanded;
                                    cx.notify();
                                }))
                        })
                        .map(|bubble| {
                            if self.row.detail == "agent" && !self.expanded {
                                bubble.py_2().child(
                                    h_flex()
                                        .min_w_0()
                                        .gap_1()
                                        .child(
                                            div().text_color(theme.muted_foreground).child("Agent"),
                                        )
                                        .child(
                                            div().min_w_0().truncate().child(self.row.text.clone()),
                                        )
                                        .child(Icon::new(IconName::ChevronDown).size(px(12.))),
                                )
                            } else {
                                bubble.child(self.row.text.clone())
                            }
                        }),
                )
                .into_any_element(),
            RowKind::Reasoning | RowKind::Tool | RowKind::Subagent => {
                let summary = self.summary.clone();
                let openable = !self.row.detail.is_empty() || !self.row.children.is_empty();
                let detail = (self.expanded && self.row.children.is_empty()).then(|| {
                    let lines: Vec<&str> = self.row.detail.lines().collect();
                    let shown = lines.len().min(DETAIL_LINES);
                    let mut text = lines[..shown].join("\n");
                    if lines.len() > shown {
                        text.push_str(&format!("\n… {} more lines", lines.len() - shown));
                    }
                    div()
                        .id("activity-detail")
                        .max_h(px(384.))
                        .overflow_y_scroll()
                        .mt_1()
                        .ml_7()
                        .py_1()
                        .text_xs()
                        .whitespace_normal()
                        .when(self.row.kind != RowKind::Reasoning, |detail| {
                            detail.font_family(theme.mono_font_family.clone())
                        })
                        .map(|detail| match &self.markdown {
                            Some(markdown) if self.row.kind == RowKind::Reasoning => {
                                detail.child(TextView::new(markdown).selectable(true))
                            }
                            _ => detail.child(text),
                        })
                        .on_click(|_, _, cx| cx.stop_propagation())
                });
                base.id("row")
                    .py_0p5()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .when(openable, |row| {
                        row.cursor_pointer().on_click(cx.listener(|view, _, _, cx| {
                            view.expanded = !view.expanded;
                            if view.expanded {
                                view.sync_children(cx);
                                if view.row.kind == RowKind::Reasoning
                                    && view.markdown.is_none()
                                    && view.row.children.is_empty()
                                {
                                    view.markdown =
                                        Some(cx.new(|cx| {
                                            TextViewState::markdown(&view.row.detail, cx)
                                        }));
                                }
                            }
                            cx.notify();
                        }))
                    })
                    .child(
                        h_flex()
                            .gap(px(6.))
                            .min_h(px(24.))
                            .child(
                                div().w(px(24.)).flex_none().flex().justify_center().child(
                                    Icon::new(match self.row.kind {
                                        RowKind::Reasoning => IconName::Brain,
                                        RowKind::Subagent => IconName::Bot,
                                        _ => IconName::Terminal,
                                    })
                                    .size(px(16.)),
                                ),
                            )
                            .child(div().flex_1().min_w_0().truncate().child(summary))
                            .when(openable, |line| {
                                line.child(
                                    Icon::new(if self.expanded {
                                        IconName::ChevronDown
                                    } else {
                                        IconName::ChevronRight
                                    })
                                    .size(px(12.))
                                    .text_color(theme.muted_foreground.opacity(0.7)),
                                )
                            })
                            .when_some(self.row.link.clone(), |line, thread_id| {
                                line.child(
                                    Button::new("open-child")
                                        .label("Open")
                                        .compact()
                                        .outline()
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.emit(RowEvent::OpenChild {
                                                thread_id: thread_id.clone(),
                                            });
                                        })),
                                )
                            }),
                    )
                    .children(detail)
                    .when(self.expanded && !self.children.is_empty(), |row| {
                        row.child(
                            v_flex()
                                .id("activity-children")
                                .max_h(px(288.))
                                .overflow_y_scroll()
                                .ml_7()
                                .children(self.children.iter().cloned())
                                .on_click(|_, _, cx| cx.stop_propagation()),
                        )
                    })
                    .into_any_element()
            }
            RowKind::Fork => base
                .py_2()
                .text_size(px(11.))
                .text_color(theme.muted_foreground)
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().h(px(1.)).flex_1().bg(theme.border.opacity(0.7)))
                        .child(
                            Button::new("fork-link")
                                .accessibility_label(self.row.detail.clone())
                                .ghost()
                                .tooltip(self.row.detail.clone())
                                .h_auto()
                                .text_size(px(11.))
                                .gap(px(6.))
                                .px(px(10.))
                                .py_1()
                                .rounded_full()
                                .border_1()
                                .border_color(theme.border.opacity(0.7))
                                .bg(theme.background)
                                .text_color(theme.muted_foreground)
                                .font_weight(FontWeight::MEDIUM)
                                .cursor_pointer()
                                .hover_style(
                                    StyleRefinement::default()
                                        .bg(theme.accent)
                                        .text_color(theme.foreground)
                                        .border_color(theme.border.opacity(0.7)),
                                )
                                .on_click(cx.listener(|view, _, _, cx| {
                                    if let Some(thread_id) = &view.row.link {
                                        cx.emit(RowEvent::OpenChild {
                                            thread_id: thread_id.clone(),
                                        });
                                    }
                                }))
                                .child(
                                    h_flex()
                                        .gap(px(6.))
                                        .text_size(px(11.))
                                        .line_height(px(16.))
                                        .child(Icon::new(IconName::GitFork).size(px(12.)))
                                        .child(self.row.text.clone()),
                                ),
                        )
                        .child(div().h(px(1.)).flex_1().bg(theme.border.opacity(0.7))),
                )
                .into_any_element(),
            RowKind::Notice => base
                .py_2()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().h(px(1.)).flex_1().bg(theme.border))
                        .when(self.row.detail == "run_interrupt_result", |row| {
                            row.child(Icon::new(IconName::CircleStop).size(px(12.)))
                        })
                        .child(if self.row.detail == "run_interrupt_result" {
                            format!("Run interrupted · {}", self.row.text)
                        } else {
                            self.row.text.clone()
                        })
                        .child(div().h(px(1.)).flex_1().bg(theme.border)),
                )
                .into_any_element(),
            RowKind::Request | RowKind::Plan => match self.row.pending.clone() {
                Some(pending) => base
                    .py_1()
                    .child(self.render_pending(&pending, cx))
                    .into_any_element(),
                None => base
                    .py_1()
                    .child(
                        div()
                            .p_2()
                            .rounded_md()
                            .border_1()
                            .border_color(theme.border)
                            .child(self.row.text.clone()),
                    )
                    .into_any_element(),
            },
            RowKind::Error => base
                .py_1()
                .text_color(theme.danger)
                .child(self.row.text.clone())
                .into_any_element(),
        };
        div().w_full().flex().justify_center().child(content)
    }
}
