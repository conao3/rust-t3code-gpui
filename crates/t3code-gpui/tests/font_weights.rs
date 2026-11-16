#![cfg(all(target_os = "linux", not(feature = "gpui-fast")))]

use std::borrow::Cow;

use gpui_kit::{
    Font, FontFallbacks, FontRun, FontStyle, FontWeight, PlatformTextSystem, RenderGlyphParams,
    font, point, px,
};
use gpui_wgpu::CosmicTextSystem;

fn text_system() -> CosmicTextSystem {
    let system = CosmicTextSystem::new_without_system_fonts("Inter Variable");
    system
        .add_fonts(vec![
            Cow::Borrowed(include_bytes!("../assets/InterVariable.ttf")),
            Cow::Borrowed(include_bytes!("fixtures/FiraMono-Medium.ttf")),
        ])
        .unwrap();
    system
}

#[test]
fn variable_weights_use_distinct_cached_instances_and_matching_layout_and_rasterization() {
    let system = text_system();
    let mut identifiers = Vec::new();
    let mut coverage = Vec::new();
    for weight in [FontWeight::NORMAL, FontWeight::MEDIUM, FontWeight::BOLD] {
        let request = Font {
            weight,
            ..font("Inter Variable")
        };
        let id = system.font_id(&request).unwrap();
        assert_eq!(system.font_id(&request).unwrap(), id);
        assert_eq!(
            system.font_weight_and_style(id).unwrap(),
            (weight, FontStyle::Normal)
        );
        identifiers.push(id);
        let glyph = system.glyph_for_char(id, 'M').unwrap();
        let metrics = system.font_metrics(id);
        let advance = system.advance(id, glyph).unwrap().width;
        let layout = system.layout_line(
            "MMMM",
            px(20.),
            &[FontRun {
                len: 4,
                font_id: id,
            }],
        );
        assert!(
            (f32::from(layout.width) - 4. * advance / metrics.units_per_em as f32 * 20.).abs()
                < 0.05
        );
        assert!(layout.runs.iter().all(|run| run.font_id == id));
        let params = RenderGlyphParams {
            font_id: id,
            glyph_id: glyph,
            font_size: px(20.),
            subpixel_variant: point(0, 0),
            scale_factor: 1.,
            is_emoji: false,
            subpixel_rendering: false,
            dilation: 0,
        };
        let bounds = system.glyph_raster_bounds(&params).unwrap();
        let (_, bitmap) = system.rasterize_glyph(&params, bounds).unwrap();
        coverage.push(bitmap.iter().map(|&alpha| u64::from(alpha)).sum::<u64>());
    }
    assert_ne!(identifiers[0], identifiers[1]);
    assert_ne!(identifiers[1], identifiers[2]);
    assert!(
        coverage[0] > 0 && coverage[0] < coverage[1] && coverage[1] < coverage[2],
        "{coverage:?}"
    );
}

#[test]
fn a_static_face_keeps_its_actual_weight_and_font_id() {
    let system = text_system();
    let selected = system.font_id(&font("Fira Mono")).unwrap();
    for weight in [FontWeight::NORMAL, FontWeight::MEDIUM, FontWeight::BOLD] {
        let id = system
            .font_id(&Font {
                weight,
                ..font("Fira Mono")
            })
            .unwrap();
        assert_eq!(id, selected);
        assert_eq!(
            system.font_weight_and_style(id).unwrap(),
            (FontWeight::MEDIUM, FontStyle::Normal)
        );
    }
}

#[test]
fn variable_fallbacks_keep_the_shaped_weight_when_their_metadata_names_a_regular_instance() {
    let system = text_system();
    let primary = system.font_id(&font("Fira Mono")).unwrap();
    let fallback = system.font_id(&font("Inter Variable")).unwrap();
    let character = (0x20..0x3000)
        .filter_map(char::from_u32)
        .find(|&character| {
            system.glyph_for_char(primary, character).is_none()
                && system.glyph_for_char(fallback, character).is_some()
        })
        .expect("fixture fonts have different coverage");
    let id = system
        .font_id(&Font {
            fallbacks: Some(FontFallbacks::from_fonts(vec!["Inter Variable".into()])),
            ..font("Fira Mono")
        })
        .unwrap();
    let text = character.to_string();
    let runs = [FontRun {
        len: text.len(),
        font_id: id,
    }];
    let layout = system.layout_line(&text, px(20.), &runs);
    assert_eq!(layout.runs.len(), 1);
    let selected = layout.runs[0].font_id;
    assert_ne!(selected, fallback);
    assert_eq!(
        system.font_weight_and_style(selected).unwrap(),
        (FontWeight::MEDIUM, FontStyle::Normal)
    );
    assert_eq!(
        system.layout_line(&text, px(20.), &runs).runs[0].font_id,
        selected
    );
}
