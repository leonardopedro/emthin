//! Read-only proof and reference queries (Kyneta K8).
//!
//! The index is a projection over mathed's existing `SemanticIndex`, marker
//! pairs and `\formal` specs — no second parser and no separate symbol table.
//! A proof result is an independently fetched fact with explicit ownership; a
//! late or foreign result cannot overwrite the live view. Queries are read-only.

use std::collections::{BTreeMap, BTreeSet};

use mathed_core::doc::MathDoc;
use mathed_core::markers::{MarkerScan, Segment};
use mathed_core::semantics::SemanticIndex;
use mathed_core::transform::{to_render_text, TransformOptions};

/// Stable identity of the document/share an index belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewOwner {
    pub share_id: String,
    pub document_key: String,
}

impl ViewOwner {
    pub fn new(share_id: impl Into<String>, document_key: impl Into<String>) -> Self {
        Self {
            share_id: share_id.into(),
            document_key: document_key.into(),
        }
    }

    fn matches(&self, other: &Self) -> bool {
        self.share_id == other.share_id && self.document_key == other.document_key
    }
}

/// One definition row in the proof view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionRow {
    pub name: String,
    pub start: usize,
    pub end: usize,
    pub uses: usize,
}

/// One reference row, with whether its target resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceRow {
    pub name: String,
    pub start: usize,
    pub end: usize,
    pub target: Option<String>,
}

/// A verified proof result fetched independently of the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofResult {
    pub owner: ViewOwner,
    pub proof_id: String,
    /// Read-only fact text, e.g. a checker verdict. Never document content.
    pub verdict: String,
}

/// The read-only query result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofView {
    pub owner: ViewOwner,
    /// A content generation or digest so a caller can tell a rebuilt view
    /// from a stale one.
    pub revision: String,
    pub definitions: Vec<DefinitionRow>,
    pub references: Vec<ReferenceRow>,
    pub missing: Vec<String>,
    /// The proof id the document's own `\formal` statement cites. A result is
    /// accepted only for this id, and it survives accepting a result.
    pub expected_proof: Option<String>,
    /// The last accepted proof result, only for this owner/proof.
    pub proof: Option<ProofResult>,
}

impl ProofView {
    /// Apply an independently fetched result only when its ownership matches
    /// this view and the proof id is the one this note references. A late
    /// result for another proof or document is ignored, and a view whose
    /// document cites no proof accepts none.
    pub fn apply_result(&mut self, result: ProofResult) -> bool {
        if !result.owner.matches(&self.owner) {
            return false;
        }
        if self.expected_proof.as_deref() != Some(result.proof_id.as_str()) {
            return false;
        }
        self.proof = Some(result);
        true
    }

    /// The proof id cited by the document, if any.
    pub fn proof_id(&self) -> Option<&str> {
        self.expected_proof.as_deref()
    }

    /// Bounded transitive dependencies from `name`, deduplicated and cycle
    /// safe. Only the direct reference edges in the semantic index are used.
    pub fn dependencies(&self, name: &str, limit: usize) -> Vec<String> {
        let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for definition in &self.definitions {
            edges.entry(definition.name.clone()).or_default();
        }
        for reference in &self.references {
            if let Some(target) = &reference.target {
                edges
                    .entry(reference.name.clone())
                    .or_default()
                    .insert(target.clone());
            }
        }
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        let mut queue = vec![name.to_string()];
        while let Some(current) = queue.pop() {
            if out.len() >= limit {
                break;
            }
            if !seen.insert(current.clone()) {
                continue;
            }
            if let Some(next) = edges.get(&current) {
                for dependency in next {
                    if !seen.contains(dependency) {
                        queue.push(dependency.clone());
                    }
                }
            }
            if current != name {
                out.push(current);
            }
        }
        out.sort();
        out
    }
}

// ----- K9 rule candidate (evaluated, not adopted) ---------------------

