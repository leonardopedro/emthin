use crate::ipc::{IncomingMessage, OutgoingMessage};

/// Parse a JSON-RPC 2.0 notification payload into an `IncomingMessage`.
pub fn parse_incoming(payload: &[u8]) -> Result<IncomingMessage, String> {
    let v: serde_json::Value =
        serde_json::from_slice(payload).map_err(|e| format!("JSON parse error: {e}"))?;
    let jsonrpc = v.get("jsonrpc").and_then(|v| v.as_str()).unwrap_or("");
    if jsonrpc != "2.0" {
        return Err(format!("invalid jsonrpc version: {jsonrpc:?}"));
    }
    let method = v["method"]
        .as_str()
        .ok_or_else(|| "missing 'method' field".to_string())?;
    let params = v.get("params").unwrap_or(&serde_json::Value::Null);
    IncomingMessage::from_jsonrpc(method, params)
}

/// Serialize an `OutgoingMessage` as a JSON-RPC 2.0 notification.
pub fn serialize_outgoing(msg: OutgoingMessage) -> Result<Vec<u8>, serde_json::Error> {
    let method = msg.method_name();
    let params = msg.into_params_value();
    let envelope = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    });
    serde_json::to_vec(&envelope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::IpcRect;

    #[test]
    fn roundtrip_connected() {
        let msg = OutgoingMessage::Connected { version: "0.1" };
        let wire = serialize_outgoing(msg).unwrap();
        let wire_str = String::from_utf8_lossy(&wire);
        assert!(wire_str.contains(r#""jsonrpc":"2.0""#));
        assert!(wire_str.contains(r#""method":"connected""#));
        assert!(wire_str.contains(r#""version":"0.1""#));
    }

    #[test]
    fn roundtrip_figure_changed_carries_absolute_pixels() {
        let msg = OutgoingMessage::FigureChanged {
            figure: "f0".into(),
            page: 1,
            rect: IpcRect {
                x: 10,
                y: 20,
                w: 640,
                h: 400,
            },
            bound: true,
        };
        let wire = serialize_outgoing(msg).unwrap();
        let s = String::from_utf8_lossy(&wire);
        assert!(s.contains(r#""method":"figure_changed""#), "{s}");
        assert!(s.contains(r#""w":640"#), "{s}");
    }

    #[test]
    fn parse_spawn() {
        let wire = br#"{"jsonrpc":"2.0","method":"spawn","params":{"cmd":"foot","args":["-T"]}}"#;
        let msg = parse_incoming(wire).unwrap();
        assert!(matches!(
            msg,
            IncomingMessage::Spawn { ref cmd, ref args } if cmd == "foot" && args == &["-T".to_string()]
        ));
    }

    #[test]
    fn parse_close_by_figure_key() {
        let wire = br#"{"jsonrpc":"2.0","method":"close","params":{"figure":"f3"}}"#;
        let msg = parse_incoming(wire).unwrap();
        assert!(matches!(msg, IncomingMessage::Close { figure } if figure == "f3"));
    }

    #[test]
    fn parse_focus_without_a_figure_means_the_document() {
        let wire = br#"{"jsonrpc":"2.0","method":"focus","params":{}}"#;
        let msg = parse_incoming(wire).unwrap();
        assert!(matches!(msg, IncomingMessage::Focus { figure: None }));
    }

    #[test]
    fn rejects_missing_jsonrpc_field() {
        let wire = br#"{"method":"close","params":{"figure":"f0"}}"#;
        assert!(parse_incoming(wire).is_err());
    }

    #[test]
    fn rejects_unknown_method() {
        let wire = br#"{"jsonrpc":"2.0","method":"bogus","params":{}}"#;
        assert!(parse_incoming(wire).is_err());
    }

    #[test]
    fn rejects_a_malformed_frame() {
        assert!(parse_incoming(b"not json").is_err());
    }
}
