// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Experimental direct and pointer calls to the same AVX2 block worker.

use super::block::{BlockContext, filter_block, visit_blocks};
use super::*;
use std::sync::OnceLock;

type BlockKernel<T> = fn(Avx2, &mut BlockContext<'_, '_, T>, usize) -> bool;

#[derive(Clone, Copy)]
pub(super) struct Avx2Table {
    bytes: BlockKernel<u8>,
    words: BlockKernel<u16>,
}

#[derive(Clone, Copy)]
pub(super) enum BlockCall {
    Direct,
    Pointer(Avx2Table),
}

static CALL: OnceLock<Option<BlockCall>> = OnceLock::new();

pub(super) fn configure() {
    CALL.get_or_init(|| match std::env::var("SPLOT_CDEF_BLOCK_CALL").as_deref() {
        Ok("direct") => Some(BlockCall::Direct),
        Ok("pointer") => Some(BlockCall::Pointer(Avx2Table {
            bytes: avx2_block::<u8>,
            words: avx2_block::<u16>,
        })),
        _ => None,
    });
}

pub(super) fn selected() -> Option<BlockCall> {
    CALL.get().copied().flatten()
}

#[inline(never)]
fn avx2_block<T: ReconSample>(
    avx2: Avx2,
    context: &mut BlockContext<'_, '_, T>,
    block: usize,
) -> bool {
    avx2.vectorize(
        #[inline(always)]
        move || filter_block(avx2, InLane, context, block),
    )
}

#[inline]
fn cast_output<'a, T, O>(
    ((luma, stride), chroma): CdefSegmentOutput<'a, T>,
    cast: impl Fn(&'a mut [T]) -> Option<&'a mut [O]>,
) -> Option<CdefSegmentOutput<'a, O>> {
    let chroma = match chroma {
        Some((pair, [(u, us), (v, vs)])) => Some((pair, [(cast(u)?, us), (cast(v)?, vs)])),
        None => None,
    };
    Some(((cast(luma)?, stride), chroma))
}

// The concrete segment entry points keep the named call and kernel in the
// same codegen unit. Both modes pay the same once-per-segment entry cost.
macro_rules! avx2_segments {
    ($(($name:ident, $sample:ty, $member:ident)),+ $(,)?) => {$(
        #[inline(never)]
        fn $name(
            call: BlockCall,
            avx2: Avx2,
            segment: &CdefLumaSegment,
            unit: &CdefUnitFilters,
            blocks: [CdefSegmentBlock; CDEF_SEGMENT_BLOCKS],
            fill_flat: [bool; 3],
            output: CdefSegmentOutput<'_, $sample>,
        ) -> bool {
            match call {
                BlockCall::Direct => visit_blocks(
                    segment,
                    unit,
                    blocks,
                    fill_flat,
                    output,
                    #[inline(always)]
                    |context, block| avx2_block(avx2, context, block),
                ),
                BlockCall::Pointer(table) => {
                    let kernel = table.$member;
                    visit_blocks(
                        segment,
                        unit,
                        blocks,
                        fill_flat,
                        output,
                        #[inline(always)]
                        |context, block| kernel(avx2, context, block),
                    )
                }
            }
        }
    )+};
}

avx2_segments!((filter_bytes, u8, bytes), (filter_words, u16, words));

#[inline]
#[expect(clippy::too_many_arguments)]
pub(super) fn filter_segment<T: ReconSample>(
    call: BlockCall,
    avx2: Avx2,
    segment: &CdefLumaSegment,
    unit: &CdefUnitFilters,
    blocks: [CdefSegmentBlock; CDEF_SEGMENT_BLOCKS],
    fill_flat: [bool; 3],
    output: CdefSegmentOutput<'_, T>,
) -> bool {
    if T::MAX_VALUE == u16::from(u8::MAX) {
        cast_output(output, T::u8_slice_mut).is_some_and(|output| {
            filter_bytes(call, avx2, segment, unit, blocks, fill_flat, output)
        })
    } else {
        cast_output(output, T::u16_slice_mut).is_some_and(|output| {
            filter_words(call, avx2, segment, unit, blocks, fill_flat, output)
        })
    }
}
