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

pub mod jit;
pub mod modifiers;

#[path = "../../crates/emthin/src/docui/citations.rs"]
pub mod citations;

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
                "startId": s.start_id,
                "endId": s.end_id,
                "span": s.span.as_ref().map(|r| vec![r.start, r.end]),
                "stmt": s.stmt,
            })
        })
        .collect();
    let modifier_links: Vec<Value> = modifiers::links(m).into_iter().map(|link| {
        let source = modifiers::source_range(m, &link.start_id, &link.end_id);
        let target_ids = link.target_ids.clone().or_else(|| modifiers::legacy_target_pair(m, &link.target));
        json!({
            "startId": link.start_id,
            "endId": link.end_id,
            "source": source.as_ref().map(|r| vec![byte_to_utf16(m.text(), r.start), byte_to_utf16(m.text(), r.end)]),
            "target": [byte_to_utf16(m.text(), link.target.start), byte_to_utf16(m.text(), link.target.end)],
            "targetIds": target_ids,
            "legacy": link.target_ids.is_none(),
            "resolved": link.resolved,
        })
    }).collect();
    let markers: Vec<Value> = m.scan().markers.iter().map(|marker| json!({
        "id": marker.id,
        "start": byte_to_utf16(m.text(), marker.range.start),
        "end": byte_to_utf16(m.text(), marker.range.end),
    })).collect();

    json!({
        "text": m.text(),
        "revision": m.revision(),
        "caret": byte_to_utf16(m.text(), m.caret()),
        "canUndo": m.can_undo(),
        "canRedo": m.can_redo(),
        "segments": segments,
        "modifierLinks": modifier_links,
        "markers": markers,
    })
    .to_string()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_setMarkerTargets<'local>(
    mut unowned_env: EnvUnowned<'local>, _class: JClass<'local>, handle: jlong,
    start_id: JString<'local>, end_id: JString<'local>, targets: JString<'local>,
) -> JString<'local> {
    unowned_env.with_env(|env| -> errors::Result<JString<'local>> {
        // SAFETY: the handle is confined to the UI thread.
        let model = unsafe { model_mut(handle) };
        let result = match modifiers::set_marker_targets(model, &start_id.to_string(),
                &end_id.to_string(), &targets.to_string()) {
            Ok(()) => state_json(model),
            Err(message) => json!({"error": message}).to_string(),
        };
        JString::from_str(env, result)
    }).resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_setModifier<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    start_id: JString<'local>,
    end_id: JString<'local>,
    start_utf16: jint,
    end_utf16: jint,
    enabled: jint,
) -> JString<'local> {
    unowned_env.with_env(|env| -> errors::Result<JString<'local>> {
        // SAFETY: the handle is confined to the UI thread.
        let model = unsafe { model_mut(handle) };
        let start = utf16_to_byte(model.text(), start_utf16.max(0) as usize);
        let end = utf16_to_byte(model.text(), end_utf16.max(0) as usize);
        let ok = modifiers::set_link(model, &start_id.to_string(), &end_id.to_string(),
                                     start.min(end)..start.max(end), enabled != 0);
        let result = if ok { state_json(model) } else { json!({"error": "Invalid modifier or selection"}).to_string() };
        JString::from_str(env, result)
    }).resolve::<errors::ThrowRuntimeExAndDefault>()
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

/// Resolve a newly typed `#` using mathed's marker naming rules.
fn typed_marker(text: &str, offset_utf16: usize) -> Option<String> {
    let at = utf16_to_byte(text, offset_utf16);
    if text.as_bytes().get(at) != Some(&b'#') {
        return None;
    }
    let mut without_typed = text.to_owned();
    without_typed.remove(at);
    mathed_core::markers::auto_marker_token(&without_typed, at)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_typedMarker<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    text: JString<'local>,
    offset_utf16: jint,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let token =
                typed_marker(&text.to_string(), offset_utf16.max(0) as usize).unwrap_or_default();
            JString::from_str(env, token)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
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

/// The australVM Cranelift lane: compile a CPS constant-return program,
/// execute it in the JIT, and answer with `{"ok": n, "entry": "run_k"}`
/// — or `{"error": "…"}`. A failed JIT is data, not a crash, matching
/// `renderPages`. Safe off the UI thread: the engine is per-thread.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_jitRoundtrip<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    value: jlong,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let json = match jit::jit_roundtrip(value) {
                Ok((entry, result)) => json!({ "ok": result, "entry": entry }).to_string(),
                Err(e) => json!({ "error": e }).to_string(),
            };
            JString::from_str(env, json)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

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

/// Restore only a snapshot that corresponds to the separately saved text.
fn model_from_snapshot(encoded: &str, expected: &str) -> Option<DocModel> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
    let model = DocModel::from_snapshot(&bytes).ok()?;
    (model.text() == expected).then_some(model)
}

fn search_ranges(text: &str, query: &str) -> Value {
    json!(mathed_core::search::find_matches(text, query).into_iter()
        .map(|range| [byte_to_utf16(text, range.start), byte_to_utf16(text, range.end)])
        .collect::<Vec<_>>())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_search<'local>(
    mut unowned_env: EnvUnowned<'local>, _class: JClass<'local>,
    text: JString<'local>, query: JString<'local>,
) -> JString<'local> {
    unowned_env.with_env(|env| -> errors::Result<JString<'local>> {
        JString::from_str(env, search_ranges(&text.to_string(), &query.to_string()).to_string())
    }).resolve::<errors::ThrowRuntimeExAndDefault>()
}

fn word_boundary_utf16(text: &str, caret: usize, direction: i32) -> usize {
    let pos = utf16_to_byte(text, caret);
    let scan = mathed_core::markers::scan(text);
    let mut atomic = mathed_core::transform::math_span_ranges(text);
    atomic.extend(scan.markers.iter().map(|marker| marker.range.clone()));
    let result = if direction < 0 {
        mathed_core::wordnav::word_boundary_left(text, pos, &atomic)
    } else {
        mathed_core::wordnav::word_boundary_right(text, pos, &atomic)
    };
    byte_to_utf16(text, result)
}

fn math_completion(text: &str, caret: usize) -> Value {
    let offset = utf16_to_byte(text, caret);
    match mathed_core::completion::completion_at(text, offset) {
        Some(c) => json!({"start": byte_to_utf16(text, c.replace.start),
            "end": byte_to_utf16(text, c.replace.end), "text": c.with, "preview": c.preview}),
        None => Value::Null,
    }
}

