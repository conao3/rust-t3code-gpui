use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

pub const MESSAGE: u32 = 0x1e1e1e;
pub const ROW_INSET: f32 = 10.;

pub fn default_code_family(cx: &App) -> String {
    let installed = cx.text_system().all_font_names();
    [
        "SF Mono",
        "SFMono-Regular",
        "Menlo",
        "Consolas",
        "Liberation Mono",
    ]
    .into_iter()
    .find(|name| installed.iter().any(|family| family == name))
    .unwrap_or("monospace")
    .to_string()
}

pub fn apply_typography(settings: &crate::client_settings::ClientSettings, cx: &mut App) {
    let code = settings.values["fontFamilyCode"]
        .as_str()
        .filter(|name| !name.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| default_code_family(cx));
    Theme::update(cx, |theme| {
        theme.font_family = settings.values["fontFamilySans"]
            .as_str()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("Inter Variable")
            .to_string()
            .into();
        theme.mono_font_family = code.into();
        theme.font_size = px(settings.font_size_interface);
        theme.mono_font_size = px(settings.font_size_code);
    });
}

pub fn preset_color(id: &str, role: &str, light: bool) -> Hsla {
    crate::appearance_presets::palette(id, light).color(role)
}

pub fn apply(settings: &crate::client_settings::ClientSettings, cx: &mut App) {
    let light = match settings.values["gpuiAppearance"].as_str() {
        Some("system") => matches!(
            cx.window_appearance(),
            WindowAppearance::Light | WindowAppearance::VibrantLight
        ),
        _ => settings.light_theme,
    };
    let id = settings.values[if light {
        "gpuiThemeLight"
    } else {
        "gpuiThemeDark"
    }]
    .as_str()
    .unwrap_or("t3-code");
    let mut palette = crate::appearance_presets::palette(id, light);
    palette.contrast = settings.values["appearanceContrast"]
        .as_f64()
        .unwrap_or(100.) as f32;
    palette.glass = settings.values["glassOpacity"].as_f64().unwrap_or(80.) as f32;
    let initialized = cx.has_global::<crate::appearance_presets::Palette>();
    cx.set_global(palette);
    let color = move |role: &str, _: bool| palette.color(role);
    cx.set_text_rendering_mode(TextRenderingMode::Grayscale);
    if !initialized {
        cx.text_system()
            .add_fonts(vec![std::borrow::Cow::Borrowed(include_bytes!(
                "../assets/InterVariable.ttf"
            ))])
            .expect("load bundled Inter font");
    }
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
        c.accent = color("accentSurface", light);
        c.accent_foreground = color("accentSurfaceForeground", light);
        c.accordion = color("surface", light);
        c.background = color("canvas", light);
        c.border = color("border", light);
        c.button = color("surface", light);
        c.button_active = color("accentSurface", light);
        c.button_foreground = color("text", light);
        c.button_hover = color("accentSurface", light);
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
        c.button_primary_hover = color("messageActionHover", light);
        c.button_secondary = color("muted", light);
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
        c.list_active = color("text", light).opacity(0.09);
        c.list_active_border = transparent_black();
        c.list_even = color("surface", light);
        c.list_head = color("surface", light);
        c.list_hover = color("text", light).opacity(0.09);
        c.muted = color("muted", light);
        c.muted_foreground = color("textMuted", light);
        c.popover = color("surface", light).opacity(palette.glass / 100.);
        c.popover_foreground = color("text", light);
        c.primary = color("accent", light);
        c.primary_active = color("accent", light);
        c.primary_foreground = color("accentForeground", light);
        c.primary_hover = color("messageActionHover", light);
        c.progress_bar = color("accent", light);
        c.ring = color("accent", light);
        c.scrollbar = color("canvas", light);
        c.scrollbar_thumb = color("scrollbarThumb", light);
        c.scrollbar_thumb_hover = color("scrollbarHover", light);
        c.secondary = color("muted", light);
        c.secondary_active = color("muted", light);
        c.secondary_foreground = color("text", light);
        c.secondary_hover = color("muted", light);
        c.selection = color("selection", light);
        c.sidebar = color("sidebar", light);
        c.sidebar_accent = color("sidebarControlSurface", light);
        c.sidebar_accent_foreground = color("sidebarForeground", light);
        c.sidebar_border = color("sidebarBorder", light);
        c.sidebar_foreground = color("sidebarForeground", light);
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
        c.title_bar = color("canvas", light);
        c.title_bar_border = color("border", light);
        c.status_bar = color("surface", light);
        c.status_bar_border = color("border", light);
        c.warning = color("warning", light);
        c.warning_active = color("warning", light);
        c.warning_hover = color("warning", light);
        c.warning_foreground = color("warningForeground", light);
        c.overlay = color("canvas", light).opacity(if light { 0.60 } else { 0.64 });
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
        theme.highlight_theme =
            serde_json::from_str::<gpui_kit::component::highlighter::HighlightTheme>(if light {
                include_str!("../assets/pierre-light.json")
            } else {
                include_str!("../assets/pierre-dark.json")
            })
            .expect("load bundled T3 syntax palette")
            .into();
        theme.radius = px(8.);
        theme.radius_lg = px(12.);
        theme.font_family = "Inter Variable".into();
    });
    let base = gpui_kit::base::Theme::global_mut(cx);
    base.scrollbar = base.scrollbar.clone().with_styles(
        gpui_kit::base::ScrollbarStyles::default()
            .track(|style| style.width(px(6.)).bg(transparent_black()))
            .track_hover(|style| style.width(px(6.)).bg(transparent_black()))
            .track_active(|style| style.width(px(6.)).bg(transparent_black()))
            .thumb(|style| {
                style
                    .width(px(6.))
                    .inset(px(0.))
                    .bg(color("scrollbarThumb", light))
            })
            .thumb_hover(|style| {
                style
                    .width(px(6.))
                    .inset(px(0.))
                    .bg(color("scrollbarHover", light))
            })
            .thumb_active(|style| {
                style
                    .width(px(6.))
                    .inset(px(0.))
                    .bg(color("scrollbarHover", light))
            }),
    );
}