/// Narrow versioned ABI for a pure advisory rule. No host I/O, bounded typed
/// facts and an explicit error value. The K9 card allows keeping the Rust
/// implementation when an Austral variant shows no concrete benefit.
pub const RULE_ABI_VERSION: u32 = 1;

/// One validated reference edge. Both ends are opaque ids, never document text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkFact {
    pub from: String,
    pub to: String,
}

/// The rule's result: either every edge resolves or the missing targets are
/// listed. Malformed input is an explicit value, not a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleResult {
    Complete { facts: usize },
    Incomplete { missing: Vec<String> },
    Malformed,
}

/// Pure rule: derive proof-dependency completeness from validated link facts.
/// It reads no document, file or clock and has no side effects.
pub fn proof_dependency_complete(facts: &[LinkFact]) -> RuleResult {
    if facts.len() > 4096 {
        return RuleResult::Malformed;
    }
    let mut declared: BTreeSet<&str> = BTreeSet::new();
    for fact in facts {
        if fact.from.is_empty() || fact.to.is_empty() {
            return RuleResult::Malformed;
        }
        declared.insert(fact.from.as_str());
    }
    let mut missing: Vec<String> = facts
        .iter()
        .filter(|fact| !declared.contains(fact.to.as_str()))
        .map(|fact| fact.to.clone())
        .collect();
    missing.sort();
    missing.dedup();
    if missing.is_empty() {
        RuleResult::Complete { facts: facts.len() }
    } else {
        RuleResult::Incomplete { missing }
    }
}

