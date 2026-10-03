{
  description = "emthin — render Wayland applications as figures inside a mathed document";

  # nixos-unstable, matching ../velysterm/flake.nix so `mathed_core`/`mathed_mini`
  # (which emthin path-depends on) and emthin itself resolve the same glib,
  # wayland and xkbcommon store paths rather than two copies.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems =
        fn:
        nixpkgs.lib.genAttrs systems (
          system: fn nixpkgs.legacyPackages.${system}
        );
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          # The NixOS-specific gaps this box has, which `cargo build` and
          # `cargo test` run into without help:
          #
          #  1. pkg-config and glib-2.0's .pc files are on no default path, and
          #     emthin-dbus -> gio -> glib-sys needs them in glib-sys's build
          #     script.
          #  2. -lxkbcommon (smithay's Wayland frontend) has no default search
          #     path, so linking the test binaries fails. LIBRARY_PATH is read
          #     by the gcc driver, which is what rustc shells out to.
          #  3. calloop's libudev-sys has the same problem via a different
          #     package: udev's .pc comes from systemd, not from a `libudev`
          #     attribute.
          #
          # This shell replaces an ad-hoc `env.sh` that hardcoded four /nix/store
          # paths, which is not reproducible and breaks on any nixpkgs bump.
          packages =
            with pkgs;
            [
              # ── toolchain ──────────────────────────────────────────────
              rustc
              cargo
              clippy
              rustfmt
              pkg-config

              # ── what the -sys crates and the linker need ───────────────
              glib
              glib.dev
              libxkbcommon
              libglvnd
              wayland
              libdrm
              libinput
              systemd # libudev's .pc, for calloop's libudev-sys

              # ── document-model path deps' build needs ─────────────────
              # mathed_mini embeds fonts and rasterises with tiny-skia; no
              # system font discovery, so nothing to provide here, but its
              # `gui` feature is off and winit/softbuffer stay out of the graph.

              # ── E2E: input injection, for docs/REWRITE_PLAN.md §6 ─────
              # Step 7 (IME commit into an app figure and into the doc caret,
              # clipboard host<->client and doc-copy) is the one manual step
              # still unverified, precisely because there was no way to type at
              # it. Four different mechanisms, because no single one covers
              # everything here:
              #
              #  xdotool   X11 + XTEST. Works against the nested compositor when
              #            it is run *non-headless* under Xvfb, because Mutter
              #            then has a real window on the X display for XTEST to
              #            target. Not usable while Mutter is --headless, which
              #            draws to no X window at all.
              #  wtype     zwp_virtual_keyboard_v1. The only injection that
              #            speaks Wayland, so it is the one that can reach a
              #            headless compositor — if Mutter implements the
              #            protocol. Unverified; see docs/build-notes.md.
              #  ydotool   uinput, kernel-level. Needs write access to
              #  dotool    /dev/uinput, which is root:root 0600 on this box, so
              #            neither can run as an ordinary user without the
              #            daemon in scripts/e2e/ydotoold-daemon.sh.
              #
              #            They deliver keys, and on this machine the keys went to
              #            the *host* GNOME Shell, not into the nested session --
              #            see the correction in inject-ydotool.sh. A nested
              #            compositor has no input devices of its own, so uinput
              #            cannot reach one; these are here for the case where
              #            emthin owns the seat.
              xdotool
              wtype
              ydotool
              dotool

              # ── E2E: IME, for the same step ────────────────────────────
              # emthin reaches the input method through its own DBus bridge
              # (crates/emthin-dbus: an InputMethod1 frontend that intercepts
              # the bus), so a real fcitx5 here is what makes the IME path
              # exercisable rather than only its stubs.
              fcitx5
              fcitx5-rime

              # ── E2E: a Wayland client that accepts text input ──────────
              # kgx is a GTK4 terminal: it takes text-input-v3, so it is both
              # a figure to look at and an IME consumer.
              gnome-console
              gnome-text-editor

              # ── E2E: clipboard, verifiable with no keys at all ────────
              # Step 7's clipboard half does not need synthetic input.
              # `wl-copy` on the *host* session and `wl-paste` inside the
              # nested one exercises host->client through emthin's proxy, and
              # the reverse exercises client->host. That is the whole of
              # "clipboard host<->client" with no keyboard involved.
              wayland-utils

              # ── E2E: the XTEST configuration needs an X server ───────
              # Xvfb for the non-headless nested compositor that `xdotool`
              # can target. See scripts/e2e/README.md for when that is
              # reachable at all.
              xorgserver
              xclip

              # ── E2E: a nested compositor that maps an X window ────────
              # Weston's x11 backend gives a full Wayland session on top of an X
              # window, under Xvfb, with no logind session involved. That matters:
              # Mutter will only run non-headless when nothing else owns the
              # session (`Failed to take control of the session: EBUSY`), which
              # would mean logging out of the desktop to run the test. Weston does
              # not care, so the xdotool configuration becomes reachable without
              # touching the live session.
              weston

              # ...and one that has an X11 backend: weston 16 in nixpkgs ships
              # without x11.so, but sway's wlroots build does. WLR_BACKENDS=x11
              # makes it map a real X window, which is what XTEST needs.
              sway

              # ── E2E: the harness itself is python + coreutils ────────
              # scripts/e2e/ipc.py speaks the control protocol and state.py
              # pretty-prints the reply. Found the hard way: the clipboard
              # checks passed without it because they do not use python, so the
              # gap only shows up on the keyboard path.
              python3
            ];

          shellHook = ''
            export PKG_CONFIG_PATH="${pkgs.glib.dev}/lib/pkgconfig:${
              pkgs.systemd
            }/lib/pkgconfig''${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
            export LIBRARY_PATH="${pkgs.libxkbcommon}/lib:${pkgs.libglvnd}/lib''${LIBRARY_PATH:+:$LIBRARY_PATH}"

            # Both of these are dlopened at runtime rather than linked, so both
            # have to be findable when `emthin` *runs*, not just when it links:
            # winit opens libwayland, and smithay's EGL backend opens libEGL.
            # Found the hard way — without libEGL emthin gets as far as creating
            # the window and then panics in smithay's ffi.
            export LD_LIBRARY_PATH="${pkgs.wayland}/lib:${pkgs.libglvnd}/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

            # So scripts/e2e/* can tell whether they are already inside the
            # flake and re-enter it if not, rather than the caller having to
            # remember to wrap every invocation. `nix develop` is interactive,
            # so pasting a multi-line block that starts with it swallows the rest
            # of the block.
            export EMTHEIN_DEVSHELL=1

            cat <<'EOF'
            emthin dev shell. Sibling checkouts are path dependencies:
              ../velysterm/crates/mathed_core   (the document engine)
              ../velysterm/crates/mathed_mini   (the paged rasteriser)
            A change to the document model must be verified in both:
              cd ../velysterm && cargo test -p mathed_core -p mathed_mini
              cd -            && cargo test -p emthin
            EOF
          '';
        };
      });
    };
}