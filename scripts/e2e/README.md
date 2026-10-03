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
  checks actually run. Current result: **both directions fail**.

      FAIL host clipboard reaches a client inside the nested session
           got:  (empty)          want: e2e-…-h2c
      FAIL a client inside the nested session reaches the host clipboard
           got:  e2e-…-h2c       want: e2e-…-c2h

  The second failure is the informative one: the *host* clipboard still held the
  marker the first leg had put there, and never received the nested session's.
  So nothing crosses in either direction.

  This was briefly reported as `client→host passes`, and that was a false pass in
  the harness: both legs shared one marker, so the second leg read back the first
  leg's selection and looked like a success. The markers are now distinct per
  direction, and each leg waits for the previous selection owner to exit first.

  The cause is structural rather than a timing artefact. The proxy's DataControl
  backend "connects to a fresh `$WAYLAND_DISPLAY`" — which, in a nested session,
  is the *nested* compositor. So it manages the nested compositor's clipboard, and
  there is no connection from there to the host session's clipboard. emthin logs
  `Host Clipboard changed` because that is the nested clipboard changing; the name
  is doing a lot of work there.

  What this means for emthin: **in a nested session the clipboard proxy has no
  path to the host clipboard.** A data-control backend that named the *host*
  display would be needed, which means the compositor needs to know what its host
  display is — not something `--session-file` or the IPC protocol carries today.

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
FAIL host clipboard reaches a client inside the nested session
FAIL a client inside the nested session reaches the host clipboard

4 ran, 0 skipped
2 check(s) failed
```

So §6 step 7's keyboard half is **verified**: keys reach the document, and
Ctrl+A / Ctrl+C / Ctrl+V round-trips through `arboard` — which is the
`1a79b38` path, live.

The clipboard half is **verified as broken**, in both directions, and the cause
is structural rather than a flake (see above). The IME commit is still not
exercised: it needs the host's fcitx engaged, which `fcitx5` being installed does
not arrange.

## Known unresolved

None outstanding on the shutdown path. `run.sh` stops emthin with SIGTERM and now
gets `emthin exited cleanly (graceful shutdown confirmed in the log)`.

Getting there took an actual diagnosis, because the symptom pointed somewhere
other than the cause. See `crates/emthin/src/shutdown.rs`: the handler ran, the
self-pipe write *succeeded*, and the calloop source on the read end was registered
without complaint — and never dispatched. Replacing the pipe with an atomic flag
the loop checks once per iteration fixed it, and removed a mechanism rather than
adding one.

The harness reports that class of thing honestly either way: it distinguishes
"exited cleanly" from "exited without logging a clean shutdown" from "had to be
SIGKILLed", so a regression here cannot pass unnoticed.