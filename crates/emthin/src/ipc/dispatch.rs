//! Control-message handlers.
//!
//! Every handler is a thin shim: validate, then call a `docui` method.
//! The document owns all policy — there is no layout arithmetic in this
//! file, which is exactly the inversion the rewrite was for.

use crate::ipc::{IncomingMessage, IpcErrorCode, IpcRect, OutgoingMessage, StateFigure};
use crate::EmthinState;

pub fn handle_ipc_message(state: &mut EmthinState, msg: IncomingMessage) {
    match msg {
        IncomingMessage::Spawn { cmd, args } => ipc_spawn(state, &cmd, &args),
        IncomingMessage::Close { figure } => ipc_close(state, &figure),
        IncomingMessage::Focus { figure } => ipc_focus(state, figure.as_deref()),
        IncomingMessage::SetFigureSize { figure, w, h } => {
            ipc_set_figure_size(state, &figure, w, h)
        }
        IncomingMessage::CloneFigure { figure } => ipc_clone_figure(state, &figure),
        IncomingMessage::GotoPage { page } => {
            tracing::debug!("IPC goto_page {page}");
            state.goto_page(page);
        }
        IncomingMessage::OpenDoc { path } => ipc_open_doc(state, &path),
        IncomingMessage::SaveDoc { path } => ipc_save_doc(state, &path),
        IncomingMessage::ExportAscii { path } => ipc_export_ascii(state, &path),
        IncomingMessage::ImportAscii { path } => ipc_import_ascii(state, &path),
        IncomingMessage::SetTextSize { pt } => ipc_set_text_size(state, pt),
        IncomingMessage::InjectTouch { phase, slot, x, y } => state.inject_touch(phase, slot, x, y),
        IncomingMessage::ListState => ipc_list_state(state),
        IncomingMessage::Voice { text, block } => ipc_voice(state, block, text.as_deref()),
        IncomingMessage::DbusRouterAddRule { rule } => {
            tracing::debug!("IPC dbus_router_add_rule id={}", rule.id);
            state
                .dbus
                .send_rpc(emthin_dbus::router::BridgeCommand::AddRule(rule));
        }
        IncomingMessage::DbusRouterRemoveRule { id } => {
            tracing::debug!("IPC dbus_router_remove_rule id={id}");
            state
                .dbus
                .send_rpc(emthin_dbus::router::BridgeCommand::RemoveRule(id));
        }
        IncomingMessage::DbusRouterListRules => {
            tracing::debug!("IPC dbus_router_list_rules");
            state
                .dbus
                .send_rpc(emthin_dbus::router::BridgeCommand::ListRules);
        }
        IncomingMessage::CollabVersion => ipc_collab_version(state),
        IncomingMessage::CollabJoin { peer, version } => ipc_collab_join(state, &peer, &version),
        IncomingMessage::CollabDelta { peer, seq, delta } => {
            ipc_collab_delta(state, &peer, seq, &delta)
        }
        IncomingMessage::CollabAck { peer, seq } => ipc_collab_ack(state, &peer, seq),
        IncomingMessage::CollabLeave { peer } => {
            tracing::debug!("IPC collab_leave {peer}");
            state.collab.leave(&peer);
        }
    }
}

/// Emit an `error` notification: a closed-set `code` for clients to
/// branch on, the human `message` for the transcript and logs. The code's
/// category/retry/remediation semantics live in `docs/error-contract.md`.
fn error(state: &mut EmthinState, code: IpcErrorCode, message: impl Into<String>) {
    let message = message.into();
    tracing::warn!("IPC error [{code:?}]: {message}");
    state.ipc.send(OutgoingMessage::Error { code, message });
}

fn ipc_collab_version(state: &mut EmthinState) {
    let version = crate::ipc::hex::encode(&state.doc.model().math_doc().version_vector());
    tracing::debug!("IPC collab_version ({} hex chars)", version.len());
    state.ipc.send(OutgoingMessage::CollabVersion { version });
}

fn ipc_collab_join(state: &mut EmthinState, peer: &str, version: &str) {
    let version = match crate::ipc::hex::decode(version) {
        Ok(v) => v,
        Err(e) => return error(state, IpcErrorCode::CollabFailed, e),
    };
    let result = if state.collab.has_peer(peer) {
        state
            .collab
            .reconnect(state.doc.model().math_doc(), peer, &version)
    } else {
        state
            .collab
            .join(state.doc.model().math_doc(), peer, &version)
    };
    let out = match result {
        Ok(out) => out,
        Err(e) => return error(state, IpcErrorCode::CollabFailed, e.to_string()),
    };
    tracing::debug!("IPC collab_join {peer}");
    state.ipc.send(OutgoingMessage::CollabDelta {
        peer: out.peer,
        seq: out.seq,
        delta: crate::ipc::hex::encode(&out.delta),
    });
}

