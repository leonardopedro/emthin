//! Fixture CLI for K7: replay a metadata-only NDJSON session trace and print
//! the settled state. No compositor, display or control client is needed.
//!
//! ```sh
//! cargo run -p emthin --bin inspect-replay < trace.ndjson
//! ```
//!
//! An unparseable line is a hard error with its line number, so a fixture
//! cannot silently settle on the wrong state.

use std::io::{self, Read};

fn main() {
    let mut input = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut input) {
        eprintln!("could not read trace: {error}");
        std::process::exit(2);
    }
    match emthin::docui::inspect::replay(&input) {
        Ok(state) => {
            println!(
                "{}",
                serde_json::json!({
                    "local": state.local,
                    "share": state.share,
                    "frontier": state.frontier,
                    "accepted": state.accepted,
                    "peers": state.peers,
                    "reason": state.reason,
                })
            );
        }
        Err(error) => {
            eprintln!("replay failed: {error}");
            std::process::exit(1);
        }
    }
}
