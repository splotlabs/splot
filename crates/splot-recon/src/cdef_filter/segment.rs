// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Row segments of [`CDEF_SEGMENT_BLOCKS`] interior CDEF blocks, gathered once
//! per plane, and the § 7.18.3 filters that read them.
//!
//! A segment's scratch and the sample range of each block's window are
//! private and only its gather writes them, so a range always bounds the
//! samples that it describes. The filters select their kernel from that
//! range.

use super::*;

const TAP_REACH: usize = 2;
const LUMA_SEGMENT_AREA: usize = 8 * (CDEF_SEGMENT_BLOCKS - 1) + CDEF_SEGMENT_BLOCK_AREA;
const PAIR_SEGMENT_AREA: usize = 8 * (CDEF_SEGMENT_BLOCKS - 1) + CDEF_PAIR_SEGMENT_BLOCK_AREA;
const PAIR_ROWS: usize = 4 + 2 * TAP_REACH;
const PAIR_SPAN: usize = CDEF_PAIR_SEGMENT_STRIDE / 2;

/// The tap rows of one luma row segment of [`CDEF_SEGMENT_BLOCKS`] 8x8
/// blocks at [`CDEF_SEGMENT_STRIDE`] lanes per row, and the least and
/// greatest sample of each block's 12x12 window.
pub struct CdefLumaSegment {
    pad: [u16; LUMA_SEGMENT_AREA],
    ranges: [[u16; 2]; CDEF_SEGMENT_BLOCKS],
}

impl Default for CdefLumaSegment {
    fn default() -> Self {
        Self {
            pad: [0; LUMA_SEGMENT_AREA],
            ranges: [[0; 2]; CDEF_SEGMENT_BLOCKS],
        }
    }
}

impl CdefLumaSegment {
    /// Copies the tap rows of the segment whose first block starts at column
    /// `origin.0`, row `origin.1` of `samples` (rows `stride` apart, `width`
    /// samples wide) and records each block's window range. Returns `false`,
    /// with every range set to `[0, u16::MAX]`, when the tap reach leaves
    /// the rows.
    pub fn gather(
        &mut self,
        samples: &[u16],
        width: usize,
        stride: usize,
        origin: (usize, usize),
    ) -> bool {
        let ranges = gather_luma(samples, width, stride, &mut self.pad, origin);
        self.ranges = ranges.unwrap_or([[0, u16::MAX]; CDEF_SEGMENT_BLOCKS]);
        ranges.is_some()
    }

    /// The scratch from the first tap-reach lane of block `block` on, or
    /// `None` when `block` is not below [`CDEF_SEGMENT_BLOCKS`].
    #[inline]
    pub fn block(&self, block: usize) -> Option<&[u16; CDEF_SEGMENT_BLOCK_AREA]> {
        if block >= CDEF_SEGMENT_BLOCKS {
            return None;
        }
        self.pad.get(8 * block..)?.first_chunk()
    }

    /// The least and greatest sample of block `block`'s window.
    #[inline]
    pub fn range(&self, block: usize) -> Option<[u16; 2]> {
        self.ranges.get(block).copied()
    }
}

/// The tap rows of one row segment of both chroma planes, interleaved as
/// for [`cdef_filter_block_chroma_pair`] at [`CDEF_PAIR_SEGMENT_STRIDE`]
/// lanes per row, and for each 4x4 block the larger of its two planes'
/// window ranges (greatest minus least sample).
pub struct CdefPairSegment {
    pad: [u16; PAIR_SEGMENT_AREA],
    ranges: [u16; CDEF_SEGMENT_BLOCKS],
}

impl Default for CdefPairSegment {
    fn default() -> Self {
        Self {
            pad: [0; PAIR_SEGMENT_AREA],
            ranges: [0; CDEF_SEGMENT_BLOCKS],
        }
    }
}

impl CdefPairSegment {
    /// [`CdefLumaSegment::gather`] for the two chroma planes `u` and `v`,
    /// which share `width` and `stride`. A failed gather sets every range to
    /// `u16::MAX`.
    pub fn gather(
        &mut self,
        u: &[u16],
        v: &[u16],
        width: usize,
        stride: usize,
        origin: (usize, usize),
    ) -> bool {
        let ranges = gather_pair(u, v, width, stride, &mut self.pad, origin);
        self.ranges = ranges.unwrap_or([u16::MAX; CDEF_SEGMENT_BLOCKS]);
        ranges.is_some()
    }

