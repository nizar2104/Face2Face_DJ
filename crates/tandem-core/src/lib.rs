//! Tandem core. Everything that is not UI lives here, so it can be tested
//! without a webview and reused by command-line tools.
//!
//! Milestone 0 contents: the wire protocol, a mock deck source, synthetic
//! waveforms, and the Sender side of the transport (localhost only).

pub mod net;
pub mod protocol;
pub mod sender;
pub mod session;
pub mod sources;
pub mod waveform;

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Locks a mutex, carrying on with the data if another thread panicked while
/// holding it. A DJ tool should keep running rather than cascade a panic.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
