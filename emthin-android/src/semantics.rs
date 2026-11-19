//! Semantic navigation and checked definition rename for the pad.
//!
//! The definitions, use sites, sections and references are projected from
//! mathed's existing `SemanticIndex` / `DocumentContext`; this module does not
//! parse Typst or keep a second symbol table. Rename plans only the target
//! definition's concrete name literal and occurrences resolved to it, rejects
//! lookalikes outside that plan, then rebuilds the index against a simulated
//! edit before anything is applied.

use crate::byte_to_utf16;
use crate::model::DocModel;
use mathed_core::semantics::{DocumentContext, SemanticIndex};
use mathed_core::transform::{TransformOptions, to_render_text};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::ops::Range;

struct Analysis {
    index: SemanticIndex,
    context: DocumentContext,
}

fn analyze(text: &str) -> Analysis {
    let scan = mathed_core::markers::scan(text);
    let segments = mathed_core::markers::resolve_segments(&scan);
    let render = to_render_text(text, &scan, &segments, &TransformOptions::default());
    let mut index = SemanticIndex::default();
    index.build_index(text, &segments, &[&render]);
    let context = DocumentContext::from_index(text, &scan, &index, &HashMap::new());
    Analysis { index, context }
}

/// Unresolved identifier occurrences from the same `SemanticIndex` the
/// Navigate panel uses. Byte ranges are in document coordinates.
pub(crate) fn unresolved_names(text: &str) -> Vec<(String, Range<usize>)> {
    let analysis = analyze(text);
    analysis
        .index
        .unresolved_occurrences()
        .iter()
        .map(|occurrence| (occurrence.name.clone(), occurrence.range.clone()))
        .collect()
}

fn uses_of(index: &SemanticIndex, def_idx: usize) -> usize {
    index
        .occurrences
        .iter()
        .filter(|occurrence| occurrence.resolved == Some(def_idx))
        .count()
}

fn range_json(text: &str, range: &Range<usize>) -> Value {
    json!([byte_to_utf16(text, range.start), byte_to_utf16(text, range.end)])
}

/// The semantic facts the Navigate panel consumes. UTF-16 offsets match the
/// Java editor; definition indexes match [`SemanticIndex::plan_rename`].
pub fn semantic_json(text: &str) -> Value {
    let analysis = analyze(text);
    let index = &analysis.index;

    let defs: Vec<Value> = index
        .defs
        .iter()
        .enumerate()
        .map(|(def_idx, definition)| {
            let uses: Vec<Value> = index
                .occurrences
                .iter()
                .filter(|occurrence| occurrence.resolved == Some(def_idx))
                .map(|occurrence| {
                    json!({
                        "start": byte_to_utf16(text, occurrence.range.start),
                        "end": byte_to_utf16(text, occurrence.range.end),
                    })
                })
                .collect();
            json!({
                "index": def_idx,
                "name": definition.name,
                "uses": uses,
                "useCount": uses.len(),
                "span": range_json(text, &definition.span),
                "nameRange": definition.name_range.as_ref().map(|range| range_json(text, range)),
            })
        })
        .collect();

    let unresolved: Vec<Value> = index
        .unresolved_occurrences()
        .iter()
        .map(|occurrence| {
            json!({
                "name": occurrence.name,
                "start": byte_to_utf16(text, occurrence.range.start),
                "end": byte_to_utf16(text, occurrence.range.end),
            })
        })
        .collect();

    let sections: Vec<Value> = analysis
        .context
        .blocks
        .iter()
        .enumerate()
        .map(|(block, section)| {
            let heading = if section.heading.is_empty() {
                format!("Block {}", block + 1)
            } else {
                section.heading.clone()
            };
            json!({
                "heading": heading,
                "start": byte_to_utf16(text, section.start),
                "end": byte_to_utf16(text, section.start + section.len),
                "statementCount": section.statement_count,
            })
        })
        .collect();

    let references: Vec<Value> = analysis
        .context
        .references
        .iter()
        .map(|reference| {
            json!({
                "label": reference.label,
                "kind": reference.kind,
                "keys": reference.keys,
            })
        })
        .collect();

    json!({
        "defs": defs,
        "unresolved": unresolved,
        "sections": sections,
        "references": references,
    })
}

/// One mathed/Typst math identifier, not a fragment or expression.
pub fn valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

struct RenamePlan {
    ops: Vec<mathed_core::doc::ReplaceOp>,
    name: String,
    uses: usize,
}