fn ipc_collab_delta(state: &mut EmthinState, peer: &str, seq: u64, delta: &str) {
    let delta = match crate::ipc::hex::decode(delta) {
        Ok(delta) => delta,
        Err(e) => return error(state, IpcErrorCode::CollabFailed, e),
    };
    let outcome = {
        let doc = state.doc.model_mut().math_doc_mut();
        state.collab.receive(doc, peer, seq, &delta)
    };
    // `math_doc_mut` dirtied the model; rebuild the scan even when the
    // import failed after applying some of the buffered commands.
    state.doc.model_mut().rescan();
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(e) => return error(state, IpcErrorCode::CollabFailed, e.to_string()),
    };
    let outbound = match outcome {
        emthin_collab::ReceiveOutcome::Applied { outbound, .. } => outbound,
        emthin_collab::ReceiveOutcome::Duplicate => return,
        emthin_collab::ReceiveOutcome::Gap { expected } => {
            tracing::warn!(
                "collab delta from {peer} arrived at seq {seq} with a gap; expected {expected}"
            );
            return;
        }
    };
    tracing::debug!(
        "IPC collab_delta {peer} seq={seq} ({} forwarded)",
        outbound.len()
    );
    state.collab_after_remote();
    for out in outbound {
        state.ipc.send(OutgoingMessage::CollabDelta {
            peer: out.peer,
            seq: out.seq,
            delta: crate::ipc::hex::encode(&out.delta),
        });
    }
}

fn ipc_collab_ack(state: &mut EmthinState, peer: &str, seq: u64) {
    if let Err(e) = state.collab.ack(peer, seq) {
        return error(state, IpcErrorCode::CollabFailed, e.to_string());
    }
    tracing::debug!("IPC collab_ack {peer} through {seq}");
}

fn ipc_spawn(state: &mut EmthinState, cmd: &str, args: &[String]) {
    tracing::debug!("IPC spawn {cmd} {args:?}");
    let display = state.xwayland.display();
    crate::util::spawn_child(cmd, args, display, state);
}

fn ipc_close(state: &mut EmthinState, figure: &str) {
    let Some(app_id) = state.doc.figures().get(figure).and_then(|f| f.app_id) else {
        return error(
            state,
            IpcErrorCode::NotBound,
            format!("no app bound to figure {figure}"),
        );
    };
    tracing::debug!("IPC close figure={figure} app={app_id}");
    if let Some(app) = state.apps.get(app_id) {
        if let Some(toplevel) = app.window.toplevel() {
            toplevel.send_close();
        }
    }
}

fn ipc_focus(state: &mut EmthinState, figure: Option<&str>) {
    let Some(keyboard) = state.seat.get_keyboard() else {
        return;
    };
    let target = match figure {
        // `None` = the document itself, which owns no Wayland surface, so
        // the seat has nothing to focus. Clearing is what puts typing
        // back on the caret.
        None => None,
        Some(key) => {
            let Some(app_id) = state.doc.figures().get(key).and_then(|f| f.app_id) else {
                return error(
                    state,
                    IpcErrorCode::NotBound,
                    format!("no app bound to figure {key}"),
                );
            };
            match state.apps.get(app_id) {
                Some(app) => Some(crate::KeyboardFocusTarget::from(app.window.clone())),
                None => {
                    return error(
                        state,
                        IpcErrorCode::AppGone,
                        format!("figure {key}'s app is gone"),
                    )
                }
            }
        }
    };
    tracing::debug!("IPC focus figure={figure:?}");
    let serial = smithay::utils::SERIAL_COUNTER.next_serial();
    keyboard.set_focus(state, target, serial);
    state.needs_redraw = true;
}

fn ipc_set_figure_size(state: &mut EmthinState, figure: &str, w: i32, h: i32) {
    let Some(fig) = state.doc.figures().get(figure).cloned() else {
        return error(
            state,
            IpcErrorCode::UnknownFigure,
            format!("unknown figure {figure}"),
        );
    };
    tracing::debug!("IPC set_figure_size figure={figure} {w}x{h}");
    if crate::docui::edit::set_figure_size(state.doc.model_mut(), &fig, w, h) {
        // Reflow reconfigures the bound app through the normal
        // relayout path.
        state.relayout_document();
        state.needs_redraw = true;
    }
}

fn ipc_clone_figure(state: &mut EmthinState, figure: &str) {
    let Some(fig) = state.doc.figures().get(figure).cloned() else {
        return error(
            state,
            IpcErrorCode::UnknownFigure,
            format!("unknown figure {figure}"),
        );
    };
    tracing::debug!("IPC clone_figure figure={figure}");
    crate::docui::edit::clone_figure(state.doc.model_mut(), &fig);
    state.relayout_document();
    state.needs_redraw = true;
}

