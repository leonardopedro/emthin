//! Structured diagnostics for A15.
//!
//! These are views of the existing scanner, semantic index and modifier
//! marks, plus Typst's own warnings/errors. No second parser or lint engine is
//! involved. A diagnostic either points at exact copied source, at the
//! producing modifier/source statement (`owner`), or at the document as a
//! whole (`document`); generated render markup is never snapped onto an
//! unrelated neighboring source byte.

use std::collections::HashMap;
use std::ops::Range;

use mathed_core::markers::PropKind;
use mathed_core::transform::OffsetMap;
use mathed_mini::MiniWorld;
use serde_json::{json, Value};
use typst::diag::SourceDiagnostic;
use typst::WorldExt;

use crate::model::DocModel;
use crate::{byte_to_utf16, modifiers};

fn diagnostic(
    capture: &str,
    index: usize,
    severity: &str,
    code: &str,
    message: String,
    text: &str,
    range: Option<Range<usize>>,
    pair: Option<(&str, &str)>,
    provenance: &str,
) -> Value {
    let (start, end) = match range {
        Some(range) => (
            json!(byte_to_utf16(text, range.start)),
            json!(byte_to_utf16(text, range.end)),
        ),
        None => (Value::Null, Value::Null),
    };
    json!({
        "id": format!("{capture}:{code}:{index}"),
        "severity": severity,
        "code": code,
        "message": message,
        "captureId": capture,
        "start": start,
        "end": end,
        "markerPair": pair.map(|(a, b)| json!([a, b])),
        "provenance": provenance,
    })
}

fn marker_pairs(
    scan: &mathed_core::markers::MarkerScan,
) -> Vec<(String, String, Range<usize>, String)> {
    let by_id: HashMap<&str, &mathed_core::markers::Marker> = scan
        .markers
        .iter()
        .map(|marker| (marker.id.as_str(), marker))
        .collect();
    let mut result = Vec::new();
    for stmt in &scan.stmts {
        let [a, b, ..] = stmt.args.as_slice() else {
            continue;
        };
        let (Some(start_id), Some(end_id)) = (a.marker_id(), b.marker_id()) else {
            continue;
        };
        let pair_start = by_id.get(start_id);
        let pair_end = by_id.get(end_id);
        let message = match (pair_start, pair_end) {
            (None, Some(_)) => Some(format!("Marker #{start_id} is missing.")),
            (Some(_), None) => Some(format!("Marker #{end_id} is missing.")),
            (None, None) => Some(format!("Markers #{start_id} and #{end_id} are missing.")),
            (Some(first), Some(last)) if first.range.end > last.range.start => Some(format!(
                "Markers #{start_id} and #{end_id} are reversed; the first must precede the second."
            )),
            _ => None,
        };
        if let Some(message) = message {
            result.push((
                start_id.to_owned(),
                end_id.to_owned(),
                stmt.range.clone(),
                message,
            ));
        }
    }
    result
}

/// Scanner, semantic-index and modifier-mark diagnostics for one capture.
/// The caller adds Typst's compile warnings/errors with [`typst_diagnostic`].
pub fn source_diagnostics(model: &DocModel, capture: &str) -> Vec<Value> {
    let text = model.text();
    let scan = model.scan();
    let mut result = Vec::new();

    for (start_id, end_id, range, message) in marker_pairs(&scan) {
        let index = result.len();
        result.push(diagnostic(
            capture,
            index,
            "error",
            "marker-pair",
            message,
            text,
            Some(range),
            Some((&start_id, &end_id)),
            "exact",
        ));
    }

    for (name, range) in crate::semantics::unresolved_names(text) {
        let index = result.len();
        result.push(diagnostic(
            capture,
            index,
            "warning",
            "unresolved-name",
            format!("Name “{name}” has no resolved definition."),
            text,
            Some(range),
            None,
            "exact",
        ));
    }

    for stmt in &scan.stmts {
        let kind = PropKind::of(&stmt.name);
        let execution = matches!(
            kind,
            PropKind::Exec
                | PropKind::Kernel
                | PropKind::Model
                | PropKind::Prior
                | PropKind::Solver
                | PropKind::Event
                | PropKind::Prob
        );
        if !execution {
            continue;
        }
        let index = result.len();
        result.push(diagnostic(
            capture,
            index,
            if matches!(kind, PropKind::Exec | PropKind::Kernel) {
                "error"
            } else {
                "warning"
            },
            "execution-unavailable",
            "Execution is unavailable on this Android frontend; the statement is preserved."
                .to_string(),
            text,
            Some(stmt.range.clone()),
            None,
            "exact",
        ));
    }

    for link in modifiers::links(model) {
        if link.resolved {
            continue;
        }
        let source = modifiers::source_range(model, &link.start_id, &link.end_id);
        let message = format!(
            "Modifier #{} … #{} no longer resolves to one forward marker pair.",
            link.start_id, link.end_id
        );
        let index = result.len();
        result.push(diagnostic(
            capture,
            index,
            "error",
            "modifier-owner",
            message,
            text,
            source,
            Some((&link.start_id, &link.end_id)),
            "owner",
        ));
    }

    // Stable order independent of the three sources above, so the UI does not
    // reshuffle equal-severity rows between identical renders.
    result.sort_by(|a, b| {
        let a = (
            a["severity"].as_str().unwrap_or(""),
            a["start"].as_u64().unwrap_or(u64::MAX),
        );
        let b = (
            b["severity"].as_str().unwrap_or(""),
            b["start"].as_u64().unwrap_or(u64::MAX),
        );
        a.cmp(&b)
    });
    result
}

