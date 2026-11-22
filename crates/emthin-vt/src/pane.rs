//! The bridge from Core attach events to one VT snapshot.
//!
//! Core owns the stream and credit; this pane owns only parser state, damage,
//! and replies. Events for another channel are ignored, and a closed stream
//! cannot be revived by late data.

use emthin_core_client::Event;

use crate::{Damage, Screen, Vt};

/// What a renderer should do after one attach event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paint {
    pub screen: Screen,
    pub damage: Vec<Damage>,
    /// Bytes the terminal generated for the program (DA, color reports, …).
    pub replies: Vec<u8>,
}

pub struct Pane {
    channel: u32,
    vt: Vt,
    screen: Screen,
    closed: Option<String>,
    history_offset: u16,
}

impl Pane {
    pub fn new(channel: u32, cols: u16, rows: u16) -> Self {
        let vt = Vt::new(cols, rows);
        let screen = vt.screen();
        Self {
            channel,
            vt,
            screen,
            closed: None,
            history_offset: 0,
        }
    }

    pub fn channel(&self) -> u32 {
        self.channel
    }

    pub fn viewport(&self) -> (u16, u16) {
        self.vt.viewport_rows()
    }

    pub fn closed(&self) -> Option<&str> {
        self.closed.as_deref()
    }

    /// A DEC private mode from the parsed program state.
    pub fn mode(&self, mode: u16) -> bool {
        self.vt.mode(mode)
    }

    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    pub fn encode_key(&self, key: i32, mods: u16, text: &str, unshifted: u32) -> Vec<u8> {
        self.vt.encode_key(key, mods, text, unshifted)
    }

    pub fn wheel(&self, up: bool) -> Vec<u8> {
        self.vt.wheel(up)
    }

    pub fn mouse_button(
        &self,
        x: u16,
        y: u16,
        button: u8,
        press: bool,
        mods: u8,
    ) -> Vec<u8> {
        self.vt.mouse_button(x, y, button, press, mods)
    }

    pub fn mouse_motion(
        &self,
        x: u16,
        y: u16,
        button: Option<u8>,
        mods: u8,
    ) -> Vec<u8> {
        self.vt.mouse_motion(x, y, button, mods)
    }

    pub fn scrollback_offset(&self) -> u16 {
        self.history_offset
    }

    pub fn scrollback_rows(&self) -> u32 {
        self.vt.scrollback_rows()
    }

    /// Scroll the viewport by `rows` (negative scrolls toward live output).
    /// The result clamps to the retained history and to the live screen.
    pub fn scroll(&mut self, rows: i32) -> Option<Paint> {
        if self.closed.is_some() {
            return None;
        }
        let limit = self.scrollback_rows().min(u32::from(u16::MAX)) as u16;
        let target = i32::from(self.history_offset) + rows;
        self.history_offset = target.clamp(0, i32::from(limit)) as u16;
        Some(self.snapshot())
    }

    /// Scroll directly, clamped to the same bounds as [`Pane::scroll`].
    pub fn scroll_to(&mut self, offset: u16) -> Option<Paint> {
        if self.closed.is_some() {
            return None;
        }
        let limit = self.scrollback_rows().min(u32::from(u16::MAX)) as u16;
        self.history_offset = offset.min(limit);
        Some(self.snapshot())
    }

    /// Apply one attach event. Returns `None` for unrelated or stale events.
    pub fn handle(&mut self, event: &Event) -> Option<Paint> {
        if self.closed.is_some() {
            return None;
        }
        match event {
            Event::Data { channel, bytes } if *channel == self.channel => {
                self.vt.feed(bytes);
                Some(self.snapshot())
            }
            Event::Resize {
                channel,
                cols,
                rows,
            } if *channel == self.channel => {
                self.vt.resize(*cols, *rows);
                Some(self.snapshot())
            }
            Event::Closed { channel, reason } if *channel == self.channel => {
                self.closed = Some(reason.clone());
                Some(Paint {
                    screen: self.screen.clone(),
                    damage: Vec::new(),
                    replies: Vec::new(),
                })
            }
            _ => None,
        }
    }

    fn snapshot(&mut self) -> Paint {
        // New output or a resize can shrink history; never leave the view past
        // the oldest retained row.
        let limit = self.scrollback_rows().min(u32::from(u16::MAX)) as u16;
        self.history_offset = self.history_offset.min(limit);
        let screen = self.vt.screen_at(self.history_offset);
        let damage = screen.damage_from(&self.screen);
        self.screen = screen.clone();
        Paint {
            screen,
            damage,
            replies: self.vt.take_replies(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_becomes_a_damage_frame_and_replies_are_owed() {
        let mut pane = Pane::new(4, 10, 2);
        let paint = pane
            .handle(&Event::Data {
                channel: 4,
                bytes: b"\x1b[31mred\x1b[0m".to_vec(),
            })
            .expect("data paints");
        assert_eq!(paint.screen.cell(0, 0).unwrap().text, "r");
        assert_eq!(paint.damage[0].cols, 3);
        assert!(pane
            .handle(&Event::Data {
                channel: 9,
                bytes: b"wrong".to_vec()
            })
            .is_none());
    }

    #[test]
    fn one_changed_cell_damages_only_that_row_run() {
        let mut pane = Pane::new(1, 10, 2);
        pane.handle(&Event::Data {
            channel: 1,
            bytes: b"abcdef".to_vec(),
        });
        let paint = pane
            .handle(&Event::Data {
                channel: 1,
                bytes: b"\x1b[2;3HXY".to_vec(),
            })
            .expect("second data");
        assert_eq!(
            paint.damage,
            vec![Damage {
                x: 2,
                y: 1,
                cols: 2,
                rows: 1
            }]
        );
    }

    #[test]
    fn resize_repaints_the_whole_pane() {
        let mut pane = Pane::new(1, 10, 2);
        let paint = pane
            .handle(&Event::Resize {
                channel: 1,
                cols: 3,
                rows: 1,
            })
            .expect("resize");
        assert_eq!(
            paint.damage,
            vec![Damage {
                x: 0,
                y: 0,
                cols: 3,
                rows: 1
            }]
        );
    }

    #[test]
    fn scrolling_is_bounded_across_output_and_close() {
        let mut pane = Pane::new(3, 4, 2);
        pane.handle(&Event::Data {
            channel: 3,
            bytes: b"a\r\nb\r\nc\r\nd".to_vec(),
        });
        let live = pane.screen().clone();
        let paint = pane.scroll(-50).expect("back to live");
        assert_eq!(pane.scrollback_offset(), 0);
        assert_eq!(paint.screen, live);

        let rows = pane.scrollback_rows();
        let paint = pane.scroll(50).expect("oldest view");
        assert_eq!(u32::from(pane.scrollback_offset()), rows);
        assert_eq!(paint.screen.history_offset, pane.scrollback_offset());
        assert!(!paint.screen.cursor.visible);

        pane.scroll_to(u16::MAX);
        assert_eq!(u32::from(pane.scrollback_offset()), rows);

        pane.handle(&Event::Closed {
            channel: 3,
            reason: "exited".into(),
        });
        assert!(pane.scroll(1).is_none(), "a closed pane cannot scroll");
    }

    #[test]
    fn close_is_final_for_late_data() {
        let mut pane = Pane::new(2, 10, 2);
        pane.handle(&Event::Closed {
            channel: 2,
            reason: "exited".into(),
        })
        .expect("close");
        assert_eq!(pane.closed(), Some("exited"));
        assert!(pane
            .handle(&Event::Data {
                channel: 2,
                bytes: b"late".to_vec()
            })
            .is_none());
    }
}
