// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Cooperative cancellation of a running encode.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{Error, Result};

/// Stops an [`Encoder`](crate::Encoder) from any thread; clones share one flag.
///
/// [`Encoder::cancel_token`](crate::Encoder::cancel_token) returns the
/// encoder's token. After [`Self::cancel`], a running
/// [`Encoder::send_frame`](crate::Encoder::send_frame) or
/// [`Encoder::receive_packet`](crate::Encoder::receive_packet) stops at its
/// next check (per lookahead frame, superblock, searched block and filter
/// candidate) and returns [`Error::Cancelled`], and so does every later call.
/// Cancelling is final: the encoder codes nothing more and can only be dropped.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// Asks the encoder to stop. Returns at once; safe to call more than once.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Whether [`Self::cancel`] was called.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// [`Error::Cancelled`] once [`Self::cancel`] was called.
    pub(crate) fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            return Err(Error::Cancelled);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "cancel_tests.rs"]
mod tests;
