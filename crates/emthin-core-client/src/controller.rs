//! Session and catalogue state on top of the wire client.
//!
//! Transports remain outside this module. The catalogue applies only the
//! documented `events.subscribe` snapshot/event stream, while the session
//! issues viewer leases that cannot survive a reconnect, endpoint change,
//! release, or replacement program.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::Error;

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Snapshot,
    Added(String),
    Updated(String),
    Removed(String),
    Doc { name: String, version: u64 },
}

#[derive(Debug, Clone)]
pub struct Catalogue {
    seq: u64,
    epoch: u64,
    terminals: BTreeMap<String, Value>,
    program_generations: BTreeMap<String, u64>,
}

impl Catalogue {
    pub fn new(seq: u64, terminals: Vec<Value>) -> Self {
        let mut catalogue = Self {
            seq,
            epoch: 0,
            terminals: BTreeMap::new(),
            program_generations: BTreeMap::new(),
        };
        catalogue.replace(seq, terminals);
        catalogue
    }

    pub fn last_seq(&self) -> u64 {
        self.seq
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn len(&self) -> usize {
        self.terminals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.terminals.is_empty()
    }

    pub fn terminal(&self, address: &str) -> Option<&Value> {
        self.terminals.get(address)
    }

    pub fn terminals(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.terminals.iter()
    }

    pub fn program_generation(&self, address: &str) -> u64 {
        self.program_generations.get(address).copied().unwrap_or(0)
    }

    /// Apply one JSON notification line from the event stream.
    pub fn apply_line(&mut self, line: &[u8]) -> Result<Vec<Change>, Error> {
        let value: Value =
            serde_json::from_slice(line).map_err(|e| Error::Protocol(e.to_string()))?;
        self.apply(&value)
    }

    pub fn apply(&mut self, value: &Value) -> Result<Vec<Change>, Error> {
        let method = value
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Protocol("catalogue message has no method".into()))?;
        let params = value
            .get("params")
            .ok_or_else(|| Error::Protocol(format!("{method} has no params")))?;
        match method {
            "heartbeat" => {
                let seq = seq(params)?;
                self.seq = self.seq.max(seq);
                Ok(Vec::new())
            }
            "missed" => {
                let seq = seq(params)?;
                let terminals = params
                    .get("terminals")
                    .and_then(Value::as_array)
                    .cloned()
                    .ok_or_else(|| Error::Protocol("missed has no terminals".into()))?;
                self.replace(seq, terminals);
                Ok(vec![Change::Snapshot])
            }
            "event" => {
                let seq = seq(params)?;
                if seq <= self.seq {
                    return Ok(Vec::new());
                }
                self.seq = seq;
                self.apply_event(params)
            }
            other => Err(Error::Protocol(format!("unknown catalogue method {other}"))),
        }
    }

    fn replace(&mut self, seq: u64, terminals: Vec<Value>) {
        self.seq = seq;
        self.epoch = self.epoch.saturating_add(1);
        self.terminals.clear();
        self.program_generations.clear();
        for terminal in terminals {
            if let Some(address) = terminal
                .get("address")
                .and_then(Value::as_str)
                .map(str::to_owned)
            {
                self.program_generations.insert(address.clone(), 1);
                self.terminals.insert(address, terminal);
            }
        }
    }

