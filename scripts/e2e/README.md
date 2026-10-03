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

## The clipboard checks

The clipboard half needs no keys: `wl-copy` on the host session and `wl-paste`
inside the nested one exercises host→client through emthin's proxy, and the
reverse exercises client→host.

The result depends on the compositor, because the proxy's backend chain does:

- under **headless Mutter**, DataControl and WlDataDevice are both aimed at
  `$WAYLAND_DISPLAY`, which is emthin itself, so the chain falls through to X11
  and talks to the host's Xwayland — while the test drives the host's *Wayland*
  selection. Whether those meet depends on the host's Wayland↔X11 selection sync,
  which does not happen here, so both directions skip with that reason.
- under **sway**, wlroots implements data-control, the chain stops there, and the
  checks actually run. Current result: **client→host passes**, **host→client
  fails**.

  client→host is the direction that matters — "text copied out of the document has
  to be pasteable in an app" is what the bridge's `HostSelectionChanged` echo and
  its `SelectionOrigin::Host` bookkeeping exist for.

  host→client failing means the proxy is not tracking the *host's* selection, even
  though the log shows it receiving `Host Clipboard changed`. That is a real gap
  and is reported as a failure, not skipped.

`step7.sh` detects the chosen backend from the log and reports a skip with the
reason when the chain has landed somewhere that cannot carry the selection at
all.

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

## What passes today

Run on this machine, `--config xdotool`, sway on Xvfb, emthin inside it:

```
 ok  typed text reaches the document
 ok  doc-copy pastes (the marker appears 3 times)
 ok  a client inside the nested session reaches the host clipboard
FAIL host clipboard reaches a client inside the nested session

4 ran, 0 skipped
1 check(s) failed
```

So §6 step 7's keyboard half is **verified**: keys reach the document, and
Ctrl+A / Ctrl+C / Ctrl+V round-trips through `arboard` — which is the
`1a79b38` path, live. The IME commit is still not exercised: it needs the host's
fcitx engaged, which `fcitx5` being installed does not arrange.

## Known unresolved

`run.sh` stops emthin with SIGTERM, and under this harness the signal path does
not fire: the handler is installed (`SigCgt` has TERM), the signal is delivered
(`SigPnd` empty afterwards), but the shutdown pipe receives no bytes and the loop
never stops, so the harness escalates to SIGKILL. The graceful shutdown itself
was verified directly in `a2b393c` — `shutdown signal received`, then
`shut down cleanly` — and the difference between the two launch paths is not
isolated. The harness reports the escalation as a real failure rather than
hiding it.