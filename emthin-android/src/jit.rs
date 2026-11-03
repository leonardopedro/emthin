//! The australVM Cranelift lane: compile CPS binary IR and execute it
//! in-process — the same pipeline australVM's `tests/cps_tests.rs`
//! drives, moved behind the pad's JNI seam so the Android app can prove
//! on-device that the JIT compiles, finalises (RW→RX over the arena of
//! `JIT_ARENA_SIZE` in the bridge) and runs real machine code.
//!
//! Two properties this module pins deliberately:
//!
//! - **The auth gate stays at its secure default (DenyAll).** The smoke
//!   programs are closed constant-return bodies with no `App` nodes, so
//!   no call ever reaches `auth::check` and nothing here calls
//!   `set_allow_all` — the roundtrip must not weaken the policy layer to
//!   pass. Features that later *do* call `uk_*`/`rs_*` symbols need a
//!   loaded manifest, exactly as the bridge's `--allow-all` comment says.
//! - **Entry names are unique per call.** The JIT engine is thread-local
//!   and persists for the life of the thread, so a second function named
//!   `run` in one engine is a `DuplicateDefinition` — the same reason
//!   `cps_tests` renames per test. The pad counts calls instead of
//!   reusing `run`.
//!
//! The byte-level CPS helpers mirror `australVM/safestos/cranelift/tests/cps_tests.rs`
//! verbatim (magic `CPS1`, one function, tail-context iconst body); if
//! that format changes upstream, this mirror is what must follow.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

use austral_cranelift_bridge as bridge;

// ── CPS binary helpers (mirror of the bridge's cps_tests.rs) ─────────

fn write_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn write_i64(buf: &mut Vec<u8>, v: i64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn write_str(buf: &mut Vec<u8>, s: &str) {
    write_u32(buf, s.len() as u32);
    buf.extend_from_slice(s.as_bytes());
}

/// One CPS v1 binary with a single function named `fname` whose body is
/// the raw `body_data`.
fn cps_v1_named(fname: &str, body_data: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    write_u32(&mut buf, 0x4350_5331); // magic "CPS1"
    write_u32(&mut buf, 1); // func_count
    write_str(&mut buf, fname);
    write_u32(&mut buf, 0); // param_count
    buf.push(0); // ret_type (i64)
    write_u32(&mut buf, body_data.len() as u32);
    buf.extend_from_slice(body_data);
    buf
}

/// Body that returns a constant i64: `0x07` (tail context) `0x01`
/// (iconst) `<val>` — the shape `cps_tests::body_return_const` executes.
fn body_return_const(val: i64) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0x07);
    body.push(0x01);
    write_i64(&mut body, val);
    body
}

// ── compile + execute ────────────────────────────────────────────────

/// Sequence for entry names: unique across every thread in the process,
/// so no engine can ever see the same name twice.
static ENTRY_SEQ: AtomicU64 = AtomicU64::new(0);

/// The bridge's last error, as a string (`null` → a stable placeholder).
fn last_error() -> String {
    let e = bridge::cranelift_last_error();
    if e.is_null() {
        return "unknown error".to_string();
    }
    unsafe { std::ffi::CStr::from_ptr(e) }
        .to_string_lossy()
        .into_owned()
}

/// Compile `cps`, resolving the entry point `name`. `Err` carries
/// `cranelift_last_error` (parse error, auth refusal, finalize failure);
/// a panic inside the compiler is caught by the bridge's own guard and
/// surfaces as an error string too.
fn compile_entry(cps: &[u8], name: &str) -> Result<*const c_void, String> {
    let name_bytes = name.as_bytes();
    let ptr = bridge::compile_to_function_named(
        cps.as_ptr(),
        cps.len(),
        name_bytes.as_ptr(),
        name_bytes.len(),
    );
    if ptr.is_null() {
        Err(last_error())
    } else {
        Ok(ptr)
    }
}

/// Compile a CPS program whose entry returns `value`, execute it, and
/// report what the JIT'd code produced together with the entry name it
/// was compiled under.
///
/// The roundtrip is the proof: bytes → CLIF → machine code → RW→RX
/// finalise over the arena → a real call whose result comes back through
/// the bridge's own calling convention (`module.rs::call_jit_function`).
pub fn jit_roundtrip(value: i64) -> Result<(String, i64), String> {
    let name = format!("run_{}", ENTRY_SEQ.fetch_add(1, Ordering::Relaxed));
    let cps = cps_v1_named(&name, &body_return_const(value));
    let ptr = compile_entry(&cps, &name)?;
    // The 0-arg i64 signature is the bridge's convention for CPS entries
    // (module.rs transmutes the same shape before calling).
    let f: unsafe extern "C" fn() -> i64 = unsafe { std::mem::transmute(ptr) };
    Ok((name, unsafe { f() }))
}

// ── tests: run in the host lane (`cargo test` in this directory) ─────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_returns_the_constant() {
        let (entry, result) = jit_roundtrip(42).expect("jit roundtrip");
        assert_eq!(result, 42);
        assert!(entry.starts_with("run_"), "entry name: {entry}");
    }

    #[test]
    fn repeated_roundtrips_never_collide_on_the_entry_name() {
        // Two compiles in one process would both ask for `run` without
        // the sequence number — the DuplicateDefinition the bridge's own
        // tests rename around. Different values, both must execute.
        for value in [-7, 0, i64::MAX] {
            let (_, result) = jit_roundtrip(value).expect("repeat roundtrip");
            assert_eq!(result, value);
        }
    }

    #[test]
    fn malformed_ir_is_an_error_not_a_crash() {
        // Truncated before func_count: the reader must refuse it, and
        // the refusal must carry the bridge's own message.
        let err = compile_entry(b"\x43\x50\x53", "run_bad").expect_err("must refuse");
        assert!(!err.is_empty(), "error message must be non-empty");
    }
}
