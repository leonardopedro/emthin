//! Event loop tick — the per-frame idle callback for the compositor.

use smithay::utils::IsAlive;

use crate::state::EmthinState;

/// Called once per event loop iteration. Handles document/page
/// lifecycle, IPC dispatch, clipboard events, and pending geometry
/// timeouts.
pub fn event_loop_tick(state: &mut EmthinState) {
    let started = std::time::Instant::now();
    // --- Host keymap: apply a layout switch the listener snooped ---
    //
    // One try-recv when nothing arrived. The input path calls this too
    // (before a key is translated); the tick call keeps the seat current
    // even when the switch lands while the user is not typing, because
    // embedded clients share the seat's keymap.
    state.sync_host_keymap();

    // --- Wake a live Core retry when its backoff is due ---
    // `ensure_live` runs in the render pass, so an idle compositor needs one
    // frame requested here to make progress after a failed attach.
    if state.core_sessions.retry_due() {
        state.needs_redraw = true;
    }

    // --- Voice: finish turns whose harness run has completed ---
    //
    // Before the relayout below so a reply spliced this tick is
    // typeset this tick (`DocUi::poll_voice` marks the model dirty
    // itself). The harness runs on a worker thread — a turn is
    // seconds, a dictation is a recording — and this is the only point
    // that collects its outcome. Events are forwarded to the control
    // client: success as `voice_result`, failure as `error`, because
    // the protocol has no responses, only notifications.
    for event in state.doc.poll_voice() {
        let msg = match event.outcome {
            Ok(turn) => crate::ipc::OutgoingMessage::VoiceResult {
                block: event.block_index,
                utterance: turn.utterance,
                reply: turn.reply,
            },
            Err(message) => crate::ipc::OutgoingMessage::Error {
                code: crate::ipc::IpcErrorCode::VoiceFailed,
                message: format!("voice: {message}"),
            },
        };
        state.ipc.send(msg);
    }

    // --- Re-layout a document an edit left dirty ---
    //
    // Keystrokes mark the model dirty and return; this is where the typeset
    // actually happens (`input::edit_document_key` for why it is not done per
    // keystroke). It runs first because everything below reads layout products:
    // app binding takes figure rects for its geometry, IPC `list_state` reports
    // them, and the autosave should snapshot a document whose on-screen layout
    // matches its text. `relayout` is a no-op when the model is clean, which is
    // every tick that had no edit.
    //
    // `needs_redraw` is re-armed when a relayout really ran: a `Redraw` can be
    // delivered in the same winit dispatch batch as the keystroke that dirtied
    // the model, so the frame may already have been drawn against the old
    // layout — and winit's Input arm sets the flag *before* this runs, so
    // nothing else would ask for another one. Without this the final raster of
    // a burst can stay one typeset behind until the next unrelated input.
    let dirty = state.doc.model().is_dirty();
    state.relayout_document();
    state.poll_document_layout();
    if dirty {
        state.needs_redraw = true;
    }

    // --- Reap exited apps ---
    // Unlike the old Emacs shell, an app exiting does **not** stop the
    // compositor: its figure stays in the document as a dormant
    // placeholder that the user can relaunch. Only the child-process
    // bookkeeping needs clearing.
    state.host.reap_children();

    // --- Embedded toplevels: classify dialog vs app, then route ---
    // Same one-tick defer rationale (parent + min/max are only accurate
    // after the client's initial dispatch burst completes).
    process_pending_app_toplevels(state);

    // --- ext-workspace-v1 client actions → page switches ---
    crate::state::page::process_page_actions(state);

    // --- Document: autosave the session ---
    state.doc.tick();

    // --- ext-workspace-v1: refresh the page list for the bar ---
    crate::state::page::refresh_page_state(state);

    // --- Clean up destroyed embedded app windows ---
    crate::handlers::apps::cleanup_dead_apps(state);
    // --- Clean up destroyed floating dialogs ---
    crate::handlers::dialogs::cleanup_dead_dialogs(state);

    // --- Forward local document changes to collaboration peers ---
    //
    // The committed Loro version vector is the trigger, so this covers
    // keyboard edits, mark-only edits, IPC-driven edits, and splices from the
    // voice/formals workers without every edit site remembering the session.
    // A remote import sets `collab_version` to the post-import vector, so it
    // is not echoed back as a local edit on the next tick.
    match state.collab_local_edit() {
        Ok(outbound) => {
            for out in outbound {
                state.ipc.send(crate::ipc::OutgoingMessage::CollabDelta {
                    peer: out.peer,
                    seq: out.seq,
                    delta: crate::ipc::hex::encode(&out.delta),
                    scope: out.scope,
                });
            }
        }
        Err(e) => tracing::warn!("collaboration local edit: {e}"),
    }

    // --- Presence heartbeat and expiry ---
    //
    // Presence publishes on a one-second heartbeat, on a caret/focus change,
    // and immediately after join. It is advisory state: this never edits the
    // document, and expired peers are pruned here.
    state.collab_prune_presence();
    state.collab_publish_presence(false);
    // A share with no recent acknowledgement is offline, not merely behind.
    state.doc.refresh_share_timeout();

    // --- Dispatch incoming IPC messages from the control client ---
    if let Some(msgs) = state.ipc.recv_all() {
        for msg in msgs {
            crate::ipc::dispatch::handle_ipc_message(state, msg);
        }
        state.needs_redraw = true;
    }

    // --- Typed committed changes (K1) ---
    // One notification per grouped content edit, drained once per tick after
    // every edit path (keyboard, voice/formals splice, IPC, peer delta) has
    // run. Presentation-only activity produces nothing here, so a client that
    // applies these summaries can keep its derived version in step with the
    // durable document without ever advancing it for a pan or a status tick.
    let changes = state.doc.take_changes();
    if !changes.is_empty() {
        // Retain a read-only snapshot of each committed generation with its
        // explicit share and owner (K5). The live model and its undo stack are
        // untouched.
        let share_id = *state.collab.share_id();
        let owner = state.collab.local_peer().to_string();
        state.doc.retain_history(share_id, &owner);
    }
    for change in changes {
        state.ipc.send(crate::ipc::OutgoingMessage::DocChanged {
            seq: change.seq,
            document: change.document,
            content_generation: change.content_generation,
            version: change.version,
            origin: change.origin.as_str().to_owned(),
            scope: change.scope.as_str().to_owned(),
            ranges: change.ranges.iter().map(|r| [r.start, r.end]).collect(),
            source_digest: change.source_digest,
            content_digest: change.content_digest,
        });
    }

    state.ipc.flush();

    // --- Process clipboard events from host compositor ---
    // `OwnedFd` backends (data-control, X11) are driven by their calloop
    // fd source in `main::register_clipboard_source`, so we just drain
    // events here. `Piggyback` backends (wl_data_device on winit's
    // shared connection) have no owned fd — this tick is the only point
    // at which we can collect buffered events from libwayland's per-queue
    // ring before draining them.
    let clipboard_events = state
        .selection
        .clipboard
        .as_mut()
        .map(|c| {
            if matches!(c.driver(), emthin_clipboard::Driver::Piggyback) {
                c.dispatch();
            }
            c.take_events()
        })
        .unwrap_or_default();
    let has_clipboard_events = !clipboard_events.is_empty();
    for event in clipboard_events {
        crate::clipboard_bridge::handle_clipboard_event(state, event);
    }
    // Flush immediately so Wayland clients see selection changes / send
    // requests without waiting for the next render frame.
    if has_clipboard_events {
        let _ = state.display_handle.flush_clients();
        state.needs_redraw = true;
    }

    // --- Force-commit pending geometries that have timed out (100ms) ---
    let timed_out = state
        .apps
        .collect_timed_out(std::time::Duration::from_millis(100));
    if !timed_out.is_empty() {
        state.needs_redraw = true;
    }
    for (window_id, window, geo) in timed_out {
        state.page.active_space.map_element(window, geo.loc, false);
        tracing::debug!("figure app window_id={window_id} geometry force-committed (timeout)");
    }

    // Drain broker-observed fcitx5 events and drive winit IME in
    // emthin-winit-local coords.
    crate::state::ime::drain_fcitx_events(state);

    // Poll text_input_v3 cursor freshness — picks up the focused
    // client's `set_cursor_rectangle` request the moment it lands,
    // without waiting for a host IME event. Fixes "popup at fallback
    // (surface origin) until first preedit" for tip-only clients
    // (Alacritty etc.) whose cursor_rectangle arrives async after
    // focus.
    state.ime.poll_tip_freshness(&state.seat, &state.apps);

    // --- Forward DBus router notifications to the control client ---
    forward_router_notifications(state);

    let elapsed = started.elapsed();
    let elapsed_us = elapsed.as_micros() as u64;
    tracing::trace!(
        target: "emthin::timing",
        stage = "event_loop_tick",
        elapsed_us,
        "event-loop tick"
    );
    if elapsed >= std::time::Duration::from_millis(1) {
        tracing::debug!(
            target: "emthin::timing",
            stage = "event_loop_tick",
            elapsed_us,
            "slow event-loop tick"
        );
    }
}

