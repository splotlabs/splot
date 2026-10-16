// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A decode-driver context that owns the decode threads plus byte-consuming
//! and parsed-stream planning entry points.

use core::num::NonZeroUsize;
use std::io::Read;

use crate::DecodeHashReport;
use crate::DecodeOptions;
use crate::bitstream::byte_stream::{ReaderUnits, UnitSource, plan_byte_stream};
use crate::bitstream::stream_plan::{DecodeStreamInput, DecodeStreamPlan, plan_stream};
use crate::error::Result;
use crate::runtime::DecodeRuntimeConfig;

/// A decode input, read forward once.
trait ReadSend: Read + Send {}

impl<R: Read + Send + ?Sized> ReadSend for R {}

/// A decode context.
///
/// Owns the decode threads, plans bounded stream metadata from raw Annex
/// B/IVF or parsed streams, and exposes the runtime hash, raw, Y4M, and
/// discard-output paths for the supported decode envelope (tracked in
/// `docs/DECODER-SUPPORT-MATRIX.toml`). It does not touch the filesystem or
/// invoke any external decoder.
///
/// A context keeps the decoder state of its last successful decode, sized
/// for the largest stream it has decoded, so the next call reuses it instead
/// of allocating it again. Drop the context to release that memory.
#[derive(Debug)]
pub struct DecodeContext {
    runtime: DecodeRuntimeConfig,
    threads: NonZeroUsize,
    session: crate::pipeline::DecodeSession,
}

impl DecodeContext {
    /// Creates a decode context and starts its row scheduler.
    ///
    /// The configured frame-pipelining depth is resolved once here against the
    /// thread count, so no decode path re-resolves it.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError::Pool`] if a worker thread cannot start.
    pub fn new(runtime: DecodeRuntimeConfig) -> Result<Self> {
        let threads = runtime.thread_count.resolve();
        let frame_delay = runtime.frame_delay.resolve(threads);
        let rows = crate::pipeline::row_scheduler(threads)?;
        Ok(Self {
            runtime,
            threads,
            session: crate::pipeline::DecodeSession::new(frame_delay, rows),
        })
    }

    /// The resolved, non-zero requested frame-pipelining policy.
    ///
    /// The pipeline uses the smaller of this value and the worker count as its
    /// effective in-flight capacity without changing the scheduling algorithm.
    #[must_use]
    pub fn frame_delay(&self) -> NonZeroUsize {
        self.session.frame_delay()
    }

    /// The runtime (non-bitstream) configuration.
    #[must_use]
    pub fn runtime(&self) -> &DecodeRuntimeConfig {
        &self.runtime
    }

    /// The resolved, non-zero thread count.
    #[must_use]
    pub fn threads(&self) -> NonZeroUsize {
        self.threads
    }

    /// Builds a deterministic plan over raw AV2 Annex B or IVF bytes.
    ///
    /// Plan-only: bounds byte
    /// traversal, decodes no tile payloads, reconstructs no pixels, writes no
    /// output, and invokes no external decoder.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for malformed sources, unsupported
    /// structures, local decode resource-limit failures, or pool failures.
    pub fn plan_bytes(bytes: &[u8], options: DecodeOptions) -> Result<DecodeStreamPlan> {
        plan_byte_stream(bytes, &options)
    }

    /// Runs `decode` over the temporal units of `reader` on the calling
    /// thread, which drives the row workers.
    fn run<T: Send>(
        reader: &mut dyn ReadSend,
        options: &DecodeOptions,
        decode: impl FnOnce(&mut dyn UnitSource) -> Result<T> + Send,
    ) -> Result<T> {
        let mut source = ReaderUnits::new(reader, options.limits());
        decode(&mut source)
    }

    /// [`Self::decode_hash_report_reader`] over in-memory bytes.
    ///
    /// # Errors
    /// See [`Self::decode_hash_report_reader`].
    pub fn decode_hash_report_bytes(
        &self,
        bytes: &[u8],
        options: DecodeOptions,
    ) -> Result<DecodeHashReport> {
        self.decode_hash_report_reader(bytes, options)
    }

    /// Decodes the supported envelope and returns a deterministic hash report.
    ///
    /// Plans the whole input first (see [`Self::plan_bytes`]) so malformed
    /// sources, resource-limit failures, layer selection, and planner-level
    /// unsupported structures stay transactional; then reads the input again
    /// and holds only the IVF frame records still being decoded. The supported
    /// decode envelope is tracked in `docs/DECODER-SUPPORT-MATRIX.toml`.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for read failures, malformed sources,
    /// unsupported structures, runtime-tier rejections, resource-limit
    /// failures, thread failures, or reconstruction model errors.
    pub fn decode_hash_report_reader<R: Read + Send>(
        &self,
        mut reader: R,
        options: DecodeOptions,
    ) -> Result<DecodeHashReport> {
        self.decode_hash_report(&mut reader, options)
    }

    fn decode_hash_report(
        &self,
        reader: &mut dyn ReadSend,
        options: DecodeOptions,
    ) -> Result<DecodeHashReport> {
        Self::run(reader, &options, |source| {
            crate::output::hash::decode_hash_report_from_plan(
                source,
                &options,
                self.threads(),
                &self.session,
            )
        })
    }

