use crate::EmthinState;

pub fn runtime_dir() -> String {
    std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string())
}

pub fn default_ipc_path() -> std::path::PathBuf {
    let pid = std::process::id();
    std::path::PathBuf::from(format!("{}/emthin-{pid}.ipc", runtime_dir()))
}

pub fn init_logging(log_file: Option<&std::path::Path>) {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    match log_file {
        Some(path) => match std::fs::File::create(path) {
            Ok(file) => tracing_subscriber::fmt()
                .with_ansi(false)
                .with_writer(std::sync::Mutex::new(file))
                .with_env_filter(env_filter)
                .init(),
            Err(e) => eprintln!("failed to open --log-file {}: {e}", path.display()),
        },
        None => tracing_subscriber::fmt().with_env_filter(env_filter).init(),
    }
}

pub fn host_wl_display_ptr(state: &EmthinState) -> Option<*mut std::ffi::c_void> {
    use winit_crate::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
    let backend = state.backend.as_ref()?;
    let handle = backend.window().display_handle().ok()?;
    match handle.as_raw() {
        RawDisplayHandle::Wayland(wl) => Some(wl.display.as_ptr()),
        _ => None,
    }
}

pub fn host_wl_surface_ptr(state: &EmthinState) -> Option<*mut std::ffi::c_void> {
    use winit_crate::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let backend = state.backend.as_ref()?;
    let handle = backend.window().window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Wayland(wl) => Some(wl.surface.as_ptr()),
        _ => None,
    }
}

/// SIGTERM, wait up to 1.5s, then SIGKILL — `Child::kill` sends SIGKILL
/// outright, which gives apps no chance to flush their own session files.
pub fn graceful_kill(child: &mut std::process::Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return; // already gone
    }
    // SAFETY: `child.id()` is a pid this process owns (it has not been
    // reaped yet — `try_wait` above returned `Ok(None)`), so the signal
    // targets exactly that child.
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1500);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
        }
    }
}

/// Spawn a client application onto emthin's own Wayland socket.
///
/// Every child gets the same environment contract the Emacs shell used
/// to get: `WAYLAND_DISPLAY` pointing at emthin's socket, and
/// `DBUS_SESSION_BUS_ADDRESS` injected by the in-process broker so
/// fcitx5-speaking clients find a single input context.
pub fn spawn_child(
    command: &str,
    args: &[String],
    x_display: Option<u32>,
    state: &mut EmthinState,
) {
    let Some(socket_name) = state.socket_name.to_str() else {
        tracing::error!("Wayland socket name is not valid UTF-8, cannot spawn child");
        return;
    };

    let display_log = match x_display {
        Some(d) => format!(":{d}"),
        None => std::env::var("DISPLAY").unwrap_or_else(|_| "<unset>".to_string()),
    };
    tracing::info!(
        "Spawning: {command} {args:?} (WAYLAND_DISPLAY={socket_name} DISPLAY={display_log})"
    );
    let mut cmd = std::process::Command::new(command);
    cmd.args(args)
        .env("WAYLAND_DISPLAY", socket_name)
        .env("XDG_SESSION_TYPE", "wayland")
        .env("XDG_SESSION_DESKTOP", "emthin");
    if let Some(d) = x_display {
        cmd.env("DISPLAY", format!(":{d}"));
    }
    state.dbus.inject_env(&mut cmd);
    match cmd.spawn() {
        Ok(child) => state.host.add_child(child),
        Err(e) => tracing::error!("Failed to spawn '{command}': {e}"),
    }
}
