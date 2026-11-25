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

use splot_core::simd::{SelectedBackend, SimdBackend, SimdScope, StaticSimdScope};

/// Scoped synchronous callback entered once with a concrete compile-time [`SimdBackend`].
pub trait DecodeScope<R> {
    /// Runs caller work inside one fixed `B` backend lifetime on the calling thread.
    fn run<B: SimdBackend>(self, context: &TypedDecodeContext<'_, B>) -> R;
}

/// Concrete synchronous view over [`DecodeContext`] bound to compile-time backend `B`.
///
/// Obtained from [`DecodeContext::with_typed_backend`]. All synchronous decode
/// operations execute directly on the calling thread without allocating extra
/// threads, queues, or input copies.
#[derive(Debug)]
pub struct TypedDecodeContext<'a, B: SimdBackend> {
    context: &'a DecodeContext,
    backend: B,
}

impl<B: SimdBackend> TypedDecodeContext<'_, B> {
    /// The concrete SIMD backend selected when the parent context opened.
    #[must_use]
    pub const fn selected_backend(&self) -> SelectedBackend {
        B::KIND
    }

    /// Returns the startup-validated [`SimdBackend`] capability token by value.
    #[must_use]
    pub const fn backend(&self) -> B {
        self.backend
    }

    /// The resolved, non-zero requested frame-pipelining policy.
    #[must_use]
    pub fn frame_delay(&self) -> NonZeroUsize {
        self.context.frame_delay()
    }

    /// The runtime (non-bitstream) configuration.
    #[must_use]
    pub fn runtime(&self) -> &DecodeRuntimeConfig {
        self.context.runtime()
    }

    /// The resolved, non-zero thread count.
    #[must_use]
    pub fn threads(&self) -> NonZeroUsize {
        self.context.threads()
    }

    /// [`Self::decode_hash_report_reader`] over in-memory bytes.
    ///
    /// # Errors
    /// See [`DecodeContext::decode_hash_report_reader`].
    pub fn decode_hash_report_bytes(
        &self,
        bytes: &[u8],
        options: DecodeOptions,
    ) -> Result<DecodeHashReport> {
        self.decode_hash_report_reader(bytes, options)
    }

    /// Decodes the supported envelope and returns a deterministic hash report.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for read, planning, decode, limit, or thread failures.
    pub fn decode_hash_report_reader<R: Read + Send>(
        &self,
        mut reader: R,
        options: DecodeOptions,
    ) -> Result<DecodeHashReport> {
        self.backend.enter_static(HashReport {
            context: self.context,
            reader: &mut reader,
            options: &options,
        })
    }

    /// Decodes the supported envelope and discards each displayed frame.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for read, planning, decode, limit, or thread failures.
    pub fn decode_discard_reader<R: Read + Send>(
        &self,
        mut reader: R,
        options: DecodeOptions,
    ) -> Result<()> {
        self.backend.enter_static(Discard {
            context: self.context,
            reader: &mut reader,
            options: &options,
        })
    }

    /// [`Self::decode_raw_reader`] over in-memory bytes.
    ///
    /// # Errors
    /// See [`DecodeContext::decode_raw_reader`].
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
    /// # Errors
    /// Returns [`crate::DecodeError`] for read, planning, decode, output, or thread failures.
    pub fn decode_raw_reader<R: Read + Send, W: std::io::Write + Send>(
        &self,
        mut reader: R,
        options: DecodeOptions,
        mut writer: W,
    ) -> Result<()> {
        self.backend.enter_static(Raw {
            context: self.context,
            reader: &mut reader,
            options: &options,
            writer: &mut writer,
        })
    }

    /// [`Self::decode_y4m_reader`] over in-memory bytes.
    ///
    /// # Errors
    /// See [`DecodeContext::decode_y4m_reader`].
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
    /// # Errors
    /// Returns [`crate::DecodeError`] for read, planning, decode, Y4M, or thread failures.
    pub fn decode_y4m_reader<R: Read + Send, W: std::io::Write + Send>(
        &self,
        mut reader: R,
        options: DecodeOptions,
        mut writer: W,
    ) -> Result<()> {
        self.backend.enter_static(Y4m {
            context: self.context,
            reader: &mut reader,
            options: &options,
            writer: &mut writer,
        })
    }
}

