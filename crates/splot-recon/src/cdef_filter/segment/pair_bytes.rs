// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Padded byte windows for native 8-bit chroma pairs.

use super::*;

pub(super) const PAIR_BYTE_BLOCK: usize = PAIR_ROWS * LUMA_BYTE_STRIDE;
const PAIR_BYTE_SPAN: usize = 2 * LUMA_BYTE_STRIDE + 32;
const PAIR_PADDED_BYTES: usize = u8::MAX as usize + PAIR_BYTE_SPAN;
pub(super) const PAIR_BYTES_AREA: usize =
    (CDEF_SEGMENT_BLOCKS - 1) * PAIR_BYTE_BLOCK + PAIR_PADDED_BYTES;
const PAIR_BYTE_STARTS: CdefTapStarts = cdef_tap_starts(
    &cdef_tap_offsets(LUMA_BYTE_STRIDE as isize, 2),
    2 * LUMA_BYTE_STRIDE + 2 * TAP_REACH,
);

impl CdefUnitFilters {
    /// Whether [`CdefPairSegment::gather_bytes`] suffices for the unit's chroma filter.
    pub fn chroma_bytes_suffice(&self) -> bool {
        self.chroma.filter.coeff_shift == 0 && self.chroma.wide8
    }
}

impl CdefPairSegment {
    /// Gathers the chroma pair as bytes for [`cdef_filter_segment`], when
    /// [`CdefUnitFilters::chroma_bytes_suffice`] admits its filter.
    pub fn gather_bytes(
        &mut self,
        u: &[u8],
        v: &[u8],
        width: usize,
        stride: usize,
        origin: (usize, usize),
    ) -> bool {
        let ranges = gather_pair::<u8, true>(
            u,
            v,
            width,
            stride,
            (&mut self.pad, &mut self.bytes),
            origin,
        );
        self.ranges = ranges.unwrap_or([u16::MAX; CDEF_SEGMENT_BLOCKS]);
        self.has_bytes = ranges.is_some();
        self.bytes_only = true;
        ranges.is_some()
    }

    #[inline]
    pub(super) fn padded_block_bytes(&self, block: usize) -> Option<&[u8; PAIR_PADDED_BYTES]> {
        if !self.bytes_only || !self.has_bytes || block >= CDEF_SEGMENT_BLOCKS {
            return None;
        }
        self.bytes.get(PAIR_BYTE_BLOCK * block..)?.first_chunk()
    }
}

#[allow(
    clippy::inline_always,
    reason = "keeps the CDEF byte pair dispatch in registers"
)]
#[inline(always)]
pub(super) fn filter_byte_pair<S: SimdToken, Q: QuadLanes<S>, T: ReconSample>(
    simd: S,
    quads: Q,
    bytes: &[u8; PAIR_PADDED_BYTES],
    range: u16,
    plan: &CdefFilterPlan,
    fill_flat: [bool; 3],
    planes: [(&mut [T], usize); 2],
) -> bool {
    const STRIDE: usize = LUMA_BYTE_STRIDE;
    const SPAN: usize = PAIR_BYTE_SPAN;
    const AREA: usize = PAIR_PADDED_BYTES;
    let [(u, u_stride), (v, v_stride)] = planes;
    if range == 0 {
        let value = |plane| u16::from(bytes[plane] ^ 0x80_u8);
        return (!fill_flat[1] || fill_rows(u, u_stride, 4, value(0)).is_some())
            && (!fill_flat[2] || fill_rows(v, v_stride, 4, value(1)).is_some());
    }
    if !plan.wide8 {
        return false;
    }
    let mut output = [0; CDEF_PAIR_OUTPUT];
    let inputs = (bytes, plan, &PAIR_BYTE_STARTS);
    macro_rules! run {
        ($mode:ident) => {
            cdef_by_families!(
                plan.filter,
                cdef_filter_rows_wide8::<S, Q, _, STRIDE, 4, 4, SPAN, AREA, $mode>(
                    simd,
                    quads,
                    inputs,
                    (&mut output, 8)
                ),
                None
            )
        };
    }
    let filtered = match plan.mode(i32::from(range)) {
        CDEF_LINEAR => run!(CDEF_LINEAR),
        CDEF_CLAMPED => run!(CDEF_CLAMPED),
        _ => run!(CDEF_GENERAL),
    };
    if filtered.is_none() {
        return false;
    }
    let (a, b) = simd.split_u8x32(u8x32::from_slice(simd, &output));
    let u_lanes = simd.unzip_low_u8x16(a, b).to_array();
    let v_lanes = simd.unzip_high_u8x16(a, b).to_array();
    if let (Some(u), Some(v)) = (T::u8_slice_mut(u), T::u8_slice_mut(v)) {
        store_quad(u_lanes, u, u_stride, Q::ROWS).is_some()
            && store_quad(v_lanes, v, v_stride, Q::ROWS).is_some()
    } else if let (Some(u), Some(v)) = (T::u16_slice_mut(u), T::u16_slice_mut(v)) {
        store_quad(u_lanes.map(u16::from), u, u_stride, Q::ROWS).is_some()
            && store_quad(v_lanes.map(u16::from), v, v_stride, Q::ROWS).is_some()
    } else {
        false
    }
}
