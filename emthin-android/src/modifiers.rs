//! Links from a marker-delimited modifier to ranges of Loro text.
//!
//! Each modifier has its own Loro mark key, so overlapping modifiers
//! and multiple disjoint targets survive edits and snapshot round trips.
//! The document text remains untouched by adding or removing a link.

use std::ops::Range;

use crate::model::DocModel;

const PREFIX: &str = "prop:modifier:";
const REF_PREFIX: &str = "prop:modref:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub start_id: String,
    pub end_id: String,
    pub target: Range<usize>,
    /// Marker identifiers for a marker-list link; absent for older selection links.
    pub target_ids: Option<(String, String)>,
    /// The Loro mark's current range, used when removing the stored attribute.
    pub stored_targets: Vec<Range<usize>>,
    pub resolved: bool,
}

fn key(start_id: &str, end_id: &str) -> Option<String> {
    if start_id.is_empty()
        || end_id.is_empty()
        || !start_id.chars().all(|c| c.is_ascii_alphanumeric())
        || !end_id.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(format!("{PREFIX}{start_id}:{end_id}"))
}

/// The source range must resolve exactly as mathed resolves marker IDs.
pub fn source_range(model: &DocModel, start_id: &str, end_id: &str) -> Option<Range<usize>> {
    let start = model.scan().markers.iter().find(|m| m.id == start_id)?;
    let end = model.scan().markers.iter().find(|m| m.id == end_id)?;
    (start.range.end <= end.range.start).then_some(start.range.end..end.range.start)
}

pub fn links(model: &DocModel) -> Vec<Link> {
    let mut result: Vec<Link> = Vec::new();
    let mut references = std::collections::HashMap::<String, usize>::new();
    for (stored_target, key) in model.math_doc().segment_marks() {
        if let Some(rest) = key.strip_prefix(REF_PREFIX) {
            if let Some(&index) = references.get(&key) {
                result[index].stored_targets.push(stored_target);
                continue;
            }
            let parts: Vec<&str> = rest.split(':').collect();
            let [start_id, end_id, target_start, target_end] = parts.as_slice() else {
                continue;
            };
            let target = source_range(model, target_start, target_end);
            references.insert(key.clone(), result.len());
            result.push(Link {
                start_id: (*start_id).into(),
                end_id: (*end_id).into(),
                target: target.clone().unwrap_or_else(|| stored_target.clone()),
                target_ids: Some(((*target_start).into(), (*target_end).into())),
                stored_targets: vec![stored_target],
                resolved: target.is_some(),
            });
        } else if let Some((start_id, end_id)) =
            key.strip_prefix(PREFIX).and_then(|s| s.split_once(':'))
        {
            result.push(Link {
                start_id: start_id.into(),
                end_id: end_id.into(),
                target: stored_target.clone(),
                target_ids: None,
                stored_targets: vec![stored_target],
                resolved: true,
            });
        }
    }
    result
}

/// Recognise an older selection link only when one marker pair unambiguously
/// bounds exactly that text (allowing whitespace beside the markers).
pub fn legacy_target_pair(model: &DocModel, target: &Range<usize>) -> Option<(String, String)> {
    let mut found = None;
    for start in &model.scan().markers {
        if start.range.end > target.start
            || !model.text()[start.range.end..target.start]
                .trim()
                .is_empty()
        {
            continue;
        }
        for end in &model.scan().markers {
            if end.range.start < target.end
                || start.range.end > end.range.start
                || !model.text()[target.end..end.range.start].trim().is_empty()
            {
                continue;
            }
            let range = start.range.end..end.range.start;
            if source_range(model, &start.id, &end.id).as_ref() != Some(&range) {
                continue;
            }
            let body = &model.text()[range.clone()];
            let trimmed_start = range.start + body.len() - body.trim_start().len();
            let trimmed_end = range.start + body.trim_end().len();
            if *target != range && *target != (trimmed_start..trimmed_end) {
                continue;
            }
            let pair = (start.id.clone(), end.id.clone());
            if found.as_ref().is_some_and(|previous| previous != &pair) {
                return None;
            }
            found = Some(pair);
        }
    }
    found
}

