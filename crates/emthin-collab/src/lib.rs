//! Collaboration primitives for emthin.
//!
//! This crate is deliberately transport-agnostic and dependency-free. It
//! adapts the useful part of Kyneta's `Line`: a reliable, ordered command
//! stream with sequence numbers, cumulative acknowledgements, replay of
//! unacknowledged commands, and duplicate suppression.
//!
//! It is not a port of Kyneta's React, transport, schema, or store layers.
//! Document convergence and presence live in `mathed_core::sync`; this crate
//! only supplies the command channel a host can carry over a Unix socket,
//! WebSocket, or an AustralVM boundary.
//!
//! [`Session`] is the multi-peer coordinator: one [`Line`] per peer and one
//! presence store, over a document the caller owns. It returns [`Outbound`]
//! frames instead of writing them. [`wire`] and [`unix`] are an optional,
//! std-only Unix-socket implementation of that boundary; a host that already
//! has a socket loop can use [`Session`] directly and ignore them.

pub mod anchor;
mod exchange;
pub mod grants;
mod line;
mod session;
#[cfg(unix)]
pub mod unix;
pub mod wire;

pub use exchange::Exchange;
pub use line::{Delivery, Line, LineError, Seq};
pub use session::{Outbound, ReceiveOutcome, Session, SessionError};