// Standard roles from T3 packages/shared/src/themePalettes.ts. Message surfaces
// use the web overrides in apps/web/src/themePalette.ts, not the mobile palette.
pub(crate) fn color(role: &str, light: bool) -> Hsla {
    let (dark, bright) = match role {
        "canvas" => (0x000000, 0xfcfcfc),
        "surface" => (0x111111, 0xffffff),
        "surfaceRaised" => (0x111111, 0xfcfcfc),
        "text" => (0xf5f5f5, 0x27272a),
        "textMuted" => (0x818181, 0x71717b),
        "border" => (0x191919, 0xe4e4e7),
        "muted" => (0x111111, 0xfafafa),
        "accent" => (0x346bf1, 0x1b4ed8),
        "accentSurface" => (0x141414, 0xf4f4f5),
        "accentSurfaceForeground" => (0xf5f5f5, 0x18181b),
        "messageActionHover" => (0x3061d9, 0x3160db),
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
        "sidebarForeground" => (0xf1f3f7, 0x27272a),
        "sidebarBorder" => (0x141414, 0xe4e4e7),
        "sidebarControlSurface" => (0x0a0a0a, 0xf4f4f5),
        "sidebarRowActive" => (0x1a1b1b, 0xffffff),
        "sidebarRowHover" => (0x131313, 0xfcfcfc),
        "sidebarMutedForeground" => (0xa3a3a3, 0x71717b),
        "secondaryLabel" => (0x8f8f8f, 0x71717b),
        "codeBackground" => (0x111111, 0xffffff),
        _ => unreachable!("unknown T3 color role"),
    };
    rgb(if light { bright } else { dark }).into()
}

pub fn composer(cx: &App) -> Hsla {
    themed_color("surface", cx)
        .opacity(cx.global::<crate::appearance_presets::Palette>().glass / 100.)
}

pub fn message(cx: &App) -> Hsla {
    use gpui_kit::component::ActiveTheme as _;
    let palette = cx.global::<crate::appearance_presets::Palette>();
    if palette.colors.is_some() {
        return palette.color("messageSurface");
    }
    rgb(if cx.theme().mode.is_dark() {
        MESSAGE
    } else {
        0xebebed
    })
    .into()
}

/// Sidebar roles are independent of menu/list selection and hover surfaces.
pub fn sidebar_active(cx: &App) -> Hsla {
    themed_color("sidebarRowActive", cx)
}

pub fn sidebar_hover(cx: &App) -> Hsla {
    themed_color("sidebarRowHover", cx)
}

pub fn sidebar_muted(cx: &App) -> Hsla {
    themed_color("sidebarMutedForeground", cx)
}

pub fn secondary_label(cx: &App) -> Hsla {
    themed_color("secondaryLabel", cx)
}

pub fn code_background(cx: &App) -> Hsla {
    themed_color("codeBackground", cx)
}

fn themed_color(role: &str, cx: &App) -> Hsla {
    cx.global::<crate::appearance_presets::Palette>()
        .color(role)
}