fn citation_index(text: &str) -> Value {
    let scan = mathed_core::markers::scan(text);
    json!(model::citation_references(&scan).iter().map(|reference| {
        let span = &scan.stmts[reference.stmt_idx].range;
        json!({"start": byte_to_utf16(text, span.start), "end": byte_to_utf16(text, span.end),
            "numbers": reference.numbers, "label": mathed_core::markers::cite_label_text(reference)})
    }).collect::<Vec<_>>())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_citationIndex<'local>(
    mut unowned_env: EnvUnowned<'local>, _class: JClass<'local>, text: JString<'local>,
) -> JString<'local> {
    unowned_env.with_env(|env| -> errors::Result<JString<'local>> {
        JString::from_str(env, citation_index(&text.to_string()).to_string())
    }).resolve::<errors::ThrowRuntimeExAndDefault>()
}

fn citation_detail(text: &str, number: u64, width: f64, scale: f32) -> Value {
    use base64::Engine as _;
    use mathed_mini::cite_popup::{resolve_popup_body, PopupBody};
    let scan = mathed_core::markers::scan(text);
    let references = model::citation_references(&scan);
    let Some(reference) = references.iter().find(|r| r.numbers.contains(&number)) else {
        return json!({"error": "Citation no longer exists"});
    };
    let upstream = mathed_core::markers::scan_references(&scan);
    let upstream_number = upstream.iter().find(|r| r.stmt_idx == reference.stmt_idx)
        .and_then(|r| r.numbers.first()).copied().unwrap_or(number);
    let Some(body) = resolve_popup_body(text, upstream_number) else {
        return json!({"error": "Citation no longer exists"});
    };
    match body {
        PopupBody::Bibliography { keys, .. } => {
            match citations::bibliography_detail(text, number) {
                Ok((citation, entries)) => json!({"kind":"bibliography", "text":citation,
                    "entries":entries, "resolved":true}),
                Err(error) => json!({"kind":"bibliography", "text":format!("[{}]", keys.join(", ")),
                    "resolved":false, "errorDetail":error}),
            }
        },
        PopupBody::DocumentRef { body_text, body_markup } => {
            let body_markup = if body_text.is_empty() { body_markup } else {
                let scan = mathed_core::markers::scan(&body_text);
                let segments = mathed_core::markers::resolve_segments(&scan);
                let options = mathed_core::transform::TransformOptions {
                    references: model::citation_references(&scan), ..Default::default()
                };
                mathed_core::transform::to_render_text(&body_text, &scan, &segments, &options).text
            };
            let world = mathed_mini::MiniWorld::new(format!("{DOCUMENT_THEME}{body_markup}"));
            let image = world.eval_main().and_then(|content| {
                use typst::layout::{Abs, Axes, Region, Size};
                world.layout(&content, Region::new(Size::new(Abs::pt(width), Abs::pt(2048.0)), Axes::splat(false)))
            }).and_then(|frame| mathed_mini::rasterize_at(&frame, scale).ok());
            match image {
                Some(image) => match png_page(image.width, image.height, &image.data) {
                    Ok(png) => json!({"kind":"document", "text":body_text, "width":image.width,
                        "height":image.height, "png":base64::engine::general_purpose::STANDARD.encode(png)}),
                    Err(error) => json!({"error":error}),
                },
                None => json!({"error":"Could not render cited text", "text":body_text}),
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_citationDetail<'local>(
    mut unowned_env: EnvUnowned<'local>, _class: JClass<'local>, text: JString<'local>,
    number: jint, width: jfloat, scale: jfloat,
) -> JString<'local> {
    unowned_env.with_env(|env| -> errors::Result<JString<'local>> {
        let width = if width.is_finite() { (width as f64).clamp(60.0, 1200.0) } else { 250.0 };
        let scale = if scale.is_finite() { scale.clamp(1.0, 3.0) } else { 1.0 };
        JString::from_str(env, citation_detail(&text.to_string(), number.max(0) as u64, width, scale).to_string())
    }).resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_mathCompletion<'local>(
    mut unowned_env: EnvUnowned<'local>, _class: JClass<'local>,
    text: JString<'local>, caret: jint,
) -> JString<'local> {
    unowned_env.with_env(|env| -> errors::Result<JString<'local>> {
        JString::from_str(env, math_completion(&text.to_string(), caret.max(0) as usize).to_string())
    }).resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_wordBoundary<'local>(
    mut unowned_env: EnvUnowned<'local>, _class: JClass<'local>,
    text: JString<'local>, caret: jint, direction: jint,
) -> jint {
    unowned_env.with_env(|_env| -> errors::Result<jint> {
        Ok(word_boundary_utf16(&text.to_string(), caret.max(0) as usize, direction) as jint)
    }).resolve::<errors::ThrowRuntimeExAndDefault>()
}

const DOCUMENT_THEME: &str = "#set text(fill: white, size: 17pt, font: \"DejaVu Sans Mono\", kerning: false, bottom-edge: \"descender\", ligatures: false)\n";

fn modifier_fragment(model: &DocModel, range: std::ops::Range<usize>, excluded: Option<(&str, &str)>, width: f64, scale: f32) -> Value {
    use base64::Engine as _;
    use unicode_segmentation::UnicodeSegmentation;
    let mut boundaries: Vec<usize> = model.text().grapheme_indices(true).map(|(i, _)| i).collect();
    boundaries.push(model.len());
    let start = boundaries.iter().copied().take_while(|i| *i <= range.start).last().unwrap_or(0);
    let end = boundaries.iter().copied().find(|i| *i >= range.end).unwrap_or(model.len());
    let range = start..end;
    let scan = mathed_core::markers::scan(model.text());
    let segments = mathed_core::markers::resolve_segments(&scan);
    let options = mathed_core::transform::TransformOptions {
        references: model::citation_references(&scan),
        block_splices: modifiers::typst_splices_in(model, range.clone(), excluded),
        ..Default::default()
    };
    let fragment = mathed_core::transform::to_render_text_range(model.text(), &scan, &segments, range, &options);
    // Match mathed_mini's document theme and rasterize at device density;
    // enlarging a 1 px/pt snippet makes its glyphs visibly pixelated.
    let markup = format!("{DOCUMENT_THEME}{}", fragment.text);
    let world = mathed_mini::MiniWorld::new(markup);
    let image = world.eval_main().and_then(|content| {
        use typst::layout::{Abs, Axes, Region, Size};
        let region = Region::new(Size::new(Abs::pt(width), Abs::pt(2048.0)), Axes::splat(false));
        world.layout(&content, region)
    }).and_then(|frame| mathed_mini::rasterize_at(&frame, scale).ok());
    match image {
        Some(image) => json!({"width": image.width, "height": image.height,
            "rgba": base64::engine::general_purpose::STANDARD.encode(&image.data)}),
        None => json!({"error": "Could not render comparison"}),
    }
}

