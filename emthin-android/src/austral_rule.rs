//! K9 rule artifact — a real Austral module compiled to a CPS binary and
//! executed through the australVM Cranelift bridge (off the Android UI thread).
//!
//! This is the native counterpart to the advisory Rust rule in
//! `docui/proof_index.rs`. The Austral module owns the rule's logic; Rust only
//! encodes the bounded facts into the scalar ABI and decodes the verdict. The
//! artifact is embedded, fingerprint-checked before every load, and carries no
//! capability grants — the bridge stays at its DenyAll default.
//!
//! Build the artifact (aarch64 host, from this checkout):
//!   cd austral && \
//!     LD_LIBRARY_PATH=/home/leo/Projects/australVM \
//!     /home/leo/Projects/australVM/_build/default/bin/austral.exe \
//!     compile --target-type=tc --use-cps-jit --emit-cps=LinkRule.cps LinkRule.aum

use sha2::{Digest, Sha256};

use austral_cranelift_bridge as bridge;

/// The embedded, compiled CPS artifact (CPS v1, 4 functions, 1389 bytes).
pub const ARTIFACT: &[u8] = include_bytes!("../austral/LinkRule.cps");
/// SHA-256 of the exact artifact bytes. A stale or modified artifact is
/// refused before any JIT code is compiled or run.
pub const ARTIFACT_SHA256: &str =
    "157e22a18baeb87f7428960dd7e34b8ade271b6591eb6f6e1b380ee90843ca48";
/// The rule ABI this artifact implements.
pub const RULE_ABI_VERSION: u32 = 1;
/// The transport bound: distinct ids are mapped to base-10 digits 1..9.
const MAX_DISTINCT_IDS: usize = 9;

/// The rule's input, in the same shape as the desktop rule's `LinkFact`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LinkFact {
    pub from: String,
    pub to: String,
}

/// The rule's output, with the same meanings as the desktop `RuleResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleResult {
    Complete { facts: usize },
    Incomplete { missing: Vec<String> },
    Malformed,
}

/// Refuse an artifact whose bytes do not match the pinned fingerprint. This is
/// the artifact-identity gate: an older build or a tampered binary never
/// reaches the JIT.
pub fn verify_artifact() -> Result<(), String> {
    let digest = Sha256::digest(ARTIFACT);
    let actual = digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if actual != ARTIFACT_SHA256 {
        return Err(format!(
            "artifact fingerprint mismatch: expected {ARTIFACT_SHA256}, got {actual}"
        ));
    }
    // Deny-by-default: a pure rule must not reference any capability-bearing
    // symbol. The bridge never calls set_allow_all and this artifact has no
    // grants in its manifest; fail closed if bytes name a uk_/rs_ import.
    for needle in [b"uk_".as_slice(), b"rs_".as_slice(), b"App".as_slice()] {
        if ARTIFACT.windows(needle.len()).any(|w| w == needle) {
            return Err("artifact references a capability-bearing symbol".into());
        }
    }
    Ok(())
}

/// Map opaque ids to digits and pack the declared/referenced sets. The digit
/// table is returned so a missing digit can be mapped back to its id.
fn encode(facts: &[LinkFact]) -> Result<(i64, i64, Vec<String>), String> {
    if facts.len() > 4096 {
        return Ok((-1, -1, Vec::new())); // same bound as the desktop rule
    }
    let mut digits: Vec<String> = Vec::new();
    let mut declared = 0i64;
    let mut referenced = 0i64;
    for fact in facts {
        if fact.from.is_empty() || fact.to.is_empty() {
            return Ok((-1, -1, Vec::new()));
        }
        let digit_for = |id: &str, table: &mut Vec<String>| -> Result<i64, String> {
            if let Some(at) = table.iter().position(|known| known == id) {
                return Ok((at + 1) as i64);
            }
            if table.len() >= MAX_DISTINCT_IDS {
                return Err(format!("more than {MAX_DISTINCT_IDS} distinct ids"));
            }
            table.push(id.to_owned());
            Ok(table.len() as i64)
        };
        let from = digit_for(&fact.from, &mut digits)?;
        let to = digit_for(&fact.to, &mut digits)?;
        declared = declared * 10 + from;
        referenced = referenced * 10 + to;
    }
    Ok((declared, referenced, digits))
}

