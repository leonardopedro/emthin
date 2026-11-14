//! ASCII-only Typst interchange projection.
//!
//! Mathed's U4 export lives in `mathed_mini::export`, but that module is
//! behind the crate's `kernel` feature — the dependency tree emthin
//! deliberately drops. The projection itself is pure text work, so it is
//! kept here on the same `mathed_core` seams and the same canonical
//! [`mathed_core::tables`] mapping: math glyphs become their Typst names,
//! everything else non-ASCII becomes an explicit `\u{HEX}` literal.
//!
//! The source document stays canonical. This is a lossy-by-design
//! projection of the *rendered* text: hidden markers and `\name(...)`
//! statements are resolved away by the transform, and an imported
//! projection therefore reconstructs the visible document, not its
//! structure.

use mathed_core::markers::{resolve_segments, scan};
use mathed_core::transform::{to_render_text, TransformOptions};

/// Render `doc_text` and map every non-ASCII code point to ASCII.
///
/// Inside math, glyphs with a backslash-name in the canonical table use
/// that name (`α` → `\alpha`, valid Typst math); every other non-ASCII
/// character becomes `\u{HEX}`, which is total and never silently drops a
/// glyph. An escaped `$` does not toggle math, matching the transform's
/// own escape rules.
pub fn export(doc_text: &str) -> String {
    let scan = scan(doc_text);
    let segments = resolve_segments(&scan);
    let render = to_render_text(doc_text, &scan, &segments, &TransformOptions::default());

    let mut out = String::with_capacity(render.text.len());
    let mut in_math = false;
    let mut escaped = false;
    for c in render.text.chars() {
        if escaped {
            out.push(c);
            escaped = false;
            continue;
        }
        if c == '\\' {
            out.push(c);
            escaped = true;
            continue;
        }
        if c == '$' {
            in_math = !in_math;
            out.push(c);
            continue;
        }
        if c.is_ascii() {
            out.push(c);
            continue;
        }
        if in_math {
            if let Some(name) = mathed_core::tables::ascii_of(c) {
                out.push_str(name);
                continue;
            }
        }
        out.push_str(&format!("\\u{{{:X}}}", c as u32));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn math_glyphs_use_typst_names_and_output_is_ascii_only() {
        let out = export("= 数学\n\n$ αβ + π $");
        assert!(out.is_ascii(), "projection must be ASCII-only: {out}");
        assert!(out.contains("\\alpha\\beta"), "math names: {out}");
        assert!(out.contains("\\pi"), "math names: {out}");
    }

    #[test]
    fn unmapped_glyphs_are_flagged_and_operator_forms_have_no_inverse() {
        let out = export("A crab 🦀 and an arrow →");
        assert!(out.is_ascii(), "{out}");
        assert!(out.contains("\\u{1F980}"), "emoji flagged: {out}");
        assert!(out.contains("\\u{2192}"), "operator arrow flagged: {out}");
    }

    #[test]
    fn escaped_dollar_does_not_toggle_math_and_projection_is_idempotent() {
        let out = export("$ a \\$ α $");
        assert!(out.contains("\\alpha"), "still math after escaped $: {out}");
        assert_eq!(export(&out), out, "an ASCII projection is a fixed point");
    }
}