fn modifier_comparisons(model: &DocModel, start_id: &str, end_id: &str, width: f64, scale: f32) -> Value {
    let Some(source) = modifiers::source_range(model, start_id, end_id) else {
        return json!({"error": "Modifier markers no longer exist"});
    };
    let mut incoming = Vec::new();
    let mut outgoing = Vec::new();
    for link in modifiers::links(model) {
        if !link.resolved { continue; }
        if link.target.start < source.end && source.start < link.target.end {
            let label = modifiers::source_range(model, &link.start_id, &link.end_id)
                .map(|r| model.text()[r].trim().to_owned())
                .unwrap_or_else(|| "Missing modifier".to_string());
            incoming.push(json!({"label": label,
                "without": modifier_fragment(model, source.clone(), Some((&link.start_id, &link.end_id)), width, scale)}));
        }
        if link.start_id == start_id && link.end_id == end_id {
            outgoing.push(json!({
                "start": byte_to_utf16(model.text(), link.target.start),
                "end": byte_to_utf16(model.text(), link.target.end),
                "legacy": link.target_ids.is_none(),
                "with": modifier_fragment(model, link.target.clone(), None, width, scale),
                "without": modifier_fragment(model, link.target, Some((start_id, end_id)), width, scale),
            }));
        }
    }
    json!({"incoming": incoming, "outgoing": outgoing})
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_modifierComparisons<'local>(
    mut unowned_env: EnvUnowned<'local>, _class: JClass<'local>,
    text: JString<'local>, snapshot: JString<'local>,
    start_id: JString<'local>, end_id: JString<'local>, width: jfloat, scale: jfloat,
) -> JString<'local> {
    unowned_env.with_env(|env| -> errors::Result<JString<'local>> {
        let text = text.to_string();
        let value = match model_from_snapshot(&snapshot.to_string(), &text) {
            Some(model) => modifier_comparisons(&model, &start_id.to_string(), &end_id.to_string(),
                if width.is_finite() { (width as f64).clamp(60.0, 1200.0) } else { 250.0 },
                if scale.is_finite() { scale.clamp(1.0, 3.0) } else { 1.0 }),
            None => json!({"error": "Could not restore document"}),
        };
        JString::from_str(env, value.to_string())
    }).resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_createFromSnapshot<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    snapshot: JString<'local>,
    expected: JString<'local>,
) -> jlong {
    unowned_env
        .with_env(|_env| -> errors::Result<jlong> {
            Ok(model_from_snapshot(&snapshot.to_string(), &expected.to_string())
                .map_or(0, |m| Box::into_raw(Box::new(m)) as jlong))
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_openSnapshot<'local>(
    mut unowned_env: EnvUnowned<'local>, _class: JClass<'local>, snapshot: JString<'local>,
) -> jlong {
    use base64::Engine as _;
    unowned_env.with_env(|_env| -> errors::Result<jlong> {
        let model = base64::engine::general_purpose::STANDARD.decode(snapshot.to_string())
            .ok().and_then(|bytes| DocModel::from_snapshot(&bytes).ok());
        Ok(model.map_or(0, |m| Box::into_raw(Box::new(m)) as jlong))
    }).resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_snapshot<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JString<'local> {
    use base64::Engine as _;
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            // SAFETY: the handle is owned by the UI thread until onDestroy.
            let bytes = unsafe { model_mut(handle).math_doc().snapshot() };
            JString::from_str(env, base64::engine::general_purpose::STANDARD.encode(bytes))
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
/// the engine's own 1.0 fallback). `caret_utf16` is the editor caret
/// as a UTF-16 index, driving reveal-on-caret exactly like the engine.
///
/// Returns `{ "total", "start", "pages": [ { "width", "height",
/// "rgba", "textTop" }, … ] }` with each `rgba` base64-encoded RGBA8 —
/// the same one-value transport mathed_android uses for its screenshots.
/// `textTop` is the first text band's y in page points at or after the
/// first resolved caption span: the pad opens the preview there, so the
/// caption is visible immediately instead of below a tall figure. It is
/// `null` for a page before that caption or with no text; a document
/// with no marker segments falls back to its first text. `total` is the
/// document's full page count, so the caller knows how many chunks there
/// are; a `start` past `total` yields an empty `pages` array ("nothing
/// more to load"), not an error.
/// Typeset all pages, but rasterize only the requested chunk. At device
/// resolution an A4 page can occupy tens of megabytes, so rasterizing the
/// whole document for every chunk would exhaust memory on long documents.
fn layout_page_chunk(
    text: &str, opts: &mathed_core::transform::TransformOptions,
    scale: f32, start: usize, max_pages: usize,
) -> Result<(usize, Vec<mathed_mini::PageLayout>), mathed_mini::RenderError> {
    let render = mathed_mini::doc_to_render_with(text, opts);
    let world = mathed_mini::MiniWorld::new(format!("{DOCUMENT_THEME}{}", render.text));
    let document = typst::compile::<typst_layout::PagedDocument>(&world)
        .output.map_err(|_| mathed_mini::RenderError::Eval)?;
    let total = document.pages().len();
    let pages = document.pages().iter().skip(start).take(max_pages.clamp(1, MAX_CHUNK_PAGES))
        .map(|page| {
            let glyphs = mathed_core::glyphs::build_glyph_index(
                &page.frame, world.main_source(), &render.map, DOCUMENT_THEME.len());
            let figures = mathed_core::figures::figures_in_frame(&page.frame);
            let image = mathed_mini::rasterize_at(&page.frame, scale)?;
            let (width, height) = (image.width, image.height);
            Ok(mathed_mini::PageLayout { image, glyphs, figures, width, height })
        }).collect::<Result<Vec<_>, _>>()?;
    Ok((total, pages))
}

fn png_page(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>, String> {
    // The rasterizer uses premultiplied RGBA; PNG expects straight alpha.
    let mut straight = pixels.to_vec();
    for pixel in straight.chunks_exact_mut(4) {
        let alpha = pixel[3] as u32;
        if alpha > 0 && alpha < 255 {
            for component in &mut pixel[..3] {
                *component = ((*component as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&straight).map_err(|e| e.to_string())?;
    }
    Ok(encoded)
}

fn render_pages(
    doc_text: &str,
    start: usize,
    max_pages: usize,
    scale: f32,
    caret_utf16: usize,
) -> Result<String, String> {
    render_pages_with_model(doc_text, start, max_pages, scale, caret_utf16, None)
}

fn render_pages_with_model(
    doc_text: &str,
    start: usize,
    max_pages: usize,
    scale: f32,
    caret_utf16: usize,
    model: Option<&DocModel>,
) -> Result<String, String> {
    render_pages_with_options(doc_text, start, max_pages, scale, caret_utf16, model, false, false)
}

fn marker_reveal_ranges(scan: &mathed_core::markers::MarkerScan, caret: usize, show: bool) -> Vec<std::ops::Range<usize>> {
    let on_marker = scan.markers.iter().any(|m| m.range.start <= caret && caret <= m.range.end);
    let mut reveal = if !show && on_marker { Vec::new() } else { vec![caret..caret] };
    if show {
        reveal.extend(scan.markers.iter().map(|marker| marker.range.clone()));
    }
    reveal
}

fn render_pages_with_options(
    doc_text: &str,
    start: usize,
    max_pages: usize,
    scale: f32,
    caret_utf16: usize,
    model: Option<&DocModel>,
    show_markers: bool,
    rendered_only: bool,
) -> Result<String, String> {
    render_pages_with_options_expanded(doc_text, start, max_pages, scale, caret_utf16,
        model, show_markers, rendered_only, &[])
}

fn render_pages_with_options_expanded(
    doc_text: &str, start: usize, max_pages: usize, scale: f32, caret_utf16: usize,
    model: Option<&DocModel>, show_markers: bool, rendered_only: bool,
    expanded_pairs: &[(String, String)],
) -> Result<String, String> {
    use base64::Engine as _;
    use mathed_core::transform::TransformOptions;

    // The caret as a UTF-8 byte offset (Java owns UTF-16 indexes; the
    // transform works in bytes, the same conversion `sync` uses).
    let caret = utf16_to_byte(doc_text, caret_utf16);

    // Reveal-on-caret, wired exactly like `mathed_android::Engine::relayout`:
    // `reveal` surfaces the raw token under the caret, and `expand` keeps
    // the caret's collapsible (a marker segment or a translator's code)
    // open so the keyboard can edit it. Citations still splice their
    // visible `[N]` label via `references` (the `\cite(...)` token is
    // hidden and only labelled when the reference entries are handed
    // over; found by the 2026-10-19 visual loop). With the caret parked
    // on plain prose both `reveal` and `expand` are empty and the
    // preview shows the collapsed view.
    let scan = mathed_core::markers::scan(doc_text);
    let segments = mathed_core::markers::resolve_segments(&scan);
    let expand = if rendered_only {
        segments.iter().filter(|s| expanded_pairs.iter().any(|(a, b)| &s.start_id == a && &s.end_id == b))
            .filter_map(|s| s.span.clone()).collect()
    } else {
        mathed_mini::render::reveal_span_in(&scan, &segments, caret).into_iter().collect()
    };
    // A caret on a marker normally reveals that token in mathed. This
    // frontend's explicit marker toggle promises hidden markers even at
    // the caret, while still revealing other token kinds as before.
    let reveal = if rendered_only {
        if show_markers { scan.markers.iter().map(|m| m.range.clone()).collect() }
        else { Vec::new() }
    } else {
        marker_reveal_ranges(&scan, caret, show_markers)
    };
    let mut opts = TransformOptions {
        reveal,
        expand,
        references: model::citation_references(&scan),
        block_splices: model.map(modifiers::typst_splices).unwrap_or_default(),
        ..TransformOptions::default()
    };

    // The document opens on its first caption, not the page margin. The
    // paged layout's glyph index is the authority for where that caption
    // landed: the first resolved segment's span gives a document byte,
    // and the page's first glyph at/after it gives that line's band. A
    // plain-prose document (no segments) falls back to its first text
    // band, and pages before a known caption report null.
    let caption_start = segments
        .iter()
        .filter_map(|s| s.span.as_ref().map(|span| span.start))
        .min();

    // `PageLayout::image` is an `imaging::RgbaImage` carrying its
    // dimensions as fields and its pixels in `data` (RGBA8), the same
    // shape engine.rs samples from. `RenderError` is `Debug`-only (a
    // sibling crate's type; not widened here), so the diagnostic uses
    // `{:?}`.
    let mut warnings = Vec::new();
    let (total, pages) = match layout_page_chunk(doc_text, &opts, scale, start, max_pages) {
        Ok(pages) => pages,
        Err(original) => {
            let Some(model) = model else {
                return Err(format!("Could not typeset the document ({original:?})"));
            };
            let pairs: std::collections::BTreeSet<(String, String)> = modifiers::links(model)
                .into_iter().map(|link| (link.start_id, link.end_id)).collect();
            if pairs.is_empty() {
                return Err(format!("Could not typeset the document ({original:?})"));
            }
            // Isolate failing modifiers against the actual document and the
            // modifiers already accepted. Never modify the Loro document.
            let mut excluded: std::collections::HashSet<_> = pairs.iter().cloned().collect();
            opts.block_splices.clear();
            let mut pages = layout_page_chunk(doc_text, &opts, scale, start, max_pages)
                .map_err(|_| format!("Could not typeset the document ({original:?})"))?;
            for pair in pairs {
                excluded.remove(&pair);
                opts.block_splices = modifiers::typst_splices_excluding(model, 0..model.len(), &excluded);
                match layout_page_chunk(doc_text, &opts, scale, start, max_pages) {
                    Ok(next) => pages = next,
                    Err(_) => {
                        excluded.insert(pair.clone());
                        let name = modifiers::source_range(model, &pair.0, &pair.1)
                            .map(|r| model.text()[r].trim().to_owned())
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| format!("#{} … #{}", pair.0, pair.1));
                        warnings.push(format!("Could not apply modifier ‘{name}’; expected a valid Typst content function."));
                    }
                }
            }
            pages
        }
    };
    if total == 0 {
        return Err("render: no pages".to_string());
    }

    let chunk: Vec<Value> = pages
        .iter()
        .map(|p| -> Result<Value, String> {
            let bytes = if rendered_only { png_page(p.image.width, p.image.height, &p.image.data)? }
                else { p.image.data.clone() };
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            // With a known caption span, a page before the caption must
            // report `null` rather than its own prose, or the pad would
            // open on an earlier paragraph. Only a document with no
            // segments falls back to its first text.
            let text_top = match caption_start {
                Some(start) => p
                    .glyphs
                    .entries
                    .iter()
                    .find(|e| e.doc_byte >= start)
                    .and_then(|e| p.glyphs.bands.get(e.band as usize)),
                None => p
                    .glyphs
                    .entries
                    .first()
                    .and_then(|e| p.glyphs.bands.get(e.band as usize)),
            }
            .map(|band| band.top);
            let hits: Vec<Value> = p.glyphs.entries.iter().filter_map(|e| {
                let band = p.glyphs.bands.get(e.band as usize)?;
                Some(json!([e.x, band.top, e.x + e.advance, band.bottom,
                    byte_to_utf16(doc_text, e.doc_byte)]))
            }).collect();
            let mut result = json!({
                "width": p.image.width,
                "height": p.image.height,
                "textTop": text_top,
                "hasFigures": !p.figures.is_empty(),
                "hits": hits,
            });
            result[if rendered_only { "png" } else { "rgba" }] = json!(b64);
            Ok(result)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({ "total": total, "start": start, "pages": chunk, "warnings": warnings }).to_string())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_renderPagesWithSnapshot<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    doc_text: JString<'local>,
    snapshot: JString<'local>,
    start: jint,
    max_pages: jint,
    scale: jfloat,
    caret_utf16: jint,
    show_markers: jint,
    expanded: JString<'local>,
) -> JString<'local> {
    unowned_env.with_env(|env| -> errors::Result<JString<'local>> {
        let text = doc_text.to_string();
        let model = model_from_snapshot(&snapshot.to_string(), &text);
        let expanded: Vec<(String, String)> = serde_json::from_str(&expanded.to_string()).unwrap_or_default();
        let result = match model {
            Some(model) => render_pages_with_options_expanded(&text, start.max(0) as usize,
                max_pages.max(0) as usize, scale as f32, caret_utf16.max(0) as usize, Some(&model), show_markers != 0, true, &expanded),
            None => Err("Could not restore document for preview".to_string()),
        };
        JString::from_str(env, result.unwrap_or_else(|e| json!({"error": e}).to_string()))
    }).resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_renderPages<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    doc_text: JString<'local>,
    start: jint,
    max_pages: jint,
    scale: jfloat,
    caret_utf16: jint,
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
                caret_utf16.max(0) as usize,
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
    fn typed_markers_use_mathed_names_and_respect_escapes_and_utf16() {
        let first = typed_marker("#", 0).unwrap();
        assert_eq!(
            first,
            format!("#{}", mathed_core::markers::auto_marker_id(1))
        );
        let text = format!("👋 {first} #");
        let offset = text[..text.len() - 1].encode_utf16().count();
        let second = typed_marker(&text, offset).unwrap();
        assert_eq!(
            second,
            format!("#{}", mathed_core::markers::auto_marker_id(2))
        );
        assert_eq!(typed_marker("\\#", 1), None);
        assert!(typed_marker("\\\\#", 2).is_some());
        assert_eq!(typed_marker("hello", 0), None);
    }

    #[test]
    fn preview_hit_positions_are_document_utf16_offsets() {
        let doc = "👋 hello";
        let json = render_pages(doc, 0, 1, 1.0, 0).unwrap();
        let value: Value = serde_json::from_str(&json).unwrap();
        let hits = value["pages"][0]["hits"].as_array().unwrap();
        assert!(hits.iter().any(|h| h[4] == 3), "h begins at UTF-16 offset 3");
        assert!(hits.iter().all(|h| h[1].as_f64().unwrap() < h[3].as_f64().unwrap()));
    }

    #[test]
    fn search_uses_mathed_case_rules_and_android_utf16_offsets() {
        assert_eq!(search_ranges("👋 Alpha α ALPHA", "alpha"), json!([[3, 8], [11, 16]]));
        assert_eq!(search_ranges("👋 Alpha α ALPHA", "Alpha"), json!([[3, 8]]));
        assert_eq!(search_ranges("👋 Alpha α ALPHA", "α"), json!([[9, 10]]));
    }

    #[test]
    fn word_navigation_uses_mathed_boundaries_with_utf16_positions() {
        let text = "👋 alpha beta";
        assert_eq!(word_boundary_utf16(text, 3, 1), 8);
        assert_eq!(word_boundary_utf16(text, 13, -1), 9);
        assert_eq!(word_boundary_utf16("#abc text", 2, 1), 4);
    }

    #[test]
    fn math_completion_uses_mathed_table_and_utf16_positions() {
        let text = "👋 $\\alpha";
        let completion = math_completion(text, text.encode_utf16().count());
        assert_eq!(completion["start"], 4);
        assert_eq!(completion["text"], "α");
        assert!(math_completion("\\alpha", 6).is_null());
    }

    #[test]
    fn citation_details_resolve_hayagriva_and_document_references() {
        let doc = "#1 crazy-rich:\n    type: Book\n    title: Crazy Rich Asians\n    author: Kwan, Kevin\n    date: 2014\n    publisher: Anchor Books\n#2 \\bibliography(#1,#2, refs)\n#3 x #4 \\cite(#3,#4, \"crazy-rich\", bib: \"refs\")";
        let detail = citation_detail(doc, 1, 300.0, 1.0);
        assert_eq!(detail["kind"], "bibliography");
        assert_eq!(detail["resolved"], true);
        assert!(detail["text"].as_str().unwrap().contains("Kwan"));
        assert!(detail["text"].as_str().unwrap().contains("2014"), "citation year missing: {detail}");
        assert!(detail["entries"][0].as_str().unwrap().contains("Crazy Rich Asians"));
        let bare = "#1 crazy-rich:\n    type: Book\n    title: Crazy Rich Asians\n    author: Kwan, Kevin\n    date: 2014\n#2 \\bibliography(#1,#2, refs)\n\\cite(\"crazy-rich\", bib: \"refs\") \\cite(#1,#2)";
        let index = citation_index(bare);
        assert_eq!(index[0]["numbers"], json!([1]));
        assert_eq!(index[1]["numbers"], json!([2]));
        assert_eq!(citation_detail(bare, 1, 300.0, 1.0)["resolved"], true);
        let misspelled = bare.replace("\"crazy-rich\", bib", "\"Kwn14\", bib");
        let detail = citation_detail(&misspelled, 1, 300.0, 1.0);
        assert_eq!(detail["resolved"], false);
        assert!(detail["errorDetail"].as_str().unwrap().contains("crazy-rich"));
        let bibtex = "#a @book{kwan:2014, title={Crazy Rich Asians}, author={Kwan, Kevin}, year={2014}} #b \\bibliography(#a,#b, refs, format: \"bibtex\") \\cite(\"kwan:2014\", bib: \"refs\")";
        let bibtex_detail = citation_detail(bibtex, 1, 300.0, 1.0);
        assert_eq!(bibtex_detail["resolved"], true, "{bibtex_detail}");
        assert!(bibtex_detail["text"].as_str().unwrap().contains("2014"));
        let reference = "#a Hello $x^2$ #b \\cite(#a,#b)";
        let index = citation_index(reference);
        assert_eq!(index[0]["numbers"], json!([1]));
        let page: Value = serde_json::from_str(&render_pages_with_options(
            reference, 0, 1, 1.0, 0, None, false, true).unwrap()).unwrap();
        let cite_start = index[0]["start"].as_u64().unwrap();
        assert!(page["pages"][0]["hits"].as_array().unwrap().iter()
            .any(|hit| hit[4].as_u64() == Some(cite_start)), "citation label must be tappable");
        let detail = citation_detail(reference, 1, 300.0, 2.0);
        assert_eq!(detail["kind"], "document");
        assert!(detail["png"].is_string());
    }

    #[test]
    fn explicit_fold_expands_only_the_selected_translator() {
        let text = "before #3 #let translate(b) = { \"[]\" } #4 \\translator(#3,#4, name: \"ho\")";
        let collapsed: Value = serde_json::from_str(&render_pages_with_options_expanded(
            text, 0, 1, 1.0, 0, None, false, true, &[]).unwrap()).unwrap();
        let expanded: Value = serde_json::from_str(&render_pages_with_options_expanded(
            text, 0, 1, 1.0, 0, None, false, true, &[("3".into(), "4".into())]).unwrap()).unwrap();
        assert_ne!(collapsed["pages"][0]["png"], expanded["pages"][0]["png"]);
        let hits = collapsed["pages"][0]["hits"].as_array().unwrap();
        let marker = byte_to_utf16(text, text.find("#3").unwrap());
        let span = marker + 2;
        assert!(hits.iter().any(|h| h[4] == marker || h[4] == span));
    }

    #[test]
    fn marker_visibility_changes_the_preview_without_changing_the_document() {
        let doc = "#a alpha #b tail";
        let hidden: Value = serde_json::from_str(&render_pages_with_options(
            doc, 0, 1, 1.0, doc.len(), None, false, true).unwrap()).unwrap();
        let shown: Value = serde_json::from_str(&render_pages_with_options(
            doc, 0, 1, 1.0, doc.len(), None, true, true).unwrap()).unwrap();
        assert_ne!(hidden["pages"][0]["png"], shown["pages"][0]["png"]);
    }

    #[test]
    fn rendered_editor_does_not_expand_source_when_caret_enters_a_figure() {
        let doc = "#a caption #b \\app(#a,#b,100,60)";
        let open: Value = serde_json::from_str(&render_pages_with_options(
            doc, 0, 1, 1.0, doc.find("caption").unwrap(), None, false, true).unwrap()).unwrap();
        let elsewhere: Value = serde_json::from_str(&render_pages_with_options(
            doc, 0, 1, 1.0, doc.len(), None, false, true).unwrap()).unwrap();
        assert!(open["pages"][0]["png"].is_string());
        assert_eq!(open["pages"][0]["png"], elsewhere["pages"][0]["png"]);
    }

    #[test]
    fn rendered_editor_transports_a_decodable_full_resolution_page() {
        use base64::Engine as _;
        let result: Value = serde_json::from_str(&render_pages_with_options(
            "Visible text", 0, 1, 2.0, 0, None, false, true).unwrap()).unwrap();
        let page = &result["pages"][0];
        let bytes = base64::engine::general_purpose::STANDARD.decode(page["png"].as_str().unwrap()).unwrap();
        let reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
        assert_eq!(reader.info().width, page["width"].as_u64().unwrap() as u32);
        assert_eq!(reader.info().height, page["height"].as_u64().unwrap() as u32);
    }

    #[test]
    fn hidden_marker_stays_hidden_even_when_caret_touches_it() {
        let doc = "#a alpha #zz";
        let scan = mathed_core::markers::scan(doc);
        let segments = mathed_core::markers::resolve_segments(&scan);
        let make = |shown| {
            let options = mathed_core::transform::TransformOptions {
                reveal: marker_reveal_ranges(&scan, doc.len(), shown),
                ..Default::default()
            };
            mathed_core::transform::to_render_text(doc, &scan, &segments, &options).text
        };
        assert!(!make(false).contains("zz"));
        assert!(make(true).contains("zz"));
    }

    #[test]
    fn saved_snapshot_preserves_loro_marks_and_rejects_mismatched_text() {
        use base64::Engine as _;
        let mut model = DocModel::new("a👋b");
        model.math_doc_mut().mark_segment(1..5, "prop:bold", "example");
        model.math_doc_mut().commit();
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(model.math_doc().snapshot());
        let restored = model_from_snapshot(&encoded, "a👋b").unwrap();
        assert_eq!(restored.math_doc().segment_marks(), vec![(1..5, "prop:bold".into())]);
        assert!(model_from_snapshot(&encoded, "different").is_none());
        assert!(model_from_snapshot("not-base64", "a👋b").is_none());
    }

    #[test]
    fn loro_modifier_changes_the_real_page_raster() {
        let mut model = DocModel::new("#a bold #b hello");
        let at = model.text().find("hello").unwrap();
        assert!(modifiers::set_link(&mut model, "a", "b", at..at + 5, true));
        let plain: Value = serde_json::from_str(&render_pages(model.text(), 0, 1, 1.0, 0).unwrap()).unwrap();
        let styled: Value = serde_json::from_str(&render_pages_with_model(model.text(), 0, 1, 1.0, 0, Some(&model)).unwrap()).unwrap();
        assert_ne!(plain["pages"][0]["rgba"], styled["pages"][0]["rgba"]);
    }

    #[test]
    fn modifier_comparisons_render_with_and_without_the_link() {
        let mut model = DocModel::new("#a bold #b hello");
        let at = model.text().find("hello").unwrap();
        assert!(modifiers::set_link(&mut model, "a", "b", at..at + 5, true));
        let result = modifier_comparisons(&model, "a", "b", 250.0, 2.0);
        let rows = result["outgoing"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0]["with"]["rgba"].is_string());
        assert!(rows[0]["without"]["rgba"].is_string());
        assert_ne!(rows[0]["with"]["rgba"], rows[0]["without"]["rgba"]);
    }

    #[test]
    fn incoming_modifier_is_listed_for_the_marked_target() {
        let mut model = DocModel::new("#a bold #b #c hello #d");
        let at = model.text().find("hello").unwrap();
        assert!(modifiers::set_link(&mut model, "a", "b", at..at + 5, true));
        let result = modifier_comparisons(&model, "c", "d", 250.0, 2.0);
        let rows = result["incoming"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["label"], "bold");
        assert!(rows[0]["without"]["rgba"].is_string());
    }

    #[test]
    fn comparison_rasters_follow_density_and_wrap_at_column_width() {
        let text = "a longer passage that wraps across several lines";
        let model = DocModel::new(text);
        let one = modifier_fragment(&model, 0..text.len(), None, 100.0, 1.0);
        let dense = modifier_fragment(&model, 0..text.len(), None, 100.0, 2.0);
        assert!(one["height"].as_u64().unwrap() > 30);
        for dimension in ["width", "height"] {
            let expected = one[dimension].as_u64().unwrap() * 2;
            assert!(dense[dimension].as_u64().unwrap().abs_diff(expected) <= 1);
        }
    }

    #[test]
    fn crossing_loro_modifiers_render_in_typst() {
        let mut model = DocModel::new("#a bold #b #c italic #d abcdefgh");
        let at = model.text().find("abcdefgh").unwrap();
        assert!(modifiers::set_link(&mut model, "a", "b", at..at + 5, true));
        assert!(modifiers::set_link(&mut model, "c", "d", at + 3..at + 8, true));
        assert!(render_pages_with_model(model.text(), 0, 1, 1.0, 0, Some(&model)).is_ok());
    }

    #[test]
    fn invalid_modifier_does_not_hide_the_document_or_valid_modifier() {
        let mut model = DocModel::new("\n#ad bold #am\n#i fgh #o\n \n    ");
        let bold = model.text().find("bold").unwrap();
        let target = model.text().find("fgh").unwrap();
        assert!(modifiers::set_link(&mut model, "ad", "am", target..target + 3, true));
        let expected: Value = serde_json::from_str(
            &render_pages_with_model(model.text(), 0, 1, 1.0, 0, Some(&model)).unwrap()
        ).unwrap();
        assert!(modifiers::set_link(&mut model, "i", "o", bold..bold + 4, true));
        let actual: Value = serde_json::from_str(
            &render_pages_with_model(model.text(), 0, 1, 1.0, 0, Some(&model)).unwrap()
        ).unwrap();
        assert_eq!(actual["pages"][0]["rgba"], expected["pages"][0]["rgba"]);
        assert!(actual["warnings"][0].as_str().unwrap().contains("fgh"));
        assert_eq!(modifiers::links(&model).len(), 2, "rendering must not remove links");
    }

    #[test]
    fn typst_function_expression_can_modify_an_interval() {
        let mut model = DocModel::new("#a text.with(fill: red) #b hello");
        let at = model.text().find("hello").unwrap();
        assert!(modifiers::set_link(&mut model, "a", "b", at..at + 5, true));
        assert!(render_pages_with_model(model.text(), 0, 1, 1.0, 0, Some(&model)).is_ok());
    }

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
    fn undo_restores_a_session_base_not_the_empty_document() {
        // Device shape (round-6 visual loop, 2026-10-20): the model is
        // created with the session's non-empty starter text, the fresh
        // EditText caret sits at 0, and `input text` prepends — so the
        // minimal diff is a *whole-document* replace. One undo (the
        // rapid keystrokes merged into one group) must restore the
        // starter, not blank the document. The other undo tests all
        // start from "" and cannot see this.
        let base = "#1 terminal #2 \\app(#1, #2, 600, 300, \"term\")\n";
        let mut m = DocModel::new(base);
        // Caret-0 insertion with the caret advancing per char (what the
        // EditText does as `input text` lands at the fresh cursor).
        let mut typed = String::new();
        for ch in "undoProbe".chars() {
            typed.push(ch);
            let full = format!("{typed}{base}");
            sync_text(&mut m, &full, typed.len());
        }
        assert!(m.text().starts_with("undoProbe"));
        assert!(m.undo(), "undo must have something to revert");
        assert_eq!(m.text(), base, "undo must restore the session base");
        assert!(m.redo(), "redo must re-apply the merged group");
        assert!(m.text().starts_with("undoProbe"));
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
            &render_pages(&starter, 0, 8, 1.0, 0).expect("the starter document must render"),
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

    /// Round-6 device-loop regression (2026-10-20): after an undo the
    /// caret can sit *inside* the starter's `\app(...)` label (byte 9 =
    /// "terminal"), and the pad's reveal-on-caret render came back a
    /// blank page on device (preview stayed black for 26 s, no error,
    /// caption "page 1 / 2" — measured with the visual loop). A render
    /// at any caret must keep visible pixels, not just a well-formed
    /// empty raster.
    #[test]
    fn render_with_caret_inside_the_starter_label_is_not_blank() {
        use base64::Engine as _;

        let starter = String::from("#1 terminal #2 \\app(#1, #2, 600, 300, \"term\")\n")
            + "#3 editor #4 \\app(#3, #4, 400, 300, \"vim\")\n";
        for (caret, scale) in [(0usize, 1.0f32), (9, 1.0), (30, 1.0), (9, 2.5), (0, 2.5)] {
            let v: Value = serde_json::from_str(
                &render_pages(&starter, 0, 3, scale, caret).expect("the starter document must render"),
            )
            .unwrap();
            assert!(v.get("error").is_none(), "caret {caret}: unexpected render error: {v}");
            let pages = v["pages"].as_array().unwrap();
            assert!(!pages.is_empty(), "caret {caret}: no page: {v}");
            let p = &pages[0];
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(p["rgba"].as_str().unwrap())
                .expect("rgba must be base64");
            let visible = bytes.chunks_exact(4).filter(|px| px[3] > 0).count();
            assert!(
                visible > 1_000,
                "caret {caret} scale {scale}: page raster nearly blank ({visible} visible px)"
            );
        }
    }

    /// The preview scrolls to the first caption on open, so the render
    /// must report where that caption typeset. On the starter document
    /// the 600x300 figure slot comes first, so the caption is below it
    /// (and below the A4 top margin), not at y=0.
    #[test]
    fn render_pages_reports_the_first_caption_top() {
        let starter = String::from("#1 terminal #2 \\app(#1, #2, 600, 300, \"term\")\n")
            + "#3 editor #4 \\app(#3, #4, 400, 300, \"vim\")\n";
        let v: Value = serde_json::from_str(
            &render_pages(&starter, 0, 8, 1.0, starter.len()).expect("the starter renders"),
        )
        .unwrap();
        let page = &v["pages"][0];
        let top = page["textTop"]
            .as_f64()
            .expect("page 1 must report the caption's y");
        let height = page["height"].as_f64().unwrap();
        assert!(
            top > 300.0,
            "the starter's first caption sits below its 300pt figure slot, got y={top}"
        );
        assert!(
            top < height,
            "caption top {top} must be inside the {height}pt page"
        );
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
            serde_json::from_str(&render_pages(&doc, start, max, 1.0, 0).expect("chunk renders"))
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
        let one = render_pages(doc, 0, 1, 1.0, 0).expect("1.0 renders");
        assert!(one.contains("\"total\":1"), "a short note is one page: {one}");
        assert_eq!(
            render_pages(doc, 0, 1, 0.0, 0).expect("scale 0 falls back"),
            one,
            "0.0 must be treated as the 1.0 default"
        );
        assert_eq!(
            render_pages(doc, 0, 1, f32::NAN, 0).expect("NaN scale falls back"),
            one,
            "a bad device density must not take the render down"
        );
        assert_eq!(
            render_pages(doc, 0, 0, 1.0, 0).expect("0 pages clamps to 1"),
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

    /// Round-6 pin (2026-10-20): the page ImageView must be laid out at
    /// an explicit pixel height (an EXACTLY spec), never via
    /// `adjustViewBounds` — on the device (waydroid API 33)
    /// adjustViewBounds(true) + wrap-content measured ~428px for a
    /// 1489x2105 raster, about 1/6 of the aspect-correct 2675, which
    /// squeezed the typeset page to sub-pixel and read as a black
    /// preview. `fillViewport(false)` is pinned for the same failure
    /// class: fillViewport(true) hands the ScrollView's children
    /// viewport-capped measure specs. Reads the shipped Java source, so
    /// it guards what `build-apk.sh` compiles.
    #[test]
    fn page_view_uses_explicit_height_and_unfilled_scroll() {
        let java = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/android/src/com/emthin/pad/MainActivity.java"
        ))
        .expect("MainActivity.java must be readable");

        assert!(
            java.contains("previewPages.addView(pageView, new LinearLayout.LayoutParams("),
            "the page ImageView must be added with an explicit LayoutParams height (EXACTLY spec)"
        );
        assert!(
            !java.contains("setAdjustViewBounds(true)"),
            "adjustViewBounds measured ~428px for a full page raster on device (round 6); keep the explicit height"
        );
        assert!(
            java.contains("previewScroll.setFillViewport(false)"),
            "fillViewport(true) caps child measurement at the viewport — the same squeeze class"
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
        // Trailing plain prose so the caret has a neutral parking spot
        // (on prose nothing is revealed or expanded, which is the
        // collapsed view this test pins).
        let cited = "#1 alpha claim #2 \\cite(#1,#2)\n\nprose tail";
        let literal = "#1 alpha claim #2 [1]\n\nprose tail";
        let a = render_pages(cited, 0, 8, 1.0, cited.find("prose").unwrap())
            .expect("cited renders");
        let b = render_pages(literal, 0, 8, 1.0, literal.find("prose").unwrap())
            .expect("literal renders");
        let a: Value = serde_json::from_str(&a).unwrap();
        let b: Value = serde_json::from_str(&b).unwrap();
        assert_eq!(
            a["pages"][0]["rgba"], b["pages"][0]["rgba"],
            "a \\cite must rasterise exactly like its visible [1] label"
        );
    }

    /// Reveal-on-caret through the pad's real render path, mirroring
    /// `mathed_android::Engine::relayout`: parking the caret on plain
    /// prose shows the collapsed view (cite label), while moving it
    /// into the cite segment expands the collapsible (raw `\\cite(...)`
    /// token, label stepped aside) and changes the raster.
    #[test]
    fn render_pages_reveals_the_carets_segment() {
        let doc = "#1 alpha claim #2 \\cite(#1,#2)\n\nprose tail";
        let collapsed = render_pages(doc, 0, 8, 1.0, doc.find("prose").unwrap())
            .expect("caret on prose renders");
        let expanded = render_pages(doc, 0, 8, 1.0, doc.find("cite").unwrap())
            .expect("caret in segment renders");
        assert_ne!(
            collapsed, expanded,
            "moving the caret into the segment must change the render"
        );
    }

    /// Opening the pad must show the typeset document, not the first
    /// statement's raw markers. The editor's caret starts at 0, which
    /// reveal-on-caret reads as the first segment's start; the first
    /// render parks its *render* caret at the document end instead. The
    /// editor caret itself is untouched, so typing still starts at 0.
    #[test]
    fn first_render_parks_the_render_caret_at_the_document_end() {
        let java = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/android/src/com/emthin/pad/MainActivity.java"
        ))
        .expect("MainActivity.java must be readable");

        assert!(
            java.contains("int caret = firstRender ? text.length() : editor.getSelectionStart();"),
            "the first render must use the document end as its caret so the startup preview is collapsed"
        );
        assert!(
            java.contains("requestChunk(text, 0, caret);"),
            "startRender must pass the first render's parked caret into chunk 0"
        );
    }

    /// The first paint must show the first caption, not the empty page
    /// margin above it. The renderer reports the caption's layout y in
    /// points; the pad maps it through the page's displayed width and
    /// scrolls once, at the first page in the stream that has the caption.
    #[test]
    fn first_paint_scrolls_to_the_rendered_caption() {
        let java = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/android/src/com/emthin/pad/MainActivity.java"
        ))
        .expect("MainActivity.java must be readable");

        assert!(
            java.contains("p.optDouble(\"textTop\", -1)"),
            "the pad must read the renderer's caption top"
        );
        assert!(
            java.contains("previewScroll.scrollTo(0, y);"),
            "the first paint must scroll the preview to the caption"
        );
        assert!(
            java.contains("scrollToFirstCaption = false;"),
            "the caption scroll must be one-shot, not jump on every edit"
        );
    }

    /// An edit rebuilds every page, so the ScrollView loses its offset.
    /// The pad keeps the pre-render offset and reapplies it once the new
    /// pages are in place; otherwise the first keystroke snaps the user
    /// back to the page margin above the caption.
    #[test]
    fn edits_restore_the_preview_scroll_position() {
        let java = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/android/src/com/emthin/pad/MainActivity.java"
        ))
        .expect("MainActivity.java must be readable");

        assert!(
            java.contains("restoreScrollY = previewScroll.getScrollY();"),
            "an edit must capture the old offset before the page rebuild"
        );
        assert!(
            java.contains("restorePreviewScroll();"),
            "the rebuilt chunk must restore the captured offset"
        );
        assert!(
            java.contains("previewScroll.scrollTo(0, visibleTarget);"),
            "restoring the offset must scroll the rebuilt preview"
        );
        // A page-count collapse clamps scrollTo to the new, shorter maximum.
        // The pad must retain the *desired* target so an undo that restores
        // the pages can return to the original reading position.
        assert!(
            java.contains("int actual = previewScroll.getScrollY();")
                && java.contains("restoreScrollY = target;"),
            "a clamped restore must keep the desired offset for the next render"
        );
        // ... but only until the user takes over the reading position.
        assert!(
            java.contains("previewScroll.setOnTouchListener")
                && java.contains("event.getActionMasked() == MotionEvent.ACTION_DOWN"),
            "a touch on the preview must release a retained scroll target"
        );
    }

    /// A cold launch rebuilds DocModel from its initial text. Without the
    /// autosave the pad always reopened the starter and every edit was lost
    /// when Android killed the process.
    #[test]
    fn the_document_survives_a_cold_launch() {
        let java = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/android/src/com/emthin/pad/MainActivity.java"
        ))
        .expect("MainActivity.java must be readable");

        assert!(
            java.contains("getSharedPreferences(PREFS, MODE_PRIVATE)")
                && java.contains("getString(PREF_DOC, null)"),
            "onCreate must load the autosaved document before creating the model"
        );
        assert!(
            java.contains(".putString(PREF_DOC, editor.getText().toString())")
                && java.contains("protected void onPause()"),
            "edits and onPause must save the editor text"
        );
        assert!(
            java.contains(".putInt(PREF_CARET, Math.max(0, editor.getSelectionStart()))")
                && java.contains(".putInt(PREF_SCROLL, readingY())"),
            "savePosition must persist the caret and preview reading offset"
        );
        assert!(
            java.contains("DocBridge.sync(handle, initial, savedCaret)")
                && java.contains("restoreScrollY = savedScroll")
                && java.contains("bringPointIntoView(caret)"),
            "a cold launch must restore the caret, scroll it into view, and restore the reading offset"
        );
    }
}
