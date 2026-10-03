use clap::Parser;
use smithay::reexports::wayland_server::Display;

use emthin::{activation, cli::Cli, ipc, state, util, EmthinState};
use emthin_clipboard::{BackendHint, ClipboardBackend};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    util::init_logging(cli.log_file.as_deref());

    // --wayland-socket is plumbed through an env var so state.rs's
    // `init_wayland_listener` can stay signature-stable; CLI flag takes
    // precedence over a pre-set env var by overwriting it here.
    if let Some(ref name) = cli.wayland_socket {
        std::env::set_var("EMTHIN_WAYLAND_SOCKET_NAME", name);
    }

    let mut event_loop: smithay::reexports::calloop::EventLoop<'static, EmthinState> =
        smithay::reexports::calloop::EventLoop::try_new()?;

    let display: Display<EmthinState> = Display::new()?;

    let ipc_path = cli.ipc_path.clone().unwrap_or_else(util::default_ipc_path);
    tracing::info!("IPC socket path: {}", ipc_path.display());

    // xkbcommon treats "" as invalid (not "use default"), so when variant is
    // set but layout is empty we must supply a base layout explicitly.
    let xkb_layout = if cli.xkb_layout.is_empty() && !cli.xkb_variant.is_empty() {
        "us".to_string()
    } else {
        cli.xkb_layout.clone()
    };
    let xkb_config = smithay::input::keyboard::XkbConfig {
        layout: &xkb_layout,
        model: &cli.xkb_model,
        variant: &cli.xkb_variant,
        options: cli.xkb_options.clone(),
        ..Default::default()
    };

    let ipc = emthin::ipc::IpcServer::bind(ipc_path)?;
    let loop_handle = event_loop.handle();
    let mut state = EmthinState::new(&mut event_loop, loop_handle, display, ipc, xkb_config)?;

    register_ipc_source(&mut event_loop, &state)?;

    // Open a Wayland/X11 window for our nested compositor. Must happen
    // before clipboard init — the wl_data_device fallback piggybacks on
    // winit's host Wayland connection to get focused-client selection
    // events without needing our own host surface.
    emthin::winit::init_winit(&mut event_loop, &mut state, cli.fullscreen)?;

    // Claim keyboard focus on the host via xdg_activation_v1 if we
    // inherited an XDG_ACTIVATION_TOKEN / DESKTOP_STARTUP_ID — real
    // GNOME/KWin startup-notification path, and the only way to get
    // focus on hosts that don't auto-focus new toplevels (Mutter).
    // No-op if env is empty or host lacks xdg_activation_v1.
    activation::activate_main_surface_if_env_token(&state);

    // Initialize clipboard synchronization with host compositor.
    // Fallback chain: Wayland data-control (no-focus, preferred) →
    // wl_data_device via winit's shared connection (focus-gated) →
    // X11 selection (if host is Xorg).
    //
    // Test hook: `EMTHIN_DISABLE_HOST_CLIPBOARD=1` disables host clipboard
    // sync entirely. Kept as a safety valve for debugging; the E2E
    // harness doesn't need it anymore because each test gets its own
    // private host compositor (see `tests/common/mod.rs::NestedHost`).
    if std::env::var_os("EMTHIN_DISABLE_HOST_CLIPBOARD").is_none() {
        // Fallback chain: data-control (owns its own host connection, focus-free)
        // → wl_data_device on winit's shared connection (focus-gated) → X11
        // selection (only meaningful when the host is Xorg). See
        // emthin_clipboard::BackendHint for per-variant semantics.
        let mut hints: Vec<BackendHint> = vec![BackendHint::DataControl];
        if let Some(ptr) = util::host_wl_display_ptr(&state) {
            // SAFETY: the wl_display is owned by winit's backend, which
            // lives in `state.backend` for the entire compositor run. The
            // returned clipboard backend sits in `state.selection.clipboard`
            // on the same struct, so default field-drop order guarantees
            // the backend drops before the wl_display.
            hints.push(unsafe { BackendHint::wl_data_device(ptr) });
        }
        hints.push(BackendHint::X11);

        state.selection.clipboard = emthin_clipboard::init(&hints);
        if let Some(ref clipboard) = state.selection.clipboard {
            register_clipboard_source(&mut event_loop, clipboard.as_ref())?;
        }
    } else {
        tracing::info!("EMTHIN_DISABLE_HOST_CLIPBOARD set; host clipboard sync disabled");
    }

    // Bind the in-process DBus broker before any child processes; its
    // listen socket must exist by the time `inject_env` stamps
    // `DBUS_SESSION_BUS_ADDRESS` on `spawn_child`. A
    // missing or unparseable upstream bus downgrades the bridge to an
    // inert state — embedded IME popups then land wherever they always
    // did (no regression vs. pre-broker behavior).
    state.dbus = if cli.dbus_isolated {
        state::dbus::DbusBridge::init_isolated()
    } else {
        state::dbus::DbusBridge::init()
    };

    if !cli.no_spawn && !cli.spawn.is_empty() {
        // Park the `--spawn` command lines; they're launched once the
        // XWayland display is resolved (or immediately, without one).
        state.xwayland.set_pending_commands(cli.spawn_commands());
    }

    start_xwayland_satellite(
        event_loop.handle(),
        &mut state,
        cli.xwayland_display.unwrap_or(0),
        &cli.xwayland_satellite_bin,
    );

    // Spawn the parked `--spawn` apps regardless of whether XWayland came
    // up. Three buckets:
    //
    //   - satellite up → child sees `DISPLAY=:N` (emthin's nested X)
    //   - satellite missing, host has Xwayland → child inherits the
    //     host `DISPLAY` and X11-only programs fall back to the host X
    //     server. Windows render outside emthin, but at least the child
    //     has a GUI instead of dropping to TUI.
    //   - satellite missing AND host has no DISPLAY → child runs
    //     headless / TUI; nothing we can do without an X server.
    if let Some(pending) = state.xwayland.take_pending_commands() {
        let display = state.xwayland.display();
        if display.is_none() {
            if let Ok(host) = std::env::var("DISPLAY") {
                tracing::warn!(
                    "xwayland-satellite unavailable; children will inherit host \
                     DISPLAY={host}. X11 windows will render on the host X \
                     server (outside emthin)."
                );
            } else {
                tracing::warn!(
                    "xwayland-satellite unavailable and host has no DISPLAY; \
                     X11-only children will fall back to TUI / headless."
                );
            }
        }
        for (command, args) in pending {
            util::spawn_child(&command, &args, display, &mut state);
        }
    }

    // Load the document: `--doc` wins, then the session's snapshot, then
    // a fresh empty one. Done after the XWayland handshake so the doc's
    // first layout already knows the display.
    state
        .doc
        .load(cli.doc.as_deref(), cli.session_file.as_deref());

    // SIGTERM/SIGINT must stop the loop, or `run` never returns.
    //
    // The graceful path below — snapshot the document and the session, reap the
    // children, shut the DBus bridge down — only runs when `event_loop.run`
    // *returns*, and nothing was wired to make that happen. With no handler,
    // SIGTERM's default action killed the process outright and Ctrl+C did the
    // same, so the shutdown code was unreachable in practice: quitting lost
    // everything since the last autosave tick, up to five seconds of typing plus
    // the current page, which that same tick is what writes.
    //
    // Found by running the compositor: `goto_page 1` followed by SIGTERM left
    // `session.json` still saying page 0.
    install_shutdown_signals(event_loop.handle(), state.loop_signal.clone());

    event_loop.run(None, &mut state, emthin::tick::event_loop_tick)?;

    // Graceful shutdown: snapshot the session, then reap children.
    state.doc.save();
    state.host.kill_children();
    state.dbus.shutdown();
    tracing::info!("shut down cleanly");

    Ok(())
}

