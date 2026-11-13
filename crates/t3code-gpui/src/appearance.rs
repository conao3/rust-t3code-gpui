use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

pub const MESSAGE: u32 = 0x1e1e1e;
pub const ROW_INSET: f32 = 10.;

pub fn apply(light: bool, cx: &mut App) {
    cx.text_system()
        .add_fonts(vec![std::borrow::Cow::Borrowed(include_bytes!(
            "../assets/InterVariable.ttf"
        ))])
        .expect("load bundled Inter font");
    Theme::change(
        if light {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        },
        None,
        cx,
    );
    Theme::update(cx, |theme| {
        let c = &mut theme.colors;
        c.accent = color("surface", light);
        c.accent_foreground = color("text", light);
        c.accordion = color("surface", light);
        c.background = color("canvas", light);
        c.border = color("border", light);
        c.button = color("surface", light);
        c.button_active = color("muted", light);
        c.button_foreground = color("text", light);
        c.button_hover = color("muted", light);
        c.button_danger = color("error", light);
        c.button_danger_active = color("error", light);
        c.button_danger_foreground = color("errorForeground", light);
        c.button_danger_hover = color("error", light);
        c.button_info = color("surface", light);
        c.button_info_active = color("muted", light);
        c.button_info_foreground = color("text", light);
        c.button_info_hover = color("muted", light);
        c.button_primary = color("accent", light);
        c.button_primary_active = color("accent", light);
        c.button_primary_foreground = color("accentForeground", light);
        c.button_primary_hover = color("accent", light);
        c.button_secondary = color("surface", light);
        c.button_secondary_active = color("muted", light);
        c.button_secondary_foreground = color("text", light);
        c.button_secondary_hover = color("muted", light);
        c.button_success = color("success", light);
        c.button_success_active = color("success", light);
        c.button_success_foreground = color("success", light);
        c.button_success_hover = color("success", light);
        c.button_warning = color("warning", light);
        c.button_warning_active = color("warning", light);
        c.button_warning_foreground = color("warningForeground", light);
        c.button_warning_hover = color("warning", light);
        c.group_box = color("surface", light);
        c.group_box_foreground = color("text", light);
        c.caret = color("text", light);
        c.chart_1 = color("accent", light);
        c.chart_2 = color("success", light);
        c.chart_3 = color("warning", light);
        c.chart_4 = color("cyan", light);
        c.chart_5 = color("magenta", light);
        c.chart_bullish = color("success", light);
        c.chart_bearish = color("error", light);
        c.chart_grid = color("border", light);
        c.danger = color("error", light);
        c.danger_active = color("error", light);
        c.danger_foreground = color("errorForeground", light);
        c.danger_hover = color("error", light);
        c.description_list_label = color("surface", light);
        c.description_list_label_foreground = color("text", light);
        c.drag_border = color("border", light);
        c.drop_target = color("surface", light);
        c.foreground = color("text", light);
        c.info = color("accent", light);
        c.info_active = color("accent", light);
        c.info_foreground = color("accent", light);
        c.info_hover = color("accent", light);
        c.input = color("input", light);
        c.link = color("accent", light);
        c.link_active = color("accent", light);
        c.link_hover = color("accent", light);
        c.list = color("surface", light);
        c.list_active = color("muted", light);
        c.list_active_border = color("muted", light);
        c.list_even = color("surface", light);
        c.list_head = color("surface", light);
        c.list_hover = color("rowHover", light);
        c.muted = color("surface", light);
        c.muted_foreground = color("textMuted", light);
        c.popover = color("surface", light);
        c.popover_foreground = color("text", light);
        c.primary = color("accent", light);
        c.primary_active = color("accent", light);
        c.primary_foreground = color("accentForeground", light);
        c.primary_hover = color("accent", light);
        c.progress_bar = color("accent", light);
        c.ring = color("accent", light);
        c.scrollbar = color("canvas", light);
        c.scrollbar_thumb = color("scrollbarThumb", light);
        c.scrollbar_thumb_hover = color("scrollbarHover", light);
        c.secondary = color("surface", light);
        c.secondary_active = color("muted", light);
        c.secondary_foreground = color("text", light);
        c.secondary_hover = color("muted", light);
        c.selection = color("selection", light);
        c.sidebar = color("sidebar", light);
        c.sidebar_accent = color("surface", light);
        c.sidebar_accent_foreground = color("text", light);
        c.sidebar_border = color("border", light);
        c.sidebar_foreground = color("text", light);
        c.sidebar_primary = color("accent", light);
        c.sidebar_primary_foreground = color("accentForeground", light);
        c.skeleton = color("surface", light);
        c.slider_bar = color("accent", light);
        c.slider_thumb = color("text", light);
        c.success = color("success", light);
        c.success_foreground = color("success", light);
        c.success_hover = color("success", light);
        c.success_active = color("success", light);
        c.switch = color("surface", light);
        c.switch_thumb = color("text", light);
        c.tab = color("surface", light);
        c.tab_active = color("muted", light);
        c.tab_active_foreground = color("text", light);
        c.tab_bar = color("surface", light);
        c.tab_bar_segmented = color("surface", light);
        c.tab_foreground = color("text", light);
        c.table = color("surface", light);
        c.table_active = color("muted", light);
        c.table_active_border = color("muted", light);
        c.table_even = color("surface", light);
        c.table_head = color("surface", light);
        c.table_head_foreground = color("text", light);
        c.table_foot = color("surface", light);
        c.table_foot_foreground = color("text", light);
        c.table_hover = color("muted", light);
        c.table_row_border = color("border", light);
        c.title_bar = color("surface", light);
        c.title_bar_border = color("border", light);
        c.status_bar = color("surface", light);
        c.status_bar_border = color("border", light);
        c.warning = color("warning", light);
        c.warning_active = color("warning", light);
        c.warning_hover = color("warning", light);
        c.warning_foreground = color("warningForeground", light);
        c.overlay = color("surface", light);
        c.window_border = color("border", light);
        c.red = color("error", light);
        c.red_light = color("error", light);
        c.green = color("success", light);
        c.green_light = color("success", light);
        c.blue = color("accent", light);
        c.blue_light = color("accent", light);
        c.yellow = color("warning", light);
        c.yellow_light = color("warning", light);
        c.magenta = color("magenta", light);
        c.magenta_light = color("magenta", light);
        c.cyan = color("cyan", light);
        c.cyan_light = color("cyan", light);
        theme.radius = px(8.);
        theme.radius_lg = px(12.);
        theme.font_family = "Inter Variable".into();
    });
    let base = gpui_kit::base::Theme::global_mut(cx);
    base.scrollbar = base.scrollbar.clone().with_styles(
        gpui_kit::base::ScrollbarStyles::default()
            .track(|style| style.width(px(6.)))
            .track_hover(|style| style.width(px(6.)))
            .track_active(|style| style.width(px(6.)))
            .thumb(|style| style.width(px(6.)).inset(px(0.)))
            .thumb_hover(|style| style.width(px(6.)).inset(px(0.)))
            .thumb_active(|style| style.width(px(6.)).inset(px(0.))),
    );
}

