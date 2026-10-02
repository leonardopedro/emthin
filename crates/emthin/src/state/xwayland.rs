//! XWayland integration state — mirrors cosmic-comp's `XWaylandState`
//! in shape (single-struct home for everything XWayland-related) even
//! though it's much thinner because emthin delegates the actual X
//! server to `xwayland-satellite` rather than running `Xwayland` under
//! smithay's `X11Wm`.
//!
//! Three orthogonal pieces live here.
//!
//! **Display number** (`display`): cached `:N` for convenience —
//! exported as `DISPLAY` to every child emthin spawns and sent over
//! IPC as `XWaylandReady`. Set once and forgotten.
//!
//! **Supervisor** (`integration`): pre-binds `/tmp/.X11-unix/X<N>` +
//! the abstract socket, arms calloop watches, and lazily spawns
//! `xwayland-satellite` on first X client connect. See the
//! `crate::xwayland_satellite` module for the state machine.
//!
//! **Pending child commands** (`pending_commands`): the `--spawn`
//! command lines, parked until XWayland reports Ready so GTK3 /
//! Electron children spawn with a valid `DISPLAY` in env. Drained
//! exactly once by `main` after the satellite setup settles.

use crate::xwayland_satellite::XwlsIntegration;

#[derive(Default)]
pub struct XwaylandState {
    display: Option<u32>,
    integration: Option<XwlsIntegration>,
    pending_commands: Option<Vec<(String, Vec<String>)>>,
}

impl XwaylandState {
    // -- Display number --------------------------------------------

    /// Cache the display number once XWayland reports Ready.
    pub fn set_display(&mut self, display: u32) {
        self.display = Some(display);
    }

    /// Read the cached display number, if XWayland came up. Returned
    /// to `main`'s spawn path so the child gets `DISPLAY=:N` only
    /// when satellite actually exists; without it, the child
    /// inherits the parent's `DISPLAY` and X11 tools fall back to
    /// the host X server.
    pub fn display(&self) -> Option<u32> {
        self.display
    }

    // -- Supervisor ------------------------------------------------

    /// Mutable access to the supervisor for arming calloop watches
    /// and driving the spawn state machine.
    pub fn integration_mut(&mut self) -> Option<&mut XwlsIntegration> {
        self.integration.as_mut()
    }

    /// Install the supervisor. Called from `main` after sockets bind.
    pub fn set_integration(&mut self, integration: XwlsIntegration) {
        self.integration = Some(integration);
    }

    /// Drop the supervisor on a fatal init error so the rest of the
    /// compositor keeps running without XWayland.
    pub fn clear_integration(&mut self) {
        self.integration = None;
    }

    // -- Pending child commands ------------------------------------

    /// Park the `--spawn` command lines until XWayland reports Ready.
    pub fn set_pending_commands(&mut self, cmds: Vec<(String, Vec<String>)>) {
        self.pending_commands = Some(cmds);
    }

    /// Drain the parked commands (exactly once).
    pub fn take_pending_commands(&mut self) -> Option<Vec<(String, Vec<String>)>> {
        self.pending_commands.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `XwlsIntegration` owns pre-bound X11 sockets + a calloop
    // `channel::Sender` and has no Default — constructing one in a
    // unit test is out of scope. Its slot (set / integration_mut /
    // clear) is covered by the e2e suite. Everything else (display
    // cache, pending-command mailbox) is pure logic.

    fn sample_cmds() -> Vec<(String, Vec<String>)> {
        vec![
            ("foot".into(), vec!["-T".into(), "xterm-256color".into()]),
            ("firefox".into(), vec![]),
        ]
    }

    #[test]
    fn default_is_empty_on_all_three_slots() {
        let s = XwaylandState::default();
        assert!(s.display.is_none());
        assert!(s.pending_commands.is_none());
        assert!(s.integration.is_none());
    }

    #[test]
    fn set_display_caches_the_number() {
        let mut s = XwaylandState::default();
        s.set_display(42);
        assert_eq!(s.display, Some(42));
    }

    #[test]
    fn set_display_overwrites_previous_value() {
        // Guards against a future where XWayland restarts mid-session —
        // the latest display wins.
        let mut s = XwaylandState::default();
        s.set_display(1);
        s.set_display(2);
        assert_eq!(s.display, Some(2));
    }

    #[test]
    fn pending_commands_set_then_take() {
        let mut s = XwaylandState::default();
        assert!(s.take_pending_commands().is_none());

        s.set_pending_commands(sample_cmds());
        assert!(s.pending_commands.is_some(), "set parks the value");

        let taken = s
            .take_pending_commands()
            .expect("take returns what was set");
        assert_eq!(taken.len(), 2);
        assert_eq!(taken[0].0, "foot");
        assert_eq!(
            taken[0].1,
            vec!["-T".to_string(), "xterm-256color".to_string()]
        );
        assert_eq!(taken[1].0, "firefox");

        assert!(s.pending_commands.is_none(), "take drains");
        assert!(s.take_pending_commands().is_none());
    }

    #[test]
    fn set_pending_commands_overwrites_previous() {
        // Re-arming before the first drain wins with the latest set.
        let mut s = XwaylandState::default();
        s.set_pending_commands(vec![("old".into(), vec![])]);
        s.set_pending_commands(vec![("new".into(), vec!["-flag".into()])]);
        let taken = s.take_pending_commands().unwrap();
        assert_eq!(taken[0].0, "new");
        assert_eq!(taken[0].1, vec!["-flag".to_string()]);
    }

    #[test]
    fn display_and_pending_commands_are_independent() {
        let mut s = XwaylandState::default();
        s.set_display(7);
        s.set_pending_commands(sample_cmds());

        assert_eq!(s.display, Some(7));
        assert!(s.pending_commands.is_some());

        let _ = s.take_pending_commands();
        assert_eq!(
            s.display,
            Some(7),
            "draining the mailbox must not touch the display"
        );
    }
}