/// Stop the loop on SIGTERM or SIGINT, via a self-pipe.
///
/// A signal handler may only touch async-signal-safe things and `LoopSignal::stop`
/// is not one of them; `write(2)` is. So the handler writes one byte and a
/// calloop source on the read end stops the loop on the next iteration.
///
/// The first attempt used a `sigwait` thread instead, and the signal never
/// arrived there: `sigwait` requires the signals blocked process-wide, and with a
/// handler installed *as well* nothing consumed them — SIGTERM was ignored
/// outright. `/proc/<pid>/status` showed both `SigBlk` and `SigCgt` set for it,
/// which is the signature of that arrangement. The self-pipe has no such
/// ambiguity: no signal is ever blocked, and the disposition is unambiguous.
fn install_shutdown_signals(
    handle: smithay::reexports::calloop::LoopHandle<'static, EmthinState>,
    signal: smithay::reexports::calloop::LoopSignal,
) {
    use smithay::reexports::calloop::{generic::Generic, Interest, Mode, PostAction};

    let mut fds = [0 as libc::c_int; 2];
    // O_CLOEXEC so the pipe does not leak into every spawned app.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        tracing::warn!(
            "could not create the shutdown pipe ({:?}); Ctrl+C will not save the session",
            std::io::Error::last_os_error()
        );
        return;
    }
    let (read_fd, write_fd) = (fds[0], fds[1]);

    extern "C" fn on_signal(_sig: libc::c_int) {
        // The only async-signal-safe work here. `write` on a pipe with room for
        // 64 KiB and a one-byte payload cannot block.
        let byte = 1u8;
        unsafe {
            libc::write(
                SHUTDOWN_PIPE_WRITE.load(std::sync::atomic::Ordering::Relaxed),
                &byte as *const u8 as *const libc::c_void,
                1,
            );
        }
    }
    SHUTDOWN_PIPE_WRITE.store(write_fd, std::sync::atomic::Ordering::SeqCst);

    // Safety: a handler that writes one byte and returns.
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_signal as usize;
        libc::sigemptyset(&mut sa.sa_mask);
        sa.sa_flags = 0;
        libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
    }

    // Safety: `read_fd` is a fresh owned descriptor we never close elsewhere.
    let owned = unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(read_fd) };
    let generic = Generic::new(owned, Interest::READ, Mode::Level);
    handle
        .insert_source(generic, move |_, _, _| {
            let mut buf = [0u8; 32];
            while unsafe { libc::read(read_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) }
                > 0
            {}
            tracing::info!("shutdown signal received, stopping the loop");
            signal.stop();
            signal.wakeup();
            Ok(PostAction::Remove)
        })
        .map(|_| ())
        .unwrap_or_else(|e| tracing::warn!("could not register the shutdown source: {e:?}"));
}