/// Forward non-fcitx router notifications (rule add/remove/list) to the
/// control client over IPC.
fn forward_router_notifications(state: &mut EmthinState) {
    let notifications = state.dbus.take_non_fcitx_notifications();
    for n in notifications {
        let msg = match n {
            emthin_dbus::router::BridgeNotification::RuleAdded { id, rule } => {
                crate::ipc::OutgoingMessage::DbusRouterRuleAdded { id, rule }
            }
            emthin_dbus::router::BridgeNotification::RuleRemoved { id } => {
                crate::ipc::OutgoingMessage::DbusRouterRuleRemoved { id }
            }
            emthin_dbus::router::BridgeNotification::RuleList { rules } => {
                crate::ipc::OutgoingMessage::DbusRouterRules { rules }
            }
            _ => continue,
        };
        state.ipc.send(msg);
    }
}

/// Drain `pending_app_toplevels`, classify each as either a floating
/// dialog (login boxes, file pickers, …) or an embedded app, and route
/// it accordingly. Same one-tick-defer rationale as
/// `new_toplevel`: by now the client's
/// `set_parent` / `set_min_size` / `set_max_size` from the same dispatch
/// burst have been processed, so `wants_floating` returns a stable
/// answer.
fn process_pending_app_toplevels(state: &mut EmthinState) {
    let pending = std::mem::take(&mut state.page.pending_app_toplevels);
    if pending.is_empty() {
        return;
    }
    state.needs_redraw = true;
    for (surface, window) in pending {
        if !window.alive() {
            continue;
        }
        let floating = crate::handlers::xdg_shell::wants_floating(state, &surface);
        // Log the full set of inputs the heuristic actually sees, so a
        // misclassified login window (wechat / feishu / file pickers)
        // is debuggable from the journal without rebuilding emthin.
        let log = inspect_toplevel_for_log(&surface);
        tracing::info!(
            target: "emthin::dialog",
            floating,
            has_parent = surface.parent().is_some(),
            title = ?log.title,
            app_id = ?log.app_id,
            min_w = log.min.w,
            min_h = log.min.h,
            max_w = log.max.w,
            max_h = log.max.h,
            "embedded toplevel classification",
        );
        if floating {
            crate::handlers::dialogs::promote_floating_dialog(state, surface, window);
        } else {
            crate::handlers::apps::register_embedded_app(state, surface, window);
        }
    }
}

/// Inputs to `wants_floating`, captured for diagnostic logging.
struct ToplevelLogInputs {
    title: Option<String>,
    app_id: Option<String>,
    min: smithay::utils::Size<i32, smithay::utils::Logical>,
    max: smithay::utils::Size<i32, smithay::utils::Logical>,
}

/// Read the inputs that `wants_floating` consults, for diagnostic
/// logging. Pure side-effect-free read of the toplevel's cached state.
fn inspect_toplevel_for_log(
    surface: &smithay::wayland::shell::xdg::ToplevelSurface,
) -> ToplevelLogInputs {
    use smithay::wayland::compositor::with_states;
    use smithay::wayland::shell::xdg::{SurfaceCachedState, XdgToplevelSurfaceData};

    let (title, app_id) = with_states(surface.wl_surface(), |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|d| d.lock().ok())
            .map(|d| (d.title.clone(), d.app_id.clone()))
            .unwrap_or((None, None))
    });
    let (min, max) = with_states(surface.wl_surface(), |states| {
        let mut cached = states.cached_state.get::<SurfaceCachedState>();
        let current = cached.current();
        (current.min_size, current.max_size)
    });
    ToplevelLogInputs {
        title,
        app_id,
        min,
        max,
    }
}
