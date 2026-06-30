// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A decode-driver context that owns a worker pool plus byte-consuming and
//! parsed-stream planning entry points.

use core::num::NonZeroUsize;

use splot_parallel::{ThreadCount, WorkerPool};

use crate::DecodeHashReport;
use crate::DecodeOptions;
use crate::byte_stream::plan_byte_stream;
use crate::error::{DecodeOutputError, DecodeOutputOperation, Result};
use crate::runtime::DecodeRuntimeConfig;
use crate::stream_plan::{DecodeStreamInput, DecodeStreamPlan, plan_stream};
use crate::tile_payload::{
    DecodeTilePayloadPlan, FrameCandidateTileBoundaryError, FrameCandidateTileBoundaryInput,
    TilePayloadBoundaryError, TilePayloadBoundaryInput, plan_derived_tile_payload_boundary,
    plan_tile_payload_boundary,
};

/// A decode context.
///
/// Owns exactly one [`WorkerPool`], plans bounded stream metadata from raw Annex
/// B/IVF or parsed streams, and exposes the narrow `minimal-intra-8bit420-hash-v1`
/// runtime hash, raw, and Y4M byte paths. It does not touch the filesystem,
/// invoke any external decoder, or claim broad AV2 runtime decode support.
#[derive(Debug)]
pub struct DecodeContext {
    runtime: DecodeRuntimeConfig,
    pool: WorkerPool,
}

impl DecodeContext {
    /// Creates a decode context and its single owned worker pool.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError::Pool`] if the worker pool cannot be built.
    pub fn new(runtime: DecodeRuntimeConfig) -> Result<Self> {
        let pool = WorkerPool::new(runtime.thread_count)?;
        Ok(Self { runtime, pool })
    }

