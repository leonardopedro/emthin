use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use emthin_dbus::router::{BridgeCommand, BridgeNotification};
use emthin_dbus::FcitxEvent;

#[derive(Default)]
pub struct DbusBridge {
    cmd_tx: Option<mpsc::Sender<BridgeCommand>>,
    notify_rx: Option<mpsc::Receiver<BridgeNotification>>,
    /// Non-fcitx notifications taken off `notify_rx` by the IME drain but not yet
    /// claimed by the router forwarder.
    ///
    /// There is one channel and two consumers, and an mpsc receiver can only be
    /// drained destructively — so whichever drains first takes *everything*. The
    /// IME drain runs first every tick, and its loop discarded anything that was
    /// not an `FcitxEvent`, so `RuleAdded` / `RuleRemoved` / `RuleList` were
    /// consumed and dropped before `take_non_fcitx_notifications` could ever see
    /// them. That made the three `dbus_router_*` IPC notifications dead on every
    /// path: a documented, advertised control-plane feature that silently never
    /// fired.
    deferred_notifications: Vec<BridgeNotification>,
    listen_path: Option<PathBuf>,
    session_dir: Option<PathBuf>,
    isolated_daemon: Option<Child>,
}

impl std::fmt::Debug for DbusBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbusBridge")
            .field("active", &self.cmd_tx.is_some())
            .field("isolated_daemon", &self.isolated_daemon.is_some())
            .finish()
    }
}

impl DbusBridge {
    pub fn init() -> Self {
        let Ok(upstream_addr) = std::env::var("DBUS_SESSION_BUS_ADDRESS") else {
            tracing::info!("DBUS_SESSION_BUS_ADDRESS not set; dbus bridge inert");
            return Self::default();
        };
        let upstream_path = match parse_bus_address(&upstream_addr) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, addr = %upstream_addr, "unsupported DBUS_SESSION_BUS_ADDRESS; bridge inert");
                return Self::default();
            }
        };

        let Some(session_dir) = create_session_dir() else {
            return Self::default();
        };

        let listen_path = session_dir.join("bus.sock");
        let (cmd_tx, notify_rx) =
            emthin_dbus::router::bridge::spawn(listen_path.clone(), upstream_path);

        tracing::info!(?listen_path, ?session_dir, "dbus bridge started");

        Self {
            cmd_tx: Some(cmd_tx),
            notify_rx: Some(notify_rx),
            deferred_notifications: Vec::new(),
            listen_path: Some(listen_path),
            session_dir: Some(session_dir),
            isolated_daemon: None,
        }
    }

    pub fn init_isolated() -> Self {
        let Some(session_dir) = create_session_dir() else {
            return Self::default();
        };

        let daemon_socket = session_dir.join("upstream-bus.sock");
        let services_dir = session_dir.join("services");
        let config_path = session_dir.join("session.conf");
        if let Err(e) = write_minimal_session_config(&config_path, &services_dir, &daemon_socket) {
            tracing::warn!(error = %e, "failed to write session.conf; bridge inert");
            let _ = std::fs::remove_dir_all(&session_dir);
            return Self::default();
        }

        let mut daemon = match spawn_isolated_daemon(&config_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "failed to spawn dbus-daemon; bridge inert");
                let _ = std::fs::remove_dir_all(&session_dir);
                return Self::default();
            }
        };

        if let Err(e) = wait_for_daemon_socket(&daemon_socket, &mut daemon) {
            tracing::warn!(error = %e, "dbus-daemon never came up; bridge inert");
            let _ = daemon.kill();
            let _ = daemon.wait();
            let _ = std::fs::remove_dir_all(&session_dir);
            return Self::default();
        }

        let listen_path = session_dir.join("bus.sock");
        let (cmd_tx, notify_rx) =
            emthin_dbus::router::bridge::spawn(listen_path.clone(), daemon_socket);

        Self {
            cmd_tx: Some(cmd_tx),
            notify_rx: Some(notify_rx),
            deferred_notifications: Vec::new(),
            listen_path: Some(listen_path),
            session_dir: Some(session_dir),
            isolated_daemon: Some(daemon),
        }
    }

    pub fn inject_env(&self, cmd: &mut Command) {
        if let Some(path) = &self.listen_path {
            cmd.env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}", path.display()),
            );
        }
    }

    pub fn take_fcitx_events(&mut self) -> Vec<FcitxEvent> {
        let Some(ref mut rx) = self.notify_rx else {
            return vec![];
        };
        let mut events = Vec::new();
        let mut deferred = std::mem::take(&mut self.deferred_notifications);
        loop {
            match rx.try_recv() {
                Ok(BridgeNotification::FcitxEvent(e)) => events.push(e),
                // Not ours — hand it to the other consumer rather than dropping
                // it on the floor. See `deferred_notifications`.
                Ok(other) => deferred.push(other),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.cmd_tx = None;
                    self.notify_rx = None;
                    break;
                }
            }
        }
        // Put back anything that arrived but was not claimed, so the other
        // consumer still sees it.
        self.deferred_notifications = deferred;
        events
    }

    pub fn send_rpc(&mut self, cmd: BridgeCommand) {
        if let Some(ref tx) = self.cmd_tx {
            let _ = tx.send(cmd);
        }
    }

    pub fn take_non_fcitx_notifications(&mut self) -> Vec<BridgeNotification> {
        // Whatever the IME drain handed over, then whatever is still on the
        // channel. Order within a tick is not meaningful; order across ticks is
        // preserved, which is what a rule-change stream needs.
        let mut notifs = std::mem::take(&mut self.deferred_notifications);
        let Some(ref mut rx) = self.notify_rx else {
            return notifs;
        };
        loop {
            match rx.try_recv() {
                Ok(BridgeNotification::FcitxEvent(_)) => continue,
                Ok(n) => notifs.push(n),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.cmd_tx = None;
                    self.notify_rx = None;
                    break;
                }
            }
        }
        notifs
    }

    pub fn shutdown(&mut self) {
        if let Some(cmd) = self.cmd_tx.take() {
            let _ = cmd.send(BridgeCommand::Shutdown);
        }
        self.notify_rx = None;
        if let Some(mut daemon) = self.isolated_daemon.take() {
            let _ = daemon.kill();
            let _ = daemon.wait();
        }
        if let Some(dir) = self.session_dir.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

fn parse_bus_address(addr: &str) -> std::io::Result<PathBuf> {
    const PREFIX: &str = "unix:path=";
    let stripped = addr.strip_prefix(PREFIX).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("unsupported bus address: {addr}"),
        )
    })?;
    let path = stripped.split(',').next().unwrap_or(stripped);
    Ok(PathBuf::from(path))
}

