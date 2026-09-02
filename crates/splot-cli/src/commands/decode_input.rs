// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The decoder's view of an input bitstream.

#![allow(
    unsafe_code,
    reason = "mapping the input file is this crate's only unsafe boundary"
)]

use std::fmt;
use std::fs::File;
use std::ops::Deref;

/// A whole bitstream the decoder reads as one slice.
///
/// Reading the file into a `Vec` makes the process resident set grow with the
/// input, which for a feature-length stream dwarfs the decoder itself. A
/// read-only mapping leaves the pages file-backed and reclaimable instead.
pub(crate) enum InputBytes {
    /// Read into the process, for inputs that cannot be mapped.
    Owned(Vec<u8>),
    /// Mapped read-only from the file.
    Mapped(memmap2::Mmap),
}

impl InputBytes {
    /// Maps `file` read-only for the life of the decode.
    ///
    /// The mapping tracks the file, so another process truncating or rewriting
    /// it while a decode runs yields torn input or a fault. Decoding is a
    /// short-lived read-only pass over a file the caller named, which is the
    /// same trade every other media tool makes for mapped input.
    pub(crate) fn map(file: &File) -> std::io::Result<Self> {
        // SAFETY: the mapping is read-only, lives no longer than this decode,
        // and is only exposed as a shared slice.
        unsafe { memmap2::Mmap::map(file) }.map(Self::Mapped)
    }
}

impl Deref for InputBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Self::Owned(bytes) => bytes,
            Self::Mapped(bytes) => bytes,
        }
    }
}

impl fmt::Debug for InputBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self {
            Self::Owned(_) => "owned",
            Self::Mapped(_) => "mapped",
        };
        formatter
            .debug_struct("InputBytes")
            .field("kind", &kind)
            .field("len", &self.len())
            .finish()
    }
}
