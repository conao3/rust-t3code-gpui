use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
};

use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    text::{
        InlineElement, InlineRenderContext, MarkdownExtensions, MarkdownNode, MarkdownParseContext,
        MarkdownPlugin,
    },
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use markdown::mdast::Node;

#[derive(Clone)]
struct Graphic {
    images: [Arc<Image>; 2],
    width: f32,
    height: f32,
    baseline: f32,
}

struct Equation(bool);
struct Diagram;

type GraphicCache = Mutex<VecDeque<(String, Result<Graphic, String>)>>;

fn cached_graphic(
    key: String,
    render: impl FnOnce() -> Result<Graphic, String>,
) -> Result<Graphic, String> {
    static CACHE: OnceLock<GraphicCache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(VecDeque::new()));
    if let Some((_, graphic)) = cache
        .lock()
        .expect("graphic cache lock")
        .iter()
        .find(|(stored, _)| *stored == key)
    {
        return graphic.clone();
    }
    let graphic = render();
    let mut cache = cache.lock().expect("graphic cache lock");
    if cache.len() >= 128 {
        cache.pop_front();
    }
    cache.push_back((key, graphic.clone()));
    graphic
}

pub fn extensions(math: bool) -> MarkdownExtensions {
    static EXTENSIONS: OnceLock<[MarkdownExtensions; 2]> = OnceLock::new();
    EXTENSIONS.get_or_init(|| {
        std::array::from_fn(|index| {
            let extensions = MarkdownExtensions::default()
                .math(index == 1)
                .plugin(Diagram);
            if index == 1 {
                extensions.plugin(Equation(true)).plugin(Equation(false))
            } else {
                extensions
            }
        })
    })[usize::from(math)]
    .clone()
}

pub fn user_source(source: &str, math: bool) -> String {
    fn visit(node: &Node, source: &str, changes: &mut Vec<(std::ops::Range<usize>, String)>) {
        if let Some(position) = node.position()
            && matches!(node, Node::Text(_) | Node::Html(_))
        {
            let range = position.start.offset..position.end.offset;
            let text = &source[range.clone()];
            let mut replacement = String::new();
            let mut previous = None;
            for character in text.chars() {
                if character == '\r' || character == '\n' && previous != Some('\r') {
                    replacement.push('\\');
                }
                if matches!(node, Node::Html(_)) && matches!(character, '<' | '>') {
                    replacement.push('\\');
                }
                replacement.push(character);
                previous = Some(character);
            }
            if replacement != text {
                changes.push((range, replacement));
            }
        }
        if let Some(children) = node.children() {
            for child in children {
                visit(child, source, changes);
            }
        }
    }
    let mut options = markdown::ParseOptions::gfm();
    options.constructs.math_text = math;
    options.constructs.math_flow = math;
    let tree = markdown::to_mdast(source, &options).expect("valid user message markdown");
    let mut changes = Vec::new();
    visit(&tree, source, &mut changes);
    let mut rendered = source.to_string();
    for (range, replacement) in changes.into_iter().rev() {
        rendered.replace_range(range, &replacement);
    }
    rendered
}

fn equation(source: &str, block: bool) -> Result<Graphic, String> {
    use ratex_types::{color::Color, math_style::MathStyle};
    let nodes = ratex_parser::parse(source).map_err(|error| error.to_string())?;
    let mut images = vec![];
    let mut geometry = (0., 0., 0.);
    for light in [true, false] {
        let rgba = crate::appearance::color("text", light).to_rgb();
        let color = Color::new(rgba.r, rgba.g, rgba.b, rgba.a * 0.9);
        let layout = ratex_layout::layout(
            &nodes,
            &ratex_layout::LayoutOptions {
                style: if block {
                    MathStyle::Display
                } else {
                    MathStyle::Text
                },
                color,
                ..Default::default()
            },
        );
        ratex_layout::validate_layout(&layout)?;
        let list = ratex_layout::to_display_list(&layout);
        geometry = (
            (list.width * 19.36 + 2.) as f32,
            ((list.height + list.depth) * 19.36 + 2.) as f32,
            (list.height * 19.36 + 1.) as f32,
        );
        let svg = ratex_svg::render_to_svg_with_color_syntax(
            &list,
            &ratex_svg::SvgOptions {
                font_size: 19.36,
                padding: 1.,
                stroke_width: 0.6,
                embed_glyphs: true,
                ..Default::default()
            },
            ratex_svg::SvgColorSyntax::Rgb,
        );
        images.push(Arc::new(Image::from_bytes(
            ImageFormat::Svg,
            svg.into_bytes(),
        )));
    }
    Ok(Graphic {
        images: images
            .try_into()
            .map_err(|_| "Missing math theme".to_string())?,
        width: geometry.0,
        height: geometry.1,
        baseline: geometry.2,
    })
}

