//! Control-message handlers.
//!
//! Every handler is a thin shim: validate, then call a `docui` method.
//! The document owns all policy — there is no layout arithmetic in this
//! file, which is exactly the inversion the rewrite was for.

use crate::ipc::{IncomingMessage, IpcRect, OutgoingMessage, StateFigure};
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
        IncomingMessage::ListState => ipc_list_state(state),
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
    }
}

fn error(state: &mut EmthinState, message: impl Into<String>) {
    let message = message.into();
    tracing::warn!("IPC error: {message}");
    state.ipc.send(OutgoingMessage::Error { message });
}

fn ipc_spawn(state: &mut EmthinState, cmd: &str, args: &[String]) {
    tracing::debug!("IPC spawn {cmd} {args:?}");
    let display = state.xwayland.display();
    crate::util::spawn_child(cmd, args, display, state);
}

fn ipc_close(state: &mut EmthinState, figure: &str) {
    let Some(app_id) = state.doc.figures().get(figure).and_then(|f| f.app_id) else {
        return error(state, format!("no app bound to figure {figure}"));
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
                return error(state, format!("no app bound to figure {key}"));
            };
            match state.apps.get(app_id) {
                Some(app) => Some(crate::KeyboardFocusTarget::from(app.window.clone())),
                None => return error(state, format!("figure {key}'s app is gone")),
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
        return error(state, format!("unknown figure {figure}"));
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
        return error(state, format!("unknown figure {figure}"));
    };
    tracing::debug!("IPC clone_figure figure={figure}");
    crate::docui::edit::clone_figure(state.doc.model_mut(), &fig);
    state.relayout_document();
    state.needs_redraw = true;
}

fn ipc_open_doc(state: &mut EmthinState, path: &str) {
    let path = std::path::PathBuf::from(path);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            *state.doc.model_mut() = crate::docui::DocModel::new(&text);
            state.relayout_document();
            state.needs_redraw = true;
            tracing::info!("opened document {}", path.display());
        }
        Err(e) => error(state, format!("could not read {}: {e}", path.display())),
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
        Err(e) => error(state, format!("could not write {}: {e}", path.display())),
    }
}

fn ipc_list_state(state: &mut EmthinState) {
    let doc = state.doc.model().text().to_string();
    let page = state.doc.current_page();
    let page_count = state.doc.page_count();
    let figures = state
        .doc
        .figures()
        .figures()
        .iter()
        .map(|f| StateFigure {
            key: f.key.clone(),
            id: f.spec.id.clone(),
            caption: state.doc.model().text()[f.span.clone()].trim().to_string(),
            rect: IpcRect {
                x: f.rect.loc.x,
                y: f.rect.loc.y,
                w: f.rect.size.w,
                h: f.rect.size.h,
            },
            page: f.page.unwrap_or(0),
            window_id: f.app_id,
            title: f.title.clone(),
        })
        .collect();
    state.ipc.send(OutgoingMessage::State {
        page,
        page_count,
        doc,
        figures,
    });
}
