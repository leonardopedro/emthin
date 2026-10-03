# Build notes

emthin's build is a plain `cargo build`, with one wrinkle worth writing
down: on NixOS (and any minimal distro) `pkg-config` and a few shared
libraries are not on the default search path.

## Why

`emthin-dbus` depends on `gio`/`glib`, whose `-sys` crates locate the C
library with `pkg-config` at **build-script** time. smithay's wayland
frontend links `libxkbcommon` directly at **link** time. Neither is
found by default outside a distribution-provided build environment.

## NixOS

Use the flake:

```sh
nix develop            # everything below, already on PATH
cargo test -p emthin
```

It sets `PKG_CONFIG_PATH` for glib (the `pkg-config` wrapper reads it, and
glib-sys's build script needs it), `LIBRARY_PATH` for `-lxkbcommon` and
libglvnd (read by the `cc` driver `rustc` shells out to for the final
link), `PKG_CONFIG_PATH` for udev (calloop's libudev-sys, whose `.pc`
comes from systemd rather than a `libudev` attribute), and
`LD_LIBRARY_PATH` for libwayland — winit `dlopen`s that one at runtime
rather than linking it, so it has to be findable when `emthin` *runs*,
not only when it links.

What that replaces: an `env.sh` that hardcoded four `/nix/store` paths.
It was not reproducible and broke on any nixpkgs bump. `nixpkgs` is
pinned to `nixos-unstable` to match `../velysterm/flake.nix`, so
`mathed_core`/`mathed_mini` and emthin share one glib/wayland/xkbcommon
rather than two copies.

### Input injection, and why §6 step 7 is still unverified

The shell also carries `xdotool`, `wtype`, `ydotool`, `dotool` and
`fcitx5`, so the manual E2E's last step has the tools it needs. On
*this* machine none of the four injection mechanisms can reach emthin,
each for a different and checked reason:

| Tool | Mechanism | Why it does not work here |
|---|---|---|
| `wtype` | `zwp_virtual_keyboard_v1` | Mutter does not implement the protocol, so there is no keyboard to bind. `wtype` reports `Wayland connection failed` against the nested compositor, with both a bare and an absolute `WAYLAND_DISPLAY`. |
| `ydotool`, `dotool` | `uinput`, kernel-level | Needs write access to `/dev/uinput`, which is `root:root` mode 0600; `ydotoold` fails `failed to open uinput device: Permission denied`, and there is no sudo. Injected events would reach the *host* input stack, not a nested compositor's, anyway. |
| `xdotool` | X11 + XTEST | Needs a window on an X display. Mutter can only run `--headless` here: run non-headless it tries to take the logind session and fails `Failed to take control of the session: EBUSY`, because the real session already owns it. Headless Mutter draws to no X window, so there is nothing for XTEST to target. |

So the E2E drives what it can over the IPC control socket, and §6 step 7
stays open. On a host where `/dev/uinput` is group-writable, `ydotool`
is the one to reach for: it needs no compositor support at all. Where
the compositor implements `zwp_virtual_keyboard_v1`, `wtype` is the
least invasive.

## Toolchain

`rust-toolchain.toml` pins the channel. `rustup` honours the pin per
directory, so emthin and a sibling checkout can pin different
versions without interfering.

## The document engine

emthin depends on `mathed_core` and `mathed_mini` from the sibling
velysterm checkout by relative path:

```toml
mathed_core  = { path = "../../../velysterm/crates/mathed_core" }
mathed_mini  = { path = "../../../velysterm/crates/mathed_mini", default-features = false }
```

`default-features = false` drops mathed_mini's `gui` feature (winit,
softbuffer, accesskit, arboard): emthin owns the window and renders the
page itself, so that frontend is dead weight here.

## Package metadata

`crates/emthin/Cargo.toml` keeps a **literal** `version` and `edition`
rather than `.workspace = true`, because cargo-aur 0.x does not
understand workspace inheritance. Both this file and
`[workspace.package]` in the root `Cargo.toml` must be bumped together
(`cargo release` handles both via `release.toml`).

## Verifying a change

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
cargo test -p emthin
```

The document side is verified separately (it is a different
workspace):

```sh
cd ../velysterm
cargo test -p mathed_core -p mathed_mini -p mathed_biblio
cargo build -p mathed --features gui   # Bevy editor
```