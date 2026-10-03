# §6 step 7 — E2E harness

Step 7 is the last manual check in `docs/REWRITE_PLAN.md` §6: IME commit into an
app figure and into the document caret, and clipboard host↔client and doc-copy.

```sh
./scripts/e2e/run.sh --list        # what works on this machine, and why not if not
./scripts/e2e/run.sh --config none # the clipboard half only; always available
```

No wrapper needed — the scripts re-enter the flake themselves. (`nix develop`
starts an *interactive* shell, so a pasted block beginning with it hands the rest
of the block to the nested shell as input.)

## The four configurations

Every backend implements the same four functions — `inject_available`,
`inject_note`, `inject_type`, `inject_key` — so `step7.sh` is written once and
runs unchanged under any of them.

| `--config` | Mechanism | Privilege | Needs |
|---|---|---|---|
| `ydotool` | `uinput`, kernel level | root, for the daemon only | emthin must **own the seat** — see below |
| `xdotool` | XTEST | none | a nested compositor with an X window, so a TTY |
| `wtype` | `zwp_virtual_keyboard_v1` | none | a compositor that implements it (not Mutter) |
| `none` | — | none | nothing; keyboard skipped, clipboard still runs |

The clipboard half needs no keys, so it runs in **every** configuration.

## What each one actually requires, measured

**`ydotool` cannot reach a nested compositor.** This is the counter-intuitive
one, and it is worth stating plainly because it is the intuitive answer.

The daemon starts, creates a socket the unprivileged side finds, and `ydotool
key`/`type` deliver to it — verified, with the injected characters appearing in a
terminal:

```
nested gnome-shell (--headless):  0 fds on /dev/input/event*
host gnome-shell:               14 fds on /dev/input/event*
```

A nested compositor is not a display server. It never opens input devices, so a
uinput device has nothing to attach to: the **host** compositor owns evdev, notices
the new device, and routes the keys to *its* focused client. The nesting boundary
sits below the input stack, not above it.

Use it when emthin owns the seat — not nested, or on a spare VT — where emthin's
compositor *is* the display server and a uinput device is just another keyboard
to it.

```sh
./scripts/e2e/step7-ydotool.sh    # sudo for the daemon, unprivileged test
```

**`xdotool` is the configuration that can work nested.** Run the compositor
non-headless so it maps a real X window; XTEST into that window enters the nested
seat through the compositor's own window rather than through the input devices.
It needs the logind session to be unowned, which is why Mutter fails with
`Failed to take control of the session: EBUSY` from inside a desktop session.
Log out, or switch to a VT:

```sh
# from a TTY, with the graphical session logged out
./scripts/e2e/run.sh --config xdotool
```

**`wtype`** works headless and needs no privileges, but Mutter does not implement
`zwp_virtual_keyboard_v1` — it answers `does not support the virtual keyboard
protocol`. Use it under sway, weston or wayfire.

## The clipboard checks, and why they skip here

The clipboard half needs no keys: `wl-copy` on the host session and `wl-paste`
inside the nested one exercises host→client through emthin's proxy, and the
reverse exercises client→host.

They skip on this machine, with the reason, because emthin's proxy walks
DataControl → WlDataDevice → X11 and in a nested session the first two are aimed
at `$WAYLAND_DISPLAY` — which is emthin itself:

```
Host supports neither ext_data_control_v1 nor zwlr_data_control_v1
wl_data_device roundtrip failed
X11 clipboard sync initialized
```

So the bridge settles on X11, talking to the host's Xwayland, while the test
drives the host's *Wayland* selection. Whether those meet depends on the host's
Wayland↔X11 selection sync. `step7.sh` detects the chosen backend and reports a
skip with that reason rather than a bare failure.

## Exit codes

| code | meaning |
|---|---|
| 0 | every executed check passed |
| 1 | at least one check failed |
| 2 | **nothing was verified** — every check was skipped |

2 exists because a run in which all the checks declined once printed "all checks
passed". Being able to tell *passed* from *did not run* is most of the value of a
harness.

## Layout

| file | role |
|---|---|
| `run.sh` | entry point: picks a configuration, brings up compositor + emthin |
| `lib.sh` | shared setup; compositor/emthin lifecycle; the injection seam; assertions |
| `step7.sh` | the checks, written against the seam |
| `inject-*.sh` | one per configuration |
| `ydotoold-daemon.sh` | the privileged half of the uinput route, and socket diagnostics |
| `step7-ydotool.sh` | the whole uinput route as one command |
| `ipc.py`, `state.py` | the control protocol, and a readable dump of `state` |

## Known unresolved

`run.sh` stops emthin with SIGTERM, and under this harness the signal path does
not fire: the handler is installed (`SigCgt` has TERM), the signal is delivered
(`SigPnd` empty afterwards), but the shutdown pipe receives no bytes and the loop
never stops, so the harness escalates to SIGKILL. The graceful shutdown itself
was verified directly in `a2b393c` — `shutdown signal received`, then
`shut down cleanly` — and the difference between the two launch paths is not
isolated. The harness reports the escalation as a real failure rather than
hiding it.