//! Native core for the **emthin pad** Android app.
//!
//! `model` is emthin's `docui/model.rs` compiled in verbatim from the
//! sibling `crates/emthin` crate in this repository (see the `#[path]`
//! attribute below), so the
//! app drives the exact document engine the desktop shell uses: a
//! Loro-backed document with hidden marker statements, grapheme-aware
//! editing, and one undo step per edit.
//!
//! Contract with the Java side (`com.emthin.pad.DocBridge`): every call
//! takes a `DocModel` handle (a leaked `Box<DocModel>` pointer) and
//! returns the full editor state as a JSON string:
//!
//! ```json
//! { "text": "...", "revision": 4, "caret": 12,
//!   "canUndo": true, "canRedo": false,
//!   "segments": [ { "prop": "app", "kind": "App", "span": [2, 7] } ] }
//! ```
//!
//! All calls happen on the Android UI thread, so the raw handle needs
//! no synchronisation.

#[path = "../../crates/emthin/src/docui/model.rs"]
pub mod model;

use jni::objects::{JClass, JString};
use jni::sys::{jfloat, jint, jlong};
use jni::{EnvUnowned, errors};
use serde_json::{Value, json};

use crate::model::DocModel;

// ----- handle management ---------------------------------------------

/// Boxes `model` and leaks it, returning the pointer as a Java `long`.
fn new_handle(text: &str) -> jlong {
    Box::into_raw(Box::new(DocModel::new(text))) as jlong
}

/// Recreates the box from a Java `long` handle.
///
/// # Safety
/// `handle` must come from [`new_handle`], must not have been freed
/// already, and the caller must be the only user of it (the Java side
/// only ever calls from the UI thread).
unsafe fn model_mut<'a>(handle: jlong) -> &'a mut DocModel {
    unsafe { &mut *(handle as *mut DocModel) }
}

/// Frees the box behind `handle`.
///
/// # Safety
/// Same requirements as [`model_mut`], plus: the handle must not be
/// used again afterwards.
unsafe fn free_handle(handle: jlong) {
    drop(unsafe { Box::from_raw(handle as *mut DocModel) });
}

// ----- state serialisation -------------------------------------------

/// The full editor state as the JSON string the Java side parses.
fn state_json(m: &DocModel) -> String {
    let segments: Vec<Value> = m
        .segments()
        .iter()
        .map(|s| {
            json!({
                "prop": s.prop,
                "kind": format!("{:?}", s.kind),
                "span": s.span.as_ref().map(|r| vec![r.start, r.end]),
                "stmt": s.stmt,
            })
        })
        .collect();

    json!({
        "text": m.text(),
        "revision": m.revision(),
        "caret": byte_to_utf16(m.text(), m.caret()),
        "canUndo": m.can_undo(),
        "canRedo": m.can_redo(),
        "segments": segments,
    })
    .to_string()
}

// ----- text-coordinate conversion ------------------------------------

/// UTF-16 code-unit index → UTF-8 byte offset (Java `String` indexes
/// are UTF-16; `DocModel` works in bytes).
///
/// An index landing inside a surrogate pair snaps **back** to the
/// start of its code point, so the result is always a char boundary
/// belonging to the requested position's character.
fn utf16_to_byte(text: &str, utf16: usize) -> usize {
    let mut units = 0usize;
    for (byte, ch) in text.char_indices() {
        if units >= utf16 {
            return byte;
        }
        let next = units + ch.len_utf16();
        if next > utf16 {
            // Mid-surrogate: snap to this code point's start.
            return byte;
        }
        units = next;
    }
    text.len()
}

/// UTF-8 byte offset → UTF-16 code-unit index.
fn byte_to_utf16(text: &str, byte: usize) -> usize {
    let byte = byte.min(text.len());
    let mut adjusted = byte;
    while adjusted > 0 && !text.is_char_boundary(adjusted) {
        adjusted -= 1;
    }
    text[..adjusted].chars().map(|c| c.len_utf16()).sum()
}

// ----- editing --------------------------------------------------------

