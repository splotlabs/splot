// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Internal zero-cost wrapper pairing a [`BitReader`] with a [`SyntaxObserver`].

use crate::bitio::BitReader;
use crate::error::{ByteAlignmentErrorKind, Error, Result};
use crate::span::{BitOffset, BitSpan, ByteOffset};
use crate::syntax::{
    ScopeKind, SyntaxDescriptor, SyntaxErrorRef, SyntaxFieldAbsent, SyntaxFieldRead,
    SyntaxObserver, SyntaxValue,
};

/// Pairs a [`BitReader`] with a [`SyntaxObserver`] and tracks the root payload
/// bit origin so sub-readers created via `take_bytes` preserve exact
/// OBU-payload-relative [`BitSpan`]s.
pub(crate) struct SyntaxReader<'r, 'a, O: SyntaxObserver + ?Sized> {
    reader: &'r mut BitReader<'a>,
    origin_abs_bit: u64,
    observer: &'r mut O,
}

impl<'a> SyntaxReader<'_, 'a, crate::syntax::NoopObserver> {
    #[inline]
    pub(crate) fn with_noop<R>(
        reader: &mut BitReader<'a>,
        f: impl FnOnce(&mut SyntaxReader<'_, 'a, crate::syntax::NoopObserver>) -> R,
    ) -> R {
        let mut noop = crate::syntax::NoopObserver;
        let mut sr = SyntaxReader::new(reader, &mut noop);
        f(&mut sr)
    }
}

impl<'r, 'a, O: SyntaxObserver + ?Sized> SyntaxReader<'r, 'a, O> {
    /// Creates a root [`SyntaxReader`] whose [`BitSpan`]s start at `0` for the
    /// current position of `reader`.
    #[inline]
    pub(crate) fn new(reader: &'r mut BitReader<'a>, observer: &'r mut O) -> Self {
        let origin_abs_bit = reader.abs_bit_offset();
        Self {
            reader,
            origin_abs_bit,
            observer,
        }
    }

    /// Creates a [`SyntaxReader`] sharing an existing `origin_abs_bit` (for
    /// length-bounded sub-readers produced by [`Self::take_bytes_sub`]).
    #[inline]
    pub(crate) fn with_origin(
        reader: &'r mut BitReader<'a>,
        origin_abs_bit: u64,
        observer: &'r mut O,
    ) -> Self {
        Self {
            reader,
            origin_abs_bit,
            observer,
        }
    }

    /// Returns the absolute bit origin of the observed payload.
    #[inline]
    pub(crate) const fn origin_abs_bit(&self) -> u64 {
        self.origin_abs_bit
    }

    /// Returns the current bit offset relative to the observed payload start.
    #[inline]
    pub(crate) const fn rel_bit(&self) -> u64 {
        self.reader
            .abs_bit_offset()
            .saturating_sub(self.origin_abs_bit)
    }

