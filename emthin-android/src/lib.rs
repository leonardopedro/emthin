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
use jni::sys::{jint, jlong};
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
//   2. The renderer is mathed's own (`mathed_mini::render_doc`), which
//      is the same engine `velysterm/crates/mathed_android` blits. The
//      pad's document syntax *is* mathed's marker syntax, so no
//      conversion is needed.

/// Renders `doc_text` at `width_pt` points wide.
///
/// Returns `{ "width", "height", "rgba" }` with `rgba` base64-encoded
/// RGBA8 — the same one-value transport mathed_android uses for its
/// screenshots.
fn render_text(doc_text: &str, width_pt: f64) -> Result<String, String> {
    use base64::Engine as _;

    let width_pt = if width_pt.is_finite() && width_pt > 0.0 {
        width_pt
    } else {
        600.0
    };

    // `imaging::RgbaImage` carries its dimensions as fields and its
    // pixels in `data` (RGBA8), the same shape engine.rs samples from.
    // `RenderError` is `Debug`-only (a sibling crate's type; not widened
    // here), so the diagnostic uses `{:?}`.
    let img = mathed_mini::render_doc(doc_text, width_pt).map_err(|e| format!("render: {e:?}"))?;
    let (w, h) = (img.width, img.height);
    if w == 0 || h == 0 {
        return Err("render: empty raster".to_string());
    }

    let b64 = base64::engine::general_purpose::STANDARD.encode(&img.data);
    Ok(json!({ "width": w, "height": h, "rgba": b64 }).to_string())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_renderText<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    doc_text: JString<'local>,
    width_pt: jint,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let text = doc_text.to_string();
            // A failed render is data, not a crash: the pad keeps
            // editing and shows the diagnostic instead of dying on a
            // half-typed expression.
            let json = match render_text(&text, width_pt.max(1) as f64) {
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

    /// The render seam actually produces pixels, and the base64 half of
    /// the contract decodes back to exactly `width * height * 4` RGBA
    /// bytes — the invariant the Java side relies on before handing the
    /// buffer to a `Bitmap`.
    #[test]
    fn render_text_returns_a_decodable_rgba_raster() {
        use base64::Engine as _;

        // The app's real starter document (MainActivity.STARTER),
        // string argument and all — not a simplified stand-in, so this
        // pins what the pad actually opens with.
        let starter = String::from("#1 terminal #2 \\app(#1, #2, 600, 300, \"term\")\n")
            + "#3 editor #4 \\app(#3, #4, 400, 300, \"vim\")\n";
        let v: Value = serde_json::from_str(&render_text(&starter, 200.0)
            .expect("the starter document must render"))
            .unwrap();

        assert!(v.get("error").is_none(), "unexpected render error: {v}");
        let (w, h) = (v["width"].as_u64().unwrap(), v["height"].as_u64().unwrap());
        assert!(w > 0 && h > 0, "empty raster {w}x{h}");

        let bytes = base64::engine::general_purpose::STANDARD
            .decode(v["rgba"].as_str().unwrap())
            .expect("rgba must be base64");
        assert_eq!(bytes.len() as u64, w * h * 4, "RGBA8 is 4 bytes/px");
    }

    /// A non-positive width falls back to the default instead of
    /// erroring, so a caller passing a not-yet-measured view width
    /// (0 before first layout) still gets a raster.
    ///
    /// Asserted by *equivalence with an explicit 600pt* rather than by
    /// pixel width: `render_doc` trims the raster to ink, so the width
    /// follows the content, not the request (measured: "hello mathed"
    /// comes back 59x8 at the default width).
    #[test]
    fn render_text_falls_back_on_a_non_positive_width() {
        let fallback = render_text("hello mathed", 0.0).expect("width 0 must fall back, not fail");
        let explicit = render_text("hello mathed", 600.0).expect("600pt must render");
        assert_eq!(fallback, explicit, "0pt must be treated as the 600pt default");
        assert!(!fallback.contains("\"error\""), "unexpected: {fallback}");
    }
}
