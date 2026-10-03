use mathed_core::glyphs::V2;
use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, MouseButton, PointerAxisEvent, PointerButtonEvent,
    },
    input::{
        keyboard::{keysyms, Keysym},
        pointer::{AxisFrame, ButtonEvent, Focus, GrabStartData, MotionEvent, RelativeMotionEvent},
    },
    reexports::wayland_server::Resource,
    utils::{IsAlive, Logical, Point, SERIAL_COUNTER},
    wayland::seat::WaylandFocus,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::docui::keymap;
use crate::docui::keymap::Action;
use crate::state::EmthinState;

impl EmthinState {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event, .. } => {
                let Some(keyboard) = self.seat.get_keyboard() else {
                    return;
                };
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);

                // One intercept pass classifies the key *and* records
                // the modifiers. `input_intercept` is the only public
                // handle that sees the keysym with the layout's shift
                // level applied, so the global keymap has to be
                // consulted here rather than after forwarding.
                let mut pending: Option<crate::docui::keymap::Action> = None;
                let mut sym = keysyms::KEY_NoSymbol;
                let (is_wakeup, mods_changed) = keyboard.input_intercept(
                    self,
                    event.key_code(),
                    event.state(),
                    |_state, mods, keysym_handle| {
                        if keysym_handle
                            .raw_latin_sym_or_raw_current_sym()
                            .is_some_and(|sym| Keysym::from(keysyms::KEY_XF86WakeUp) == sym)
                        {
                            return true;
                        }
                        sym = u32::from(keysym_handle.modified_sym());
                        pending = crate::docui::keymap::classify(
                            sym,
                            event.state(),
                            mods.ctrl,
                            mods.shift,
                            mods.alt,
                        );
                        false
                    },
                );

                if is_wakeup && event.state() == KeyState::Pressed {
                    self.handle_wakeup();
                    return;
                }
                // Global document bindings (page nav, launcher, …).
                // Checked *before* forwarding so they work whether a
                // figure has keyboard focus or not.
                if let Some(action) = pending {
                    self.run_doc_action(action);
                    return;
                }

                if keyboard.current_focus().is_some() {
                    // A figure (or dialog) owns the keyboard: forward
                    // verbatim. Every non-global key here belongs to the
                    // client, including the ones that look like editing
                    // — inside a terminal, Ctrl+C must reach the app.
                    keyboard.input_forward(
                        self,
                        event.key_code(),
                        event.state(),
                        serial,
                        time,
                        mods_changed,
                    );
                } else if event.state() == KeyState::Pressed && self.edit_document_key(sym) {
                    // No Wayland surface has focus, so this keystroke is
                    // the document's. `edit_document_key` reports
                    // whether it consumed it.
                } else {
                    keyboard.input_forward(
                        self,
                        event.key_code(),
                        event.state(),
                        serial,
                        time,
                        mods_changed,
                    );
                }
            }

            // Smithay's winit backend never emits relative motion; we
            // synthesize a delta from successive absolutes in the
            // `PointerMotionAbsolute` arm below.
            InputEvent::PointerMotion { .. } => {}

            InputEvent::PointerMotionAbsolute { event, .. } => {
                let Some(output) = self.page.active_space.outputs().next() else {
                    return;
                };
                let Some(output_geo) = self.page.active_space.output_geometry(output) else {
                    return;
                };
                let Some(pointer) = self.seat.get_pointer() else {
                    return;
                };
                let new_abs = event.position_transformed(output_geo.size) + output_geo.loc.to_f64();
                let delta = self.cursor.consume_raw_location(new_abs);
                let time_msec = event.time_msec();
                let new_under = self.surface_under(new_abs);

                let serial = SERIAL_COUNTER.next_serial();

                // Always emit relative motion — no-op for clients that
                // haven't bound zwp_relative_pointer_v1.
                pointer.relative_motion(
                    self,
                    new_under.clone(),
                    &RelativeMotionEvent {
                        delta,
                        delta_unaccel: delta,
                        utime: time_msec as u64 * 1000,
                    },
                );

                if tracing::enabled!(tracing::Level::DEBUG) {
                    let new_id = new_under.as_ref().map(|(s, _)| s.id());
                    let old_id = pointer.current_focus().map(|s| s.id());
                    if new_id != old_id {
                        let loc = new_under.as_ref().map(|(_, p)| *p);
                        tracing::debug!(
                            "pointer focus change: {:?} -> {:?} pos=({:.0},{:.0}) loc={:?}",
                            old_id,
                            new_id,
                            new_abs.x,
                            new_abs.y,
                            loc,
                        );
                    }
                }

                pointer.motion(
                    self,
                    new_under.clone(),
                    &MotionEvent {
                        location: new_abs,
                        serial,
                        time: time_msec,
                    },
                );
                pointer.frame(self);
            }

            InputEvent::PointerButton { event, .. } => {
                let Some(pointer) = self.seat.get_pointer() else {
                    return;
                };
                let Some(keyboard) = self.seat.get_keyboard() else {
                    return;
                };

                let serial = SERIAL_COUNTER.next_serial();
                let button = event.button_code();
                let button_state = event.state();

                if ButtonState::Pressed == button_state && !pointer.is_grabbed() {
                    let pos = pointer.current_location();
                    let under = self.surface_under(pos);
                    let under_surface = under.map(|(s, _)| s);

                    if event.button() == Some(MouseButton::Left) {
                        self.handle_left_click(pos);
                    }

                    // Clicking inside a figure focuses its app; clicking
                    // the document clears focus (so typing goes to the
                    // caret) and a drag selects.
                    let focus = under_surface
                        .as_ref()
                        .and_then(|s| self.focus_target_for_surface(s));

                    // Only change keyboard focus when clicking a
                    // different client. Clicking a popup surface from
                    // the same client (e.g. a Firefox menu) must NOT
                    // send wl_keyboard.leave to the toplevel —
                    // otherwise the client dismisses the popup before
                    // processing the button event.
                    let same_client = focus.as_ref().is_some_and(|new| {
                        keyboard.current_focus().is_some_and(|old| {
                            new.wl_surface()
                                .is_some_and(|s| old.same_client_as(&s.id()))
                        })
                    });
                    if !same_client {
                        keyboard.set_focus(self, focus, serial);
                        // A focus change invalidates the document's
                        // caret overlay and the focused figure's
                        // border.
                        self.needs_redraw = true;
                    }
                }

                pointer.button(
                    self,
                    &ButtonEvent {
                        button,
                        state: button_state,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }

            InputEvent::PointerAxis { event, .. } => {
                let Some(pointer) = self.seat.get_pointer() else {
                    return;
                };
                let source = event.source();

                let horizontal_amount = event.amount(Axis::Horizontal).unwrap_or_else(|| {
                    event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.
                });
                let vertical_amount = event.amount(Axis::Vertical).unwrap_or_else(|| {
                    event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.
                });
                let horizontal_amount_discrete = event.amount_v120(Axis::Horizontal);
                let vertical_amount_discrete = event.amount_v120(Axis::Vertical);

                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                if horizontal_amount != 0.0 {
                    frame = frame.value(Axis::Horizontal, horizontal_amount);
                    if let Some(discrete) = horizontal_amount_discrete {
                        frame = frame.v120(Axis::Horizontal, discrete as i32);
                    }
                }
                if vertical_amount != 0.0 {
                    frame = frame.value(Axis::Vertical, vertical_amount);
                    if let Some(discrete) = vertical_amount_discrete {
                        frame = frame.v120(Axis::Vertical, discrete as i32);
                    }
                }

                if source == AxisSource::Finger {
                    if event.amount(Axis::Horizontal) == Some(0.0) {
                        frame = frame.stop(Axis::Horizontal);
                    }
                    if event.amount(Axis::Vertical) == Some(0.0) {
                        frame = frame.stop(Axis::Vertical);
                    }
                }

                pointer.axis(self, frame);
                pointer.frame(self);
            }

            _ => {}
        }
    }

    /// Left click: figure hit-test first, then the document.
    ///
    /// A figure's **edge** starts a resize grab (which rewrites the
    /// `\app` args); its interior focuses its app; anywhere else on the
    /// page places the caret.
    fn handle_left_click(&mut self, pos: Point<f64, Logical>) {
        let Some(figure) = self.doc.figure_at(pos).cloned() else {
            // The document. Place the caret where the user clicked so the
            // next keystroke lands where they expected.
            let hit = self.doc.layout().screen_to_doc(pos).and_then(|p| {
                self.doc
                    .layout()
                    .glyphs()
                    .and_then(|g| g.byte_for_point(V2::new(p.x as f32, p.y as f32)))
            });
            match hit {
                Some((byte, _after_space)) => self.doc.model_mut().set_caret(byte),
                None => self.doc.model_mut().clear_selection(),
            }
            self.needs_redraw = true;
            return;
        };

        let edge = crate::grabs::FigureResizeGrab::edge_at(&figure.rect, pos);
        if edge != smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge::None
    {
        let start_data = GrabStartData {
            focus: None,
            button: MouseButton::Left as u32,
            location: pos,
        };
        let serial = SERIAL_COUNTER.next_serial();
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let grab = crate::grabs::FigureResizeGrab::new(
            start_data,
            figure.key.clone(),
            figure.rect,
            edge,
        );
        // `Focus::Clear`: a figure resize manipulates the page, not a
        // client surface, so nothing should take pointer focus mid-drag.
        pointer.set_grab(self, grab, serial, Focus::Clear);
        return;
    }

        tracing::debug!("figure {} clicked", figure.key);
        if let Some(app_id) = figure.app_id {
            // Report the click to the control plane so an external frontend
            // can highlight the same figure.
            self.ipc.send(crate::ipc::OutgoingMessage::FigureBound {
                figure: figure.key.clone(),
                window_id: app_id,
                title: figure.title.clone().unwrap_or_default(),
            });
        } else if self.relaunch_dormant(&figure.key) {
            self.needs_redraw = true;
        }
    }

    /// Spawn `key`'s saved command, if it is dormant and has one.
    ///
    /// A single click is enough, and deliberately so: a bound figure's click
    /// belongs to its app, but a dormant figure has no client, so nothing else
    /// can claim the click. Waiting for a double-click would be inventing a
    /// gesture out of a case where the click is already unclaimed.
    fn relaunch_dormant(&mut self, key: &str) -> bool {
        let Some((cmd, args)) = self.doc.relaunch_target(key) else {
            // Dormant with no saved command: an empty slot with nothing to open.
            // Not an error, and not worth a warning per click.
            tracing::debug!("relaunch: {key} has no saved command");
            return false;
        };
        tracing::info!("relaunch: {cmd} into {key} (click)");
        let display = self.xwayland.display();
        crate::util::spawn_child(&cmd, &args, display, self);
        true
    }

    /// Apply a global document binding.
    fn run_doc_action(&mut self, action: crate::docui::keymap::Action) {
        use crate::docui::keymap::Action;
        match action {
            Action::NextPage => {
                let next = self.doc.current_page() + 1;
                self.goto_page(next);
            }
            Action::PreviousPage => {
                let prev = self.doc.current_page().saturating_sub(1);
                self.goto_page(prev);
            }
            Action::DocumentStart => self.doc.model_mut().set_caret(0),
            Action::DocumentEnd => {
                let end = self.doc.model().len();
                self.doc.model_mut().set_caret(end);
            }
            Action::LineStart | Action::LineEnd => {
                self.move_caret_linewise(action == Action::LineStart)
            }
            Action::FocusDocument => self.focus_document(),
            Action::OpenLauncher => self.open_launcher(),
            Action::CloneFigure => self.clone_focused_figure(),
        }
        self.relayout_document();
        self.needs_redraw = true;
    }

    /// Move the caret to the start/end of its line.
    fn move_caret_linewise(&mut self, to_start: bool) {
        let (text, caret) = {
            let m = self.doc.model();
            (m.text().to_string(), m.caret())
        };
        let line_start = text[..caret].rfind('\n').map_or(0, |i| i + 1);
        let line_end = text[caret..].find('\n').map_or(text.len(), |i| caret + i);
        self.doc
            .model_mut()
            .set_caret(if to_start { line_start } else { line_end });
    }

    /// Drop keyboard focus from the focused figure back to the document
    /// caret.
    fn focus_document(&mut self) {
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, None, serial);
        }
    }

    /// `Ctrl+Shift+Return`: spawn the app bound to the focused figure's
    /// saved command, or the dormant figure the pointer is over.
    ///
    /// There is no interactive prompt in v1 — a launcher needs a text
    /// input surface of its own, and building one is a bigger decision
    /// than a key binding. This wires the key to the one thing that
    /// unambiguously means "put an app here".
    ///
    /// Preference order is the pointed-at figure, then the first dormant one.
    /// It used to be *only* the first, which meant a document with several
    /// figures always relaunched whichever came first rather than the one
    /// under the pointer.
    fn open_launcher(&mut self) {
        let pointer = self
            .seat
            .get_pointer()
            .map(|p| p.current_location())
            .unwrap_or_default();
        let target = self
            .doc
            .dormant_figure_at(pointer)
            .map(str::to_string)
            .or_else(|| self.doc.first_dormant_figure().map(str::to_string));
        let Some(key) = target else {
            tracing::info!("launcher: no dormant figure to launch into");
            return;
        };
        if !self.relaunch_dormant(&key) {
            tracing::info!("launcher: {key} names no launch command");
        }
    }

    /// Plain `Return` over a dormant figure relaunches it instead of inserting
    /// a newline.
    ///
    /// The document has no key handler of its own here, so without this a
    /// dormant figure is unreachable: it has no client to take focus, so
    /// `Return` would always mean "insert a newline" in the caption beside it.
    ///
    /// Returns `true` if the key was consumed by a relaunch.
    fn relaunch_dormant_under_pointer(&mut self) -> bool {
        let pointer = self
            .seat
            .get_pointer()
            .map(|p| p.current_location())
            .unwrap_or_default();
        let Some(key) = self.doc.dormant_figure_at(pointer).map(str::to_string) else {
            return false;
        };
        self.relaunch_dormant(&key)
    }

    /// `Ctrl+Shift+M`: duplicate the focused figure's statement, making a
    /// mirror that renders the same app twice.
    fn clone_focused_figure(&mut self) {
        let key = self
            .seat
            .get_keyboard()
            .and_then(|k| k.current_focus())
            .and_then(|focus| {
                let wl = focus.wl_surface()?;
                let app_id = self.apps.id_for_surface(&wl)?;
                self.doc
                    .figures()
                    .figure_of_app(app_id)
                    .map(|f| f.key.clone())
            })
            .or_else(|| {
                // Page-aware: an off-page figure has a placeholder rect at the
                // origin, so matching it here would clone the wrong statement.
                self.doc
                    .figure_at(
                        self.seat
                            .get_pointer()
                            .map(|p| p.current_location())
                            .unwrap_or_default(),
                    )
                    .map(|f| f.key.clone())
            });
        let Some(key) = key else {
            tracing::debug!("clone_figure: no figure to clone");
            return;
        };
        if let Some(figure) = self.doc.figures().get(&key).cloned() {
            crate::docui::edit::clone_figure(self.doc.model_mut(), &figure);
            tracing::info!("cloned figure {key}");
        }
    }

    /// Toggle keyboard focus between the focused figure and the document.
    /// Called on WakeUp key press (the "wake the shell" key).
    fn handle_wakeup(&mut self) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let serial = SERIAL_COUNTER.next_serial();
        match keyboard.current_focus() {
            // A figure (or dialog) has focus: drop back to the document.
            Some(_) => {
                if let Some(last) = self.focus.last_app_focus.take() {
                    self.focus.last_app_focus = Some(last);
                }
                keyboard.set_focus(self, None, serial);
                self.needs_redraw = true;
            }
            // Nothing focused: return to the last figure the user had.
            None => {
                if let Some(saved) = self.focus.last_app_focus.take() {
                    if saved.alive() {
                        keyboard.set_focus(self, Some(saved), serial);
                        self.needs_redraw = true;
                    }
                }
            }
        }
    }

    /// Apply a key to the document when nothing else has keyboard focus.
    ///
    /// Returns `true` if the key was consumed. This is ordinary text
    /// editing — the same set any editor has — so it lives in one place
    /// rather than being scattered through the input path.
    fn edit_document_key(&mut self, keysym: u32) -> bool {
        // Ctrl+Home/End are in the global table (LineStart/LineEnd); reuse
        // the same implementation rather than spelling them twice.
        if keymap::classify(keysym, KeyState::Pressed, false, false, false)
            == Some(Action::LineStart)
        {
            self.move_caret_linewise(true);
            return true;
        }

        let Some(keyboard) = self.seat.get_keyboard() else {
            return false;
        };
        let mods = keyboard.modifier_state();
        let ctrl = mods.ctrl;
        let shift = mods.shift;

        // A plain Return over a dormant figure means "relaunch", not "new
        // paragraph" — checked before the text path so the newline is never
        // inserted and then undone.
        if keysym == keysyms::KEY_Return && !ctrl && !shift && self.relaunch_dormant_under_pointer()
        {
            return true;
        }

        match keysym {
            keysyms::KEY_BackSpace => {
                self.doc.model_mut().backspace();
            }
            keysyms::KEY_Delete => {
                self.doc.model_mut().delete_forward();
            }
            keysyms::KEY_Left => self.move_caret_horizontally(ctrl, shift, false),
            keysyms::KEY_Right => self.move_caret_horizontally(ctrl, shift, true),
            k if k == keysyms::KEY_Up || k == keysyms::KEY_Down => {
                self.move_caret_vertically(k == keysyms::KEY_Down)
            }
            _ => {
                // Ctrl+letter is a chord, not a character; anything else
                // with a control modifier is the client's business.
                if ctrl {
                    return self.edit_document_control(keysym);
                }
                let Some(text) = Self::char_for_keysym(keysym) else {
                    return false;
                };
                self.doc.model_mut().insert_at_caret(&text);
            }
        }
        true
    }

    /// Ctrl chords that operate on the document.
    fn edit_document_control(&mut self, keysym: u32) -> bool {
        // Read the modifiers before taking `&mut DocModel`, or the borrow
        // checker will (rightly) refuse.
        let shift = self
            .seat
            .get_keyboard()
            .is_some_and(|kb| kb.modifier_state().shift);
        // Ctrl+C/X/V take `&mut self`, so they're handled before the model
        // borrow rather than inside the match.
        match keysym {
            keysyms::KEY_c => self.copy_selection(),
            keysyms::KEY_x => self.cut_selection(),
            keysyms::KEY_v => self.paste_clipboard(),
            _ => {
                let m = self.doc.model_mut();
                match keysym {
                    // Ctrl+Z / Ctrl+Shift+Z (and Ctrl+Y) — undo/redo.
                    k if k == keysyms::KEY_z || k == keysyms::KEY_y => {
                        let redo = k == keysyms::KEY_y || shift;
                        if redo {
                            m.redo();
                        } else {
                            m.undo();
                        }
                    }
                    keysyms::KEY_a => m.set_selection(0..m.len()),
                    _ => return false,
                }
            }
        }
        true
    }

    /// Copy the selection to the system clipboard.
    fn copy_selection(&mut self) {
        let text = self.doc.model().selected_text();
        if text.is_empty() {
            return;
        }
        crate::clipboard_bridge::set_host_clipboard(self, &text);
    }

    /// Cut: copy the selection, then delete it.
    fn cut_selection(&mut self) {
        if !self.doc.model().has_selection() {
            return;
        }
        self.copy_selection();
        self.doc.model_mut().take_selection();
    }

    /// Paste the host clipboard at the caret.
    fn paste_clipboard(&mut self) {
        let Some(text) = crate::clipboard_bridge::host_clipboard(self) else {
            return;
        };
        if text.is_empty() {
            return;
        }
        self.doc.model_mut().take_selection();
        self.doc.model_mut().insert_at_caret(&text);
    }

    /// Move the caret left/right by one grapheme, or one word with Ctrl.
    fn move_caret_horizontally(&mut self, word: bool, extend: bool, forward: bool) {
        let (text, caret, len) = {
            let m = self.doc.model();
            (m.text().to_string(), m.caret(), m.len())
        };
        let next = if word {
            // `wordnav` needs the atomic ranges math spans occupy, so that
            // Ctrl+Left doesn't stop inside a formula's source text.
            let atomic = mathed_core::transform::math_span_ranges(&text);
            if forward {
                mathed_core::wordnav::word_boundary_right(&text, caret, &atomic)
            } else {
                mathed_core::wordnav::word_boundary_left(&text, caret, &atomic)
            }
            .min(len)
        } else if forward {
            Self::next_grapheme(&text, caret)
        } else {
            Self::prev_grapheme(&text, caret)
        };
        let model = self.doc.model_mut();
        if extend {
            model.extend_selection_to(next);
        } else {
            model.set_caret(next);
        }
    }

    /// Move the caret between lines, using the glyph index's line bands so
    /// navigation agrees with what's actually on screen.
    fn move_caret_vertically(&mut self, down: bool) {
        let Some(glyphs) = self.doc.layout().glyphs() else {
            return;
        };
        let caret = self.doc.model().caret();
        let Some(band) = glyphs.band_for_byte(caret) else {
            return;
        };
        let target = if down {
            band.saturating_add(1)
        } else {
            band.saturating_sub(1)
        };
        // `band_for_byte` counts from the top; the first entry of the target
        // band is that line's leftmost byte.
        let target_byte = glyphs
            .entries
            .iter()
            .find(|e| e.band as usize == target)
            .map(|e| e.doc_byte);
        if let Some(byte) = target_byte {
            self.doc.model_mut().set_caret(byte);
        }
    }

    fn next_grapheme(text: &str, pos: usize) -> usize {
        if pos >= text.len() {
            return text.len();
        }
        let mut start = pos;
        while !text.is_char_boundary(start) {
            start += 1;
        }
        match text[start..].graphemes(true).next() {
            Some(g) => start + g.len(),
            None => text.len(),
        }
    }

    fn prev_grapheme(text: &str, pos: usize) -> usize {
        let end = pos.min(text.len());
        let mut start = end;
        while start > 0 {
            start -= 1;
            if !text.is_char_boundary(start) {
                continue;
            }
            if let Some((offset, _)) = text[..start].grapheme_indices(true).next_back() {
                return offset;
            }
            return 0;
        }
        0
    }

    /// The character a printable keysym stands for.
    fn char_for_keysym(keysym: u32) -> Option<String> {
        // The keysym itself is the character's code point for the printable
        // ASCII range (X11 keysym convention), so no keymap lookup is
        // needed — and it already has Shift applied.
        let raw = keysym;
        if (0x20..0x7f).contains(&raw) {
            return char::from_u32(raw).map(String::from);
        }
        let s = match keysym {
            keysyms::KEY_space => " ",
            keysyms::KEY_Return | keysyms::KEY_KP_Enter => "\n",
            keysyms::KEY_Tab => "\t",
            keysyms::KEY_KP_Add => "+",
            keysyms::KEY_KP_Subtract => "-",
            keysyms::KEY_KP_Multiply => "*",
            keysyms::KEY_KP_Divide => "/",
            keysyms::KEY_KP_Decimal => ".",
            _ => return None,
        };
        Some(s.to_string())
    }

    /// emthin's winit window lost focus (Alt+Tab away, minimize, etc.).
    /// Save the current keyboard focus and clear it so embedded clients
    /// stop thinking they still have focus. `focus_changed` cascades the
    /// clear to IME, data_device, and primary_selection. Pointer focus
    /// is released here too (bundled with keyboard until winit gives us
    /// separate `CursorLeft` events — YAGNI for now).
    pub fn on_focus_leave(&mut self) {
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            self.focus
                .enter(crate::state::FocusOverride::Host, keyboard.current_focus());
            keyboard.set_focus(self, None, serial);
        }
        if let Some(pointer) = self.seat.get_pointer() {
            pointer.motion(
                self,
                None,
                &MotionEvent {
                    location: pointer.current_location(),
                    serial,
                    time: 0,
                },
            );
            pointer.frame(self);
        }
    }

    /// emthin's winit window regained focus. Restore the keyboard focus
    /// saved by `on_focus_leave`; the `focus_changed` cascade re-enables
    /// IME (if the restored client has text_input_v3 bound) and rewires
    /// data_device / primary_selection.
    pub fn on_focus_enter(&mut self) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let Some(Some(saved)) = self.focus.exit(crate::state::FocusOverride::Host) else {
            return;
        };
        let serial = SERIAL_COUNTER.next_serial();
        keyboard.set_focus(self, Some(saved), serial);
    }
}