fn create_session_dir() -> Option<PathBuf> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let dir = runtime_dir.join(format!("emthin-dbus-{}", std::process::id()));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(error = %e, ?dir, "failed to create dbus session dir");
        return None;
    }
    Some(dir)
}

fn spawn_isolated_daemon(config_path: &std::path::Path) -> std::io::Result<Child> {
    let mut cmd = Command::new("dbus-daemon");
    cmd.arg("--nofork")
        .arg(format!("--config-file={}", config_path.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        cmd.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    cmd.spawn()
}

fn wait_for_daemon_socket(socket: &std::path::Path, daemon: &mut Child) -> std::io::Result<()> {
    const TIMEOUT: Duration = Duration::from_secs(3);
    const POLL_INTERVAL: Duration = Duration::from_millis(25);
    let start = Instant::now();
    loop {
        if socket.exists() {
            return Ok(());
        }
        if let Some(status) = daemon.try_wait()? {
            return Err(std::io::Error::other(format!(
                "dbus-daemon exited before binding socket: {status}"
            )));
        }
        if start.elapsed() > TIMEOUT {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "dbus-daemon listen socket did not appear within 3s",
            ));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn write_minimal_session_config(
    path: &std::path::Path,
    services_dir: &std::path::Path,
    socket: &std::path::Path,
) -> std::io::Result<()> {
    std::fs::create_dir_all(services_dir)?;
    let xml = format!(
        r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={socket}</listen>
  <auth>EXTERNAL</auth>
  <servicedir>{services}</servicedir>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
  <limit name="reply_timeout">300000</limit>
</busconfig>
"#,
        socket = socket.display(),
        services = services_dir.display(),
    );
    std::fs::write(path, xml)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_env_is_noop_when_inert() {
        let bridge = DbusBridge::default();
        let mut cmd = Command::new("true");
        bridge.inject_env(&mut cmd);
        let dbg = format!("{cmd:?}");
        assert!(
            !dbg.contains("DBUS_SESSION_BUS_ADDRESS"),
            "inert bridge must not set env; got: {dbg}"
        );
    }

    #[test]
    fn inject_env_sets_unix_path_when_live() {
        let bridge = DbusBridge {
            listen_path: Some(PathBuf::from("/run/user/1000/emthin-dbus-42/bus.sock")),
            ..DbusBridge::default()
        };
        let mut cmd = Command::new("true");
        bridge.inject_env(&mut cmd);
        let dbg = format!("{cmd:?}");
        assert!(dbg.contains("unix:path=/run/user/1000/emthin-dbus-42/bus.sock"));
    }

    #[test]
    fn parse_bus_address_works() {
        assert_eq!(
            parse_bus_address("unix:path=/run/user/1000/bus").unwrap(),
            PathBuf::from("/run/user/1000/bus")
        );
    }

    #[cfg(test)]
    mod drain_tests {
        use super::*;

        /// A bridge wired to a plain channel, so notifications can be injected
        /// without a live broker.
        fn with_channel() -> (DbusBridge, mpsc::Sender<BridgeNotification>) {
            let (tx, rx) = mpsc::channel();
            let bridge = DbusBridge {
                cmd_tx: None,
                notify_rx: Some(rx),
                deferred_notifications: Vec::new(),
                listen_path: None,
                session_dir: None,
                isolated_daemon: None,
            };
            (bridge, tx)
        }

        fn rule_added(id: &str) -> BridgeNotification {
            BridgeNotification::RuleAdded {
                id: id.to_string(),
                rule: emthin_dbus::router::RouteRule {
                    id: id.to_string(),
                    priority: 1,
                    destination: None,
                    interface: None,
                    method: None,
                    target: "host".into(),
                },
            }
        }

        /// The two consumers must not starve each other.
        ///
        /// `tick` drains fcitx events first and forwards router notifications second,
        /// every tick, in that order. Both drained the same `mpsc::Receiver` to
        /// `Empty` and discarded what they did not want, so the first one consumed the
        /// other's notifications and the three `dbus_router_*` IPC messages were dead
        /// on every path.
        #[test]
        fn a_router_notification_survives_the_ime_drain() {
            let (mut bridge, tx) = with_channel();
            tx.send(rule_added("r1")).expect("send");

            // Exactly what `tick` does, in order.
            let fcitx = bridge.take_fcitx_events();
            let router = bridge.take_non_fcitx_notifications();

            assert!(fcitx.is_empty(), "a router rule is not an fcitx event");
            assert_eq!(
                router.len(),
                1,
                "the rule must reach the router forwarder, not be eaten by the IME drain"
            );
        }

        /// And the other way round: an fcitx event is not mistaken for a router
        /// notification.
        #[test]
        fn an_fcitx_event_reaches_the_ime_drain() {
            let (mut bridge, tx) = with_channel();
            tx.send(rule_added("r1")).expect("send");
            tx.send(BridgeNotification::RuleRemoved { id: "r1".into() })
                .expect("send");

            let _ = bridge.take_fcitx_events();
            let router = bridge.take_non_fcitx_notifications();
            assert_eq!(router.len(), 2, "both rule notifications survive, in order");
        }

        /// A second drain with nothing new yields nothing, rather than replaying
        /// what the first one already handed over.
        #[test]
        fn draining_twice_does_not_replay() {
            let (mut bridge, tx) = with_channel();
            tx.send(rule_added("r1")).expect("send");
            let _ = bridge.take_fcitx_events();
            assert_eq!(bridge.take_non_fcitx_notifications().len(), 1);
            assert!(
                bridge.take_non_fcitx_notifications().is_empty(),
                "a notification must be delivered once, not on every tick"
            );
        }

        /// Notifications that arrive *after* the IME drain in the same tick are
        /// picked up normally — the handoff buffer is a queue, not a replacement.
        #[test]
        fn notifications_arriving_late_are_still_delivered() {
            let (mut bridge, tx) = with_channel();
            let _ = bridge.take_fcitx_events();
            tx.send(rule_added("late")).expect("send");
            assert_eq!(bridge.take_non_fcitx_notifications().len(), 1);
        }

        /// Order across a tick boundary is preserved, which is what a rule-change
        /// stream needs: an add followed by a remove must not arrive reversed.
        #[test]
        fn cross_tick_order_is_preserved() {
            let (mut bridge, tx) = with_channel();
            tx.send(rule_added("a")).expect("send");
            let _ = bridge.take_fcitx_events();
            let first = bridge.take_non_fcitx_notifications();
            assert_eq!(first.len(), 1);

            tx.send(rule_added("b")).expect("send");
            let _ = bridge.take_fcitx_events();
            tx.send(BridgeNotification::RuleRemoved { id: "a".into() })
                .expect("send");
            let second = bridge.take_non_fcitx_notifications();
            assert_eq!(second.len(), 2, "the later pair arrives together");
        }
    }
}