/// Write end of the shutdown pipe, for the signal handler. An atomic rather than
/// a plain `static mut`, because the handler reads it while it is being set.
static SHUTDOWN_PIPE_WRITE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

fn register_ipc_source(
    event_loop: &mut smithay::reexports::calloop::EventLoop<EmthinState>,
    state: &EmthinState,
) -> Result<(), Box<dyn std::error::Error>> {
    use smithay::reexports::calloop::{generic::Generic, Interest, Mode, PostAction};
    use std::os::unix::io::FromRawFd;
    let listener_fd = state.ipc.listener_fd();
    // SAFETY: We duplicate the fd so the Generic source owns its own copy.
    // The original fd remains valid inside IpcServer for the lifetime of state.
    let dup_fd = unsafe { libc::dup(listener_fd) };
    if dup_fd < 0 {
        return Err("dup(ipc listener fd) failed".into());
    }
    // SAFETY: dup_fd is a valid open fd (dup succeeded above, dup_fd >= 0).
    // Ownership transfers to File; the original listener_fd stays open in IpcServer.
    let file = unsafe { std::fs::File::from_raw_fd(dup_fd) };
    event_loop
        .handle()
        .insert_source(
            Generic::new(file, Interest::READ, Mode::Level),
            |_, _, state| {
                state.ipc.accept();
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| format!("failed to register IPC listener: {e}"))?;
    Ok(())
}

/// niri-style xwayland-satellite integration.
///
/// emthin pre-binds the X11 display sockets and only spawns the external
/// `xwayland-satellite` process when an X11 client first connects.
/// satellite crashes are handled transparently: the spawner thread
/// observes the exit, sends `ToMain::Rearm` through a calloop channel,
/// and the main loop re-installs the socket watch.
fn start_xwayland_satellite(
    handle: smithay::reexports::calloop::LoopHandle<'static, EmthinState>,
    state: &mut EmthinState,
    display_start: u32,
    binary: &std::path::Path,
) {
    use emthin::xwayland_satellite::{
        setup_connection, test_ondemand, SpawnConfig, ToMain, XwlsIntegration,
    };
    use smithay::reexports::calloop::channel;

    // Niri pattern: probe the binary first. A missing / incompatible
    // satellite disables the XWayland integration rather than crashing
    // the compositor.
    if !test_ondemand(binary) {
        tracing::warn!(
            "xwayland-satellite at {} not available or lacks --test-listenfd-support; \
             XWayland disabled",
            binary.display()
        );
        return;
    }

    let sockets = match setup_connection(display_start) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("xwayland-satellite: failed to bind X11 sockets: {e}");
            return;
        }
    };
    let display = sockets.display;
    let display_name = sockets.display_name.clone();

    let Some(socket_name) = state.socket_name.to_str() else {
        tracing::error!(
            "xwayland-satellite: wayland socket name is not valid UTF-8; aborting setup"
        );
        return;
    };
    let spawn_cfg = SpawnConfig {
        binary: binary.to_path_buf(),
        wayland_socket: std::path::PathBuf::from(socket_name),
        xdg_runtime_dir: std::path::PathBuf::from(util::runtime_dir()),
    };

    let (tx, rx) = channel::channel::<ToMain>();
    state
        .xwayland
        .set_integration(XwlsIntegration::new(sockets, spawn_cfg, tx));

    // Rearm handler: when the spawner thread reports child exit, drain
    // pending connections and re-install the socket watch.
    let rearm_handle = handle.clone();
    if let Err(e) = handle.insert_source(rx, move |event, _, st| {
        if let channel::Event::Msg(ToMain::Rearm) = event {
            if let Some(x) = st.xwayland.integration_mut() {
                if let Err(e) = x.on_rearm(&rearm_handle) {
                    tracing::warn!("xwayland-satellite rearm failed: {e}");
                }
            }
        }
    }) {
        tracing::error!("xwayland-satellite: failed to install rearm channel: {e}");
        state.xwayland.clear_integration();
        return;
    }

    if let Err(e) = state
        .xwayland
        .integration_mut()
        .expect("set_integration above guarantees Some")
        .arm(&handle)
    {
        tracing::error!("xwayland-satellite: arm() failed: {e}");
        state.xwayland.clear_integration();
        return;
    }

    // Socket is ready — export DISPLAY and announce it over IPC. The first X
    // client connect triggers the on-demand satellite spawn automatically.
    std::env::set_var("DISPLAY", &display_name);
    state.xwayland.set_display(display);
    state
        .ipc
        .send(ipc::OutgoingMessage::XWaylandReady { display });
    tracing::info!("xwayland-satellite: socket ready on {display_name}");

    // Pending child is spawned by `main` after this fn returns, so the
    // satellite-missing path (early return above) can still launch it
    // without DISPLAY.
}