/// Build a view from the shared document facts. `revision` is supplied by the
/// caller (a content digest or generation) so a caller can compare a
/// maintained view against a full rebuild on the same fixture.
pub fn build_view(
    doc: &MathDoc,
    scan: &MarkerScan,
    segments: &[Segment],
    owner: ViewOwner,
    revision: String,
) -> ProofView {
    let render = to_render_text(doc.text(), scan, segments, &TransformOptions::default());
    let mut index = SemanticIndex::default();
    index.build_index(doc.text(), segments, &[&render]);

    let unquote = |name: &str| name.trim_matches('"').to_string();

    let mut definition_uses: BTreeMap<String, usize> = BTreeMap::new();
    for occurrence in &index.occurrences {
        if let Some(target) = occurrence.resolved {
            if let Some(definition) = index.defs.get(target) {
                *definition_uses
                    .entry(unquote(&definition.name))
                    .or_default() += 1;
            }
        }
    }

    let definitions: Vec<DefinitionRow> = index
        .defs
        .iter()
        .map(|definition| DefinitionRow {
            name: unquote(&definition.name),
            start: definition.span.start,
            end: definition.span.end,
            // `definition_uses` is keyed by the unquoted name; looking it up
            // by the raw name reports zero uses for every quoted `\def`.
            uses: definition_uses
                .get(&unquote(&definition.name))
                .copied()
                .unwrap_or(0),
        })
        .collect();

    // A resolved occurrence names the definition the shared index resolved it
    // to, which need not be the occurrence's own spelling (an unnamed `\def`
    // takes its name from the caption body). Report that target rather than a
    // second name lookup, which would report `null` for a resolved reference.
    let references: Vec<ReferenceRow> = index
        .occurrences
        .iter()
        .map(|occurrence| ReferenceRow {
            name: occurrence.name.clone(),
            start: occurrence.range.start,
            end: occurrence.range.end,
            target: occurrence
                .resolved
                .and_then(|resolved| index.defs.get(resolved))
                .map(|definition| unquote(&definition.name)),
        })
        .collect();

    let missing: Vec<String> = index
        .unresolved_occurrences()
        .into_iter()
        .map(|occurrence| occurrence.name)
        .collect();

    // A `\formal` statement can cite a proof id; it becomes the expected id
    // of an independently fetched result. The view starts with no result.
    // `resolve_formal` does not filter by kind, so a `\def` must not be
    // mistaken for the note's proof.
    let expected_proof = segments
        .iter()
        .filter(|segment| segment.kind == mathed_core::markers::PropKind::Formal)
        .find_map(|segment| mathed_core::formalize::resolve_formal(segment).map(|r| r.key));
    ProofView {
        owner,
        revision,
        definitions,
        references,
        missing,
        expected_proof,
        proof: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> MathDoc {
        MathDoc::with_text(
            "#a sort #b \\def(#a,#b,\"sort\")\n\
             #c prop #d \\def(#c,#d,\"prop\")\n\
             #e uses #f \\ref(#e,#f,sort)\n\
             #g claim #h \\formal(#g,#h,\"sort Prop\")\n\
             $ghost$",
        )
    }

    fn view(doc: &MathDoc) -> ProofView {
        let scan = mathed_core::markers::scan(doc.text());
        let segments = mathed_core::markers::resolve_segments(&scan);
        build_view(
            doc,
            &scan,
            &segments,
            ViewOwner::new("share", "proof-note"),
            "rev1".to_string(),
        )
    }

    #[test]
    fn the_view_lists_definitions_references_and_missing_targets() {
        let doc = fixture();
        let view = view(&doc);
        assert!(
            view.definitions.iter().any(|row| row.name == "sort"),
            "{:?}",
            view.definitions
        );
        assert!(!view.references.is_empty(), "{:?}", view.references);
        // An occurrence that resolves to no definition is reported missing.
        assert!(
            view.missing.iter().any(|name| name == "ghost"),
            "{:?}",
            view.missing
        );
    }

    #[test]
    fn a_maintained_view_matches_a_full_rebuild_on_the_same_fixture() {
        let doc = fixture();
        let mut maintained = view(&doc);
        let rebuilt = view(&doc);
        assert_eq!(maintained, rebuilt);
        // A rewrite changes the affected row after a rebuild.
        let mut changed = MathDoc::from_snapshot(&doc.snapshot()).unwrap();
        let at = changed.text().find("\"sort\"").unwrap();
        changed.replace(at..at + 6, "\"kind\"");
        let rebuilt = view(&changed);
        assert!(rebuilt.definitions.iter().any(|row| row.name == "kind"));
        maintained = rebuilt.clone();
        assert_eq!(maintained, rebuilt);
    }

    #[test]
    fn a_mark_change_does_not_change_the_text_rows_but_does_change_the_revision() {
        let mut doc = fixture();
        let before = view(&doc);
        let scan = mathed_core::markers::scan(doc.text());
        let _ = scan;
        doc.mark_segment(0..4, "prop:bold", "yes");
        doc.commit();
        let after = view(&doc);
        assert_eq!(before.definitions, after.definitions);
        assert_eq!(before.references, after.references);
        assert_eq!(
            before.revision, after.revision,
            "revision is supplied by the caller"
        );
    }

    #[test]
    fn a_document_with_no_formal_statement_accepts_no_proof_result() {
        let doc = MathDoc::with_text("#1 a proof #2 \\def(#1,#2,proof)");
        // `resolve_formal` accepts any spanned statement, so the filter that
        // keeps a `\def` from becoming the expected proof matters here.
        let mut view = view(&doc);
        assert_eq!(view.proof_id(), None);
        let result = ProofResult {
            owner: view.owner.clone(),
            proof_id: "anything".into(),
            verdict: "verified".into(),
        };
        assert!(!view.apply_result(result));
        assert!(view.proof.is_none());
    }

    #[test]
    fn a_quoted_definition_counts_its_uses() {
        // The `\def` name keeps its quotes in the raw arg text, so the use
        // count must key off the unquoted name.
        let doc = MathDoc::with_text("#1 $quoted$ #2 \\def(#1,#2,\"quoted\")");
        let view = view(&doc);
        let row = view
            .definitions
            .iter()
            .find(|row| row.name == "quoted")
            .expect("the quoted definition is listed");
        assert!(row.uses >= 1, "{row:?}");
    }

    #[test]
    fn a_resolved_reference_reports_the_definition_it_resolved_to() {
        // An unnamed `\def` takes its name from the caption body, so the
        // occurrence spelling and the definition name differ.
        let doc = MathDoc::with_text("#1 my $name$ #2 \\def(#1,#2)");
        let view = view(&doc);
        let resolved: Vec<_> = view
            .references
            .iter()
            .filter(|row| row.target.is_some())
            .collect();
        assert!(!resolved.is_empty(), "{:?}", view.references);
        for row in resolved {
            // `name` resolved to the unnamed definition, whose name is the
            // caption body: a plain name lookup would have reported `null`.
            let target = row.target.as_deref().unwrap();
            assert!(target.contains("name"), "{target:?}");
            assert_ne!(target, row.name);
        }
    }

    #[test]
    fn a_stale_or_foreign_proof_result_is_ignored() {
        let doc = fixture();
        let mut view = view(&doc);
        let proof_id = view.proof_id().unwrap().to_string();
        let foreign = ProofResult {
            owner: ViewOwner::new("other-share", "proof-note"),
            proof_id: proof_id.clone(),
            verdict: "verified".into(),
        };
        assert!(!view.apply_result(foreign));
        assert!(view.proof.is_none(), "a refused result is not installed");

        let stale = ProofResult {
            owner: ViewOwner::new("share", "proof-note"),
            proof_id: "wrong-proof".into(),
            verdict: "verified".into(),
        };
        assert!(!view.apply_result(stale));

        let current = ProofResult {
            owner: ViewOwner::new("share", "proof-note"),
            proof_id,
            verdict: "verified".into(),
        };
        assert!(view.apply_result(current));
        assert_eq!(view.proof.unwrap().verdict, "verified");
    }

    #[test]
    fn the_rule_candidate_is_pure_and_has_an_explicit_error_value() {
        let facts = vec![
            LinkFact {
                from: "a".into(),
                to: "b".into(),
            },
            LinkFact {
                from: "b".into(),
                to: "a".into(),
            },
        ];
        assert_eq!(
            proof_dependency_complete(&facts),
            RuleResult::Complete { facts: 2 }
        );
        assert_eq!(
            proof_dependency_complete(&[LinkFact {
                from: "a".into(),
                to: "ghost".into()
            }]),
            RuleResult::Incomplete {
                missing: vec!["ghost".into()]
            }
        );
        assert_eq!(
            proof_dependency_complete(&[LinkFact {
                from: "".into(),
                to: "b".into()
            }]),
            RuleResult::Malformed
        );
        let huge = vec![
            LinkFact {
                from: "a".into(),
                to: "b".into()
            };
            4097
        ];
        assert_eq!(proof_dependency_complete(&huge), RuleResult::Malformed);
        assert_eq!(RULE_ABI_VERSION, 1);
    }

    #[test]
    fn a_dependency_query_deduplicates_and_handles_cycles() {
        let mut view = view(&fixture());
        view.definitions = vec![
            DefinitionRow {
                name: "a".into(),
                start: 0,
                end: 1,
                uses: 0,
            },
            DefinitionRow {
                name: "b".into(),
                start: 0,
                end: 1,
                uses: 0,
            },
        ];
        view.references = vec![
            ReferenceRow {
                name: "a".into(),
                start: 0,
                end: 1,
                target: Some("b".into()),
            },
            ReferenceRow {
                name: "b".into(),
                start: 0,
                end: 1,
                target: Some("a".into()),
            },
        ];
        let deps = view.dependencies("a", 8);
        assert_eq!(deps, vec!["b".to_string()], "the cycle is visited once");
        assert_eq!(view.dependencies("a", 0), Vec::<String>::new());
    }
}