    /// The runtime (non-bitstream) configuration.
    #[must_use]
    pub fn runtime(&self) -> &DecodeRuntimeConfig {
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

    /// Builds a deterministic plan over raw AV2 Annex B or IVF bytes.
    ///
    /// Runs inside the context-owned worker pool. Plan-only: bounds byte
    /// traversal, decodes no tile payloads, reconstructs no pixels, writes no
    /// output, and invokes no external decoder.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for malformed sources, unsupported
    /// structures, local decode resource-limit failures, or pool failures.
    pub fn plan_bytes(&self, bytes: &[u8], options: DecodeOptions) -> Result<DecodeStreamPlan> {
        self.pool.install(|| plan_byte_stream(bytes, &options))
    }

    /// Decodes the documented minimal tier and returns a deterministic hash report.
    ///
    /// Runs [`Self::plan_bytes`] first so malformed sources, resource-limit
    /// failures, layer selection, and planner-level unsupported structures stay
    /// transactional. Limited to the `minimal-intra-8bit420-hash-v1` tier.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for malformed sources, unsupported
    /// structures, runtime-tier rejections, resource-limit failures, worker-pool
    /// failures, or reconstruction model errors.
    pub fn decode_hash_report_bytes(
        &self,
        bytes: &[u8],
        options: DecodeOptions,
    ) -> Result<DecodeHashReport> {
        let plan = self.plan_bytes(bytes, options)?;
        self.pool.install(|| {
            crate::runtime_hash::decode_hash_report_from_plan(
                bytes,
                &options,
                &plan,
                self.threads(),
            )
        })
    }

    /// Decodes the documented minimal tier and writes complete raw sample bytes.
    ///
    /// Runs [`Self::plan_bytes`] first (see [`Self::decode_hash_report_bytes`]),
    /// limited to the same `minimal-intra-8bit420-hash-v1` IVF tier. The complete
    /// raw byte stream is buffered and checked against
    /// [`crate::DecodeLimitName::MaxOutputBytes`] before any bytes reach `writer`.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for malformed sources, unsupported
    /// structures, runtime-tier rejections, resource-limit failures, worker-pool
    /// failures, reconstruction model errors, raw serialization errors, or
    /// caller-writer I/O errors.
    pub fn decode_raw_bytes<W: std::io::Write>(
        &self,
        bytes: &[u8],
        options: DecodeOptions,
        mut writer: W,
    ) -> Result<()> {
        let plan = self.plan_bytes(bytes, options)?;
        let raw = self
            .pool
            .install(|| crate::runtime_raw::encode_raw_stream_from_plan(bytes, &options, &plan))?;
        std::io::Write::write_all(&mut writer, &raw).map_err(|source| {
            DecodeOutputError::io(DecodeOutputOperation::WriteRawStream, source)
        })?;
        Ok(())
    }

    /// Decodes the documented minimal tier and writes a complete Y4M stream.
    ///
    /// Runs [`Self::plan_bytes`] first (see [`Self::decode_hash_report_bytes`]),
    /// limited to the same `minimal-intra-8bit420-hash-v1` IVF tier. The complete
    /// Y4M stream is buffered and checked against
    /// [`crate::DecodeLimitName::MaxOutputBytes`] before any bytes reach `writer`.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for malformed sources, unsupported
    /// structures, runtime-tier rejections, resource-limit failures, worker-pool
    /// failures, reconstruction model errors, Y4M serialization errors, or
    /// caller-writer I/O errors.
    pub fn decode_y4m_bytes<W: std::io::Write>(
        &self,
        bytes: &[u8],
        options: DecodeOptions,
        mut writer: W,
    ) -> Result<()> {
        let plan = self.plan_bytes(bytes, options)?;
        let y4m = self
            .pool
            .install(|| crate::runtime_y4m::encode_y4m_stream_from_plan(bytes, &options, &plan))?;
        std::io::Write::write_all(&mut writer, &y4m).map_err(|source| {
            DecodeOutputError::io(DecodeOutputOperation::WriteY4mStream, source)
        })?;
        Ok(())
    }

    /// Builds a deterministic plan over an already parsed AV2 stream.
    ///
    /// Runs inside the context-owned worker pool. Serial and plan-only: consumes
    /// no raw bytes, decodes no tile payloads, reconstructs no pixels, and
    /// invokes no external decoder.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for malformed parsed sources,
    /// unsupported structures, or local decode resource-limit failures.
    pub fn plan_stream(
        &self,
        input: DecodeStreamInput<'_>,
        options: DecodeOptions,
    ) -> Result<DecodeStreamPlan> {
        self.pool.install(|| plan_stream(input, &options))
    }

    /// Builds a deterministic tile-payload boundary plan inside this context's
    /// worker pool.
    ///
    /// Plan-only: does not run `decode_tile()`, reconstruct pixels, update
    /// references, write output, or invoke external decoders.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "crate-private tile handoff is tested before runtime decode derives tile facts"
        )
    )]
    pub(crate) fn plan_tile_payload_boundary<'a>(
        &self,
        input: &TilePayloadBoundaryInput<'a, '_>,
    ) -> core::result::Result<DecodeTilePayloadPlan<'a>, TilePayloadBoundaryError> {
        self.pool.install(|| plan_tile_payload_boundary(input))
    }

    /// Derives and plans a deterministic tile-payload boundary inside this
    /// context's worker pool.
    ///
    /// Plan-only: validates source-backed parser facts before slicing the § 5.20
    /// payload region, then stops at the unsupported `decode_tile()` boundary
    /// without reconstructing pixels, updating references, writing output, or
    /// invoking external decoders.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "crate-private derived tile handoff is tested before runtime decode wires it"
        )
    )]
    pub(crate) fn plan_derived_tile_payload_boundary<'a>(
        &self,
        input: &FrameCandidateTileBoundaryInput<'a, '_>,
    ) -> core::result::Result<DecodeTilePayloadPlan<'a>, FrameCandidateTileBoundaryError> {
        self.pool
            .install(|| plan_derived_tile_payload_boundary(input))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn default_runtime_config_is_auto() {
        assert_eq!(
            DecodeRuntimeConfig::default().thread_count,
            ThreadCount::Auto
        );
    }

    #[test]
    fn context_resolves_fixed_thread_count() {
        let ctx = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(4usize))).unwrap();
        assert_eq!(ctx.threads().get(), 4);
    }

    #[test]
    fn requested_threads_round_trips() {
        let ctx = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(4usize))).unwrap();
        assert_eq!(ctx.requested_threads(), ThreadCount::from(4usize));
    }

    #[test]
    fn zero_threads_maps_to_auto() {
        let ctx = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(0usize))).unwrap();
        assert!(ctx.threads().get() >= 1);
    }
}
