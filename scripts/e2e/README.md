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

Tested on the host socket and emthin's own socket, one boundary apart — which is
the only relationship a nested compositor has. Reaching the *desktop* session as
well would mean crossing a second compositor boundary, and
`clip-bisect.sh` shows where that breaks.

The chain here is longer than it looks:

```
GNOME (wayland-0) → Xwayland (:0)     a different X server from the one below
Xvfb (:99)        → sway (x11 backend)
                  → emthin
```

So `scripts/e2e/clip-bisect.sh` tests each hop on its own, which is what turns
"the clipboard does not work" into "hop N is broken":

```
hop 1 — sway ↔ Xvfb X selection (wlroots' x11 backend)
  FAIL  X selection reaches sway's clients
  FAIL  sway's clients reach the X selection

hop 2 — emthin ↔ sway (emthin's clipboard proxy)
  PASS  sway's selection reaches emthin's clients
  PASS  emthin's clients reach sway
```

**emthin is not the broken hop.** Its proxy works in both directions against the
compositor it is nested in; the chain breaks below that, in wlroots' X11 backend,
which does not sync a Wayland selection to the X selection on a bare Xvfb. An
earlier version of this harness reported the clipboard as "verified broken in
both directions" — that was wrong twice over: it asserted across two compositor
boundaries, and it shared one marker between the legs so a leak read as a pass.

Current state of the two checks in `step7.sh`:

- **host → client passes**, and the log shows why it works:
  `Host Clipboard changed (5 types)`, then `Wayland paste request`, then
  `selection Clipboard: … age=5.3s`.
- **client → host fails**, and is order-dependent: the same operation passes in
  `clip-bisect.sh` when the previous leg has not just run. emthin's log records
  *no* event for it, so the bridge never sees the client's offer at all. Leading
  hypothesis: `wl-copy` from wayland-utils creates no surface, and a client's
  selection offer is per-surface, whereas `wl-paste` is a request and works
  headless — which would explain the asymmetry exactly. Testing it needs a
  surface-owning client (an app inside emthin doing a real copy), which the
  harness does not yet drive. **Left as a failing check rather than skipped**,
  because "asymmetric" is a bug shape worth keeping an eye on.

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