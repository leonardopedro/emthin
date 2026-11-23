//! A small read-only view over the pinned libghostty-vt parser.
//!
//! The parser is Ghostty's; this crate copies the active screen into plain Rust
//! values so a renderer can diff frames without holding a short-lived grid
//! reference. It is independent of Core transport and figure ownership.

pub mod font;
#[cfg(not(target_os = "android"))]
pub mod gpu;
pub mod pane;
pub mod render;

pub use font::DejaVuFont;
use ghostty_vt::Terminal;
pub use ghostty_vt::{Cell, CellStyle, Cursor, StyleColor};
pub use pane::{Paint, Pane};
pub use render::{Atlas, Color, Glyph, GlyphDraw, GlyphSource, Image, Metrics, Renderer, Stats};

/// The glyph identity a renderer caches.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GlyphKey {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
}

/// A rectangular run of cells that differ from the previous snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Damage {
    pub x: u16,
    pub y: u16,
    pub cols: u16,
    pub rows: u16,
}

/// One complete active screen at a point in time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<Cell>,
    pub cursor: Cursor,
    /// How many rows above the active screen this view is scrolled. Zero is
    /// the live screen; the cursor is only shown there.
    pub history_offset: u16,
}

impl Screen {
    pub fn cell(&self, x: u16, y: u16) -> Option<&Cell> {
        if x >= self.cols || y >= self.rows {
            return None;
        }
        self.cells.get(y as usize * self.cols as usize + x as usize)
    }

    /// Row runs that changed since `old`. Adjacent changed cells are merged;
    /// separate rows stay separate so a renderer never repaints between them.
    pub fn damage_from(&self, old: &Screen) -> Vec<Damage> {
        if self.cols != old.cols
            || self.rows != old.rows
            || self.history_offset != old.history_offset
        {
            return vec![Damage {
                x: 0,
                y: 0,
                cols: self.cols,
                rows: self.rows,
            }];
        }
        let mut damage = Vec::new();
        for y in 0..self.rows {
            let mut start = None;
            for x in 0..=self.cols {
                let changed = x < self.cols && self.cell(x, y) != old.cell(x, y);
                match (start, changed) {
                    (None, true) => start = Some(x),
                    (Some(first), false) => {
                        damage.push(Damage {
                            x: first,
                            y,
                            cols: x - first,
                            rows: 1,
                        });
                        start = None;
                    }
                    _ => {}
                }
            }
        }
        damage
    }
}

/// A parser worker with a copyable screen snapshot.
pub struct Vt {
    terminal: Terminal,
    cols: u16,
    rows: u16,
}

