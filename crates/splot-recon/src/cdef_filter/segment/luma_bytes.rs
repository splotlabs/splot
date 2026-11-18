// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Padded byte windows for the bytes-only luma segment.

use super::*;

pub(super) const LUMA_BYTE_STRIDE: usize = 16;
pub(super) const LUMA_BYTE_BLOCK: usize = 12 * LUMA_BYTE_STRIDE;
pub(super) const LUMA_BYTE_SPAN: usize = 6 * LUMA_BYTE_STRIDE + 32;
pub(super) const LUMA_PADDED_BYTES: usize = u8::MAX as usize + LUMA_BYTE_SPAN;
pub(super) const LUMA_BYTES_AREA: usize =
    (CDEF_SEGMENT_BLOCKS - 1) * LUMA_BYTE_BLOCK + LUMA_PADDED_BYTES;
pub(super) const LUMA_BYTE_STARTS: CdefTapStartTable = cdef_tap_starts(
    &cdef_tap_offsets(LUMA_BYTE_STRIDE as isize, 1),
    2 * LUMA_BYTE_STRIDE + TAP_REACH,
);

/// [`gather_luma`] into the byte scratch alone.
pub(super) fn gather_luma_bytes(
    samples: &[u8],
    width: usize,
    stride: usize,
    bytes: &mut [u8; LUMA_BYTES_AREA],
    origin: (usize, usize),
) -> Option<[[u16; 2]; CDEF_SEGMENT_BLOCKS]> {
    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = Level::baseline().as_avx2() {
        return gather_luma_bytes_with(avx2, MinPos, samples, width, stride, bytes, origin);
    }
    baseline_simd!(simd => gather_luma_bytes_with(simd, Reduced, samples, width, stride, bytes, origin))
}

#[allow(clippy::inline_always, reason = "measured CDEF gather hot path")]
#[inline(always)]
fn gather_luma_bytes_with<S: SimdToken, R: LaneBounds<S>>(
    simd: S,
    reduce: R,
    samples: &[u8],
    width: usize,
    stride: usize,
    bytes: &mut [u8; LUMA_BYTES_AREA],
    origin: (usize, usize),
) -> Option<[[u16; 2]; CDEF_SEGMENT_BLOCKS]> {
    const { assert!(CDEF_SEGMENT_BLOCKS == 4 && CDEF_SEGMENT_STRIDE == 36) };
    let rows = reach_rows(samples, width, stride, origin, (CDEF_SEGMENT_STRIDE, 12))?;
    let bias = u8x32::splat(simd, 0x80);
    let mut min = [u8x32::splat(simd, u8::MAX); 2];
    let mut max = [u8x32::splat(simd, 0); 2];
    for row in 0..12 {
        let src = rows
            .get(row * stride..)?
            .first_chunk::<CDEF_SEGMENT_STRIDE>()?;
        let lanes = [
            u8x32::from_slice(simd, &src[..32]),
            u8x32::from_slice(simd, &src[4..36]),
        ];
        let (a, c) = simd.split_u8x32(lanes[0] ^ bias);
        let (_, tail) = simd.split_u8x32(lanes[1] ^ bias);
        let b = simd.slide_u8x16::<8>(a, c);
        let d = simd.slide_u8x16::<4>(tail, tail);
        for (block, window) in [a, b, c, d].into_iter().enumerate() {
            let at = block * LUMA_BYTE_BLOCK + row * LUMA_BYTE_STRIDE;
            window.store_slice(&mut bytes[at..at + LUMA_BYTE_STRIDE]);
        }
        for half in 0..2 {
            min[half] = min[half].min(lanes[half]);
            max[half] = max[half].max(lanes[half]);
        }
    }
    let chunks = |[head, tail]: [u8x32<S>; 2]| byte_eighths(simd, head, tail);
    Some(segment_bounds(simd, reduce, chunks(min), chunks(max)))
}

/// The § 7.18.2 luma direction inputs from bytes already offset by 128.
#[allow(clippy::inline_always, reason = "measured CDEF direction hot path")]
#[inline(always)]
pub(super) fn direction_byte_rows<S: SimdToken>(
    simd: S,
    bytes: &[u8; LUMA_PADDED_BYTES],
) -> [i16x8<S>; 8] {
    core::array::from_fn(|i| {
        let start = (i + TAP_REACH) * LUMA_BYTE_STRIDE + TAP_REACH;
        let lanes = u8x16::from_slice(simd, &bytes[start..start + 16]).bitcast::<i8x16<S>>();
        simd.widen_i8x16(lanes).0
    })
}