fn apply_ops(text: &str, ops: &[mathed_core::doc::ReplaceOp]) -> Result<String, String> {
    let mut sorted = ops.to_vec();
    sorted.sort_by_key(|op| op.range.start);
    for pair in sorted.windows(2) {
        if pair[1].range.start < pair[0].range.end {
            return Err("Rename ranges overlap".into());
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut end = 0;
    for op in sorted {
        if op.range.start < end || op.range.end > text.len()
            || !text.is_char_boundary(op.range.start)
            || !text.is_char_boundary(op.range.end)
        {
            return Err("Rename range is invalid".into());
        }
        out.push_str(&text[end..op.range.start]);
        out.push_str(&op.with);
        end = op.range.end;
    }
    out.push_str(&text[end..]);
    Ok(out)
}

fn plan(text: &str, def_idx: usize, new_name: &str) -> Result<RenamePlan, String> {
    if !valid_identifier(new_name) {
        return Err("Use one math identifier (letters, digits, underscore)".into());
    }
    let analysis = analyze(text);
    let index = &analysis.index;
    let Some(definition) = index.defs.get(def_idx) else {
        return Err("Definition no longer exists".into());
    };
    if definition.name == new_name {
        return Err("Name is unchanged".into());
    }
    if definition.name_range.is_none() {
        return Err("This definition has no editable name".into());
    }
    for (other_idx, other) in index.defs.iter().enumerate() {
        if other_idx != def_idx && other.name == new_name {
            return Err(format!("‘{new_name}’ is already a definition"));
        }
    }
    let uses = uses_of(index, def_idx);
    let ops = SemanticIndex::plan_rename(index, def_idx, new_name);
    if ops.is_empty() {
        return Err("Definition no longer exists".into());
    }
    for occurrence in &index.occurrences {
        if occurrence.name != new_name {
            continue;
        }
        let planned = ops.iter().any(|op| {
            op.range.start == occurrence.range.start && op.range.end == occurrence.range.end
        });
        if !planned {
            return Err(format!("‘{new_name}’ already occurs outside this rename"));
        }
    }
    let simulated = apply_ops(text, &ops)?;
    let rebuilt = analyze(&simulated);
    if rebuilt.index.defs.len() != index.defs.len() {
        return Err("Rename would change the document's definitions".into());
    }
    let renamed = rebuilt
        .index
        .defs
        .iter()
        .enumerate()
        .filter(|(_, definition)| definition.name == new_name)
        .collect::<Vec<_>>();
    if renamed.len() != 1 || uses_of(&rebuilt.index, renamed[0].0) != uses {
        return Err("Rename would change which definitions resolve".into());
    }
    Ok(RenamePlan {
        ops,
        name: new_name.to_string(),
        uses,
    })
}

/// Validate and preview a rename without mutating the document.
pub fn rename_preview_json(text: &str, def_idx: usize, new_name: &str) -> Value {
    match plan(text, def_idx, new_name) {
        Ok(plan) => json!({
            "ok": true,
            "name": plan.name,
            "uses": plan.uses,
        }),
        Err(error) => json!({ "error": error }),
    }
}

/// Apply the checked plan as one grouped edit. `expected` must still be the
/// exact text the preview saw.
pub fn rename_definition_json(
    model: &mut DocModel,
    expected: &str,
    def_idx: usize,
    new_name: &str,
) -> Result<String, String> {
    if model.text() != expected {
        return Err("The document changed; refresh the rename preview".into());
    }
    let plan = plan(expected, def_idx, new_name)?;
    model.replace_many(plan.ops);
    Ok(crate::state_json(model))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHADOWED: &str = concat!(
        "#1 $norm$ #2 \\def(#1,#2, norm)\n\n",
        "$norm(x)$\n\n",
        "#3 $norm$ #4 \\def(#3,#4, norm)\n\n",
        "$norm(y)$\n"
    );

    #[test]
    fn semantic_index_projects_sections_defs_and_uses() {
        let doc = "= First\n\n#1 $norm$ #2 \\def(#1,#2, norm)\n\n$norm(x)$\n\n= Second\n\nplain";
        let value = semantic_json(doc);
        assert_eq!(value["defs"][0]["name"], "norm");
        assert_eq!(value["defs"][0]["useCount"], 2);
        assert_eq!(value["defs"][0]["uses"].as_array().unwrap().len(), 2);
        assert!(value["sections"].as_array().unwrap().len() >= 2);
        assert_eq!(value["sections"][0]["heading"], "First");
        assert_eq!(value["defs"][0]["nameRange"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn shadowed_same_spelling_definition_is_not_renamed() {
        let idx = analyze(SHADOWED).index;
        assert_eq!(idx.defs.len(), 2);
        let plan = plan(SHADOWED, 0, "magnitude").unwrap();
        assert_eq!(plan.ops.len(), 2, "the target name and its resolved use");
        let simulated = apply_ops(SHADOWED, &plan.ops).unwrap();
        assert_eq!(simulated.matches("\\def(").count(), 2);
        assert!(simulated.contains(", magnitude)"));
        assert!(simulated.contains(", norm)"), "the later same-spelling definition stays");
    }

    #[test]
    fn rename_rejects_bad_duplicate_unresolved_and_unchanged_names() {
        let doc = "#1 $a$ #2 \\def(#1,#2, a)\n\n$a$ $b$ $z$";
        assert!(plan(doc, 0, "two words").is_err());
        assert!(plan(doc, 0, "a").is_err());
        assert!(plan(doc, 0, "b").is_err(), "existing unresolved lookalike");
        assert!(plan(doc, 0, "c").is_ok());
        let duplicate = "#1 $a$ #2 \\def(#1,#2, a)\n\n#3 $c$ #4 \\def(#3,#4, c)";
        assert!(plan(duplicate, 0, "c").is_err());
    }

    #[test]
    fn rename_applies_as_one_grouped_undo_and_updates_uses() {
        let doc = "#1 $a$ #2 \\def(#1,#2, a)\n\n$a(x)$";
        let mut model = DocModel::new(doc);
        let state = rename_definition_json(&mut model, doc, 0, "magnitude").unwrap();
        let value: Value = serde_json::from_str(&state).unwrap();
        assert!(value["text"].as_str().unwrap().contains(", magnitude)"));
        assert!(value["text"].as_str().unwrap().contains("$magnitude(x)$"));
        assert_eq!(value["canUndo"], true);
        model.undo();
        assert_eq!(model.text(), doc);
    }
}
