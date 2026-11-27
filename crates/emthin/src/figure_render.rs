//! Figure compositing: paint each bound app's live surface exactly over
//! its figure rect in the rendered page.
//!
//! This is the old `mirror_render` generalised. Both walk the same
//! machinery — one `TextureRenderElement` per mapped surface in a
//! layer's subsurface tree, placed by an accumulated logical offset and
//! namespaced per *view* id so the damage tracker doesn't collapse two
//! views of one surface. What changed is the destination: a mirror's
//! rect came from Emacs over IPC, a figure's rect comes from Typst
//! frame introspection.
//!
//! ## Why figures are *not* in the `Space`
//!
//! smithay's `render_output` draws custom elements **above** space
//! elements, and `Space::map_element` re-stacks to the top even with
//! `activate = false`. A figure is an image in a page: its rect is set
//! by the document, and nothing may cover it. So app toplevels stay
//! mapped in the `Space` purely to keep receiving configures and frame
//! callbacks (an unmapped toplevel never commits), while their pixels
//! are composited here, outside the space's z-order entirely.
//!
//! ## Gotchas this file must keep honouring
//!
//! - `TextureRenderElement` positions are **physical** px — convert with
//!   `output.current_scale().fractional_scale()`, never a hardcoded 1.0.
//! - Walk the **full** subsurface tree via `with_surface_tree_downward`;
//!   GTK/Firefox paint onto subsurface children, so reading only the
//!   toplevel yields nothing.
//! - `buffer_scale`, `buffer_transform`, and the viewport `src`/`dst`
//!   must come from `RendererSurfaceState`, or the surface is wrong
//!   under fractional scaling.
//! - Subtract `window.geometry().loc`: GTK/Chrome put CSD shadow
//!   padding in the buffer and mark the visible start with
//!   `xdg_surface.set_window_geometry`. `SurfaceLayer::render_offset`
//!   already cancels it (it matches `Space::render_location()`).
//! - Element `Id`s must be namespaced per figure — the same surface in
//!   two figures (a mirror) would otherwise collapse into one damage
//!   region and one of them would go blank.

use smithay::{
    backend::renderer::{
        element::{texture::TextureRenderElement, Id, Kind},
        gles::{GlesRenderer, GlesTexture},
        utils::{import_surface_tree, RendererSurfaceStateUserData},
        Color32F, Renderer,
    },
    utils::{Logical, Point, Rectangle, Size, Transform},
    wayland::compositor::{with_surface_tree_downward, TraversalAction},
};

use crate::element::CustomElement;
use crate::EmthinState;

/// Snapshot of one mapped surface within a layer's subsurface tree, in
/// the app's own logical coordinates. Collected **once per layer** so
/// each figure (and each mirror of it) only has to translate/scale,
/// which is cheap — re-walking a GTK app's subsurface tree per figure is
/// not.
struct SurfaceSnapshot {
    surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    offset: Point<f64, Logical>,
    view_src: Rectangle<f64, Logical>,
    view_dst: Size<i32, Logical>,
    texture: GlesTexture,
    buffer_scale: i32,
    buffer_transform: Transform,
}