fn graphic_node(
    name: &'static str,
    source: &str,
    label: &str,
    data: Result<Graphic, String>,
) -> MarkdownNode {
    MarkdownNode::new(name, data)
        .text(source.to_string())
        .markdown(source.to_string())
        .accessibility_label(label.to_string())
}

impl MarkdownPlugin for Equation {
    fn is_block(&self) -> bool {
        self.0
    }
    fn name(&self) -> &str {
        if self.0 {
            "display-equation"
        } else {
            "inline-equation"
        }
    }
    fn parse(&self, node: &Node, _: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        let source = match (self.0, node) {
            (true, Node::Math(value)) => &value.value,
            (true, Node::Code(value)) if value.lang.as_deref() == Some("math") => &value.value,
            (false, Node::InlineMath(value)) => &value.value,
            _ => return None,
        };
        Some(graphic_node(
            if self.0 {
                "display-equation"
            } else {
                "inline-equation"
            },
            source,
            if self.0 {
                "Display equation"
            } else {
                "Inline equation"
            },
            cached_graphic(format!("equation:{}:{source}", self.0), || {
                equation(source, self.0)
            }),
        ))
    }
    fn render(&self, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> impl IntoElement {
        render_graphic(
            node,
            cx,
            true,
            window.text_style().font_size.to_pixels(window.rem_size()) / px(16.),
        )
    }
    fn render_inline(
        &self,
        node: &MarkdownNode,
        context: &InlineRenderContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<InlineElement> {
        match node.data::<Result<Graphic, String>>()? {
            Ok(graphic) => {
                let scale = context.font_size() / px(16.);
                Some(
                    InlineElement::new(
                        img(graphic.images[usize::from(cx.theme().mode.is_dark())].clone())
                            .w(px(graphic.width * scale))
                            .h(px(graphic.height * scale)),
                    )
                    .with_baseline(px(graphic.baseline * scale)),
                )
            }
            Err(error) => Some(InlineElement::new(
                div()
                    .text_color(cx.theme().danger)
                    .child(format!("{} ({error})", node.as_text())),
            )),
        }
    }
}

fn render_graphic(node: &MarkdownNode, cx: &App, centered: bool, scale: f32) -> AnyElement {
    match node.data::<Result<Graphic, String>>() {
        Some(Ok(graphic)) => div()
            .flex()
            .w_full()
            .min_w_0()
            .py_2()
            .when(centered, |el| el.justify_center())
            .child(
                img(graphic.images[usize::from(cx.theme().mode.is_dark())].clone())
                    .w(px(graphic.width * scale))
                    .aspect_ratio(graphic.width / graphic.height)
                    .max_w_full()
                    .object_fit(ObjectFit::Contain),
            )
            .into_any_element(),
        Some(Err(error)) => div()
            .text_color(cx.theme().danger)
            .child(format!("{error}\n{}", node.as_text()))
            .into_any_element(),
        None => div().into_any_element(),
    }
}

impl MarkdownPlugin for Diagram {
    fn is_block(&self) -> bool {
        true
    }
    fn name(&self) -> &str {
        "mermaid"
    }
    fn parse(&self, node: &Node, _: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        let Node::Code(code) = node else {
            return None;
        };
        if code.lang.as_deref() != Some("mermaid") {
            return None;
        }
        let render = || -> Result<Graphic, String> {
            let mut images = vec![];
            let mut geometry = (0., 0.);
            for light in [true, false] {
                let theme = diagram_theme(light);
                let result = pelago::render(
                    &code.value,
                    &pelago::RenderOptions {
                        theme,
                        font_family: Some("Inter Variable".into()),
                        font_family_mono: Some("monospace".into()),
                        font_size: Some(16.),
                        auto_direction: Some(false),
                        max_width: Some(640.),
                        links: Some(false),
                        images: Some(false),
                        ..Default::default()
                    },
                )
                .map_err(|error| error.to_string())?;
                geometry = (result.width as f32, result.height as f32);
                let mut options = usvg::Options {
                    font_family: "Inter Variable".into(),
                    font_size: 16.,
                    ..Default::default()
                };
                options
                    .fontdb_mut()
                    .load_font_data(include_bytes!("../assets/InterVariable.ttf").to_vec());
                let svg = usvg::Tree::from_str(&result.svg, &options)
                    .map_err(|error| error.to_string())?
                    .to_string(&usvg::WriteOptions::default());
                images.push(Arc::new(Image::from_bytes(
                    ImageFormat::Svg,
                    svg.into_bytes(),
                )));
            }
            Ok(Graphic {
                images: images
                    .try_into()
                    .map_err(|_| "Missing diagram theme".to_string())?,
                width: geometry.0,
                height: geometry.1,
                baseline: geometry.1,
            })
        };
        Some(graphic_node(
            "mermaid",
            &code.value,
            "Mermaid diagram",
            cached_graphic(format!("mermaid:{}", code.value), render),
        ))
    }
    fn render(&self, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view = window.use_keyed_state(
            (
                "mermaid",
                node.source_range().map_or(0, |range| range.start),
            ),
            cx,
            |_, _| DiagramView {
                node: node.clone(),
                code: false,
                copied: false,
                wrapped: true,
            },
        );
        view.update(cx, |view, _| view.node = node.clone());
        view
    }
}

struct DiagramView {
    node: MarkdownNode,
    code: bool,
    copied: bool,
    wrapped: bool,
}

impl Render for DiagramView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let source = self.node.as_text().to_owned();
        let graphic = self
            .node
            .data::<Result<Graphic, String>>()
            .and_then(|value| value.as_ref().ok())
            .cloned();
        div()
            .id("mermaid-block")
            .w_full()
            .rounded_lg()
            .border_1()
            .border_color(if self.code {
                cx.theme().border
            } else {
                black().opacity(0.)
            })
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(32.))
                    .px_3()
                    .gap_1()
                    .when(self.code, |header| header.bg(cx.theme().muted))
                    .child(div().flex_1().text_xs().when(self.code, |label| {
                        label.child(
                            div()
                                .id("diagram-language")
                                .role(accesskit::Role::Label)
                                .aria_value("mermaid")
                                .child("mermaid"),
                        )
                    }))
                    .when(self.code, |header| {
                        header.child(
                            Button::new("diagram-wrap")
                                .icon(gpui_kit::assets::IconName::TextWrap)
                                .accessibility_label(if self.wrapped {
                                    "Disable line wrap"
                                } else {
                                    "Wrap lines"
                                })
                                .selected(self.wrapped)
                                .ghost()
                                .compact()
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.wrapped = !view.wrapped;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        Button::new("diagram-code")
                            .icon(gpui_kit::assets::IconName::Code)
                            .ghost()
                            .compact()
                            .accessibility_label(if self.code {
                                "Show diagram"
                            } else {
                                "Show code"
                            })
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.code = !view.code;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("diagram-copy")
                            .icon(gpui_kit::assets::IconName::Copy)
                            .ghost()
                            .compact()
                            .accessibility_label(if self.copied { "Copied" } else { "Copy code" })
                            .on_click(cx.listener(move |view, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    if source.is_empty() {
                                        String::new()
                                    } else {
                                        format!("{source}\n")
                                    },
                                ));
                                view.copied = true;
                                cx.notify();
                                cx.spawn(async move |this, cx| {
                                    cx.background_executor()
                                        .timer(std::time::Duration::from_millis(1200))
                                        .await;
                                    let _ = this.update(cx, |view, cx| {
                                        view.copied = false;
                                        cx.notify();
                                    });
                                })
                                .detach();
                            })),
                    ),
            )
            .when(self.code, |body| {
                body.child(
                    div()
                        .id("diagram-source")
                        .role(accesskit::Role::Label)
                        .aria_value(self.node.as_text().to_owned())
                        .p_3()
                        .text_size(px(13.))
                        .font_family(cx.theme().mono_font_family.clone())
                        .when(!self.wrapped, |code| code.whitespace_nowrap())
                        .child(self.node.as_text().to_owned()),
                )
            })
            .when(!self.code, |body| {
                body.child(
                    Button::new("expand-diagram")
                        .accessibility_label("Expand diagram")
                        .ghost()
                        .h_auto()
                        .p_0()
                        .w_full()
                        .child(render_graphic(&self.node, cx, true, 1.))
                        .on_click(move |_, window, cx| {
                            let Some(graphic) = graphic.clone() else {
                                return;
                            };
                            let viewer = cx.new(|cx| DiagramImage::new(graphic, cx));
                            window.open_dialog(cx, move |dialog, window, _| {
                                dialog
                                    .accessibility_label("Expanded image preview")
                                    .close_button(false)
                                    .centered()
                                    .p_0()
                                    .border_0()
                                    .bg(transparent_black())
                                    .width(window.viewport_size().width * 0.92)
                                    .child(
                                        div()
                                            .relative()
                                            .top(px(18.))
                                            .h(window.viewport_size().height * 0.86)
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .justify_center()
                                            .gap_2()
                                            .child(
                                                Button::new("close-image")
                                                    .absolute()
                                                    .top_0()
                                                    .right_0()
                                                    .icon(gpui_kit::assets::IconName::X)
                                                    .accessibility_label("Close image preview")
                                                    .ghost()
                                                    .on_click(|_, window, cx| {
                                                        window.close_dialog(cx);
                                                    }),
                                            )
                                            .child(viewer.clone())
                                            .child(
                                                div()
                                                    .id("diagram-caption")
                                                    .text_xs()
                                                    .role(accesskit::Role::Label)
                                                    .aria_value("Mermaid diagram")
                                                    .child("Mermaid diagram"),
                                            ),
                                    )
                            });
                        }),
                )
            })
    }
}

