//! Linkage wrapper for the pinned Ghostty VT binding.
//!
//! The module is the same source Herdr Core uses; this crate only replaces the
//! build script so Android can link the prebuilt arm64 archive instead of
//! invoking Zig for the host.

#[path = "../../../../herdr-core/crates/ghostty-vt/src/lib.rs"]
mod binding;

pub use binding::*;