    fn apply_event(&mut self, params: &Value) -> Result<Vec<Change>, Error> {
        let kind = params
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Protocol("event has no type".into()))?;
        let data = params.get("data").cloned().unwrap_or(Value::Null);
        let terminal = params.get("terminal").and_then(Value::as_str);
        match kind {
            "created" | "restarted" => {
                let address = data
                    .get("address")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Error::Protocol(format!("{kind} has no address")))?
                    .to_owned();
                if kind == "restarted" {
                    let generation = self.program_generations.entry(address.clone()).or_default();
                    *generation = generation.saturating_add(1);
                } else {
                    self.program_generations.entry(address.clone()).or_insert(1);
                }
                self.terminals.insert(address.clone(), data);
                Ok(vec![if kind == "created" {
                    Change::Added(address)
                } else {
                    Change::Updated(address)
                }])
            }
            "lost" => {
                let address = required_terminal(terminal)?;
                if let Some(record) = self.terminals.get_mut(address) {
                    merge(record, &data);
                    record["pid"] = Value::Null;
                    record["foreground"] = Value::Null;
                }
                Ok(vec![Change::Updated(address.to_owned())])
            }
            "finished" => {
                let address = required_terminal(terminal)?;
                if let Some(record) = self.terminals.get_mut(address) {
                    merge(record, &data);
                    record["state"] = Value::String("finished".into());
                    record["status"] = Value::Null;
                    record["foreground"] = Value::Null;
                }
                Ok(vec![Change::Updated(address.to_owned())])
            }
            "removed" => {
                let address = required_terminal(terminal)?;
                self.terminals.remove(address);
                self.program_generations.remove(address);
                Ok(vec![Change::Removed(address.to_owned())])
            }
            "status" => {
                let address = required_terminal(terminal)?;
                if let Some(record) = self.terminals.get_mut(address) {
                    copy(record, &data, "status");
                    copy(record, &data, "records");
                    copy(record, &data, "reported");
                }
                Ok(vec![Change::Updated(address.to_owned())])
            }
            "turn" => {
                let address = required_terminal(terminal)?;
                if let Some(record) = self.terminals.get_mut(address) {
                    copy(record, &data, "turn");
                }
                Ok(vec![Change::Updated(address.to_owned())])
            }
            "title" => {
                let address = required_terminal(terminal)?;
                if let Some(record) = self.terminals.get_mut(address) {
                    copy(record, &data, "title");
                }
                Ok(vec![Change::Updated(address.to_owned())])
            }
            "labels" => {
                let address = required_terminal(terminal)?;
                if let Some(record) = self.terminals.get_mut(address) {
                    copy(record, &data, "labels");
                    if let Some(full) = data.get("record").cloned() {
                        self.terminals.insert(address.to_owned(), full);
                    }
                }
                Ok(vec![Change::Updated(address.to_owned())])
            }
            "size" => {
                let address = required_terminal(terminal)?;
                if let Some(record) = self.terminals.get_mut(address) {
                    copy(record, &data, "cols");
                    copy(record, &data, "rows");
                }
                Ok(vec![Change::Updated(address.to_owned())])
            }
            "foreground" => {
                let address = required_terminal(terminal)?;
                if let Some(record) = self.terminals.get_mut(address) {
                    record["foreground"] = data;
                }
                Ok(vec![Change::Updated(address.to_owned())])
            }
            "doc" => {
                let name = data
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Error::Protocol("doc event has no name".into()))?
                    .to_owned();
                let version = data.get("version").and_then(Value::as_u64).unwrap_or(0);
                Ok(vec![Change::Doc { name, version }])
            }
            // New event types must not break an older client.
            _ => Ok(Vec::new()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub core_id: String,
    pub socket: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Detached,
    Watching,
    Controlling,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewerLease {
    connection: u64,
    viewer: u64,
    program: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    EndpointMismatch,
    StaleViewer,
    NotControlling,
}

struct SessionState {
    endpoint: Endpoint,
    address: String,
    connection: u64,
    viewer: u64,
    program: u64,
    mode: Mode,
}

/// One figure's view of a holder. Dropping the session does not stop the
/// holder; only CLOSE on a live protocol channel does.
pub struct Session {
    state: SessionState,
}

impl Session {
    pub fn new(endpoint: Endpoint, address: impl Into<String>) -> Self {
        Self {
            state: SessionState {
                endpoint,
                address: address.into(),
                connection: 1,
                viewer: 1,
                program: 1,
                mode: Mode::Watching,
            },
        }
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.state.endpoint
    }

    pub fn address(&self) -> &str {
        &self.state.address
    }

    pub fn mode(&self) -> Mode {
        self.state.mode
    }

    pub fn connection_generation(&self) -> u64 {
        self.state.connection
    }

    pub fn program_generation(&self) -> u64 {
        self.state.program
    }

    pub fn watch(&mut self) -> ViewerLease {
        self.state.viewer = self.state.viewer.saturating_add(1);
        self.state.mode = Mode::Watching;
        self.lease()
    }

    pub fn take_control(&mut self) -> ViewerLease {
        self.state.viewer = self.state.viewer.saturating_add(1);
        self.state.mode = Mode::Controlling;
        self.lease()
    }

    pub fn release(&mut self) -> ViewerLease {
        self.state.viewer = self.state.viewer.saturating_add(1);
        self.state.mode = Mode::Watching;
        self.lease()
    }

    pub fn detach(&mut self) {
        self.state.viewer = self.state.viewer.saturating_add(1);
        self.state.mode = Mode::Detached;
    }

    /// A reconnect always starts as a watcher. It must be explicitly followed
    /// by `take_control` before input is accepted.
    pub fn reconnect(&mut self, endpoint: &Endpoint) -> Result<ViewerLease, SessionError> {
        if endpoint != &self.state.endpoint {
            return Err(SessionError::EndpointMismatch);
        }
        self.state.connection = self.state.connection.saturating_add(1);
        self.state.viewer = self.state.viewer.saturating_add(1);
        self.state.mode = Mode::Watching;
        Ok(self.lease())
    }

    /// A replacement program keeps the holder but every old viewer lease and
    /// any pending input generation is dead.
    pub fn note_restarted(&mut self) -> ViewerLease {
        self.state.connection = self.state.connection.saturating_add(1);
        self.state.viewer = self.state.viewer.saturating_add(1);
        self.state.program = self.state.program.saturating_add(1);
        self.state.mode = Mode::Watching;
        self.lease()
    }

    pub fn input(&self, lease: ViewerLease, bytes: &[u8]) -> Result<Vec<u8>, SessionError> {
        self.check(lease)?;
        if self.state.mode != Mode::Controlling {
            return Err(SessionError::NotControlling);
        }
        Ok(bytes.to_vec())
    }

    pub fn resize(
        &self,
        lease: ViewerLease,
        cols: u16,
        rows: u16,
    ) -> Result<(u16, u16), SessionError> {
        self.check(lease)?;
        if self.state.mode != Mode::Controlling {
            return Err(SessionError::NotControlling);
        }
        Ok((cols, rows))
    }

    fn check(&self, lease: ViewerLease) -> Result<(), SessionError> {
        if lease.connection == self.state.connection
            && lease.viewer == self.state.viewer
            && lease.program == self.state.program
        {
            Ok(())
        } else {
            Err(SessionError::StaleViewer)
        }
    }

    fn lease(&self) -> ViewerLease {
        ViewerLease {
            connection: self.state.connection,
            viewer: self.state.viewer,
            program: self.state.program,
        }
    }
}

fn seq(params: &Value) -> Result<u64, Error> {
    params
        .get("seq")
        .and_then(Value::as_u64)
        .ok_or_else(|| Error::Protocol("catalogue message has no seq".into()))
}

fn required_terminal(terminal: Option<&str>) -> Result<&str, Error> {
    terminal.ok_or_else(|| Error::Protocol("event has no terminal".into()))
}

fn merge(record: &mut Value, data: &Value) {
    if let (Some(record), Some(data)) = (record.as_object_mut(), data.as_object()) {
        for (key, value) in data {
            record.insert(key.clone(), value.clone());
        }
    }
}

fn copy(record: &mut Value, data: &Value, key: &str) {
    if let Some(value) = data.get(key) {
        record[key] = value.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn terminal(address: &str, state: &str) -> Value {
        json!({
            "address": address,
            "name": null,
            "state": state,
            "status": null,
            "records": {},
            "reported": false,
            "labels": {},
            "cols": 80,
            "rows": 24,
            "created_ms": 1,
            "screen": "exact",
            "argv": ["sh"],
            "cwd": "/"
        })
    }

    fn event(seq: u64, kind: &str, terminal: Option<&str>, data: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "method": "event",
            "params": {
                "seq": seq,
                "type": kind,
                "terminal": terminal,
                "data": data
            }
        })
    }

    #[test]
    fn a_missed_snapshot_replaces_the_catalogue_and_ignores_stale_events() {
        let mut catalogue = Catalogue::new(10, vec![terminal("t1", "live")]);
        let missed = json!({
            "jsonrpc": "2.0",
            "method": "missed",
            "params": {"seq": 20, "terminals": [terminal("t2", "live")]}
        });
        assert_eq!(catalogue.apply(&missed).unwrap(), vec![Change::Snapshot]);
        assert!(catalogue.terminal("t1").is_none());
        assert_eq!(catalogue.terminal("t2").unwrap()["state"], "live");

        let stale = event(15, "title", Some("t2"), json!({"title": "old"}));
        assert!(catalogue.apply(&stale).unwrap().is_empty());
        assert!(catalogue.terminal("t2").unwrap().get("title").is_none());
    }

    #[test]
    fn restart_advances_the_program_generation_and_finish_clears_status() {
        let mut catalogue = Catalogue::new(1, vec![terminal("t1", "live")]);
        assert_eq!(catalogue.program_generation("t1"), 1);
        let restarted = event(2, "restarted", Some("t1"), terminal("t1", "live"));
        catalogue.apply(&restarted).unwrap();
        assert_eq!(catalogue.program_generation("t1"), 2);

        let finished = event(
            3,
            "finished",
            Some("t1"),
            json!({"reason": "exited", "exit_code": 0, "signal": null, "finished_ms": 9}),
        );
        catalogue.apply(&finished).unwrap();
        let record = catalogue.terminal("t1").unwrap();
        assert_eq!(record["state"], "finished");
        assert_eq!(record["exit_code"], 0);
        assert!(record["status"].is_null());
    }

    #[test]
    fn created_status_size_and_removed_update_one_record() {
        let mut catalogue = Catalogue::new(0, Vec::new());
        catalogue
            .apply(&event(1, "created", Some("t1"), terminal("t1", "live")))
            .unwrap();
        catalogue
            .apply(&event(
                2,
                "status",
                Some("t1"),
                json!({"status": {"state": "working"}, "records": {"r": {"state": "idle"}}, "reported": true}),
            ))
            .unwrap();
        catalogue
            .apply(&event(
                3,
                "size",
                Some("t1"),
                json!({"cols": 100, "rows": 40}),
            ))
            .unwrap();
        let record = catalogue.terminal("t1").unwrap();
        assert_eq!(record["status"]["state"], "working");
        assert_eq!(record["cols"], 100);
        assert_eq!(record["rows"], 40);
        catalogue
            .apply(&event(4, "removed", Some("t1"), json!({})))
            .unwrap();
        assert!(catalogue.terminal("t1").is_none());
    }

    #[test]
    fn reconnect_invalidates_input_until_explicit_control() {
        let endpoint = Endpoint {
            core_id: "core-1".into(),
            socket: PathBuf::from("/run/user/1000/hc.sock"),
        };
        let mut session = Session::new(endpoint.clone(), "t1");
        let lease = session.take_control();
        assert_eq!(session.input(lease, b"ls\n"), Ok(b"ls\n".to_vec()));
        session.reconnect(&endpoint).unwrap();
        assert_eq!(session.mode(), Mode::Watching);
        assert_eq!(
            session.input(lease, b"old\n"),
            Err(SessionError::StaleViewer)
        );
        let watch = session.watch();
        assert_eq!(
            session.input(watch, b"old\n"),
            Err(SessionError::NotControlling)
        );
        let control = session.take_control();
        assert_eq!(session.input(control, b"new\n"), Ok(b"new\n".to_vec()));
    }

    #[test]
    fn endpoint_mismatch_and_restart_reject_old_leases() {
        let endpoint = Endpoint {
            core_id: "core-1".into(),
            socket: PathBuf::from("/run/user/1000/hc.sock"),
        };
        let wrong = Endpoint {
            core_id: "core-2".into(),
            socket: PathBuf::from("/run/user/1000/other.sock"),
        };
        let mut session = Session::new(endpoint, "t1");
        let lease = session.take_control();
        assert_eq!(
            session.reconnect(&wrong),
            Err(SessionError::EndpointMismatch)
        );
        assert_eq!(session.input(lease, b"still\n"), Ok(b"still\n".to_vec()));
        session.note_restarted();
        assert_eq!(
            session.input(lease, b"old\n"),
            Err(SessionError::StaleViewer)
        );
    }
}
