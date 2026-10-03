//! Document edits the *compositor* makes on the user's behalf: binding
//! an app to a figure, resizing one, and appending a figure for a
//! toplevel that arrived with nowhere to go.
//!
//! These are pure functions over a [`DocModel`] so the interesting part
//! — which byte ranges to rewrite, and what text to write — is unit
//! testable without a compositor.

use std::ops::Range;

use mathed_core::figures::{resolve_figure, FigureSpec};
use mathed_core::markers::{auto_marker_id, lowest_free_marker_numbers, Arg, Segment};

use crate::docui::figures::Figure;
use crate::docui::model::DocModel;

/// Default figure size for an auto-appended `\app` statement.
pub const DEFAULT_FIGURE_W: i32 = 640;
pub const DEFAULT_FIGURE_H: i32 = 400;

/// Build the [`Figure`] for a resolved `\app` segment, including the doc
/// byte ranges of its `w`/`h` literal args (which a resize rewrites).
pub fn figure_from_segment(seg: &Segment) -> Option<Figure> {
    let resolved = resolve_figure(seg)?;
    let (w_arg, h_arg) = arg_ranges(&seg.extra_args)?;
    Some(Figure {
        key: resolved.key,
        stmt: seg.stmt,
        spec: resolved.spec,
        span: resolved.span,
        w_arg,
        h_arg,
        page: None,
        rect: smithay::utils::Rectangle::default(),
        app_id: None,
        title: None,
    })
}

/// The doc byte ranges of the first two literal args (`w`, `h`).
///
/// A marker-ref in either slot means the statement isn't a figure at
/// all — the same condition `app_figure_spec` rejects.
fn arg_ranges(extra_args: &[Arg]) -> Option<(Range<usize>, Range<usize>)> {
    let w = literal_range(extra_args.first()?)?;
    let h = literal_range(extra_args.get(1)?)?;
    Some((w, h))
}

fn literal_range(arg: &Arg) -> Option<Range<usize>> {
    match arg {
        Arg::Literal { range, .. } => Some(range.clone()),
        Arg::MarkerRef { .. } => None,
    }
}

/// Rewrite a figure's declared size in place.
///
/// Returns `false` when the new size equals the old one (nothing to
/// commit — an unnecessary CRDT op would still invalidate the layout).
pub fn set_figure_size(model: &mut DocModel, figure: &Figure, w: i32, h: i32) -> bool {
    let (w, h) = (
        w.clamp(MIN_FIGURE, MAX_FIGURE),
        h.clamp(MIN_FIGURE, MAX_FIGURE),
    );
    if figure.spec.w == w && figure.spec.h == h {
        return false;
    }
    // Both ranges come from the *same* scan, so rewriting them in one
    // pass is safe: `replace_keeping_caret` re-scans after each write,
    // and we captured the ranges up front.
    model.replace_keeping_caret(figure.w_arg.clone(), &w.to_string());
    // The `h` arg's offsets shift if the `w` literal's length changed,
    // which is exactly what the write above may have done — so its
    // range is re-read from the post-write scan rather than reused.
    let Some(h_arg) = current_arg_range(model, &figure.key, 1) else {
        return false;
    };
    model.replace_keeping_caret(h_arg, &h.to_string());
    true
}

/// Smallest figure side we allow, so a drag can never collapse a figure
/// to nothing (which would leave the bound app with no rect to draw in).
pub const MIN_FIGURE: i32 = 64;
/// Largest figure side, bounding pathological drags.
pub const MAX_FIGURE: i32 = 8192;

/// The doc byte range of the n-th literal arg of the figure named `key`
/// in the model's *current* scan.
fn current_arg_range(model: &DocModel, key: &str, index: usize) -> Option<Range<usize>> {
    let stmt_idx: usize = key.strip_prefix('f')?.parse().ok()?;
    let seg = model.segments().iter().find(|s| s.stmt == stmt_idx)?;
    literal_range(seg.extra_args.get(index)?)
}

/// Set (or clear) a figure's binding id in place.
///
/// `None` removes the `"id"` argument entirely rather than writing an
/// empty string, so a cleared figure parses back as id-less.
pub fn set_figure_id(model: &mut DocModel, figure: &Figure, id: Option<&str>) -> bool {
    if figure.spec.id.as_deref() == id {
        return false;
    }
    let Some(seg) = model.segments().iter().find(|s| s.stmt == figure.stmt) else {
        return false;
    };
    let stmt = &model.scan().stmts[figure.stmt];
    match id {
        Some(id) => {
            if let Some(range) = seg.extra_args.get(2).and_then(literal_range) {
                model.replace_keeping_caret(range, &format!("\"{id}\""));
            } else {
                // No id arg yet: insert one before the closing paren.
                let at = stmt.range.end.saturating_sub(1);
                model.replace_keeping_caret(at..at, &format!(", \"{id}\""));
            }
        }
        None => {
            let Some(range) = seg.extra_args.get(2).and_then(literal_range) else {
                return false;
            };
            // Drop the preceding ", " separator too, so the statement
            // doesn't keep a dangling comma.
            let text = model.text();
            let start = range.start.saturating_sub(2);
            let range = if text.get(start..range.start) == Some(", ") {
                start..range.end
            } else {
                range
            };
            model.replace_keeping_caret(range, "");
        }
    }
    true
}