fn copied_source(map: &OffsetMap, render: &Range<usize>) -> Option<Range<usize>> {
    let span = map.spans.iter().find(|span| {
        span.len > 0
            && span.render_start <= render.start
            && render.end <= span.render_start + span.len
    })?;
    Some(
        map.render_to_doc(span.render_start + (render.start - span.render_start))
            ..map.render_to_doc(span.render_start + (render.end - span.render_start)),
    )
}

fn owner_for(model: &DocModel, doc_pos: usize) -> Option<(String, String, Range<usize>)> {
    for link in modifiers::links(model)
        .into_iter()
        .filter(|link| link.resolved)
    {
        let Some(source) = modifiers::source_range(model, &link.start_id, &link.end_id) else {
            continue;
        };
        if source.contains(&doc_pos) || link.target.contains(&doc_pos) {
            return Some((link.start_id, link.end_id, source));
        }
    }
    None
}

/// Convert one Typst warning/error into the Android diagnostic shape. The
/// `prelude_len` strips the theme prefix; only spans wholly inside a verbatim
/// copy span are `exact`, so generated wrapper text is attributed to its
/// modifier or reported at document level instead.
pub fn typst_diagnostic(
    world: &MiniWorld,
    source_diag: &SourceDiagnostic,
    map: &OffsetMap,
    prelude_len: usize,
    model: &DocModel,
    capture: &str,
    index: usize,
) -> Value {
    let severity = match source_diag.severity {
        typst::diag::Severity::Error => "error",
        typst::diag::Severity::Warning => "warning",
    };
    let mut message = source_diag.message.to_string();
    for hint in &source_diag.hints {
        if !hint.v.is_empty() {
            message.push_str(" · ");
            message.push_str(&hint.v);
        }
    }
    let text = model.text();
    let source_range = world
        .range(source_diag.span)
        .filter(|range| range.start >= prelude_len)
        .map(|range| range.start - prelude_len..range.end.saturating_sub(prelude_len));
    if let Some(render) = source_range {
        if let Some(exact) = copied_source(map, &render) {
            return diagnostic(
                capture,
                index,
                severity,
                "typst",
                message,
                text,
                Some(exact),
                None,
                "exact",
            );
        }
        let doc_pos = map.render_to_doc(render.start).min(text.len());
        if let Some((start_id, end_id, source)) = owner_for(model, doc_pos) {
            return diagnostic(
                capture,
                index,
                severity,
                "typst",
                message,
                text,
                Some(source),
                Some((&start_id, &end_id)),
                "owner",
            );
        }
    }
    diagnostic(
        capture, index, severity, "typst", message, text, None, None, "document",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use typst::World;

    #[test]
    fn typst_spans_over_copied_source_are_exact_and_detached_spans_are_document_level() {
        let text = "#1 hello #2";
        let model = DocModel::new(text);
        let render = mathed_mini::doc_to_render_with(text, &Default::default());
        let prelude = mathed_mini::theme_prelude(17.0);
        let world = MiniWorld::new(format!("{prelude}{}", render.text));
        let at = render.text.find("hello").unwrap();

        let exact = typst::diag::SourceDiagnostic::warning(
            typst::syntax::DiagSpan::from_range(
                world.main(),
                prelude.len() + at..prelude.len() + at + "hello".len(),
            ),
            "check this text",
        );
        let exact = typst_diagnostic(&world, &exact, &render.map, prelude.len(), &model, "cap", 0);
        assert_eq!(exact["provenance"], "exact");
        assert_eq!(
            exact["start"],
            json!(3),
            "the source range excludes hidden markers"
        );
        assert_eq!(exact["end"], json!(8));

        let unlocated = typst::diag::SourceDiagnostic::warning(
            typst::syntax::Span::detached(),
            "document-level warning",
        );
        let unlocated = typst_diagnostic(
            &world,
            &unlocated,
            &render.map,
            prelude.len(),
            &model,
            "cap",
            1,
        );
        assert_eq!(unlocated["provenance"], "document");
        assert_eq!(unlocated["start"], Value::Null);
    }
}