/// Applies the *minimal diff* between the model's current text and
/// `new_text` as a single [`DocModel::replace`] — so every Java edit
/// (keystroke, paste, IME commit) is exactly one undo step — then moves
/// the caret to the Java caret position.
fn sync_text(m: &mut DocModel, new_text: &str, caret_utf16: usize) {
    let old = m.text().to_string();
    if old != new_text {
        let (ob, nb) = (old.as_bytes(), new_text.as_bytes());

        let mut prefix = 0;
        while prefix < ob.len() && prefix < nb.len() && ob[prefix] == nb[prefix] {
            prefix += 1;
        }
        // Equal bytes imply equal boundary structure in both strings,
        // but a divergence *inside* a multi-byte char leaves `prefix`
        // mid-char — back off to a boundary shared by both.
        while prefix > 0 && !(old.is_char_boundary(prefix) && new_text.is_char_boundary(prefix)) {
            prefix -= 1;
        }

        let mut suffix = 0;
        while suffix < ob.len() - prefix
            && suffix < nb.len() - prefix
            && ob[ob.len() - 1 - suffix] == nb[nb.len() - 1 - suffix]
        {
            suffix += 1;
        }
        while suffix > 0
            && !(old.is_char_boundary(ob.len() - suffix)
                && new_text.is_char_boundary(nb.len() - suffix))
        {
            suffix -= 1;
        }

        m.replace(
            prefix..ob.len() - suffix,
            &new_text[prefix..nb.len() - suffix],
        );
    }
    m.set_caret(utf16_to_byte(new_text, caret_utf16));
}