struct DiagramImage {
    graphic: Graphic,
    zoom: f32,
    scroll: ScrollHandle,
    drag: Option<(Point<Pixels>, Point<Pixels>)>,
    dragged: bool,
    focus: FocusHandle,
    viewport: Option<Size<Pixels>>,
    _keys: Subscription,
}

impl DiagramImage {
    fn new(graphic: Graphic, cx: &mut Context<Self>) -> Self {
        let weak = cx.entity().downgrade();
        let keys = cx.intercept_keystrokes(move |event, window, cx| {
            let handled = weak.update(cx, |view, cx| {
                let key = &event.keystroke;
                if !view.focus.is_focused(window)
                    || key.modifiers.control
                    || key.modifiers.platform
                    || key.modifiers.alt
                    || !matches!(key.key.as_str(), "enter" | "space")
                {
                    return false;
                }
                if !event.is_held {
                    view.zoom(if view.zoom > 1. { 1. } else { 2. }, cx);
                }
                true
            });
            if handled.is_ok_and(|handled| handled) {
                cx.stop_propagation();
            }
        });
        Self {
            graphic,
            zoom: 1.,
            scroll: ScrollHandle::new(),
            drag: None,
            dragged: false,
            focus: cx.focus_handle(),
            viewport: None,
            _keys: keys,
        }
    }

