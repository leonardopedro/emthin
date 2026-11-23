//! Build the pinned libghostty-vt for desktop, or link the prebuilt Android
//! archive produced by `scripts/build-ghostty-vt`.
//!
//! The Rust binding source is the same one Herdr Core uses. Keeping the
//! linkage here avoids a second copy of the binding while still allowing the
//! Gradle-free Android build to skip Zig on the host.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let target = std::env::var("TARGET").unwrap();
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("../..");
    let pin = GhosttyPin::load(&root);
    if target.contains("android") {
        android_link(&manifest, &target, &pin);
    } else {
        desktop_build(&manifest, &target, &pin);
    }
}

/// The compatibility revision recorded in `docs/HERDR_COMPATIBILITY.toml`.
/// Build scripts cannot use a TOML crate, so read the single `[ghostty]`
/// revision line directly. A missing manifest is not fatal: the build still
/// runs against the checkout, but setup errors cannot name the pin.
struct GhosttyPin {
    revision: Option<String>,
    setup: String,
}

impl GhosttyPin {
    fn load(root: &Path) -> Self {
        let manifest = root.join("docs/HERDR_COMPATIBILITY.toml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        let revision = std::fs::read_to_string(&manifest)
            .ok()
            .and_then(|text| revision_in_ghostty_section(&text));
        Self {
            revision,
            setup: "git -C ../herdr-core submodule update --init --depth 1 vendor/ghostty"
                .to_string(),
        }
    }

    fn revision(&self) -> &str {
        self.revision
            .as_deref()
            .unwrap_or("<unknown: read docs/HERDR_COMPATIBILITY.toml>")
    }

    fn missing_source(&self, path: &Path, manifest: &Path) -> ! {
        panic!(
            "Ghostty VT source not found at {}\n\
             required revision: {} (docs/HERDR_COMPATIBILITY.toml [ghostty])\n\
             setup: {}\n\
             source path is resolved from the crate manifest {}",
            path.display(),
            self.revision(),
            self.setup,
            manifest.display(),
        );
    }

    fn zig_error(&self, path: &Path, error: &str) -> ! {
        panic!(
            "cannot build libghostty-vt from {}\n\
             required revision: {}\n\
             Zig 0.16 is required on PATH (or set ZIG): {error}\n\
             setup: {}",
            path.display(),
            self.revision(),
            self.setup,
        );
    }
}

fn revision_in_ghostty_section(text: &str) -> Option<String> {
    let mut in_ghostty = false;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            in_ghostty = line == "[ghostty]";
            continue;
        }
        if !in_ghostty {
            continue;
        }
        if let Some(value) = line.strip_prefix("revision") {
            let value = value.trim_start().strip_prefix('=')?.trim();
            return value
                .strip_prefix('"')?
                .strip_suffix('"')
                .map(str::to_string);
        }
    }
    None
}

fn desktop_build(manifest: &Path, target: &str, pin: &GhosttyPin) {
    println!("cargo:rerun-if-env-changed=GHOSTTY_VT_SOURCE");
    // The default path is relative to the crate manifest, unlike the
    // `#[path]` in src/lib.rs which is relative to that source file.
    let src = std::env::var_os("GHOSTTY_VT_SOURCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join("../../../herdr-core/vendor/ghostty"));
    let src = src
        .canonicalize()
        .unwrap_or_else(|_| pin.missing_source(&src, manifest));
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    for path in ["src", "build.zig", "build.zig.zon", "include"] {
        println!("cargo:rerun-if-changed={}", src.join(path).display());
    }
    println!("cargo:rerun-if-env-changed=ZIG");

    let host = std::env::var("HOST").unwrap();
    let zig = PathBuf::from(std::env::var("ZIG").unwrap_or_else(|_| "zig".into()));
    let mut build = Command::new(&zig);
    build.current_dir(&src).args([
        "build",
        "-Demit-lib-vt",
        "-Doptimize=ReleaseFast",
        "-Dcpu=baseline",
    ]);
    if target != host {
        build.arg(format!("-Dtarget={}", zig_target(target)));
    }
    let status = build
        .arg("--prefix")
        .arg(out.join("zig-out"))
        .arg("--cache-dir")
        .arg(out.join("zig-cache"))
        .status()
        .unwrap_or_else(|error| pin.zig_error(&src, &format!("cannot run {zig:?}: {error}")));
    if !status.success() {
        pin.zig_error(&src, &format!("zig build exited with {status}"));
    }

    let lib = out.join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::copy(
        out.join("zig-out/lib/libghostty-vt.a"),
        lib.join("libghostty-vt.a"),
    )
    .unwrap();
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt");
}

fn android_link(manifest: &Path, target: &str, pin: &GhosttyPin) {
    println!("cargo:rerun-if-env-changed=GHOSTTY_VT_ANDROID_LIB");
    let abi = android_abi(target).unwrap_or_else(|| {
        panic!(
            "unsupported Android target {target}\n\
             required revision: {}\n\
             the prebuilt lane supports arm64-v8a only; set GHOSTTY_VT_ANDROID_LIB to a \
             matching archive or add the target to scripts/build-ghostty-vt",
            pin.revision(),
        )
    });
    let lib = std::env::var_os("GHOSTTY_VT_ANDROID_LIB")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join(format!("../../target/ghostty-vt/{abi}/lib")));
    let archive = lib.join("libghostty-vt.a");
    if !archive.is_file() {
        panic!(
            "Ghostty VT Android archive not found at {}\n\
             target: {target} (ABI {abi})\n\
             required revision: {} (docs/HERDR_COMPATIBILITY.toml [ghostty])\n\
             setup: GHOSTTY_VT_ANDROID_LIB={} scripts/build-ghostty-vt\n\
             (Android builds need ANDROID_NDK_HOME so the script builds {abi})",
            archive.display(),
            pin.revision(),
            lib.display(),
        );
    }
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt");
}

fn android_abi(target: &str) -> Option<&'static str> {
    if target.starts_with("aarch64-linux-android") {
        Some("arm64-v8a")
    } else {
        None
    }
}

fn zig_target(triple: &str) -> String {
    let arch = triple.split('-').next().unwrap_or("native");
    let os = if triple.contains("apple") {
        "macos"
    } else if triple.contains("windows") {
        "windows"
    } else {
        "linux"
    };
    let abi = ["musl", "gnu", "msvc"]
        .into_iter()
        .find(|abi| triple.ends_with(abi));
    abi.map_or(format!("{arch}-{os}"), |abi| format!("{arch}-{os}-{abi}"))
}
