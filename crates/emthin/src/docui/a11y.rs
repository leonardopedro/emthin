//! Desktop rendered-text accessibility projection (parked item).
//!
//! `mathed_mini::a11y` is feature-gated behind `gui`, which emthin deliberately
//! disables, so this module builds the same read-only tree data from
//! `mathed_core::accessibility::build_access_nodes`, which is always available.
//! It is the testable desktop authority: a flat, ordered list of nodes with
//! stable byte ranges and roles that a control client or an AT bridge can map
//! to a platform tree. No platform API is called here.

use mathed_core::accessibility::{build_access_nodes, AccessNode};
use mathed_core::semantics::SemanticIndex;
use mathed_core::transform::{to_render_text, TransformOptions};

/// One accessible node, mirroring `mathed_core::accessibility::AccessNode`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub role: String,
    pub label: String,
    pub value: Option<String>,
    pub range: Option<std::ops::Range<usize>>,
}

impl From<AccessNode> for Node {
    fn from(node: AccessNode) -> Self {
        Self {
            role: node.role.as_str().to_string(),
            label: node.label,
            value: node.value,
            range: node.range,
        }
    }
}

/// The read-only accessibility index for one document revision.
#[derive(Debug, Clone)]
pub struct A11yIndex {
    pub nodes: Vec<Node>,
}

impl A11yIndex {
    /// Build the node list from the document facts DocUi owns.
    pub fn build(
        text: &str,
        scan: &mathed_core::markers::MarkerScan,
        segments: &[mathed_core::markers::Segment],
    ) -> Self {
        let render = to_render_text(text, scan, segments, &TransformOptions::default());
        let mut index = SemanticIndex::default();
        index.build_index(text, segments, &[&render]);
        let nodes = build_access_nodes(text, segments, &index)
            .into_iter()
            .map(Node::from)
            .collect();
        Self { nodes }
    }

    /// The node whose source range contains `byte`, if any.
    pub fn node_at(&self, byte: usize) -> Option<&Node> {
        self.nodes.iter().find(|node| {
            node.range
                .as_ref()
                .is_some_and(|range| range.contains(&byte))
        })
    }

    /// Focusable nodes: those with a source range, in document order.
    pub fn focusable(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter().filter(|node| node.range.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(text: &str) -> A11yIndex {
        let scan = mathed_core::markers::scan(text);
        let segments = mathed_core::markers::resolve_segments(&scan);
        A11yIndex::build(text, &scan, &segments)
    }

    #[test]
    fn a_marked_document_exposes_nodes_in_source_order() {
        let idx = index("#a bold #b and an #c undefined #d\n$math$");
        assert!(!idx.nodes.is_empty());
        for pair in idx.nodes.windows(2) {
            let a = pair[0].range.as_ref().map(|r| r.start);
            let b = pair[1].range.as_ref().map(|r| r.start);
            if let (Some(a), Some(b)) = (a, b) {
                assert!(a <= b, "nodes must be source-ordered");
            }
        }
    }

    #[test]
    fn node_at_finds_a_segment_range() {
        let idx = index("#a bold #b \\bold(#a,#b)");
        let node = idx.nodes.first().expect("a node");
        let range = node.range.clone().expect("a ranged node");
        assert_eq!(
            idx.node_at(range.start).map(|n| n.label.as_str()),
            Some(node.label.as_str())
        );
    }

    #[test]
    fn every_focusable_node_has_a_source_range() {
        let idx = index("#a bold #b $x$");
        assert!(idx.focusable().all(|node| node.range.is_some()));
    }
}
