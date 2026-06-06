// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Future push/pull encoder API surface.
//!
//! All encoding operations currently return [`splot_core::Error::Unimplemented`].

use splot_core::{Error, Result};

use crate::config::EncoderConfig;

/// An input video frame (stub).
// TODO(spec: ENC-Y4M-INPUT): model plane data, stride, and color format.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Frame {}

/// An output coded packet.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Packet {
    /// Coded bytes for one access unit.
    pub data: Vec<u8>,
}

/// Status of the encoder's push/pull state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderStatus {
    /// The encoder needs more frames before it can emit a packet.
    NeedMoreData,
    /// The encoder has buffered as many frames as it will accept right now.
    EnoughData,
    /// A configured limit (for example, a frame count) was reached.
    LimitReached,
}

/// The encoder context. Holds configuration and runtime parameters.
#[derive(Debug)]
pub struct Context {
    config: EncoderConfig,
    threads: usize,
}

impl Context {
    /// Creates an encoder context from a configuration and a worker-thread count.
    ///
    /// # Errors
    /// Currently always succeeds; the encoding operations are unimplemented.
    pub fn new(config: EncoderConfig, threads: usize) -> Result<Self> {
        Ok(Self { config, threads })
    }

    /// Returns the bitstream-affecting configuration.
    #[must_use]
    pub fn config(&self) -> &EncoderConfig {
        &self.config
    }

    /// Returns the configured worker-thread count.
    #[must_use]
    pub fn threads(&self) -> usize {
        self.threads
    }

    /// Submits a frame to the encoder.
    ///
    /// # Errors
    /// Always returns [`Error::Unimplemented`].
    pub fn send_frame(&mut self, frame: Frame) -> Result<()> {
        let _ = frame;
        Err(Error::Unimplemented {
            feature: "AV2 encoder",
        })
    }

    /// Retrieves the next coded packet.
    ///
    /// # Errors
    /// Always returns [`Error::Unimplemented`].
    pub fn receive_packet(&mut self) -> Result<Packet> {
        Err(Error::Unimplemented {
            feature: "AV2 encoder",
        })
    }

    /// Flushes the encoder, signalling end of input.
    ///
    /// # Errors
    /// Always returns [`Error::Unimplemented`].
    pub fn flush(&mut self) -> Result<()> {
        Err(Error::Unimplemented {
            feature: "AV2 encoder",
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn context_exposes_config_and_threads() {
        let ctx = Context::new(EncoderConfig::new(1920, 1080), 4).unwrap();
        assert_eq!(ctx.config().width, 1920);
        assert_eq!(ctx.config().height, 1080);
        assert_eq!(ctx.threads(), 4);
    }

    #[test]
    fn encoding_operations_are_unimplemented() {
        let mut ctx = Context::new(EncoderConfig::default(), 1).unwrap();
        assert!(matches!(
            ctx.send_frame(Frame::default()),
            Err(Error::Unimplemented { .. })
        ));
        assert!(matches!(
            ctx.receive_packet(),
            Err(Error::Unimplemented { .. })
        ));
        assert!(matches!(ctx.flush(), Err(Error::Unimplemented { .. })));
    }
}
