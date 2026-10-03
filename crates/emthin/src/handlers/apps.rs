//! Binding a client's toplevel to a document figure.
//!
//! This is the whole of the old "window management" story: a toplevel
//! arrives, it gets a figure (or one is written into the document for
//! it), and it's configured to that figure's size. There is no
//! placement policy, because the document already decided where it
//! goes.

use smithay::{
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    utils::{Logical, Rectangle, Size},
    wayland::{
        compositor::with_states,
        shell::xdg::{ToplevelSurface, XdgToplevelSurfaceData},
    },
};

use crate::ipc::OutgoingMessage;
use crate::EmthinState;

/// How an unbound toplevel is matched to a free figure.
fn matches_id(figure_id: Option<&str>, app_id: &str, title: &str) -> bool {
    let Some(pattern) = figure_id else {
        return false;
    };
    // A figure's binding id is a glob, driftwm-window-rules style, so
    // one `\app(..., "foot*")` can claim both `foot` and `footclient`.
    let matches = |candidate: &str| {
        let ok = glob_match(pattern, candidate);
        tracing::trace!("glob {pattern:?} vs {candidate:?} → {ok}");
        ok
    };
    matches(app_id) || (!title.is_empty() && matches(title))
}

/// Minimal `*`-only glob: `*` matches any run of characters.
/// Deliberately not full glob syntax — this is a window-rule matcher,
/// not a path matcher, and a full engine would be a dependency with a
/// config syntax nobody asked for.
fn glob_match(pattern: &str, candidate: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let c: Vec<char> = candidate.chars().collect();
    let (mut pi, mut ci) = (0usize, 0usize);
    // The position of the most recent `*`, so a mismatch can fall back
    // to "let this star swallow one more character".
    let mut star: Option<(usize, usize)> = None;
    while ci < c.len() {
        match p.get(pi) {
            Some('*') => {
                star = Some((pi, ci));
                pi += 1;
            }
            Some(&ch) if ch.eq_ignore_ascii_case(&c[ci]) => {
                pi += 1;
                ci += 1;
            }
            _ => match star {
                Some((sp, sc)) => {
                    pi = sp + 1;
                    ci = sc + 1;
                    star = Some((sp, ci));
                }
                None => return false,
            },
        }
    }
    while p.get(pi) == Some(&'*') {
        pi += 1;
    }
    pi == p.len()
}

/// Register a toplevel as a figure-bound app.
///
/// Binding rules, in order:
/// 1. an **unbound** figure whose `\app` binding id matches the
///    client's `app_id` or title (glob);
/// 2. else the first unbound figure in document order;
/// 3. else **append** a figure to the document for this app.
///
/// The toplevel is then configured to the figure's `w × h` with all
/// four `Tiled*` states — terminals (foot, alacritty) only hit an exact
/// pixel size when every tiled edge is set, otherwise they pad to a
/// cell boundary and the figure is a few pixels too small.
pub fn register_embedded_app(
    state: &mut EmthinState,
    surface: ToplevelSurface,
    window: smithay::desktop::Window,
) {
    let (title, app_id) = with_states(surface.wl_surface(), |s| {
        s.data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|d| d.lock().ok())
            .map(|d| {
                (
                    d.title.clone().unwrap_or_default(),
                    d.app_id.clone().unwrap_or_default(),
                )
            })
            .unwrap_or_default()
    });

    let window_id = state.apps.alloc_id();
    let page = state.doc.current_page();
    tracing::info!(
        "figure app toplevel connected: window_id={window_id} title={title:?} app_id={app_id:?}"
    );

    state.apps.insert(crate::apps::AppWindow {
        window_id,
        window: window.clone(),
        page,
        geometry: None,
        pending_geometry: None,
        pending_since: None,
        visible: true,
        mirrors: std::collections::HashMap::new(),
    });

    let figure_key = claim_figure(state, window_id, &app_id, &title);
    let Some(figure_key) = figure_key else {
        tracing::warn!("could not bind window_id={window_id} to a figure");
        return;
    };
    let spec = state
        .doc
        .figures()
        .get(&figure_key)
        .map(|f| f.spec.clone())
        .unwrap_or_else(|| {
            let (w, h) = crate::docui::edit::default_figure_size();
            crate::docui::edit::spec_of(w, h)
        });

    configure_to_figure(&surface, spec.w, spec.h);
    // The source box `figure_render` composites against. Only recorded once the
    // client is bound, because that is when there is a figure rect to take a
    // location from.
    note_figure_geometry(state, window_id, &figure_key, spec.w, spec.h);

    state.ipc.send(OutgoingMessage::FigureBound {
        figure: figure_key.clone(),
        window_id,
        title: title.clone(),
    });
    tracing::info!(
        "window_id={window_id} bound to figure {figure_key} ({}x{})",
        spec.w,
        spec.h
    );

    state.auto_focus_new_window(window, window_id);
}