/// Walk a layer's subsurface tree and collect one snapshot per mapped
/// surface that has a texture.
fn collect_layer_surfaces(
    renderer: &mut GlesRenderer,
    layer: &crate::apps::SurfaceLayer,
) -> Vec<SurfaceSnapshot> {
    let ctx = renderer.context_id();
    let mut out: Vec<SurfaceSnapshot> = Vec::new();
    let initial =
        Point::<f64, Logical>::from((layer.render_offset.x as f64, layer.render_offset.y as f64));
    with_surface_tree_downward(
        &layer.surface,
        initial,
        |_, states, loc| {
            let mut loc = *loc;
            if let Some(data) = states.data_map.get::<RendererSurfaceStateUserData>() {
                if let Some(view) = data.lock().unwrap().view() {
                    loc.x += view.offset.x as f64;
                    loc.y += view.offset.y as f64;
                    return TraversalAction::DoChildren(loc);
                }
            }
            TraversalAction::SkipChildren
        },
        |surface, states, loc| {
            let mut loc = *loc;
            let Some(data) = states.data_map.get::<RendererSurfaceStateUserData>() else {
                return;
            };
            let data = data.lock().unwrap();
            let Some(view) = data.view() else { return };
            loc.x += view.offset.x as f64;
            loc.y += view.offset.y as f64;
            let Some(texture) = data.texture::<GlesTexture>(ctx.clone()).cloned() else {
                return;
            };
            out.push(SurfaceSnapshot {
                surface: surface.clone(),
                offset: loc,
                view_src: view.src,
                view_dst: view.dst,
                texture,
                buffer_scale: data.buffer_scale(),
                buffer_transform: data.buffer_transform(),
            });
        },
        |_, _, _| true,
    );
    out
}

/// Build the [`CustomElement`]s for every bound figure on the visible
/// page, plus the caret/selection overlays.
///
/// Figure groups are front-to-back, with later figures above earlier ones.
/// Each Wayland surface tree retains Smithay's front-to-back traversal order.
pub fn build_figure_elements(
    state: &mut EmthinState,
    renderer: &mut GlesRenderer,
    scale: f64,
) -> Vec<CustomElement<GlesRenderer>> {
    let ctx = renderer.context_id();
    let mut elements = Vec::new();
    let current_page = state.doc.current_page();

    // Terminal figures have no Wayland surface. Their pixels come from a
    // desktop Core session source; collect first so the document and the
    // source registry are not borrowed mutably at the same time.
    let terminals: Vec<_> = state
        .doc
        .figures()
        .figures()
        .iter()
        .rev()
        .filter(|figure| figure.page == Some(current_page))
        .filter_map(|figure| {
            let reference = figure.spec.terminal.clone()?;
            (figure.rect.size.w > 0 && figure.rect.size.h > 0).then_some((reference, figure.rect))
        })
        .collect();
    for (reference, rect) in terminals {
        let handle = state.loop_handle.clone();
        state.core_sessions.ensure_live(&reference, rect, &handle);
        if let Some(element) = state
            .core_sessions
            .element(renderer, &reference, rect, scale)
        {
            elements.push(element);
        }
    }

    for figure in state.doc.figures().figures().iter().rev() {
        // Only figures on the visible page are composited.
        if figure.page != Some(current_page) {
            continue;
        }
        let Some(app_id) = figure.app_id else {
            continue;
        };
        let Some(src_geo) = state.apps.get(app_id).and_then(|a| a.geometry) else {
            continue;
        };
        let dst = figure.rect;
        if dst.size.w <= 0 || dst.size.h <= 0 {
            continue;
        }
        let src_size = src_geo.size.to_f64();
        // How much the figure's rect differs from the size the client
        // actually committed at. A figure stretches by default (true
        // "image in a PDF" semantics); `aspect_fit_ratio` only kicks in
        // when the client ignored our configure, in which case we
        // aspect-fit inside the figure instead of distorting it.
        let ratio = crate::apps::AppManager::aspect_fit_ratio(src_size, dst.size.to_f64())
            .unwrap_or_else(|| {
                let sx = f64::from(dst.size.w) / src_size.w;
                let sy = f64::from(dst.size.h) / src_size.h;
                if sx <= 0.0 || sy <= 0.0 {
                    1.0
                } else {
                    sx.min(sy)
                }
            });

        // Figures later in the document draw over earlier ones.
        for (layer_idx, layer) in state
            .apps
            .get(app_id)
            .expect("checked above")
            .surface_layers()
            .iter()
            .enumerate()
            .rev()
        {
            if let Err(e) = import_surface_tree(renderer, &layer.surface) {
                tracing::warn!(
                    "import_surface_tree failed for figure={} app={app_id} layer={layer_idx}: {e:?}",
                    figure.key
                );
                continue;
            }
            let snapshots = collect_layer_surfaces(renderer, layer);
            for snap in &snapshots {
                let loc = Point::<f64, Logical>::from((
                    f64::from(dst.loc.x) + snap.offset.x * ratio,
                    f64::from(dst.loc.y) + snap.offset.y * ratio,
                ));
                let fit_w = (snap.view_dst.w as f64 * ratio).round().max(1.0) as i32;
                let fit_h = (snap.view_dst.h as f64 * ratio).round().max(1.0) as i32;
                // Namespace by figure key so a mirror (same surface, two
                // figures) keeps distinct damage regions.
                let render_id =
                    Id::from_wayland_resource(&snap.surface).namespaced(namespace(&figure.key));
                elements.push(
                    TextureRenderElement::from_static_texture(
                        render_id,
                        ctx.clone(),
                        loc.to_physical(scale),
                        snap.texture.clone(),
                        snap.buffer_scale,
                        snap.buffer_transform,
                        None,
                        Some(snap.view_src),
                        Some((fit_w, fit_h).into()),
                        None,
                        Kind::Unspecified,
                    )
                    .into(),
                );
            }
        }
    }

    elements
}