/// Append a figure statement for an app that had no free figure to bind
/// to, and return the appended text.
///
/// The statement is written with freshly allocated marker ids (via
/// `lowest_free_marker_numbers`) so it can never collide with an
/// existing marker, and the caption is the app's title (falling back to
/// its app_id, then to the command name).
/// Escape a client-supplied string for use as the body of a `\app` argument.
///
/// The caption and the binding id both come from `xdg_toplevel.set_title` and
/// `set_app_id`, so they are attacker-controlled: *any* client can set either.
/// Two things go wrong unescaped, and neither is visible at the point of
/// insertion:
///
/// - a `"` closes the argument early, so `\app(#a, #b, 640, 400, "x", 1)`
///   carries the wrong `w`/`h` and the figure silently changes size;
/// - a `#` starts a marker, and marker ids are resolved **first-occurrence-wins**
///   (`DocModel::scan`). A title of `#1 pwned` appended to a document that
///   already has a `#1` re-points every segment after it.
///
/// The result is client-triggered corruption of the user's persisted document,
/// which autosaves every five seconds.
fn escape_arg(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Escape a client-supplied caption for the document body.
///
/// Backslash-escaping is the document's own convention, so a `\` the user meant
/// literally round-trips rather than becoming a Typst escape.
fn escape_caption(s: &str) -> String {
    s.replace('\\', "\\\\").replace('#', "\\#")
}

pub fn append_figure(
    model: &mut DocModel,
    caption: &str,
    w: i32,
    h: i32,
    id: Option<&str>,
) -> String {
    let ids = lowest_free_marker_numbers(model.scan(), 2);
    let (Some(&a), Some(&b)) = (ids.first(), ids.get(1)) else {
        // `lowest_free_marker_numbers(2)` always yields two ids; this arm
        // only exists to keep the destructure total.
        return String::new();
    };
    // `auto_marker_id` returns the bare RFC-1751 word; the marker token
    // is that word behind a `#`.
    let start = format!("#{}", auto_marker_id(a));
    let end = format!("#{}", auto_marker_id(b));
    let id_arg = id.map_or_else(String::new, |id| format!(", \"{}\"", escape_arg(id)));
    let statement = format!(
        "{start} {} {end} \\app({start}, {end}, {w}, {h}{id_arg})\n",
        escape_caption(caption)
    );
    // A statement is a block: never glue it onto the tail of the last
    // line, or it lands inside the previous statement's argument list.
    let at = model.text().len();
    let needs_newline = !model.text().is_empty() && !model.text().ends_with('\n');
    let statement = if needs_newline {
        format!("\n{statement}")
    } else {
        statement
    };
    model.insert(at, &statement);
    statement
}

/// Clone a figure's statement (caption + `\app(...)`) to the end of the
/// document, with fresh marker ids. This is how a mirror is created: two
/// `\app` statements sharing one id render the same app twice.
///
/// Returns the appended text.
pub fn clone_figure(model: &mut DocModel, figure: &Figure) -> String {
    let caption = model.text()[figure.span.clone()].trim().to_string();
    let id = figure.spec.id.clone();
    append_figure(model, &caption, figure.spec.w, figure.spec.h, id.as_deref())
}

/// The default size a new app should get: the size of the first
/// unbound figure if there is one, else [`DEFAULT_FIGURE_W`] ×
/// [`DEFAULT_FIGURE_H`].
pub fn default_figure_size() -> (i32, i32) {
    (DEFAULT_FIGURE_W, DEFAULT_FIGURE_H)
}

/// A figure spec equal to a (w, h) pair with no id.
pub fn spec_of(w: i32, h: i32) -> FigureSpec {
    FigureSpec {
        w,
        h,
        id: None,
        launch: None,
    }
}

#[cfg(test)]
impl DocModel {
    /// Test shorthand for "append a figure to this document".
    fn append(&mut self, caption: &str, w: i32, h: i32, id: Option<&str>) -> String {
        append_figure(self, caption, w, h, id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docui::figures::FigureManager;
    use crate::docui::layout::DocLayoutCache;

    fn model_with_two_figures() -> DocModel {
        DocModel::new("#1 one #2 \\app(#1, #2, 300, 200)\n#3 two #4 \\app(#3, #4, 100, 50)")
    }

    fn figures(model: &DocModel) -> Vec<Figure> {
        let mut mgr = FigureManager::new();
        let mut cache = DocLayoutCache::new();
        mgr.sync(model, &mut cache);
        mgr.figures().to_vec()
    }

    #[test]
    fn figure_args_are_located_in_the_document_text() {
        let model = model_with_two_figures();
        let figs = figures(&model);
        assert_eq!(figs.len(), 2);
        // The recorded w/h ranges must actually slice the declared
        // numbers out of the source.
        assert_eq!(&model.text()[figs[0].w_arg.clone()], "300");
        assert_eq!(&model.text()[figs[0].h_arg.clone()], "200");
        assert_eq!(&model.text()[figs[1].w_arg.clone()], "100");
        assert_eq!(&model.text()[figs[1].h_arg.clone()], "50");
    }

    #[test]
    fn set_figure_size_rewrites_both_literals() {
        let mut model = model_with_two_figures();
        let figs = figures(&model);
        assert!(set_figure_size(&mut model, &figs[0], 640, 480));
        assert!(
            model.text().contains("\\app(#1, #2, 640, 480)"),
            "{}",
            model.text()
        );
        // The second figure is untouched.
        assert!(model.text().contains("\\app(#3, #4, 100, 50)"));
    }

    #[test]
    fn set_figure_size_handles_a_length_change() {
        // "300" → "1000" shifts every later byte; the h arg must still
        // land on the right literal. (A height of 20 clamps up to
        // MIN_FIGURE, so the expectation is the clamp, not 20.)
        let mut model = model_with_two_figures();
        let figs = figures(&model);
        assert!(set_figure_size(&mut model, &figs[0], 1000, 20));
        let after = figures(&model);
        assert_eq!(after[0].spec.size(), (1000, MIN_FIGURE));
        assert_eq!(after[1].spec.size(), (100, 50), "second figure unchanged");
    }

    #[test]
    fn set_figure_size_is_a_no_op_when_unchanged() {
        let mut model = model_with_two_figures();
        let figs = figures(&model);
        assert!(!set_figure_size(&mut model, &figs[0], 300, 200));
    }

    #[test]
    fn set_figure_size_clamps_to_the_minimum() {
        let mut model = model_with_two_figures();
        let figs = figures(&model);
        assert!(set_figure_size(&mut model, &figs[0], 1, 1));
        let after = figures(&model);
        assert_eq!(after[0].spec.size(), (MIN_FIGURE, MIN_FIGURE));
    }

    #[test]
    fn set_figure_size_clamps_both_ways() {
        let mut model = model_with_two_figures();
        let figs = figures(&model);
        assert!(set_figure_size(&mut model, &figs[0], 100_000, 0));
        let after = figures(&model);
        assert_eq!(
            after[0].spec.size(),
            (MAX_FIGURE, MIN_FIGURE),
            "a runaway drag can't produce a zero-sized figure"
        );
    }

    #[test]
    fn set_figure_id_adds_and_removes_the_argument() {
        let mut model = DocModel::new("#1 one #2 \\app(#1, #2, 300, 200)");
        let figs = figures(&model);
        assert!(set_figure_id(&mut model, &figs[0], Some("term")));
        assert!(
            model.text().contains("\\app(#1, #2, 300, 200, \"term\")"),
            "{}",
            model.text()
        );
        let figs = figures(&model);
        assert_eq!(figs[0].spec.id.as_deref(), Some("term"));

        assert!(set_figure_id(&mut model, &figs[0], None));
        assert!(
            model.text().contains("\\app(#1, #2, 300, 200)"),
            "{}",
            model.text()
        );
        let figs = figures(&model);
        assert_eq!(figs[0].spec.id, None);
    }

    #[test]
    fn set_figure_id_replaces_an_existing_id() {
        let mut model = DocModel::new("#1 one #2 \\app(#1, #2, 300, 200, \"old\")");
        let figs = figures(&model);
        assert!(set_figure_id(&mut model, &figs[0], Some("new")));
        assert!(model.text().contains("\"new\""), "{}", model.text());
        assert!(!model.text().contains("\"old\""));
    }

    #[test]
    fn append_figure_adds_a_usable_statement() {
        let mut model = DocModel::new("#1 one #2 \\app(#1, #2, 300, 200)");
        model.append("foot", 500, 300, Some("foot"));
        let figs = figures(&model);
        assert_eq!(figs.len(), 2, "the appended figure parses");
        let new = &figs[1];
        assert_eq!(new.spec.size(), (500, 300));
        assert_eq!(new.spec.id.as_deref(), Some("foot"));
        assert_eq!(&model.text()[new.span.clone()], " foot ");
    }

    #[test]
    fn append_figure_starts_on_its_own_line() {
        // A document with no trailing newline must not get the new
        // statement glued onto the previous one's `)`.
        let mut model = DocModel::new("#1 one #2 \\app(#1, #2, 300, 200)");
        model.append("foot", 500, 300, None);
        assert!(!model.text().contains(")\n\\app"), "{}", model.text());
        assert_eq!(figures(&model).len(), 2);
    }

    #[test]
    fn append_figure_allocates_fresh_marker_ids() {
        // The document already uses #1..#4; the appended statement must
        // not reuse them or its caption span would resolve to the
        // *existing* markers.
        let mut model = model_with_two_figures();
        model.append("third", 10, 10, None);
        let figs = figures(&model);
        assert_eq!(figs.len(), 3);
        assert_eq!(&model.text()[figs[2].span.clone()], " third ");
    }

    #[test]
    fn clone_figure_copies_the_caption_and_id() {
        let mut model = DocModel::new("#1 Terminal #2 \\app(#1, #2, 300, 200, \"term\")");
        let figs = figures(&model);
        clone_figure(&mut model, &figs[0]);
        let figs = figures(&model);
        assert_eq!(figs.len(), 2);
        assert_eq!(figs[1].spec.size(), (300, 200));
        assert_eq!(figs[1].spec.id.as_deref(), Some("term"));
        assert_eq!(&model.text()[figs[1].span.clone()], " Terminal ");
        // Distinct keys → distinct statements.
        assert_ne!(figs[0].key, figs[1].key);
    }

    #[test]
    fn caption_text_with_markup_is_carried_through() {
        let mut model = DocModel::new("hello");
        model.append("a $x^2$ b", 10, 10, None);
        let figs = figures(&model);
        assert_eq!(figs.len(), 1);
        assert_eq!(&model.text()[figs[0].span.clone()], " a $x^2$ b ");
    }
    /// A hostile `title` must not be able to reshape the document.
    ///
    /// `title` and `app_id` come from `xdg_toplevel.set_title` / `set_app_id`, so
    /// any client controls them. Appended verbatim they could close the `\app`
    /// argument list with a `"` — silently changing the figure's `w`/`h` — or
    /// introduce a `#N` marker. Marker ids resolve first-occurrence-wins, so a
    /// second `#1` re-points every segment after it: client-triggered corruption
    /// of the user's persisted document, which autosaves every five seconds.
    #[test]
    fn a_hostile_title_cannot_corrupt_the_document() {
        let mut model = DocModel::new("#1 existing #2 \\app(#1, #2, 640, 400)\n");
        append_figure(&mut model, "#1 pwned \" 1, 2, 3, \"y", 640, 400, None);
        let text = model.text();
        assert!(
            text.contains("\\#1"),
            "the caption hash must be escaped so it cannot become a marker: {text}"
        );

        let scan = model.scan();
        let apps: Vec<_> = scan.stmts.iter().filter(|s| s.name == "app").collect();
        assert_eq!(apps.len(), 2, "both figures must still parse: {text}");

        // The pre-existing statement is untouched: its start marker is still #1
        // and its width is still 640. A leaked marker would have re-pointed one.
        let first = &apps[0];
        let first_args: Vec<String> = first.args.iter().map(|a| format!("{a:?}")).collect();
        assert!(
            first_args[0].contains("\"1\""),
            "the original start marker must still be #1: {text}"
        );
        assert!(
            first_args[2].contains("640"),
            "and the original width must be unchanged: {text}"
        );
    }

    /// A hostile `app_id` must not be able to close the argument list either.
    #[test]
    fn a_hostile_app_id_cannot_close_the_argument_list() {
        let mut model = DocModel::new("");
        let hostile_id = "x\", 1, 1, \"y";
        append_figure(&mut model, "Terminal", 640, 400, Some(hostile_id));
        let text = model.text();
        let scan = model.scan();
        let app = scan
            .stmts
            .iter()
            .find(|s| s.name == "app")
            .expect("an app statement");
        assert_eq!(
            app.args.len(),
            5,
            "two marker refs, w, h and one quoted id, so the injected quote added nothing: {text}"
        );
        assert_eq!(
            app.range.end,
            text.trim_end().len(),
            "nothing leaked past the statement: {text}"
        );
    }

    /// Escaping is a round trip: the caption the user sees is the caption the
    /// client set, not the escaped form.
    #[test]
    fn an_ordinary_title_is_written_verbatim() {
        let mut model = DocModel::new("");
        append_figure(&mut model, "Alacritty", 320, 240, Some("alacritty"));
        let text = model.text();
        assert!(
            text.contains(" Alacritty ") && text.contains(r" \app("),
            "an ordinary caption must not be mangled: {text}"
        );
        assert!(
            text.contains(r#""alacritty""#),
            "nor an ordinary id: {text}"
        );
    }
}