macro_rules! decode_request {
    ($request:ident, $method:ident, $output:ty $(, $field:ident: $ty:ty)*) => {
        struct $request<'a> {
            context: &'a DecodeContext,
            reader: &'a mut (dyn Read + Send),
            options: &'a DecodeOptions,
            $($field: $ty,)*
        }

        impl StaticSimdScope<Result<$output>> for $request<'_> {
            #[cfg(target_arch = "x86_64")]
            fn run_sse2(self, backend: splot_core::simd::Sse2Backend) -> Result<$output> {
                self.context.$method(backend, self.reader, self.options $(, self.$field)*)
            }
            #[cfg(target_arch = "x86_64")]
            fn run_avx2(self, backend: splot_core::simd::Avx2Backend) -> Result<$output> {
                self.context.$method(backend, self.reader, self.options $(, self.$field)*)
            }

            #[cfg(target_arch = "x86_64")]
            fn run_avx512(self, backend: splot_core::simd::Avx512Backend) -> Result<$output> {
                self.context.$method(backend, self.reader, self.options $(, self.$field)*)
            }

            #[cfg(target_arch = "aarch64")]
            fn run_neon(self, backend: splot_core::simd::NeonBackend) -> Result<$output> {
                self.context.$method(backend, self.reader, self.options $(, self.$field)*)
            }

            #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
            fn run_fallback(self, backend: splot_core::simd::FallbackBackend) -> Result<$output> {
                self.context.$method(backend, self.reader, self.options $(, self.$field)*)
            }
        }
    };
}

