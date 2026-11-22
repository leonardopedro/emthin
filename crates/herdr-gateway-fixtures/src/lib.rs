//! Machine-readable fixtures for the emthin terminal gateway v1 contract.
//!
//! The corpus lives outside this crate so Go and Rust implementations can read
//! the same files. Every JSON fixture is checked for duplicate keys before
//! decoding, then for the gateway's method/parameter allowlists. Binary frames
//! are checked against the documented envelope and per-kind bounds.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde_json::Value;

pub const METHODS: &[&str] = &[
    "hello",
    "sessions.list",
    "terminal.attach",
    "control.request",
    "control.respond",
    "session.stop",
    "fleet.status",
    "fleet.launch",
];

pub const ERROR_KINDS: &[&str] = &[
    "core_mismatch",
    "core_offline",
    "unsupported",
    "not_found",
    "forbidden",
    "in_use",
    "stale_generation",
    "stale_revision",
    "expired",
    "busy",
    "needs_reconciliation",
];

pub const MAX_CONTROL_PAYLOAD: usize = 64 * 1024;
pub const MAX_DATA_PAYLOAD: usize = 16 * 1024;
pub const MAX_FACTS_PAYLOAD: usize = 1024;
pub const MAX_CLOSE_PAYLOAD: usize = 1024;