/// A stable `usize` namespace for a figure key.
///
/// `Id::namespaced` takes a `usize`, and the damage tracker keys on it
/// forever, so this must be deterministic across runs — a per-process
/// counter would leak ids as the document is edited. FNV-1a over the key
/// is cheap and stable.
fn namespace(key: &str) -> usize {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // Keep it non-zero so a namespace is never mistaken for "unset".
    (hash | 1) as usize
}

// ---------------------------------------------------------------------------
// Document overlays
// ---------------------------------------------------------------------------

/// Stable namespace for one peer's overlay elements.
fn peer_key(peer: &str) -> usize {
    peer.bytes().fold(0usize, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(byte as usize)
    })
}

/// A readable caret colour for a peer id, from a small fixed palette.
fn peer_color(peer: &str) -> [f32; 4] {
    const PALETTE: [[f32; 4]; 5] = [
        [0.95, 0.45, 0.35, 0.95],
        [0.35, 0.85, 0.55, 0.95],
        [0.95, 0.75, 0.30, 0.95],
        [0.65, 0.55, 0.95, 0.95],
        [0.35, 0.75, 0.95, 0.95],
    ];
    PALETTE[peer_key(peer) % PALETTE.len()]
}

/// The compositor's own marks on the page: the caret, the selection,
/// and the focused figure's border.
///
/// Drawn **after** the app surfaces so a caret inside a focused figure's
/// figure is never hidden by the app painting over it — the point of the
/// overlay is to be the top layer of the document, exactly like a
/// comment highlight in a PDF viewer.
pub fn build_overlay_elements(
    state: &mut EmthinState,
    renderer: &mut GlesRenderer,
    scale: f64,
) -> Vec<CustomElement<GlesRenderer>> {
    let mut out = Vec::new();
    let focused = focused_figure_key(state);

    // Selection: one translucent box per glyph band. Geometry from an older
    // source generation must not paint boxes over the retained raster.
    if state.doc.geometry_current() {
        if let Some(range) = state.doc.model().selection() {
            for rect in state.doc.layout().selection_rects(range) {
                out.push(solid(
                    &format!("emthin-sel-{}", out.len()),
                    rect,
                    scale,
                    [0.20, 0.45, 0.85, 0.45],
                ));
            }
        }
    }

    // Find matches: every hit faintly, the current one more strongly. Drawn
    // after the normal selection, because stepping a find sets the selection
    // to the current match and the search colour is the useful one then.
    if state.doc.geometry_current() && state.doc.find().is_open() {
        let current = state.doc.find().current_index();
        for (i, range) in state.doc.find().matches().iter().enumerate() {
            let rgba = if current == Some(i) {
                [1.0, 0.72, 0.15, 0.52]
            } else {
                [1.0, 0.90, 0.35, 0.24]
            };
            for rect in state.doc.layout().selection_rects(range.clone()) {
                out.push(solid(
                    &format!("emthin-find-{i}-{}", out.len()),
                    rect,
                    scale,
                    rgba,
                ));
            }
        }
    }

    // Remote collaborators: a thin caret on the visible page. `caret_rect`
    // returns `None` for a byte on another page, so a remote caret is hidden
    // rather than guessed into view. Presence never changes the document.
    let text_len = state.doc.model().len();
    for peer in state.collab.present() {
        let Some(cursor) = peer.cursor else {
            continue;
        };
        let Some(caret) = state
            .doc
            .geometry_current()
            .then(|| state.doc.layout().caret_rect(cursor.min(text_len)))
            .flatten()
        else {
            continue;
        };
        let rect = Rectangle::new(caret.loc, (2, caret.size.h).into());
        out.push(solid(
            &format!("emthin-peer-{}", peer_key(&peer.peer)),
            rect,
            scale,
            peer_color(&peer.peer),
        ));
    }

    // Focused figure border: a visible tint so the user always knows
    // which figure keystrokes are going to.
    if let Some(key) = focused {
        if let Some(figure) = state.doc.figures().get(&key) {
            let rect = figure.rect;
            if rect.size.w > 0 && rect.size.h > 0 {
                let stroke = 2.0;
                // Four thin bars rather than a texture: `SolidColor`
                // needs no upload and stays crisp at any scale.
                let bars = [
                    Rectangle::new(rect.loc, (rect.size.w, stroke as i32).into()),
                    Rectangle::new(
                        (rect.loc.x, rect.loc.y + rect.size.h - stroke as i32).into(),
                        (rect.size.w, stroke as i32).into(),
                    ),
                    Rectangle::new(rect.loc, (stroke as i32, rect.size.h).into()),
                    Rectangle::new(
                        (rect.loc.x + rect.size.w - stroke as i32, rect.loc.y).into(),
                        (stroke as i32, rect.size.h).into(),
                    ),
                ];
                for (i, bar) in bars.into_iter().enumerate() {
                    out.push(solid(
                        &format!("emthin-foc-{key}-{i}"),
                        bar,
                        scale,
                        [0.35, 0.75, 1.0, 0.9],
                    ));
                }
            }
        }
    }

    // Dormant figures: an inset mark so an empty slot is visibly an empty slot
    // and not a figure whose app failed to start. Without it the `Return`
    // relaunch binding is real but undiscoverable.
    //
    // Inset by 1px on every side so it reads as a border *of* the slot rather
    // than a second box next to it, and namespaced by figure key so editing
    // the document does not collapse two marks into one damage-tracker id.
    for (key, rect) in state.doc.dormant_rects_on_current_page() {
        let inset = 1i32;
        let x = rect.loc.x + inset;
        let y = rect.loc.y + inset;
        let w = rect.size.w - 2 * inset;
        let h = rect.size.h - 2 * inset;
        if w <= 0 || h <= 0 {
            continue;
        }
        let stroke = 1i32;
        let bars = [
            Rectangle::new((x, y).into(), (w, stroke).into()),
            Rectangle::new((x, y + h - stroke).into(), (w, stroke).into()),
            Rectangle::new((x, y).into(), (stroke, h).into()),
            Rectangle::new((x + w - stroke, y).into(), (stroke, h).into()),
        ];
        for (i, bar) in bars.into_iter().enumerate() {
            out.push(solid(
                &format!("emthin-dormant-{key}-{i}"),
                bar,
                scale,
                [0.45, 0.50, 0.58, 0.75],
            ));
        }
    }

    // The label goes inside the frame, centred near the top so it reads as a
    // caption for the slot rather than floating over the page. Compact inline
    // slots omit the caption text when too small so text never bleeds outside.
    for (key, rect) in state.doc.dormant_rects_on_current_page() {
        if rect.size.w < 48 || rect.size.h < 28 {
            continue;
        }
        let Some(text) = state.doc.dormant_label(&key) else {
            continue;
        };
        let width_pt = (rect.size.w - 16).max(64) as f64;
        let Some(image) = state.dormant_labels.get(&key, &text, width_pt) else {
            continue;
        };
        let x = rect.loc.x + 8;
        let y = rect.loc.y + 8;
        if let Some(el) = crate::dormant_label::element(renderer, image, x, y, scale) {
            out.push(el);
        }
    }

    // Live terminal retry state. A disconnected Core figure is otherwise
    // indistinguishable from a terminal that is simply quiet, so name the
    // reconnect attempt and its remaining backoff.
    let retry_terminals: Vec<_> = state
        .doc
        .figures()
        .figures()
        .iter()
        .filter(|figure| figure.page == Some(state.doc.current_page()))
        .filter_map(|figure| {
            let reference = figure.spec.terminal.clone()?;
            (figure.rect.size.w > 0 && figure.rect.size.h > 0).then_some((
                figure.key.clone(),
                reference,
                figure.rect,
            ))
        })
        .collect();
    for (key, reference, rect) in retry_terminals {
        let Some(text) = state.core_sessions.retry_status(&reference) else {
            continue;
        };
        let width_pt = (rect.size.w - 16).max(64) as f64;
        let cache_key = format!("reconnect-{key}");
        let Some(image) = state.dormant_labels.get(&cache_key, &text, width_pt) else {
            continue;
        };
        let x = rect.loc.x + 8;
        let y = rect.loc.y + 8;
        let w = image.width as i32;
        let h = image.height as i32;
        out.push(solid(
            &format!("emthin-retry-bg-{key}"),
            Rectangle::new((x - 4, y - 3).into(), (w + 8, h + 6).into()),
            scale,
            [0.06, 0.09, 0.12, 0.92],
        ));
        if let Some(el) = crate::dormant_label::element(renderer, image, x, y, scale) {
            out.push(el);
        }
    }

    // Caret last: it is the single most important mark on the page.
    // Only drawn when a Wayland surface does *not* have keyboard focus —
    // otherwise the caret belongs to an app, not to us.
    let doc_has_focus = state
        .seat
        .get_keyboard()
        .is_some_and(|k| k.current_focus().is_none());
    if doc_has_focus {
        if let Some(rect) = state.doc.caret_rect() {
            out.push(solid("emthin-caret", rect, scale, [1.0, 1.0, 1.0, 0.9]));
        }
    }

    // Math completion preview: the glyph a delimiter would commit, drawn just
    // after the caret. Only while the document owns the keyboard, for the same
    // reason the caret is.
    if doc_has_focus {
        if let Some(completion) = state.doc.completion().cloned() {
            if let Some(caret) = state.doc.caret_rect() {
                if let Some(image) = state.completion_hint.get(&completion.preview) {
                    let x = caret.loc.x + caret.size.w + 2;
                    let y = caret.loc.y - 2;
                    let w = image.width as i32;
                    let h = image.height as i32;
                    out.push(solid(
                        "emthin-completion-bg",
                        Rectangle::new((x - 2, y - 1).into(), (w + 4, h + 2).into()),
                        scale,
                        [0.06, 0.09, 0.12, 0.94],
                    ));
                    if let Some(element) =
                        crate::completion_hint::draw(renderer, image, x, y, scale)
                    {
                        out.push(element);
                    }
                }
            }
        }
    }

    // The find bar sits on top. Its raster is cached by query/status, so
    // redraws while typing only re-raster when the text actually changed.
    if let Some(status) = state.doc.find_status() {
        let query = state.doc.find().query().to_owned();
        if let Some(image) = state.find_bar.get(&query, &status) {
            let x = 12;
            let y = 12;
            let w = image.width as i32;
            let h = image.height as i32;
            let pad_x = 8;
            let pad_y = 6;
            out.push(solid(
                "emthin-find-bg",
                Rectangle::new(
                    (x - pad_x, y - pad_y).into(),
                    (w + pad_x * 2, h + pad_y * 2).into(),
                ),
                scale,
                [0.06, 0.09, 0.12, 0.94],
            ));
            if let Some(element) = crate::find_bar::draw(renderer, image, x, y, scale) {
                out.push(element);
            }
        }
    }

    // Status line: one compact bottom overlay, and nothing at all when the
    // projection is quiet.
    let peer_count = state.collab.present().len();
    if let Some(line) = state.doc.status(peer_count).line() {
        if let Some(image) = state.status_bar.get(&line) {
            let viewport = state.doc.layout().viewport();
            let x = 12;
            let y = (viewport.h - image.height as i32 - 12).max(12);
            let w = image.width as i32;
            let h = image.height as i32;
            let pad_x = 8;
            let pad_y = 5;
            out.push(solid(
                "emthin-status-bg",
                Rectangle::new(
                    (x - pad_x, y - pad_y).into(),
                    (w + pad_x * 2, h + pad_y * 2).into(),
                ),
                scale,
                [0.06, 0.09, 0.12, 0.90],
            ));
            if let Some(element) = crate::status_bar::draw(renderer, image, x, y, scale) {
                out.push(element);
            }
        }
    }

    out
}

