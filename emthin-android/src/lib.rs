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

#[path = "../../crates/emthin/src/docui/change.rs"]
pub mod change;

#[path = "../../crates/emthin/src/docui/inspect.rs"]
pub mod inspect;

#[path = "../../crates/emthin/src/docui/model.rs"]
pub mod model;

pub mod austral_rule;
pub mod diagnostics;
pub mod jit;
pub mod layout;
pub mod modifiers;
pub mod scene;
pub mod semantics;
pub mod sharing;

#[path = "../../crates/emthin/src/docui/citations.rs"]
pub mod citations;

use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jboolean, jbyte, jfloat, jint, jlong};
use jni::{errors, EnvUnowned};
use serde_json::{json, Value};
use unicode_segmentation::UnicodeSegmentation;

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
    let markers: Vec<Value> = m
        .scan()
        .markers
        .iter()
        .map(|marker| {
            json!({
                "id": marker.id,
                "start": byte_to_utf16(m.text(), marker.range.start),
                "end": byte_to_utf16(m.text(), marker.range.end),
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
        "modifierLinks": modifier_links,
        "markers": markers,
    })
    .to_string()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_setMarkerTargets<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    start_id: JString<'local>,
    end_id: JString<'local>,
    targets: JString<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            // SAFETY: the handle is confined to the UI thread.
            let model = unsafe { model_mut(handle) };
            let result = match modifiers::set_marker_targets(
                model,
                &start_id.to_string(),
                &end_id.to_string(),
                &targets.to_string(),
            ) {
                Ok(()) => state_json(model),
                Err(message) => json!({"error": message}).to_string(),
            };
            JString::from_str(env, result)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
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
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            // SAFETY: the handle is confined to the UI thread.
            let model = unsafe { model_mut(handle) };
            let start = utf16_to_byte(model.text(), start_utf16.max(0) as usize);
            let end = utf16_to_byte(model.text(), end_utf16.max(0) as usize);
            let ok = modifiers::set_link(
                model,
                &start_id.to_string(),
                &end_id.to_string(),
                start.min(end)..start.max(end),
                enabled != 0,
            );
            let result = if ok {
                state_json(model)
            } else {
                json!({"error": "Invalid modifier or selection"}).to_string()
            };
            JString::from_str(env, result)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
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

/// Start of the grapheme cluster containing or ending at `byte`.
fn grapheme_start(text: &str, byte: usize) -> usize {
    let byte = byte.min(text.len());
    if byte == 0 {
        return 0;
    }
    for (start, grapheme) in text.grapheme_indices(true) {
        if byte <= start {
            return start;
        }
        if byte < start + grapheme.len() {
            return start;
        }
    }
    text.len()
}

/// End of the grapheme cluster containing or starting at `byte`.
fn grapheme_end(text: &str, byte: usize) -> usize {
    let byte = byte.min(text.len());
    if byte == text.len() {
        return byte;
    }
    for (start, grapheme) in text.grapheme_indices(true) {
        if byte <= start {
            return start;
        }
        let end = start + grapheme.len();
        if byte < end {
            return end;
        }
    }
    text.len()
}

/// Previous grapheme boundary strictly before `byte`.
fn previous_grapheme_start(text: &str, byte: usize) -> usize {
    let mut previous = 0;
    for (start, _) in text.grapheme_indices(true) {
        if start >= byte {
            break;
        }
        previous = start;
    }
    previous
}

/// Next grapheme boundary strictly after `byte`.
fn next_grapheme_end(text: &str, byte: usize) -> usize {
    let byte = byte.min(text.len());
    for (start, grapheme) in text.grapheme_indices(true) {
        if start >= byte {
            return (start + grapheme.len()).min(text.len());
        }
    }
    text.len()
}

/// Grapheme boundary in UTF-16 units. `direction` is -1 for the previous
/// boundary, 0/1 for the start/end of the cluster containing `offset`, and
/// 2 for the next boundary. The same UAX #29 rules used by `DocModel`
/// backspace/delete and modifier range snapping are used here.
fn grapheme_boundary_utf16(text: &str, offset: usize, direction: i32) -> usize {
    let byte = utf16_to_byte(text, offset);
    let boundary = match direction {
        d if d < 0 => {
            let start = grapheme_start(text, byte);
            if start == byte {
                previous_grapheme_start(text, byte)
            } else {
                start
            }
        }
        0 => grapheme_start(text, byte),
        1 => grapheme_end(text, byte),
        _ => {
            let end = grapheme_end(text, byte);
            if end == byte {
                next_grapheme_end(text, byte)
            } else {
                end
            }
        }
    };
    byte_to_utf16(text, boundary)
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
        // A byte diff can land inside a multi-byte char or a grapheme
        // cluster (combining marks, ZWJ emoji). Expand both ends to whole
        // clusters so a keystroke never leaves half a user-perceived
        // character behind or splits a surrogate/ZWJ sequence.
        let start = grapheme_start(&old, prefix).min(grapheme_start(new_text, prefix));

        let mut suffix = 0;
        while suffix < ob.len() - prefix
            && suffix < nb.len() - prefix
            && ob[ob.len() - 1 - suffix] == nb[nb.len() - 1 - suffix]
        {
            suffix += 1;
        }
        let old_end = grapheme_end(&old, ob.len() - suffix).max(start);
        let new_end = grapheme_end(new_text, nb.len() - suffix).max(start);

        m.replace(start..old_end, &new_text[start..new_end]);
    }
    m.set_caret(utf16_to_grapheme_byte(new_text, caret_utf16));
}

/// UTF-16 index to a UTF-8 byte start snapped to a grapheme boundary.
fn utf16_to_grapheme_byte(text: &str, utf16: usize) -> usize {
    grapheme_start(text, utf16_to_byte(text, utf16))
}

/// Escape a catalogue caption for the document body. A caption is ordinary
/// text, but `#` would start a marker and a newline could open a statement.
fn escape_caption(caption: &str) -> String {
    caption
        .replace('\\', "\\\\")
        .replace('#', "\\#")
        .replace(['\n', '\r'], " ")
}

/// Escape a terminal reference for a quoted `\app` argument.
fn escape_arg(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\n', '\r'], " ")
}

/// The `terminal:` literal argument of one segment, if it has one.
fn terminal_argument(
    segment: &mathed_core::markers::Segment,
) -> Option<&mathed_core::markers::Arg> {
    segment.extra_args.iter().find(|arg| match arg {
        mathed_core::markers::Arg::Literal { text, .. } => text
            .split_once(':')
            .is_some_and(|(name, _)| name.trim() == "terminal"),
        mathed_core::markers::Arg::MarkerRef { .. } => false,
    })
}

/// Public terminal figures a Rebind picker can target. Only source metadata is
/// returned; captions and references are safe to show and persist.
fn terminal_figure_options(text: &str) -> String {
    let scan = mathed_core::markers::scan(text);
    let figures: Vec<Value> = mathed_core::markers::resolve_segments(&scan)
        .iter()
        .filter_map(|segment| {
            let terminal = terminal_argument(segment)?;
            let figure = mathed_core::figures::resolve_figure(segment)?;
            let caption = segment
                .span
                .as_ref()
                .map(|span| text[span.clone()].trim())
                .unwrap_or("Figure");
            Some(json!({
                "key": figure.key,
                "caption": caption,
                "reference": figure.spec.terminal.as_ref().map(|r| r.canonical()),
                "bindingError": figure.spec.binding_error,
                "argument": match terminal {
                    mathed_core::markers::Arg::Literal { text, .. } => text,
                    mathed_core::markers::Arg::MarkerRef { .. } => "",
                },
            }))
        })
        .collect();
    json!(figures).to_string()
}

/// Rewrite only a figure's `terminal:` argument. Everything else in the
/// statement-and the document-stays byte-identical, and the edit is one undo
/// step through the normal `DocModel` transaction.
fn rebind_terminal_figure(
    model: &mut DocModel,
    expected_text: &str,
    key: &str,
    reference: &str,
) -> Result<(), String> {
    if model.text() != expected_text {
        return Err("Document changed before the session was rebound".to_string());
    }
    let reference = mathed_core::figures::TerminalRef::parse(reference)?;
    let scan = mathed_core::markers::scan(model.text());
    let segments = mathed_core::markers::resolve_segments(&scan);
    let segment = segments
        .iter()
        .find(|segment| mathed_core::figures::figure_key(segment.stmt) == key)
        .ok_or_else(|| "That figure no longer exists".to_string())?;
    let argument = terminal_argument(segment)
        .ok_or_else(|| "That figure has no terminal binding to replace".to_string())?;
    let range = match argument {
        mathed_core::markers::Arg::Literal { range, .. } => range.clone(),
        mathed_core::markers::Arg::MarkerRef { .. } => {
            return Err("That figure has no terminal binding to replace".to_string());
        }
    };
    let canonical = reference.canonical();
    if model.text()[range.clone()] == format!("terminal: \"{canonical}\"") {
        return Ok(());
    }
    model.replace(range, &format!("terminal: \"{}\"", escape_arg(&canonical)));
    Ok(())
}

/// Append a terminal figure as one undo step. The caller's expected text makes
/// a stale Sessions tap fail closed instead of editing a replaced document.
fn append_terminal_figure(
    model: &mut DocModel,
    expected_text: &str,
    reference: &str,
    caption: &str,
) -> Result<(), String> {
    if model.text() != expected_text {
        return Err("Document changed before the session was pinned".to_string());
    }
    let reference = mathed_core::figures::TerminalRef::parse(reference)?;
    let ids = {
        let scan = model.scan();
        mathed_core::markers::lowest_free_marker_numbers(scan, 2)
    };
    let (Some(&a), Some(&b)) = (ids.first(), ids.get(1)) else {
        return Err("Could not allocate figure markers".to_string());
    };
    let start = format!("#{}", mathed_core::markers::auto_marker_id(a));
    let end = format!("#{}", mathed_core::markers::auto_marker_id(b));
    let caption = escape_caption(caption.trim());
    let caption = if caption.is_empty() {
        "Terminal"
    } else {
        caption.as_str()
    };
    let terminal = escape_arg(&reference.canonical());
    let mut statement = format!(
        "{start} {caption} {end} \\app({start}, {end}, 640, 400, terminal: \"{terminal}\")\n"
    );
    let at = model.text().len();
    if at > 0 && !model.text().ends_with('\n') {
        statement.insert(0, '\n');
    }
    model.insert(at, &statement);
    Ok(())
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

/// K9 native rule artifact: run the embedded Austral module on a worker
/// thread (never on the Android UI thread) and return
/// `{"result": "complete"|"incomplete"|"malformed", ...}` or `{"error": …}`.
/// The artifact is fingerprint-checked and capability-free before it loads.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_australRule<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    facts_json: JString<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let json = (|| -> Result<serde_json::Value, String> {
                let parsed: serde_json::Value =
                    serde_json::from_str(&facts_json.to_string()).map_err(|e| e.to_string())?;
                let facts: Vec<austral_rule::LinkFact> = serde_json::from_value(
                    parsed
                        .get("facts")
                        .cloned()
                        .unwrap_or(serde_json::json!([])),
                )
                .map_err(|e| e.to_string())?;
                match austral_rule::evaluate_off_ui_thread(facts)? {
                    austral_rule::RuleResult::Complete { facts } => {
                        Ok(json!({ "result": "complete", "facts": facts }))
                    }
                    austral_rule::RuleResult::Incomplete { missing } => {
                        Ok(json!({ "result": "incomplete", "missing": missing }))
                    }
                    austral_rule::RuleResult::Malformed => Ok(json!({ "result": "malformed" })),
                }
            })();
            let value = match json {
                Ok(value) => value,
                Err(error) => json!({ "error": error }),
            };
            JString::from_str(env, value.to_string())
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
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let model = DocModel::from_snapshot(&bytes).ok()?;
    (model.text() == expected).then_some(model)
}

