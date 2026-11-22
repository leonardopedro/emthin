//! Unix-socket adapter for local Herdr Core endpoints.
//!
//! This is deliberately thin: the socket owns bytes and ordering, while
//! [`Client`] owns JSON-RPC correlation, handshakes, streams, and credit.
//! Relative paths and parent traversal are rejected; the caller still owns
//! the endpoint's provenance and permissions.

use std::collections::VecDeque;
use std::fmt;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Component, Path};

use crate::wire::{self, Frame, FrameDecoder};
use crate::{Client, Error, Outcome};

#[derive(Debug)]
pub enum TransportError {
    Io(io::Error),
    Frame(wire::FrameError),
    Protocol(Error),
    Truncated,
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "socket: {error}"),
            Self::Frame(error) => write!(f, "frame: {error}"),
            Self::Protocol(error) => write!(f, "{error}"),
            Self::Truncated => write!(f, "socket closed in the middle of a frame"),
        }
    }
}

impl std::error::Error for TransportError {}

impl From<io::Error> for TransportError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<wire::FrameError> for TransportError {
    fn from(error: wire::FrameError) -> Self {
        Self::Frame(error)
    }
}

impl From<Error> for TransportError {
    fn from(error: Error) -> Self {
        Self::Protocol(error)
    }
}

pub struct Connection {
    stream: UnixStream,
    decoder: FrameDecoder,
    incoming: VecDeque<Frame>,
    eof: bool,
}

impl Connection {
    /// Connect to an absolute local socket path.
    pub fn connect(path: impl AsRef<Path>) -> Result<Self, TransportError> {
        let path = path.as_ref();
        validate_local_path(path)?;
        Self::from_stream(UnixStream::connect(path)?)
    }

    pub fn from_stream(stream: UnixStream) -> Result<Self, TransportError> {
        Ok(Self {
            stream,
            decoder: FrameDecoder::default(),
            incoming: VecDeque::new(),
            eof: false,
        })
    }

    pub fn set_read_timeout(&self, timeout: Option<std::time::Duration>) -> io::Result<()> {
        self.stream.set_read_timeout(timeout)
    }

    pub fn set_write_timeout(&self, timeout: Option<std::time::Duration>) -> io::Result<()> {
        self.stream.set_write_timeout(timeout)
    }

    /// Switch the socket to nonblocking reads. Callers that register the
    /// stream with an event loop must treat `WouldBlock` as "no frame yet".
    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        self.stream.set_nonblocking(nonblocking)
    }

    /// A duplicate handle for event-loop registration. Both handles refer to
    /// the same socket; only one should read at a time.
    pub fn try_clone_stream(&self) -> io::Result<UnixStream> {
        self.stream.try_clone()
    }

    pub fn send(&mut self, frame: Frame) -> Result<(), TransportError> {
        let bytes = wire::encode(frame)?;
        self.stream.write_all(&bytes)?;
        Ok(())
    }

    /// Read one frame. `Ok(None)` is a clean peer close; a partial frame or
    /// protocol error is returned as an error, never hidden as EOF.
    pub fn receive(&mut self) -> Result<Option<Frame>, TransportError> {
        if let Some(frame) = self.incoming.pop_front() {
            return Ok(Some(frame));
        }
        if self.eof {
            return Ok(None);
        }
        let mut buffer = [0u8; 8192];
        loop {
            let read = self.stream.read(&mut buffer)?;
            if read == 0 {
                self.eof = true;
                self.decoder
                    .finish()
                    .map_err(|_| TransportError::Truncated)?;
                return Ok(self.incoming.pop_front());
            }
            self.incoming.extend(self.decoder.push(&buffer[..read])?);
            if let Some(frame) = self.incoming.pop_front() {
                return Ok(Some(frame));
            }
        }
    }

    /// Receive one frame, apply it to the client, and write every frame the
    /// client owes (notably CREDIT). `Ok(None)` means the peer closed cleanly.
    pub fn pump(&mut self, client: &mut Client) -> Result<Option<Outcome>, TransportError> {
        let Some(frame) = self.receive()? else {
            return Ok(None);
        };
        let outcome = client.handle(frame)?;
        for frame in &outcome.outgoing {
            self.send(frame.clone())?;
        }
        Ok(Some(outcome))
    }
}

fn validate_local_path(path: &Path) -> Result<(), TransportError> {
    if !path.is_absolute() {
        return Err(TransportError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Core socket path must be absolute: {}", path.display()),
        )));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(TransportError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Core socket path may not contain ..: {}", path.display()),
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partial_frame_at_eof_is_an_error_not_a_clean_close() {
        let (mut peer, stream) = UnixStream::pair().expect("socket pair");
        let mut connection = Connection::from_stream(stream).expect("connection");
        peer.write_all(&[1, 0, 0, 0, wire::DATA, 5, 0, 0])
            .expect("partial header");
        peer.shutdown(std::net::Shutdown::Write).expect("shutdown");
        assert!(matches!(
            connection.receive(),
            Err(TransportError::Truncated)
        ));
    }

    #[test]
    fn relative_and_parent_paths_are_rejected_before_connect() {
        assert!(matches!(
            Connection::connect("hc.sock"),
            Err(TransportError::Io(error)) if error.kind() == io::ErrorKind::InvalidInput
        ));
        assert!(matches!(
            Connection::connect("/tmp/../run/hc.sock"),
            Err(TransportError::Io(error)) if error.kind() == io::ErrorKind::InvalidInput
        ));
    }
}