/// Pairs in a user-entered list. Resolve all pairs before changing the doc.
fn parse_targets(
    model: &DocModel,
    input: &str,
) -> Result<Vec<(String, String, Range<usize>)>, String> {
    let mut result = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if input.trim().is_empty() {
        return Ok(result);
    }
    for item in input.split(',') {
        let words: Vec<&str> = item.split_whitespace().collect();
        if words.len() != 2 {
            return Err("Enter two marker names per interval, separated by commas.".into());
        }
        let start = words[0].strip_prefix('#').unwrap_or(words[0]);
        let end = words[1].strip_prefix('#').unwrap_or(words[1]);
        if key(start, end).is_none() {
            return Err("Marker names must contain only letters and digits.".into());
        }
        let pair = (start.to_owned(), end.to_owned());
        if !seen.insert(pair.clone()) {
            return Err(format!("Interval {start} {end} is repeated."));
        }
        source_range(model, start, end)
            .ok_or_else(|| format!("Markers {start} and {end} must exist in that order."))?;
        let first = model.scan().markers.iter().find(|m| m.id == start).unwrap();
        let last = model.scan().markers.iter().find(|m| m.id == end).unwrap();
        // Include the anchors in the Loro mark. The relationship then
        // survives deleting all text between them; rendering still uses
        // only the resolved body between the markers.
        result.push((pair.0, pair.1, first.range.start..last.range.end));
    }
    Ok(result)
}

pub fn marker_targets(model: &DocModel, start_id: &str, end_id: &str) -> Vec<(String, String)> {
    let mut pairs: Vec<_> = links(model)
        .into_iter()
        .filter(|l| l.start_id == start_id && l.end_id == end_id)
        .filter_map(|l| {
            l.target_ids
                .or_else(|| legacy_target_pair(model, &l.target))
        })
        .collect();
    pairs.sort();
    pairs.dedup();
    pairs
}

/// Atomically replace only marker-list links; existing selection links stay intact.
pub fn set_marker_targets(
    model: &mut DocModel,
    start_id: &str,
    end_id: &str,
    input: &str,
) -> Result<(), String> {
    if key(start_id, end_id).is_none() || source_range(model, start_id, end_id).is_none() {
        return Err("The modifier's markers no longer exist.".into());
    }
    let targets = parse_targets(model, input)?;
    let old: Vec<Link> = links(model)
        .into_iter()
        .filter(|l| {
            l.start_id == start_id
                && l.end_id == end_id
                && (l.target_ids.is_some() || legacy_target_pair(model, &l.target).is_some())
        })
        .collect();
    let mut desired: Vec<_> = targets
        .iter()
        .map(|(a, b, _)| (a.clone(), b.clone()))
        .collect();
    desired.sort();
    if desired == marker_targets(model, start_id, end_id)
        && old.iter().all(|link| link.target_ids.is_some())
    {
        return Ok(());
    }
    let doc = model.math_doc_mut();
    for link in old {
        let key = if let Some((a, b)) = link.target_ids {
            format!("{REF_PREFIX}{start_id}:{end_id}:{a}:{b}")
        } else {
            format!("{PREFIX}{start_id}:{end_id}")
        };
        for range in link.stored_targets {
            doc.unmark_segment(range, &key);
        }
    }
    for (a, b, target) in targets {
        let key = format!("{REF_PREFIX}{start_id}:{end_id}:{a}:{b}");
        doc.mark_segment(target, &key, "1");
    }
    doc.commit();
    Ok(())
}

/// Typst wrappers at link boundaries, derived from Loro without editing text.
/// Crossing intervals are split into nested runs, so the emitted brackets
/// remain balanced regardless of overlap order.
pub fn typst_splices(model: &DocModel) -> std::collections::HashMap<usize, String> {
    typst_splices_in(model, 0..model.len(), None)
}

pub fn typst_splices_in(
    model: &DocModel,
    scope: Range<usize>,
    excluded: Option<(&str, &str)>,
) -> std::collections::HashMap<usize, String> {
    let mut excluded_pairs = std::collections::HashSet::new();
    if let Some((start, end)) = excluded {
        excluded_pairs.insert((start.to_owned(), end.to_owned()));
    }
    typst_splices_excluding(model, scope, &excluded_pairs)
}

