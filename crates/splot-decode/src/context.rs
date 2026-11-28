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

use splot_core::simd::SelectedBackend;

/// A raw or Y4M stream writer of the decode output paths.
type StreamWriter = fn(
    &mut dyn UnitSource,
    &DecodeOptions,
    &crate::pipeline::DecodeSession,
    &mut (dyn std::io::Write + Send),
) -> Result<()>;

impl DecodeContext {
    /// Creates a decode context, validates its SIMD policy, and starts its row scheduler.
    ///
    /// Both the frame-pipelining depth and SIMD backend policy are resolved once
    /// here, so no decode path re-resolves them.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError::UnsupportedBackend`] if the requested SIMD
    /// policy cannot run on this host, or [`crate::DecodeError::Pool`] if a
    /// worker thread cannot start.
    pub fn new(runtime: DecodeRuntimeConfig) -> Result<Self> {
        let backend = splot_core::simd::resolve_backend(runtime.simd_policy)?;
        let threads = runtime.thread_count.resolve();
        let frame_delay = runtime.frame_delay.resolve(threads);
        let rows = crate::pipeline::row_scheduler(threads, backend)?;
        Ok(Self {
            runtime,
            threads,
            session: crate::pipeline::DecodeSession::new(frame_delay, rows, backend),
        })
    }

    /// Supplies external atlas metadata for each decode on this context.
    #[must_use]
    pub fn with_external_atlases(mut self, atlases: crate::ExternalAtlases) -> Self {
        self.session.external_atlases = atlases;
        self
    }

    /// The concrete SIMD backend selected when this context opened.
    #[must_use]
    pub const fn selected_backend(&self) -> SelectedBackend {
        self.session.backend().kind()
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
    /// thread, which drives the row workers, with this context's session.
    fn run<T>(
        &self,
        reader: &mut (dyn Read + Send),
        options: &DecodeOptions,
        decode: impl FnOnce(&mut dyn UnitSource, &crate::pipeline::DecodeSession) -> Result<T>,
    ) -> Result<T> {
        let mut source = ReaderUnits::new(reader, options.limits());
        decode(&mut source, &self.session)
    }

    /// Streams the displayed frames of `reader` into `writer` through `write`.
    fn write_stream(
        &self,
        reader: &mut (dyn Read + Send),
        options: &DecodeOptions,
        writer: &mut (dyn std::io::Write + Send),
        write: StreamWriter,
    ) -> Result<()> {
        self.run(reader, options, |source, session| {
            write(source, options, session, writer)
        })
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
        self.run(&mut reader, &options, |source, session| {
            crate::output::hash::decode_hash_report_from_plan(
                source,
                &options,
                self.threads(),
                session,
            )
        })
    }

    /// Decodes the supported envelope and discards each displayed frame.
    ///
    /// Plans and reads the input like [`Self::decode_hash_report_reader`] and
    /// does all the work of raw output, film grain included, but does not
    /// hash or serialize the samples.
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
        self.run(&mut reader, &options, |source, session| {
            crate::output::raw::discard_raw_stream_from_plan(source, &options, session)
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
        mut writer: W,
    ) -> Result<()> {
        self.write_stream(
            &mut reader,
            &options,
            &mut writer,
            crate::output::raw::write_raw_stream_from_plan,
        )
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
        mut writer: W,
    ) -> Result<()> {
        self.write_stream(
            &mut reader,
            &options,
            &mut writer,
            crate::output::y4m::write_y4m_stream_to_writer,
        )
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