fn register_clipboard_source(
    event_loop: &mut smithay::reexports::calloop::EventLoop<EmthinState>,
    clipboard: &dyn ClipboardBackend,
) -> Result<(), Box<dyn std::error::Error>> {
    use emthin_clipboard::Driver;
    use smithay::reexports::calloop::{generic::Generic, Interest, Mode, PostAction};
    use std::os::unix::io::{AsRawFd, FromRawFd};

    // Piggyback backends (wl_data_device on a foreign wl_display) are drained
    // every tick from tick.rs — no owned fd to register here.
    let raw_fd = match clipboard.driver() {
        Driver::OwnedFd(fd) => fd.as_raw_fd(),
        Driver::Piggyback => return Ok(()),
    };

    // SAFETY: dup() returns a valid fd that we transfer ownership to File.
    let dup_fd = unsafe { libc::dup(raw_fd) };
    if dup_fd < 0 {
        return Err("dup(clipboard connection fd) failed".into());
    }
    let file = unsafe { std::fs::File::from_raw_fd(dup_fd) };

    event_loop
        .handle()
        .insert_source(
            Generic::new(file, Interest::READ, Mode::Level),
            |_, _, state| {
                if let Some(ref mut clipboard) = state.selection.clipboard {
                    clipboard.dispatch();
                }
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| format!("failed to register clipboard source: {e}"))?;
    Ok(())
}
