//! Interactive app launcher / command palette (parked item).
//!
//! A discovery surface over the document's own `launch:` arguments. It holds
//! **no command registry**: every entry is a figure whose `\app` statement
//! already carries a `launch:` command, and accepting an entry delegates to
//! the existing `relaunch_dormant` path (the only legal figure spawn path).
//! The document stays the authority; this module only filters and labels.

use crate::docui::figures::FigureManager;
use mathed_core::figures::FigureSpec;

/// One launchable row. `key` is the figure key `relaunch_dormant` expects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherEntry {
    pub key: String,
    pub label: String,
    pub page: usize,
}

/// The human name of a dormant figure, factored out of `DocUi::dormant_label`
/// so the label logic has one owner and the palette cannot drift from the
/// figure overlay.
pub fn dormant_name(spec: &FigureSpec, key: &str) -> String {
    spec.launch
        .as_deref()
        .and_then(|cmd| crate::cli::split_command(cmd).first().cloned())
        .or_else(|| spec.id.clone())
        .unwrap_or_else(|| key.to_owned())
}

/// Every launchable figure, in document order. A figure is launchable when it
/// is dormant, is a Wayland figure (not a terminal/inert reference), carries a
/// `launch:` command, and has landed on a page. Figures without `launch:` stay
/// invisible here: click/Return still relaunches them in place, but a *palette*
/// exists to discover commands the document named, not to guess programs.
pub fn entries(figures: &FigureManager) -> Vec<LauncherEntry> {
    figures
        .figures()
        .iter()
        .filter(|f| f.is_dormant() && f.spec.is_wayland() && f.spec.launch.is_some())
        .filter_map(|f| {
            f.page.map(|page| LauncherEntry {
                key: f.key.clone(),
                label: dormant_name(&f.spec, &f.key),
                page,
            })
        })
        .collect()
}

/// An open palette: the query, the entries it was built from, and the
/// highlighted row. Pure text logic, like [`super::search::FindBar`].
#[derive(Default)]
pub struct LauncherBar {
    open: bool,
    query: String,
    selection: usize,
    entries: Vec<LauncherEntry>,
}

impl LauncherBar {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Open with a fresh snapshot of the launchable figures.
    pub fn open(&mut self, figures: &FigureManager) {
        self.open = true;
        self.query.clear();
        self.selection = 0;
        self.entries = entries(figures);
    }

    pub fn close(&mut self) -> bool {
        let was = self.open;
        self.open = false;
        was
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The entries matching the current query (case-insensitive substring).
    #[must_use]
    pub fn matches(&self) -> Vec<&LauncherEntry> {
        let q = self.query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| q.is_empty() || e.label.to_lowercase().contains(&q))
            .collect()
    }

    /// Status line for the overlay: `n/m`, or a reason when no entry matches.
    #[must_use]
    pub fn status(&self) -> Option<String> {
        if !self.open {
            return None;
        }
        let matched = self.matches().len();
        if matched == 0 {
            return Some(if self.entries.is_empty() {
                "no launch: figures".to_owned()
            } else {
                "no match".to_owned()
            });
        }
        let at = self.selection.min(matched - 1) + 1;
        Some(format!("{at}/{matched}"))
    }

    pub fn push(&mut self, ch: char) {
        if self.open {
            self.query.push(ch);
            self.selection = 0;
        }
    }

    pub fn backspace(&mut self) {
        if self.open {
            self.query.pop();
            self.selection = 0;
        }
    }

    pub fn step(&mut self, forward: bool) {
        let n = self.matches().len();
        if n == 0 {
            return;
        }
        if forward {
            self.selection = (self.selection + 1) % n;
        } else {
            self.selection = (self.selection + n - 1) % n;
        }
    }

    /// The entry Return would launch, if any.
    #[must_use]
    pub fn accept(&self) -> Option<&LauncherEntry> {
        if !self.open {
            return None;
        }
        let matched = self.matches();
        matched
            .get(self.selection.min(matched.len().saturating_sub(1)))
            .copied()
    }

    /// Refresh after the document/figure set changed, keeping the query.
    pub fn refresh(&mut self, figures: &FigureManager) {
        if self.open {
            self.entries = entries(figures);
            self.selection = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docui::figures::Figure;

    fn mk_spec(w: i32, h: i32) -> FigureSpec {
        FigureSpec {
            w,
            h,
            id: None,
            launch: None,
            dx: None,
            dy: None,
            terminal: None,
            binding_error: None,
        }
    }

    fn figure(key: &str, launch: Option<&str>, page: Option<usize>, bound: bool) -> Figure {
        let mut s = mk_spec(640, 480);
        if let Some(cmd) = launch {
            s.launch = Some(cmd.to_owned());
        }
        Figure {
            key: key.to_owned(),
            stable_id: key.to_owned(),
            stmt: 0,
            spec: s,
            span: 0..0,
            w_arg: 0..0,
            h_arg: 0..0,
            page,
            rect: smithay::utils::Rectangle::from_size((640, 480).into()),
            app_id: bound.then_some(1),
            title: None,
        }
    }

    fn manager(figures: Vec<Figure>) -> FigureManager {
        let mut m = FigureManager::new();
        // FigureManager has no public push; build via the same sync path the
        // rest of the tests use is heavier than needed. Insert directly in
        // test scope through the private field's module visibility.
        m.figures = figures;
        m
    }

    #[test]
    fn entries_are_only_dormant_launchable_landed_figures() {
        let m = manager(vec![
            figure("f0", Some("foot"), Some(0), false),
            figure("f1", Some("foot"), Some(0), true), // bound
            figure("f2", None, Some(0), false),        // no launch:
            figure("f3", Some("foot"), None, false),   // unplaced
        ]);
        let e = entries(&m);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].key, "f0");
        assert_eq!(e[0].label, "foot");
        assert_eq!(e[0].page, 0);
    }

    #[test]
    fn label_prefers_the_launch_program_then_the_binding_id() {
        let spec = FigureSpec {
            launch: Some("foot --title x".to_owned()),
            id: Some("term*".to_owned()),
            ..mk_spec(10, 10)
        };
        assert_eq!(dormant_name(&spec, "f9"), "foot");
        let spec = FigureSpec {
            id: Some("term*".to_owned()),
            ..mk_spec(10, 10)
        };
        assert_eq!(dormant_name(&spec, "f9"), "term*");
        assert_eq!(dormant_name(&mk_spec(10, 10), "f9"), "f9");
    }

    #[test]
    fn the_bar_filters_steps_and_accepts() {
        let m = manager(vec![
            figure("f0", Some("foot"), Some(0), false),
            figure("f1", Some("firefox"), Some(1), false),
        ]);
        let mut bar = LauncherBar::new();
        bar.open(&m);
        assert_eq!(bar.matches().len(), 2);
        assert_eq!(bar.accept().map(|e| e.key.as_str()), Some("f0"));
        bar.step(true);
        assert_eq!(bar.accept().map(|e| e.key.as_str()), Some("f1"));
        bar.push('o');
        bar.push('o');
        assert_eq!(bar.matches().len(), 1);
        assert_eq!(bar.accept().map(|e| e.key.as_str()), Some("f0"));
        bar.push('x');
        assert_eq!(bar.matches().len(), 0);
        assert_eq!(bar.status().as_deref(), Some("no match"));
        bar.backspace();
        bar.backspace();
        assert_eq!(bar.accept().map(|e| e.key.as_str()), Some("f0"));
        assert!(bar.close());
        assert!(bar.accept().is_none());
    }
}