impl Vt {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            terminal: Terminal::new(cols, rows, 1024 * 1024),
            cols: cols.max(1),
            rows: rows.max(1),
        }
    }

    /// Feed bytes from Core's attach stream.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.terminal.write(bytes);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols.max(1);
        self.rows = rows.max(1);
        self.terminal.resize(self.cols, self.rows);
    }

    pub fn screen(&self) -> Screen {
        self.screen_at(0)
    }

    /// Rows retained above the active screen. A scroll offset is clamped to
    /// this, so callers may keep an offset across output and resizes.
    pub fn scrollback_rows(&self) -> u32 {
        self.terminal.history_rows()
    }

    /// The active screen, or a viewport scrolled `offset` rows into bounded
    /// scrollback. Offsets past the oldest retained row clamp to the oldest
    /// row. The cursor is hidden in history.
    pub fn screen_at(&self, offset: u16) -> Screen {
        let history = self.terminal.history_rows();
        let offset = u32::from(offset).min(history);
        let mut cells = Vec::with_capacity(self.cols as usize * self.rows as usize);
        let blank = Cell {
            text: String::new(),
            wide: 0,
            style: CellStyle::default(),
        };
        for y in 0..self.rows {
            let history_y = history + u32::from(y) - offset;
            for x in 0..self.cols {
                cells.push(if history_y < history {
                    self.terminal
                        .history_cell(x, history_y)
                        .unwrap_or_else(|| blank.clone())
                } else {
                    self.terminal
                        .cell(x, (history_y - history) as u16)
                        .unwrap_or_else(|| blank.clone())
                });
            }
        }
        let mut cursor = self.terminal.cursor();
        if offset > 0 {
            cursor.visible = false;
        }
        Screen {
            cols: self.cols,
            rows: self.rows,
            cells,
            cursor,
            history_offset: offset as u16,
        }
    }

    /// Rows occupied by the active screen at this offset.
    pub fn viewport_rows(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }

    pub fn mode(&self, mode: u16) -> bool {
        self.terminal.dec_mode(mode)
    }

    /// Encode a key using the modes this screen's program set.
    pub fn encode_key(&self, key: i32, mods: u16, text: &str, unshifted: u32) -> Vec<u8> {
        self.terminal.encode_key(key, mods, text, unshifted)
    }

    /// A wheel notch, empty when the program did not request mouse reports.
    pub fn wheel(&self, up: bool) -> Vec<u8> {
        self.terminal.wheel(up)
    }

    /// A button event at cell `(x, y)`, using SGR or X10 encoding according to
    /// the program's modes. Empty when it did not request button reports.
    /// `mods` uses shift=1, alt=2, ctrl=4.
    pub fn mouse_button(&self, x: u16, y: u16, button: u8, press: bool, mods: u8) -> Vec<u8> {
        if !(self.mode(1000) || self.mode(1002) || self.mode(1003)) {
            return Vec::new();
        }
        let code = mouse_code(button & 3, mods);
        if self.mode(1006) {
            let final_byte = if press { 'M' } else { 'm' };
            return mouse_sgr(code, x, y, final_byte);
        }
        // X10 cannot report releases distinctly; button 3 is the release code.
        let code = if press { code } else { 3 };
        mouse_x10(code, x, y)
    }

    /// A pointer-motion report at cell `(x, y)`. 1002 sends motion only while
    /// a button is held; 1003 sends it for hover too (`button` is then None).
    pub fn mouse_motion(&self, x: u16, y: u16, button: Option<u8>, mods: u8) -> Vec<u8> {
        if !(self.mode(1002) || self.mode(1003)) {
            return Vec::new();
        }
        let Some(button) = button.or_else(|| self.mode(1003).then_some(3)) else {
            return Vec::new();
        };
        let code = mouse_code(button & 3, mods) | 32;
        if self.mode(1006) {
            return mouse_sgr(code, x, y, 'M');
        }
        mouse_x10(code, x, y)
    }

    /// Generated replies (DA, color reports, …) owed back to Core.
    pub fn take_replies(&mut self) -> Vec<u8> {
        self.terminal.take_responses()
    }

    pub fn title(&self) -> String {
        self.terminal.title()
    }
}

fn mouse_code(button: u8, mods: u8) -> u8 {
    let mut code = button & 3;
    if mods & 1 != 0 {
        code |= 4;
    }
    if mods & 2 != 0 {
        code |= 8;
    }
    if mods & 4 != 0 {
        code |= 16;
    }
    code
}

fn mouse_sgr(code: u8, x: u16, y: u16, final_byte: char) -> Vec<u8> {
    format!(
        "\x1b[<{code};{};{}{final_byte}",
        u32::from(x) + 1,
        u32::from(y) + 1
    )
    .into_bytes()
}