pub fn typst_splices_excluding(
    model: &DocModel,
    scope: Range<usize>,
    excluded: &std::collections::HashSet<(String, String)>,
) -> std::collections::HashMap<usize, String> {
    use std::collections::{BTreeSet, HashMap};
    use unicode_segmentation::UnicodeSegmentation;
    let mut graphemes: Vec<usize> = model
        .text()
        .grapheme_indices(true)
        .map(|(i, _)| i)
        .collect();
    graphemes.push(model.len());
    let mut spans = Vec::new();
    for link in links(model) {
        if !link.resolved {
            continue;
        }
        if excluded.contains(&(link.start_id.clone(), link.end_id.clone())) {
            continue;
        }
        let Some(source) = source_range(model, &link.start_id, &link.end_id) else {
            continue;
        };
        let text = model.text()[source].trim();
        if text.is_empty() {
            continue;
        }
        let expression = match text {
            "bold" => "strong",
            "italic" => "emph",
            other => other.strip_prefix('#').unwrap_or(other),
        };
        // Edits can extend a grapheme across an existing mark boundary.
        // Render the whole cluster without changing the underlying mark.
        let start = graphemes
            .iter()
            .copied()
            .take_while(|i| *i <= link.target.start)
            .last()
            .unwrap_or(0);
        let end = graphemes
            .iter()
            .copied()
            .find(|i| *i >= link.target.end)
            .unwrap_or(model.len());
        let start = start.max(scope.start);
        let end = end.min(scope.end);
        if start < end {
            spans.push((start..end, expression.to_owned()));
        }
    }
    let mut boundaries = BTreeSet::new();
    for (range, _) in &spans {
        boundaries.insert(range.start);
        boundaries.insert(range.end);
    }
    let mut result = HashMap::new();
    let mut open = 0;
    for pos in boundaries {
        let mut markup = "]".repeat(open);
        open = 0;
        for (range, expression) in &spans {
            if range.start <= pos && pos < range.end {
                markup.push_str(&format!("#({expression})["));
                open += 1;
            }
        }
        if !markup.is_empty() {
            result.insert(pos, markup);
        }
    }
    result
}