const EDITOR_ASSETS: &[(&str, &[u8])] = &[
    (
        "icons/editors/antigravity.svg",
        include_bytes!("../assets/editors/antigravity.svg"),
    ),
    (
        "icons/editors/aqua.svg",
        include_bytes!("../assets/editors/aqua.svg"),
    ),
    (
        "icons/editors/clion.svg",
        include_bytes!("../assets/editors/clion.svg"),
    ),
    (
        "icons/editors/cursor.svg",
        include_bytes!("../assets/editors/cursor.svg"),
    ),
    (
        "icons/editors/datagrip.svg",
        include_bytes!("../assets/editors/datagrip.svg"),
    ),
    (
        "icons/editors/dataspell.svg",
        include_bytes!("../assets/editors/dataspell.svg"),
    ),
    (
        "icons/editors/goland.svg",
        include_bytes!("../assets/editors/goland.svg"),
    ),
    (
        "icons/editors/idea.svg",
        include_bytes!("../assets/editors/idea.svg"),
    ),
    (
        "icons/editors/kiro.svg",
        include_bytes!("../assets/editors/kiro.svg"),
    ),
    (
        "icons/editors/phpstorm.svg",
        include_bytes!("../assets/editors/phpstorm.svg"),
    ),
    (
        "icons/editors/pycharm.svg",
        include_bytes!("../assets/editors/pycharm.svg"),
    ),
    (
        "icons/editors/rider.svg",
        include_bytes!("../assets/editors/rider.svg"),
    ),
    (
        "icons/editors/rubymine.svg",
        include_bytes!("../assets/editors/rubymine.svg"),
    ),
    (
        "icons/editors/rustrover.svg",
        include_bytes!("../assets/editors/rustrover.svg"),
    ),
    (
        "icons/editors/trae.svg",
        include_bytes!("../assets/editors/trae.svg"),
    ),
    (
        "icons/editors/vscode-insiders.svg",
        include_bytes!("../assets/editors/vscode-insiders.svg"),
    ),
    (
        "icons/editors/vscode.svg",
        include_bytes!("../assets/editors/vscode.svg"),
    ),
    (
        "icons/editors/vscodium.svg",
        include_bytes!("../assets/editors/vscodium.svg"),
    ),
    (
        "icons/editors/webstorm.svg",
        include_bytes!("../assets/editors/webstorm.svg"),
    ),
    (
        "icons/editors/zed.svg",
        include_bytes!("../assets/editors/zed.svg"),
    ),
];

pub struct Assets;
mod providers {
    include!("../assets/providers/assets.rs");
}
impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if let Some(data) = crate::appearance_presets::preview(path) {
            return Ok(Some(std::borrow::Cow::Owned(data)));
        }
        if let Some((_, data)) = EDITOR_ASSETS.iter().find(|(name, _)| *name == path) {
            return Ok(Some(std::borrow::Cow::Borrowed(*data)));
        }
        if let Some((_, data)) = providers::ASSETS.iter().find(|(name, _)| *name == path) {
            return Ok(Some(std::borrow::Cow::Borrowed(*data)));
        }
        match path {
            "icons/t3-queue.svg" => Ok(Some(std::borrow::Cow::Borrowed(include_bytes!(
                "../assets/queue.svg"
            )))),
            "icons/t3-steer.svg" => Ok(Some(std::borrow::Cow::Borrowed(include_bytes!(
                "../assets/steer.svg"
            )))),
            "icons/t3-submit.svg" => Ok(Some(std::borrow::Cow::Borrowed(include_bytes!(
                "../assets/submit.svg"
            )))),
            "icons/pr-empty.svg" => Ok(Some(std::borrow::Cow::Borrowed(include_bytes!(
                "../assets/pr-empty.svg"
            )))),
            "icons/t3-stop.svg" => Ok(Some(std::borrow::Cow::Borrowed(include_bytes!(
                "../assets/stop.svg"
            )))),
            "icons/t3-openai.svg" => Ok(Some(std::borrow::Cow::Borrowed(include_bytes!(
                "../assets/openai.svg"
            )))),
            "icons/t3-claude.svg" => Ok(Some(std::borrow::Cow::Borrowed(include_bytes!(
                "../assets/claude.svg"
            )))),
            _ => gpui_kit::assets::AllAssets.load(path),
        }
    }
    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::AllAssets.list(path)?;
        paths.extend(
            providers::ASSETS
                .iter()
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| SharedString::from(*name)),
        );
        paths.extend(
            [
                "icons/t3-openai.svg",
                "icons/t3-claude.svg",
                "icons/t3-stop.svg",
                "icons/t3-queue.svg",
                "icons/t3-steer.svg",
                "icons/t3-submit.svg",
            ]
            .into_iter()
            .filter(|p| p.starts_with(path))
            .map(Into::into),
        );
        paths.extend(
            EDITOR_ASSETS
                .iter()
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| SharedString::from(*name)),
        );
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn provider_svgs_decode_with_bounded_square_viewports_and_visible_pixels() {
        use gpui_kit::SvgRenderer;
        let renderer = SvgRenderer::new(std::sync::Arc::new(super::Assets));
        for (path, bytes) in super::providers::ASSETS {
            let image = renderer
                .render_single_frame(bytes, 1.)
                .unwrap_or_else(|error| panic!("{path}: {error}"));
            let size = image.size(0);
            assert_eq!(size.width, size.height, "{path}");
            assert!(i32::from(size.width) <= 256, "{path}");
            assert!(
                image
                    .as_bytes(0)
                    .unwrap()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel[3] > 0),
                "{path}"
            );
        }
    }

    #[test]
    fn bundled_syntax_palettes_decode_for_the_pinned_highlighter() {
        for source in [
            include_str!("../assets/pierre-dark.json"),
            include_str!("../assets/pierre-light.json"),
        ] {
            let theme: gpui_kit::component::highlighter::HighlightTheme =
                serde_json::from_str(source).unwrap();
            assert!(theme.style.syntax.keyword.is_some());
            assert!(theme.style.syntax.string.is_some());
            assert!(theme.style.syntax.function.is_some());
        }
    }
}
