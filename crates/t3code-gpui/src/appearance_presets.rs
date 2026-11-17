use gpui_kit::{Global, Hsla, Rgba, rgb};
use serde_json::Value;
use std::sync::OnceLock;

#[derive(Clone, Copy)]
pub struct Palette {
    pub colors: Option<&'static Value>,
    pub light: bool,
    pub contrast: f32,
    pub glass: f32,
}

impl Global for Palette {}

pub fn themes() -> &'static [Value] {
    static THEMES: OnceLock<Vec<Value>> = OnceLock::new();
    THEMES.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/settings-themes.json"))
            .expect("bundled T3 theme palettes")
    })
}

pub fn palette(id: &str, light: bool) -> Palette {
    let colors = if id == "t3-code" {
        None
    } else {
        let theme = themes()
            .iter()
            .find(|theme| theme["id"] == id)
            .expect("known theme");
        Some(if light {
            &theme["colors"]
        } else {
            &theme["variants"]["dark"]
        })
    };
    Palette {
        colors,
        light,
        contrast: 100.,
        glass: 80.,
    }
}

impl Palette {
    fn raw(self, role: &str) -> Hsla {
        if let Some(colors) = self.colors {
            if let Some(value) = colors.get(role) {
                return rgb(u32::from_str_radix(
                    value.as_str().expect("theme color").trim_start_matches('#'),
                    16,
                )
                .expect("hex theme color"))
                .into();
            }
            assert!(
                matches!(
                    role,
                    "success"
                        | "cyan"
                        | "magenta"
                        | "selection"
                        | "scrollbarThumb"
                        | "scrollbarHover"
                ),
                "missing theme role: {role}"
            );
        }
        crate::appearance::color(role, self.light)
    }
}

impl Palette {
    pub fn color(self, role: &str) -> Hsla {
        let original = self.raw(role);
        let base = self.contrast.min(100.) / 100.;
        let boost = (self.contrast - 100.).max(0.) / 100.;
        if matches!(role, "border" | "input" | "sidebarBorder") {
            let foreground = self.raw(if role == "sidebarBorder" {
                "sidebarForeground"
            } else {
                "text"
            });
            let mut foreground: Rgba = foreground.into();
            foreground.a = boost / 4.;
            return Rgba::from(original.opacity(base)).blend(foreground).into();
        }
        let background = match role {
            "text" | "textMuted" | "secondaryLabel" => "canvas",
            "sidebarForeground" | "sidebarMutedForeground" => "sidebar",
            "accentSurfaceForeground" => "accentSurface",
            "accentForeground" => "accent",
            "messageForeground" => "messageSurface",
            _ => return original,
        };
        let target: Hsla = rgb(if self.light { 0 } else { 0xffffff }).into();
        mix_oklab(
            mix_oklab(self.raw(background), original, base),
            target,
            boost,
        )
    }
}

fn mix_oklab(from: Hsla, to: Hsla, amount: f32) -> Hsla {
    let lab = |color: Hsla| {
        let color: Rgba = color.into();
        let linear = |c: f32| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let (r, g, b) = (linear(color.r), linear(color.g), linear(color.b));
        let l = (0.41222146 * r + 0.53633255 * g + 0.051445995 * b).cbrt();
        let m = (0.2119035 * r + 0.6806995 * g + 0.10739696 * b).cbrt();
        let s = (0.08830246 * r + 0.28171885 * g + 0.6299787 * b).cbrt();
        [
            0.21045426 * l + 0.7936178 * m - 0.004072047 * s,
            1.9779985 * l - 2.4285922 * m + 0.4505937 * s,
            0.025904037 * l + 0.78277177 * m - 0.80867577 * s,
        ]
    };
    let a = lab(from);
    let b = lab(to);
    let [l, a, b] = std::array::from_fn(|i| a[i] * (1. - amount) + b[i] * amount);
    let ll = (l + 0.39633778 * a + 0.21580376 * b).powi(3);
    let mm = (l - 0.105561346 * a - 0.06385417 * b).powi(3);
    let ss = (l - 0.08948418 * a - 1.2914855 * b).powi(3);
    let srgb = |c: f32| {
        (if c <= 0.0031308 {
            c * 12.92
        } else {
            1.055 * c.powf(1. / 2.4) - 0.055
        })
        .clamp(0., 1.)
    };
    Rgba {
        r: srgb(4.0767417 * ll - 3.3077116 * mm + 0.23096994 * ss),
        g: srgb(-1.268438 * ll + 2.6097574 * mm - 0.3413194 * ss),
        b: srgb(-0.0041960863 * ll - 0.7034186 * mm + 1.7076147 * ss),
        a: 1.,
    }
    .into()
}

pub fn preview(path: &str) -> Option<Vec<u8>> {
    static PREVIEWS: OnceLock<Value> = OnceLock::new();
    PREVIEWS
        .get_or_init(|| {
            serde_json::from_str(include_str!("../assets/settings-theme-previews.json"))
                .expect("bundled theme previews")
        })
        .get(path)?
        .as_str()
        .map(|svg| svg.as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_theme_has_complete_light_and_dark_palettes() {
        let roles = themes()[0]["colors"].as_object().expect("colors");
        assert_eq!(roles.len(), 61);
        assert_eq!(themes().len(), 5);
        for theme in themes() {
            for light in [true, false] {
                let palette = palette(theme["id"].as_str().expect("id"), light);
                for role in roles.keys() {
                    let color = palette.color(role);
                    assert!(color.h.is_finite() && color.s.is_finite() && color.l.is_finite());
                }
            }
        }
        assert_ne!(
            palette("grove", true).color("canvas"),
            palette("grove", false).color("canvas")
        );
    }
}