    /// [`CdefLumaSegment::block`] for the interleaved chroma pair.
    #[inline]
    pub fn block(&self, block: usize) -> Option<&[u16; CDEF_PAIR_SEGMENT_BLOCK_AREA]> {
        if block >= CDEF_SEGMENT_BLOCKS {
            return None;
        }
        self.pad.get(8 * block..)?.first_chunk()
    }

    /// The larger of block `block`'s two plane window ranges.
    #[inline]
    pub fn range(&self, block: usize) -> Option<u16> {
        self.ranges.get(block).copied()
    }
}

/// The `rows` rows of `span` samples from column `x0 - TAP_REACH`, row
/// `y0 - TAP_REACH` on, as one slice whose row `r` starts at `r * stride`,
/// provided they lie inside `width` and `samples`.
fn reach_rows(
    samples: &[u16],
    width: usize,
    stride: usize,
    (x0, y0): (usize, usize),
    (span, rows): (usize, usize),
) -> Option<&[u16]> {
    let left = x0.checked_sub(TAP_REACH)?;
    if left.checked_add(span)? > width {
        return None;
    }
    let first = y0
        .checked_sub(TAP_REACH)?
        .checked_mul(stride)?
        .checked_add(left)?;
    let len = (rows - 1).checked_mul(stride)?.checked_add(span)?;
    samples.get(first..)?.get(..len)
}

/// Copies the luma segment's tap rows into `pad` and returns the least and
/// greatest sample of each block's 12x12 window.
fn gather_luma(
    samples: &[u16],
    width: usize,
    stride: usize,
    pad: &mut [u16; LUMA_SEGMENT_AREA],
    origin: (usize, usize),
) -> Option<[[u16; 2]; CDEF_SEGMENT_BLOCKS]> {
    const STARTS: [usize; 5] = [0, 8, 16, 24, CDEF_SEGMENT_STRIDE - 8];
    const { assert!(CDEF_SEGMENT_BLOCKS == 4 && CDEF_SEGMENT_STRIDE == 36) };
    let rows = reach_rows(samples, width, stride, origin, (CDEF_SEGMENT_STRIDE, 12))?;
    let mut min = [Simd::<u16, 8>::splat(u16::MAX); 5];
    let mut max = [Simd::<u16, 8>::splat(0); 5];
    for row in 0..12 {
        let src = rows
            .get(row * stride..)?
            .first_chunk::<CDEF_SEGMENT_STRIDE>()?;
        let dst = pad
            .get_mut(row * CDEF_SEGMENT_STRIDE..)?
            .first_chunk_mut::<CDEF_SEGMENT_STRIDE>()?;
        for (chunk, start) in STARTS.into_iter().enumerate() {
            let lanes = Simd::<u16, 8>::from_slice(&src[start..start + 8]);
            lanes.copy_to_slice(&mut dst[start..start + 8]);
            min[chunk] = min[chunk].simd_min(lanes);
            max[chunk] = max[chunk].simd_max(lanes);
        }
    }
    let tail = |chunks: &[Simd<u16, 8>; 5], block: usize| {
        if block + 1 < CDEF_SEGMENT_BLOCKS {
            simd_swizzle!(chunks[block + 1], [0, 1, 2, 3, 0, 1, 2, 3])
        } else {
            chunks[4]
        }
    };
    Some(core::array::from_fn(|block| {
        [
            min[block].simd_min(tail(&min, block)).reduce_min(),
            max[block].simd_max(tail(&max, block)).reduce_max(),
        ]
    }))
}

