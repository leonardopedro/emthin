//! The document find bar: query editing over `mathed_core`'s search.
//!
//! The matching itself is shared with the pad — [`mathed_core::search`] finds
//! the ranges, keeps the case rule (all-lowercase query means case-insensitive)
//! and steps through them. What belongs here is the *bar*: whether it is open,
//! what the user has typed, what the status line should say, and which byte
//! range the document should select. All of it is pure logic over a text
//! string, so it is tested without a compositor (the same split as
//! [`crate::docui::citations`]).
//!
//! The bar does not draw itself yet: the caller reads [`FindBar::status`] and
//! [`FindBar::current_range`]. Rendering the query and the match highlights is
//! the remaining half of the desktop parity item.

use std::ops::Range;

use mathed_core::search::SearchState;

/// An open find bar and the search it is steering.
#[derive(Default)]
pub struct FindBar {
    state: SearchState,
    open: bool,
}

impl FindBar {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the bar with the search starting at `origin` (the caret), so that
    /// `Ctrl+F` then typing finds the next match *after* the cursor rather than
    /// from the top of the document.
    pub fn open_at(&mut self, origin: usize) {
        self.state.start(origin);
        self.open = true;
    }

    /// Close the bar. Returns whether it was open, so a caller can give
    /// `Escape` to the bar first and only then to whatever else Escape does.
    pub fn close(&mut self) -> bool {
        let was = self.open;
        self.open = false;
        self.state.start(0);
        was
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    #[must_use]
    pub fn query(&self) -> &str {
        &self.state.query
    }

    #[must_use]
    pub fn match_count(&self) -> usize {
        self.state.matches.len()
    }

    /// The match the bar is on, as a byte range the document can select.
    #[must_use]
    pub fn current_range(&self) -> Option<Range<usize>> {
        self.state
            .current
            .and_then(|i| self.state.matches.get(i))
            .cloned()
    }

    /// What a status line should show: `n/m` while there is a match, and the
    /// reason when there is not (an empty query is not "no match").
    #[must_use]
    pub fn status(&self) -> Option<String> {
        if !self.open {
            return None;
        }
        if self.state.query.is_empty() {
            return Some(String::new());
        }
        let total = self.state.matches.len();
        if total == 0 {
            return Some("no match".to_owned());
        }
        let at = self.state.current.map_or(1, |i| i + 1);
        Some(format!("{at}/{total}"))
    }

    /// Append one typed character to the query and re-search `text`.
    pub fn push(&mut self, text: &str, ch: char) {
        if !self.open {
            return;
        }
        let mut query = self.state.query.clone();
        query.push(ch);
        self.state.update_query(text, &query);
    }

    /// Delete the last character of the query and re-search `text`.
    pub fn backspace(&mut self, text: &str) {
        if !self.open {
            return;
        }
        let mut query = self.state.query.clone();
        query.pop();
        self.state.update_query(text, &query);
    }

    /// Replace the whole query (a paste, or a test).
    pub fn set_query(&mut self, text: &str, query: &str) {
        if !self.open {
            return;
        }
        self.state.update_query(text, query);
    }

    /// Step to the next/previous match and return the range to select.
    ///
    /// The search is refreshed against `text` first: the document can change
    /// between two steps (that is the point of a find bar in an editor), and
    /// stepping through ranges computed against a stale text would select the
    /// wrong bytes.
    pub fn step(&mut self, text: &str, forward: bool) -> Option<Range<usize>> {
        if !self.open || self.state.query.is_empty() {
            return None;
        }
        let query = self.state.query.clone();
        self.state.update_query(text, &query);
        if forward {
            self.state.next();
        } else {
            self.state.prev();
        }
        self.current_range()
    }

    /// Re-run the current query after an edit elsewhere in the document.
    pub fn on_doc_changed(&mut self, text: &str) {
        if self.open && !self.state.query.is_empty() {
            self.state.on_doc_changed(text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "alpha beta\nGamma alpha\nbeta alpha";

    #[test]
    fn a_closed_bar_reports_nothing() {
        let mut bar = FindBar::new();
        assert!(!bar.is_open());
        assert_eq!(bar.status(), None);
        assert_eq!(bar.step(DOC, true), None);
    }

    #[test]
    fn opening_starts_the_search_after_the_caret() {
        let mut bar = FindBar::new();
        // Caret at the first `alpha` end: the first match after it is the
        // second `alpha`, not the one behind the cursor.
        bar.open_at(5);
        bar.set_query(DOC, "alpha");
        assert_eq!(bar.match_count(), 3);
        assert_eq!(bar.current_range(), Some(17..22));
        assert_eq!(bar.status().as_deref(), Some("2/3"));
    }

    #[test]
    fn typing_moves_the_match_and_backspace_undoes_it() {
        let mut bar = FindBar::new();
        bar.open_at(0);
        for ch in "beta".chars() {
            bar.push(DOC, ch);
        }
        assert_eq!(bar.query(), "beta");
        assert_eq!(bar.match_count(), 2);
        assert_eq!(bar.current_range(), Some(6..10));
        bar.backspace(DOC);
        assert_eq!(bar.query(), "bet");
        // `bet` is still a prefix of both `beta`s, so it still matches twice.
        assert_eq!(bar.match_count(), 2);
        bar.backspace(DOC);
        assert_eq!(bar.query(), "be");
        assert_eq!(bar.match_count(), 2);
        bar.push(DOC, 'x');
        assert_eq!(bar.status().as_deref(), Some("no match"));
    }

    #[test]
    fn stepping_wraps_and_reports_the_selected_range() {
        let mut bar = FindBar::new();
        bar.open_at(0);
        bar.set_query(DOC, "alpha");
        assert_eq!(bar.step(DOC, true), Some(17..22));
        assert_eq!(bar.step(DOC, true), Some(28..33));
        // Past the last match wraps to the first.
        assert_eq!(bar.step(DOC, true), Some(0..5));
        assert_eq!(bar.step(DOC, false), Some(28..33));
    }

    #[test]
    fn an_all_lowercase_query_is_case_insensitive() {
        let mut bar = FindBar::new();
        bar.open_at(0);
        bar.set_query(DOC, "gamma");
        assert_eq!(bar.match_count(), 1);
        // An uppercase letter makes the query case-sensitive, as in mathed.
        bar.set_query(DOC, "Gamma");
        assert_eq!(bar.match_count(), 1);
        bar.set_query(DOC, "ALPHA");
        assert_eq!(bar.match_count(), 0);
    }

    #[test]
    fn closing_resets_the_query() {
        let mut bar = FindBar::new();
        bar.open_at(0);
        bar.set_query(DOC, "alpha");
        assert!(bar.close());
        assert!(!bar.close());
        assert_eq!(bar.query(), "");
        bar.open_at(0);
        assert_eq!(bar.status().as_deref(), Some(""));
    }

    #[test]
    fn an_edit_between_steps_refreshes_the_ranges() {
        let mut bar = FindBar::new();
        bar.open_at(0);
        bar.set_query(DOC, "alpha");
        // A new match appears before every existing one.
        let longer = format!("alpha {DOC}");
        bar.on_doc_changed(&longer);
        assert_eq!(bar.match_count(), 4);
        bar.set_query(&longer, "alpha");
        // The second match of `alpha alpha beta…` is the newly inserted one.
        assert_eq!(bar.step(&longer, true).map(|r| r.start), Some(6));
    }
}