/// The key of the figure whose app currently has keyboard focus.
fn focused_figure_key(state: &EmthinState) -> Option<String> {
    use smithay::wayland::seat::WaylandFocus;
    let focus = state.seat.get_keyboard()?.current_focus()?;
    let crate::KeyboardFocusTarget::Window(w) = focus else {
        // A layer-shell surface or popup has focus; no figure does.
        return None;
    };
    let wl = w.wl_surface()?;
    let app_id = state.apps.id_for_surface(&wl)?;
    // Page-aware, like `DocUi::figure_at`. A page switch clears the *saved* and
    // pointer focus but leaves the keyboard focus on the app that was on the old
    // page, and an off-page figure's rect is the `(0, 0)` placeholder — so the
    // border was drawn as a full-page-sized rectangle in the output's corner for
    // as long as that app held focus.
    let page = state.doc.current_page();
    state
        .doc
        .figures()
        .figure_of_app(app_id)
        .filter(|f| f.page == Some(page))
        .map(|f| f.key.clone())
}

/// A flat coloured rectangle as a [`CustomElement`].
///
/// `SolidColorRenderElement` is placed in physical coords like every
/// other custom element, so the rect is scaled here rather than relying
/// on the caller's `scale`.
pub(crate) fn solid(
    _id: &str,
    rect: Rectangle<i32, Logical>,
    scale: f64,
    rgba: [f32; 4],
) -> CustomElement<GlesRenderer> {
    use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
    let buffer = SolidColorBuffer::new(rect.size, Color32F::new(rgba[0], rgba[1], rgba[2], 1.0));
    SolidColorRenderElement::from_buffer(
        &buffer,
        rect.loc.to_f64().to_physical(scale).to_i32_round(),
        scale,
        rgba[3],
        Kind::Unspecified,
    )
    .into()
}

#[cfg(test)]
mod tests {
    use super::namespace;

    #[test]
    fn namespace_is_stable_and_nonzero() {
        assert_eq!(namespace("f0"), namespace("f0"));
        assert_ne!(namespace("f0"), namespace("f1"));
        assert_ne!(namespace("f0"), 0, "a zero id reads as unset");
    }

    #[test]
    fn namespace_never_collides_with_itself_across_keys() {
        let keys: Vec<String> = (0..256).map(|i| format!("f{i}")).collect();
        let mut seen = std::collections::HashSet::new();
        for k in &keys {
            assert!(seen.insert(namespace(k)), "collision on {k}");
        }
    }
}