/// Canonical UTF-8 bytes for all rich-text content in a Loro snapshot.
///
/// Adjacent runs with identical attributes are coalesced and attribute names
/// are sorted. Every attribute (including `prop:*` and marker-link values) is
/// included. Loro operation history and view state are not.
fn canonical_content(snapshot: &[u8]) -> Option<Vec<u8>> {
    use loro::ToJson as _;
    let doc = loro::LoroDoc::new();
    doc.import(snapshot).ok()?;
    let text = doc.get_text("source");
    let value = text.get_richtext_value().to_json_value();
    let deltas = value.as_array()?;
    let mut runs: Vec<(String, serde_json::Map<String, Value>)> = Vec::new();
    for delta in deltas {
        let Some(map) = delta.as_object() else {
            continue;
        };
        let insert = map.get("insert").and_then(Value::as_str).unwrap_or("");
        let mut attrs = serde_json::Map::new();
        if let Some(original) = map.get("attributes").and_then(Value::as_object) {
            let mut keys: Vec<&String> = original.keys().collect();
            keys.sort();
            for key in keys {
                attrs.insert(key.clone(), original[key].clone());
            }
        }
        if let Some((last_text, last_attrs)) = runs.last_mut() {
            if *last_attrs == attrs {
                last_text.push_str(insert);
                continue;
            }
        }
        runs.push((insert.to_owned(), attrs));
    }
    let canonical: Vec<Value> = runs
        .into_iter()
        .map(|(insert, attributes)| {
            json!({
                "insert": insert,
                "attributes": Value::Object(attributes),
            })
        })
        .collect();
    serde_json::to_vec(&canonical).ok()
}

/// SHA-256 of [`canonical_content`], as lowercase hex.
fn logical_fingerprint(snapshot: &[u8]) -> Option<String> {
    use sha2::{Digest as _, Sha256};
    let canonical = canonical_content(snapshot)?;
    Some(format!("{:x}", Sha256::digest(canonical)))
}

fn search_ranges(text: &str, query: &str) -> Value {
    json!(mathed_core::search::find_matches(text, query)
        .into_iter()
        .map(|range| [
            byte_to_utf16(text, range.start),
            byte_to_utf16(text, range.end)
        ])
        .collect::<Vec<_>>())
}

fn protected_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let scan = mathed_core::markers::scan(text);
    let mut ranges: Vec<_> = scan
        .markers
        .iter()
        .map(|marker| marker.range.clone())
        .collect();
    ranges.extend(scan.stmts.iter().map(|stmt| stmt.range.clone()));
    ranges
}

fn intersects(ranges: &[std::ops::Range<usize>], range: &std::ops::Range<usize>) -> bool {
    ranges
        .iter()
        .any(|protected| protected.start < range.end && range.start < protected.end)
}

