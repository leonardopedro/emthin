//! State owned by the emthin **host window** itself, plus the child
//! processes emthin spawns.
//!
//! This replaces the old `EmacsState`: the shell is no longer a client
//! emthin embeds, so there is no "main surface" to track and no
//! "first toplevel is the shell" heuristic to latch. What survives is
//! the genuinely host-scoped bookkeeping — the winit window's title,
//! the fullscreen/maximize requests a *client* can make on the host
//! (`xdg_toplevel.set_fullscreen` on the compositor's own surface), and
//! the spawned app processes emthin reaps on exit.

use std::process::Child;

/// See module docs.
#[derive(Debug, Default)]
pub struct HostState {
    /// Host window title, forwarded from a client's title/app_id.
    pub title: Option<String>,
    /// Host window app_id, forwarded from a client's app_id.
    pub app_id: Option<String>,
    /// A client's `xdg_toplevel.set_fullscreen` on the host surface.
    pub pending_fullscreen: Option<bool>,
    /// A client's `xdg_toplevel.set_maximized` on the host surface.
    pub pending_maximize: Option<bool>,
    /// Processes spawned via `--spawn` / the `spawn` IPC op.
    children: Vec<Child>,
}

impl HostState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take the pending host-title update, if any.
    pub fn take_title(&mut self) -> Option<String> {
        self.title.take()
    }

    /// Set the host window title (`None` clears it).
    pub fn set_title(&mut self, title: Option<&str>) {
        self.title = title.map(str::to_owned);
    }

    /// Set the host window app_id.
    pub fn set_app_id(&mut self, app_id: Option<&str>) {
        self.app_id = app_id.map(str::to_owned);
    }

    pub fn request_fullscreen(&mut self, fullscreen: bool) {
        self.pending_fullscreen = Some(fullscreen);
    }

    pub fn take_pending_fullscreen(&mut self) -> Option<bool> {
        self.pending_fullscreen.take()
    }

    pub fn request_maximize(&mut self, maximized: bool) {
        self.pending_maximize = Some(maximized);
    }

    pub fn take_pending_maximize(&mut self) -> Option<bool> {
        self.pending_maximize.take()
    }

    /// Track a spawned child process.
    pub fn add_child(&mut self, child: Child) {
        self.children.push(child);
    }

    /// Reap every child that has exited. Returns `true` if any did.
    ///
    /// The last app closing does **not** quit the compositor: the
    /// document (and its dormant figures) outlives its apps. The event
    /// loop keeps running so the user can keep editing, spawn more
    /// apps, or quit explicitly.
    pub fn reap_children(&mut self) -> bool {
        let mut any = false;
        self.children.retain_mut(|child| match child.try_wait() {
            Ok(Some(_)) => {
                any = true;
                false
            }
            Ok(None) => true,
            Err(_) => {
                any = true;
                false
            }
        });
        any
    }

    /// Kill and reap every spawned child (graceful exit path).
    pub fn kill_children(&mut self) {
        for child in self.children.drain(..) {
            crate::util::graceful_kill(&mut { child });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_is_taken_once() {
        let mut host = HostState::new();
        assert!(host.take_title().is_none());
        host.set_title(Some("foot"));
        assert_eq!(host.take_title().as_deref(), Some("foot"));
        assert!(host.take_title().is_none(), "take is destructive");
        host.set_title(None);
        assert!(host.take_title().is_none());
    }

    #[test]
    fn pending_states_are_taken_once() {
        let mut host = HostState::new();
        host.request_fullscreen(true);
        host.request_maximize(true);
        assert_eq!(host.take_pending_fullscreen(), Some(true));
        assert_eq!(host.take_pending_fullscreen(), None);
        assert_eq!(host.take_pending_maximize(), Some(true));
        assert_eq!(host.take_pending_maximize(), None);
    }

    #[test]
    fn reaping_reports_exited_children() {
        let mut host = HostState::new();
        // `/bin/sh` rather than `/bin/true`: NixOS has no /bin/true.
        host.add_child(
            std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg("exit 0")
                .spawn()
                .expect("spawn"),
        );
        // The child has to actually exit before `try_wait` reports it.
        let mut reported = false;
        for _ in 0..200 {
            if host.reap_children() {
                reported = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(reported, "an exited child is reported");
        assert!(!host.reap_children(), "reaping is idempotent");
    }

    #[test]
    fn live_children_are_kept() {
        let mut host = HostState::new();
        host.add_child(
            std::process::Command::new("sleep")
                .arg("60")
                .spawn()
                .expect("run"),
        );
        assert!(!host.reap_children(), "a live child is not reported");
        host.kill_children();
    }
}
