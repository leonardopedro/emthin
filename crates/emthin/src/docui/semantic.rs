//! Semantic outline, definition/use navigation and checked rename (parked item).
//!
//! A projection over mathed's existing `SemanticIndex` and marker segments —
//! no second parser or symbol table. Ranges are UTF-8 document bytes, the same
//! coordinate the caret and glyph index use. Rename reuses
//! `SemanticIndex::plan_rename` and is applied by the caller as one checked
//! grouped edit.

use std::ops::Range;

use mathed_core::doc::ReplaceOp;
use mathed_core::semantics::SemanticIndex;
use mathed_core::transform::{to_render_text, TransformOptions};

/// One definition row in the outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionRow {
    pub index: usize,
    pub name: String,
    pub uses: usize,
    pub span: Range<usize>,
    pub name_range: Option<Range<usize>>,
}

/// One use site of a definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseRow {
    pub name: String,
    pub range: Range<usize>,
    pub definition: usize,
}

/// A read-only semantic view of the current document. It retains the index it
/// was built from so rename can reuse mathed's own rename plan.
#[derive(Debug, Clone)]
pub struct SemanticView {
    pub definitions: Vec<DefinitionRow>,
    pub unresolved: Vec<UseRow>,
    pub uses: Vec<UseRow>,
    index: SemanticIndex,
}

impl SemanticView {
    /// Build a view from the document facts DocUi already owns.
    pub fn build(
        text: &str,
        scan: &mathed_core::markers::MarkerScan,
        segments: &[mathed_core::markers::Segment],
    ) -> Self {
        let render = to_render_text(text, scan, segments, &TransformOptions::default());
        let mut index = SemanticIndex::default();
        index.build_index(text, segments, &[&render]);
        let mut uses = Vec::new();
        let mut unresolved = Vec::new();
        for occurrence in &index.occurrences {
            let row = UseRow {
                name: occurrence.name.clone(),
                range: occurrence.range.clone(),
                definition: occurrence.resolved.unwrap_or(usize::MAX),
            };
            match occurrence.resolved {
                Some(_) => uses.push(row),
                None => unresolved.push(row),
            }
        }
        uses.sort_by_key(|row| row.range.start);
        unresolved.sort_by_key(|row| row.range.start);
        let definitions = index
            .defs
            .iter()
            .enumerate()
            .map(|(index, definition)| DefinitionRow {
                index,
                name: definition.name.clone(),
                uses: uses.iter().filter(|row| row.definition == index).count(),
                span: definition.span.clone(),
                name_range: definition.name_range.clone(),
            })
            .collect();
        Self {
            definitions,
            unresolved,
            uses,
            index,
        }
    }

    /// Definition whose name range contains `byte`, or whose span contains it.
    pub fn definition_at(&self, byte: usize) -> Option<usize> {
        self.definitions
            .iter()
            .find(|row| {
                row.name_range
                    .as_ref()
                    .is_some_and(|range| range.contains(&byte))
                    || row.span.contains(&byte)
            })
            .map(|row| row.index)
    }

    /// Use site at `byte` (resolved to a definition).
    pub fn use_at(&self, byte: usize) -> Option<&UseRow> {
        self.uses.iter().find(|row| row.range.contains(&byte))
    }

    /// Plan a rename using mathed's own checked planner. Returns `None` for an
    /// empty name or an unknown definition; the caller applies the ops as one
    /// grouped edit.
    pub fn plan_rename(&self, def_idx: usize, new_name: &str) -> Option<Vec<ReplaceOp>> {
        if new_name.trim().is_empty() || def_idx >= self.definitions.len() {
            return None;
        }
        let ops = SemanticIndex::plan_rename(&self.index, def_idx, new_name);
        (!ops.is_empty()).then_some(ops)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(text: &str) -> SemanticView {
        let scan = mathed_core::markers::scan(text);
        let segments = mathed_core::markers::resolve_segments(&scan);
        SemanticView::build(text, &scan, &segments)
    }

    #[test]
    fn a_view_lists_definitions_and_uses() {
        let view = view("#a sort #b \\def(#a,#b, sort)\n$sort$");
        assert_eq!(view.definitions.len(), 1);
        assert!(!view.uses.is_empty());
        assert_eq!(view.definitions[0].uses, view.uses.len());
    }

    #[test]
    fn definition_at_accepts_name_and_body() {
        let view = view("#a sort #b \\def(#a,#b, sort)");
        let row = &view.definitions[0];
        let at_name = row.name_range.as_ref().unwrap().start;
        assert_eq!(view.definition_at(at_name), Some(0));
        assert_eq!(view.definition_at(row.span.start), Some(0));
    }

    #[test]
    fn rename_plan_covers_name_and_uses() {
        let view = view("#a sort #b \\def(#a,#b, sort)\n$sort$");
        let ops = view.plan_rename(0, "kind").expect("a rename plan");
        // The definition's name plus the `$sort$` use both change.
        assert!(ops.len() >= 2, "expected name + uses, got {}", ops.len());
        for op in &ops {
            assert_eq!(op.with, "kind");
        }
    }

    #[test]
    fn rename_plan_rejects_empty_names_and_unknown_definitions() {
        let view = view("#a sort #b \\def(#a,#b, sort)");
        assert!(view.plan_rename(0, "").is_none());
        assert!(view.plan_rename(9, "x").is_none());
    }

    #[test]
    fn use_at_finds_a_resolved_occurrence() {
        let view = view("#a sort #b \\def(#a,#b, sort)\n$sort$");
        let row = view.uses.first().expect("a use");
        assert_eq!(
            view.use_at(row.range.start).map(|u| u.name.as_str()),
            Some("sort")
        );
    }
}
