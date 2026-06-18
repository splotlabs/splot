// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Future push/pull encoder API surface.
//!
//! All encoding operations currently return [`splot_core::Error::Unimplemented`].

use core::num::NonZeroUsize;

use splot_core::{Error as CoreError, Result as CoreResult};
use splot_parallel::{ThreadCount, WorkerPool};

use crate::config::EncoderConfig;
use crate::error::Result;
use crate::frame::Frame;
use crate::runtime::EncoderRuntimeConfig;

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
    runtime: EncoderRuntimeConfig,
    pool: WorkerPool,
}

impl Context {
    /// Creates an encoder context from a bitstream configuration and a runtime
    /// configuration, building the context's single owned [`WorkerPool`].
    ///
    /// # Errors
    /// Returns [`crate::Error::Pool`] if the worker pool cannot be constructed.
    pub fn new(config: EncoderConfig, runtime: EncoderRuntimeConfig) -> Result<Self> {
        let pool = WorkerPool::new(runtime.thread_count)?;
        Ok(Self {
            config,
            runtime,
            pool,
        })
    }

    /// Returns the bitstream-affecting configuration.
    #[must_use]
    pub fn config(&self) -> &EncoderConfig {
        &self.config
    }

    /// The runtime (non-bitstream) configuration.
    #[must_use]
    pub fn runtime(&self) -> &EncoderRuntimeConfig {
        &self.runtime
    }

    /// The originally requested (unresolved) thread-count policy.
    #[must_use]
    pub fn requested_threads(&self) -> ThreadCount {
        self.runtime.thread_count
    }

    /// The resolved, non-zero worker-thread count.
    #[must_use]
    pub fn threads(&self) -> NonZeroUsize {
        self.pool.threads()
    }

    /// The context's single owned worker pool.
    #[must_use]
    pub fn pool(&self) -> &WorkerPool {
        &self.pool
    }

    /// Submits a frame to the encoder.
    ///
    /// # Errors
    /// Always returns [`CoreError::Unimplemented`].
    pub fn send_frame(&mut self, frame: Frame<'_>) -> CoreResult<()> {
        let _ = frame;
        Err(CoreError::Unimplemented {
            feature: "AV2 encoder",
        })
    }

    /// Retrieves the next coded packet.
    ///
    /// # Errors
    /// Always returns [`CoreError::Unimplemented`].
    pub fn receive_packet(&mut self) -> CoreResult<Packet> {
        Err(CoreError::Unimplemented {
            feature: "AV2 encoder",
        })
    }

    /// Flushes the encoder, signalling end of input.
    ///
    /// # Errors
    /// Always returns [`CoreError::Unimplemented`].
    pub fn flush(&mut self) -> CoreResult<()> {
        Err(CoreError::Unimplemented {
            feature: "AV2 encoder",
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use splot_core::Error as CoreError;
    use splot_parallel::ThreadCount;
    use splot_recon::{PlaneRect, PlaneSize};

    fn size(width: usize, height: usize) -> PlaneSize {
        PlaneSize::new(width, height).unwrap()
    }

    fn rect(width: usize, height: usize) -> PlaneRect {
        PlaneRect::new(0, 0, width, height).unwrap()
    }

    fn frame<'a>(y: &'a [u8], u: &'a [u8], v: &'a [u8]) -> Frame<'a> {
        crate::frame::Frame::from_planes(
            crate::frame::FrameInfo::yuv420_8bit(crate::frame::FrameId::new(0), size(2, 2)),
            crate::frame::FramePlanesInput::yuv(
                crate::frame::FramePlaneInput::new(y, 2, rect(2, 2)),
                crate::frame::FramePlaneInput::new(u, 1, rect(1, 1)),
                crate::frame::FramePlaneInput::new(v, 1, rect(1, 1)),
            ),
        )
        .unwrap()
    }

    #[test]
    fn context_exposes_config_and_threads() {
        let runtime = EncoderRuntimeConfig::new(ThreadCount::from(4usize));
        let ctx = Context::new(EncoderConfig::new(1920, 1080), runtime).unwrap();
        assert_eq!(ctx.config().width, 1920);
        assert_eq!(ctx.config().height, 1080);
        assert_eq!(ctx.threads().get(), 4);
        assert_eq!(ctx.requested_threads(), ThreadCount::from(4usize));
    }

    #[test]
    fn encoding_operations_are_unimplemented() {
        let mut ctx =
            Context::new(EncoderConfig::default(), EncoderRuntimeConfig::default()).unwrap();
        let y = [0_u8; 4];
        let u = [0_u8; 1];
        let v = [0_u8; 1];
        assert!(matches!(
            ctx.send_frame(frame(&y, &u, &v)),
            Err(CoreError::Unimplemented { .. })
        ));
        assert!(matches!(
            ctx.receive_packet(),
            Err(CoreError::Unimplemented { .. })
        ));
        assert!(matches!(ctx.flush(), Err(CoreError::Unimplemented { .. })));
    }

    #[test]
    fn default_runtime_config_is_auto() {
        assert_eq!(
            EncoderRuntimeConfig::default().thread_count,
            ThreadCount::Auto
        );
    }

    #[test]
    fn zero_threads_maps_to_auto() {
        let runtime = EncoderRuntimeConfig::new(ThreadCount::from(0usize));
        assert_eq!(runtime.thread_count, ThreadCount::Auto);
        let ctx = Context::new(EncoderConfig::default(), runtime).unwrap();
        assert!(ctx.threads().get() >= 1);
    }

    #[test]
    fn thread_count_does_not_change_bitstream_config() {
        let a = Context::new(
            EncoderConfig::new(640, 480),
            EncoderRuntimeConfig::new(ThreadCount::from(1usize)),
        )
        .unwrap();
        let b = Context::new(
            EncoderConfig::new(640, 480),
            EncoderRuntimeConfig::new(ThreadCount::from(4usize)),
        )
        .unwrap();
        assert_eq!(a.config(), b.config());
    }

    #[test]
    fn context_pool_runs_parallel_iterators_via_prelude_deterministically() {
        // splot-encode has no direct `rayon` dependency: parallel iteration is
        // reached only through `splot_parallel::prelude`, driven on the context's
        // own `WorkerPool` (not Rayon's global pool). Indexed map + ordered collect
        // is deterministic regardless of worker count.
        use splot_parallel::prelude::*;

        let ctx = Context::new(
            EncoderConfig::default(),
            EncoderRuntimeConfig::new(ThreadCount::from(4usize)),
        )
        .unwrap();
        let doubled: Vec<u64> = ctx
            .pool()
            .install(|| (0..16u64).into_par_iter().map(|x| x * 2).collect());
        assert_eq!(doubled, (0..16u64).map(|x| x * 2).collect::<Vec<_>>());
    }
}
