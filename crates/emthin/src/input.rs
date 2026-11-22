use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, MouseButton, PointerAxisEvent, PointerButtonEvent, TouchEvent,
    },
    input::{
        keyboard::{keysyms, Keysym},
        pointer::{AxisFrame, ButtonEvent, Focus, GrabStartData, MotionEvent, RelativeMotionEvent},
    },
    reexports::wayland_server::Resource,
    utils::{IsAlive, Logical, Point, Rectangle, SERIAL_COUNTER},
    wayland::seat::WaylandFocus,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::docui::keymap;
use crate::docui::keymap::Action;
use crate::state::{EmthinState, TextClick, TextSelectionDrag, TouchPan, TouchPoint, ViewPanDrag};

impl EmthinState {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event, .. } => {
                // Apply a host keymap that arrived since the last key
                // *before* this one is translated — the intercept below
                // reads the seat's keymap. See `crate::host_keymap`.
                self.sync_host_keymap();
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
                let mut ghostty_mods = 0u16;
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
                        ghostty_mods = u16::from(mods.shift)
                            | (u16::from(mods.ctrl) << 1)
                            | (u16::from(mods.alt) << 2)
                            | (u16::from(mods.logo) << 3);
                        false
                    },
                );

                // One line per key event at DEBUG: the raw code, the
                // keysym the seat's keymap produced, and where the key
                // landed. This is the trace that localizes keymap
                // translation bugs (see `crate::host_keymap`).
                tracing::debug!(
                    "key: code={:?} state={:?} sym={:#x} action={} client_focus={}",
                    event.key_code(),
                    event.state(),
                    sym,
                    pending.is_some(),
                    keyboard.current_focus().is_some(),
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

                // A Core terminal has no Wayland surface, so it cannot hold
                // Smithay keyboard focus. Its focused figure is the fallback
                // owner while no client surface is focused.
                if keyboard.current_focus().is_none() {
                    if let Some(reference) = self.core_sessions.focused().cloned() {
                        if event.state() == KeyState::Pressed {
                            let bytes = self.core_sessions.key_bytes(&reference, sym, ghostty_mods);
                            if !bytes.is_empty()
                                && !self.core_sessions.send_input(&reference, &bytes)
                            {
                                self.core_sessions.clear_focus();
                                self.needs_redraw = true;
                            }
                        }
                        return;
                    }
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
                let core_mods = self
                    .seat
                    .get_keyboard()
                    .map(|keyboard| keyboard.modifier_state())
                    .map_or((false, false, false), |mods| {
                        (mods.shift, mods.alt, mods.ctrl)
                    });
                if let Some(drag) = self.core_mouse_drag.clone() {
                    let bytes = self.core_sessions.mouse_motion(
                        &drag.reference,
                        drag.rect,
                        new_abs,
                        Some(drag.button),
                        core_mods,
                    );
                    if !bytes.is_empty() {
                        self.core_sessions.send_input(&drag.reference, &bytes);
                    }
                    pointer.motion(
                        self,
                        None,
                        &MotionEvent {
                            location: new_abs,
                            serial: SERIAL_COUNTER.next_serial(),
                            time: time_msec,
                        },
                    );
                    pointer.frame(self);
                    return;
                }
                if let Some(pan) = self.view_pan_drag {
                    let dx = new_abs.x - pan.last.x;
                    let dy = new_abs.y - pan.last.y;
                    self.view_pan_drag = Some(ViewPanDrag { last: new_abs });
                    if self.doc.pan_view(dx, dy) {
                        self.relayout_document();
                        self.needs_redraw = true;
                    }
                }
                self.drag_text_selection(new_abs);
                let new_under = self.surface_under(new_abs);
                if new_under.is_none() {
                    if let Some((reference, rect)) = self.core_terminal_at(new_abs) {
                        let bytes = self
                            .core_sessions
                            .mouse_motion(&reference, rect, new_abs, None, core_mods);
                        if !bytes.is_empty() {
                            self.core_sessions.send_input(&reference, &bytes);
                            pointer.motion(
                                self,
                                None,
                                &MotionEvent {
                                    location: new_abs,
                                    serial: SERIAL_COUNTER.next_serial(),
                                    time: time_msec,
                                },
                            );
                            pointer.frame(self);
                            return;
                        }
                    }
                }

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

                if !pointer.is_grabbed() {
                    let pos = pointer.current_location();
                    let core_hit = self.core_terminal_at(pos).filter(|(_, rect)| {
                        crate::grabs::FigureResizeGrab::edge_at(
                            rect,
                            pos,
                        ) == smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge::None
                    });
                    if let Some((reference, rect)) = core_hit {
                        if let Some(button_index) = match event.button() {
                            Some(MouseButton::Left) => Some(0),
                            Some(MouseButton::Middle) => Some(1),
                            Some(MouseButton::Right) => Some(2),
                            _ => None,
                        } {
                            if button_state == ButtonState::Pressed {
                                self.core_sessions.focus(&reference);
                                keyboard.set_focus(self, None, serial);
                                self.core_mouse_drag = Some(crate::state::CoreMouseDrag {
                                    reference: reference.clone(),
                                    rect,
                                    button: button_index,
                                });
                                self.needs_redraw = true;
                            } else if self
                                .core_mouse_drag
                                .as_ref()
                                .is_some_and(|drag| drag.button == button_index)
                            {
                                self.core_mouse_drag = None;
                            }
                            let mods = keyboard.modifier_state();
                            let bytes = self.core_sessions.mouse_button(
                                &reference,
                                rect,
                                pos,
                                button_index,
                                button_state == ButtonState::Pressed,
                                (mods.shift, mods.alt, mods.ctrl),
                            );
                            if !bytes.is_empty() {
                                self.core_sessions.send_input(&reference, &bytes);
                            }
                            pointer.frame(self);
                            return;
                        }
                    }
                }

                if ButtonState::Pressed == button_state && !pointer.is_grabbed() {
                    let pos = pointer.current_location();
                    let under = self.surface_under(pos);
                    let under_surface = under.map(|(s, _)| s);

                    if event.button() == Some(MouseButton::Left) {
                        self.handle_left_click(pos, event.time_msec());
                    } else if event.button() == Some(MouseButton::Middle)
                        && under_surface.is_none()
                        && self.doc.layout().screen_to_doc(pos).is_some()
                    {
                        // Middle-drag pans the document background. Over an
                        // app surface the button still belongs to the client
                        // (and to primary-selection paste).
                        self.view_pan_drag = Some(ViewPanDrag { last: pos });
                        self.text_selection.drag = None;
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
                        if focus.is_some() {
                            self.core_sessions.clear_focus();
                        }
                        keyboard.set_focus(self, focus, serial);
                        // A focus change invalidates the document's
                        // caret overlay and the focused figure's
                        // border.
                        self.needs_redraw = true;
                    }
                }

                if event.button() == Some(MouseButton::Left)
                    && button_state == ButtonState::Released
                {
                    // The drag's selection stays; only the gesture ends.
                    self.text_selection.drag = None;
                    self.sync_primary_selection();
                }
                if event.button() == Some(MouseButton::Middle)
                    && button_state == ButtonState::Released
                {
                    self.view_pan_drag = None;
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

                // Ctrl+wheel and Ctrl+two-finger scroll zoom the document
                // around the pointer. Over an app surface the gesture belongs
                // to the client, exactly like the wheel itself.
                let pointer_pos = pointer.current_location();
                let ctrl = self
                    .seat
                    .get_keyboard()
                    .is_some_and(|keyboard| keyboard.modifier_state().ctrl);
                let vertical_notches = vertical_amount_discrete
                    .map(|discrete| discrete / 120.0)
                    .unwrap_or(vertical_amount / 15.0);
                if ctrl
                    && vertical_notches.abs() > 0.01
                    && self.surface_under(pointer_pos).is_none()
                    && self.core_terminal_at(pointer_pos).is_none()
                {
                    let factor = wheel_zoom_factor(vertical_notches);
                    if self.doc.zoom_view_at(factor, pointer_pos) {
                        self.relayout_document();
                        self.needs_redraw = true;
                    }
                    pointer.frame(self);
                    return;
                }

                if let Some((reference, _)) = self.core_terminal_at(pointer_pos) {
                    if let Some(bytes) =
                        self.core_sessions.wheel(&reference, vertical_notches > 0.0)
                    {
                        if !bytes.is_empty() {
                            self.core_sessions.send_input(&reference, &bytes);
                        }
                        self.needs_redraw = true;
                    }
                    pointer.frame(self);
                    return;
                }

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

            InputEvent::TouchDown { event, .. } => {
                let Some(output) = self.page.active_space.outputs().next() else {
                    return;
                };
                let Some(output_geo) = self.page.active_space.output_geometry(output) else {
                    return;
                };
                let slot = i32::from(event.slot());
                let pos = event.position_transformed(output_geo.size) + output_geo.loc.to_f64();
                self.handle_touch_down(slot, pos, event.time_msec());
            }

            InputEvent::TouchMotion { event, .. } => {
                let slot = i32::from(event.slot());
                let Some(output) = self.page.active_space.outputs().next() else {
                    return;
                };
                let Some(output_geo) = self.page.active_space.output_geometry(output) else {
                    return;
                };
                let pos = event.position_transformed(output_geo.size) + output_geo.loc.to_f64();
                self.handle_touch_motion(slot, pos);
            }

            InputEvent::TouchUp { event, .. } => {
                self.handle_touch_up(i32::from(event.slot()));
            }

            InputEvent::TouchCancel { event, .. } => {
                self.handle_touch_cancel(i32::from(event.slot()));
            }

            _ => {}
        }
    }

    /// Feed a synthetic touch contact from the IPC test seam. The live harness
    /// uses this because XTEST cannot synthesize multitouch; it goes through
    /// the same handlers as a real `TouchDown`/`TouchMotion`/`TouchUp`.
    pub fn inject_touch(&mut self, phase: crate::ipc::TouchPhase, slot: i32, x: f64, y: f64) {
        let pos = Point::new(x, y);
        match phase {
            crate::ipc::TouchPhase::Down => self.handle_touch_down(slot, pos, 0),
            crate::ipc::TouchPhase::Motion => self.handle_touch_motion(slot, pos),
            crate::ipc::TouchPhase::Up => self.handle_touch_up(slot),
            crate::ipc::TouchPhase::Cancel => self.handle_touch_cancel(slot),
        }
        self.needs_redraw = true;
    }

    fn handle_touch_down(&mut self, slot: i32, pos: Point<f64, Logical>, time_msec: u32) {
        self.touch.points.retain(|point| point.slot != slot);
        self.touch.points.push(TouchPoint {
            slot,
            position: pos,
        });

        // Two fingers on a zoomed page pan it. Turning the one-finger
        // selection into a pan as soon as the second contact lands is
        // what keeps the gestures from fighting: from here until the
        // contacts lift, neither slot is a selection anchor.
        if self.touch.points.len() == 2 && self.touch.pan.is_none() && self.doc.view_can_pan() {
            let a = self.touch.points[0].position;
            let b = self.touch.points[1].position;
            self.touch.pan = Some(TouchPan {
                slots: [self.touch.points[0].slot, self.touch.points[1].slot],
                last_center: Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0),
            });
            self.text_selection.touch_slot = None;
            self.text_selection.drag = None;
            self.needs_redraw = true;
            return;
        }
        if self.touch.points.len() > 1 || self.text_selection.touch_slot.is_some() {
            // A second finger must not fight the selection anchor, and
            // once a pan has ended the remaining contact stays inert
            // until it lifts.
            return;
        }
        self.text_selection.touch_slot = Some(slot);
        // Touch is a document gesture here. A touch on a figure or a
        // rendered citation keeps the mouse rules (which would start a
        // resize/relaunch or popup) out of the way.
        if self.doc.figure_at(pos).is_none() && self.doc.citation_at(pos).is_none() {
            self.focus_document();
            self.start_text_selection(pos, time_msec);
            self.sync_primary_selection();
            self.needs_redraw = true;
        }
    }

    fn handle_touch_motion(&mut self, slot: i32, pos: Point<f64, Logical>) {
        if let Some(point) = self
            .touch
            .points
            .iter_mut()
            .find(|point| point.slot == slot)
        {
            point.position = pos;
        }
        if let Some(pan) = self.touch.pan {
            if !pan.slots.contains(&slot) {
                return;
            }
            let first = self
                .touch
                .points
                .iter()
                .find(|point| point.slot == pan.slots[0]);
            let second = self
                .touch
                .points
                .iter()
                .find(|point| point.slot == pan.slots[1]);
            let (Some(first), Some(second)) = (first, second) else {
                return;
            };
            let center = Point::new(
                (first.position.x + second.position.x) / 2.0,
                (first.position.y + second.position.y) / 2.0,
            );
            let dx = center.x - pan.last_center.x;
            let dy = center.y - pan.last_center.y;
            if let Some(pan) = self.touch.pan.as_mut() {
                pan.last_center = center;
            }
            if self.doc.pan_view(dx, dy) {
                tracing::debug!("touch pan: {dx:+.0},{dy:+.0}");
                self.relayout_document();
                self.needs_redraw = true;
            }
            return;
        }
        if self.text_selection.touch_slot != Some(slot) {
            return;
        }
        self.drag_text_selection(pos);
        self.sync_primary_selection();
        self.needs_redraw = true;
    }

    fn handle_touch_up(&mut self, slot: i32) {
        self.touch.points.retain(|point| point.slot != slot);
        if let Some(pan) = self.touch.pan {
            if pan.slots.contains(&slot) {
                self.touch.pan = None;
                self.needs_redraw = true;
            }
            return;
        }
        if self.text_selection.touch_slot != Some(slot) {
            return;
        }
        self.text_selection.touch_slot = None;
        self.text_selection.drag = None;
        self.sync_primary_selection();
        self.needs_redraw = true;
    }

    fn handle_touch_cancel(&mut self, slot: i32) {
        self.touch.points.retain(|point| point.slot != slot);
        if let Some(pan) = self.touch.pan {
            if pan.slots.contains(&slot) {
                self.touch.pan = None;
                self.needs_redraw = true;
            }
            return;
        }
        if self.text_selection.touch_slot != Some(slot) {
            return;
        }
        self.text_selection.touch_slot = None;
        self.text_selection.drag = None;
        self.needs_redraw = true;
    }

    /// Publish the current document selection as PRIMARY, or clear it when the
    /// selection collapsed. Cached so ordinary typing does not re-own the host
    /// selection on every keystroke.
    pub fn sync_primary_selection(&mut self) {
        let text = self.doc.model().selected_text();
        if text.is_empty() {
            // Do not clear another app's PRIMARY just because typing in emthin
            // collapsed an already-empty document selection. Only retire a
            // selection emthin actually published.
            let owned_text = self
                .selection
                .primary_cache
                .as_deref()
                .is_some_and(|cached| !cached.is_empty());
            if owned_text {
                crate::clipboard_bridge::set_host_primary(self, "");
            }
            return;
        }
        if self.selection.primary_cache.as_deref() == Some(text.as_str()) {
            return;
        }
        crate::clipboard_bridge::set_host_primary(self, &text);
    }

    /// Start a document selection gesture at `pos`.
    ///
    /// One click places the caret and arms a drag; a second click within
    /// 400 ms and 4 px selects the word; a third selects the source line. The
    /// gesture state is reset by a click on a figure or citation and by the
    /// next click elsewhere, so double-click counting never leaks across
    /// unrelated targets.
    fn start_text_selection(&mut self, pos: Point<f64, Logical>, time: u32) {
        let Some(byte) = self.doc.byte_at(pos) else {
            self.doc.model_mut().clear_selection();
            self.text_selection.drag = None;
            self.text_selection.last_click = None;
            self.sync_primary_selection();
            return;
        };

        let count = match self.text_selection.last_click {
            Some(last)
                if time.saturating_sub(last.time) <= 400
                    && (pos.x - last.position.x).abs() <= 4.0
                    && (pos.y - last.position.y).abs() <= 4.0 =>
            {
                // 1 → 2 → 3 → 1: a fourth click starts a new group rather
                // than selecting ever-larger regions.
                last.count % 3 + 1
            }
            _ => 1,
        };
        self.text_selection.last_click = Some(TextClick {
            time,
            position: pos,
            count,
        });
        tracing::debug!("text selection: click count={count} byte={byte}");

        let selected = match count {
            2 => self.doc.select_word_at(byte),
            3 => self.doc.select_line_at(byte),
            _ => false,
        };
        if selected {
            self.text_selection.drag = None;
        } else {
            self.doc.model_mut().set_caret(byte);
            self.text_selection.drag = Some(TextSelectionDrag {
                start: pos,
                moved: false,
            });
        }
        self.sync_primary_selection();
    }

    /// Extend an in-progress document drag. A small movement is click slop:
    /// the press's caret must survive a shaky hand.
    fn drag_text_selection(&mut self, pos: Point<f64, Logical>) {
        let Some(drag) = self.text_selection.drag else {
            return;
        };
        if !drag.moved {
            let dx = pos.x - drag.start.x;
            let dy = pos.y - drag.start.y;
            if dx * dx + dy * dy < 9.0 {
                return;
            }
            // Re-borrow after the check: `drag` is a copy, the state is not.
            if let Some(drag) = self.text_selection.drag.as_mut() {
                drag.moved = true;
            }
        }
        let Some(byte) = self.doc.byte_at(pos) else {
            return;
        };
        // The anchor is the caret set by the press; extending keeps it fixed
        // while the pointer moves in either direction.
        self.doc.model_mut().extend_selection_to(byte);
        self.sync_primary_selection();
        tracing::debug!("text selection: extend to byte={byte}");
        self.needs_redraw = true;
    }

    fn core_terminal_at(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(mathed_core::figures::TerminalRef, Rectangle<i32, Logical>)> {
        let figure = self.doc.figure_at(pos)?;
        Some((figure.spec.terminal.clone()?, figure.rect))
    }

    /// Left click: figure hit-test first, then the document.
    ///
    /// A figure's **edge** starts a resize grab (which rewrites the
    /// `\app` args); its interior focuses its app; anywhere else on the
    /// page starts a text selection.
    fn handle_left_click(&mut self, pos: Point<f64, Logical>, time: u32) {
        let Some(figure) = self.doc.figure_at(pos).cloned() else {
            if let Some(number) = self.doc.citation_at(pos) {
                self.text_selection.drag = None;
                self.text_selection.last_click = None;
                let ctrl = self
                    .seat
                    .get_keyboard()
                    .is_some_and(|keyboard| keyboard.modifier_state().ctrl);
                if ctrl {
                    // Ctrl+click is go-to-definition. A bibliography or a
                    // dangling document reference has no target, so fall
                    // back to the details popup rather than doing nothing.
                    if let Err(error) = self.doc.goto_citation(number) {
                        tracing::debug!("citation goto: {error}");
                        if let Err(error) = self.doc.toggle_citation(number) {
                            tracing::debug!("citation: {error}");
                        }
                    } else {
                        self.publish_document_page();
                    }
                } else if let Err(error) = self.doc.toggle_citation(number) {
                    tracing::debug!("citation: {error}");
                }
                self.needs_redraw = true;
                return;
            }
            // The document. Group the click, then place the caret or select
            // the word/line under it; a single-click drag extends from there.
            self.start_text_selection(pos, time);
            self.needs_redraw = true;
            return;
        };

        self.text_selection.drag = None;
        self.text_selection.last_click = None;
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
            figure.stable_id.clone(),
            figure.rect,
            edge,
            self.doc.document_instance(),
            self.doc.model().content_generation(),
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
    pub(crate) fn relaunch_dormant(&mut self, key: &str) -> bool {
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
        let context = keymap::ActionContext {
            has_query: !self.doc.find().query().is_empty(),
            can_history_back: self.doc.can_history_back(),
            can_history_forward: self.doc.can_history_forward(),
            citation_open: self.doc.citation_popup().is_open(),
        };
        if !keymap::available(action, context) {
            tracing::debug!("action {action:?} does not apply in this context");
            return;
        }
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
            Action::FocusDocument => {
                // A completion preview is the innermost transient state, then
                // the find bar, then a citation popup. Escape only leaves the
                // figure once none of them is open.
                if self.doc.cancel_completion() {
                    // The completion was the innermost transient state.
                } else if self.doc.close_launcher() {
                    // The launcher palette closes before the find bar.
                } else if self.doc.close_find() {
                    // Find may have moved the page while stepping matches.
                    self.publish_document_page();
                } else if !self.doc.close_citation() {
                    self.focus_document();
                }
            }
            Action::TextLarger | Action::TextSmaller => {
                let size = self.doc.step_text_size(action == Action::TextLarger);
                tracing::info!("reading size: {size}pt");
            }
            Action::TextReset => {
                self.doc.reset_text_size();
                tracing::info!("reading size reset to {}pt", self.doc.text_size());
            }
            Action::ViewLarger | Action::ViewSmaller => {
                let zoom = self.doc.step_view_zoom(action == Action::ViewLarger);
                tracing::info!("view zoom: {zoom:.2}x");
            }
            Action::ViewReset => {
                self.doc.reset_view();
                tracing::info!("view reset to fit ({}x)", self.doc.view_zoom());
            }
            Action::HistoryBack => {
                if self.history_back() {
                    tracing::debug!("history: back");
                }
            }
            Action::HistoryForward => {
                if self.history_forward() {
                    tracing::debug!("history: forward");
                }
            }
            Action::ViewPanUp | Action::ViewPanDown => {
                const PAN_STEP: f64 = 48.0;
                let (dx, dy) = match action {
                    Action::ViewPanUp => (0.0, -PAN_STEP),
                    _ => (0.0, PAN_STEP),
                };
                if self.doc.pan_view(dx, dy) {
                    tracing::debug!("view pan: {dx:+.0},{dy:+.0}");
                }
            }
            Action::CitationStyleNext => {
                if self.doc.citation_popup().is_open() {
                    match self.doc.cycle_citation_style() {
                        Ok(style) => tracing::info!("citation style: {style}"),
                        Err(error) => tracing::warn!("citation style: {error}"),
                    }
                } else {
                    tracing::debug!("citation style: no citation is open");
                }
            }
            Action::CitationLocaleNext => {
                if self.doc.citation_popup().is_open() {
                    match self.doc.cycle_citation_locale() {
                        Ok(locale) => tracing::info!("citation locale: {locale}"),
                        Err(error) => tracing::warn!("citation locale: {error}"),
                    }
                } else {
                    tracing::debug!("citation locale: no citation is open");
                }
            }
            Action::CitationReset => {
                if self.doc.citation_popup().is_open() {
                    match self.doc.reset_citation_format() {
                        Ok(true) => tracing::info!("citation format reset"),
                        Ok(false) => tracing::debug!("citation format already default"),
                        Err(error) => tracing::warn!("citation format: {error}"),
                    }
                } else {
                    tracing::debug!("citation format: no citation is open");
                }
            }
            Action::Find => self.doc.open_find(),
            Action::FindNext | Action::FindPrevious => {
                // F3 works with the bar closed too: the last query keeps
                // steering, which is how every editor behaves.
                if self.doc.find_step(action == Action::FindNext).is_none() {
                    tracing::debug!("find: no match for {:?}", self.doc.find().query());
                } else {
                    self.publish_document_page();
                }
            }
            Action::OpenLauncher => self.relaunch_pointed(),
            Action::LauncherPalette => self.doc.open_launcher(),
            Action::CloneFigure => self.clone_focused_figure(),
            Action::VoiceTurn => {
                // Queue one turn for the `\voice` block at the caret.
                // A rejection (no harness installed, caret outside a
                // block, nothing to ask) is a normal press, not an
                // error: the document is untouched and the log line is
                // the only trace.
                if let Err(reason) = self.doc.voice_turn() {
                    tracing::info!("voice: {reason}");
                }
            }
        }
        self.sync_primary_selection();
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
        self.core_sessions.clear_focus();
        self.core_mouse_drag = None;
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, None, serial);
        }
    }

    /// `Ctrl+Shift+Return`: relaunch the dormant figure under the pointer,
    /// or the first dormant figure when the pointer is over none.
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
    fn relaunch_pointed(&mut self) {
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
                // Mirrors share one app, so `figure_of_app` would return
                // the first in document order — a figure that may be on
                // another page with its own `w × h`. Clone the one the
                // user is actually looking at.
                let page = self.doc.current_page();
                self.doc
                    .figures()
                    .figures_of_app(app_id)
                    .into_iter()
                    .find(|f| f.page == Some(page))
                    .or_else(|| self.doc.figures().figure_of_app(app_id))
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
    /// Returns `true` if the key was consumed. A keystroke that changed the
    /// document leaves the model dirty, and the event loop tick performs the
    /// re-layout.
    ///
    /// The re-layout itself is not optional — the page raster, the glyph index
    /// the caret is measured against, and every figure rect are products of the
    /// layout, and without it a keystroke changed the model while the screen
    /// went on showing the previous page. What changed is *when*: it used to run
    /// here, once per keystroke, and `layout_doc_paged` is a whole-document
    /// Typst compile. Measured on the e2e session document, that is 858 ms in a
    /// debug build, so a ten-character burst queued eight and a half seconds of
    /// serial typesetting — the loop could not service IPC or input while it
    /// ran, and the e2e's keyboard check read an empty document back.
    ///
    /// Deferring to `event_loop_tick` keeps the guarantee the old comment
    /// demanded (the screen still catches up before anything observes it —
    /// the tick runs at least every 100 ms and before IPC dispatch) while
    /// coalescing a burst into **one** typeset instead of one per character.
    /// A single keystroke is one loop iteration behind the model; a burst is
    /// one typeset for the whole burst.
    fn edit_document_key(&mut self, keysym: u32) -> bool {
        let revision = self.doc.model().revision();
        let handled = self.edit_document_key_inner(keysym);
        if self.doc.model().revision() != revision {
            self.doc.clear_citations();
        }
        if handled {
            // Backspace, arrows and typing all move the caret or the run; the
            // preview must describe the new position, not the previous key's.
            self.doc.refresh_completion();
            self.sync_primary_selection();
        }
        handled
    }

    /// Returns `true` if the key was consumed. This is ordinary text
    /// editing — the same set any editor has — so it lives in one place
    /// rather than being scattered through the input path.
    fn edit_document_key_inner(&mut self, keysym: u32) -> bool {
        // The launcher palette is the innermost text-capturing overlay: its
        // query must not leak into the find bar or the document behind it.
        if self.doc.launcher().is_open() {
            return self.edit_launcher_key(keysym);
        }
        // An open find bar owns the keyboard: its query is being typed, so a
        // character must not land in the document behind it.
        if self.doc.find().is_open() {
            return self.edit_find_key(keysym);
        }
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
                // Typing replaces an active selection, like any editor:
                // `insert_at_caret` alone would leave the selected text in
                // place and clear the anchor. Deleted first, so a `#` scan
                // below also sees the words the deletion freed as reusable.
                self.doc.model_mut().take_selection();
                // A non-run key (space, comma, punctuation, …) commits the
                // pending glyph before inserting itself: `-> ` becomes `→ `.
                // A run character instead extends the ASCII run, so the
                // preview is refreshed by the wrapper rather than committed.
                self.doc.refresh_completion();
                let extends = text
                    .chars()
                    .next()
                    .is_some_and(mathed_mini::completion_ui::CompletionUi::extends_run);
                if !extends {
                    self.doc.commit_completion();
                }
                if text == "#" {
                    // A typed `#` is a marker position, not a character: it
                    // becomes the RFC 1751 word for the lowest free marker
                    // number (`#i`, `#o`, `#ad`, …) unless the caret sits
                    // after a backslash, where `\#` is the Typst escape and
                    // a literal `#` is wanted. Same contract as mathed and
                    // mathed_mini's `insert_hash`.
                    let token = crate::docui::edit::typed_hash_token(self.doc.model());
                    self.doc.model_mut().insert_at_caret(&token);
                } else {
                    self.doc.model_mut().insert_at_caret(&text);
                }
            }
        }
        true
    }

    /// A key while the find bar is open: text edits the query, Backspace
    /// shortens it, Return steps. Everything else falls through, so paging and
    /// the global bindings keep working over an open bar.
    /// Keys for the app launcher palette. Return accepts (spawning through the
    /// document's own `launch:`), arrows step, and ordinary characters filter.
    fn edit_launcher_key(&mut self, keysym: u32) -> bool {
        match keysym {
            keysyms::KEY_BackSpace => {
                self.doc.launcher_backspace();
            }
            keysyms::KEY_Down => self.doc.launcher_step(true),
            keysyms::KEY_Up => self.doc.launcher_step(false),
            keysyms::KEY_Return | keysyms::KEY_KP_Enter => {
                if let Some(key) = self.doc.launcher_accept() {
                    if !self.relaunch_dormant(&key) {
                        tracing::info!("launcher: {key} names no launch command");
                    }
                }
                self.doc.close_launcher();
            }
            _ => {
                let Some(text) = Self::char_for_keysym(keysym) else {
                    return false;
                };
                for ch in text.chars() {
                    self.doc.launcher_push(ch);
                }
            }
        }
        self.needs_redraw = true;
        true
    }

    fn edit_find_key(&mut self, keysym: u32) -> bool {
        match keysym {
            keysyms::KEY_BackSpace => {
                self.doc.find_backspace();
            }
            keysyms::KEY_Return | keysyms::KEY_KP_Enter => {
                // Enter accepts the current match and closes the bar. With no
                // match yet (the query is still being typed) it steps instead.
                let found = self.doc.accept_find() || self.doc.find_step(true).is_some();
                if found {
                    self.publish_document_page();
                } else {
                    tracing::debug!("find: no match for {:?}", self.doc.find().query());
                }
                self.needs_redraw = true;
            }
            _ => {
                let Some(text) = Self::char_for_keysym(keysym) else {
                    return false;
                };
                for ch in text.chars() {
                    self.doc.find_push(ch);
                }
            }
        }
        self.needs_redraw = true;
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
            keysyms::KEY_1..=keysyms::KEY_9 => {
                let number = u64::from(keysym - keysyms::KEY_1 + 1);
                if let Err(error) = self.doc.toggle_citation(number) {
                    tracing::debug!("citation: {error}");
                }
                self.needs_redraw = true;
            }
            keysyms::KEY_c => self.copy_selection(),
            keysyms::KEY_x => self.cut_selection(),
            keysyms::KEY_v => self.paste_clipboard(),
            _ => {
                let m = self.doc.model_mut();
                match keysym {
                    // Ctrl+Z / Ctrl+Shift+Z (and Ctrl+Y) — undo/redo.
                    // Uppercase accepted: `sym` is the modified keysym, so
                    // Ctrl+Shift+Z (the documented redo) arrives as `Z`,
                    // not `z` — matching only lowercase made it unreachable.
                    k if k == keysyms::KEY_z
                        || k == keysyms::KEY_Z
                        || k == keysyms::KEY_y
                        || k == keysyms::KEY_Y =>
                    {
                        let redo = shift || k == keysyms::KEY_y || k == keysyms::KEY_Y;
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
        let Some(glyphs) = self.doc.current_glyphs() else {
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
        let mut end = pos.min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text[..end]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(offset, _)| offset)
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
        // The button-up may never arrive once the host window loses focus.
        self.text_selection.drag = None;
        self.text_selection.touch_slot = None;
        self.view_pan_drag = None;
        self.core_mouse_drag = None;
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

/// One wheel notch is 1/5 of an e-fold. Wayland axis deltas are positive
/// downwards, so wheel-up (a negative delta) must increase the zoom.
fn wheel_zoom_factor(vertical_notches: f64) -> f64 {
    (-vertical_notches * 0.2).exp()
}

#[cfg(test)]
mod tests {
    use super::{wheel_zoom_factor, EmthinState};

    #[test]
    fn wheel_up_zooms_in_and_wheel_down_zooms_out() {
        assert!(wheel_zoom_factor(-1.0) > 1.0);
        assert!(wheel_zoom_factor(1.0) < 1.0);
        assert!((wheel_zoom_factor(0.0) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn left_moves_one_grapheme_including_combining_and_emoji_sequences() {
        let text = "ab e\u{301} 👩‍💻";
        assert_eq!(EmthinState::prev_grapheme("abc", 3), 2);
        assert_eq!(
            EmthinState::prev_grapheme(text, text.len()),
            text.find('👩').unwrap()
        );
        let end = text.find(' ').unwrap() + 1 + "e\u{301}".len();
        assert_eq!(
            EmthinState::prev_grapheme(text, end),
            text.find('e').unwrap()
        );
    }
}
