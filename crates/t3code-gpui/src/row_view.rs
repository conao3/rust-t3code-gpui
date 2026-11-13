use gpui_kit::assets::IconName;
use std::collections::HashMap;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon,
    button::{Button, ButtonVariants as _},
    h_flex,
    text::{TextView, TextViewState, TextViewStyle},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::timeline::{Draft, Pending, Question, Row, RowKind};

const DETAIL_LINES: usize = 400;

#[derive(Clone)]
pub enum RowEvent {
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
    markdown: Option<Entity<TextViewState>>,
    found: bool,
    expanded: bool,
    drafts: HashMap<String, Draft>,
    responding: bool,
}

impl EventEmitter<RowEvent> for RowView {}

impl RowView {
    pub fn new(row: Row, cx: &mut Context<Self>) -> Self {
        let markdown = (row.kind == RowKind::Assistant)
            .then(|| cx.new(|cx| TextViewState::markdown(&row.text, cx)));
        Self {
            row,
            markdown,
            found: false,
            expanded: false,
            drafts: HashMap::new(),
            responding: false,
        }
    }

    pub fn update(&mut self, row: Row, cx: &mut Context<Self>) {
        if row == self.row {
            return;
        }
        if let Some(markdown) = &self.markdown {
            markdown.update(cx, |state, cx| state.set_text(&row.text, cx));
        }
        if row.pending != self.row.pending {
            self.responding = false;
            if row.pending.is_none() {
                self.drafts.clear();
            }
        }
        self.row = row;
        cx.notify();
    }

    pub fn set_found(&mut self, found: bool, cx: &mut Context<Self>) {
        if self.found != found {
            self.found = found;
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let base = div()
            .w_full()
            .max_w(px(736.))
            .mx_auto()
            .text_sm()
            .px_0()
            .border_l_2()
            .border_color(if self.found {
                theme.warning
            } else {
                gpui::transparent_black()
            });
        let content = match self.row.kind {
            RowKind::Assistant => match &self.markdown {
                Some(state) => base
                    .py_1()
                    .text_color(theme.foreground.opacity(0.9))
                    .line_height(relative(1.625))
                    .child(
                        TextView::new(state).selectable(true).style(
                            TextViewStyle::default()
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
                    .into_any_element(),
                None => base.into_any_element(),
            },
            RowKind::User => base
                .py_2()
                .flex()
                .justify_end()
                .child(
                    div()
                        .max_w(relative(0.8))
                        .p_3()
                        .rounded(px(16.))
                        .bg(crate::appearance::message(cx))
                        .child(self.row.text.clone()),
                )
                .into_any_element(),
            RowKind::Reasoning | RowKind::Tool | RowKind::Subagent => {
                let summary = if self.row.kind == RowKind::Reasoning {
                    format!("Thinking: {}", self.row.text)
                } else {
                    self.row.text.clone()
                };
                let openable = !self.row.detail.is_empty();
                let marker = match (openable, self.expanded) {
                    (false, _) => "",
                    (true, false) => "▸ ",
                    (true, true) => "▾ ",
                };
                let detail = self.expanded.then(|| {
                    let lines: Vec<&str> = self.row.detail.lines().collect();
                    let shown = lines.len().min(DETAIL_LINES);
                    let mut text = lines[..shown].join("\n");
                    if lines.len() > shown {
                        text.push_str(&format!("\n… {} more lines", lines.len() - shown));
                    }
                    div()
                        .mt_1()
                        .p_2()
                        .rounded_md()
                        .bg(crate::appearance::message(cx))
                        .font_family(theme.mono_font_family.clone())
                        .text_xs()
                        .whitespace_normal()
                        .child(text)
                });
                base.id("row")
                    .py_0p5()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .when(self.row.kind == RowKind::Tool, |row| {
                        row.font_family(theme.mono_font_family.clone())
                    })
                    .when(openable, |row| {
                        row.cursor_pointer().on_click(cx.listener(|view, _, _, cx| {
                            view.expanded = !view.expanded;
                            cx.notify();
                        }))
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Icon::new(match self.row.kind {
                                    RowKind::Reasoning => IconName::Lightbulb,
                                    RowKind::Subagent => IconName::Bot,
                                    _ => IconName::Terminal,
                                })
                                .size(px(14.)),
                            )
                            .child(div().flex_1().min_w_0().child(format!("{marker}{summary}")))
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
                    .into_any_element()
            }
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
