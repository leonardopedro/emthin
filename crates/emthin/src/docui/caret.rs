use mathed_core::GlyphIndex;
use unicode_segmentation::GraphemeCursor;

/// Resolve a glyph edge to a source insertion point. A glyph can refer to
/// part of a grapheme, but the editor should never split that grapheme.
pub(super) fn resolve_edge(text: &str, byte: usize, after: bool) -> usize {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    if text.as_bytes().get(byte) == Some(&b'\n') {
        return byte;
    }
    let mut cursor = GraphemeCursor::new(byte, text.len(), true);
    if after {
        cursor.next_boundary(text, 0).ok().flatten().unwrap_or(byte)
    } else if cursor.is_boundary(text, 0) == Ok(false) {
        cursor.prev_boundary(text, 0).ok().flatten().unwrap_or(byte)
    } else {
        byte
    }
}

/// Move to the closest insertion edge on the adjacent visual line. Return
/// the original column as well, so a short line does not reset it.
pub(super) fn vertical_target(
    glyphs: &GlyphIndex,
    text: &str,
    caret: usize,
    down: bool,
    goal: Option<f32>,
) -> Option<(usize, f32)> {
    let current = glyphs.band_for_byte(caret)?;
    let target = if down {
        current.checked_add(1)?
    } else {
        current.checked_sub(1)?
    };
    let x = goal.unwrap_or(glyphs.caret_for_byte(caret)?.x);
    let (byte, after, _) = glyphs
        .entries
        .iter()
        .filter(|entry| entry.band as usize == target)
        .flat_map(|entry| {
            [
                (entry.doc_byte, false, (x - entry.x).abs()),
                (entry.doc_byte, true, (x - entry.x - entry.advance).abs()),
            ]
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))?;
    Some((resolve_edge(text, byte, after), x))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mathed_core::glyphs::{GlyphEntry, LineBand};

    #[test]
    fn edge_clicks_keep_combining_marks_and_emoji_together() {
        let text = "a\u{301}👩‍🔬\n";
        let emoji = "a\u{301}".len();
        let newline = text.len() - 1;
        assert_eq!(resolve_edge(text, 1, false), 0);
        assert_eq!(resolve_edge(text, 1, true), emoji);
        assert_eq!(resolve_edge(text, emoji, true), newline);
        assert_eq!(resolve_edge(text, newline, true), newline);
    }

    #[test]
    fn a_short_line_preserves_the_goal_column() {
        let entries = [
            (0, 0, 0.),
            (1, 0, 10.),
            (2, 0, 20.),
            (4, 1, 0.),
            (6, 2, 0.),
            (7, 2, 10.),
            (8, 2, 20.),
        ]
        .into_iter()
        .map(|(doc_byte, band, x)| GlyphEntry {
            doc_byte,
            band,
            x,
            advance: 10.,
        })
        .collect();
        let glyphs = GlyphIndex {
            entries,
            bands: (0..3)
                .map(|i| {
                    let top = i as f32 * 10.;
                    LineBand {
                        top,
                        bottom: top + 10.,
                        baseline: top + 8.,
                    }
                })
                .collect(),
        };
        let text = "ABC\nX\nDEF";
        let (short, goal) = vertical_target(&glyphs, text, 2, true, None).unwrap();
        assert_eq!(short, 5, "the short line ends after X");
        assert_eq!(
            vertical_target(&glyphs, text, short, true, Some(goal)),
            Some((8, 20.))
        );
        assert_eq!(vertical_target(&glyphs, text, 2, false, None), None);
        assert_eq!(vertical_target(&glyphs, text, 8, true, None), None);
    }
}
