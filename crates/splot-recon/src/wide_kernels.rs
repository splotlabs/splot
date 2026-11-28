// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The kernels a decoder that selected AVX-512 calls in place of their AVX2
//! forms, bound once to safe function pointers.

use std::sync::OnceLock;

use fearless_simd::Avx512;

use crate::IntraRectBlockSize;

/// PAETH prediction of one block: its size, the left and above edges with the
/// corner sample, and the output rows with their stride.
pub(crate) type PaethFn<O> = fn(IntraRectBlockSize, (&[O], &[O], u16), &mut [O], usize);

/// The palette lookup of one block: the low and high bytes of its colors, its
/// color map and the output samples; returns the largest index of the map.
pub(crate) type PaletteFn<O> = fn([u8; 16], Option<[u8; 16]>, &[u8], &mut [O]) -> u8;

/// One entry per kernel and sample type. Each is made by a factory beside
/// its kernel, which converts the kernel to a pointer where the AVX-512
/// features are enabled.
pub(crate) struct WideKernels {
    pub(crate) paeth_u8: PaethFn<u8>,
    pub(crate) paeth_u16: PaethFn<u16>,
    pub(crate) palette_u8: PaletteFn<u8>,
    pub(crate) palette_u16: PaletteFn<u16>,
}

impl WideKernels {
    /// The set for `token`, bound on first use.
    pub(crate) fn of(token: Avx512) -> &'static Self {
        static SET: OnceLock<WideKernels> = OnceLock::new();
        SET.get_or_init(|| Self {
            paeth_u8: crate::intra_basic::bind_paeth_u8(token),
            paeth_u16: crate::intra_basic::bind_paeth_u16(token),
            palette_u8: crate::intra::bind_palette_u8(token),
            palette_u16: crate::intra::bind_palette_u16(token),
        })
    }
}