/// Queue a voice turn. The harness runs on a worker thread, so this
/// returns immediately: the outcome arrives as a `voice_result`
/// notification (or `error`) from the tick that polls the bridge.
fn ipc_voice(state: &mut EmthinState, block: Option<usize>, text: Option<&str>) {
    tracing::debug!("IPC voice block={block:?} text={text:?}");
    match state.doc.voice_submit(block, text) {
        Ok(submit) => tracing::info!(
            "IPC voice: turn queued for block #{} (append_utterance={}, say={})",
            submit.block_index,
            submit.append_utterance,
            submit.say
        ),
        Err(reason) => error(state, IpcErrorCode::VoiceFailed, format!("voice: {reason}")),
    }
}

fn ipc_open_doc(state: &mut EmthinState, path: &str) {
    let path = std::path::PathBuf::from(path);
    match state.doc.open_document(&path) {
        Ok(()) => {
            // The session's peer versions belong to the previous document.
            state.collab.clear_peers();
            state.collab_revision = state.doc.model().revision();
            state.relayout_document();
            state.needs_redraw = true;
            tracing::info!("opened document {}", path.display());
        }
        Err(reason) => error(state, IpcErrorCode::IoFailed, reason),
    }
}

fn ipc_save_doc(state: &mut EmthinState, path: &str) {
    let path = std::path::PathBuf::from(path);
    match std::fs::write(&path, state.doc.model().text()) {
        Ok(()) => {
            tracing::info!("saved document {}", path.display());
            state.ipc.send(OutgoingMessage::DocSaved {
                path: path.display().to_string(),
            });
        }
        Err(e) => error(
            state,
            IpcErrorCode::IoFailed,
            format!("could not write {}: {e}", path.display()),
        ),
    }
}

fn ipc_export_ascii(state: &mut EmthinState, path: &str) {
    let path = std::path::PathBuf::from(path);
    match std::fs::write(&path, state.doc.export_ascii()) {
        Ok(()) => {
            tracing::info!("exported ASCII projection {}", path.display());
            state.ipc.send(OutgoingMessage::AsciiExported {
                path: path.display().to_string(),
            });
        }
        Err(e) => error(
            state,
            IpcErrorCode::IoFailed,
            format!("could not write {}: {e}", path.display()),
        ),
    }
}

fn ipc_import_ascii(state: &mut EmthinState, path: &str) {
    let path = std::path::PathBuf::from(path);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) => {
            return error(
                state,
                IpcErrorCode::IoFailed,
                format!("could not read {}: {e}", path.display()),
            );
        }
    };
    match state.doc.import_ascii(&text) {
        Ok(()) => {
            state.relayout_document();
            state.needs_redraw = true;
            tracing::info!("imported ASCII projection {}", path.display());
        }
        Err(reason) => error(state, IpcErrorCode::NotAscii, reason),
    }
}

/// Set the reading size. Clamping is the document's policy; this shim
/// reports the size that actually took effect, even when it was unchanged.
fn ipc_set_text_size(state: &mut EmthinState, pt: f32) {
    let changed = state.doc.set_text_size(pt);
    if changed {
        state.relayout_document();
        state.needs_redraw = true;
    }
    let pt = state.doc.text_size();
    tracing::info!(
        "IPC set_text_size {pt}pt ({} pages, changed={changed})",
        state.doc.page_count()
    );
    state.ipc.send(OutgoingMessage::TextSizeChanged {
        pt,
        page_count: state.doc.page_count(),
    });
}

fn ipc_list_state(state: &mut EmthinState) {
    let doc = state.doc.model().text().to_string();
    let page = state.doc.current_page();
    let page_count = state.doc.page_count();
    let text_size = state.doc.text_size();
    let view_zoom = state.doc.view_zoom();
    let pan = state.doc.view_pan();
    let citation_style = state
        .doc
        .citation_popup()
        .style_override()
        .map(str::to_string);
    let citation_locale = state
        .doc
        .citation_popup()
        .locale_override()
        .map(str::to_string);
    let figures = state
        .doc
        .figures()
        .figures()
        .iter()
        .map(|f| StateFigure {
            key: f.key.clone(),
            id: f.spec.id.clone(),
            caption: state.doc.model().text()[f.span.clone()].trim().to_string(),
            // `docs/ipc.md`: "rect is zero-sized for a figure that isn't on the
            // visible page". It was not: an off-page figure keeps the `(0, 0)`
            // placeholder `page_rect_for` hands it, so a frontend drawing
            // `state.figures` stacked every off-page figure in the output's
            // top-left corner.
            rect: if f.page == Some(page) {
                IpcRect {
                    x: f.rect.loc.x,
                    y: f.rect.loc.y,
                    w: f.rect.size.w,
                    h: f.rect.size.h,
                }
            } else {
                IpcRect {
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 0,
                }
            },
            page: f.page.unwrap_or(0),
            window_id: f.app_id,
            title: f.title.clone(),
        })
        .collect();
    state.ipc.send(OutgoingMessage::State {
        page,
        page_count,
        text_size,
        view_zoom,
        view_pan_x: pan.x,
        view_pan_y: pan.y,
        citation_style,
        citation_locale,
        doc,
        figures,
    });
}
