//! One page-local native terminal scene.
//!
//! The parser, scrollback, retained cell batches and glyph atlas live here.
//! Android receives a composed RGBA buffer (no base64/PNG frame transport) and
//! draws it inside the figure rectangle exported by the retained Typst layout.
//! Input bytes are encoded with the parsed program's modes; the fixture echo
//! mode feeds them back so an APK lane can prove delivery without Core.

use emthin_core_client::Event;
use emthin_vt::{Color, DejaVuFont, Metrics, Pane, Renderer, Screen};
use serde_json::{json, Value};

const CHANNEL: u32 = 1;
const BACKGROUND: Color = Color([20, 24, 29, 255]);

pub struct Scene {
    pane: Pane,
    renderer: Renderer,
    font: DejaVuFont,
    echo: bool,
    frame: u64,
    generation: u64,
    input_log: String,
    closed: bool,
    /// Last damage reported by the parser, kept for diagnostics.
    damage: Vec<emthin_vt::Damage>,
}

impl Scene {
    pub fn new(
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
        baseline: u16,
        echo: bool,
    ) -> Self {
        let cols = cols.clamp(1, 300);
        let rows = rows.clamp(1, 120);
        let metrics = Metrics {
            cell_width: cell_width.clamp(1, 64),
            cell_height: cell_height.clamp(1, 96),
            baseline: baseline.min(cell_height.max(1)),
        };
        let pane = Pane::new(CHANNEL, cols, rows);
        let mut renderer = Renderer::new(metrics, 8 * 1024 * 1024, 1024);
        let mut font = DejaVuFont::new();
        let screen = pane.screen().clone();
        renderer.update(&screen, &[], &mut font);
        Self {
            pane,
            renderer,
            font,
            echo,
            frame: 0,
            generation: 0,
            input_log: String::new(),
            closed: false,
            damage: Vec::new(),
        }
    }

