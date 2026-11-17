use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use wasmtime::TypedFunc;

use crate::runtime::{Runtime, checked};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Theme {
    pub foreground: Color,
    pub background: Color,
    pub cursor: Color,
    pub palette: Option<Vec<Color>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Cell {
    pub text: String,
    pub wide: u32,
    pub foreground: Color,
    pub background: Color,
    pub bold: bool,
    pub italic: bool,
    pub invisible: bool,
    pub strikethrough: bool,
    pub overline: bool,
    pub underline: bool,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub cells: Vec<Cell>,
    pub text: String,
    pub is_wrap_continuation: bool,
    pub wraps_to_next: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub cols: u16,
    pub rows: u16,
    pub foreground: Color,
    pub background: Color,
    pub cursor: Color,
    pub cursor_x: i32,
    pub cursor_y: i32,
    pub cursor_visible: bool,
    pub cursor_blinking: bool,
    pub cursor_style: u32,
    pub dirty_rows: BTreeSet<usize>,
    pub row_data: Vec<Arc<Row>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Key {
    pub key: i32,
    pub mods: i32,
    pub consumed_mods: i32,
    pub unshifted_codepoint: u32,
    pub text: String,
    pub action: i32,
    pub composing: bool,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseAction {
    Press,
    Release,
    Motion,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mouse {
    pub action: MouseAction,
    pub button: Option<i32>,
    pub mods: i32,
    pub x: f32,
    pub y: f32,
    pub screen_width: f32,
    pub screen_height: f32,
    pub cell_width: f32,
    pub cell_height: f32,
    pub padding_left: f32,
    pub padding_right: f32,
    pub padding_top: f32,
    pub padding_bottom: f32,
    pub any_button_pressed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Point {
    pub x: u32,
    pub y: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Range {
    pub start: Point,
    pub end: Point,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Selection {
    pub viewport: Range,
    pub screen: Range,
}

#[derive(Clone, Debug, Serialize)]
pub struct Scrollbar {
    pub total: u64,
    pub offset: u64,
    pub len: u64,
}

struct Hot {
    write: TypedFunc<(i32, i32, i32), ()>,
    next_row: TypedFunc<i32, i32>,
    next_cell: TypedFunc<i32, i32>,
    get_cell: TypedFunc<(i32, i32, i32), i32>,
    get_multi: TypedFunc<(i32, i32, i32, i32, i32), i32>,
    get_raw_cell: TypedFunc<(i64, i32, i32), i32>,
    get_raw_row: TypedFunc<(i64, i32, i32), i32>,
}

pub struct Terminal {
    runtime: Runtime,
    terminal: i32,
    render: i32,
    row_slot: i32,
    cell_slot: i32,
    key_encoder: i32,
    key_event: i32,
    mouse_encoder: i32,
    mouse_event: i32,
    scratch: i32,
    style: i32,
    style_size: usize,
    style_offsets: [i32; 8],
    style_size_offset: i32,
    query: i32,
    graphemes: i32,
    grapheme_capacity: usize,
    input: i32,
    input_capacity: usize,
    rows: Vec<Arc<Row>>,
    hot: Hot,
}

impl Terminal {
    pub fn new(
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
        theme: &Theme,
    ) -> Result<Self> {
        let mut runtime = Runtime::new()?;
        let size = runtime.layout("GhosttyTerminalOptions")?.size;
        let terminal = runtime.temporary(size, |runtime, options| {
            for (field, value) in [
                ("cols", cols.max(1) as u64),
                ("rows", rows.max(1) as u64),
                ("max_scrollback", 10_000),
            ] {
                runtime.set_field(options, "GhosttyTerminalOptions", field, value)?;
            }
            let slot = runtime.call("ghostty_wasm_alloc_opaque", &[])?;
            runtime.write(slot, &[0; 4])?;
            runtime.success("ghostty_terminal_new", &[0, slot, options])?;
            let handle = runtime.u32(slot)? as i32;
            runtime.call("ghostty_wasm_free_opaque", &[slot])?;
            Ok(handle)
        })?;
        let (render, _) = Self::handle(&mut runtime, "ghostty_render_state_new", false)?;
        let (_, row_slot) =
            Self::handle(&mut runtime, "ghostty_render_state_row_iterator_new", true)?;
        let (_, cell_slot) =
            Self::handle(&mut runtime, "ghostty_render_state_row_cells_new", true)?;
        let (key_encoder, _) = Self::handle(&mut runtime, "ghostty_key_encoder_new", false)?;
        let (key_event, _) = Self::handle(&mut runtime, "ghostty_key_event_new", false)?;
        let (mouse_encoder, _) = Self::handle(&mut runtime, "ghostty_mouse_encoder_new", false)?;
        let (mouse_event, _) = Self::handle(&mut runtime, "ghostty_mouse_event_new", false)?;
        let scratch = runtime.alloc(16)?;
        let style_layout = runtime.layout("GhosttyStyle")?.clone();
        ensure!(
            style_layout.fields["size"].size == 4,
            "Ghostty style size ABI"
        );
        let style_size_offset = style_layout.fields["size"].offset as i32;
        let style_offsets = [
            "inverse",
            "faint",
            "bold",
            "italic",
            "invisible",
            "strikethrough",
            "overline",
            "underline",
        ]
        .map(|name| style_layout.fields[name].offset as i32);
        let style_size = style_layout.size;
        let style = runtime.alloc(style_size)?;
        let query = runtime.alloc(60)?;
        for (index, (key, pointer)) in [
            (2, style),
            (3, query + 40),
            (7, query + 44),
            (5, query + 48),
            (6, query + 52),
        ]
        .into_iter()
        .enumerate()
        {
            runtime.write(query + index as i32 * 4, &i32::to_le_bytes(key))?;
            runtime.write(query + 20 + index as i32 * 4, &pointer.to_le_bytes())?;
        }
        let hot = Hot {
            write: runtime.typed("ghostty_terminal_vt_write")?,
            next_row: runtime.typed("ghostty_render_state_row_iterator_next")?,
            next_cell: runtime.typed("ghostty_render_state_row_cells_next")?,
            get_cell: runtime.typed("ghostty_render_state_row_cells_get")?,
            get_multi: runtime.typed("ghostty_render_state_row_cells_get_multi")?,
            get_raw_cell: runtime.typed("ghostty_cell_get")?,
            get_raw_row: runtime.typed("ghostty_row_get")?,
        };
        let mut value = Self {
            runtime,
            terminal,
            render,
            row_slot,
            cell_slot,
            key_encoder,
            key_event,
            mouse_encoder,
            mouse_event,
            scratch,
            style,
            style_size,
            style_offsets,
            style_size_offset,
            query,
            graphemes: 0,
            grapheme_capacity: 0,
            input: 0,
            input_capacity: 0,
            rows: Vec::new(),
            hot,
        };
        let index = value.runtime.callback_index;
        value
            .runtime
            .call("ghostty_terminal_set", &[terminal, 0, 1])?;
        value
            .runtime
            .call("ghostty_terminal_set", &[terminal, 1, index])?;
        value.default_blink()?;
        value.set_theme(theme)?;
        value.resize(cols, rows, cell_width, cell_height)?;
        Ok(value)
    }

    fn handle(runtime: &mut Runtime, constructor: &str, keep_slot: bool) -> Result<(i32, i32)> {
        let slot = runtime.call("ghostty_wasm_alloc_opaque", &[])?;
        runtime.write(slot, &[0; 4])?;
        runtime.success(constructor, &[0, slot])?;
        let value = runtime.u32(slot)? as i32;
        if !keep_slot {
            runtime.call("ghostty_wasm_free_opaque", &[slot])?;
        }
        Ok((value, if keep_slot { slot } else { 0 }))
    }

    fn default_blink(&mut self) -> Result<()> {
        self.runtime.write(self.scratch, &[1])?;
        self.runtime
            .call("ghostty_terminal_set", &[self.terminal, 23, self.scratch])?;
        Ok(())
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        if bytes.len() > self.input_capacity {
            let capacity = bytes.len().max(self.input_capacity.saturating_mul(2));
            let pointer = self.runtime.alloc(capacity)?;
            self.runtime.free(self.input, self.input_capacity)?;
            self.input = pointer;
            self.input_capacity = capacity;
        }
        self.runtime.write(self.input, bytes)?;
        checked(self.hot.write.call(
            &mut self.runtime.store,
            (self.terminal, self.input, i32::try_from(bytes.len())?),
        ))?;
        Ok(())
    }

    pub fn reset_and_write(&mut self, bytes: &[u8]) -> Result<()> {
        self.runtime
            .call("ghostty_terminal_reset", &[self.terminal])?;
        self.default_blink()?;
        self.rows.clear();
        self.runtime.store.data_mut().suppress_replies = true;
        let result = self.write(bytes);
        self.runtime.store.data_mut().suppress_replies = false;
        result
    }

    pub fn write_with_sync_end(&mut self, bytes: &[u8]) -> Result<bool> {
        if bytes.is_empty() {
            return Ok(false);
        }
        let mut synchronized = self.mode(2026)?;
        if !synchronized {
            self.write(bytes)?;
            return Ok(false);
        }
        let mut start = 0;
        let mut ended = false;
        while start < bytes.len() {
            let end = if synchronized {
                bytes[start + 1..]
                    .iter()
                    .position(|byte| *byte == 0x1b)
                    .map_or(bytes.len(), |offset| start + 1 + offset)
            } else {
                bytes.len()
            };
            self.write(&bytes[start..end])?;
            start = end;
            if synchronized && !self.mode(2026)? {
                synchronized = false;
                ended = true;
            }
        }
        Ok(ended)
    }

    pub fn take_replies(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.runtime.store.data_mut().replies)
    }

    pub fn resize(
        &mut self,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    ) -> Result<()> {
        self.runtime.success(
            "ghostty_terminal_resize",
            &[
                self.terminal,
                cols.max(1) as i32,
                rows.max(1) as i32,
                cell_width.max(1) as i32,
                cell_height.max(1) as i32,
            ],
        )
    }

    pub fn set_theme(&mut self, theme: &Theme) -> Result<()> {
        for (option, color) in [
            (11, theme.foreground),
            (12, theme.background),
            (13, theme.cursor),
        ] {
            self.runtime
                .write(self.scratch, &[color.r, color.g, color.b])?;
            self.runtime.call(
                "ghostty_terminal_set",
                &[self.terminal, option, self.scratch],
            )?;
        }
        if let Some(colors) = &theme.palette {
            let terminal = self.terminal;
            self.runtime.temporary(256 * 3, |runtime, pointer| {
                runtime.success("ghostty_terminal_get", &[terminal, 25, pointer])?;
                for (index, color) in colors.iter().take(16).enumerate() {
                    runtime.write(pointer + index as i32 * 3, &[color.r, color.g, color.b])?;
                }
                runtime.success("ghostty_terminal_set", &[terminal, 14, pointer])
            })?;
        }
        Ok(())
    }

    pub fn mode(&mut self, mode: i32) -> Result<bool> {
        self.runtime.write(self.scratch, &[0])?;
        let result = self.runtime.call(
            "ghostty_terminal_mode_get",
            &[self.terminal, mode, self.scratch],
        )?;
        Ok(result == 0 && self.runtime.u8(self.scratch)? != 0)
    }

    pub fn viewport_active(&mut self) -> Result<bool> {
        self.terminal_bool(32)
    }
    pub fn mouse_tracking(&mut self) -> Result<bool> {
        self.terminal_bool(11)
    }

    fn terminal_bool(&mut self, key: i32) -> Result<bool> {
        self.runtime.write(self.scratch, &[0])?;
        let result = self
            .runtime
            .call("ghostty_terminal_get", &[self.terminal, key, self.scratch])?;
        Ok(result == 0 && self.runtime.u8(self.scratch)? != 0)
    }

    pub fn end_synchronized_output(&mut self) -> Result<()> {
        self.runtime
            .success("ghostty_terminal_mode_set", &[self.terminal, 2026, 0])
    }

    pub fn is_alternate_screen(&mut self) -> Result<bool> {
        self.runtime.write(self.scratch, &[0; 4])?;
        let result = self
            .runtime
            .call("ghostty_terminal_get", &[self.terminal, 6, self.scratch])?;
        Ok(result == 0 && self.runtime.u32(self.scratch)? == 1)
    }

    pub fn scroll(&mut self, rows: i32) -> Result<()> {
        self.scroll_viewport(2, rows)
    }
    pub fn scroll_to_bottom(&mut self) -> Result<()> {
        self.scroll_viewport(1, 0)
    }

    fn scroll_viewport(&mut self, tag: u64, rows: i32) -> Result<()> {
        let layout = self
            .runtime
            .layout("GhosttyTerminalScrollViewport")?
            .clone();
        let terminal = self.terminal;
        self.runtime.temporary(layout.size, |runtime, pointer| {
            runtime.set_field(pointer, "GhosttyTerminalScrollViewport", "tag", tag)?;
            if tag == 2 {
                runtime.write(
                    pointer + layout.fields["value"].offset as i32,
                    &rows.to_le_bytes(),
                )?;
            }
            runtime.success("ghostty_terminal_scroll_viewport", &[terminal, pointer])
        })
    }

    pub fn scrollbar(&mut self) -> Result<Option<Scrollbar>> {
        let size = self.runtime.layout("GhosttyTerminalScrollbar")?.size;
        let terminal = self.terminal;
        self.runtime.temporary(size, |runtime, pointer| {
            if runtime.call("ghostty_terminal_get", &[terminal, 9, pointer])? != 0 {
                return Ok(None);
            }
            Ok(Some(Scrollbar {
                total: runtime.read_field(pointer, "GhosttyTerminalScrollbar", "total")?,
                offset: runtime.read_field(pointer, "GhosttyTerminalScrollbar", "offset")?,
                len: runtime.read_field(pointer, "GhosttyTerminalScrollbar", "len")?,
            }))
        })
    }

    fn render_u32(&mut self, key: i32) -> Result<u32> {
        self.runtime.write(self.scratch, &[0; 4])?;
        self.runtime.success(
            "ghostty_render_state_get",
            &[self.render, key, self.scratch],
        )?;
        self.runtime.u32(self.scratch)
    }

    fn render_u16(&mut self, key: i32) -> Result<u16> {
        self.runtime.write(self.scratch, &[0; 2])?;
        self.runtime.success(
            "ghostty_render_state_get",
            &[self.render, key, self.scratch],
        )?;
        self.runtime.u16(self.scratch)
    }

    fn render_bool(&mut self, key: i32) -> Result<bool> {
        self.runtime.write(self.scratch, &[0])?;
        self.runtime.success(
            "ghostty_render_state_get",
            &[self.render, key, self.scratch],
        )?;
        Ok(self.runtime.u8(self.scratch)? != 0)
    }

    fn color(&self, pointer: i32) -> Result<Color> {
        let bytes = self.runtime.bytes(pointer, 3)?;
        Ok(Color {
            r: bytes[0],
            g: bytes[1],
            b: bytes[2],
        })
    }

    fn render_color(&mut self, key: i32, fallback: Color) -> Result<Color> {
        if self.runtime.call(
            "ghostty_render_state_get",
            &[self.render, key, self.scratch],
        )? != 0
        {
            return Ok(fallback);
        }
        self.color(self.scratch)
    }

    pub fn snapshot(&mut self) -> Result<Snapshot> {
        self.runtime
            .success("ghostty_render_state_update", &[self.render, self.terminal])?;
        let cols = self.render_u16(1)?;
        let rows = self.render_u16(2)?;
        let dirty = self.render_u32(3)?;
        let foreground = self.render_color(
            6,
            Color {
                r: 229,
                g: 231,
                b: 235,
            },
        )?;
        let background = self.render_color(5, Color::default())?;
        let cursor = if self.render_bool(8)? {
            self.render_color(7, foreground)?
        } else {
            foreground
        };
        let cursor_in_viewport = self.render_bool(14)?;
        let cursor_visible = self.render_bool(11)? && cursor_in_viewport;
        let cursor_x = if cursor_in_viewport {
            self.render_u16(15)? as i32
        } else {
            -1
        };
        let cursor_y = if cursor_in_viewport {
            self.render_u16(16)? as i32
        } else {
            -1
        };
        if self.rows.len() != rows as usize
            || self.rows.iter().any(|row| row.cells.len() != cols as usize)
        {
            self.rows = (0..rows)
                .map(|_| {
                    Arc::new(Row {
                        cells: (0..cols)
                            .map(|_| Self::empty_cell(foreground, background))
                            .collect(),
                        text: String::new(),
                        is_wrap_continuation: false,
                        wraps_to_next: false,
                    })
                })
                .collect();
        }
        let mut dirty_rows = BTreeSet::new();
        if dirty != 0 {
            self.runtime
                .success("ghostty_render_state_get", &[self.render, 4, self.row_slot])?;
            let iterator = self.runtime.u32(self.row_slot)? as i32;
            for index in 0..rows as usize {
                if checked(self.hot.next_row.call(&mut self.runtime.store, iterator))? == 0 {
                    break;
                }
                self.runtime.write(self.scratch, &[0])?;
                self.runtime
                    .success("ghostty_render_state_row_get", &[iterator, 1, self.scratch])?;
                if dirty == 2 || self.runtime.u8(self.scratch)? != 0 {
                    self.rows[index] =
                        Arc::new(self.read_row(iterator, cols, foreground, background)?);
                    dirty_rows.insert(index);
                    self.runtime.write(self.scratch, &[0])?;
                    self.runtime
                        .success("ghostty_render_state_row_set", &[iterator, 0, self.scratch])?;
                }
            }
            self.runtime.write(self.scratch, &[0; 4])?;
            self.runtime
                .success("ghostty_render_state_set", &[self.render, 0, self.scratch])?;
        }
        Ok(Snapshot {
            cols,
            rows,
            foreground,
            background,
            cursor,
            cursor_x,
            cursor_y,
            cursor_visible,
            cursor_blinking: self.render_bool(12)?,
            cursor_style: self.render_u32(10)?,
            dirty_rows,
            row_data: self.rows.clone(),
        })
    }

    fn read_row(
        &mut self,
        iterator: i32,
        cols: u16,
        default_foreground: Color,
        default_background: Color,
    ) -> Result<Row> {
        self.runtime
            .success("ghostty_render_state_row_get", &[iterator, 2, self.scratch])?;
        let raw = self.runtime.u64(self.scratch)? as i64;
        self.runtime.write(self.scratch + 8, &[0])?;
        ensure!(
            checked(
                self.hot
                    .get_raw_row
                    .call(&mut self.runtime.store, (raw, 2, self.scratch + 8))
            )? == 0,
            "Ghostty row wrap continuation"
        );
        let is_wrap_continuation = self.runtime.u8(self.scratch + 8)? != 0;
        self.runtime.write(self.scratch + 8, &[0])?;
        ensure!(
            checked(
                self.hot
                    .get_raw_row
                    .call(&mut self.runtime.store, (raw, 1, self.scratch + 8))
            )? == 0,
            "Ghostty row wrap"
        );
        let wraps_to_next = self.runtime.u8(self.scratch + 8)? != 0;
        self.runtime.success(
            "ghostty_render_state_row_get",
            &[iterator, 3, self.cell_slot],
        )?;
        let iterator = self.runtime.u32(self.cell_slot)? as i32;
        let mut cells: Vec<Cell> = Vec::with_capacity(cols as usize);
        while cells.len() < cols as usize
            && checked(self.hot.next_cell.call(&mut self.runtime.store, iterator))? != 0
        {
            self.runtime.zero(self.style, self.style_size)?;
            self.runtime.write(
                self.style + self.style_size_offset,
                &(self.style_size as u32).to_le_bytes(),
            )?;
            let result = checked(self.hot.get_multi.call(
                &mut self.runtime.store,
                (iterator, 5, self.query, self.query + 20, self.query + 56),
            ))?;
            let written = self.runtime.u32(self.query + 56)?;
            ensure!(
                written >= 3 || result == 0,
                "Ghostty cell query returned {result}"
            );
            let length = self.runtime.u32(self.query + 40)? as usize;
            let selected = self.runtime.u8(self.query + 44)? != 0;
            let mut background = if written >= 4 {
                self.color(self.query + 48)?
            } else {
                default_background
            };
            let mut foreground = if written == 5 {
                self.color(self.query + 52)?
            } else if written == 3 {
                if checked(
                    self.hot
                        .get_cell
                        .call(&mut self.runtime.store, (iterator, 6, self.scratch)),
                )? == 0
                {
                    self.color(self.scratch)?
                } else {
                    default_foreground
                }
            } else {
                default_foreground
            };
            let mut text = String::new();
            if length > 0 {
                let size = length
                    .checked_mul(4)
                    .ok_or_else(|| anyhow::anyhow!("Ghostty grapheme size overflow"))?;
                if size > self.grapheme_capacity {
                    let capacity = size.max(self.grapheme_capacity.saturating_mul(2));
                    let pointer = self.runtime.alloc(capacity)?;
                    self.runtime.free(self.graphemes, self.grapheme_capacity)?;
                    self.graphemes = pointer;
                    self.grapheme_capacity = capacity;
                }
                if checked(
                    self.hot
                        .get_cell
                        .call(&mut self.runtime.store, (iterator, 4, self.graphemes)),
                )? == 0
                {
                    for code in self.runtime.bytes(self.graphemes, size)?.as_chunks::<4>().0 {
                        text.push(char::from_u32(u32::from_le_bytes(*code)).unwrap_or('\u{fffd}'));
                    }
                }
            }
            let mut wide = 0;
            if text.is_empty()
                && (cells.last().is_some_and(|cell| !cell.text.is_empty())
                    || cells.len() + 1 == cols as usize)
            {
                ensure!(
                    checked(
                        self.hot
                            .get_cell
                            .call(&mut self.runtime.store, (iterator, 1, self.scratch))
                    )? == 0,
                    "Ghostty raw cell"
                );
                let raw = self.runtime.u64(self.scratch)? as i64;
                self.runtime.write(self.scratch + 8, &[0; 4])?;
                ensure!(
                    checked(
                        self.hot
                            .get_raw_cell
                            .call(&mut self.runtime.store, (raw, 3, self.scratch + 8))
                    )? == 0,
                    "Ghostty cell width"
                );
                wide = self.runtime.u32(self.scratch + 8)?;
            }
            let flag = |index: usize| -> Result<bool> {
                Ok(self.runtime.u8(self.style + self.style_offsets[index])? != 0)
            };
            if flag(0)? {
                std::mem::swap(&mut foreground, &mut background);
            }
            if flag(1)? {
                let channel =
                    |front: u8, back: u8| ((front as u32 * 155 + back as u32 * 100) / 255) as u8;
                foreground = Color {
                    r: channel(foreground.r, background.r),
                    g: channel(foreground.g, background.g),
                    b: channel(foreground.b, background.b),
                };
            }
            cells.push(Cell {
                text,
                wide,
                foreground,
                background,
                bold: flag(2)?,
                italic: flag(3)?,
                invisible: flag(4)?,
                strikethrough: flag(5)?,
                overline: flag(6)?,
                underline: self.runtime.u32(self.style + self.style_offsets[7])? != 0,
                selected,
            });
        }
        while cells.len() < cols as usize {
            cells.push(Self::empty_cell(default_foreground, default_background));
        }
        let text = cells
            .iter()
            .map(|cell| {
                if cell.text.is_empty() {
                    " "
                } else {
                    &cell.text
                }
            })
            .collect::<String>()
            .trim_end()
            .to_string();
        Ok(Row {
            cells,
            text,
            is_wrap_continuation,
            wraps_to_next,
        })
    }

    fn empty_cell(foreground: Color, background: Color) -> Cell {
        Cell {
            text: String::new(),
            wide: 0,
            foreground,
            background,
            bold: false,
            italic: false,
            invisible: false,
            strikethrough: false,
            overline: false,
            underline: false,
            selected: false,
        }
    }

    fn encode(&mut self, name: &str, prefix: &[i32]) -> Result<Vec<u8>> {
        self.runtime.temporary(4, |runtime, written| {
            let mut args = prefix.to_vec();
            args.extend([0, 0, written]);
            let result = runtime.call(name, &args)?;
            let size = runtime.u32(written)? as usize;
            if result == 0 && size == 0 {
                return Ok(Vec::new());
            }
            if result != -3 || size == 0 {
                return Ok(Vec::new());
            }
            runtime.temporary(size, |runtime, pointer| {
                let mut args = prefix.to_vec();
                args.extend([pointer, i32::try_from(size)?, written]);
                if runtime.call(name, &args)? != 0 {
                    return Ok(Vec::new());
                }
                Ok(runtime
                    .bytes(pointer, runtime.u32(written)? as usize)?
                    .to_vec())
            })
        })
    }

    pub fn encode_key(&mut self, key: &Key) -> Result<Vec<u8>> {
        self.runtime.call(
            "ghostty_key_encoder_setopt_from_terminal",
            &[self.key_encoder, self.terminal],
        )?;
        for (name, value) in [
            ("action", key.action),
            ("key", key.key),
            ("mods", key.mods),
            ("consumed_mods", key.consumed_mods),
            ("composing", key.composing as i32),
            ("unshifted_codepoint", key.unshifted_codepoint as i32),
        ] {
            self.runtime.call(
                &format!("ghostty_key_event_set_{name}"),
                &[self.key_event, value],
            )?;
        }
        self.temporary(key.text.len(), |this, pointer| {
            if !key.text.is_empty() {
                this.runtime.write(pointer, key.text.as_bytes())?;
            }
            this.runtime.call(
                "ghostty_key_event_set_utf8",
                &[this.key_event, pointer, i32::try_from(key.text.len())?],
            )?;
            this.encode(
                "ghostty_key_encoder_encode",
                &[this.key_encoder, this.key_event],
            )
        })
    }

    pub fn encode_paste(&mut self, text: &str) -> Result<Vec<u8>> {
        if text.is_empty() {
            return Ok(Vec::new());
        }
        self.temporary(text.len(), |this, pointer| {
            this.runtime.write(pointer, text.as_bytes())?;
            let bracketed = this.mode(2004)? as i32;
            this.encode(
                "ghostty_paste_encode",
                &[pointer, i32::try_from(text.len())?, bracketed],
            )
        })
    }

    pub fn encode_mouse(&mut self, mouse: &Mouse) -> Result<Vec<u8>> {
        self.runtime.call(
            "ghostty_mouse_encoder_setopt_from_terminal",
            &[self.mouse_encoder, self.terminal],
        )?;
        let size = self.runtime.layout("GhosttyMouseEncoderSize")?.size;
        let encoder = self.mouse_encoder;
        self.runtime.temporary(size, |runtime, pointer| {
            for (field, value) in [
                ("size", size as f32),
                ("screen_width", mouse.screen_width),
                ("screen_height", mouse.screen_height),
                ("cell_width", mouse.cell_width),
                ("cell_height", mouse.cell_height),
                ("padding_top", mouse.padding_top),
                ("padding_bottom", mouse.padding_bottom),
                ("padding_left", mouse.padding_left),
                ("padding_right", mouse.padding_right),
            ] {
                runtime.set_field(
                    pointer,
                    "GhosttyMouseEncoderSize",
                    field,
                    value.round().max(0.) as u64,
                )?;
            }
            runtime.success("ghostty_mouse_encoder_setopt", &[encoder, 2, pointer])
        })?;
        self.runtime
            .write(self.scratch, &[mouse.any_button_pressed as u8])?;
        self.runtime
            .call("ghostty_mouse_encoder_setopt", &[encoder, 3, self.scratch])?;
        self.runtime.write(self.scratch, &[1])?;
        self.runtime
            .call("ghostty_mouse_encoder_setopt", &[encoder, 4, self.scratch])?;
        self.runtime.call(
            "ghostty_mouse_event_set_action",
            &[
                self.mouse_event,
                match mouse.action {
                    MouseAction::Press => 0,
                    MouseAction::Release => 1,
                    MouseAction::Motion => 2,
                },
            ],
        )?;
        match mouse.button {
            Some(button) => {
                self.runtime.call(
                    "ghostty_mouse_event_set_button",
                    &[self.mouse_event, button],
                )?;
            }
            None => {
                self.runtime
                    .call("ghostty_mouse_event_clear_button", &[self.mouse_event])?;
            }
        }
        self.runtime.call(
            "ghostty_mouse_event_set_mods",
            &[self.mouse_event, mouse.mods],
        )?;
        let layout = self.runtime.layout("GhosttyMousePosition")?.clone();
        let event = self.mouse_event;
        self.runtime.temporary(layout.size, |runtime, pointer| {
            runtime.write(
                pointer + layout.fields["x"].offset as i32,
                &mouse.x.to_le_bytes(),
            )?;
            runtime.write(
                pointer + layout.fields["y"].offset as i32,
                &mouse.y.to_le_bytes(),
            )?;
            runtime.success("ghostty_mouse_event_set_position", &[event, pointer])
        })?;
        self.encode(
            "ghostty_mouse_encoder_encode",
            &[self.mouse_encoder, self.mouse_event],
        )
    }

    fn grid_ref(&mut self, point: Point, tag: u32) -> Result<i32> {
        let layout = self.runtime.layout("GhosttyPoint")?.clone();
        let terminal = self.terminal;
        self.runtime.temporary(layout.size, |runtime, pointer| {
            runtime.set_field(pointer, "GhosttyPoint", "tag", tag as u64)?;
            let offset = pointer + layout.fields["value"].offset as i32;
            runtime.write(offset, &(point.x as u16).to_le_bytes())?;
            runtime.write(offset + 4, &point.y.to_le_bytes())?;
            let size = runtime.layout("GhosttyGridRef")?.size;
            let reference = runtime.alloc(size)?;
            runtime.set_field(reference, "GhosttyGridRef", "size", size as u64)?;
            if let Err(error) =
                runtime.success("ghostty_terminal_grid_ref", &[terminal, pointer, reference])
            {
                runtime.free(reference, size)?;
                return Err(error);
            }
            Ok(reference)
        })
    }

    fn point_from_ref(&mut self, reference: i32, tag: u32) -> Result<Option<Point>> {
        let size = self.runtime.layout("GhosttyPointCoordinate")?.size;
        let terminal = self.terminal;
        self.runtime.temporary(size, |runtime, pointer| {
            if runtime.call(
                "ghostty_terminal_point_from_grid_ref",
                &[terminal, reference, tag as i32, pointer],
            )? != 0
            {
                return Ok(None);
            }
            Ok(Some(Point {
                x: runtime.read_field(pointer, "GhosttyPointCoordinate", "x")? as u32,
                y: runtime.read_field(pointer, "GhosttyPointCoordinate", "y")? as u32,
            }))
        })
    }

    pub fn convert_point(&mut self, point: Point, from: u32, to: u32) -> Result<Option<Point>> {
        self.with_grid_ref(point, from, |this, reference| {
            this.point_from_ref(reference, to)
        })
    }

    fn range_from_selection(&mut self, selection: i32, tag: u32) -> Result<Option<Range>> {
        let layout = self.runtime.layout("GhosttySelection")?.clone();
        let start = self.point_from_ref(selection + layout.fields["start"].offset as i32, tag)?;
        let end = self.point_from_ref(selection + layout.fields["end"].offset as i32, tag)?;
        Ok(start.zip(end).map(|(start, end)| Range { start, end }))
    }

    pub fn set_selection(
        &mut self,
        start: Point,
        end: Point,
        start_tag: u32,
        end_tag: u32,
    ) -> Result<()> {
        let layout = self.runtime.layout("GhosttySelection")?.clone();
        self.temporary(layout.size, |this, selection| {
            this.runtime
                .set_field(selection, "GhosttySelection", "size", layout.size as u64)?;
            for (point, tag, field) in [(start, start_tag, "start"), (end, end_tag, "end")] {
                let field = &layout.fields[field];
                this.with_grid_ref(point, tag, |this, reference| {
                    let bytes = this.runtime.bytes(reference, field.size)?.to_vec();
                    this.runtime.write(selection + field.offset as i32, &bytes)
                })?;
            }
            this.runtime
                .success("ghostty_terminal_set", &[this.terminal, 21, selection])
        })
    }

    pub fn select_all(&mut self) -> Result<Option<Range>> {
        self.selection_query("ghostty_terminal_select_all", 0, true)
    }
    pub fn selection_range(&mut self) -> Result<Option<Range>> {
        self.selection_query("ghostty_terminal_get", 31, false)
    }

    fn selection_query(&mut self, operation: &str, key: i32, apply: bool) -> Result<Option<Range>> {
        let size = self.runtime.layout("GhosttySelection")?.size;
        self.temporary(size, |this, selection| {
            this.runtime
                .set_field(selection, "GhosttySelection", "size", size as u64)?;
            let args = if apply {
                vec![this.terminal, selection]
            } else {
                vec![this.terminal, key, selection]
            };
            if this.runtime.call(operation, &args)? != 0 {
                return Ok(None);
            }
            let range = this.range_from_selection(selection, 2)?;
            if apply && range.is_some() {
                this.runtime
                    .success("ghostty_terminal_set", &[this.terminal, 21, selection])?;
            }
            Ok(range)
        })
    }

    pub fn select_word(&mut self, point: Point) -> Result<Option<Selection>> {
        self.select_at(
            "GhosttyTerminalSelectWordOptions",
            "ghostty_terminal_select_word",
            point,
        )
    }
    pub fn select_line(&mut self, point: Point) -> Result<Option<Selection>> {
        self.select_at(
            "GhosttyTerminalSelectLineOptions",
            "ghostty_terminal_select_line",
            point,
        )
    }

    fn select_at(
        &mut self,
        name: &str,
        operation: &str,
        point: Point,
    ) -> Result<Option<Selection>> {
        let layout = self.runtime.layout(name)?.clone();
        let selection_size = self.runtime.layout("GhosttySelection")?.size;
        self.temporary(layout.size, |this, options| {
            this.runtime
                .set_field(options, name, "size", layout.size as u64)?;
            this.with_grid_ref(point, 1, |this, reference| {
                let field = &layout.fields["ref"];
                let bytes = this.runtime.bytes(reference, field.size)?.to_vec();
                this.runtime.write(options + field.offset as i32, &bytes)?;
                this.temporary(selection_size, |this, selection| {
                    this.runtime.set_field(
                        selection,
                        "GhosttySelection",
                        "size",
                        selection_size as u64,
                    )?;
                    if this
                        .runtime
                        .call(operation, &[this.terminal, options, selection])?
                        != 0
                    {
                        return Ok(None);
                    }
                    let viewport = this.range_from_selection(selection, 1)?;
                    let screen = this.range_from_selection(selection, 2)?;
                    this.runtime
                        .success("ghostty_terminal_set", &[this.terminal, 21, selection])?;
                    Ok(viewport
                        .zip(screen)
                        .map(|(viewport, screen)| Selection { viewport, screen }))
                })
            })
        })
    }

    pub fn clear_selection(&mut self) -> Result<()> {
        self.runtime
            .success("ghostty_terminal_set", &[self.terminal, 21, 0])
    }

    pub fn selection_text(&mut self) -> Result<String> {
        self.temporary(16, |this, options| {
            this.runtime.write(options, &16u32.to_le_bytes())?;
            this.runtime.write(options + 8, &[1, 1])?;
            let bytes = this.encode(
                "ghostty_terminal_selection_format_buf",
                &[this.terminal, options],
            )?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        })
    }

    pub fn hyperlink_at(&mut self, point: Point) -> Result<Option<String>> {
        self.with_grid_ref(point, 1, |this, reference| {
            let bytes = this.encode("ghostty_grid_ref_hyperlink_uri", &[reference])?;
            Ok((!bytes.is_empty()).then(|| String::from_utf8_lossy(&bytes).into_owned()))
        })
    }

    fn temporary<T>(
        &mut self,
        size: usize,
        f: impl FnOnce(&mut Self, i32) -> Result<T>,
    ) -> Result<T> {
        if size == 0 {
            return f(self, 0);
        }
        let pointer = self.runtime.alloc(size)?;
        let result = f(self, pointer);
        let freed = self.runtime.free(pointer, size);
        match result {
            Ok(value) => {
                freed?;
                Ok(value)
            }
            Err(error) => Err(error),
        }
    }

    fn with_grid_ref<T>(
        &mut self,
        point: Point,
        tag: u32,
        f: impl FnOnce(&mut Self, i32) -> Result<T>,
    ) -> Result<T> {
        let pointer = self.grid_ref(point, tag)?;
        let result = f(self, pointer);
        let freed = self
            .runtime
            .free(pointer, self.runtime.layout("GhosttyGridRef")?.size);
        match result {
            Ok(value) => {
                freed?;
                Ok(value)
            }
            Err(error) => Err(error),
        }
    }

    pub fn memory_bytes(&self) -> usize {
        self.runtime.memory.data_size(&self.runtime.store)
    }
}