decode_request!(HashReport, decode_hash_report, DecodeHashReport);
decode_request!(Discard, decode_discard, ());
decode_request!(Raw, decode_raw, (), writer: &'a mut (dyn std::io::Write + Send));
decode_request!(Y4m, decode_y4m, (), writer: &'a mut (dyn std::io::Write + Send));

struct ContextScopeAdapter<'a, S> {
    context: &'a DecodeContext,
    scope: S,
}

#[derive(Clone, Copy)]
enum StreamWriteKind {
    Raw,
    Y4m,
}

impl<R, S: DecodeScope<R>> SimdScope<R> for ContextScopeAdapter<'_, S> {
    #[inline]
    fn run<B: SimdBackend>(self, backend: B) -> R {
        let typed = TypedDecodeContext {
            context: self.context,
            backend,
        };
        self.scope.run(&typed)
    }
}

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

    /// Enters `scope` once with this context's concrete compile-time backend `B`.
    ///
    /// Callers with a repeated synchronous decode loop should wrap their loop in
    /// one [`Self::with_typed_backend`] call instead of paying one outer selection
    /// per call on the compatibility methods below.
    #[inline]
    pub fn with_typed_backend<R>(&self, scope: impl DecodeScope<R>) -> R {
        self.session.backend().enter(ContextScopeAdapter {
            context: self,
            scope,
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
        reader: &mut (dyn Read + Send),
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
    /// Ergonomic compatibility facade that selects the session backend once per
    /// call; use [`Self::with_typed_backend`] to enclose repeated synchronous
    /// calls in a single typed backend lifetime.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for read failures, malformed sources,
    /// unsupported structures, runtime-tier rejections, resource-limit
    /// failures, thread failures, or reconstruction model errors.
    pub fn decode_hash_report_reader<R: Read + Send>(
        &self,
        reader: R,
        options: DecodeOptions,
    ) -> Result<DecodeHashReport> {
        struct Scope<R>(R, DecodeOptions);
        impl<R: Read + Send> DecodeScope<Result<DecodeHashReport>> for Scope<R> {
            fn run<B: SimdBackend>(
                self,
                context: &TypedDecodeContext<'_, B>,
            ) -> Result<DecodeHashReport> {
                context.decode_hash_report_reader(self.0, self.1)
            }
        }
        self.with_typed_backend(Scope(reader, options))
    }

    fn decode_hash_report(
        &self,
        backend: impl SimdBackend,
        reader: &mut (dyn Read + Send),
        options: &DecodeOptions,
    ) -> Result<DecodeHashReport> {
        Self::run(reader, options, |source| {
            crate::output::hash::decode_hash_report_from_plan(
                backend.narrow(),
                source,
                options,
                self.threads(),
                &self.session,
            )
        })
    }

    /// Decodes the supported envelope and discards each displayed frame.
    ///
    /// Plans and reads the input like [`Self::decode_hash_report_reader`] and
    /// does all the work of raw output, film grain included, but does not
    /// hash or serialize the samples.
    ///
    /// Ergonomic compatibility facade that selects the session backend once per
    /// call; use [`Self::with_typed_backend`] to enclose repeated synchronous
    /// calls in a single typed backend lifetime.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for read failures, malformed sources,
    /// unsupported structures, runtime-tier rejections, resource-limit
    /// failures, thread failures, or reconstruction model errors.
    pub fn decode_discard_reader<R: Read + Send>(
        &self,
        reader: R,
        options: DecodeOptions,
    ) -> Result<()> {
        struct Scope<R>(R, DecodeOptions);
        impl<R: Read + Send> DecodeScope<Result<()>> for Scope<R> {
            fn run<B: SimdBackend>(self, context: &TypedDecodeContext<'_, B>) -> Result<()> {
                context.decode_discard_reader(self.0, self.1)
            }
        }
        self.with_typed_backend(Scope(reader, options))
    }

    fn decode_discard(
        &self,
        backend: impl SimdBackend,
        reader: &mut (dyn Read + Send),
        options: &DecodeOptions,
    ) -> Result<()> {
        Self::run(reader, options, |source| {
            crate::output::raw::discard_raw_stream_from_plan(
                backend,
                source,
                options,
                &self.session,
            )
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
    /// Ergonomic compatibility facade that selects the session backend once per
    /// call; use [`Self::with_typed_backend`] to enclose repeated synchronous
    /// calls in a single typed backend lifetime.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for read failures, malformed sources,
    /// unsupported structures, runtime-tier rejections, resource-limit
    /// failures, thread failures, reconstruction model errors, raw
    /// serialization errors, or caller-writer I/O errors.
    pub fn decode_raw_reader<R: Read + Send, W: std::io::Write + Send>(
        &self,
        reader: R,
        options: DecodeOptions,
        writer: W,
    ) -> Result<()> {
        self.decode_with_writer_scope(StreamWriteKind::Raw, reader, &options, writer)
    }

    fn decode_with_writer_scope<R: Read + Send, W: std::io::Write + Send>(
        &self,
        kind: StreamWriteKind,
        reader: R,
        options: &DecodeOptions,
        writer: W,
    ) -> Result<()> {
        struct Scope<'a, R, W>(StreamWriteKind, R, &'a DecodeOptions, W);
        impl<R: Read + Send, W: std::io::Write + Send> DecodeScope<Result<()>> for Scope<'_, R, W> {
            fn run<B: SimdBackend>(self, context: &TypedDecodeContext<'_, B>) -> Result<()> {
                match self.0 {
                    StreamWriteKind::Raw => context.decode_raw_reader(self.1, *self.2, self.3),
                    StreamWriteKind::Y4m => context.decode_y4m_reader(self.1, *self.2, self.3),
                }
            }
        }
        self.with_typed_backend(Scope(kind, reader, options, writer))
    }

    fn decode_raw(
        &self,
        backend: impl SimdBackend,
        reader: &mut (dyn Read + Send),
        options: &DecodeOptions,
        writer: &mut (dyn std::io::Write + Send),
    ) -> Result<()> {
        Self::run(reader, options, |source| {
            crate::output::raw::write_raw_stream_from_plan(
                backend,
                source,
                options,
                &self.session,
                writer,
            )
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
    /// Ergonomic compatibility facade that selects the session backend once per
    /// call; use [`Self::with_typed_backend`] to enclose repeated synchronous
    /// calls in a single typed backend lifetime.
    ///
    /// # Errors
    /// Returns [`crate::DecodeError`] for read failures, malformed sources,
    /// unsupported structures, runtime-tier rejections, resource-limit
    /// failures, thread failures, reconstruction model errors, Y4M
    /// serialization errors, or caller-writer I/O errors.
    pub fn decode_y4m_reader<R: Read + Send, W: std::io::Write + Send>(
        &self,
        reader: R,
        options: DecodeOptions,
        writer: W,
    ) -> Result<()> {
        self.decode_with_writer_scope(StreamWriteKind::Y4m, reader, &options, writer)
    }

    fn decode_y4m(
        &self,
        backend: impl SimdBackend,
        reader: &mut (dyn Read + Send),
        options: &DecodeOptions,
        writer: &mut (dyn std::io::Write + Send),
    ) -> Result<()> {
        Self::run(reader, options, |source| {
            crate::output::y4m::write_y4m_stream_to_writer(
                backend,
                source,
                options,
                &self.session,
                writer,
            )
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