/// Pick the figure this toplevel belongs to, appending one if every
/// figure is taken. Returns the figure key.
fn claim_figure(
    state: &mut EmthinState,
    window_id: u64,
    app_id: &str,
    title: &str,
) -> Option<String> {
    // 1. an unbound figure whose binding id matches.
    let by_id = state
        .doc
        .figures()
        .figures()
        .iter()
        .find(|f| f.is_dormant() && matches_id(f.spec.id.as_deref(), app_id, title))
        .map(|f| f.key.clone());
    if let Some(key) = by_id {
        state.doc.figures_mut().bind(&key, window_id);
        return Some(key);
    }

    // 2. the first unbound figure in document order.
    if let Some(key) = state
        .doc
        .figures()
        .figures()
        .iter()
        .find(|f| f.is_dormant())
        .map(|f| f.key.clone())
    {
        state.doc.figures_mut().bind(&key, window_id);
        return Some(key);
    }

    // 3. every figure is bound — write a new one into the document.
    let caption = if !title.is_empty() {
        title
    } else if !app_id.is_empty() {
        app_id
    } else {
        "app"
    };
    let (w, h) = crate::docui::edit::default_figure_size();
    let id = (!app_id.is_empty()).then_some(app_id);
    crate::docui::edit::append_figure(state.doc.model_mut(), caption, w, h, id);
    state.relayout_document();
    // The appended statement is last in document order, so its key is
    // the highest `f<stmt-index>`.
    let key = state
        .doc
        .figures()
        .figures()
        .last()
        .map(|f| f.key.clone())?;
    state.doc.figures_mut().bind(&key, window_id);
    tracing::info!("no free figure; appended {key} to the document");
    Some(key)
}

/// Configure a toplevel to exactly `w × h`.
pub fn configure_to_figure(surface: &ToplevelSurface, w: i32, h: i32) {
    if w <= 0 || h <= 0 {
        return;
    }
    surface.with_pending_state(|s| {
        s.size = Some(Size::<i32, Logical>::from((w, h)));
        s.states.set(xdg_toplevel::State::TiledLeft);
        s.states.set(xdg_toplevel::State::TiledRight);
        s.states.set(xdg_toplevel::State::TiledTop);
        s.states.set(xdg_toplevel::State::TiledBottom);
    });
    surface.send_pending_configure();
}

/// Record `app_id`'s pending geometry from the figure it is bound to.
///
/// The location is the figure's placed rect, because that is where the element is
/// mapped so the app keeps receiving frame callbacks; the size is the figure's
/// *declared* size, because that is what the client is being configured into and
/// therefore what `figure_render` must measure the client's buffer against.
fn note_figure_geometry(state: &mut EmthinState, app_id: u64, figure_key: &str, w: i32, h: i32) {
    let loc = state
        .doc
        .figures()
        .get(figure_key)
        .map(|f| f.rect.loc)
        .unwrap_or_default();
    state
        .apps
        .set_pending_geometry(app_id, Rectangle::new(loc, Size::from((w, h))));
}

/// Reconfigure the app bound to `figure_stable_id` to the figure's current
/// size, and report the new geometry to the control plane.
///
/// Used by the figure resize grab after it has rewritten the `\app`
/// arguments and re-laid out the document.
///
/// Takes the statement's marker pair rather than its layout key: a grab spans a
/// whole pointer drag, and an IPC append or a newly bound app can insert an
/// `\app` in that window, renumbering every figure below it. Keying off the
/// layout index meant a release could rewrite a different statement's `\app`
/// arguments than the one the user dragged.
pub fn reconfigure_after_resize(state: &mut EmthinState, figure_stable_id: &str) {
    let Some(figure) = state.doc.figures().by_stable_id(figure_stable_id).cloned() else {
        return;
    };
    let figure_key = figure.key.clone();
    let (w, h) = (figure.spec.w, figure.spec.h);
    let page = figure.page.unwrap_or(0);
    let rect = figure.rect;

    if let Some(app_id) = figure.app_id {
        if let Some(app) = state.apps.get(app_id) {
            if let Some(toplevel) = app.window.toplevel() {
                configure_to_figure(toplevel, w, h);
            }
        }
        note_figure_geometry(state, app_id, &figure_key, w, h);
    }
    // Keep the app mapped at the figure's rect so it keeps receiving
    // frame callbacks; `figure_render` does the actual compositing.
    let want_map = rect.size.w > 0 && rect.size.h > 0;
    if let (true, Some(app_id)) = (want_map, figure.app_id) {
        if let Some(app) = state.apps.get(app_id) {
            if app.geometry.is_none() {
                let win = app.window.clone();
                state.page.active_space.map_element(win, rect.loc, false);
            }
        }
    }

    state.ipc.send(crate::ipc::OutgoingMessage::FigureChanged {
        figure: figure_key,
        page,
        // The *placed* rect, both origin and size. `rect.loc` is letterboxed but
        // `w`/`h` were the declared size, so after `d8f889c` this reported a 640
        // wide figure as `w=640` here and `w=608` from `list_state` — and
        // `docs/ipc.md` says every rect is "what actually got placed".
        rect: crate::ipc::IpcRect {
            x: rect.loc.x,
            y: rect.loc.y,
            w: rect.size.w,
            h: rect.size.h,
        },
        bound: figure.app_id.is_some(),
    });
    state.needs_redraw = true;
}