// ----- JNI entry points -----------------------------------------------
//
// Names follow the JNI spec: `Java_com_emthin_pad_DocBridge_<method>`.
// `EnvUnowned` captures the raw JNI env; `with_env` upgrades it (and
// installs the panic/error policy), `resolve` maps errors/panics to a
// Java `RuntimeException` plus a `Default` return value.

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_create<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    initial: JString<'local>,
) -> jlong {
    unowned_env
        .with_env(|_env| -> errors::Result<jlong> {
            let text = initial.to_string();
            Ok(new_handle(&text))
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_free<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) {
    unowned_env
        .with_env(|_env| -> errors::Result<()> {
            if handle != 0 {
                // SAFETY: see free_handle; the Java side frees its
                // handle exactly once, from the UI thread.
                unsafe { free_handle(handle) };
            }
            Ok(())
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_state<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            // SAFETY: handle invariants documented on model_mut.
            let json = unsafe { state_json(model_mut(handle)) };
            JString::from_str(env, json)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_sync<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    text: JString<'local>,
    caret_utf16: jint,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let new_text = text.to_string();
            let caret = caret_utf16.max(0) as usize;
            // SAFETY: handle invariants documented on model_mut.
            let json = unsafe {
                let m = model_mut(handle);
                sync_text(m, &new_text, caret);
                state_json(m)
            };
            JString::from_str(env, json)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_undo<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            // SAFETY: handle invariants documented on model_mut.
            let json = unsafe {
                let m = model_mut(handle);
                m.undo();
                state_json(m)
            };
            JString::from_str(env, json)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_redo<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            // SAFETY: handle invariants documented on model_mut.
            let json = unsafe {
                let m = model_mut(handle);
                m.redo();
                state_json(m)
            };
            JString::from_str(env, json)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

// ----- rendering ------------------------------------------------------
//
// Deliberately *stateless*: it takes the document text, never the
// `DocModel` handle. Two reasons, both load-bearing:
//
//   1. The handle is documented as UI-thread-only (no synchronisation),
//      and Typst layout costs ~400 ms for a few pages — far too long to
//      hold the UI thread without an ANR. Taking text lets the caller
//      copy the string on the UI thread and rasterise on a worker.
//   2. The renderer is mathed's own (`mathed_mini`), which is the same
//      engine `velysterm/crates/mathed_android` blits. The pad's
//      document syntax *is* mathed's marker syntax, so no conversion is
//      needed.
//
// Paged, not one giant raster: the layout is Typst's own page model —
// the PDF-viewer unit `mathed_android` shows one page at a time — so a
// long document streams page by page and scrolling the preview never
// re-rasterises. A chunk (`start`, `max_pages`) is the transport unit:
// every page of a long document in one response would be megabytes of
// base64 across the JNI string boundary.

/// Upper bound on one response's page count whatever the caller asks
/// for — a runaway request cannot balloon the JNI string.
const MAX_CHUNK_PAGES: usize = 8;

/// Renders pages `start..start + max_pages` of `doc_text`'s paged
/// layout at `scale` pixels per point (`max_pages` is clamped to
/// `1..=`[`MAX_CHUNK_PAGES`]; a non-positive or non-finite `scale` is
/// the engine's own 1.0 fallback).
///
/// Returns `{ "total", "start", "pages": [ { "width", "height",
/// "rgba" }, … ] }` with each `rgba` base64-encoded RGBA8 — the same
/// one-value transport mathed_android uses for its screenshots.
/// `total` is the document's full page count, so the caller knows how
/// many chunks there are; a `start` past `total` yields an empty
/// `pages` array ("nothing more to load"), not an error.
fn render_pages(
    doc_text: &str,
    start: usize,
    max_pages: usize,
    scale: f32,
) -> Result<String, String> {
    use base64::Engine as _;
    use mathed_core::transform::TransformOptions;

    // Citations render as their visible `[N]` label only when the
    // transform is handed the document's reference entries: the
    // `\cite(...)` token itself is hidden, so the default (empty)
    // options list made every citation disappear from the preview
    // (found by the 2026-10-19 visual loop). The preview has no caret,
    // so it renders the collapsed view — labels shown, tokens hidden.
    let scan = mathed_core::markers::scan(doc_text);
    let opts = TransformOptions {
        references: mathed_core::markers::scan_references(&scan),
        ..TransformOptions::default()
    };

    // `PageLayout::image` is an `imaging::RgbaImage` carrying its
    // dimensions as fields and its pixels in `data` (RGBA8), the same
    // shape engine.rs samples from. `RenderError` is `Debug`-only (a
    // sibling crate's type; not widened here), so the diagnostic uses
    // `{:?}`.
    let pages = mathed_mini::layout_doc_paged_scaled(doc_text, &opts, scale)
        .map_err(|e| format!("render: {e:?}"))?;
    let total = pages.len();
    if total == 0 {
        return Err("render: no pages".to_string());
    }

    let chunk: Vec<Value> = pages
        .iter()
        .skip(start)
        .take(max_pages.clamp(1, MAX_CHUNK_PAGES))
        .map(|p| {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&p.image.data);
            json!({ "width": p.image.width, "height": p.image.height, "rgba": b64 })
        })
        .collect();
    Ok(json!({ "total": total, "start": start, "pages": chunk }).to_string())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_renderPages<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    doc_text: JString<'local>,
    start: jint,
    max_pages: jint,
    scale: jfloat,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let text = doc_text.to_string();
            // A failed render is data, not a crash: the pad keeps
            // editing and shows the diagnostic instead of dying on a
            // half-typed expression.
            let json = match render_pages(
                &text,
                start.max(0) as usize,
                max_pages.max(0) as usize,
                scale as f32,
            ) {
                Ok(ok) => ok,
                Err(e) => json!({ "error": e }).to_string(),
            };
            JString::from_str(env, json)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

// ----- tests ----------------------------------------------------------
//
// These run the *emthin* `DocModel` tests too (they live inside the
// included file), plus the bridge-specific behaviour above.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_byte_round_trip() {
        let t = "a👋é b"; // code-point boundaries at units 0,1,3,4,5,6
        for unit in [0usize, 1, 3, 4, 5, 6] {
            let b = utf16_to_byte(t, unit);
            assert_eq!(byte_to_utf16(t, b), unit);
        }
        // Mid-surrogate (unit 2, inside 👋) snaps back to the emoji's
        // start (unit 1 / byte 1), never forward past it.
        assert_eq!(utf16_to_byte(t, 2), 1);
        assert_eq!(utf16_to_byte(t, 999), t.len());
        // byte→utf16 clamps to the *UTF-16* length (6), not bytes (9).
        assert_eq!(byte_to_utf16(t, 999), 6);
    }

    #[test]
    fn utf16_index_landing_mid_surrogate_snaps_to_char_start() {
        // "👋" is one char, two UTF-16 units, four UTF-8 bytes.
        assert_eq!(utf16_to_byte("👋", 1), 0);
        assert_eq!(utf16_to_byte("👋", 2), 4);
        assert_eq!(byte_to_utf16("👋", 4), 2);
    }

    #[test]
    fn sync_edits_undo_one_step_at_a_time() {
        let mut m = DocModel::new("");
        m.take_dirty();
        sync_text(&mut m, "hello", 5);
        assert_eq!(m.text(), "hello");
        assert_eq!(m.caret(), 5);
        // `MathDoc`'s UndoManager merges commits inside its 400ms
        // merge interval (rapid typing undoes as a group, like every
        // editor), so wait it out before the second edit to get a
        // separate undo step.
        std::thread::sleep(std::time::Duration::from_millis(450));
        sync_text(&mut m, "hello, world", 12);
        assert_eq!(m.text(), "hello, world");
        // Each edit undone one at a time.
        assert!(m.undo());
        assert_eq!(m.text(), "hello");
        assert!(m.undo());
        assert_eq!(m.text(), "");
        assert!(!m.undo());
    }

    #[test]
    fn rapid_syncs_merge_into_one_undo_group() {
        // …the flip side: two syncs inside the merge interval undo
        // together (emthin's engine-level coalescing, not a bridge bug).
        let mut m = DocModel::new("");
        sync_text(&mut m, "he", 2);
        sync_text(&mut m, "hello", 5);
        assert_eq!(m.text(), "hello");
        assert!(m.undo());
        assert_eq!(m.text(), "");
        assert!(!m.undo());
    }

    #[test]
    fn sync_handles_multibyte_edits() {
        let mut m = DocModel::new("ab");
        sync_text(&mut m, "a👋b", 3);
        assert_eq!(m.text(), "a👋b");
        // caret in UTF-16 units: after the emoji = unit 3 → byte 5.
        assert_eq!(m.caret(), 5);
        assert!(m.undo());
        assert_eq!(m.text(), "ab");
    }

    #[test]
    fn state_json_shape() {
        let m = DocModel::new("#1 cap #2 \\app(#1, #2, 10, 20)");
        let v: Value = serde_json::from_str(&state_json(&m)).unwrap();
        assert_eq!(v["revision"], json!(m.revision()));
        assert_eq!(v["caret"], json!(0));
        assert_eq!(v["canUndo"], json!(false));
        // emthin resolves one segment for the \app statement.
        assert_eq!(v["segments"].as_array().unwrap().len(), 1);
        assert_eq!(v["segments"][0]["prop"], json!("app"));
        assert_eq!(v["segments"][0]["span"], json!([2, 7]));
    }

    /// The page seam actually produces pixels: each returned page's
    /// base64 half decodes back to exactly `width * height * 4` RGBA
    /// bytes — the invariant the Java side relies on before handing a
    /// buffer to a `Bitmap` — and `total` counts the whole document.
    #[test]
    fn render_pages_returns_decodable_rgba_rasters() {
        use base64::Engine as _;

        // The app's real starter document (MainActivity.STARTER),
        // string argument and all — not a simplified stand-in, so this
        // pins what the pad actually opens with.
        let starter = String::from("#1 terminal #2 \\app(#1, #2, 600, 300, \"term\")\n")
            + "#3 editor #4 \\app(#3, #4, 400, 300, \"vim\")\n";
        let v: Value = serde_json::from_str(
            &render_pages(&starter, 0, 8, 1.0).expect("the starter document must render"),
        )
        .unwrap();

        assert!(v.get("error").is_none(), "unexpected render error: {v}");
        let total = v["total"].as_u64().unwrap();
        assert!(total >= 1, "the starter document has pages: {v}");
        let pages = v["pages"].as_array().unwrap();
        assert_eq!(pages.len() as u64, total.min(8), "one entry per page in range");
        for p in pages {
            let (w, h) = (p["width"].as_u64().unwrap(), p["height"].as_u64().unwrap());
            assert!(w > 0 && h > 0, "empty raster {w}x{h}");
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(p["rgba"].as_str().unwrap())
                .expect("rgba must be base64");
            assert_eq!(bytes.len() as u64, w * h * 4, "RGBA8 is 4 bytes/px");
        }
    }

    /// Chunking is a transport detail, not a change of document:
    /// consecutive chunks both see the same `total` and together cover
    /// the document's pages in order, and a `start` past the end
    /// returns an empty `pages` ("nothing more"), not an error.
    #[test]
    fn render_pages_chunks_cover_the_document_in_order() {
        // ~40 figure slots flow well past one A4 page (each `\app`
        // reserves its w×h box), so this exercises real pagination.
        // Raw strings: the document syntax is full of backslashes and
        // quotes and must not be double-escaped into a different doc.
        let mut doc = String::new();
        for i in 0..40u32 {
            let m = 1000 + i;
            doc.push_str(&format!(r#"#{i} block{i} #{m} \app(#{i}, #{m}, 300, 100, "w{i}")"#));
            doc.push('\n');
        }

        let chunk = |start: usize, max: usize| -> Value {
            serde_json::from_str(&render_pages(&doc, start, max, 1.0).expect("chunk renders"))
                .unwrap()
        };
        let first = chunk(0, 2);
        let total = first["total"].as_u64().unwrap();
        assert!(total >= 3, "40 slots must paginate: {total} page(s) only");
        assert_eq!(first["pages"].as_array().unwrap().len(), 2);

        let second = chunk(2, 2);
        assert_eq!(second["total"].as_u64().unwrap(), total, "total is document-wide");
        assert_eq!(second["start"], json!(2));
        assert_eq!(second["pages"].as_array().unwrap().len(), 2);

        // Past the end: empty pages, same total — the "stop loading"
        // signal the Java side keys its scroll streaming on.
        let tail = chunk(total as usize + 1, 2);
        assert_eq!(tail["total"].as_u64().unwrap(), total);
        assert!(tail["pages"].as_array().unwrap().is_empty(), "past the end: no pages");
    }

    /// Bad inputs fall back instead of failing: a non-positive or
    /// non-finite `scale` is the engine's own 1.0 fallback
    /// (`rasterize_at`'s contract), and `max_pages` clamps to at least
    /// 1 — a 0-page response would strand the preview with nothing to
    /// show. Asserted by *byte-equality with the explicit call*: both
    /// routes must produce the identical document view.
    #[test]
    fn render_pages_falls_back_on_bad_scale_and_clamps_the_chunk() {
        let doc = "hello mathed";
        let one = render_pages(doc, 0, 1, 1.0).expect("1.0 renders");
        assert!(one.contains("\"total\":1"), "a short note is one page: {one}");
        assert_eq!(
            render_pages(doc, 0, 1, 0.0).expect("scale 0 falls back"),
            one,
            "0.0 must be treated as the 1.0 default"
        );
        assert_eq!(
            render_pages(doc, 0, 1, f32::NAN).expect("NaN scale falls back"),
            one,
            "a bad device density must not take the render down"
        );
        assert_eq!(
            render_pages(doc, 0, 0, 1.0).expect("0 pages clamps to 1"),
            one,
            "max_pages 0 must clamp to 1, not return an empty chunk"
        );
    }

    /// The preview's `pageView` canvas must stay mathed's engine
    /// background (`mathed_android::engine::BG`). The page raster is
    /// white text on transparent, so a light canvas silently hides the
    /// typeset document — the exact bug the `#14181D` fix closed.
    /// Pinned against the real constant, not a copy: change either side
    /// and this fails instead of the preview going blank. Reads the
    /// shipped Java source, so it guards what `build-apk.sh` compiles.
    #[test]
    fn page_view_canvas_matches_mathed_engine_bg() {
        let java = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/android/src/com/emthin/pad/MainActivity.java"
        ))
        .expect("MainActivity.java must be readable");

        // The pageView background call — anchored on the view name so
        // the root's #101418 (a deliberate, different colour) cannot
        // match. Exactly one occurrence: a renamed call must fail here
        // rather than leave the canvas unpinned.
        let marker = "pageView.setBackgroundColor(Color.parseColor(\"#";
        let occurrences = java.matches(marker).count();
        assert_eq!(
            occurrences, 1,
            "expected exactly one pageView.setBackgroundColor(\"#…\") call"
        );
        let hex_at = java.find(marker).unwrap() + marker.len();
        let hex = java
            .get(hex_at..hex_at + 6)
            .expect("6 hex digits after the marker");
        let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex digits");
        let java_rgb = [channel(0), channel(2), channel(4)];

        assert_eq!(
            java_rgb,
            mathed_android::engine::BG,
            "MainActivity's pageView canvas {hex} must equal mathed's engine BG \
             (#{:02x}{:02x}{:02x}) or white-on-transparent page text is invisible",
            mathed_android::engine::BG[0],
            mathed_android::engine::BG[1],
            mathed_android::engine::BG[2],
        );
    }

    /// Wiring pin (2026-10-19 visual loop): `renderPages` must splice
    /// cite labels. With the default options the `\cite(...)` token is
    /// hidden with *no* label — the citation disappeared from the
    /// preview entirely. Pinned by byte-equivalence (the codebase's
    /// established style): a `\cite` must rasterise exactly like its
    /// visible `[1]` label written literally, so a missing label
    /// changes the raster and fails here.
    #[test]
    fn render_pages_splices_cite_labels_into_the_raster() {
        let cited = "#1 alpha claim #2 \\cite(#1,#2)";
        let literal = "#1 alpha claim #2 [1]";
        let a = render_pages(cited, 0, 8, 1.0).expect("cited renders");
        let b = render_pages(literal, 0, 8, 1.0).expect("literal renders");
        assert_eq!(
            a, b,
            "a \\cite must rasterise exactly like its visible [1] label"
        );
    }
}