/// A bounded search page. Text scope omits structure; Source and Selection
/// return every match and mark which ones replacement must protect.
fn search_scoped(
    text: &str,
    query: &str,
    scope: &str,
    selection_start: usize,
    selection_end: usize,
    anchor: usize,
    page_start: i32,
    page_size: usize,
    include_all: bool,
) -> Value {
    let protected = protected_ranges(text);
    let search_range = if scope == "selection" {
        let start = utf16_to_byte(text, selection_start.min(selection_end));
        let end = utf16_to_byte(text, selection_start.max(selection_end));
        start..end
    } else {
        0..text.len()
    };
    let selection_scope = scope == "selection";
    let text_scope = scope != "source" && !selection_scope;
    let all: Vec<_> = mathed_core::search::find_matches_in(text, query, search_range)
        .into_iter()
        .filter(|range| !text_scope || !intersects(&protected, range))
        .collect();
    let anchor_byte = utf16_to_byte(text, anchor);
    let first = all
        .iter()
        .position(|range| range.start >= anchor_byte)
        .unwrap_or(0);
    let page_size = page_size.max(1).min(10_000);
    let page_start = if include_all {
        0
    } else if page_start >= 0 {
        (page_start as usize).min(all.len())
    } else {
        (first / page_size) * page_size
    };
    let end = if include_all {
        all.len().min(10_000)
    } else {
        (page_start + page_size).min(all.len())
    };
    let matches: Vec<Value> = all[page_start..end]
        .iter()
        .map(|range| {
            json!({
                "start": byte_to_utf16(text, range.start),
                "end": byte_to_utf16(text, range.end),
                "protected": intersects(&protected, range),
            })
        })
        .collect();
    json!({
        "total": all.len(),
        "replaceable": all.iter().filter(|range| !intersects(&protected, range)).count(),
        "protected": all.iter().filter(|range| intersects(&protected, range)).count(),
        "first": first,
        "pageStart": page_start,
        "matches": matches,
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_searchScoped<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    text: JString<'local>,
    query: JString<'local>,
    scope: JString<'local>,
    selection_start: jint,
    selection_end: jint,
    anchor: jint,
    page_start: jint,
    page_size: jint,
    include_all: jint,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let result = search_scoped(
                &text.to_string(),
                &query.to_string(),
                &scope.to_string(),
                selection_start.max(0) as usize,
                selection_end.max(0) as usize,
                anchor.max(0) as usize,
                page_start,
                page_size.max(1) as usize,
                include_all != 0,
            );
            JString::from_str(env, result.to_string())
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

/// Apply a checked batch of literal replacements. `expected` must still be
/// the document's exact text; every range is validated again against marker
/// and property ranges. `replace_many` makes the whole batch one Undo step.
fn replace_many_json(model: &mut DocModel, expected: &str, ops: &Value) -> Result<String, String> {
    if model.text() != expected {
        return Err("The document changed; refresh the replacement preview".into());
    }
    let entries = ops
        .as_array()
        .ok_or_else(|| "replacement plan is not an array".to_string())?;
    if entries.len() > 10_000 {
        return Err("replacement plan exceeds 10,000 operations".into());
    }
    let protected = protected_ranges(expected);
    let mut replacements = Vec::with_capacity(entries.len());
    for entry in entries {
        let start = entry
            .get("start")
            .and_then(Value::as_u64)
            .ok_or("missing start")? as usize;
        let end = entry
            .get("end")
            .and_then(Value::as_u64)
            .ok_or("missing end")? as usize;
        let with = entry
            .get("text")
            .and_then(Value::as_str)
            .ok_or("missing text")?;
        let range = utf16_to_byte(expected, start)..utf16_to_byte(expected, end);
        if range.start >= range.end
            || range.end > expected.len()
            || !expected.is_char_boundary(range.start)
            || !expected.is_char_boundary(range.end)
        {
            return Err("replacement range is invalid".into());
        }
        if intersects(&protected, &range) {
            return Err("Structural matches are not replaced".into());
        }
        replacements.push((range, with.to_string()));
    }
    replacements.sort_by_key(|(range, _)| range.start);
    for pair in replacements.windows(2) {
        if pair[1].0.start < pair[0].0.end {
            return Err("replacement ranges overlap".into());
        }
    }
    let first = replacements
        .first()
        .map(|(range, with)| (range.start, with.len()))
        .unwrap_or((0, 0));
    let ops: Vec<mathed_core::doc::ReplaceOp> = replacements
        .into_iter()
        .map(|(range, with)| mathed_core::doc::ReplaceOp { range, with })
        .collect();
    model.replace_many(ops);
    let caret = byte_to_utf16(model.text(), first.0.saturating_add(first.1));
    model.set_caret(utf16_to_grapheme_byte(model.text(), caret));
    Ok(state_json(model))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_replaceMany<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    expected: JString<'local>,
    ops: JString<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let expected = expected.to_string();
            let parsed: Value = match serde_json::from_str(&ops.to_string()) {
                Ok(value) => value,
                Err(error) => {
                    return Ok(JString::from_str(
                        env,
                        json!({"error": format!("invalid replacement plan: {error}")}).to_string(),
                    )?)
                }
            };
            // SAFETY: the handle is confined to the UI thread.
            let model = unsafe { model_mut(handle) };
            let result = match replace_many_json(model, &expected, &parsed) {
                Ok(state) => state,
                Err(message) => json!({"error": message}).to_string(),
            };
            JString::from_str(env, result)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_search<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    text: JString<'local>,
    query: JString<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            JString::from_str(
                env,
                search_ranges(&text.to_string(), &query.to_string()).to_string(),
            )
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
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

fn citation_source(
    text: &str,
    scan: &mathed_core::markers::MarkerScan,
    stmt_idx: usize,
) -> (String, usize, usize) {
    let span = &scan.stmts[stmt_idx].range;
    let mut snippet = text
        .get(span.clone())
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if snippet.chars().count() > 180 {
        snippet = snippet.chars().take(180).collect::<String>() + "…";
    }
    (
        snippet,
        byte_to_utf16(text, span.start),
        byte_to_utf16(text, span.end),
    )
}

fn citation_keys(reference: &mathed_core::markers::ReferenceEntry) -> Vec<String> {
    use mathed_core::markers::ReferenceKind;
    match &reference.kind {
        ReferenceKind::Bibliography { keys } => keys
            .iter()
            .map(|key| key.trim().trim_matches('"').to_string())
            .collect(),
        ReferenceKind::DocumentRef { .. } => Vec::new(),
    }
}

fn citation_index(text: &str, resolve_status: bool) -> Value {
    use mathed_core::markers::ReferenceKind;
    let scan = mathed_core::markers::scan(text);
    let upstream = mathed_core::markers::scan_references(&scan);
    let entries = model::citation_references(&scan);
    let mut result = Vec::new();
    for reference in &entries {
        let (source, start, end) = citation_source(text, &scan, reference.stmt_idx);
        let keys = citation_keys(reference);
        let kind = match reference.kind {
            ReferenceKind::Bibliography { .. } => "bibliography",
            ReferenceKind::DocumentRef { .. } => "document",
        };
        let mut resolved = true;
        let mut status_error = String::new();
        if resolve_status {
            match &reference.kind {
                ReferenceKind::Bibliography { .. } => {
                    if let Some(number) = reference.numbers.first() {
                        if let Err(error) = citations::bibliography_detail(
                            text,
                            *number,
                            None,
                            citations::DEFAULT_CITATION_LOCALE,
                        ) {
                            resolved = false;
                            status_error = error;
                        }
                    }
                }
                ReferenceKind::DocumentRef { body, .. } => {
                    let has_body = body.is_some();
                    let popup = upstream
                        .iter()
                        .find(|r| r.stmt_idx == reference.stmt_idx)
                        .and_then(|r| r.numbers.first())
                        .copied()
                        .and_then(|number| {
                            mathed_mini::cite_popup::resolve_popup_body(text, number)
                        });
                    if !has_body || popup.is_none() {
                        resolved = false;
                        status_error = "Citation has no referenced text".to_string();
                    }
                }
            }
        }
        result.push(json!({"start": start, "end": end,
            "numbers": reference.numbers, "label": mathed_core::markers::cite_label_text(reference),
            "kind": kind, "keys": keys, "source": source, "resolved": resolved,
            "statusError": status_error}));
    }
    json!(result)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_citationIndex<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    text: JString<'local>,
    resolve_status: jboolean,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            JString::from_str(
                env,
                citation_index(&text.to_string(), resolve_status).to_string(),
            )
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_citationChoices<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            JString::from_str(
                env,
                json!({
                    "styles": citations::CITATION_STYLES,
                    "locales": citations::CITATION_LOCALES,
                    "defaultStyle": mathed_biblio::DEFAULT_STYLE,
                    "defaultLocale": citations::DEFAULT_CITATION_LOCALE,
                })
                .to_string(),
            )
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

fn citation_detail(
    text: &str,
    number: u64,
    width: f64,
    scale: f32,
    style: Option<&str>,
    locale: &str,
) -> Value {
    use base64::Engine as _;
    use mathed_mini::cite_popup::{resolve_popup_body, PopupBody};
    let scan = mathed_core::markers::scan(text);
    let references = model::citation_references(&scan);
    let Some(reference) = references.iter().find(|r| r.numbers.contains(&number)) else {
        return json!({"error": "Citation no longer exists"});
    };
    let (source, source_start, source_end) = citation_source(text, &scan, reference.stmt_idx);
    let label = mathed_core::markers::cite_label_text(reference);
    let keys = citation_keys(reference);
    let metadata = json!({"number": number, "label": label, "keys": keys,
        "source": source, "sourceStart": source_start, "sourceEnd": source_end});
    let upstream = mathed_core::markers::scan_references(&scan);
    let upstream_number = upstream
        .iter()
        .find(|r| r.stmt_idx == reference.stmt_idx)
        .and_then(|r| r.numbers.first())
        .copied()
        .unwrap_or(number);
    let Some(body) = resolve_popup_body(text, upstream_number) else {
        return json!({"error": "Citation no longer exists", "source": source,
            "sourceStart": source_start, "sourceEnd": source_end});
    };
    let locale = if locale.trim().is_empty() {
        citations::DEFAULT_CITATION_LOCALE
    } else {
        locale
    };
    let style = style.map(str::trim).filter(|name| !name.is_empty());
    match body {
        PopupBody::Bibliography { keys, .. } => {
            match citations::bibliography_detail(text, number, style, locale) {
                Ok((citation, entries)) => json!({"kind":"bibliography", "text":citation,
                    "entries":entries, "resolved":true, "style": style.unwrap_or(mathed_biblio::DEFAULT_STYLE),
                    "locale": locale, "metadata": metadata}),
                Err(error) => {
                    json!({"kind":"bibliography", "text":format!("[{}]", keys.join(", ")),
                    "resolved":false, "errorDetail":error, "style": style.unwrap_or(mathed_biblio::DEFAULT_STYLE),
                    "locale": locale, "metadata": metadata})
                }
            }
        }
        PopupBody::DocumentRef {
            body_text,
            body_markup,
        } => {
            let body_markup = if body_text.is_empty() {
                body_markup
            } else {
                let scan = mathed_core::markers::scan(&body_text);
                let segments = mathed_core::markers::resolve_segments(&scan);
                let options = mathed_core::transform::TransformOptions {
                    references: model::citation_references(&scan),
                    ..Default::default()
                };
                mathed_core::transform::to_render_text(&body_text, &scan, &segments, &options).text
            };
            let world = mathed_mini::MiniWorld::new(format!("{DOCUMENT_THEME}{body_markup}"));
            let image = world
                .eval_main()
                .and_then(|content| {
                    use typst::layout::{Abs, Axes, Region, Size};
                    world.layout(
                        &content,
                        Region::new(
                            Size::new(Abs::pt(width), Abs::pt(2048.0)),
                            Axes::splat(false),
                        ),
                    )
                })
                .and_then(|frame| mathed_mini::rasterize_at(&frame, scale).ok());
            match image {
                Some(image) => match png_page(image.width, image.height, &image.data) {
                    Ok(png) => json!({"kind":"document", "text":body_text, "scope":body_text,
                        "body":body_text, "width":image.width, "height":image.height,
                        "png":base64::engine::general_purpose::STANDARD.encode(png),
                        "resolved":true, "locale": locale, "metadata": metadata}),
                    Err(error) => json!({"error":error, "metadata": metadata}),
                },
                None => json!({"error":"Could not render cited text", "text":body_text,
                    "metadata": metadata}),
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_citationDetail<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    text: JString<'local>,
    number: jint,
    width: jfloat,
    scale: jfloat,
    style: JString<'local>,
    locale: JString<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let width = if width.is_finite() {
                (width as f64).clamp(60.0, 1200.0)
            } else {
                250.0
            };
            let scale = if scale.is_finite() {
                scale.clamp(1.0, 3.0)
            } else {
                1.0
            };
            let style = style.to_string();
            let locale = locale.to_string();
            JString::from_str(
                env,
                citation_detail(
                    &text.to_string(),
                    number.max(0) as u64,
                    width,
                    scale,
                    (!style.is_empty()).then_some(style.as_str()),
                    &locale,
                )
                .to_string(),
            )
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_semanticIndex<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    text: JString<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            JString::from_str(env, semantics::semantic_json(&text.to_string()).to_string())
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_renamePreview<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    text: JString<'local>,
    def_index: jint,
    new_name: JString<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let result = semantics::rename_preview_json(
                &text.to_string(),
                def_index.max(0) as usize,
                &new_name.to_string(),
            );
            JString::from_str(env, result.to_string())
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_renameDefinition<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    expected: JString<'local>,
    def_index: jint,
    new_name: JString<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            // SAFETY: the handle is confined to the UI thread.
            let model = unsafe { model_mut(handle) };
            let result = match semantics::rename_definition_json(
                model,
                &expected.to_string(),
                def_index.max(0) as usize,
                &new_name.to_string(),
            ) {
                Ok(state) => state,
                Err(message) => json!({"error": message}).to_string(),
            };
            JString::from_str(env, result)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_mathCompletion<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    text: JString<'local>,
    caret: jint,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            JString::from_str(
                env,
                math_completion(&text.to_string(), caret.max(0) as usize).to_string(),
            )
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_graphemeBoundary<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    text: JString<'local>,
    offset: jint,
    direction: jint,
) -> jint {
    unowned_env
        .with_env(|_env| -> errors::Result<jint> {
            Ok(
                grapheme_boundary_utf16(&text.to_string(), offset.max(0) as usize, direction)
                    as jint,
            )
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_wordBoundary<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    text: JString<'local>,
    caret: jint,
    direction: jint,
) -> jint {
    unowned_env
        .with_env(|_env| -> errors::Result<jint> {
            Ok(word_boundary_utf16(&text.to_string(), caret.max(0) as usize, direction) as jint)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

const DOCUMENT_THEME: &str = "#set text(fill: white, size: 17pt, font: \"DejaVu Sans Mono\", kerning: false, bottom-edge: \"descender\", ligatures: false)\n";

fn modifier_fragment(
    model: &DocModel,
    range: std::ops::Range<usize>,
    excluded: Option<(&str, &str)>,
    width: f64,
    scale: f32,
) -> Value {
    use base64::Engine as _;
    use unicode_segmentation::UnicodeSegmentation;
    let mut boundaries: Vec<usize> = model
        .text()
        .grapheme_indices(true)
        .map(|(i, _)| i)
        .collect();
    boundaries.push(model.len());
    let start = boundaries
        .iter()
        .copied()
        .take_while(|i| *i <= range.start)
        .last()
        .unwrap_or(0);
    let end = boundaries
        .iter()
        .copied()
        .find(|i| *i >= range.end)
        .unwrap_or(model.len());
    let range = start..end;
    let scan = mathed_core::markers::scan(model.text());
    let segments = mathed_core::markers::resolve_segments(&scan);
    let options = mathed_core::transform::TransformOptions {
        references: model::citation_references(&scan),
        block_splices: modifiers::typst_splices_in(model, range.clone(), excluded),
        ..Default::default()
    };
    let fragment = mathed_core::transform::to_render_text_range(
        model.text(),
        &scan,
        &segments,
        range,
        &options,
    );
    // Match mathed_mini's document theme and rasterize at device density;
    // enlarging a 1 px/pt snippet makes its glyphs visibly pixelated.
    let markup = format!("{DOCUMENT_THEME}{}", fragment.text);
    let world = mathed_mini::MiniWorld::new(markup);
    let image = world
        .eval_main()
        .and_then(|content| {
            use typst::layout::{Abs, Axes, Region, Size};
            let region = Region::new(
                Size::new(Abs::pt(width), Abs::pt(2048.0)),
                Axes::splat(false),
            );
            world.layout(&content, region)
        })
        .and_then(|frame| mathed_mini::rasterize_at(&frame, scale).ok());
    match image {
        Some(image) => json!({"width": image.width, "height": image.height,
            "rgba": base64::engine::general_purpose::STANDARD.encode(&image.data)}),
        None => json!({"error": "Could not render comparison"}),
    }
}

fn modifier_comparisons(
    model: &DocModel,
    start_id: &str,
    end_id: &str,
    width: f64,
    scale: f32,
) -> Value {
    let Some(source) = modifiers::source_range(model, start_id, end_id) else {
        return json!({"error": "Modifier markers no longer exist"});
    };
    let mut incoming = Vec::new();
    let mut outgoing = Vec::new();
    for link in modifiers::links(model) {
        if !link.resolved {
            continue;
        }
        let source_start = modifiers::source_range(model, &link.start_id, &link.end_id);
        if link.target.start < source.end && source.start < link.target.end {
            let label = source_start
                .as_ref()
                .map(|r| model.text()[r.clone()].trim().to_owned())
                .unwrap_or_else(|| "Missing modifier".to_string());
            incoming.push(json!({"label": label,
                "startId": link.start_id, "endId": link.end_id,
                "targetIds": link.target_ids,
                "start": byte_to_utf16(model.text(), link.target.start),
                "end": byte_to_utf16(model.text(), link.target.end),
                "sourceStart": source_start.as_ref().map(|r| byte_to_utf16(model.text(), r.start)),
                "sourceEnd": source_start.as_ref().map(|r| byte_to_utf16(model.text(), r.end)),
                "snippet": snippet_utf16(model.text(), link.target.clone()),
                "legacy": link.target_ids.is_none(),
                "resolved": link.resolved,
                "without": modifier_fragment(model, source.clone(), Some((&link.start_id, &link.end_id)), width, scale)}));
        }
        if link.start_id == start_id && link.end_id == end_id {
            outgoing.push(json!({
                "start": byte_to_utf16(model.text(), link.target.start),
                "end": byte_to_utf16(model.text(), link.target.end),
                "targetIds": link.target_ids,
                "snippet": snippet_utf16(model.text(), link.target.clone()),
                "legacy": link.target_ids.is_none(),
                "resolved": link.resolved,
                "with": modifier_fragment(model, link.target.clone(), None, width, scale),
                "without": modifier_fragment(model, link.target, Some((start_id, end_id)), width, scale),
            }));
        }
    }
    json!({"incoming": incoming, "outgoing": outgoing})
}

fn modifier_relationships(model: &DocModel, start_id: &str, end_id: &str) -> Value {
    let Some(source) = modifiers::source_range(model, start_id, end_id) else {
        return json!({"error": "Modifier markers no longer exist"});
    };
    let mut incoming = Vec::new();
    let mut outgoing = Vec::new();
    for link in modifiers::links(model) {
        let source_start = modifiers::source_range(model, &link.start_id, &link.end_id);
        if link.target.start < source.end && source.start < link.target.end {
            let label = source_start
                .as_ref()
                .map(|r| model.text()[r.clone()].trim().to_owned())
                .unwrap_or_else(|| "Missing modifier".to_string());
            incoming.push(json!({"label": label,
                "startId": link.start_id, "endId": link.end_id,
                "targetIds": link.target_ids,
                "start": byte_to_utf16(model.text(), link.target.start),
                "end": byte_to_utf16(model.text(), link.target.end),
                "sourceStart": source_start.as_ref().map(|r| byte_to_utf16(model.text(), r.start)),
                "sourceEnd": source_start.as_ref().map(|r| byte_to_utf16(model.text(), r.end)),
                "snippet": snippet_utf16(model.text(), link.target.clone()),
                "legacy": link.target_ids.is_none(),
                "resolved": link.resolved,
                "error": if link.resolved { "" } else { "Modifier source markers no longer resolve" }}));
        }
        if link.start_id == start_id && link.end_id == end_id {
            outgoing.push(json!({
                "start": byte_to_utf16(model.text(), link.target.start),
                "end": byte_to_utf16(model.text(), link.target.end),
                "targetIds": link.target_ids,
                "snippet": snippet_utf16(model.text(), link.target.clone()),
                "legacy": link.target_ids.is_none(),
                "resolved": link.resolved,
                "error": if link.resolved { "" } else { "Target markers no longer resolve" },
                "sourceStart": byte_to_utf16(model.text(), source.start),
                "sourceEnd": byte_to_utf16(model.text(), source.end),
                "label": model.text()[source.clone()].trim().to_owned(),
            }));
        }
    }
    json!({"incoming": incoming, "outgoing": outgoing})
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_modifierRelationships<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    text: JString<'local>,
    snapshot: JString<'local>,
    start_id: JString<'local>,
    end_id: JString<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let text = text.to_string();
            let value = match model_from_snapshot(&snapshot.to_string(), &text) {
                Some(model) => {
                    modifier_relationships(&model, &start_id.to_string(), &end_id.to_string())
                }
                None => json!({"error": "Could not restore document"}),
            };
            JString::from_str(env, value.to_string())
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

fn snippet_utf16(text: &str, range: std::ops::Range<usize>) -> String {
    let mut value = text
        .get(range)
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if value.chars().count() > 80 {
        value = value.chars().take(80).collect::<String>() + "…";
    }
    value
}

/// One expanded relationship row. Rendering is requested only for the row
/// the user opened, never for every incoming/outgoing relationship.
fn modifier_comparison_row(
    model: &DocModel,
    target_start: i32,
    target_end: i32,
    exclude_start: &str,
    exclude_end: &str,
    width: f64,
    scale: f32,
) -> Value {
    let text = model.text();
    let start = utf16_to_byte(text, target_start.max(0) as usize);
    let end = utf16_to_byte(text, target_end.max(0) as usize);
    if start >= end
        || end > text.len()
        || !text.is_char_boundary(start)
        || !text.is_char_boundary(end)
    {
        return json!({"error": "This relationship has no visible text"});
    }
    let excluded = (!exclude_start.is_empty() && !exclude_end.is_empty())
        .then_some((exclude_start, exclude_end));
    json!({
        "with": modifier_fragment(model, start..end, None, width, scale),
        "without": modifier_fragment(model, start..end, excluded, width, scale),
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_modifierComparisonRow<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    text: JString<'local>,
    snapshot: JString<'local>,
    target_start: jint,
    target_end: jint,
    exclude_start: JString<'local>,
    exclude_end: JString<'local>,
    width: jfloat,
    scale: jfloat,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let text = text.to_string();
            let value = match model_from_snapshot(&snapshot.to_string(), &text) {
                Some(model) => modifier_comparison_row(
                    &model,
                    target_start,
                    target_end,
                    &exclude_start.to_string(),
                    &exclude_end.to_string(),
                    if width.is_finite() {
                        (width as f64).clamp(60.0, 1200.0)
                    } else {
                        250.0
                    },
                    if scale.is_finite() {
                        scale.clamp(1.0, 3.0)
                    } else {
                        1.0
                    },
                ),
                None => json!({"error": "Could not restore document"}),
            };
            JString::from_str(env, value.to_string())
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_modifierComparisons<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    text: JString<'local>,
    snapshot: JString<'local>,
    start_id: JString<'local>,
    end_id: JString<'local>,
    width: jfloat,
    scale: jfloat,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let text = text.to_string();
            let value = match model_from_snapshot(&snapshot.to_string(), &text) {
                Some(model) => modifier_comparisons(
                    &model,
                    &start_id.to_string(),
                    &end_id.to_string(),
                    if width.is_finite() {
                        (width as f64).clamp(60.0, 1200.0)
                    } else {
                        250.0
                    },
                    if scale.is_finite() {
                        scale.clamp(1.0, 3.0)
                    } else {
                        1.0
                    },
                ),
                None => json!({"error": "Could not restore document"}),
            };
            JString::from_str(env, value.to_string())
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
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
            Ok(
                model_from_snapshot(&snapshot.to_string(), &expected.to_string())
                    .map_or(0, |m| Box::into_raw(Box::new(m)) as jlong),
            )
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_openSnapshot<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    snapshot: JString<'local>,
) -> jlong {
    use base64::Engine as _;
    unowned_env
        .with_env(|_env| -> errors::Result<jlong> {
            let model = base64::engine::general_purpose::STANDARD
                .decode(snapshot.to_string())
                .ok()
                .and_then(|bytes| DocModel::from_snapshot(&bytes).ok());
            Ok(model.map_or(0, |m| Box::into_raw(Box::new(m)) as jlong))
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
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

/// Capture a stable Loro cursor for a UTF-16 offset. The caller stores the
/// document identity alongside the encoded position and rejects it elsewhere.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_createAnchor<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    offset_utf16: jint,
    side: jint,
) -> JString<'local> {
    use base64::Engine as _;
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let model = unsafe { model_mut(handle) };
            let byte = utf16_to_byte(model.text(), offset_utf16.max(0) as usize);
            let value = match model.math_doc().create_anchor(byte, side) {
                Some(anchor) => json!({
                    "encoded": base64::engine::general_purpose::STANDARD.encode(anchor.encoded),
                    "side": anchor.side,
                    "fallback": byte_to_utf16(model.text(), anchor.fallback),
                }),
                None => json!({"error": "Could not create a stable position"}),
            };
            JString::from_str(env, value.to_string())
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

/// Resolve a stable Loro cursor to a grapheme-snapped UTF-16 offset, or -1 for
/// an anchor that does not belong to this document.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_resolveAnchor<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    encoded: JString<'local>,
    side: jint,
    fallback_utf16: jint,
) -> jint {
    use base64::Engine as _;
    unowned_env
        .with_env(|_env| -> errors::Result<jint> {
            let model = unsafe { model_mut(handle) };
            let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded.to_string())
            else {
                return Ok(-1);
            };
            let fallback = utf16_to_byte(model.text(), fallback_utf16.max(0) as usize);
            let resolved = model.math_doc().resolve_anchor(&bytes, fallback);
            let boundary = if side > 0 {
                grapheme_end(model.text(), resolved)
            } else {
                grapheme_start(model.text(), resolved)
            };
            Ok(byte_to_utf16(model.text(), boundary) as jint)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

/// Capture the live document's snapshot as bytes for recovery/render work.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_snapshotBytes<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JByteArray<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JByteArray<'local>> {
            // SAFETY: the handle is owned by the UI thread until onDestroy.
            let bytes = unsafe { model_mut(handle).math_doc().snapshot() };
            let array = JByteArray::new(env, bytes.len())?;
            array.set_region(
                env,
                0,
                bytes
                    .iter()
                    .map(|b| *b as jbyte)
                    .collect::<Vec<_>>()
                    .as_slice(),
            )?;
            Ok(array)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

/// Open a Loro snapshot without a separate text assertion. Zero on invalid data.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_openSnapshotBytes<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    snapshot: JByteArray<'local>,
) -> jlong {
    unowned_env
        .with_env(|env| -> errors::Result<jlong> {
            let bytes = env.convert_byte_array(&snapshot)?;
            let model = DocModel::from_snapshot(&bytes).ok();
            Ok(model.map_or(0, |m| Box::into_raw(Box::new(m)) as jlong))
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

/// SHA-256 of canonical rich-text content, safe to call on a worker.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_logicalFingerprint<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    snapshot: JByteArray<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let bytes = env.convert_byte_array(&snapshot)?;
            let fingerprint = logical_fingerprint(&bytes).unwrap_or_default();
            JString::from_str(env, fingerprint)
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
pub extern "system" fn Java_com_emthin_pad_DocBridge_terminalFigures<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    text: JString<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let json = terminal_figure_options(&text.to_string());
            JString::from_str(env, json)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_rebindTerminalFigure<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    expected: JString<'local>,
    key: JString<'local>,
    reference: JString<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let expected = expected.to_string();
            let key = key.to_string();
            let reference = reference.to_string();
            // SAFETY: handle invariants documented on model_mut.
            let json = unsafe {
                let m = model_mut(handle);
                match rebind_terminal_figure(m, &expected, &key, &reference) {
                    Ok(()) => state_json(m),
                    Err(error) => json!({ "error": error }).to_string(),
                }
            };
            JString::from_str(env, json)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_appendTerminalFigure<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    expected: JString<'local>,
    reference: JString<'local>,
    caption: JString<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let expected = expected.to_string();
            let reference = reference.to_string();
            let caption = caption.to_string();
            // SAFETY: handle invariants documented on model_mut.
            let json = unsafe {
                let m = model_mut(handle);
                match append_terminal_figure(m, &expected, &reference, &caption) {
                    Ok(()) => state_json(m),
                    Err(error) => json!({ "error": error }).to_string(),
                }
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
    text: &str,
    opts: &mathed_core::transform::TransformOptions,
    scale: f32,
    start: usize,
    max_pages: usize,
) -> Result<(usize, Vec<mathed_mini::PageLayout>), mathed_mini::RenderError> {
    let render = mathed_mini::doc_to_render_with(text, opts);
    let world = mathed_mini::MiniWorld::new(format!("{DOCUMENT_THEME}{}", render.text));
    let document = typst::compile::<typst_layout::PagedDocument>(&world)
        .output
        .map_err(|_| mathed_mini::RenderError::Eval)?;
    let total = document.pages().len();
    let pages = document
        .pages()
        .iter()
        .skip(start)
        .take(max_pages.clamp(1, MAX_CHUNK_PAGES))
        .map(|page| {
            let glyphs = mathed_core::glyphs::build_glyph_index(
                &page.frame,
                world.main_source(),
                &render.map,
                DOCUMENT_THEME.len(),
            );
            let figures = mathed_core::figures::figures_in_frame(&page.frame);
            let image = mathed_mini::rasterize_at(&page.frame, scale)?;
            let (width, height) = (image.width, image.height);
            Ok(mathed_mini::PageLayout {
                image,
                glyphs,
                figures,
                width,
                height,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
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
        writer
            .write_image_data(&straight)
            .map_err(|e| e.to_string())?;
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
    render_pages_with_options(
        doc_text,
        start,
        max_pages,
        scale,
        caret_utf16,
        model,
        false,
        false,
    )
}

fn marker_reveal_ranges(
    scan: &mathed_core::markers::MarkerScan,
    caret: usize,
    show: bool,
) -> Vec<std::ops::Range<usize>> {
    let on_marker = scan
        .markers
        .iter()
        .any(|m| m.range.start <= caret && caret <= m.range.end);
    let mut reveal = if !show && on_marker {
        Vec::new()
    } else {
        vec![caret..caret]
    };
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
    render_pages_with_options_expanded(
        doc_text,
        start,
        max_pages,
        scale,
        caret_utf16,
        model,
        show_markers,
        rendered_only,
        &[],
    )
}

fn render_pages_with_options_expanded(
    doc_text: &str,
    start: usize,
    max_pages: usize,
    scale: f32,
    caret_utf16: usize,
    model: Option<&DocModel>,
    show_markers: bool,
    rendered_only: bool,
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
        segments
            .iter()
            .filter(|s| {
                expanded_pairs
                    .iter()
                    .any(|(a, b)| &s.start_id == a && &s.end_id == b)
            })
            .filter_map(|s| s.span.clone())
            .collect()
    } else {
        mathed_mini::render::reveal_span_in(&scan, &segments, caret)
            .into_iter()
            .collect()
    };
    // A caret on a marker normally reveals that token in mathed. This
    // frontend's explicit marker toggle promises hidden markers even at
    // the caret, while still revealing other token kinds as before.
    let reveal = if rendered_only {
        if show_markers {
            scan.markers.iter().map(|m| m.range.clone()).collect()
        } else {
            Vec::new()
        }
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
                .into_iter()
                .map(|link| (link.start_id, link.end_id))
                .collect();
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
                opts.block_splices =
                    modifiers::typst_splices_excluding(model, 0..model.len(), &excluded);
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
            let bytes = if rendered_only {
                png_page(p.image.width, p.image.height, &p.image.data)?
            } else {
                p.image.data.clone()
            };
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
            let hits: Vec<Value> = p
                .glyphs
                .entries
                .iter()
                .filter_map(|e| {
                    let band = p.glyphs.bands.get(e.band as usize)?;
                    Some(json!([
                        e.x,
                        band.top,
                        e.x + e.advance,
                        band.bottom,
                        byte_to_utf16(doc_text, e.doc_byte)
                    ]))
                })
                .collect();
            let bindings = crate::layout::figure_bindings(doc_text);
            let figures: Vec<Value> = p
                .figures
                .iter()
                .map(|figure| {
                    let (reference, binding_error) =
                        bindings.get(&figure.key).cloned().unwrap_or((None, None));
                    json!({
                        "key": figure.key,
                        "x": figure.rect.x0,
                        "y": figure.rect.y0,
                        "w": figure.rect.x1 - figure.rect.x0,
                        "h": figure.rect.y1 - figure.rect.y0,
                        "reference": reference,
                        "bindingError": binding_error,
                    })
                })
                .collect();
            let mut result = json!({
                "width": p.image.width,
                "height": p.image.height,
                "textTop": text_top,
                "hasFigures": !figures.is_empty(),
                "figures": figures,
                "hits": hits,
            });
            result[if rendered_only { "png" } else { "rgba" }] = json!(b64);
            Ok(result)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({ "total": total, "start": start, "pages": chunk, "warnings": warnings }).to_string())
}

/// Compile or reuse the one worker-retained layout for this capture and
/// presentation. Safe off the UI thread; it imports captured snapshot bytes
/// and never accesses the live model handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_prepareLayout<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    snapshot: JByteArray<'local>,
    doc_text: JString<'local>,
    layout_id: JString<'local>,
    show_markers: jint,
    expanded: JString<'local>,
    text_size: jfloat,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let bytes = env.convert_byte_array(&snapshot)?;
            let text = doc_text.to_string();
            let expanded: Vec<(String, String)> =
                serde_json::from_str(&expanded.to_string()).unwrap_or_default();
            let result = layout::prepare(
                &bytes,
                &text,
                &layout_id.to_string(),
                show_markers != 0,
                &expanded,
                text_size as f32,
            )
            .unwrap_or_else(|error| error);
            JString::from_str(env, result.to_string())
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

/// Rasterize one page from the retained layout at the requested density.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_renderLayoutPage<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    layout_id: JString<'local>,
    doc_text: JString<'local>,
    page: jint,
    scale: jfloat,
    pixel_limit: jlong,
    retain_from: jint,
    retain_to: jint,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let result = layout::render_page(
                &layout_id.to_string(),
                &doc_text.to_string(),
                page.max(0) as usize,
                scale as f32,
                pixel_limit.max(1) as u64,
                retain_from.max(0) as usize,
                retain_to.max(0) as usize,
            )
            .unwrap_or_else(|error| json!({"error": error}));
            JString::from_str(env, result.to_string())
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

/// Locate a UTF-16 source range in the retained layout without rasterizing.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_locateLayoutRange<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    layout_id: JString<'local>,
    doc_text: JString<'local>,
    start_utf16: jint,
    end_utf16: jint,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let result = layout::locate_range(
                &layout_id.to_string(),
                &doc_text.to_string(),
                start_utf16.max(0) as usize,
                end_utf16.max(0) as usize,
            )
            .unwrap_or_else(|error| json!({"error": error}));
            JString::from_str(env, result.to_string())
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
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
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let text = doc_text.to_string();
            let model = model_from_snapshot(&snapshot.to_string(), &text);
            let expanded: Vec<(String, String)> =
                serde_json::from_str(&expanded.to_string()).unwrap_or_default();
            let result = match model {
                Some(model) => render_pages_with_options_expanded(
                    &text,
                    start.max(0) as usize,
                    max_pages.max(0) as usize,
                    scale as f32,
                    caret_utf16.max(0) as usize,
                    Some(&model),
                    show_markers != 0,
                    true,
                    &expanded,
                ),
                None => Err("Could not restore document for preview".to_string()),
            };
            JString::from_str(
                env,
                result.unwrap_or_else(|e| json!({"error": e}).to_string()),
            )
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
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

// ----- native terminal scene -----------------------------------------

/// # Safety
/// `handle` must come from `NativeTerminal.create` and be confined to one
/// Android thread until `close`.
unsafe fn scene_mut<'a>(handle: jlong) -> &'a mut scene::Scene {
    unsafe { &mut *(handle as *mut scene::Scene) }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_NativeTerminal_nativeCreate<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    cols: jint,
    rows: jint,
    cell_width: jint,
    cell_height: jint,
    baseline: jint,
    echo: jboolean,
) -> jlong {
    unowned_env
        .with_env(|_env| -> errors::Result<jlong> {
            let scene = scene::Scene::new(
                cols.max(1) as u16,
                rows.max(1) as u16,
                cell_width.max(1) as u16,
                cell_height.max(1) as u16,
                baseline.max(0) as u16,
                echo,
            );
            Ok(Box::into_raw(Box::new(scene)) as jlong)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_NativeTerminal_nativeFeed<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    bytes: JByteArray<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let bytes = env.convert_byte_array(&bytes)?;
            let scene = unsafe { scene_mut(handle) };
            scene.feed(&bytes);
            JString::from_str(env, scene.state())
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_NativeTerminal_nativeState<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            JString::from_str(env, unsafe { scene_mut(handle) }.state())
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_NativeTerminal_nativeRgba<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JByteArray<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JByteArray<'local>> {
            let pixels = unsafe { scene_mut(handle) }.rgba();
            let array = JByteArray::new(env, pixels.len())?;
            array.set_region(
                env,
                0,
                pixels
                    .iter()
                    .map(|b| *b as jbyte)
                    .collect::<Vec<_>>()
                    .as_slice(),
            )?;
            Ok(array)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_NativeTerminal_nativeResize<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    cols: jint,
    rows: jint,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let scene = unsafe { scene_mut(handle) };
            scene.resize(cols.max(1) as u16, rows.max(1) as u16);
            JString::from_str(env, scene.state())
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_NativeTerminal_nativeKey<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    key: jint,
    mods: jint,
    text: JString<'local>,
    unshifted: jint,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let text = text.to_string();
            let scene = unsafe { scene_mut(handle) };
            scene.key(key, mods as u16, &text, unshifted as u32);
            JString::from_str(env, scene.state())
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_NativeTerminal_nativeText<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    text: JString<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let text = text.to_string();
            let scene = unsafe { scene_mut(handle) };
            scene.text(&text);
            JString::from_str(env, scene.state())
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_NativeTerminal_nativeMouse<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    x: jint,
    y: jint,
    button: jint,
    press: jboolean,
    mods: jint,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let scene = unsafe { scene_mut(handle) };
            scene.mouse(
                x.max(0) as u16,
                y.max(0) as u16,
                button.clamp(0, 3) as u8,
                press,
                mods.clamp(0, 7) as u8,
            );
            JString::from_str(env, scene.state())
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_NativeTerminal_nativeMouseMotion<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    x: jint,
    y: jint,
    button: jint,
    mods: jint,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let scene = unsafe { scene_mut(handle) };
            let button = (button >= 0).then_some(button.clamp(0, 3) as u8);
            scene.mouse_motion(
                x.max(0) as u16,
                y.max(0) as u16,
                button,
                mods.clamp(0, 7) as u8,
            );
            JString::from_str(env, scene.state())
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_NativeTerminal_nativeSelection<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    x0: jint,
    y0: jint,
    x1: jint,
    y1: jint,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let scene = unsafe { scene_mut(handle) };
            let text = scene.selection(
                x0.max(0) as u16,
                y0.max(0) as u16,
                x1.max(0) as u16,
                y1.max(0) as u16,
            );
            JString::from_str(env, text)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_NativeTerminal_nativeWordSelection<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    x: jint,
    y: jint,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let scene = unsafe { scene_mut(handle) };
            let bounds = scene
                .word_at(x.max(0) as u16, y.max(0) as u16)
                .unwrap_or((0, 0, 0, 0));
            JString::from_str(
                env,
                json!({
                    "x0": bounds.0,
                    "y0": bounds.1,
                    "x1": bounds.2,
                    "y1": bounds.3,
                })
                .to_string(),
            )
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_NativeTerminal_nativeRecreate<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let scene = unsafe { scene_mut(handle) };
            scene.recreate();
            JString::from_str(env, scene.state())
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_NativeTerminal_nativeClose(
    _env: EnvUnowned,
    _class: JClass,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: the Java wrapper calls close exactly once.
        drop(unsafe { Box::from_raw(handle as *mut scene::Scene) });
    }
}

// ----- native document sharing (K6) ------------------------------------

fn hex_decode(value: &str) -> Option<Vec<u8>> {
    let bytes = value.as_bytes();
    if bytes.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

/// Start a sharing worker from a snapshot. The worker owns its transport-side
/// replica; the returned handle is the UI-thread command channel only.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_shareOpen<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    snapshot: JByteArray<'local>,
    peer: JString<'local>,
    name: JString<'local>,
) -> jlong {
    unowned_env
        .with_env(|env| -> errors::Result<jlong> {
            let bytes = env.convert_byte_array(&snapshot)?;
            let peer = peer.to_string();
            let name = name.to_string();
            let handle = match sharing::ShareHandle::start(&bytes, peer, name) {
                Ok(handle) => handle,
                Err(_) => return Ok(0),
            };
            Ok(Box::into_raw(Box::new(handle)) as jlong)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

/// SAFETY: the Java wrapper owns this handle and closes it exactly once.
unsafe fn share_handle_mut<'a>(handle: jlong) -> &'a mut sharing::ShareHandle {
    unsafe { &mut *(handle as *mut sharing::ShareHandle) }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_shareClose(
    _env: EnvUnowned,
    _class: JClass,
    handle: jlong,
) {
    if handle != 0 {
        unsafe {
            drop(Box::from_raw(handle as *mut sharing::ShareHandle));
        }
    }
}

fn share_json_result<'local>(
    env: &mut jni::Env<'local>,
    result: Result<(), String>,
) -> errors::Result<JString<'local>> {
    let json = match result {
        Ok(()) => "{\"ok\":true}".to_string(),
        Err(message) => serde_json::json!({"ok": false, "error": message}).to_string(),
    };
    JString::from_str(env, json)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_shareJoin<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    peer: JString<'local>,
    version: JString<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let peer = peer.to_string();
            let version = hex_decode(&version.to_string()).unwrap_or_default();
            // SAFETY: UI-thread-owned handle.
            let result = unsafe { share_handle_mut(handle) }.join(peer, version);
            share_json_result(env, result)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_sharePublish<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    delta: JString<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let delta = hex_decode(&delta.to_string()).unwrap_or_default();
            // SAFETY: UI-thread-owned handle.
            let result = unsafe { share_handle_mut(handle) }.publish(delta);
            share_json_result(env, result)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_shareReceive<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    peer: JString<'local>,
    seq: jlong,
    delta: JString<'local>,
    scope: jlong,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let peer = peer.to_string();
            let delta = hex_decode(&delta.to_string()).unwrap_or_default();
            let scope = (scope >= 0).then_some(scope as u64);
            // SAFETY: UI-thread-owned handle.
            let result =
                unsafe { share_handle_mut(handle) }.receive(peer, seq as u64, delta, scope);
            share_json_result(env, result)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_shareLeave<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            // SAFETY: UI-thread-owned handle.
            let result = unsafe { share_handle_mut(handle) }.leave();
            share_json_result(env, result)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_sharePoll<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            // SAFETY: UI-thread-owned handle.
            let json = unsafe { share_handle_mut(handle) }.poll_json();
            JString::from_str(env, json)
        })
        .resolve::<errors::ThrowRuntimeExAndDefault>()
}

/// Apply an accepted remote delta to the live `DocModel` as one grouped edit
/// on the UI thread. The transport worker never touches this handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_emthin_pad_DocBridge_shareApply<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    delta: JString<'local>,
) -> JString<'local> {
    unowned_env
        .with_env(|env| -> errors::Result<JString<'local>> {
            let delta = hex_decode(&delta.to_string()).unwrap_or_default();
            // SAFETY: UI-thread-owned handle.
            let json = unsafe {
                let m = model_mut(handle);
                match m.math_doc_mut().import_delta(&delta) {
                    Ok(()) => {
                        m.rescan();
                        state_json(m)
                    }
                    Err(error) => serde_json::json!({"error": error.to_string()}).to_string(),
                }
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
    fn the_proof_note_golden_diagnostics_match_the_desktop_contract() {
        // The pad compiles the desktop `DocModel` and the K1 `change` module
        // verbatim, so this fixture has the same source ranges and codes the
        // desktop golden-set test asserts.
        let text = "#1 claim #2 \\formal(#1,#2, \"a claim\")\n\
                    #3 fig #4 \\app(#3,#8, 10, 20)\n\
                    $missing$";
        let model = DocModel::new(text);
        let diagnostics = model.diagnostics();
        let codes: Vec<_> = diagnostics.iter().map(|d| d.code.as_str()).collect();
        for expected in [
            "marker-pair",
            "dangling-figure",
            "missing-proof-result",
            "unresolved-name",
        ] {
            assert!(
                codes.contains(&expected),
                "missing {expected}: {diagnostics:?}"
            );
        }
        assert!(diagnostics.iter().all(|d| d.range.end <= text.len()));
    }

    #[test]
    fn a_replayed_trace_settles_the_same_state_as_the_desktop_reducer() {
        use inspect::{EventKind, SessionEvent};
        let mut event = |kind, frontier: &str, reason: &str, count: u64| {
            let mut e = SessionEvent::new(kind).with("run", "share", "pad");
            e.frontier = frontier.to_string();
            e.reason = reason.to_string();
            e.count = count;
            e
        };
        let trace = [
            event(EventKind::Load, "", "", 0),
            event(EventKind::Recovery, "", "late recovery", 0),
            event(EventKind::Handshake, "v1", "", 0),
            event(EventKind::AcceptedChanges, "v2", "", 2),
        ];
        let ndjson = trace
            .iter()
            .map(SessionEvent::to_ndjson)
            .collect::<Vec<_>>()
            .join("\n");
        let settled = inspect::replay(&ndjson).unwrap();
        assert_eq!(settled.local, "recovered");
        assert_eq!(settled.share, "current");
        assert_eq!(settled.accepted, 2);
        assert_eq!(settled.frontier, "v2");
    }

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
    fn page_metadata_carries_the_terminal_reference_for_each_figure_key() {
        let reference = "hc://6ab09a20fd1288983dfbbac2bb349231/t3";
        let text = format!("#a Figure #b \\app(#a, #b, 100, 100, terminal: \"{reference}\")\n");
        let bindings = layout::figure_bindings(&text);
        assert_eq!(bindings.len(), 1);
        let (terminal, error) = bindings.values().next().unwrap();
        assert_eq!(terminal.as_deref(), Some(reference));
        assert!(error.is_none());
    }

    #[test]
    fn terminal_figure_pin_is_one_step_and_rejects_stale_text() {
        let mut model = DocModel::new("hello\n");
        let reference = "hc://6ab09a20fd1288983dfbbac2bb349231/t3".to_string();
        let before = model.text().to_string();
        assert!(append_terminal_figure(&mut model, &before, &reference, "Build #log").is_ok());
        let text = model.text().to_string();
        assert!(text.contains("Build \\#log"), "{text}");
        assert!(text.contains(&format!("terminal: \"{reference}\"")));
        let scan = mathed_core::markers::scan(&text);
        let figures: Vec<_> = mathed_core::markers::resolve_segments(&scan)
            .iter()
            .filter_map(mathed_core::figures::resolve_figure)
            .collect();
        assert_eq!(figures.len(), 1);
        assert_eq!(
            figures[0]
                .spec
                .terminal
                .as_ref()
                .map(|terminal| terminal.canonical()),
            Some(reference.clone())
        );
        model.undo();
        assert_eq!(model.text(), before);
        assert!(append_terminal_figure(&mut model, "stale", &reference, "x").is_err());
    }

    #[test]
    fn rebind_replaces_only_the_terminal_argument_and_undoes_as_one_step() {
        let old = "hc://6ab09a20fd1288983dfbbac2bb349231/t3";
        let new = "hc://6ab09a20fd1288983dfbbac2bb349231/t4";
        let source = format!("#a Build log #b \\app(#a, #b, 640, 400, terminal: \"{old}\")\n");
        let mut model = DocModel::new(&source);
        let options = terminal_figure_options(model.text());
        assert!(options.contains("Build log"), "{options}");
        assert!(options.contains(old), "{options}");
        let scan = mathed_core::markers::scan(model.text());
        let key = mathed_core::markers::resolve_segments(&scan)
            .iter()
            .find_map(mathed_core::figures::resolve_figure)
            .expect("figure")
            .key;
        let before = model.text().to_string();
        rebind_terminal_figure(&mut model, &before, &key, new).unwrap();
        assert!(!model.text().contains(old));
        assert!(model.text().contains(new));
        assert!(model.text().contains("Build log"));
        assert!(model.text().contains("640, 400"));
        model.undo();
        assert_eq!(model.text(), before);
        assert!(rebind_terminal_figure(&mut model, &before, "f99", new).is_err());
        assert!(rebind_terminal_figure(&mut model, "stale", &key, new).is_err());
    }

    fn long_fixture() -> String {
        (0..90)
            .map(|i| {
                format!("Paragraph {i}: retained layout keeps the source map and glyph bands.\n\n")
            })
            .collect()
    }

    #[test]
    fn retained_layout_compiles_once_for_initial_and_later_chunks() {
        let text = long_fixture();
        let model = DocModel::new(&text);
        let snapshot = model.math_doc().snapshot();
        let prepared = layout::prepare(&snapshot, &text, "retained-a", false, &[], 17.0).unwrap();
        assert!(
            prepared["total"].as_u64().unwrap() > 1,
            "fixture must span pages"
        );
        let compiles = layout::compiles();

        let first = layout::render_page("retained-a", &text, 0, 1.0, 4_000_000, 0, 1).unwrap();
        let repeated = layout::render_page("retained-a", &text, 0, 1.0, 4_000_000, 0, 1).unwrap();
        let later = layout::render_page("retained-a", &text, 1, 1.0, 4_000_000, 0, 1).unwrap();
        assert_eq!(
            first["png"], repeated["png"],
            "page rasters are deterministic"
        );
        assert!(!later["hits"].as_array().unwrap().is_empty());
        assert_eq!(
            compiles,
            layout::compiles(),
            "later/repeated raster requests must not compile"
        );

        let independent: Value = serde_json::from_str(
            &render_pages_with_options_expanded(
                &text,
                0,
                1,
                1.0,
                0,
                Some(&model),
                false,
                true,
                &[],
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            first["png"], independent["pages"][0]["png"],
            "cached and independent rasters agree"
        );
        let cached_hits = first["hits"].as_array().unwrap();
        let independent_hits = independent["pages"][0]["hits"].as_array().unwrap();
        assert_eq!(
            cached_hits.len(),
            independent_hits.len(),
            "cached and independent hit counts agree"
        );
        for (i, (cached, standalone)) in cached_hits.iter().zip(independent_hits).enumerate() {
            let cached = cached.as_array().unwrap();
            let standalone = standalone.as_array().unwrap();
            assert_eq!(
                cached[4], standalone[4],
                "cached/independent hit {i} source offset differs"
            );
            for axis in 0..4 {
                let a = cached[axis].as_f64().unwrap();
                let b = standalone[axis].as_f64().unwrap();
                assert!(
                    (a - b).abs() < 0.01,
                    "cached/independent hit {i} geometry differs on axis {axis}: {a} vs {b}"
                );
            }
        }
    }

    #[test]
    fn presentation_and_mark_changes_replace_the_retained_layout() {
        let text = "#a bold #b visible text tail";
        let mut model = DocModel::new(text);
        let hidden = layout::prepare(
            &model.math_doc().snapshot(),
            text,
            "retained-hidden",
            false,
            &[],
            17.0,
        )
        .unwrap();
        assert!(hidden["total"].as_u64().unwrap() >= 1);
        let hidden_png = layout::render_page("retained-hidden", text, 0, 1.0, 4_000_000, 0, 0)
            .unwrap()["png"]
            .clone();

        let before_markers = layout::compiles();
        layout::prepare(
            &model.math_doc().snapshot(),
            text,
            "retained-markers",
            true,
            &[],
            17.0,
        )
        .unwrap();
        assert!(
            layout::compiles() > before_markers,
            "marker visibility is a layout input"
        );
        let shown_png = layout::render_page("retained-markers", text, 0, 1.0, 4_000_000, 0, 0)
            .unwrap()["png"]
            .clone();
        assert_ne!(hidden_png, shown_png);

        let visible = text.find("visible").unwrap();
        assert!(modifiers::set_link(
            &mut model,
            "a",
            "b",
            visible..visible + "visible".len(),
            true
        ));
        let before_mark = layout::compiles();
        layout::prepare(
            &model.math_doc().snapshot(),
            text,
            "retained-marks",
            false,
            &[],
            17.0,
        )
        .unwrap();
        assert!(
            layout::compiles() > before_mark,
            "a marks-only capture recompiles"
        );
        let marked_png = layout::render_page("retained-marks", text, 0, 1.0, 4_000_000, 0, 0)
            .unwrap()["png"]
            .clone();
        assert_ne!(
            hidden_png, marked_png,
            "a modifier-only change changes its output"
        );
    }

    #[test]
    fn retained_layout_locates_a_late_range_without_rasterizing() {
        let text = long_fixture();
        let model = DocModel::new(&text);
        layout::prepare(
            &model.math_doc().snapshot(),
            &text,
            "retained-locate",
            false,
            &[],
            17.0,
        )
        .unwrap();
        let target = text.rfind("Paragraph 89").unwrap();
        let start = byte_to_utf16(&text, target);
        let located = layout::locate_range("retained-locate", &text, start, start + 2).unwrap();
        assert_eq!(
            located["found"], true,
            "late source range must be locatable: {located}"
        );
        assert!(
            located["page"].as_u64().unwrap() > 0,
            "target belongs on a later page"
        );
        let page = located["page"].as_u64().unwrap() as usize;
        let raster =
            layout::render_page("retained-locate", &text, page, 1.0, 4_000_000, page, page)
                .unwrap();
        assert!(!raster["hits"].as_array().unwrap().is_empty());
    }

    #[test]
    fn stable_anchors_track_utf16_positions_across_edits() {
        let mut model = DocModel::new("alpha 👋 target");
        let target = model.text().find("target").unwrap();
        let target_utf16 = byte_to_utf16(model.text(), target);
        let anchor = model.math_doc().create_anchor(target, -1).unwrap();
        model.insert(0, "new 👋 prefix\n");
        let resolved = model
            .math_doc()
            .resolve_anchor(&anchor.encoded, anchor.fallback);
        assert_eq!(
            byte_to_utf16(model.text(), resolved),
            target_utf16 + "new 👋 prefix\n".encode_utf16().count()
        );
        assert!(model.text().is_char_boundary(resolved));
    }

    #[test]
    fn preview_hit_positions_are_document_utf16_offsets() {
        let doc = "👋 hello";
        let json = render_pages(doc, 0, 1, 1.0, 0).unwrap();
        let value: Value = serde_json::from_str(&json).unwrap();
        let hits = value["pages"][0]["hits"].as_array().unwrap();
        assert!(
            hits.iter().any(|h| h[4] == 3),
            "h begins at UTF-16 offset 3"
        );
        assert!(hits
            .iter()
            .all(|h| h[1].as_f64().unwrap() < h[3].as_f64().unwrap()));
    }

    #[test]
    fn search_uses_mathed_case_rules_and_android_utf16_offsets() {
        assert_eq!(
            search_ranges("👋 Alpha α ALPHA", "alpha"),
            json!([[3, 8], [11, 16]])
        );
        assert_eq!(search_ranges("👋 Alpha α ALPHA", "Alpha"), json!([[3, 8]]));
        assert_eq!(search_ranges("👋 Alpha α ALPHA", "α"), json!([[9, 10]]));
    }

    #[test]
    fn scoped_search_protects_structure_and_honours_selection_pages() {
        let text = "#a bold #b visible alpha";
        let protected = search_scoped(text, "#a", "text", 0, 0, 0, -1, 200, false);
        assert_eq!(
            protected["total"], 0,
            "Text scope omits marker tokens: {protected}"
        );
        let source = search_scoped(text, "#a", "source", 0, 0, 0, -1, 200, false);
        assert_eq!(source["total"], 1);
        assert_eq!(source["matches"][0]["protected"], true);
        let statement = "#a body #b \\cite(#a, #b)";
        let statement_text = search_scoped(statement, "cite", "text", 0, 0, 0, -1, 200, false);
        assert_eq!(
            statement_text["total"], 0,
            "Text scope omits property statements"
        );
        let statement_source = search_scoped(statement, "cite", "source", 0, 0, 0, -1, 200, false);
        assert_eq!(statement_source["total"], 1);
        assert_eq!(statement_source["matches"][0]["protected"], true);

        let visible = text.find("visible").unwrap();
        let start = byte_to_utf16(text, visible);
        let frozen = search_scoped(
            text,
            "visible",
            "selection",
            start,
            start + 7,
            0,
            -1,
            200,
            false,
        );
        assert_eq!(frozen["total"], 1);
        let outside = search_scoped(
            text,
            "alpha",
            "selection",
            start,
            start + 7,
            0,
            -1,
            200,
            false,
        );
        assert_eq!(
            outside["total"], 0,
            "Selection scope never crosses its frozen endpoints"
        );

        let starter = "#1 terminal #2 \\app(#1, #2, 600, 300, \"term\")\n".to_string()
            + "#3 editor #4 \\app(#3, #4, 400, 300, \"vim\")\n";
        let prose = search_scoped(&starter, "editor", "text", 0, 0, 0, -1, 200, false);
        assert_eq!(
            prose["total"], 1,
            "marker bodies remain searchable text: {prose}"
        );

        let many = "x ".repeat(250);
        let first = search_scoped(&many, "x", "text", 0, 0, 0, -1, 200, false);
        assert_eq!(first["total"], 250);
        assert_eq!(first["matches"].as_array().unwrap().len(), 200);
        let last = search_scoped(&many, "x", "text", 0, 0, 0, 200, 200, false);
        assert_eq!(last["matches"].as_array().unwrap().len(), 50);
    }

    #[test]
    fn literal_replace_batch_is_protected_and_one_undo_step() {
        let text = "#a bold #b one two one";
        let mut model = DocModel::new(text);
        let first = text.find("one").unwrap();
        let second = text.rfind("one").unwrap();
        let ops = json!([
            {"start": byte_to_utf16(text, first), "end": byte_to_utf16(text, first + 3), "text": "1"},
            {"start": byte_to_utf16(text, second), "end": byte_to_utf16(text, second + 3), "text": "1"},
        ]);
        replace_many_json(&mut model, text, &ops).unwrap();
        assert_eq!(model.text(), "#a bold #b 1 two 1");
        model.undo();
        assert_eq!(
            model.text(),
            text,
            "one Undo restores the whole replacement batch"
        );

        assert!(replace_many_json(&mut model, "stale", &ops).is_err());
        let bad = json!([{"start": 0, "end": 3, "text": "x"}]);
        assert!(
            replace_many_json(&mut model, text, &bad).is_err(),
            "markers/property statements cannot be replaced"
        );
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
        let detail = citation_detail(doc, 1, 300.0, 1.0, None, citations::DEFAULT_CITATION_LOCALE);
        assert_eq!(detail["kind"], "bibliography");
        assert_eq!(detail["resolved"], true);
        assert!(detail["text"].as_str().unwrap().contains("Kwan"));
        assert!(
            detail["text"].as_str().unwrap().contains("2014"),
            "citation year missing: {detail}"
        );
        assert!(detail["entries"][0]
            .as_str()
            .unwrap()
            .contains("Crazy Rich Asians"));
        let bare = "#1 crazy-rich:\n    type: Book\n    title: Crazy Rich Asians\n    author: Kwan, Kevin\n    date: 2014\n#2 \\bibliography(#1,#2, refs)\n\\cite(\"crazy-rich\", bib: \"refs\") \\cite(#1,#2)";
        let index = citation_index(bare, true);
        assert_eq!(index[0]["numbers"], json!([1]));
        assert_eq!(index[1]["numbers"], json!([2]));
        assert_eq!(index[0]["keys"], json!(["crazy-rich"]));
        assert!(index[0]["source"].as_str().unwrap().contains("\\cite("));
        assert_eq!(index[0]["resolved"], true);
        assert_eq!(
            citation_detail(
                bare,
                1,
                300.0,
                1.0,
                None,
                citations::DEFAULT_CITATION_LOCALE
            )["resolved"],
            true
        );
        let vancouver = citation_detail(bare, 1, 300.0, 1.0, Some("vancouver"), "de-DE");
        assert_eq!(vancouver["resolved"], true);
        assert_eq!(vancouver["style"], "vancouver");
        assert_eq!(vancouver["locale"], "de-DE");
        assert_eq!(vancouver["metadata"]["number"], 1);
        assert!(vancouver["metadata"]["sourceStart"].as_u64().is_some());
        assert_eq!(citations::CITATION_STYLES[0], "apa");
        assert!(citations::CITATION_LOCALES.contains(&citations::DEFAULT_CITATION_LOCALE));
        let misspelled = bare.replace("\"crazy-rich\", bib", "\"Kwn14\", bib");
        let detail = citation_detail(
            &misspelled,
            1,
            300.0,
            1.0,
            None,
            citations::DEFAULT_CITATION_LOCALE,
        );
        assert_eq!(detail["resolved"], false);
        assert!(detail["errorDetail"]
            .as_str()
            .unwrap()
            .contains("crazy-rich"));
        let unresolved_index = citation_index(&misspelled, true);
        assert_eq!(unresolved_index[0]["resolved"], false);
        assert!(unresolved_index[0]["statusError"]
            .as_str()
            .unwrap()
            .contains("crazy-rich"));
        let bibtex = "#a @book{kwan:2014, title={Crazy Rich Asians}, author={Kwan, Kevin}, year={2014}} #b \\bibliography(#a,#b, refs, format: \"bibtex\") \\cite(\"kwan:2014\", bib: \"refs\")";
        let bibtex_detail = citation_detail(
            bibtex,
            1,
            300.0,
            1.0,
            None,
            citations::DEFAULT_CITATION_LOCALE,
        );
        assert_eq!(bibtex_detail["resolved"], true, "{bibtex_detail}");
        assert!(bibtex_detail["text"].as_str().unwrap().contains("2014"));
        let reference = "#a Hello $x^2$ #b \\cite(#a,#b)";
        let index = citation_index(reference, true);
        assert_eq!(index[0]["numbers"], json!([1]));
        let page: Value = serde_json::from_str(
            &render_pages_with_options(reference, 0, 1, 1.0, 0, None, false, true).unwrap(),
        )
        .unwrap();
        let cite_start = index[0]["start"].as_u64().unwrap();
        assert!(
            page["pages"][0]["hits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|hit| hit[4].as_u64() == Some(cite_start)),
            "citation label must be tappable"
        );
        let detail = citation_detail(
            reference,
            1,
            300.0,
            2.0,
            None,
            citations::DEFAULT_CITATION_LOCALE,
        );
        assert_eq!(detail["kind"], "document");
        assert!(detail["png"].is_string());
    }

    #[test]
    fn explicit_fold_expands_only_the_selected_translator() {
        let text = "before #3 #let translate(b) = { \"[]\" } #4 \\translator(#3,#4, name: \"ho\")";
        let collapsed: Value = serde_json::from_str(
            &render_pages_with_options_expanded(text, 0, 1, 1.0, 0, None, false, true, &[])
                .unwrap(),
        )
        .unwrap();
        let expanded: Value = serde_json::from_str(
            &render_pages_with_options_expanded(
                text,
                0,
                1,
                1.0,
                0,
                None,
                false,
                true,
                &[("3".into(), "4".into())],
            )
            .unwrap(),
        )
        .unwrap();
        assert_ne!(collapsed["pages"][0]["png"], expanded["pages"][0]["png"]);
        let hits = collapsed["pages"][0]["hits"].as_array().unwrap();
        let marker = byte_to_utf16(text, text.find("#3").unwrap());
        let span = marker + 2;
        assert!(hits.iter().any(|h| h[4] == marker || h[4] == span));
    }

    #[test]
    fn marker_visibility_changes_the_preview_without_changing_the_document() {
        let doc = "#a alpha #b tail";
        let hidden: Value = serde_json::from_str(
            &render_pages_with_options(doc, 0, 1, 1.0, doc.len(), None, false, true).unwrap(),
        )
        .unwrap();
        let shown: Value = serde_json::from_str(
            &render_pages_with_options(doc, 0, 1, 1.0, doc.len(), None, true, true).unwrap(),
        )
        .unwrap();
        assert_ne!(hidden["pages"][0]["png"], shown["pages"][0]["png"]);
    }

    #[test]
    fn rendered_editor_does_not_expand_source_when_caret_enters_a_figure() {
        let doc = "#a caption #b \\app(#a,#b,100,60)";
        let open: Value = serde_json::from_str(
            &render_pages_with_options(
                doc,
                0,
                1,
                1.0,
                doc.find("caption").unwrap(),
                None,
                false,
                true,
            )
            .unwrap(),
        )
        .unwrap();
        let elsewhere: Value = serde_json::from_str(
            &render_pages_with_options(doc, 0, 1, 1.0, doc.len(), None, false, true).unwrap(),
        )
        .unwrap();
        assert!(open["pages"][0]["png"].is_string());
        assert_eq!(open["pages"][0]["png"], elsewhere["pages"][0]["png"]);
    }

    #[test]
    fn rendered_editor_transports_a_decodable_full_resolution_page() {
        use base64::Engine as _;
        let result: Value = serde_json::from_str(
            &render_pages_with_options("Visible text", 0, 1, 2.0, 0, None, false, true).unwrap(),
        )
        .unwrap();
        let page = &result["pages"][0];
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(page["png"].as_str().unwrap())
            .unwrap();
        let reader = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        assert_eq!(reader.info().width, page["width"].as_u64().unwrap() as u32);
        assert_eq!(
            reader.info().height,
            page["height"].as_u64().unwrap() as u32
        );
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
        model
            .math_doc_mut()
            .mark_segment(1..5, "prop:bold", "example");
        model.math_doc_mut().commit();
        let encoded = base64::engine::general_purpose::STANDARD.encode(model.math_doc().snapshot());
        let restored = model_from_snapshot(&encoded, "a👋b").unwrap();
        assert_eq!(
            restored.math_doc().segment_marks(),
            vec![(1..5, "prop:bold".into())]
        );
        assert!(model_from_snapshot(&encoded, "different").is_none());
        assert!(model_from_snapshot("not-base64", "a👋b").is_none());
    }

    #[test]
    fn logical_fingerprint_tracks_marks_but_not_snapshot_history() {
        let mut model = DocModel::new("a👋b");
        let before = logical_fingerprint(&model.math_doc().snapshot()).unwrap();
        assert_eq!(before.len(), 64, "lowercase SHA-256 hex");
        let revision = model.math_doc().revision();
        model
            .math_doc_mut()
            .mark_segment(1..5, "prop:bold", "example");
        model.math_doc_mut().commit();
        let after = logical_fingerprint(&model.math_doc().snapshot()).unwrap();
        assert_eq!(
            model.math_doc().revision(),
            revision,
            "a mark-only change must not bump the text revision"
        );
        assert_ne!(
            before, after,
            "all mark attributes contribute to the fingerprint"
        );

        let restored = DocModel::from_snapshot(&model.math_doc().snapshot()).unwrap();
        assert_eq!(
            logical_fingerprint(&restored.math_doc().snapshot()).unwrap(),
            after,
            "the fingerprint survives snapshot round-trip"
        );
        assert_eq!(canonical_content(b"not a loro snapshot"), None);
    }

    #[test]
    fn loro_modifier_changes_the_real_page_raster() {
        let mut model = DocModel::new("#a bold #b hello");
        let at = model.text().find("hello").unwrap();
        assert!(modifiers::set_link(&mut model, "a", "b", at..at + 5, true));
        let plain: Value =
            serde_json::from_str(&render_pages(model.text(), 0, 1, 1.0, 0).unwrap()).unwrap();
        let styled: Value = serde_json::from_str(
            &render_pages_with_model(model.text(), 0, 1, 1.0, 0, Some(&model)).unwrap(),
        )
        .unwrap();
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
        let relationships = modifier_relationships(&model, "c", "d");
        let incoming = &relationships["incoming"][0];
        assert_eq!(incoming["startId"], "a");
        assert_eq!(incoming["endId"], "b");
        assert!(incoming["snippet"].as_str().unwrap().contains("hello"));
        assert_eq!(incoming["resolved"], true);
        let row = modifier_comparison_row(
            &model,
            incoming["start"].as_i64().unwrap() as i32,
            incoming["end"].as_i64().unwrap() as i32,
            "a",
            "b",
            250.0,
            2.0,
        );
        assert!(row["with"]["rgba"].is_string());
        assert!(row["without"]["rgba"].is_string());
        assert_ne!(row["with"]["rgba"], row["without"]["rgba"]);
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
        assert!(modifiers::set_link(
            &mut model,
            "c",
            "d",
            at + 3..at + 8,
            true
        ));
        assert!(render_pages_with_model(model.text(), 0, 1, 1.0, 0, Some(&model)).is_ok());
    }

    #[test]
    fn invalid_modifier_does_not_hide_the_document_or_valid_modifier() {
        let mut model = DocModel::new("\n#ad bold #am\n#i fgh #o\n \n    ");
        let bold = model.text().find("bold").unwrap();
        let target = model.text().find("fgh").unwrap();
        assert!(modifiers::set_link(
            &mut model,
            "ad",
            "am",
            target..target + 3,
            true
        ));
        let expected: Value = serde_json::from_str(
            &render_pages_with_model(model.text(), 0, 1, 1.0, 0, Some(&model)).unwrap(),
        )
        .unwrap();
        assert!(modifiers::set_link(
            &mut model,
            "i",
            "o",
            bold..bold + 4,
            true
        ));
        let actual: Value = serde_json::from_str(
            &render_pages_with_model(model.text(), 0, 1, 1.0, 0, Some(&model)).unwrap(),
        )
        .unwrap();
        assert_eq!(actual["pages"][0]["rgba"], expected["pages"][0]["rgba"]);
        assert!(actual["warnings"][0].as_str().unwrap().contains("fgh"));
        assert_eq!(
            modifiers::links(&model).len(),
            2,
            "rendering must not remove links"
        );
    }

    #[test]
    fn diagnostics_report_marker_pairs_and_unimplemented_execution() {
        let text = "#1 text #2 \\app(#1, #9, 10, 10)\n#3 thing #4 \\prob(#3, #4)";
        let model = DocModel::new(text);
        let diagnostics = diagnostics::source_diagnostics(&model, "capture-a");
        let missing = diagnostics
            .iter()
            .find(|diagnostic| diagnostic["code"] == "marker-pair")
            .expect("dangling marker pair is reported");
        assert_eq!(missing["severity"], "error");
        assert_eq!(missing["provenance"], "exact");
        assert_eq!(missing["markerPair"], json!(["1", "9"]));
        assert!(
            missing["start"].as_u64().is_some(),
            "statement range is exact source"
        );
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic["code"] == "execution-unavailable"
                && diagnostic["message"]
                    .as_str()
                    .unwrap()
                    .contains("Execution is unavailable")
        }));
        assert!(diagnostics
            .iter()
            .all(|diagnostic| diagnostic["captureId"] == "capture-a"));
    }

    #[test]
    fn diagnostic_attributes_a_generated_typst_error_to_its_modifier_owner() {
        let text = "#1 target #2\n#3 nope #4\n";
        let mut model = DocModel::new(text);
        assert!(modifiers::set_marker_targets(&mut model, "3", "4", "1 2").is_ok());
        let prepared = layout::prepare(
            &model.math_doc().snapshot(),
            text,
            "diag-owner",
            false,
            &[],
            17.0,
        )
        .expect("the invalid modifier is isolated and the document still renders");
        let diagnostics = prepared["diagnostics"].as_array().unwrap();
        let owner = diagnostics
            .iter()
            .find(|diagnostic| diagnostic["code"] == "modifier-application")
            .expect("the failing modifier is a diagnostic, not a discarded Eval");
        assert_eq!(owner["provenance"], "owner");
        assert_eq!(owner["markerPair"], json!(["3", "4"]));
        assert_eq!(
            owner["start"],
            json!(byte_to_utf16(text, text.find("nope").unwrap() - 1))
        );
        assert!(owner["end"].as_u64().unwrap() > owner["start"].as_u64().unwrap());
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
        assert_eq!(utf16_to_grapheme_byte("e\u{301}", 1), 0);
        assert_eq!(utf16_to_grapheme_byte("e\u{301}", 2), "e\u{301}".len());
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
    fn grapheme_boundaries_keep_zwj_combining_and_rtl_clusters_whole() {
        let family = "x👨‍👩‍👧y";
        let family_start = 1;
        let family_end = byte_to_utf16(family, family.find('y').unwrap());
        assert_eq!(
            grapheme_boundary_utf16(family, family_start + 2, 1),
            family_end
        );
        assert_eq!(
            grapheme_boundary_utf16(family, family_end, -1),
            family_start
        );
        assert_eq!(
            grapheme_boundary_utf16(family, family_end, 2),
            family_end + 1
        );
        let accented = "e\u{301}x";
        assert_eq!(grapheme_boundary_utf16(accented, 1, -1), 0);
        assert_eq!(grapheme_boundary_utf16(accented, 1, 1), 2);
        let rtl = "مرحبا\u{64e}x";
        let after = byte_to_utf16(rtl, rtl.find('x').unwrap());
        assert_eq!(grapheme_boundary_utf16(rtl, after - 1, 1), after);
    }

    #[test]
    fn sync_expands_edits_to_whole_grapheme_clusters() {
        let family = "👨‍👩‍👧";
        let mut m = DocModel::new(family);
        let without_mother = "👨‍👧";
        sync_text(
            &mut m,
            without_mother,
            without_mother.encode_utf16().count(),
        );
        assert_eq!(m.text(), without_mother);
        assert!(m.undo());
        assert_eq!(m.text(), family, "one undo restores the whole old cluster");

        let mut combining = DocModel::new("e\u{301}x");
        sync_text(&mut combining, "ex", 1);
        assert_eq!(combining.text(), "ex");
        assert!(combining.undo());
        assert_eq!(combining.text(), "e\u{301}x");
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
        assert_eq!(
            pages.len() as u64,
            total.min(8),
            "one entry per page in range"
        );
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
                &render_pages(&starter, 0, 3, scale, caret)
                    .expect("the starter document must render"),
            )
            .unwrap();
            assert!(
                v.get("error").is_none(),
                "caret {caret}: unexpected render error: {v}"
            );
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
            doc.push_str(&format!(
                r#"#{i} block{i} #{m} \app(#{i}, #{m}, 300, 100, "w{i}")"#
            ));
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
        assert_eq!(
            second["total"].as_u64().unwrap(),
            total,
            "total is document-wide"
        );
        assert_eq!(second["start"], json!(2));
        assert_eq!(second["pages"].as_array().unwrap().len(), 2);

        // Past the end: empty pages, same total — the "stop loading"
        // signal the Java side keys its scroll streaming on.
        let tail = chunk(total as usize + 1, 2);
        assert_eq!(tail["total"].as_u64().unwrap(), total);
        assert!(
            tail["pages"].as_array().unwrap().is_empty(),
            "past the end: no pages"
        );
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
        assert!(
            one.contains("\"total\":1"),
            "a short note is one page: {one}"
        );
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
        let marker = "page.setBackgroundColor(Color.parseColor(\"#";
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
            java.contains("previewPages.addView(page, new LinearLayout.LayoutParams("),
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
        let a =
            render_pages(cited, 0, 8, 1.0, cited.find("prose").unwrap()).expect("cited renders");
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
            java.contains("requestChunk(text, 0, caret, \"initial\");"),
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

    /// A6 wiring guard: page work stays on the render worker, decoding and
    /// pixel sampling do not run on the main looper, old pages remain until a
    /// replacement is accepted, and caret overlays do not request a relayout.
    #[test]
    fn a6_rendering_uses_a_retained_worker_layout() {
        let java = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/android/src/com/emthin/pad/MainActivity.java"
        ))
        .expect("MainActivity.java must be readable");
        assert!(
            java.contains("DocBridge.prepareLayout(")
                && java.contains("DocBridge.renderLayoutPage(")
                && java.contains("renderAndDecodeChunk("),
            "the app must prepare one layout and raster pages from it on the worker"
        );
        assert!(
            java.contains("BitmapFactory.decodeByteArray(png, 0, png.length);")
                && java.contains("decodeMs += (System.nanoTime() - decodeStart)"),
            "PNG decode and pixel sampling must be counted in worker work"
        );
        assert!(
            !java.contains("DocBridge.renderPagesWithSnapshot(text, snapshot"),
            "the app must use captured snapshot bytes, not the superseded string path"
        );
        assert!(
            java.contains("renderPresentationKey()")
                && !java.contains("renderPresentationKey(caret)"),
            "the layout key must exclude the caret"
        );
        let caret_start = java.find("private final class CaretEditText").unwrap();
        let caret_end = java.find("private final class SourceEditText").unwrap();
        assert!(
            !java[caret_start..caret_end].contains("scheduleRender()"),
            "caret movement must repaint overlays without a layout request"
        );
        assert!(
            java.contains("pagesStale = true;")
                && java.contains("if (pagesStale) return true;")
                && java.contains("pagesStale = false;"),
            "old pages must remain visible but reject stale hit positions"
        );
        assert!(
            java.contains("if (session.isLatest(owner))") && java.contains("applyChunk(chunk);"),
            "a superseded worker chunk must not erase the newer frame"
        );
    }

    /// A8 wiring guard: long-press selection shares the rendered selection
    /// and clipboard path, and every loaded page exposes source-backed
    /// virtual text blocks through an AccessibilityNodeProvider.
    #[test]
    fn a8_touch_selection_and_virtual_accessibility_are_wired() {
        let java = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/android/src/com/emthin/pad/MainActivity.java"
        ))
        .expect("MainActivity.java must be readable");
        assert!(
            java.contains("touchLongPress")
                && java.contains("selectWordAt(")
                && java.contains("showSelectionBar();")
                && java.contains("copyRenderedSelection();")
                && java.contains("deleteRenderedSelection();")
                && java.contains("pasteRenderedClipboard();"),
            "touch selection must use the existing rendered selection and clipboard path"
        );
        assert!(
            java.contains("getAccessibilityNodeProvider()")
                && java.contains("accessibleBlocks(")
                && java.contains("visibleSource(")
                && java.contains("AccessibilityAction.ACTION_CLICK")
                && java.contains("requestPage(index, \"accessible\")"),
            "rendered pages must expose source-backed virtual text and load on accessibility focus"
        );
    }

    /// A11 wiring guard: Find has scoped bounded pages, a separable
    /// controller, and literal replacement goes through one native batch.
    #[test]
    fn a11_scoped_find_and_grouped_replace_are_wired() {
        let java = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/android/src/com/emthin/pad/MainActivity.java"
        ))
        .expect("MainActivity.java must be readable");
        let find = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/android/src/com/emthin/pad/FindController.java"
        ))
        .expect("FindController.java must be readable");
        assert!(
            java.contains("new FindController(")
                && java.contains("DocBridge.searchScoped(")
                && java.contains("DocBridge.replaceMany(")
                && java.contains("occurrences")
                && java.contains("Structural matches are not replaced"),
            "the UI must use the scoped search and checked replacement boundary"
        );
        assert!(
            find.contains("public static final int PAGE_SIZE = 200;")
                && find.contains("if (token != requestToken) return;")
                && find.contains("public void searchFrom("),
            "Find must retain one bounded page and reject stale scans"
        );
        assert!(
            java.contains("closeSearch(true)")
                && java.contains("restoreSearchOrigin()")
                && java.contains("expandedSegments.putAll(origin.folds)"),
            "Cancel must restore the captured viewport/folds/selection"
        );
    }

    /// A5 wiring guard: candidates stay in the native field until commit,
    /// and rendered movement/deletion asks the shared Rust rules for
    /// grapheme boundaries instead of using Java's char/code-point units.
    #[test]
    fn editing_defers_composition_and_uses_native_grapheme_boundaries() {
        let java = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/android/src/com/emthin/pad/MainActivity.java"
        ))
        .expect("MainActivity.java must be readable");
        assert!(
            java.contains("DocBridge.graphemeBoundary"),
            "rendered editing must use mathed's native grapheme rules"
        );
        assert!(
            java.contains("boolean isComposing()")
                && java.contains("finishActiveComposition();")
                && java.contains("isComposing(editor)"),
            "composition must be observable and flushed before Save/New/Open"
        );
        assert!(
            java.contains("replaceRenderedRange"),
            "selection replacement must combine removal and insertion"
        );
        assert!(
            !java.contains("Character.charCount(Character.codePointAt"),
            "rendered movement must not use a second code-point boundary rule"
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
            java.contains("recoveryStore = new RecoveryStore(getFilesDir())")
                && java.contains("recoveryStore.load()")
                && java.contains("recoveryWorker.execute"),
            "onCreate must inspect recovery slots on the owned background worker"
        );
        assert!(
            java.contains("DocBridge.openSnapshotBytes(load.best.snapshot)")
                && java.contains("DocBridge.createFromSnapshot(legacy.snapshot, legacy.text)"),
            "cold launch must prefer a valid recovery slot and migrate a matching legacy snapshot"
        );
        assert!(
            !java.contains(".putString(PREF_SNAPSHOT,") && !java.contains(".putString(PREF_DOC,"),
            "document-sized state must not be written back to SharedPreferences"
        );
        assert!(
            java.contains("protected void onPause()")
                && java.contains("captureRecoveryNow()")
                && java.contains("captureViewNow()"),
            "pause must capture recovery and view state for the owned worker"
        );
        assert!(
            java.contains(".putInt(PREF_CARET, Math.max(0, editor.getSelectionStart()))")
                && java.contains(".putInt(PREF_SCROLL, readingY())"),
            "savePosition must persist the caret and preview reading offset"
        );
        assert!(
            java.contains("restoreScrollY = scroll")
                && java.contains("editor.setSelection(Math.min(caret, selection),"),
            "a cold launch must restore the caret, selection and reading offset"
        );
    }
}