    fn zoom(&mut self, value: f32, cx: &mut Context<Self>) {
        self.zoom_at(value, None, cx);
    }

    fn zoom_at(&mut self, value: f32, position: Option<Point<Pixels>>, cx: &mut Context<Self>) {
        let next = value.clamp(1., 8.);
        let offset = self.scroll.offset();
        let bounds = self.scroll.bounds();
        let anchor = position
            .map(|position| position - bounds.origin)
            .unwrap_or_else(|| point(bounds.size.width / 2., bounds.size.height / 2.));
        self.scroll
            .set_offset(anchor - (anchor - offset) * (next / self.zoom));
        self.zoom = next;
        cx.notify();
    }
}

impl Render for DiagramImage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        if self.viewport != Some(viewport) {
            self.viewport = Some(viewport);
            self.zoom = 1.;
            self.scroll.set_offset(point(px(0.), px(0.)));
        }
        let width = (f32::from(viewport.width) * 0.92
            - if viewport.width >= px(640.) { 96. } else { 0. })
        .max(1.);
        let height = (f32::from(viewport.height) * 0.86)
            .min(f32::from(viewport.height) - 160.)
            .max(1.);
        let fit = (width / self.graphic.width).min(height / self.graphic.height);
        div()
            .id("diagram-zoom")
            .bg(black())
            .border_1()
            .border_color(white().opacity(0.08))
            .rounded_lg()
            .role(accesskit::Role::Region)
            .aria_label("Mermaid diagram, zoomable image")
            .aria_description("Click to zoom in or return to fit. Pinch or Ctrl+scroll to zoom, scroll or drag to pan. Use Enter to toggle zoom, plus or minus to zoom, and 0 to fit.")
            .track_focus(&self.focus)
            .tab_index(0)
            .w(px((self.graphic.width * fit * self.zoom).min(width)))
            .h(px((self.graphic.height * fit * self.zoom).min(height)))
            .overflow_scroll()
            .track_scroll(&self.scroll)
            .on_click(cx.listener(|view, _, window, cx| {
                view.focus.focus(window, cx);
                if !view.dragged {
                    view.zoom(if view.zoom > 1. { 1. } else { 2. }, cx);
                }
            }))
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, _, cx| {
                if event.keystroke.modifiers.control
                    || event.keystroke.modifiers.platform
                    || event.keystroke.modifiers.alt
                {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "+" | "=" => view.zoom(view.zoom * 1.5, cx),
                    "-" => view.zoom(view.zoom / 1.5, cx),
                    "0" => view.zoom(1., cx),
                    "left" | "right" | "up" | "down" if view.zoom > 1. => {
                        let delta = match event.keystroke.key.as_str() {
                            "left" => point(px(40.), px(0.)),
                            "right" => point(px(-40.), px(0.)),
                            "up" => point(px(0.), px(40.)),
                            _ => point(px(0.), px(-40.)),
                        };
                        view.scroll.set_offset(view.scroll.offset() + delta);
                        cx.notify();
                    }
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .on_scroll(cx.listener(|view, event: &ScrollEvent, _, cx| {
                if event.modifiers.control {
                    view.zoom_at(
                        view.zoom * (-f32::from(event.delta.pixel_delta(px(16.)).y) * 0.01).exp(),
                        Some(event.position),
                        cx,
                    );
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &MouseDownEvent, _, _| {
                    view.dragged = false;
                    view.drag = (view.zoom > 1.).then(|| (event.position, view.scroll.offset()));
                }),
            )
            .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, _, cx| {
                if event.pressed_button == Some(MouseButton::Left)
                    && let Some((start, offset)) = view.drag
                {
                    let delta = event.position - start;
                    view.dragged |= delta.x.abs() > px(4.) || delta.y.abs() > px(4.);
                    view.scroll.set_offset(offset + delta);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, _| view.drag = None),
            )
            .child(
                img(self.graphic.images[usize::from(cx.theme().mode.is_dark())].clone())
                    .w(px(self.graphic.width * fit * self.zoom))
                    .h(px(self.graphic.height * fit * self.zoom)),
            )
    }
}

fn diagram_theme(light: bool) -> pelago::Theme {
    let color = |role, alpha: f32| {
        let value = crate::appearance::color(role, light).to_rgb();
        Some(format!(
            "#{:02x}{:02x}{:02x}{:02x}",
            (value.r * 255.).round() as u8,
            (value.g * 255.).round() as u8,
            (value.b * 255.).round() as u8,
            (value.a * alpha * 255.).round() as u8
        ))
    };
    pelago::Theme {
        bg: color("canvas", 1.),
        fg: color("text", 1.),
        muted: color("textMuted", 1.),
        line: color("textMuted", 1.),
        surface: color("muted", 1.),
        surface_alt: color("accentSurface", 1.),
        border: color("text", 0.28),
        accent: color("accent", 1.),
        critical: color("error", 1.),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::equation;
    #[test]
    fn equations_have_embedded_outlines_and_positive_geometry() {
        let graphic = equation(r"\frac{x^2}{\sqrt{y}}", true).unwrap();
        assert!(graphic.width > 0. && graphic.height > 0. && graphic.baseline > 0.);
        assert!(equation(r"\definitelyUnknownCommand", false).is_err());
    }

    #[test]
    fn user_messages_keep_breaks_without_rewriting_code_or_links() {
        let source = "**First**\nSecond\r\nThird\n\n```rust\nlet a = 1;\nlet b = 2;\n```\n\n[Link](https://example.com)\n<mark>literal</mark>";
        let rendered = super::user_source(source, false);
        assert!(rendered.starts_with("**First**\\\nSecond\\\r\nThird"));
        assert!(rendered.contains("```rust\nlet a = 1;\nlet b = 2;\n```"));
        assert!(rendered.contains("[Link](https://example.com)"));
        assert!(rendered.ends_with("\\<mark\\>literal\\</mark\\>"));
        let tree = markdown::to_mdast(&rendered, &markdown::ParseOptions::gfm()).unwrap();
        let paragraph = &tree.children().unwrap()[0];
        assert_eq!(
            paragraph
                .children()
                .unwrap()
                .iter()
                .filter(|node| matches!(node, markdown::mdast::Node::Break(_)))
                .count(),
            2
        );
    }
}