fn mouse_x10(code: u8, x: u16, y: u16) -> Vec<u8> {
    let x = (u32::from(x) + 1).min(223) as u8 + 32;
    let y = (u32::from(y) + 1).min(223) as u8 + 32;
    vec![0x1b, b'[', b'M', 32 + code, x, y]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vt(bytes: &[u8]) -> Vt {
        let mut vt = Vt::new(20, 4);
        vt.feed(bytes);
        vt
    }

    #[test]
    fn reads_styles_combining_and_width() {
        let screen = vt(b"\x1b[1;31mA\x1b[0m e\xcc\x81 \xe7\x95\x8c").screen();
        let a = screen.cell(0, 0).unwrap();
        assert_eq!(a.text, "A");
        assert!(a.style.bold);
        assert_eq!(a.style.fg, Some(StyleColor::Palette(1)));
        let combining = screen.cell(2, 0).unwrap();
        assert_eq!(combining.text.chars().count(), 2);
        let wide = screen.cell(4, 0).unwrap();
        assert_eq!(wide.text, "界");
        assert_eq!(wide.wide, 1);
        assert_eq!(screen.cell(5, 0).unwrap().wide, 2, "spacer tail");
    }

    #[test]
    fn cursor_visibility_and_modes_come_from_the_parser() {
        let mut t = vt(b"\x1b[2;3H\x1b[?25l\x1b[?2004h");
        let screen = t.screen();
        assert_eq!((screen.cursor.x, screen.cursor.y), (2, 1));
        assert!(!screen.cursor.visible);
        assert!(t.mode(2004));
        t.feed(b"\x1b[?25h");
        assert!(t.screen().cursor.visible);
    }

    #[test]
    fn damage_is_row_runs_and_a_resize_is_the_whole_screen() {
        let mut t = vt(b"abcdef");
        let before = t.screen();
        t.feed(b"\x1b[2;3HXY");
        let damage = t.screen().damage_from(&before);
        assert_eq!(
            damage,
            vec![Damage {
                x: 2,
                y: 1,
                cols: 2,
                rows: 1
            }]
        );
        t.resize(10, 2);
        assert_eq!(
            t.screen().damage_from(&before),
            vec![Damage {
                x: 0,
                y: 0,
                cols: 10,
                rows: 2
            }]
        );
    }

    #[test]
    fn scrollback_views_are_clamped_to_retained_history() {
        let mut t = Vt::new(5, 2);
        t.feed(b"a\r\nb\r\nc\r\nd\r\ne");
        let history = t.scrollback_rows();
        assert!(history >= 2, "older lines are retained");
        let live = t.screen();
        assert_eq!(live.history_offset, 0);
        assert_eq!(live.cell(0, 0).unwrap().text, "d");

        let one = t.screen_at(1);
        assert_eq!(one.history_offset, 1);
        assert_eq!(one.cell(0, 1).unwrap().text, "d");
        assert!(!one.cursor.visible, "history has no live cursor");

        let oldest = t.screen_at(u16::MAX);
        assert_eq!(u32::from(oldest.history_offset), history);
        assert_eq!(oldest.cell(0, 0).unwrap().text, "a");
        assert!(!oldest.cursor.visible);

        let back = t.screen_at(0);
        assert_eq!(back.history_offset, 0);
        assert_eq!(back.cursor.visible, live.cursor.visible);
    }

    #[test]
    fn mouse_encoding_follows_the_programs_modes() {
        let mut t = Vt::new(20, 4);
        assert!(t.wheel(true).is_empty(), "no report requested");
        t.feed(b"\x1b[?1000h\x1b[?1006h");
        assert!(!t.wheel(true).is_empty(), "tracking mode asks for a report");
    }

    #[test]
    fn mouse_buttons_use_the_programs_encoding() {
        let mut t = Vt::new(20, 4);
        assert!(t.mouse_button(1, 2, 0, true, 0).is_empty());
        t.feed(b"\x1b[?1000h\x1b[?1006h");
        assert_eq!(t.mouse_button(1, 2, 0, true, 0), b"\x1b[<0;2;3M");
        assert_eq!(t.mouse_button(1, 2, 0, false, 4), b"\x1b[<16;2;3m");
        t.feed(b"\x1b[?1006l");
        assert_eq!(
            t.mouse_button(0, 0, 0, true, 0),
            vec![0x1b, b'[', b'M', 32, 33, 33]
        );
    }

    #[test]
    fn drag_motion_follows_button_event_and_any_event_modes() {
        let mut t = Vt::new(20, 4);
        t.feed(b"\x1b[?1002h\x1b[?1006h");
        assert!(t.mouse_motion(1, 2, None, 0).is_empty());
        assert_eq!(t.mouse_motion(1, 2, Some(0), 0), b"\x1b[<32;2;3M");
        assert_eq!(t.mouse_motion(1, 2, Some(0), 4), b"\x1b[<48;2;3M");

        t.feed(b"\x1b[?1003h");
        assert_eq!(t.mouse_motion(1, 2, None, 0), b"\x1b[<35;2;3M");

        t.feed(b"\x1b[?1006l");
        assert_eq!(
            t.mouse_motion(0, 0, Some(2), 0),
            vec![0x1b, b'[', b'M', 32 + 34, 33, 33]
        );
    }

    #[test]
    fn replies_are_owed_to_core() {
        let mut t = Vt::new(20, 4);
        t.feed(b"\x1b[c");
        assert!(!t.take_replies().is_empty());
        assert!(t.take_replies().is_empty(), "replies are taken once");
    }
}