/// The figure rect an app currently occupies, in output-local logical px.
///
/// The input path's answer to "where does this client draw?" — the
/// pointer hit-test uses it to map a click inside a figure into
/// surface-local coordinates.
pub fn app_figure_rect(state: &EmthinState, window_id: u64) -> Option<Rectangle<i32, Logical>> {
    state.doc.figures().figure_of_app(window_id).map(|f| f.rect)
}

/// Unmap and forget apps whose Wayland surface died. Their figures stay
/// in the document and go dormant.
pub fn cleanup_dead_apps(state: &mut EmthinState) {
    let dead = state.apps.drain_dead();
    if dead.is_empty() {
        return;
    }
    state.needs_redraw = true;
    for app in &dead {
        state.page.active_space.unmap_elem(&app.window);
        // The figure keeps its `\app` statement; only the binding goes,
        // so it renders as a dormant stand-in the user can relaunch.
        let released = state.doc.figures_mut().release_app(app.window_id);
        for figure in released {
            state.ipc.send(OutgoingMessage::FigureUnbound {
                figure,
                window_id: app.window_id,
            });
        }
        tracing::info!(
            "app window_id={} destroyed; its figure went dormant",
            app.window_id
        );
    }
    // Focus falls back to the document (no surface) when the focused app
    // died with nothing else to focus.
    //
    // The test is whether the *focused* window is the dead one. It used to be
    // `current_focus().is_none()`, which is the negation of that: by the time this
    // runs the dead window has already been unmapped and dropped from
    // `state.apps`, so it could never be found again and focus was left pointing
    // at it. Keystrokes then went to a dead surface — `input.rs` takes the
    // "a client has focus" branch — so the document caret was dead until the user
    // clicked, and the `focus.last_app_focus` wake toggle had nothing to restore.
    //
    // The equivalent check in `dialogs.rs` (`Some(Window(w)) => !w.alive()`) is
    // unreachable for apps: `cleanup_dead_apps` runs first and unmaps the
    // element, so `cleanup_dead_dialogs` never sees it.
    if let Some(keyboard) = state.seat.get_keyboard() {
        use smithay::wayland::seat::WaylandFocus;
        let focus_dead = keyboard.current_focus().is_some_and(|w| {
            let Some(xdg) = w.wl_surface().map(std::borrow::Cow::into_owned) else {
                // A layer-shell or popup focus is not one of ours to reclaim.
                return false;
            };
            state.apps.id_for_surface(&xdg).is_none()
        });
        if focus_dead {
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            keyboard.set_focus(state, None, serial);
            tracing::debug!("focus returned to the document after window destroy");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches_exact_names() {
        assert!(glob_match("foot", "foot"));
        assert!(!glob_match("foot", "footclient"));
        assert!(!glob_match("foot", "FOO"));
    }

    #[test]
    fn glob_is_case_insensitive() {
        assert!(glob_match("Foot", "foot"));
    }

    #[test]
    fn star_matches_any_run() {
        assert!(glob_match("foot*", "foot"));
        assert!(glob_match("foot*", "footclient"));
        assert!(glob_match("*foot", "libfoot"));
        assert!(glob_match("*foot*", "libfoot.so"));
        assert!(glob_match("f*t", "foot"));
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn glob_rejects_non_matches() {
        assert!(!glob_match("foot*", "alacritty"));
        assert!(!glob_match("*foot", "alacritty"));
        // "f*t" *does* match "faot" (f + "ao" + t) — the star is
        // greedy about what it eats, not about where the tail lands.
        assert!(glob_match("f*t", "faot"));
    }
    #[test]
    fn an_id_matches_app_id_or_title() {
        assert!(matches_id(Some("foot"), "foot", ""), "app_id matches");
        assert!(
            matches_id(Some("term*"), "", "terminal"),
            "title glob matches"
        );
        assert!(
            matches_id(Some("term*"), "other", "terminal"),
            "title glob matches too"
        );
        assert!(!matches_id(Some("foot"), "", "terminal"), "neither matches");
        assert!(
            !matches_id(Some("term"), "", "terminal"),
            "an exact id is not a prefix glob"
        );
    }

    #[test]
    fn an_idless_figure_never_claims_by_glob() {
        // Rule 2 ("first unbound figure in document order") handles
        // id-less figures; a glob rule must not steal them.
        assert!(!matches_id(None, "foot", "foot"));
    }

    #[test]
    fn an_empty_title_is_not_matched_against() {
        // A client that has set no title yet shouldn't be claimable by
        // a title glob — otherwise every anonymous toplevel would race
        // for the same figure.
        assert!(!matches_id(Some("foot"), "", ""));
    }
}