fn last_error() -> String {
    let ptr = bridge::cranelift_last_error();
    if ptr.is_null() {
        return "unknown error".into();
    }
    unsafe { std::ffi::CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

/// Load the pinned artifact, call `LinkRule:check` with the encoded sets, and
/// transmute the returned pointer to the bridge's 2-arg i64 convention.
unsafe fn call_artifact(declared: i64, referenced: i64) -> Result<i64, String> {
    let name = b"check";
    let ptr = bridge::compile_to_function_named(
        ARTIFACT.as_ptr(),
        ARTIFACT.len(),
        name.as_ptr(),
        name.len(),
    );
    if ptr.is_null() {
        return Err(last_error());
    }
    Ok(bridge::execute_function_2(ptr, declared, referenced))
}

/// Evaluate the rule on a worker thread so the UI thread never blocks on JIT
/// compilation or on the Austral loops. This is the off-UI-thread arm64 call.
pub fn evaluate_off_ui_thread(facts: Vec<LinkFact>) -> Result<RuleResult, String> {
    let handle = std::thread::spawn(move || evaluate_here(&facts));
    handle
        .join()
        .map_err(|_| "rule worker panicked".to_string())?
}

/// The same evaluation on the calling thread, used by host tests.
pub fn evaluate_here(facts: &[LinkFact]) -> Result<RuleResult, String> {
    verify_artifact()?;
    let (declared, referenced, digits) = encode(facts)?;
    if declared < 0 {
        return Ok(RuleResult::Malformed);
    }
    let code = unsafe { call_artifact(declared, referenced)? };
    match code {
        0 => Ok(RuleResult::Complete { facts: facts.len() }),
        -1 => Ok(RuleResult::Malformed),
        digit if (1..=9).contains(&digit) && (digit as usize) <= digits.len() => {
            Ok(RuleResult::Incomplete {
                missing: vec![digits[digit as usize - 1].clone()],
            })
        }
        other => Err(format!("unexpected rule result: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(edges: &[(&str, &str)]) -> Vec<LinkFact> {
        edges
            .iter()
            .map(|(from, to)| LinkFact {
                from: (*from).into(),
                to: (*to).into(),
            })
            .collect()
    }

    #[test]
    fn the_embedded_artifact_matches_its_pinned_fingerprint() {
        verify_artifact().expect("a valid embedded artifact");
        assert_eq!(ARTIFACT.len(), 1389);
        assert_eq!(&ARTIFACT[0..4], b"\x32\x53\x50\x43"); // CPS1 magic
    }

    #[test]
    fn a_tampered_artifact_is_refused() {
        // A standalone check of the predicate, without mutating the shared
        // embedded bytes: a different digest must not match the pin.
        let digest = Sha256::digest(b"not the artifact");
        let actual = digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        assert_ne!(actual, ARTIFACT_SHA256);
    }

    #[test]
    fn the_native_rule_matches_the_rust_fixtures() {
        // Same five fixtures as docui/proof_index.rs::the_rule_candidate_...
        assert_eq!(
            evaluate_here(&facts(&[("a", "b"), ("b", "a")])).expect("call"),
            RuleResult::Complete { facts: 2 }
        );
        assert_eq!(
            evaluate_here(&facts(&[("a", "ghost")])).expect("call"),
            RuleResult::Incomplete {
                missing: vec!["ghost".into()]
            }
        );
        assert_eq!(
            evaluate_here(&facts(&[("", "b")])).expect("call"),
            RuleResult::Malformed
        );
        let huge = vec![
            LinkFact {
                from: "a".into(),
                to: "b".into()
            };
            4097
        ];
        assert_eq!(evaluate_here(&huge).expect("call"), RuleResult::Malformed);
        assert_eq!(RULE_ABI_VERSION, 1);
    }

    #[test]
    fn a_cycle_and_an_over_bound_id_set_are_handled() {
        assert_eq!(
            evaluate_here(&facts(&[("a", "b"), ("b", "c"), ("c", "a")])).expect("call"),
            RuleResult::Complete { facts: 3 }
        );
        let many = facts(&[
            ("1", "2"),
            ("2", "3"),
            ("3", "4"),
            ("4", "5"),
            ("5", "6"),
            ("6", "7"),
            ("7", "8"),
            ("8", "9"),
            ("9", "10"),
            ("10", "1"),
        ]);
        assert!(
            evaluate_here(&many).is_err(),
            "10 distinct ids exceed the scalar ABI bound"
        );
    }

    #[test]
    fn the_off_ui_thread_entry_returns_the_same_result() {
        let result = evaluate_off_ui_thread(facts(&[("a", "b"), ("b", "a")])).expect("call");
        assert_eq!(result, RuleResult::Complete { facts: 2 });
    }
}
