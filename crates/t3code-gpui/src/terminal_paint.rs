use std::sync::Arc;

use gpui_kit::*;
use t3_terminal::{Cell, Color, Row, Snapshot};

pub const PADDING: f32 = 4.;

#[derive(Clone, Debug, PartialEq)]
pub struct Typography {
    pub family: SharedString,
    pub size: f32,
    pub width: f32,
    pub height: f32,
    pub baseline: f32,
    pub scale: f32,
}

impl Typography {
    pub fn measure(family: SharedString, size: f32, window: &Window) -> Self {
        let text = window.text_system();
        let normal = font(family.clone());
        let run = |len| TextRun {
            len,
            font: normal.clone(),
            color: black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let width = f32::from(text.shape_line("M".into(), px(size), &[run(1)], None).width).max(1.);
        let line = text.shape_line("Mg".into(), px(size), &[run(2)], None);
        let scale = window.scale_factor();
        let mut top = 0_f32;
        let mut bottom = 0_f32;
        for run in &line.runs {
            for glyph in &run.glyphs {
                let params = RenderGlyphParams {
                    font_id: run.font_id,
                    glyph_id: glyph.id,
                    font_size: px(size),
                    subpixel_variant: point(0, 0),
                    scale_factor: scale,
                    is_emoji: glyph.is_emoji,
                    subpixel_rendering: false,
                    dilation: 0,
                };
                if let Ok(bounds) = text.raster_bounds(&params) {
                    top = top.min(bounds.origin.y.0 as f32 / scale + f32::from(glyph.position.y));
                    bottom = bottom.max(
                        (bounds.origin.y.0 + bounds.size.height.0) as f32 / scale
                            + f32::from(glyph.position.y),
                    );
                }
            }
        }
        let ascent = if top < 0. { -top } else { size };
        let glyph_height = ascent + bottom;
        let height = (size * 1.35).round().max(glyph_height.ceil()).max(1.);
        Self {
            family,
            size,
            width,
            height,
            baseline: ((height - glyph_height) / 2. + ascent).round(),
            scale,
        }
    }

    pub fn shape(
        &self,
        text: SharedString,
        cell: &Cell,
        color: Hsla,
        window: &Window,
    ) -> ShapedLine {
        let mut face = font(self.family.clone());
        face.weight = if cell.bold {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        };
        face.style = if cell.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        };
        window.text_system().shape_line(
            text.clone(),
            px(self.size),
            &[TextRun {
                len: text.len(),
                font: face,
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        )
    }
}

pub fn color(color: Color) -> Hsla {
    rgb((u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)).into()
}

struct Text {
    start: usize,
    end: usize,
    line: ShapedLine,
}

struct CachedRow {
    source: Arc<Row>,
    runs: Vec<Text>,
}

#[derive(Default)]
pub struct Cache {
    typography: Option<Typography>,
    rows: Vec<CachedRow>,
}

fn same_style(left: &Cell, right: &Cell) -> bool {
    left.foreground == right.foreground
        && left.bold == right.bold
        && left.italic == right.italic
        && left.invisible == right.invisible
}

fn separator(text: &str) -> bool {
    matches!(text, "\u{e0b0}" | "\u{e0b2}")
}

fn shape(row: &Row, typography: &Typography, window: &Window) -> Vec<Text> {
    let mut runs = Vec::new();
    let mut start = 0;
    while let Some(first) = row.cells.get(start) {
        if first.text.is_empty() || separator(&first.text) {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while let Some(next) = row.cells.get(end) {
            if next.wide == 2 {
                end += 1;
                continue;
            }
            if next.text.is_empty() || separator(&next.text) || !same_style(first, next) {
                break;
            }
            end += 1;
        }
        let text: String = row.cells[start..end]
            .iter()
            .map(|cell| cell.text.as_str())
            .collect();
        if !first.invisible && !text.trim().is_empty() {
            runs.push(Text {
                start,
                end,
                line: typography.shape(text.into(), first, color(first.foreground), window),
            });
        }
        start = end;
    }
    runs
}

fn rect(origin: Point<Pixels>, x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
    Bounds::new(origin + point(px(x), px(y)), size(px(width), px(height)))
}

fn triangle(text: &str, bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    let (base, tip) = if text == "\u{e0b0}" {
        (bounds.left(), bounds.right())
    } else {
        (bounds.right(), bounds.left())
    };
    let mut path = PathBuilder::fill();
    path.move_to(point(base, bounds.top()));
    path.line_to(point(tip, bounds.top() + bounds.size.height / 2.));
    path.line_to(point(base, bounds.bottom()));
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn paint_text(
    line: &ShapedLine,
    bounds: Bounds<Pixels>,
    typography: &Typography,
    window: &mut Window,
    cx: &mut App,
) {
    let padding = (typography.height - f32::from(line.ascent) - f32::from(line.descent)) / 2.;
    let origin = bounds.origin
        + point(
            px(0.),
            px(typography.baseline - padding - f32::from(line.ascent)),
        );
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        let _ = line.paint(
            origin,
            px(typography.height),
            TextAlign::Left,
            None,
            window,
            cx,
        );
    });
}

pub struct PaintState {
    pub selection: Hsla,
    pub cursor_on: bool,
    pub focused: bool,
}

impl Cache {
    pub fn paint(
        &mut self,
        snapshot: &Snapshot,
        typography: &Typography,
        bounds: Bounds<Pixels>,
        state: PaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let PaintState {
            selection,
            cursor_on,
            focused,
        } = state;
        if self.typography.as_ref() != Some(typography) {
            self.rows.clear();
            self.typography = Some(typography.clone());
        }
        self.rows.truncate(snapshot.row_data.len());
        window.paint_quad(fill(bounds, color(snapshot.background)));
        let origin = bounds.origin + point(px(PADDING), px(PADDING));
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for (index, row) in snapshot.row_data.iter().enumerate() {
                if self
                    .rows
                    .get(index)
                    .is_none_or(|cached| !Arc::ptr_eq(&cached.source, row))
                {
                    let cached = CachedRow {
                        source: row.clone(),
                        runs: shape(row, typography, window),
                    };
                    if index == self.rows.len() {
                        self.rows.push(cached);
                    } else {
                        self.rows[index] = cached;
                    }
                }
                let top = index as f32 * typography.height;
                let mut start = 0;
                while let Some(first) = row.cells.get(start) {
                    let mut end = start + 1;
                    while row.cells.get(end).is_some_and(|next| {
                        next.background == first.background && next.selected == first.selected
                    }) {
                        end += 1;
                    }
                    let bounds = rect(
                        origin,
                        start as f32 * typography.width,
                        top,
                        (end - start) as f32 * typography.width,
                        typography.height,
                    );
                    if first.background != snapshot.background {
                        window.paint_quad(fill(bounds, color(first.background)));
                    }
                    if first.selected {
                        window.paint_quad(fill(bounds, selection));
                    }
                    start = end;
                }
                for run in &self.rows[index].runs {
                    paint_text(
                        &run.line,
                        rect(
                            origin,
                            run.start as f32 * typography.width,
                            top,
                            (run.end - run.start) as f32 * typography.width,
                            typography.height,
                        ),
                        typography,
                        window,
                        cx,
                    );
                }
                for (column, cell) in row.cells.iter().enumerate() {
                    let left = column as f32 * typography.width;
                    if separator(&cell.text) && !cell.invisible {
                        triangle(
                            &cell.text,
                            rect(origin, left, top, typography.width, typography.height),
                            color(cell.foreground),
                            window,
                        );
                    }
                    for (enabled, offset) in [
                        (cell.underline, typography.height - 2.),
                        (cell.strikethrough, (typography.height * 0.55).floor()),
                        (cell.overline, 1.),
                    ] {
                        if enabled {
                            window.paint_quad(fill(
                                rect(origin, left, top + offset, typography.width, 1.),
                                color(cell.foreground),
                            ));
                        }
                    }
                }
            }
            if cursor_on
                && snapshot.cursor_visible
                && snapshot.cursor_x >= 0
                && snapshot.cursor_y >= 0
            {
                let bounds = rect(
                    origin,
                    snapshot.cursor_x as f32 * typography.width,
                    snapshot.cursor_y as f32 * typography.height,
                    typography.width,
                    typography.height,
                );
                let cursor = color(snapshot.cursor);
                if !focused || snapshot.cursor_style == 3 {
                    for (x, y, width, height) in [
                        (0., 0., typography.width, 1.),
                        (0., typography.height - 1., typography.width, 1.),
                        (0., 0., 1., typography.height),
                        (typography.width - 1., 0., 1., typography.height),
                    ] {
                        window.paint_quad(fill(rect(bounds.origin, x, y, width, height), cursor));
                    }
                } else if snapshot.cursor_style == 0 {
                    window.paint_quad(fill(
                        Bounds::new(bounds.origin, size(px(2.), bounds.size.height)),
                        cursor,
                    ));
                } else if snapshot.cursor_style == 2 {
                    window.paint_quad(fill(
                        rect(
                            bounds.origin,
                            0.,
                            typography.height - 2.,
                            typography.width,
                            2.,
                        ),
                        cursor,
                    ));
                } else {
                    window.paint_quad(fill(bounds, cursor));
                    if let Some(cell) = snapshot
                        .row_data
                        .get(snapshot.cursor_y as usize)
                        .and_then(|row| row.cells.get(snapshot.cursor_x as usize))
                        .filter(|cell| !cell.text.is_empty())
                    {
                        if separator(&cell.text) {
                            triangle(&cell.text, bounds, color(snapshot.background), window);
                        } else {
                            let line = typography.shape(
                                cell.text.clone().into(),
                                cell,
                                color(snapshot.background),
                                window,
                            );
                            paint_text(&line, bounds, typography, window, cx);
                        }
                    }
                }
            }
        });
    }
}