/// Interleaves the chroma segment's tap rows, `PAIR_SPAN` samples of each
/// plane, into `pad` and returns for each block the larger of its two
/// planes' window ranges. Block `b` covers plane columns `4b..4b + 8`: the
/// three 8-column chunks of a row hold blocks 0, 2 and 3, and block 1 is the
/// upper half of chunk 0 and the lower half of chunk 1.
fn gather_pair(
    u_samples: &[u16],
    v_samples: &[u16],
    width: usize,
    stride: usize,
    pad: &mut [u16; PAIR_SEGMENT_AREA],
    origin: (usize, usize),
) -> Option<[u16; CDEF_SEGMENT_BLOCKS]> {
    const { assert!(CDEF_SEGMENT_BLOCKS == 4 && PAIR_SPAN == 20) };
    let u_rows = reach_rows(u_samples, width, stride, origin, (PAIR_SPAN, PAIR_ROWS))?;
    let v_rows = reach_rows(v_samples, width, stride, origin, (PAIR_SPAN, PAIR_ROWS))?;
    let mut min = [[Simd::<u16, 8>::splat(u16::MAX); 3]; 2];
    let mut max = [[Simd::<u16, 8>::splat(0); 3]; 2];
    for row in 0..PAIR_ROWS {
        let u_row = u_rows.get(row * stride..)?.first_chunk::<PAIR_SPAN>()?;
        let v_row = v_rows.get(row * stride..)?.first_chunk::<PAIR_SPAN>()?;
        let lanes = pad
            .get_mut(row * CDEF_PAIR_SEGMENT_STRIDE..)?
            .first_chunk_mut::<CDEF_PAIR_SEGMENT_STRIDE>()?;
        for (chunk, start) in [0, 8, PAIR_SPAN - 8].into_iter().enumerate() {
            let planes = [
                Simd::<u16, 8>::from_slice(&u_row[start..start + 8]),
                Simd::from_slice(&v_row[start..start + 8]),
            ];
            for plane in 0..2 {
                min[plane][chunk] = min[plane][chunk].simd_min(planes[plane]);
                max[plane][chunk] = max[plane][chunk].simd_max(planes[plane]);
            }
            let (low, high) = planes[0].interleave(planes[1]);
            low.copy_to_slice(&mut lanes[2 * start..2 * start + 8]);
            high.copy_to_slice(&mut lanes[2 * start + 8..2 * start + 16]);
        }
    }
    let blocks = |chunks: [Simd<u16, 8>; 3]| {
        [
            chunks[0],
            simd_swizzle!(chunks[0], chunks[1], [4, 5, 6, 7, 8, 9, 10, 11]),
            chunks[1],
            chunks[2],
        ]
    };
    let reduce = |[b0, b1, b2, b3]: [Simd<u16, 8>; 4],
                  pick: fn(Simd<u16, 8>, Simd<u16, 8>) -> Simd<u16, 8>| {
        let halves = |a: Simd<u16, 8>, b: Simd<u16, 8>| {
            let (even, odd) = a.deinterleave(b);
            pick(even, odd)
        };
        let quarter = halves(halves(b0, b1), halves(b2, b3));
        halves(quarter, quarter)
    };
    let ranges = [0, 1].map(|plane| {
        reduce(blocks(max[plane]), SimdOrd::simd_max)
            - reduce(blocks(min[plane]), SimdOrd::simd_min)
    });
    let range = ranges[0].simd_max(ranges[1]).to_array();
    Some([range[0], range[1], range[2], range[3]])
}

/// [`cdef_filter_block_chroma_pair`] for block `block` of `segment`. The
/// block's window range selects the kernel: the linear kernel, which skips
/// `constrain`, when `cdef_window_linear` admits the range; its `CLAMP`
/// mode, which clips each tap to the centre -/+ strength, when
/// `cdef_window_clamped` admits it; else the general kernel. The taps of
/// one lane all come from one plane, so the larger plane range bounds every
/// tap difference. Returns `false` when `block` is not below
/// [`CDEF_SEGMENT_BLOCKS`].
pub fn cdef_filter_block_chroma_pair_segment(
    segment: &CdefPairSegment,
    block: usize,
    filter: &CdefBlockFilter,
    out: &mut [u16; CDEF_PAIR_OUTPUT],
) -> bool {
    const SPAN: usize = 3 * CDEF_PAIR_SEGMENT_STRIDE + 8;
    const AREA: usize = CDEF_PAIR_SEGMENT_BLOCK_AREA;
    let (Some(pad), Some(range)) = (segment.block(block), segment.range(block)) else {
        return false;
    };
    let starts = &CDEF_PAIR_SEGMENT_TAP_STARTS;
    macro_rules! clamped {
        ($clamp:literal) => {
            cdef_by_families!(
                filter,
                cdef_filter_rows_clamped::<CDEF_PAIR_SEGMENT_STRIDE, 4, 4, SPAN, AREA, $clamp>(
                    pad, 4, filter, starts, out, 8
                ),
                cdef_copy_centre_rows::<8, CDEF_PAIR_SEGMENT_STRIDE, 4, 4, AREA>(pad, 4, out, 8)
            )
            .is_some()
        };
    }
    let range = i32::from(range);
    if cdef_window_linear(filter, range) {
        return clamped!(false);
    }
    if cdef_window_clamped(filter, range) {
        return clamped!(true);
    }
    cdef_filter_block_rows::<8, 16, false, CDEF_PAIR_SEGMENT_STRIDE, 4, 4, SPAN, AREA>(
        pad, 4, filter, starts, out, 8,
    )
    .is_some()
}