#[derive(Debug, Deserialize)]
pub struct Fixture {
    pub name: String,
    /// `valid`, `invalid` or `policy` (well-formed JSON that the gateway must
    /// still refuse, such as an arbitrary Core method).
    pub expect: String,
    pub kind: String,
    #[serde(default)]
    pub message: Option<Value>,
    #[serde(default)]
    pub hex: Option<String>,
    /// A frame split across transport reads. Used with `kind: "chunks"`.
    #[serde(default)]
    pub chunks: Option<Vec<String>>,
    /// Number of complete frames the chunks must decode to.
    #[serde(default)]
    pub frames: Option<usize>,
    /// Gateway state-machine steps. Used with `kind: "scenario"`.
    #[serde(default)]
    pub steps: Option<Vec<ScenarioStep>>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioStep {
    pub op: String,
    #[serde(default)]
    pub channel: Option<u32>,
    #[serde(default)]
    pub lease: Option<u64>,
}

impl Fixture {
    pub fn validates(&self) -> bool {
        self.expect == "valid"
    }
}

/// Load and validate every `.json` fixture in `dir`.
pub fn validate_corpus(dir: &Path) -> Result<usize, String> {
    let mut paths: Vec<_> = fs::read_dir(dir)
        .map_err(|error| format!("read {}: {error}", dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(format!("no fixtures in {}", dir.display()));
    }

    let mut names = HashSet::new();
    for path in &paths {
        let raw = fs::read_to_string(path)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        let fixture: Fixture = serde_json::from_str(&raw)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if !names.insert(fixture.name.clone()) {
            return Err(format!("duplicate fixture name {:?}", fixture.name));
        }
        if let Err(error) = check_no_duplicate_keys(&raw) {
            if !fixture.validates()
                && fixture
                    .error
                    .as_deref()
                    .is_some_and(|expected| error.contains(expected))
            {
                continue;
            }
            return Err(format!("{}: {error}", path.display()));
        }
        let result = validate_fixture(&fixture);
        match (fixture.validates(), result) {
            (true, Ok(())) => {}
            (false, Err(actual)) => {
                if let Some(expected) = fixture.error.as_deref() {
                    if !actual.contains(expected) {
                        return Err(format!(
                            "{}: expected error containing {expected:?}, got {actual:?}",
                            fixture.name
                        ));
                    }
                }
            }
            (true, Err(error)) => {
                return Err(format!(
                    "{}: expected valid, got {error:?}",
                    fixture.name
                ))
            }
            (false, Ok(())) => {
                return Err(format!(
                    "{}: expected invalid ({}), but it validated",
                    fixture.name,
                    fixture.error.as_deref().unwrap_or("no reason")
                ))
            }
        }
    }
    Ok(paths.len())
}

pub fn validate_fixture(fixture: &Fixture) -> Result<(), String> {
    match fixture.kind.as_str() {
        "json" => validate_message(
            fixture
                .message
                .as_ref()
                .ok_or_else(|| "json fixture has no message".to_owned())?,
        ),
        "frame" => validate_frame_hex(
            fixture
                .hex
                .as_deref()
                .ok_or_else(|| "frame fixture has no hex".to_owned())?,
        ),
        "chunks" => validate_chunks(fixture),
        "scenario" => validate_scenario(fixture),
        other => Err(format!("unknown fixture kind {other:?}")),
    }
}

/// Decode a frame delivered across several reads. Every declared frame must be
/// complete at the end, and all but the last chunk must actually split one.
fn validate_chunks(fixture: &Fixture) -> Result<(), String> {
    let chunks = fixture
        .chunks
        .as_ref()
        .ok_or_else(|| "chunks fixture has no chunks".to_owned())?;
    if chunks.len() < 2 {
        return Err("chunks fixture needs at least two reads".into());
    }
    let mut pending = Vec::new();
    let mut decoded = 0usize;
    for chunk in chunks {
        pending.extend(decode_hex(chunk)?);
        loop {
            if pending.len() < 9 {
                break;
            }
            let len = u32::from_le_bytes(pending[5..9].try_into().expect("four bytes")) as usize;
            if len > MAX_DATA_PAYLOAD.max(MAX_CONTROL_PAYLOAD) {
                return Err(format!("chunked frame payload is {len} bytes"));
            }
            if pending.len() < 9 + len {
                break;
            }
            let frame: Vec<u8> = pending.drain(..9 + len).collect();
            validate_frame(&frame)?;
            decoded += 1;
        }
    }
    if !pending.is_empty() {
        return Err(format!("chunks leave {} trailing bytes", pending.len()));
    }
    let expected = fixture.frames.unwrap_or(1);
    if decoded != expected {
        return Err(format!("chunks decode to {decoded} frames, want {expected}"));
    }
    Ok(())
}

/// Small state machine for channel reuse and viewer-lease race fixtures. It
/// mirrors the gateway rules: a watch channel cannot write, a closed channel ID
/// may be reused, and an expired or superseded lease cannot send input.
fn validate_scenario(fixture: &Fixture) -> Result<(), String> {
    let steps = fixture
        .steps
        .as_ref()
        .ok_or_else(|| "scenario fixture has no steps".to_owned())?;
    let mut channels: BTreeMap<u32, bool> = BTreeMap::new(); // channel -> writable
    let mut lease = None;
    let mut expired = false;
    for step in steps {
        match step.op.as_str() {
            "attach" => {
                let channel = step
                    .channel
                    .filter(|channel| *channel != 0)
                    .ok_or_else(|| "attach needs a non-zero channel".to_owned())?;
                if channels.contains_key(&channel) {
                    return Err(format!("channel {channel} is already attached"));
                }
                channels.insert(channel, true);
            }
            "watch" => {
                let channel = step
                    .channel
                    .filter(|channel| *channel != 0)
                    .ok_or_else(|| "watch needs a non-zero channel".to_owned())?;
                if channels.contains_key(&channel) {
                    return Err(format!("channel {channel} is already attached"));
                }
                channels.insert(channel, false);
            }
            "close" => {
                let channel = step
                    .channel
                    .ok_or_else(|| "close needs a channel".to_owned())?;
                if channels.remove(&channel).is_none() {
                    return Err(format!("close for unattached channel {channel}"));
                }
            }
            "input" | "resize" => {
                let channel = step
                    .channel
                    .ok_or_else(|| format!("{} needs a channel", step.op))?;
                match channels.get(&channel) {
                    None => return Err(format!("no such channel {channel}")),
                    Some(false) => {
                        return Err(format!("watch channel {channel} cannot send input"))
                    }
                    Some(true) => {}
                }
            }
            "take_lease" => {
                let id = step
                    .lease
                    .filter(|lease| *lease != 0)
                    .ok_or_else(|| "take_lease needs a non-zero lease".to_owned())?;
                lease = Some(id);
                expired = false;
            }
            "expire" | "revoke" => expired = true,
            "input_lease" => {
                let id = step
                    .lease
                    .ok_or_else(|| "input_lease needs a lease".to_owned())?;
                if expired || lease != Some(id) {
                    return Err("stale viewer lease".into());
                }
            }
            other => return Err(format!("unknown scenario op {other:?}")),
        }
    }
    Ok(())
}

/// Strict one-object message validation for the gateway allowlists.
pub fn validate_message(message: &Value) -> Result<(), String> {
    let object = message
        .as_object()
        .ok_or_else(|| "message must be an object".to_owned())?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err("jsonrpc must be \"2.0\"".into());
    }
    if let Some(method) = object.get("method") {
        let method = method.as_str().ok_or_else(|| "method must be a string".to_owned())?;
        if method == "sessions.changed" && !object.contains_key("id") {
            let params = object
                .get("params")
                .and_then(Value::as_object)
                .ok_or_else(|| "sessions.changed params must be an object".to_owned())?;
            reject_unknown_fields(params, &["revision"], "sessions.changed params")?;
            params
                .get("revision")
                .and_then(Value::as_u64)
                .ok_or_else(|| "sessions.changed revision is missing".to_owned())?;
            reject_unknown_fields(object, &["jsonrpc", "method", "params"], "notification")?;
            return Ok(());
        }
        let id = object
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| "request id must be a positive u32".to_owned())?;
        if id == 0 || id > u64::from(u32::MAX) {
            return Err("request id must be a positive u32".into());
        }
        if !METHODS.contains(&method) {
            return Err(format!("method {method:?} is not in the gateway allowlist"));
        }
        let params = object
            .get("params")
            .ok_or_else(|| "request has no params".to_owned())?
            .as_object()
            .ok_or_else(|| "params must be an object".to_owned())?;
        validate_params(method, params)?;
        reject_unknown_fields(object, &["jsonrpc", "id", "method", "params"], "request")?;
        return Ok(());
    }
    if let Some(error) = object.get("error") {
        let error = error.as_object().ok_or_else(|| "error must be an object".to_owned())?;
        let kind = error
            .get("data")
            .and_then(|data| data.get("kind"))
            .and_then(Value::as_str)
            .ok_or_else(|| "error.data.kind is missing".to_owned())?;
        if !ERROR_KINDS.contains(&kind) {
            return Err(format!("unknown error kind {kind:?}"));
        }
        error
            .get("code")
            .and_then(Value::as_i64)
            .ok_or_else(|| "error code is missing".to_owned())?;
        return Ok(());
    }
    if object.contains_key("result") {
        reject_unknown_fields(object, &["jsonrpc", "id", "result"], "reply")?;
        return Ok(());
    }
    // Notifications carry a method but no id.
    if object.contains_key("method") {
        return Err("notification must not carry a request id".into());
    }
    Err("message is neither a request, reply, error nor notification".into())
}

