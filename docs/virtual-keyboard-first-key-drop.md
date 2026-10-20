# Virtual keyboards lose their first key — wire-level analysis and upstream drafts

Measured on 2026-10-19 against the e2e's headless sway (wlroots) with
`wtype` (zwp_virtual_keyboard_v1), tracing every layer: the
compositor→client wire (`WAYLAND_DEBUG=1`, with global event serials),
winit's dispatch (instrumented build), and emthin's input seam. The first
**press** of every virtual-keyboard burst is lost; its release arrives
alone.

## What actually happens (the wire)

Per `wtype` invocation (each one installs its own keymap on its own
virtual keyboard and exits):

```
wl_keyboard.keymap(1, fd …)              ← the virtual keyboard's map
wl_keyboard.repeat_info(25, 600)
wl_keyboard.modifiers(…)
wl_seat.capabilities(2)                  ← ★ keyboard bit FLAPS off
… wl_keyboard.enter(…) …                 ← seat re-attaches its keyboard
wl_keyboard.modifiers(…)
wl_keyboard.key(<serial>, 0, <code>, 0)  ← only the RELEASE — no press
```

Two facts make the conclusion airtight:

1. **No press exists on the wire.** Wayland event serials are global and
   consecutive; the release's serial has no predecessor in the gap, so the
   compositor never emitted a press event. Nothing a client does can
   recover a key the compositor dropped.
2. **The trigger is the seat's keyboard-capability flap.** A
   `zwp_virtual_keyboard_v1` client installs a new keyboard on the seat;
   sway emits `wl_seat.capabilities` with the Keyboard bit cleared and
   re-added, and wlroots' `wlr_seat_keyboard_send_key` has no focused
   client during the re-attach — `wtype`'s first key lands exactly there
   and is dropped server-side. The release, ~2 ms later, arrives after
   the re-attach and is delivered.

Multi-key bursts lose exactly the first press (the rest arrive intact);
single-key invocations (`wtype -k Return`) deliver only an orphan release
and therefore do nothing. A physical keyboard never swaps the seat's
keyboard, so real typing is unaffected; the victims are one-shot virtual
keyboards — `wtype`-style injection and the first tap of an on-screen
keyboard right after it creates its virtual keyboard.

## Why this is not (only) a winit bug

An earlier hypothesis blamed winit's `key_input` dropping keys in the
`wl_keyboard.leave`→`enter` gap (`window_id == None`) — plausible, since
`KeyboardData`'s own doc admits the state occurs "on bugged compositors,
like mutter". A patched winit was built (queue gap keys, replay on enter)
and A/B tested: **the observable behavior did not change** — the presses
never reach `key_input` at all (instrumented: every dispatched event
shows `window=Some(..)`). The vendor was removed rather than ship 2.7 MB
of patched winit with no observable effect.

winit's gap-drop is still a real (unexercised here) robustness hole and
the queue-and-replay patch below mirrors what winit already does for
modifiers (`modifiers_pending`, deferred to Enter). Keep it in mind if
upstreaming to winit.

## Upstream issue draft 1 — wlroots / sway (the measured cause)

> **Title:** virtual-keyboard key events are dropped during the seat
> keyboard swap (first key of every `wtype` burst lost)
>
> **Body:**
> When a `zwp_virtual_keyboard_v1` client installs its keymap (sway:
> `wlr_seat_set_keyboard` + re-enter of the focused surface), the seat's
> keyboard capability flaps (`wl_seat.capabilities` with Keyboard cleared
> and re-added) and the swap's first key is dropped: with
> `WAYLAND_DEBUG=1` on the focused client, the press never appears on the
> wire (consecutive serials), while the release ~2 ms later does.
>
> Repro (sway, focused client, `wtype`):
> `wtype -- x` — the client sees only the key release.
> `wtype -- abc` — `a`'s press is lost, `bc` complete.
> `wtype -k Return` — no effect (only an orphan release is delivered).
>
> `wlr_seat_keyboard_send_key` no-ops while `focused_client` is unset;
> during the swap there is a window where that is true even though a
> surface has (and keeps) focus. Suggest buffering keys across the
> keyboard swap — or keeping the seat's keyboard focus stable across
> `wlr_seat_set_keyboard` — instead of dropping them.

## Upstream issue draft 2 — winit (client-side hardening)

> **Title:** Wayland: keys arriving in the Leave→Enter gap are dropped
> silently (`key_input` with `window_id == None`)
>
> **Body:**
> `key_input` (`platform_impl/linux/wayland/seat/keyboard/mod.rs`)
> returns early when `KeyboardData::window_id` is `None`, discarding the
> key. That field is cleared on `wl_keyboard.leave` and set on
> `wl_keyboard.enter`; compositors re-enter the focused surface around an
> active-keyboard swap and can deliver keys in the gap (the state is
> already documented in `KeyboardData` as occurring "on bugged
> compositors, like mutter"). A key dropped there is lost forever — its
> release then arrives alone.
>
> Since a client's wl_keyboard receives no keys while genuinely
> unfocused, keys seen in the gap are swap fallout and belong to the next
> enter. Suggest queueing them (non-repeat, bounded, short TTL) and
> replaying on `enter`, after `Focused(true)` — the same deferral winit
> already applies to modifiers via `modifiers_pending`.

## What emthin does meanwhile

The e2e's `wtype` lane leads every injection with `wtype -s 150`
(`INJECT_SWAP_SETTLE_MS`), letting the seat swap settle before the first
key; with it, all keyboard checks pass. `INJECT_SWAP_SETTLE_MS=0` is the
regression probe for the upstream fixes: when sway (and possibly winit)
stop dropping keys, the lane goes green at 0 too.
