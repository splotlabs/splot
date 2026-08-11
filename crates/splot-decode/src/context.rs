// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A decode-driver context that owns a worker pool plus byte-consuming and
//! parsed-stream planning entry points.

use core::num::NonZeroUsize;

use splot_parallel::WorkerPool;

use crate::DecodeHashReport;
use crate::DecodeOptions;
use crate::bitstream::byte_stream::{PreparedByteStream, plan_byte_stream, prepare_byte_stream};
use crate::bitstream::stream_plan::{DecodeStreamInput, DecodeStreamPlan, plan_stream};
use crate::error::Result;
use crate::runtime::DecodeRuntimeConfig;

/// A decode context.
///
/// Owns exactly one [`WorkerPool`], plans bounded stream metadata from raw Annex
/// B/IVF or parsed streams, and exposes the runtime hash, raw, Y4M, and
/// discard-output paths for the supported decode envelope (tracked in
/// `docs/DECODER-SUPPORT-MATRIX.toml`). It does not touch the filesystem or
/// invoke any external decoder.
#[derive(Debug)]
pub struct DecodeContext {
    runtime: DecodeRuntimeConfig,
    pool: WorkerPool,
    frame_delay: NonZeroUsize,
}

impl DecodeContext {
    /// Creates a decode context and its single owned worker pool.
    ///
    /// The configured frame-pipelining depth is resolved once here against the
    /// pool's worker-thread count, so no decode path re-resolves it.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError::Pool`] if the worker pool cannot be built.
    pub fn new(runtime: DecodeRuntimeConfig) -> Result<Self> {
        let pool = WorkerPool::new(runtime.thread_count)?;
        let frame_delay = runtime.frame_delay.resolve(pool.threads());
        Ok(Self {
            runtime,
            pool,
            frame_delay,
        })
    }

    /// The resolved, non-zero frame-pipelining depth: how many frames may be in
    /// flight at once. One means serial decode.
    #[must_use]
    pub fn frame_delay(&self) -> NonZeroUsize {
        self.frame_delay
    }