fn validate_params(method: &str, params: &serde_json::Map<String, Value>) -> Result<(), String> {
    let allowed: &[&str] = match method {
        "hello" => &["version", "core_id", "features"],
        "sessions.list" => &["if_revision", "cursor", "limit", "addresses"],
        "terminal.attach" => &[
            "address",
            "generation",
            "mode",
            "history",
            "grid",
            "display",
            "reservation",
        ],
        "control.request" => &["address", "generation"],
        "control.respond" => &["handoff_id", "lease_id", "accept"],
        "session.stop" => &["address", "generation", "request_id"],
        "fleet.status" => &[],
        "fleet.launch" => &["profile", "slot", "request_id"],
        _ => unreachable!("method checked before params"),
    };
    reject_unknown_fields(params, allowed, "params")?;
    match method {
        "hello" => {
            if params.get("version").and_then(Value::as_u64) != Some(1) {
                return Err("hello version must be 1".into());
            }
            let core = params
                .get("core_id")
                .and_then(Value::as_str)
                .ok_or_else(|| "hello core_id is missing".to_owned())?;
            if !is_core_id(core) {
                return Err("hello core_id must be 32 lowercase hex digits".into());
            }
            if let Some(features) = params.get("features") {
                features
                    .as_array()
                    .ok_or_else(|| "features must be an array".to_owned())?;
            }
        }
        "sessions.list" => {
            if let Some(limit) = params.get("limit") {
                let limit = limit
                    .as_u64()
                    .ok_or_else(|| "limit must be an integer".to_owned())?;
                if !(1..=64).contains(&limit) {
                    return Err("limit must be 1..=64".into());
                }
            }
            if let Some(addresses) = params.get("addresses") {
                let addresses = addresses
                    .as_array()
                    .ok_or_else(|| "addresses must be an array".to_owned())?;
                if addresses.len() > 8 {
                    return Err("addresses accepts at most 8 entries".into());
                }
                for address in addresses {
                    validate_address(
                        address
                            .as_str()
                            .ok_or_else(|| "address must be a string".to_owned())?,
                    )?;
                }
            }
        }
        "terminal.attach" => {
            validate_address(
                params
                    .get("address")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "attach address is missing".to_owned())?,
            )?;
            let generation = params
                .get("generation")
                .and_then(Value::as_str)
                .ok_or_else(|| "attach generation is missing".to_owned())?;
            if generation.is_empty() || generation.len() > 128 {
                return Err("generation is out of bounds".into());
            }
            let mode = params
                .get("mode")
                .and_then(Value::as_str)
                .ok_or_else(|| "attach mode is missing".to_owned())?;
            if !matches!(mode, "watch" | "control") {
                return Err("attach mode must be watch or control".into());
            }
            if let Some(history) = params.get("history") {
                let history = history
                    .as_u64()
                    .ok_or_else(|| "history must be an integer".to_owned())?;
                if history > 1000 {
                    return Err("history must be at most 1000".into());
                }
            }
            if mode == "watch" && (params.contains_key("grid") || params.contains_key("display")) {
                return Err("only a controller may send grid or display".into());
            }
            if let Some(grid) = params.get("grid") {
                validate_grid(grid)?;
            }
        }
        "control.request" | "session.stop" => {
            validate_address(
                params
                    .get("address")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "address is missing".to_owned())?,
            )?;
            if params
                .get("generation")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            {
                return Err("generation is missing".into());
            }
        }
        "control.respond" => {
            if params.get("handoff_id").and_then(Value::as_str).is_none() {
                return Err("handoff_id is missing".into());
            }
            if params.get("lease_id").and_then(Value::as_str).is_none() {
                return Err("lease_id is missing".into());
            }
            if params.get("accept").and_then(Value::as_bool).is_none() {
                return Err("accept must be a boolean".into());
            }
        }
        "fleet.launch" => {
            for key in ["profile", "slot", "request_id"] {
                let value = params
                    .get(key)
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("{key} is missing"))?;
                if value.is_empty() || value.len() > 64 {
                    return Err(format!("{key} is out of bounds"));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_address(address: &str) -> Result<(), String> {
    if address.is_empty() || address.len() > 128 {
        return Err("address is out of bounds".into());
    }
    if address.contains('/') || address.contains('\\') || address.contains(':') {
        return Err("address must be an opaque local terminal address".into());
    }
    if address
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        Ok(())
    } else {
        Err("address has characters outside the local address alphabet".into())
    }
}

fn validate_grid(grid: &Value) -> Result<(), String> {
    let object = grid
        .as_object()
        .ok_or_else(|| "grid must be an object".to_owned())?;
    reject_unknown_fields(object, &["cols", "rows"], "grid")?;
    let cols = object
        .get("cols")
        .and_then(Value::as_u64)
        .ok_or_else(|| "grid cols is missing".to_owned())?;
    let rows = object
        .get("rows")
        .and_then(Value::as_u64)
        .ok_or_else(|| "grid rows is missing".to_owned())?;
    if cols > 300 || rows > 120 {
        return Err("grid exceeds 300x120".into());
    }
    if cols < 20 || rows < 5 {
        return Err("controller grid needs at least 20x5".into());
    }
    Ok(())
}

fn reject_unknown_fields(
    object: &serde_json::Map<String, Value>,
    allowed: &[&str],
    what: &str,
) -> Result<(), String> {
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("{what} has unexpected property {key:?}"));
    }
    Ok(())
}