/// AV2 § 7.18.3 CDEF over interior 8x8 luma block `block` of `segment`.
/// With `use_range`, the block's window range selects the kernel: the
/// linear kernel, which skips `constrain`, when `cdef_window_linear` admits
/// the range; its `CLAMP` mode, which clips each tap to the centre -/+
/// strength, when `cdef_window_clamped` admits it; else `u8` lanes when the
/// range is below 256 and the filter fits them. Other blocks take the
/// kernel of [`cdef_filter_block_interior_to`]. Returns `false` when `block`
/// is not below [`CDEF_SEGMENT_BLOCKS`] or `out` cannot hold the block at
/// `out_stride`.
pub fn cdef_filter_block_segment(
    segment: &CdefLumaSegment,
    block: usize,
    filter: &CdefBlockFilter,
    use_range: bool,
    out: &mut [u16],
    out_stride: usize,
) -> bool {
    const SPAN: usize = 7 * CDEF_SEGMENT_STRIDE + 8;
    const AREA: usize = CDEF_SEGMENT_BLOCK_AREA;
    let (Some(pad), Some([base, max])) = (segment.block(block), segment.range(block)) else {
        return false;
    };
    let starts = &CDEF_SEGMENT_TAP_STARTS;
    if !use_range {
        return cdef_filter_block_rows::<8, 16, false, CDEF_SEGMENT_STRIDE, 2, 8, SPAN, AREA>(
            pad, 8, filter, starts, out, out_stride,
        )
        .is_some();
    }
    let range = i32::from(max) - i32::from(base);
    macro_rules! copy {
        () => {
            cdef_copy_centre_rows::<8, CDEF_SEGMENT_STRIDE, 2, 8, AREA>(pad, 8, out, out_stride)
        };
    }
    macro_rules! clamped {
        ($clamp:literal) => {
            cdef_by_families!(
                filter,
                cdef_filter_rows_clamped::<CDEF_SEGMENT_STRIDE, 2, 8, SPAN, AREA, $clamp>(
                    pad, 8, filter, starts, out, out_stride
                ),
                copy!()
            )
            .is_some()
        };
    }
    if cdef_window_linear(filter, range) {
        return clamped!(false);
    }
    if cdef_window_clamped(filter, range) {
        return clamped!(true);
    }
    if range <= 255 && cdef_u8_lanes_fit(filter) {
        return cdef_by_families!(
            filter,
            cdef_filter_rows_8bit::<CDEF_SEGMENT_STRIDE, 2, 8, SPAN, AREA>(
                pad, 8, filter, starts, out, out_stride, base
            ),
            copy!()
        )
        .is_some();
    }
    cdef_filter_block_rows::<8, 16, false, CDEF_SEGMENT_STRIDE, 2, 8, SPAN, AREA>(
        pad, 8, filter, starts, out, out_stride,
    )
    .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(len: usize, seed: u32, value: impl Fn(u32) -> u16) -> Vec<u16> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                value(state)
            })
            .collect()
    }

    fn window_bounds(window: impl Iterator<Item = u16>) -> [u16; 2] {
        window.fold([u16::MAX, 0], |[min, max], sample| {
            [min.min(sample), max.max(sample)]
        })
    }

    #[test]
    fn luma_gather_records_each_block_window_range() {
        let (width, stride) = (44, 48);
        let samples = noise(stride * 14, 0x2468_ace1, |state| (state >> 22) as u16);
        let mut segment = CdefLumaSegment::default();
        assert!(segment.gather(&samples, width, stride, (4, 3)));
        for block in 0..CDEF_SEGMENT_BLOCKS {
            let window = (1..13).flat_map(|y| {
                let start = y * stride + 2 + 8 * block;
                samples[start..start + 12].iter().copied()
            });
            assert_eq!(
                segment.range(block),
                Some(window_bounds(window)),
                "block {block}"
            );
        }
        for row in 0..12 {
            let start = (row + 1) * stride + 2;
            assert_eq!(
                &segment.pad[row * CDEF_SEGMENT_STRIDE..(row + 1) * CDEF_SEGMENT_STRIDE],
                &samples[start..start + CDEF_SEGMENT_STRIDE]
            );
        }
    }

    /// The plane of squares `x^2` makes each block's range depend on where its
    /// window starts, and either plane may hold the larger range.
    #[test]
    fn pair_gather_records_each_block_plane_range() {
        let stride = 24;
        let random = noise(stride * 10, 0x1357_9bdf, |state| {
            300 + ((state >> 13) % 40) as u16
        });
        let square: Vec<u16> = (0..stride * 10)
            .map(|index| 300 + ((index % stride) * (index % stride)) as u16)
            .collect();
        for (u, v) in [(&random, &square), (&square, &random)] {
            let mut segment = CdefPairSegment::default();
            assert!(segment.gather(u, v, stride, stride, (3, 3)));
            for block in 0..CDEF_SEGMENT_BLOCKS {
                let plane_range = |samples: &[u16]| {
                    let [min, max] = window_bounds((1..9).flat_map(|y| {
                        let start = y * stride + 1 + 4 * block;
                        samples[start..start + 8].iter().copied()
                    }));
                    max - min
                };
                assert_eq!(
                    segment.range(block),
                    Some(plane_range(u).max(plane_range(v))),
                    "block {block}"
                );
            }
            for row in 0..8 {
                for col in 0..PAIR_SPAN {
                    let start = (row + 1) * stride + 1 + col;
                    let lanes = &segment.pad[row * CDEF_PAIR_SEGMENT_STRIDE + 2 * col..][..2];
                    assert_eq!(lanes, [u[start], v[start]]);
                }
            }
        }
    }

    #[test]
    fn gather_outside_the_rows_fails_with_full_ranges() {
        let (width, stride) = (44, 48);
        let samples = vec![7u16; stride * 14];
        let mut luma = CdefLumaSegment::default();
        let mut pair = CdefPairSegment::default();
        for (luma_width, origin, len) in [
            (width, (1, 3), samples.len()),
            (width, (4, 1), samples.len()),
            (width, (11, 3), samples.len()),
            (width, (4, 3), 405),
            (width, (4, usize::MAX / 8), samples.len()),
            (usize::MAX, (usize::MAX, 3), samples.len()),
        ] {
            assert!(luma.gather(&samples, width, stride, (4, 3)));
            assert_eq!(luma.range(0), Some([7, 7]));
            assert!(!luma.gather(&samples[..len], luma_width, stride, origin));
            assert_eq!(luma.range(3), Some([0, u16::MAX]), "{origin:?}");
            assert!(pair.gather(&samples, &samples, 24, stride, (3, 3)));
            assert_eq!(pair.range(0), Some(0));
            assert!(!pair.gather(&samples, &samples[..len], 24, stride, origin));
            assert_eq!(pair.range(3), Some(u16::MAX), "{origin:?}");
        }
        assert!(!luma.gather(&samples, width, usize::MAX / 4, (4, 3)));
        assert!(!pair.gather(&samples, &samples, 24, usize::MAX / 4, (3, 3)));
    }

    #[test]
    fn blocks_past_the_segment_are_rejected() {
        let filter = CdefBlockFilter {
            pri_str: 4,
            sec_str: 2,
            damping: 6,
            dir: 1,
            coeff_shift: 2,
        };
        let (luma, pair) = (CdefLumaSegment::default(), CdefPairSegment::default());
        let (mut out, mut pair_out) = ([0u16; 64], [0u16; CDEF_PAIR_OUTPUT]);
        assert!(cdef_filter_block_segment(
            &luma,
            CDEF_SEGMENT_BLOCKS - 1,
            &filter,
            true,
            &mut out,
            8
        ));
        for block in [CDEF_SEGMENT_BLOCKS, 1 << 61, usize::MAX] {
            assert!(luma.block(block).is_none() && luma.range(block).is_none());
            assert!(pair.block(block).is_none() && pair.range(block).is_none());
            assert!(!cdef_filter_block_segment(
                &luma, block, &filter, true, &mut out, 8
            ));
            assert!(!cdef_filter_block_chroma_pair_segment(
                &pair,
                block,
                &filter,
                &mut pair_out
            ));
        }
    }
}
