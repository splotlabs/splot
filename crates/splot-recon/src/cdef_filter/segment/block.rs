// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Iteration and filtering for the four blocks of an interior segment.

use super::*;

#[inline(always)]
pub(super) fn segment_with<S: SimdToken, Q: QuadLanes<S>, T: ReconSample>(
    simd: S,
    quads: Q,
    segment: &CdefLumaSegment,
    unit: &CdefUnitFilters,
    blocks: [CdefSegmentBlock; CDEF_SEGMENT_BLOCKS],
    fill_flat: [bool; 3],
    output: CdefSegmentOutput<'_, T>,
) -> bool {
    visit_blocks(
        segment,
        unit,
        blocks,
        fill_flat,
        output,
        #[inline(always)]
        |context, block| filter_block(simd, quads, context, block),
    )
}

pub(super) struct BlockContext<'a, 'o, T> {
    segment: &'a CdefLumaSegment,
    unit: &'a CdefUnitFilters,
    blocks: [CdefSegmentBlock; CDEF_SEGMENT_BLOCKS],
    shift: u32,
    fill_flat: [bool; 3],
    output: CdefSegmentOutput<'o, T>,
}

#[expect(
    clippy::inline_always,
    reason = "keeps the selected block call in the segment loop"
)]
#[inline(always)]
pub(super) fn visit_blocks<T: ReconSample>(
    segment: &CdefLumaSegment,
    unit: &CdefUnitFilters,
    blocks: [CdefSegmentBlock; CDEF_SEGMENT_BLOCKS],
    fill_flat: [bool; 3],
    output: CdefSegmentOutput<'_, T>,
    mut run: impl FnMut(&mut BlockContext<'_, '_, T>, usize) -> bool,
) -> bool {
    let ((luma_out, luma_stride), chroma) = &output;
    const WIDTH: usize = 8 * CDEF_SEGMENT_BLOCKS;
    let fits = |len: usize, stride: usize, rows: usize, width: usize| {
        stride >= width
            && stride
                .checked_mul(rows - 1)
                .and_then(|span| span.checked_add(width))
                .is_some_and(|span| span <= len)
    };
    if !fits(luma_out.len(), *luma_stride, 8, WIDTH)
        || chroma.as_ref().is_some_and(|(_, planes)| {
            planes
                .iter()
                .any(|(rows, stride)| !fits(rows.len(), *stride, 4, WIDTH / 2))
        })
    {
        return false;
    }
    let mut context = BlockContext {
        segment,
        unit,
        blocks,
        shift: unit.luma[0].filter.coeff_shift,
        fill_flat,
        output,
    };
    let has_chroma = context.output.1.is_some();
    for (block, filters) in blocks.into_iter().enumerate() {
        if !filters.luma && !(has_chroma && filters.chroma) {
            continue;
        }
        if !run(&mut context, block) {
            return false;
        }
    }
    true
}

#[expect(
    clippy::inline_always,
    reason = "keeps the full block in its selected SIMD scope"
)]
#[inline(always)]
pub(super) fn filter_block<S: SimdToken, Q: QuadLanes<S>, T: ReconSample>(
    simd: S,
    quads: Q,
    context: &mut BlockContext<'_, '_, T>,
    block: usize,
) -> bool {
    let BlockContext {
        segment,
        unit,
        blocks,
        shift,
        fill_flat,
        output: ((luma_out, luma_stride), chroma),
    } = context;
    let filters = blocks[block];
    let (shift, fill_flat, luma_stride) = (*shift, *fill_flat, *luma_stride);
    let (Some(luma), Some([least, greatest])) = (segment.scratch(block), segment.range(block))
    else {
        return false;
    };
    let packed = segment.padded_block_bytes(block);
    let use_range = filters.luma && unit.luma_active;
    let luma_flat = use_range && least == greatest;
    let (y_dir, var) = if !unit.directions || luma_flat {
        (0, 0)
    } else if segment.has_pad {
        quads.direction(
            simd,
            cdef_direction_input::<_, CDEF_SEGMENT_STRIDE, _>(simd, luma, shift),
        )
    } else if let Some(bytes) = packed {
        quads.direction(simd, direction_byte_rows(simd, bytes))
    } else {
        return false;
    };
    let [y_plan, uv_plan] = unit.block(y_dir, var);
    if filters.luma && (y_plan.filter.pri_str != 0 || y_plan.filter.sec_str != 0) {
        let Some(out) = luma_out.get_mut(8 * block..) else {
            return false;
        };
        let written = if luma_flat {
            !fill_flat[0] || fill_rows(out, luma_stride, 8, least).is_some()
        } else if segment.has_pad {
            luma_segment_with(
                simd,
                quads,
                (luma, segment.block_bytes(block)),
                Some([least, greatest]),
                &y_plan,
                (out, luma_stride),
            )
        } else {
            let range = i32::from(greatest) - i32::from(least);
            luma_wide8_with(simd, quads, packed, range, &y_plan, (out, luma_stride))
        };
        if !written {
            return false;
        }
    }
    let Some((pair, [(u, u_stride), (v, v_stride)])) = chroma.as_mut().filter(|_| filters.chroma)
    else {
        return true;
    };
    let (u_stride, v_stride) = (*u_stride, *v_stride);
    if uv_plan.filter.pri_str == 0 && uv_plan.filter.sec_str == 0 {
        return true;
    }
    let Some(range) = pair.range(block) else {
        return false;
    };
    let (Some(u), Some(v)) = (u.get_mut(4 * block..), v.get_mut(4 * block..)) else {
        return false;
    };
    if let Some(bytes) = pair.padded_block_bytes(block) {
        filter_byte_pair(
            simd,
            quads,
            bytes,
            range,
            &uv_plan,
            fill_flat,
            [(u, u_stride), (v, v_stride)],
        )
    } else {
        let Some(pad) = pair.block(block) else {
            return false;
        };
        if range == 0 {
            (!fill_flat[1] || fill_rows(u, u_stride, 4, pad[0]).is_some())
                && (!fill_flat[2] || fill_rows(v, v_stride, 4, pad[1]).is_some())
        } else {
            let mut output = [0u16; CDEF_PAIR_OUTPUT];
            pair_segment_with(
                simd,
                quads,
                (pad, pair.block_bytes(block)),
                range,
                &uv_plan,
                &mut output,
            ) && publish_pair(simd, &output, [(u, u_stride), (v, v_stride)]).is_some()
        }
    }
}