    /// [`Self::decode_discard_reader`] over in-memory bytes.
    ///
    /// # Errors
    /// See [`Self::decode_discard_reader`].
    pub fn decode_discard_bytes(&self, bytes: &[u8], options: DecodeOptions) -> Result<()> {
        self.decode_discard_reader(bytes, options)
    }

    /// Decodes the supported envelope and discards each displayed frame.
    ///
    /// Plans and reads the input like [`Self::decode_hash_report_reader`] and
    /// waits for each displayed frame to settle, but does not hash or
    /// serialize its samples.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for read failures, malformed sources,
    /// unsupported structures, runtime-tier rejections, resource-limit
    /// failures, thread failures, or reconstruction model errors.
    pub fn decode_discard_reader<R: Read + Send>(
        &self,
        mut reader: R,
        options: DecodeOptions,
    ) -> Result<()> {
        self.decode_discard(&mut reader, options)
    }

    fn decode_discard(&self, reader: &mut dyn ReadSend, options: DecodeOptions) -> Result<()> {
        Self::run(reader, &options, |source| {
            crate::pipeline::emit_frames(source, &options, &self.session, |_| Ok(()), |_| Ok(()))
        })
    }

    /// [`Self::decode_raw_reader`] over in-memory bytes.
    ///
    /// # Errors
    /// See [`Self::decode_raw_reader`].
    pub fn decode_raw_bytes<W: std::io::Write + Send>(
        &self,
        bytes: &[u8],
        options: DecodeOptions,
        writer: W,
    ) -> Result<()> {
        self.decode_raw_reader(bytes, options, writer)
    }

    /// Decodes the supported envelope and streams raw sample bytes.
    ///
    /// Plans and reads the input like [`Self::decode_hash_report_reader`].
    /// Each displayed frame is written before its output-only decoded storage
    /// is reclaimed; the complete output is not retained in decoder memory.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for read failures, malformed sources,
    /// unsupported structures, runtime-tier rejections, resource-limit
    /// failures, thread failures, reconstruction model errors, raw
    /// serialization errors, or caller-writer I/O errors.
    pub fn decode_raw_reader<R: Read + Send, W: std::io::Write + Send>(
        &self,
        mut reader: R,
        options: DecodeOptions,
        writer: W,
    ) -> Result<()> {
        Self::run(&mut reader, &options, |source| {
            crate::output::raw::write_raw_stream_from_plan(source, &options, &self.session, writer)
        })
    }

    /// Decodes raw output through output-effect materialization without
    /// serializing its sample bytes.
    ///
    /// This is the raw-output equivalent of writing to a platform null device:
    /// displayed frames and output-only effects are still resolved, but no
    /// temporary sample-byte buffer is produced.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for read failures, malformed sources,
    /// unsupported structures, runtime-tier rejections, resource-limit
    /// failures, thread failures, reconstruction model errors, or
    /// output-effect errors.
    pub fn decode_raw_discard_reader<R: Read + Send>(
        &self,
        mut reader: R,
        options: DecodeOptions,
    ) -> Result<()> {
        self.decode_raw_discard(&mut reader, options)
    }

    fn decode_raw_discard(&self, reader: &mut dyn ReadSend, options: DecodeOptions) -> Result<()> {
        Self::run(reader, &options, |source| {
            crate::output::raw::discard_raw_stream_from_plan(source, &options, &self.session)
        })
    }

    /// [`Self::decode_y4m_reader`] over in-memory bytes.
    ///
    /// # Errors
    /// See [`Self::decode_y4m_reader`].
    pub fn decode_y4m_bytes<W: std::io::Write + Send>(
        &self,
        bytes: &[u8],
        options: DecodeOptions,
        writer: W,
    ) -> Result<()> {
        self.decode_y4m_reader(bytes, options, writer)
    }

    /// Decodes the supported envelope and streams a Y4M stream.
    ///
    /// Plans and reads the input like [`Self::decode_hash_report_reader`].
    /// The stream header is written with the first displayed frame, and each
    /// frame is written before its output-only decoded storage is reclaimed.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for read failures, malformed sources,
    /// unsupported structures, runtime-tier rejections, resource-limit
    /// failures, thread failures, reconstruction model errors, Y4M
    /// serialization errors, or caller-writer I/O errors.
    pub fn decode_y4m_reader<R: Read + Send, W: std::io::Write + Send>(
        &self,
        mut reader: R,
        options: DecodeOptions,
        writer: W,
    ) -> Result<()> {
        Self::run(&mut reader, &options, |source| {
            crate::output::y4m::write_y4m_stream_to_writer(source, &options, &self.session, writer)
                .map(drop)
        })
    }

    /// Builds a deterministic plan over an already parsed AV2 stream.
    ///
    /// Serial and plan-only: consumes
    /// no raw bytes, decodes no tile payloads, reconstructs no pixels, and
    /// invokes no external decoder.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for malformed parsed sources,
    /// unsupported structures, or local decode resource-limit failures.
    pub fn plan_stream(
        input: DecodeStreamInput<'_>,
        options: DecodeOptions,
    ) -> Result<DecodeStreamPlan> {
        plan_stream(input, &options)
    }
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
