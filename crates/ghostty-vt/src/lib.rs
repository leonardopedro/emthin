//! Linkage wrapper for the pinned Ghostty VT binding.
//!
//! The module is the same source Herdr Core uses; this crate only replaces the
//! build script so Android can link the prebuilt arm64 archive instead of
//! invoking Zig for the host.

// The vendored source belongs to the sibling herdr-core checkout and has its
// own formatting gate there; rustfmt must not rewrite it from this crate.
#[rustfmt::skip]
#[path = "../../../../herdr-core/crates/ghostty-vt/src/lib.rs"]
mod binding;

pub use binding::*;
