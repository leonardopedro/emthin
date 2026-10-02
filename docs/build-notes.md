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

```sh
# Pick the store paths your system actually has; these are examples.
export PATH="/nix/store/<pkg-config-wrapper>/bin:$PATH"
export PKG_CONFIG_PATH="/nix/store/<glib-dev>/lib/pkgconfig"
export LIBRARY_PATH="/nix/store/<libxkbcommon>/lib:/nix/store/<libglvnd>/lib:$LIBRARY_PATH"
```

`PKG_CONFIG_PATH` is read by the `pkg-config` wrapper for the build
scripts; `LIBRARY_PATH` is read by the `cc` driver that `rustc` shells
out to for the final link.

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