fn is_core_id(text: &str) -> bool {
    text.len() == 32
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Parse and validate one `channel|kind|length|payload` frame.
pub fn validate_frame_hex(hex: &str) -> Result<(), String> {
    let bytes = decode_hex(hex)?;
    validate_frame(&bytes)
}

pub fn validate_frame(frame: &[u8]) -> Result<(), String> {
    if frame.len() < 9 {
        return Err("frame is shorter than its header".into());
    }
    let channel = u32::from_le_bytes(frame[0..4].try_into().unwrap());
    let kind = frame[4];
    let len = u32::from_le_bytes(frame[5..9].try_into().unwrap()) as usize;
    if frame.len() != 9 + len {
        return Err("frame length does not match its payload".into());
    }
    let payload = &frame[9..];
    match kind {
        0 => {
            if channel == 0 {
                if payload.len() > MAX_CONTROL_PAYLOAD {
                    return Err("control DATA exceeds 64 KiB".into());
                }
                serde_json::from_slice::<Value>(payload)
                    .map_err(|_| "control DATA is not JSON".to_owned())?;
            } else if payload.len() > MAX_DATA_PAYLOAD {
                return Err("terminal DATA exceeds 16 KiB".into());
            }
        }
        1 => {
            if channel == 0 {
                return Err("CREDIT is not a control-channel frame".into());
            }
            if payload.len() != 4 {
                return Err("CREDIT payload must be four bytes".into());
            }
        }
        2 => {
            if channel == 0 {
                return Err("CLOSE is not a control-channel frame".into());
            }
            if payload.len() > MAX_CLOSE_PAYLOAD {
                return Err("CLOSE reason exceeds 1 KiB".into());
            }
            if !payload.is_empty() {
                serde_json::from_slice::<Value>(payload)
                    .map_err(|_| "CLOSE reason is not JSON".to_owned())?;
            }
        }
        3 => {
            if channel == 0 {
                return Err("RESIZE is not a control-channel frame".into());
            }
            if payload.len() != 4 {
                return Err("RESIZE payload must be four bytes".into());
            }
            let cols = u16::from_le_bytes([payload[0], payload[1]]);
            let rows = u16::from_le_bytes([payload[2], payload[3]]);
            if cols == 0 || rows == 0 || cols > 300 || rows > 120 {
                return Err("RESIZE grid is outside 1..=300 by 1..=120".into());
            }
        }
        4 => {
            if channel == 0 || payload.len() != 1 || payload[0] > 1 {
                return Err("FOCUS must be one zero-or-one byte on a stream".into());
            }
        }
        5 => {
            if channel == 0 || payload.len() > MAX_FACTS_PAYLOAD {
                return Err("FACTS must be bounded JSON on a stream".into());
            }
            serde_json::from_slice::<Value>(payload)
                .map_err(|_| "FACTS is not JSON".to_owned())?;
        }
        6 => {
            if channel == 0 || payload.len() > MAX_DATA_PAYLOAD {
                return Err("AUTOMATIC_INPUT exceeds 16 KiB".into());
            }
        }
        other => return Err(format!("unknown frame kind {other}")),
    }
    Ok(())
}

fn decode_hex(hex: &str) -> Result<Vec<u8>, String> {
    let compact: String = hex.split_whitespace().collect();
    if !compact.len().is_multiple_of(2) || !compact.is_ascii() {
        return Err("hex must contain whole bytes".into());
    }
    let mut bytes = Vec::with_capacity(compact.len() / 2);
    for pair in compact.as_bytes().as_chunks::<2>().0 {
        let text = std::str::from_utf8(pair).map_err(|_| "hex is not UTF-8".to_owned())?;
        bytes.push(u8::from_str_radix(text, 16).map_err(|_| "invalid hex byte".to_owned())?);
    }
    Ok(bytes)
}

/// Reject duplicate object keys before `serde_json::Value` collapses them.
pub fn check_no_duplicate_keys(raw: &str) -> Result<(), String> {
    let mut deserializer = serde_json::Deserializer::from_str(raw);
    NoDuplicates
        .deserialize(&mut deserializer)
        .map_err(|error| format!("duplicate-key check: {error}"))?;
    deserializer
        .end()
        .map_err(|error| format!("trailing JSON: {error}"))
}

struct NoDuplicates;

impl<'de> DeserializeSeed<'de> for NoDuplicates {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: de::Deserializer<'de>,
    {
        deserializer.deserialize_any(NoDuplicatesVisitor)
    }
}

struct NoDuplicatesVisitor;

impl<'de> Visitor<'de> for NoDuplicatesVisitor {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_map<A>(self, mut map: A) -> Result<(), A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut seen = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(de::Error::custom(format!("duplicate key {key:?}")));
            }
            map.next_value_seed(NoDuplicates)?;
        }
        Ok(())
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<(), A::Error>
    where
        A: SeqAccess<'de>,
    {
        while seq.next_element_seed(NoDuplicates)?.is_some() {}
        Ok(())
    }

    fn visit_bool<E>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_string<E>(self, _: String) -> Result<(), E> {
        Ok(())
    }
    fn visit_none<E>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_some<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: de::Deserializer<'de>,
    {
        deserializer.deserialize_any(NoDuplicatesVisitor)
    }
    fn visit_newtype_struct<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: de::Deserializer<'de>,
    {
        deserializer.deserialize_any(NoDuplicatesVisitor)
    }
    fn visit_enum<A>(self, data: A) -> Result<(), A::Error>
    where
        A: de::EnumAccess<'de>,
    {
        use serde::de::VariantAccess;
        let (_, variant) = data.variant::<IgnoredAny>()?;
        variant.unit_variant()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture(name: &str, expect: &str, kind: &str) -> Fixture {
        Fixture {
            name: name.into(),
            expect: expect.into(),
            kind: kind.into(),
            message: None,
            hex: None,
            chunks: None,
            frames: None,
            steps: None,
            error: None,
        }
    }

    #[test]
    fn corpus_loads_and_every_case_matches_its_expectation() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/herdr-gateway-v1");
        let count = validate_corpus(&dir).expect("corpus validates");
        assert!(count >= 18, "expected the documented matrix, got {count}");
    }

    #[test]
    fn duplicate_keys_are_rejected_before_decoding() {
        assert!(check_no_duplicate_keys(r#"{"a":1,"a":2}"#).is_err());
        assert!(check_no_duplicate_keys(r#"{"a":{"b":1,"b":2}}"#).is_err());
        assert!(check_no_duplicate_keys(r#"{"a":[{"b":1},{"b":2}]}"#).is_ok());
    }

    #[test]
    fn arbitrary_core_methods_and_machine_addresses_are_policy_errors() {
        let mut f = fixture("core-eval", "invalid", "json");
        f.message = Some(json!({
            "jsonrpc": "2.0", "id": 1, "method": "core.eval", "params": {}
        }));
        assert!(validate_fixture(&f).is_err());

        let mut f = fixture("absolute-address", "invalid", "json");
        f.message = Some(json!({
            "jsonrpc": "2.0", "id": 1, "method": "terminal.attach",
            "params": {"address": "/home/me/t3", "generation": "g", "mode": "watch"}
        }));
        assert!(validate_fixture(&f).is_err());
    }

    #[test]
    fn chunked_frames_and_scenarios_enforce_transport_state() {
        let mut chunks = fixture("split", "valid", "chunks");
        chunks.chunks = Some(vec![
            "07 00 00 00 00 03".into(),
            "00 00 00 18 4f 4b".into(),
        ]);
        chunks.frames = Some(1);
        assert!(validate_fixture(&chunks).is_ok());

        let mut reused = fixture("reuse", "valid", "scenario");
        reused.steps = Some(vec![
            ScenarioStep { op: "watch".into(), channel: Some(7), lease: None },
            ScenarioStep { op: "close".into(), channel: Some(7), lease: None },
            ScenarioStep { op: "attach".into(), channel: Some(7), lease: None },
            ScenarioStep { op: "input".into(), channel: Some(7), lease: None },
        ]);
        assert!(validate_fixture(&reused).is_ok());

        let mut expired = fixture("expired", "invalid", "scenario");
        expired.steps = Some(vec![
            ScenarioStep { op: "take_lease".into(), channel: None, lease: Some(1) },
            ScenarioStep { op: "expire".into(), channel: None, lease: None },
            ScenarioStep { op: "input_lease".into(), channel: None, lease: Some(1) },
        ]);
        assert!(validate_fixture(&expired).unwrap_err().contains("stale"));
    }

    #[test]
    fn frame_bounds_and_kinds_are_enforced() {
        let mut data = vec![7, 0, 0, 0, 0];
        data.extend((MAX_DATA_PAYLOAD as u32 + 1).to_le_bytes());
        data.resize(9 + MAX_DATA_PAYLOAD + 1, b'x');
        assert!(validate_frame(&data).is_err());

        let mut credit = vec![7, 0, 0, 0, 1];
        credit.extend(3u32.to_le_bytes());
        credit.extend([1, 2, 3]);
        assert!(validate_frame(&credit).is_err(), "CREDIT is four bytes");

        let mut focus = vec![7, 0, 0, 0, 4];
        focus.extend(1u32.to_le_bytes());
        focus.push(2);
        assert!(validate_frame(&focus).is_err());
    }
}
