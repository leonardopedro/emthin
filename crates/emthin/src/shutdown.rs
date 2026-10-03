//! Graceful shutdown on SIGTERM / SIGINT.
//!
//! # Why this is a flag and not a self-pipe
//!
//! The first version signalled the loop through a self-pipe: the handler wrote one
//! byte and a calloop `Generic` source on the read end called `LoopSignal::stop`.
//! It is the shape everyone reaches for, and it is what upstream guidance
//! recommends, and **it did not work here** — reproducibly, and only here.
//!
//! What is established, by instrumenting the handler so it could be observed from
//! outside the process (`EMTHIN_SIGNAL_TRACE`, which writes to a regular file
//! because `write(2)` is async-signal-safe):
//!
//! * the handler runs — a byte lands in the trace file;
//! * the pipe write *succeeds* — the trace records a `1` verdict and errno 0;
//! * the `Generic` source registers without complaint;
//! * and the callback never runs, so `stop()` is never called and the loop never
//!   exits.
//!
//! Since the byte is in the pipe and the fd is registered, the only remaining
//! explanation is in the loop's dispatch, and this is not the place to keep digging
//! for it: emthin has its own event loop with sources it must not disturb, and a
//! shutdown path that depends on one more moving part inside it is a worse trade
//! than one that does not.
//!
//! So: the handler does the only thing it can safely do — an atomic store — and
//! the loop checks it on every iteration, which it already does at least sixty
//! times a second because winit delivers frame callbacks. No pipe, no fd, no
//! epoll registration, nothing to go wrong at 3am.
//!
//! The cost is honest and worth stating: if the loop is completely idle, with no
//! frame callback pending and no input, the request is noticed on the next event
//! rather than immediately. A compositor that renders on demand can sit idle; the
//! shutdown then waits for the next thing that happens. That is a delay, not a
//! hang, and it is the right trade against a mechanism that does not work at all.
//!
//! `request()` is called from a signal handler, so it must stay async-signal-safe:
//! a relaxed atomic store and nothing else. No allocation, no locks, no logging.

use std::sync::atomic::{AtomicBool, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);

/// Ask the compositor to stop. Async-signal-safe; callable from a handler.
#[inline]
pub fn request() {
    REQUESTED.store(true, Ordering::SeqCst);
}

/// Has a shutdown been requested? Checked once per event-loop iteration.
#[inline]
pub fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}

/// Forget a previous request. Only for tests.
pub fn reset() {
    REQUESTED.store(false, Ordering::SeqCst);
}

/// Install handlers for SIGTERM and SIGINT.
///
/// Returns `false` if the dispositions could not be installed, which leaves the
/// process with the default behaviour: SIGTERM kills it outright, so the session
/// is not saved. Callers log that rather than pretending.
pub fn install() -> bool {
    extern "C" fn on_signal(_sig: libc::c_int) {
        request();
    }
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_signal as *const () as usize;
        libc::sigemptyset(&mut sa.sa_mask);
        // Deliberately no SA_RESTART: a handler that does not return is not the
        // point, but leaving syscalls restartable would hide EINTR from the loop
        // for reasons unrelated to shutdown.
        sa.sa_flags = 0;
        if libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut()) != 0 {
            return false;
        }
        if libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut()) != 0 {
            return false;
        }
    }
    tracing::info!("SIGTERM and SIGINT will stop the loop and save the session");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_visible_and_clearable() {
        reset();
        assert!(!requested(), "a fresh process has not been asked to stop");
        request();
        assert!(requested(), "request() must be visible to the loop");
        reset();
        assert!(!requested(), "reset() is for tests only, and must work");
    }

    #[test]
    fn the_handler_is_installed() {
        // The disposition has to be observable, because the previous mechanism's
        // whole failure was invisible: the handler ran, the pipe write succeeded,
        // and nothing said so.
        assert!(install(), "sigaction for SIGTERM and SIGINT must succeed");
        // A real signal, to the real handler: this is the test that would have
        // caught it if the handler were not installed.
        assert_eq!(
            unsafe { libc::raise(libc::SIGTERM) },
            0,
            "raising SIGTERM must succeed"
        );
        assert!(
            requested(),
            "a raised SIGTERM must set the flag the loop polls"
        );
        reset();
    }
}