    fn apply(&mut self, event: Event) {
        if self.closed {
            return;
        }
        if let Some(paint) = self.pane.handle(&event) {
            self.renderer
                .update(&paint.screen, &paint.damage, &mut self.font);
            self.damage = paint.damage;
            self.frame = self.frame.saturating_add(1);
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.apply(Event::Data {
            channel: CHANNEL,
            bytes: bytes.to_vec(),
        });
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let cols = cols.clamp(1, 300);
        let rows = rows.clamp(1, 120);
        self.generation = self.generation.saturating_add(1);
        self.apply(Event::Resize {
            channel: CHANNEL,
            cols,
            rows,
        });
    }

    pub fn key(&mut self, key: i32, mods: u16, text: &str, unshifted: u32) -> Vec<u8> {
        if self.closed {
            return Vec::new();
        }
        let bytes = self.pane.encode_key(key, mods, text, unshifted);
        self.log_input("key", &bytes);
        if self.echo && !bytes.is_empty() {
            self.feed(&bytes);
        }
        bytes
    }

    pub fn text(&mut self, text: &str) -> usize {
        if self.closed {
            return 0;
        }
        let bytes = text.as_bytes();
        self.log_input("text", bytes);
        if self.echo && !bytes.is_empty() {
            self.feed(bytes);
        }
        bytes.len()
    }

    pub fn mouse(&mut self, x: u16, y: u16, button: u8, press: bool, mods: u8) -> Vec<u8> {
        if self.closed {
            return Vec::new();
        }
        let bytes = self.pane.mouse_button(x, y, button, press, mods);
        self.log_input("mouse", &bytes);
        // Reports are input, not output. Feeding them back in the fixture echo
        // mode would print the escape sequence over the terminal screen.
        bytes
    }

    pub fn mouse_motion(&mut self, x: u16, y: u16, button: Option<u8>, mods: u8) -> Vec<u8> {
        if self.closed {
            return Vec::new();
        }
        let bytes = self.pane.mouse_motion(x, y, button, mods);
        self.log_input("motion", &bytes);
        bytes
    }

    /// Text in a cell rectangle, inclusive at both corners. The rectangle is
    /// normalized so a drag may start at either corner.
    pub fn selection(&self, x0: u16, y0: u16, x1: u16, y1: u16) -> String {
        let screen = self.pane.screen();
        if screen.cols == 0 || screen.rows == 0 {
            return String::new();
        }
        let (mut x0, mut y0, mut x1, mut y1) = (x0, y0, x1, y1);
        if y0 > y1 || (y0 == y1 && x0 > x1) {
            std::mem::swap(&mut x0, &mut x1);
            std::mem::swap(&mut y0, &mut y1);
        }
        y0 = y0.min(screen.rows - 1);
        y1 = y1.min(screen.rows - 1);
        let mut out = String::new();
        for y in y0..=y1 {
            let start = if y == y0 { x0.min(screen.cols - 1) } else { 0 };
            let end = if y == y1 {
                x1.min(screen.cols - 1)
            } else {
                screen.cols - 1
            };
            let mut line = String::new();
            for x in start..=end {
                let Some(cell) = screen.cell(x, y) else {
                    continue;
                };
                // A wide glyph's spacer tail has no text of its own.
                if cell.wide == 2 {
                    continue;
                }
                if cell.text.is_empty() {
                    line.push(' ');
                } else {
                    line.push_str(&cell.text);
                }
            }
            out.push_str(line.trim_end_matches(' '));
            if y != y1 {
                out.push('\n');
            }
        }
        out
    }

    /// Cell bounds for the word under `(x, y)`, or the single cell when the
    /// pointer is not over a word. Returned as `(x0, y0, x1, y1)`.
    pub fn word_at(&self, x: u16, y: u16) -> Option<(u16, u16, u16, u16)> {
        let screen = self.pane.screen();
        if screen.cols == 0 || screen.rows == 0 {
            return None;
        }
        let y = y.min(screen.rows - 1);
        let mut x = x.min(screen.cols - 1);
        // Tapping the spacer of a wide glyph selects the glyph itself.
        if screen.cell(x, y).is_some_and(|cell| cell.wide == 2) && x > 0 {
            x -= 1;
        }
        let wordish = |x: u16| {
            screen.cell(x, y).is_some_and(|cell| {
                cell.wide != 2
                    && !cell.text.is_empty()
                    && !cell.text.chars().all(char::is_whitespace)
            })
        };
        if !wordish(x) {
            // A long press on blank space still selects nearby text instead of
            // producing an empty clipboard entry.
            let nearest = (0..x)
                .rev()
                .find(|candidate| wordish(*candidate))
                .or_else(|| ((x + 1)..screen.cols).find(|candidate| wordish(*candidate)));
            match nearest {
                Some(candidate) => x = candidate,
                None => {
                    // A blank line has no word: select the visible screen so
                    // the long press always yields copyable text.
                    return Some((0, 0, screen.cols - 1, screen.rows - 1));
                }
            }
        }
        let mut x0 = x;
        while x0 > 0 && wordish(x0 - 1) {
            x0 -= 1;
        }
        let mut x1 = x;
        while x1 + 1 < screen.cols && wordish(x1 + 1) {
            x1 += 1;
        }
        if x1 + 1 < screen.cols && screen.cell(x1 + 1, y).is_some_and(|cell| cell.wide == 2) {
            x1 += 1;
        }
        Some((x0, y, x1, y))
    }

    pub fn recreate(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.renderer.recreate_gl();
        self.frame = self.frame.saturating_add(1);
    }

    pub fn close(&mut self) {
        self.closed = true;
    }

    fn log_input(&mut self, kind: &str, bytes: &[u8]) {
        if !self.input_log.is_empty() {
            self.input_log.push(' ');
        }
        self.input_log.push_str(kind);
        self.input_log.push(':');
        for byte in bytes {
            use std::fmt::Write as _;
            let _ = write!(self.input_log, "{byte:02x}");
        }
    }

    fn screen_text(screen: &Screen) -> String {
        let mut out = String::new();
        for y in 0..screen.rows {
            if y > 0 {
                out.push('\n');
            }
            for x in 0..screen.cols {
                if let Some(cell) = screen.cell(x, y) {
                    if cell.text.is_empty() {
                        out.push(' ');
                    } else {
                        out.push_str(&cell.text);
                    }
                }
            }
        }
        out
    }

    pub fn state(&self) -> String {
        let screen = self.pane.screen();
        json!({
            "cols": screen.cols,
            "rows": screen.rows,
            "width": screen.cols as u32 * self.renderer.metrics().cell_width as u32,
            "height": screen.rows as u32 * self.renderer.metrics().cell_height as u32,
            "frame": self.frame,
            "generation": self.generation,
            "closed": self.closed,
            "cursor": {
                "x": screen.cursor.x,
                "y": screen.cursor.y,
                "visible": screen.cursor.visible,
            },
            "damage": self.damage.iter().map(|damage| json!({
                "x": damage.x,
                "y": damage.y,
                "cols": damage.cols,
                "rows": damage.rows,
            })).collect::<Vec<Value>>(),
            "input": self.input_log,
            "mouse": {
                "button": self.pane.screen().rows > 0 && (self.pane.mode(1000) || self.pane.mode(1002) || self.pane.mode(1003)),
                "motion": self.pane.mode(1002) || self.pane.mode(1003),
                "sgr": self.pane.mode(1006),
            },
            "text": Self::screen_text(screen),
        })
        .to_string()
    }

    pub fn rgba(&self) -> Vec<u8> {
        self.renderer.compose(BACKGROUND).pixels
    }
}
