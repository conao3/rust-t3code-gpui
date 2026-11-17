use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
};

use gpui_kit::component::{
    ActiveTheme as _, WindowExt as _,
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

pub fn extensions() -> MarkdownExtensions {
    static EXTENSIONS: OnceLock<MarkdownExtensions> = OnceLock::new();
    EXTENSIONS
        .get_or_init(|| {
            MarkdownExtensions::default()
                .plugin(Equation(true))
                .plugin(Equation(false))
                .plugin(Diagram)
        })
        .clone()
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
    fn render(&self, node: &MarkdownNode, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(Ok(graphic)) = node.data::<Result<Graphic, String>>() else {
            return render_graphic(node, cx, true, 1.);
        };
        let image = graphic.images[usize::from(cx.theme().mode.is_dark())].clone();
        let ratio = graphic.width / graphic.height;
        Button::new(ElementId::Name(node.as_text().to_string().into()))
            .accessibility_label("Expand diagram")
            .ghost()
            .h_auto()
            .p_0()
            .w_full()
            .child(render_graphic(node, cx, true, 1.))
            .on_click(move |_, window, cx| {
                let image = image.clone();
                window.open_dialog(cx, move |dialog, window, _| {
                    dialog
                        .title("Diagram")
                        .accessibility_label("Diagram")
                        .centered()
                        .width(window.viewport_size().width * 0.9)
                        .child(
                            img(image.clone())
                                .w_full()
                                .aspect_ratio(ratio)
                                .max_h(window.viewport_size().height * 0.8)
                                .object_fit(ObjectFit::Contain),
                        )
                });
            })
            .into_any_element()
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
}