fn color(role: &str, light: bool) -> Hsla {
    let (dark, bright) = match role {
        "canvas" => (0x000000, 0xfcfcfc),
        "surface" => (0x111111, 0xffffff),
        "text" => (0xf5f5f5, 0x27272a),
        "textMuted" => (0x818181, 0x71717b),
        "border" => (0x191919, 0xe4e4e7),
        "muted" => (0x111111, 0xfafafa),
        "accent" => (0x346bf1, 0x1b4ed8),
        "accentForeground" => (0xffffff, 0xffffff),
        "error" => (0xfb414a, 0xfb2c36),
        "errorForeground" => (0xff6467, 0xc10007),
        "warning" => (0xfe9a00, 0xfe9a00),
        "warningForeground" => (0xffb900, 0xbb4d00),
        "success" => (0x00a63e, 0x00a63e),
        "cyan" => (0x00b8db, 0x0092b8),
        "magenta" => (0xc27aff, 0x9810fa),
        "selection" => (0x343a47, 0xd0d6dd),
        "input" => (0x1e1e1e, 0xd4d4d8),
        "scrollbarThumb" => (0x222222, 0xd6d6d6),
        "scrollbarHover" => (0x363636, 0xbdbdbd),
        "sidebar" => (0x000000, 0xfafafa),
        "rowHover" => (0x131313, 0xfcfcfc),
        _ => unreachable!("unknown T3 color role"),
    };
    rgb(if light { bright } else { dark }).into()
}

pub fn message(cx: &App) -> Hsla {
    use gpui_kit::component::ActiveTheme as _;
    rgb(if cx.theme().mode.is_dark() {
        MESSAGE
    } else {
        0xebebed
    })
    .into()
}