    /// The runtime (non-bitstream) configuration.
    #[must_use]
    pub fn runtime(&self) -> &DecodeRuntimeConfig {
        &self.runtime
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

    fn prepare_bytes<'a>(
        &self,
        bytes: &'a [u8],
        options: &DecodeOptions,
    ) -> Result<PreparedByteStream<'a>> {
        let plan_started = crate::timing::start();
        let prepared = self.pool.install(|| prepare_byte_stream(bytes, options))?;
        crate::timing::report("plan", plan_started);
        Ok(prepared)
    }

    fn install_decode<F, R>(&self, decode: F) -> R
    where
        F: FnOnce() -> R + Send,
        R: Send,
    {
        let wait_metrics = self.pool.wait_metrics();
        let result = self.pool.install(decode);
        crate::timing::report_pool_wait(self.pool.wait_metrics().since(wait_metrics));
        result
    }

    fn decode_raw_with<'a>(
        &self,
        bytes: &'a [u8],
        options: DecodeOptions,
        decode: impl FnOnce(
            &'a [u8],
            &PreparedByteStream<'a>,
            &DecodeOptions,
            NonZeroUsize,
        ) -> Result<()>
        + Send,
    ) -> Result<()> {
        let prepared = self.prepare_bytes(bytes, &options)?;
        self.install_decode(|| decode(bytes, &prepared, &options, self.frame_delay))
    }

    /// Decodes the supported envelope and returns a deterministic hash report.
    ///
    /// Runs the same bounded byte planning as [`Self::plan_bytes`] first so
    /// malformed sources, resource-limit failures, layer selection, and
    /// planner-level unsupported structures stay transactional. The supported
    /// decode envelope is tracked in `docs/DECODER-SUPPORT-MATRIX.toml`.
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
        let prepared = self.prepare_bytes(bytes, &options)?;
        let runtime_started = crate::timing::start();
        let report = self.install_decode(|| {
            crate::output::hash::decode_hash_report_from_plan(
                bytes,
                prepared.parsed(),
                &options,
                prepared.plan(),
                self.threads(),
                self.frame_delay,
            )
        });
        crate::timing::report("runtime_decode", runtime_started);
        report
    }

    /// Decodes the supported envelope and discards each displayed frame.
    ///
    /// Runs the same bounded byte planning as [`Self::decode_hash_report_bytes`]
    /// and waits for each displayed frame to settle, but does not hash or
    /// serialize its samples.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for malformed sources, unsupported
    /// structures, runtime-tier rejections, resource-limit failures, worker-pool
    /// failures, or reconstruction model errors.
    pub fn decode_discard_bytes(&self, bytes: &[u8], options: DecodeOptions) -> Result<()> {
        let prepared = self.prepare_bytes(bytes, &options)?;
        let runtime_started = crate::timing::start();
        let decoded = self.install_decode(|| {
            crate::pipeline::emit_frames_from_prepared(
                bytes,
                prepared.parsed(),
                &options,
                prepared.plan(),
                self.frame_delay,
                |_| Ok(()),
            )
        });
        crate::timing::report("runtime_decode", runtime_started);
        decoded
    }

    /// Decodes the supported envelope and streams raw sample bytes.
    ///
    /// Runs bounded byte planning first (see [`Self::decode_hash_report_bytes`]).
    /// Each displayed frame is written before its output-only decoded storage
    /// is reclaimed; the complete output is not retained in decoder memory.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for malformed sources, unsupported
    /// structures, runtime-tier rejections, resource-limit failures, worker-pool
    /// failures, reconstruction model errors, raw serialization errors, or
    /// caller-writer I/O errors.
    pub fn decode_raw_bytes<W: std::io::Write + Send>(
        &self,
        bytes: &[u8],
        options: DecodeOptions,
        writer: W,
    ) -> Result<()> {
        self.decode_raw_with(
            bytes,
            options,
            move |bytes, prepared, options, frame_delay| {
                crate::output::raw::write_raw_stream_from_plan(
                    bytes,
                    prepared.parsed(),
                    options,
                    prepared.plan(),
                    frame_delay,
                    writer,
                )
            },
        )
    }

    /// Decodes raw output through output-effect materialization without
    /// serializing its sample bytes.
    ///
    /// This is the raw-output equivalent of writing to a platform null device:
    /// displayed frames and output-only effects are still resolved, but no
    /// temporary sample-byte buffer is produced.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for malformed sources, unsupported
    /// structures, runtime-tier rejections, resource-limit failures, worker-pool
    /// failures, reconstruction model errors, or output-effect errors.
    pub fn decode_raw_discard_bytes(&self, bytes: &[u8], options: DecodeOptions) -> Result<()> {
        self.decode_raw_with(bytes, options, |bytes, prepared, options, frame_delay| {
            crate::output::raw::discard_raw_stream_from_plan(
                bytes,
                prepared.parsed(),
                options,
                prepared.plan(),
                frame_delay,
            )
        })
    }

    /// Decodes the supported envelope and streams a Y4M stream.
    ///
    /// Runs bounded byte planning first (see [`Self::decode_hash_report_bytes`]).
    /// The stream header is written with the first displayed frame, and each
    /// frame is written before its output-only decoded storage is reclaimed.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for malformed sources, unsupported
    /// structures, runtime-tier rejections, resource-limit failures, worker-pool
    /// failures, reconstruction model errors, Y4M serialization errors, or
    /// caller-writer I/O errors.
    pub fn decode_y4m_bytes<W: std::io::Write + Send>(
        &self,
        bytes: &[u8],
        options: DecodeOptions,
        writer: W,
    ) -> Result<()> {
        let prepared = self.prepare_bytes(bytes, &options)?;
        self.install_decode(|| {
            crate::output::y4m::write_y4m_stream_to_writer(
                bytes,
                prepared.parsed(),
                &options,
                prepared.plan(),
                self.frame_delay,
                writer,
            )
            .map(drop)
        })
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
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