    /// Borrows the underlying [`BitReader`] read-only.
    #[inline]
    pub(crate) fn reader(&self) -> &BitReader<'a> {
        self.reader
    }

    /// Borrows the underlying [`BitReader`] mutably.
    #[inline]
    pub(crate) fn reader_mut(&mut self) -> &mut BitReader<'a> {
        self.reader
    }

    /// Borrows the observer mutably.
    #[inline]
    pub(crate) fn observer_mut(&mut self) -> &mut O {
        self.observer
    }

    #[inline]
    pub(crate) const fn byte_offset(&self) -> ByteOffset {
        self.reader.byte_offset()
    }

    #[inline]
    pub(crate) const fn bit_offset(&self) -> BitOffset {
        self.reader.bit_offset()
    }

    #[inline]
    pub(crate) const fn is_byte_aligned(&self) -> bool {
        self.reader.is_byte_aligned()
    }

    #[inline]
    pub(crate) fn remaining_bits(&self) -> u64 {
        self.reader.remaining_bits()
    }

    #[inline]
    pub(crate) fn consumed_bits(&self) -> u64 {
        self.reader.consumed_bits()
    }

    /// Runs `f` inside a named AV2 syntax structure scope, always emitting
    /// `exit_scope` even on error.
    #[inline]
    pub(crate) fn in_struct<T>(
        &mut self,
        name: &'static str,
        spec_section: Option<&'static str>,
        f: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        self.observer
            .enter_scope(ScopeKind::Structure { name, spec_section });
        let result = f(self);
        self.observer.exit_scope();
        result
    }

    /// Runs `f` inside an indexed loop-iteration scope, always emitting
    /// `exit_scope` even on error.
    #[inline]
    pub(crate) fn in_loop_item<T>(
        &mut self,
        name: &'static str,
        index: usize,
        f: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        self.observer
            .enter_scope(ScopeKind::LoopItem { name, index });
        let result = f(self);
        self.observer.exit_scope();
        result
    }

    /// Reports a semantic validation error at `bit_offset` and returns `Err(error)`.
    #[inline]
    pub(crate) fn fail<T>(
        &mut self,
        bit_offset: u64,
        field: Option<&'static str>,
        error: Error,
    ) -> Result<T> {
        self.observer.on_error(SyntaxErrorRef {
            bit_offset,
            field,
            error: &error,
        });
        Err(error)
    }

    /// Records an absent field with a boolean inferred default.
    #[inline]
    pub(crate) fn inferred_bool(
        &mut self,
        name: &'static str,
        condition: &'static str,
        value: bool,
    ) {
        self.observer.on_field_absent(SyntaxFieldAbsent {
            name,
            condition,
            inferred: Some(SyntaxValue::Bool(value)),
        });
    }

    /// Records an absent field with an unsigned integer inferred default.
    #[inline]
    pub(crate) fn inferred_u64(&mut self, name: &'static str, condition: &'static str, value: u64) {
        self.observer.on_field_absent(SyntaxFieldAbsent {
            name,
            condition,
            inferred: Some(SyntaxValue::Unsigned(value)),
        });
    }

    /// Records an absent field with a signed integer inferred default.
    #[inline]
    pub(crate) fn inferred_i64(&mut self, name: &'static str, condition: &'static str, value: i64) {
        self.observer.on_field_absent(SyntaxFieldAbsent {
            name,
            condition,
            inferred: Some(SyntaxValue::Signed(value)),
        });
    }

    /// Records a conditionally absent field that has no spec-inferred value.
    #[inline]
    pub(crate) fn absent(&mut self, name: &'static str, condition: &'static str) {
        self.observer.on_field_absent(SyntaxFieldAbsent {
            name,
            condition,
            inferred: None,
        });
    }

    /// Emits a custom-decoded field read (such as `tu(mx)` or `subexp`).
    #[inline]
    pub(crate) fn emit_read(
        &mut self,
        name: &'static str,
        descriptor: SyntaxDescriptor,
        value: SyntaxValue,
        start_bit: u64,
    ) {
        let end_bit = self.rel_bit();
        self.observer.on_field_read(SyntaxFieldRead {
            name,
            descriptor,
            value,
            span: BitSpan::new(start_bit, end_bit.saturating_sub(start_bit)),
        });
    }

    #[inline]
    fn read_elem<T: Copy + Into<SyntaxValue>>(
        &mut self,
        name: &'static str,
        descriptor: SyntaxDescriptor,
        read: impl FnOnce(&mut BitReader<'a>) -> Result<T>,
    ) -> Result<T> {
        let start = self.rel_bit();
        match read(self.reader) {
            Ok(value) => {
                self.emit_read(name, descriptor, value.into(), start);
                Ok(value)
            }
            Err(error) => self.fail(start, Some(name), error),
        }
    }

    #[inline]
    pub(crate) fn read_flag(&mut self, name: &'static str) -> Result<bool> {
        self.read_elem(name, SyntaxDescriptor::F(1), BitReader::read_flag)
    }

    #[inline]
    pub(crate) fn read_bit(&mut self, name: &'static str) -> Result<u8> {
        self.read_elem(name, SyntaxDescriptor::F(1), BitReader::read_bit)
    }

    #[inline]
    pub(crate) fn read_bits(&mut self, name: &'static str, n: u32) -> Result<u32> {
        self.read_elem(name, SyntaxDescriptor::F(n), |r| r.read_bits(n))
    }

    #[inline]
    pub(crate) fn read_bits_u8(&mut self, name: &'static str, n: u32) -> Result<u8> {
        self.read_elem(name, SyntaxDescriptor::F(n), |r| r.read_bits_u8(n))
    }

    #[inline]
    pub(crate) fn read_su(&mut self, name: &'static str, n: u32) -> Result<i32> {
        self.read_elem(name, SyntaxDescriptor::Su(n), |r| r.read_su(n))
    }

    #[inline]
    pub(crate) fn read_uvlc(&mut self, name: &'static str) -> Result<u32> {
        self.read_elem(name, SyntaxDescriptor::Uvlc, BitReader::read_uvlc)
    }

    #[inline]
    pub(crate) fn read_svlc(&mut self, name: &'static str) -> Result<i32> {
        self.read_elem(name, SyntaxDescriptor::Svlc, BitReader::read_svlc)
    }

    #[inline]
    pub(crate) fn read_leb128(&mut self, name: &'static str) -> Result<u32> {
        self.read_elem(name, SyntaxDescriptor::Leb128, BitReader::read_leb128)
    }

    #[inline]
    pub(crate) fn read_ns(&mut self, name: &'static str, n: u32) -> Result<u32> {
        self.read_elem(name, SyntaxDescriptor::Ns(n), |r| r.read_ns(n))
    }

    #[inline]
    pub(crate) fn read_rg(&mut self, name: &'static str, n: u32) -> Result<u32> {
        self.read_elem(name, SyntaxDescriptor::Rg(n), |r| r.read_rg(n))
    }

    /// Splits a length-bounded sub-reader for a child syntax structure without
    /// emitting a leaf field.
    #[inline]
    pub(crate) fn take_bytes_sub(
        &mut self,
        n: usize,
        field: &'static str,
    ) -> Result<BitReader<'a>> {
        let start = self.rel_bit();
        match self.reader.take_bytes(n) {
            Ok(sub) => Ok(sub),
            Err(error) => self.fail(start, Some(field), error),
        }
    }

    /// Reads `n` opaque bytes via `read_bits_u8(8)` and emits one
    /// [`SyntaxDescriptor::Bytes`] field covering the full span.
    pub(crate) fn read_raw_bytes(&mut self, name: &'static str, n: usize) -> Result<Vec<u8>> {
        let start = self.rel_bit();
        let mut out = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            match self.reader.read_bits_u8(8) {
                Ok(byte) => out.push(byte),
                Err(error) => return self.fail(self.rel_bit(), Some(name), error),
            }
        }
        self.observer.on_field_read(SyntaxFieldRead {
            name,
            descriptor: SyntaxDescriptor::Bytes(n),
            value: SyntaxValue::Bytes { len: n },
            span: BitSpan::new(start, (n as u64).saturating_mul(8)),
        });
        Ok(out)
    }

    /// Parses AV2 `byte_alignment()` (§ 5.2.4) and observes each `zero_bit`.
    pub(crate) fn byte_align_zero(&mut self) -> Result<()> {
        if self.is_byte_aligned() {
            return Ok(());
        }
        self.in_struct("byte_alignment", Some("5.2.4"), |sr| {
            while !sr.is_byte_aligned() {
                let start = sr.rel_bit();
                let offset = sr.byte_offset();
                let bit_offset = sr.bit_offset();
                if sr.read_flag("zero_bit")? {
                    return sr.fail(
                        start,
                        Some("zero_bit"),
                        Error::InvalidByteAlignment {
                            offset,
                            bit_offset,
                            kind: ByteAlignmentErrorKind::ZeroBitNotZero,
                        },
                    );
                }
            }
            Ok(())
        })
    }
}