/// Add or remove one link; invalid or empty UTF-8 ranges are rejected.
pub fn set_link(
    model: &mut DocModel,
    start_id: &str,
    end_id: &str,
    target: Range<usize>,
    enabled: bool,
) -> bool {
    let Some(key) = key(start_id, end_id) else {
        return false;
    };
    if source_range(model, start_id, end_id).is_none()
        || target.start >= target.end
        || target.end > model.len()
        || !model.text().is_char_boundary(target.start)
        || !model.text().is_char_boundary(target.end)
    {
        return false;
    }
    let doc = model.math_doc_mut();
    if enabled {
        doc.mark_segment(target, &key, "1");
    } else {
        doc.unmark_segment(target, &key);
    }
    doc.commit();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_follow_edits_and_survive_snapshot_and_undo() {
        let mut model = DocModel::new("#a bold #b hello 👋");
        let at = model.text().find("hello").unwrap();
        assert!(set_link(&mut model, "a", "b", at..at + 5, true));
        assert_eq!(links(&model)[0].target, at..at + 5);
        let snapshot = model.math_doc().snapshot();
        let restored = DocModel::from_snapshot(&snapshot).unwrap();
        assert_eq!(links(&restored), links(&model));
        assert!(model.undo());
        assert!(links(&model).is_empty());
        assert!(model.redo());
        model.insert(0, "x");
        assert_eq!(links(&model)[0].target, at + 1..at + 6);
    }

    #[test]
    fn links_reject_dangling_markers_and_non_utf8_boundaries() {
        let mut model = DocModel::new("#a bold #b 👋");
        let at = model.text().find('👋').unwrap();
        assert!(!set_link(&mut model, "a", "missing", at..at + 4, true));
        assert!(!set_link(&mut model, "a", "b", at + 1..at + 4, true));
        assert!(links(&model).is_empty());
    }

    #[test]
    fn marker_list_round_trips_and_follows_marker_boundaries() {
        let mut model = DocModel::new("#a bold #b #c hello #d #e world #f");
        set_marker_targets(&mut model, "a", "b", "c d, #e #f").unwrap();
        assert_eq!(
            marker_targets(&model, "a", "b"),
            vec![("c".into(), "d".into()), ("e".into(), "f".into())]
        );
        let saved = model.math_doc().snapshot();
        let restored = DocModel::from_snapshot(&saved).unwrap();
        assert_eq!(
            marker_targets(&restored, "a", "b"),
            marker_targets(&model, "a", "b")
        );
        let at = model
            .scan()
            .markers
            .iter()
            .find(|m| m.id == "c")
            .unwrap()
            .range
            .end;
        model.insert(at, " extra");
        let item = links(&model)
            .into_iter()
            .find(|l| l.target_ids.as_ref().is_some_and(|p| p.0 == "c"))
            .unwrap();
        assert!(model.text()[item.target].contains("extra"));
    }

    #[test]
    fn invalid_marker_lists_do_not_change_existing_links() {
        let mut model = DocModel::new("#a bold #b #c hello #d");
        set_marker_targets(&mut model, "a", "b", "c d").unwrap();
        for invalid in ["c", "c missing", "d c", "c d, c d", "c d,"] {
            assert!(set_marker_targets(&mut model, "a", "b", invalid).is_err());
            assert_eq!(
                marker_targets(&model, "a", "b"),
                vec![("c".into(), "d".into())]
            );
        }
        set_marker_targets(&mut model, "a", "b", "").unwrap();
        assert!(marker_targets(&model, "a", "b").is_empty());
    }

    #[test]
    fn marker_link_survives_an_empty_target_and_can_be_undone() {
        let mut model = DocModel::new("#a bold #b #c hello #d");
        set_marker_targets(&mut model, "a", "b", "c d").unwrap();
        assert!(model.undo());
        assert!(marker_targets(&model, "a", "b").is_empty());
        assert!(model.redo());
        let body = source_range(&model, "c", "d").unwrap();
        model.replace(body, "");
        assert_eq!(
            marker_targets(&model, "a", "b"),
            vec![("c".into(), "d".into())]
        );
        assert!(links(&model)[0].target.is_empty());
    }

    #[test]
    fn marker_list_converts_only_unambiguously_anchored_old_links() {
        let mut model = DocModel::new("#a bold #b #c hello #d plain");
        let hello = model.text().find("hello").unwrap();
        let plain = model.text().find("plain").unwrap();
        assert!(set_link(&mut model, "a", "b", hello..hello + 5, true));
        assert!(set_link(&mut model, "a", "b", plain..plain + 5, true));
        assert_eq!(
            marker_targets(&model, "a", "b"),
            vec![("c".into(), "d".into())]
        );
        set_marker_targets(&mut model, "a", "b", "c d").unwrap();
        let links = links(&model);
        assert_eq!(links.len(), 2);
        assert!(links
            .iter()
            .any(|l| l.target_ids.is_none() && l.target == (plain..plain + 5)));
        assert!(links.iter().any(|l| l
            .target_ids
            .as_ref()
            .is_some_and(|p| p.0 == "c" && p.1 == "d")));
    }

    #[test]
    fn marker_link_split_by_another_mark_is_one_link_and_removes_fully() {
        let mut model = DocModel::new("#a bold #b #e italic #f #c hello #d");
        set_marker_targets(&mut model, "a", "b", "c d").unwrap();
        let at = model.text().find("hello").unwrap();
        assert!(set_link(&mut model, "e", "f", at + 1..at + 4, true));
        assert_eq!(
            links(&model)
                .iter()
                .filter(|link| link.start_id == "a")
                .count(),
            1
        );
        set_marker_targets(&mut model, "a", "b", "").unwrap();
        assert!(marker_targets(&model, "a", "b").is_empty());
        assert_eq!(links(&model).len(), 1);
        assert_eq!(links(&model)[0].start_id, "e");
    }

    #[test]
    fn distinct_modifiers_can_overlap_and_have_disjoint_targets() {
        let mut model = DocModel::new("#a strong #b #c emph #d one two three");
        let one = model.text().find("one").unwrap();
        let three = model.text().find("three").unwrap();
        assert!(set_link(&mut model, "a", "b", one..one + 7, true));
        assert!(set_link(&mut model, "c", "d", one + 4..three + 5, true));
        assert!(set_link(&mut model, "a", "b", three..three + 5, true));
        let found = links(&model);
        assert_eq!(found.iter().filter(|link| link.start_id == "a").count(), 2);
        assert!(found
            .iter()
            .any(|link| link.start_id == "c" && link.target == (one + 4..three + 5)));
        assert!(set_link(&mut model, "a", "b", one..one + 7, false));
        assert_eq!(links(&model).len(), 2);
    }

    #[test]
    fn typst_conventions_render_without_writing_statements() {
        let mut model = DocModel::new("#a bold #b hello");
        let at = model.text().find("hello").unwrap();
        assert!(set_link(&mut model, "a", "b", at..at + 5, true));
        let splices = typst_splices(&model);
        assert_eq!(splices.get(&at).map(String::as_str), Some("#(strong)["));
        assert_eq!(splices.get(&(at + 5)).map(String::as_str), Some("]"));
        let actual = mathed_mini::render_markup("#(strong)[hello]", 200.0).unwrap();
        let expected = mathed_mini::render_markup("#strong[hello]", 200.0).unwrap();
        assert_eq!(actual.data, expected.data);
        assert!(!model.text().contains("\\bold("));
    }

    #[test]
    fn rendering_expands_a_mark_to_cover_a_combining_cluster() {
        let mut model = DocModel::new("#a bold #b e\u{301}");
        let at = model.text().find('e').unwrap();
        assert!(set_link(&mut model, "a", "b", at..at + 1, true));
        let splices = typst_splices(&model);
        assert!(splices.contains_key(&(at + "e\u{301}".len())));
        assert!(!splices.contains_key(&(at + 1)));
    }
}
