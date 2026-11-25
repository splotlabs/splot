// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Experimental direct and pointer calls to the same AVX2 block worker.

use super::block::{BlockWork, filter_block, visit_blocks};
use super::*;
use std::sync::OnceLock;

type BlockKernel<T> = for<'a, 'o> fn(Avx2, BlockWork<'a, 'o, T>) -> bool;

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
fn avx2_block<T: ReconSample>(avx2: Avx2, job: BlockWork<'_, '_, T>) -> bool {
    avx2.vectorize(
        #[inline(always)]
        || filter_block(avx2, InLane, job),
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

#[expect(
    clippy::inline_always,
    reason = "branches on the startup selection outside the block loop"
)]
#[expect(clippy::too_many_arguments)]
#[inline(always)]
pub(super) fn filter_segment<T: ReconSample>(
    call: BlockCall,
    avx2: Avx2,
    segment: &CdefLumaSegment,
    unit: &CdefUnitFilters,
    blocks: [CdefSegmentBlock; CDEF_SEGMENT_BLOCKS],
    fill_flat: [bool; 3],
    output: CdefSegmentOutput<'_, T>,
) -> bool {
    macro_rules! run {
        ($cast:expr, $member:ident) => {
            cast_output(output, $cast).is_some_and(|output| match call {
                BlockCall::Direct => visit_blocks(
                    segment,
                    unit,
                    blocks,
                    fill_flat,
                    output,
                    #[inline(always)]
                    |job| avx2_block(avx2, job),
                ),
                BlockCall::Pointer(table) => {
                    let block = table.$member;
                    visit_blocks(
                        segment,
                        unit,
                        blocks,
                        fill_flat,
                        output,
                        #[inline(always)]
                        |job| block(avx2, job),
                    )
                }
            })
        };
    }
    if T::MAX_VALUE == u16::from(u8::MAX) {
        run!(T::u8_slice_mut, bytes)
    } else {
        run!(T::u16_slice_mut, words)
    }
}
