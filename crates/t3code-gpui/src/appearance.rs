use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

pub const MESSAGE: u32 = 0x1e1e1e;
pub const SURFACE: u32 = 0x111111;
pub const ROW_INSET: f32 = 10.;

pub fn apply(cx: &mut App) {
    cx.text_system()
        .add_fonts(vec![std::borrow::Cow::Borrowed(include_bytes!(
            "../assets/InterVariable.ttf"
        ))])
        .expect("load bundled Inter font");
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |theme| {
        let c = &mut theme.colors;
        c.background = rgb(0x000000).into();
        c.foreground = rgb(0xf5f5f5).into();
        c.border = rgb(0x191919).into();
        c.secondary = rgb(SURFACE).into();
        c.secondary_foreground = c.foreground;
        c.muted = c.secondary;
        c.muted_foreground = rgb(0x818181).into();
        c.input = rgb(MESSAGE).into();
        c.popover = c.secondary;
        c.popover_foreground = c.foreground;
        c.accent = rgb(0x346bf1).into();
        c.accent_foreground = rgb(0xffffff).into();
        c.primary = c.accent;
        c.primary_foreground = c.accent_foreground;
        c.ring = c.accent;
        c.caret = c.foreground;
        c.sidebar = c.background;
        c.sidebar_foreground = rgb(0xf1f3f7).into();
        c.list_active = rgb(0x111111).into();
        c.list_hover = rgb(0x131313).into();
        c.button = c.secondary;
        c.button_foreground = c.foreground;
        c.button_hover = rgb(0x141414).into();
        c.button_active = c.list_active;
        c.button_primary = c.primary;
        c.button_primary_foreground = c.primary_foreground;
        c.button_primary_hover = rgb(0x3061d9).into();
        c.scrollbar = c.background;
        c.scrollbar_thumb = rgb(0x222222).into();
        c.scrollbar_thumb_hover = rgb(0x363636).into();
        theme.radius = px(8.);
        theme.radius_lg = px(12.);
        theme.font_family = "Inter".into();
    });
}
