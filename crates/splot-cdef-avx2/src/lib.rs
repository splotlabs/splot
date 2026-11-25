// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AVX2 entry for the packed CDEF chroma gather.

#![cfg(target_arch = "x86_64")]

use fearless_simd::{Avx2, Simd};
use splot_cdef_base::{PACKED_BYTES, gather_packed_with, minpos_bounds};

/// Gathers the four packed chroma windows using the caller's AVX2 proof.
/// Row addressing and partial scratch writes on failure follow
/// [`splot_cdef_base::gather_with`], including strides that overlap rows.
#[inline(never)]
pub fn gather_packed(
    simd: Avx2,
    planes: [&[u8]; 2],
    stride: usize,
    bytes: &mut [u8; PACKED_BYTES],
) -> Option<[u16; 4]> {
    simd.vectorize(
        #[inline(always)]
        || {
            gather_packed_with(simd, planes, stride, bytes, |min, max| {
                minpos_bounds(simd, min, max)
            })
        },
    )
}
