// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;

use super::{Mv, unsupported_compound_at};
use crate::Result;
use crate::bitstream::tile_payload::{TileCdfSelector, TileCdfSubset};

const COMPOUND_MODE_NEAR_NEARMV: u8 = 0;
const COMPOUND_MODE_NEAR_NEWMV: u8 = 1;
const COMPOUND_MODE_NEW_NEARMV: u8 = 2;
const COMPOUND_MODE_GLOBAL_GLOBALMV: u8 = 3;
const COMPOUND_MODE_SAME_REF_NEW_NEWMV: u8 = 3;
const COMPOUND_MODE_NEW_NEWMV: u8 = 4;
const RANKED_REF0_TO_PRUNE: usize = 3;
const MAX_REFS_PER_FRAME: usize = 7;
const SPEC_READ_REF_FRAMES: &str = "5.20.7.10";
const SPEC_INTER_BLOCK_MODE_INFO: &str = "5.20.7.6";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CompoundParseInput {
    pub(crate) num_total_refs: usize,
    pub(crate) num_same_ref_compound: u8,
    pub(crate) ref_contexts: [usize; MAX_REFS_PER_FRAME],
    pub(crate) ref_distance_nonnegative: [bool; MAX_REFS_PER_FRAME],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompoundYMode {
    NearNear,
    NearNew,
    NewNear,
    GlobalGlobal,
    JointNew,
    NewNew,
}

impl CompoundYMode {
    pub(crate) const fn has_newmv(self) -> bool {
        match self {
            Self::NearNear | Self::GlobalGlobal => false,
            Self::NearNew | Self::NewNear | Self::JointNew | Self::NewNew => true,
        }
    }

    pub(crate) const fn has_nearmv(self) -> bool {
        match self {
            Self::NearNear | Self::NearNew | Self::NewNear => true,
            Self::GlobalGlobal | Self::JointNew | Self::NewNew => false,
        }
    }

    pub(crate) const fn reads_drl_idx(self) -> bool {
        self.has_newmv() || self.has_nearmv()
    }

    pub(crate) const fn has_second_drl(self) -> bool {
        match self {
            Self::NearNear | Self::NearNew => true,
            Self::NewNear | Self::GlobalGlobal | Self::JointNew | Self::NewNew => false,
        }
    }

    pub(crate) const fn mvd_sign_derivation_threshold(self) -> usize {
        match self {
            Self::NearNear
            | Self::NearNew
            | Self::NewNear
            | Self::GlobalGlobal
            | Self::JointNew => 1,
            Self::NewNew => 4,
        }
    }

    pub(crate) const fn use_amvd_index(self, use_optflow: bool) -> Option<usize> {
        match (self, use_optflow) {
            (Self::NearNear | Self::GlobalGlobal, _) => None,
            (Self::NearNew, false) => Some(0),
            (Self::NearNew, true) => Some(2),
            (Self::NewNear, false) => Some(1),
            (Self::NewNear, true) => Some(3),
            (Self::JointNew, false) => Some(5),
            (Self::JointNew, true) => Some(6),
            (Self::NewNew, false) => Some(7),
            (Self::NewNew, true) => Some(8),
        }
    }

    pub(crate) const fn list0_is_newmv(self) -> bool {
        match self {
            Self::NearNear | Self::NearNew | Self::GlobalGlobal => false,
            Self::NewNear | Self::JointNew | Self::NewNew => true,
        }
    }

    pub(crate) const fn list1_is_newmv(self) -> bool {
        match self {
            Self::NearNear | Self::NewNear | Self::GlobalGlobal => false,
            Self::NearNew | Self::JointNew | Self::NewNew => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CompoundBlockSyntax {
    pub(crate) y_mode: CompoundYMode,
    pub(crate) use_optflow: bool,
    pub(crate) ref_frame0: i8,
    pub(crate) ref_frame1: i8,
    pub(crate) mv0: Mv,
    pub(crate) mv1: Mv,
}

pub(crate) fn read_compound_reference_pair(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CompoundParseInput,
    tile_offset: ByteOffset,
) -> Result<(i8, i8)> {
    let mut read_symbol = |selector| {
        cdfs.read_block_symbol_trace(selector, symbols)
            .map(splot_core::symbol::Symbol::get)
            .map_err(|_| compound_symbol_read_error(tile_offset))
    };

    let n_refs = input.num_total_refs;
    let same_ref_compound = usize::from(input.num_same_ref_compound).min(n_refs);
    let mut ref_frames = [n_refs.saturating_sub(1); 2];
    let mut n_bits = 0usize;
    let mut may_have_same_ref_compound = same_ref_compound > 0;
    let mut ref_idx = 0usize;

    while (compound_ref_loop_has_more(ref_idx, n_refs, n_bits) || may_have_same_ref_compound)
        && n_bits < 2
    {
        let implicit_ref0 = n_bits == 0
            && (ref_idx >= RANKED_REF0_TO_PRUNE - 1
                || ref_idx >= n_refs.saturating_sub(2) && ref_idx + 1 >= same_ref_compound);
        let comp_ref = if implicit_ref0 {
            1
        } else if n_bits == 0 {
            read_symbol(TileCdfSelector::CompRef0 {
                ctx: *input.ref_contexts.get(ref_idx).ok_or_else(|| {
                    compound_missing!(
                        "compound_missing_ref0_context",
                        tile_offset,
                        "inter.compound.ref_context",
                        SPEC_READ_REF_FRAMES
                    )
                })?,
                ref_idx,
            })?
        } else {
            let first_ref = ref_frames[0];
            let bit_type = compound_ref_bit_type(input, first_ref, ref_idx, tile_offset)?;
            read_symbol(TileCdfSelector::CompRef1 {
                ctx: *input.ref_contexts.get(ref_idx).ok_or_else(|| {
                    compound_missing!(
                        "compound_missing_ref1_context",
                        tile_offset,
                        "inter.compound.ref_context",
                        SPEC_READ_REF_FRAMES
                    )
                })?,
                bit_type,
                ref_idx,
            })?
        };

        if comp_ref != 0 {
            ref_frames[n_bits] = ref_idx;
            n_bits += 1;
        }

        if ref_idx < same_ref_compound && may_have_same_ref_compound {
            may_have_same_ref_compound = comp_ref == 0 && ref_idx + 1 < same_ref_compound;
            if comp_ref == 0 {
                ref_idx += 1;
            }
        } else {
            may_have_same_ref_compound = false;
            ref_idx += 1;
        }
    }

    if n_bits < 2 {
        ref_frames[1] = n_refs.saturating_sub(1);
    }
    if n_bits < 1 {
        ref_frames[0] = if same_ref_compound > 0 && n_refs.saturating_sub(1) < same_ref_compound {
            n_refs.saturating_sub(1)
        } else {
            n_refs.saturating_sub(2)
        };
    }

    let ref0 = i8::try_from(ref_frames[0]).map_err(|_| {
        compound_cap!(
            "compound_ref0_out_of_range",
            tile_offset,
            "inter.compound.ref_frame0",
            SPEC_READ_REF_FRAMES
        )
    })?;
    let ref1 = i8::try_from(ref_frames[1]).map_err(|_| {
        compound_cap!(
            "compound_ref1_out_of_range",
            tile_offset,
            "inter.compound.ref_frame1",
            SPEC_READ_REF_FRAMES
        )
    })?;
    Ok((ref0, ref1))
}

pub(crate) fn read_compound_mode_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    pair: (i8, i8),
    new_mv_context: usize,
    is_joint_ctx: usize,
    tile_offset: ByteOffset,
) -> Result<CompoundBlockSyntax> {
    let mut read_symbol = |selector| {
        cdfs.read_block_symbol_trace(selector, symbols)
            .map(splot_core::symbol::Symbol::get)
            .map_err(|_| compound_symbol_read_error(tile_offset))
    };

    if pair.0 == pair.1 {
        let compound_mode = read_symbol(TileCdfSelector::CompoundModeSameRefs {
            ctx: new_mv_context,
        })?;
        let y_mode = match compound_mode {
            COMPOUND_MODE_NEAR_NEARMV => CompoundYMode::NearNear,
            1 => CompoundYMode::NearNew,
            COMPOUND_MODE_SAME_REF_NEW_NEWMV => CompoundYMode::NewNew,
            _ => {
                return Err(compound_cap!(
                    "compound_block_unsupported_same_ref_mode",
                    tile_offset,
                    "inter.compound.same_ref_mode not in {NEAR_NEARMV, NEAR_NEWMV, NEW_NEWMV}",
                    SPEC_INTER_BLOCK_MODE_INFO
                ));
            }
        };
        return Ok(CompoundBlockSyntax {
            y_mode,
            use_optflow: false,
            ref_frame0: pair.0,
            ref_frame1: pair.1,
            mv0: Mv::ZERO,
            mv1: Mv::ZERO,
        });
    }

    let is_joint = read_symbol(TileCdfSelector::IsJoint { ctx: is_joint_ctx })?;
    if is_joint != 0 {
        return Ok(CompoundBlockSyntax {
            y_mode: CompoundYMode::JointNew,
            use_optflow: false,
            ref_frame0: pair.0,
            ref_frame1: pair.1,
            mv0: Mv::ZERO,
            mv1: Mv::ZERO,
        });
    }

    let compound_mode = read_symbol(TileCdfSelector::CompoundModeNonJoint {
        ctx: new_mv_context,
    })?;
    let y_mode = match compound_mode {
        COMPOUND_MODE_NEAR_NEARMV => CompoundYMode::NearNear,
        COMPOUND_MODE_NEAR_NEWMV => CompoundYMode::NearNew,
        COMPOUND_MODE_NEW_NEARMV => CompoundYMode::NewNear,
        COMPOUND_MODE_NEW_NEWMV => CompoundYMode::NewNew,
        COMPOUND_MODE_GLOBAL_GLOBALMV => CompoundYMode::GlobalGlobal,
        _ => return Err(compound_symbol_read_error(tile_offset)),
    };

    Ok(CompoundBlockSyntax {
        y_mode,
        use_optflow: false,
        ref_frame0: pair.0,
        ref_frame1: pair.1,
        mv0: Mv::ZERO,
        mv1: Mv::ZERO,
    })
}

fn compound_ref_loop_has_more(ref_idx: usize, n_refs: usize, n_bits: usize) -> bool {
    n_refs
        .checked_add(n_bits)
        .and_then(|limit| limit.checked_sub(2))
        .is_some_and(|limit| ref_idx < limit)
}

fn compound_ref_bit_type(
    input: CompoundParseInput,
    first_ref: usize,
    second_ref: usize,
    tile_offset: ByteOffset,
) -> Result<usize> {
    let first_nonnegative = *input
        .ref_distance_nonnegative
        .get(first_ref)
        .ok_or_else(|| {
            compound_missing!(
                "compound_missing_first_ref_distance",
                tile_offset,
                "inter.compound.ref_distance[first]",
                SPEC_READ_REF_FRAMES
            )
        })?;
    let second_nonnegative = *input
        .ref_distance_nonnegative
        .get(second_ref)
        .ok_or_else(|| {
            compound_missing!(
                "compound_missing_second_ref_distance",
                tile_offset,
                "inter.compound.ref_distance[second]",
                SPEC_READ_REF_FRAMES
            )
        })?;
    Ok(usize::from(first_nonnegative ^ second_nonnegative))
}

fn compound_symbol_read_error(tile_offset: ByteOffset) -> crate::error::DecodeError {
    compound_missing!(
        "compound_block_mode_parse",
        tile_offset,
        "inter.compound.mode_info_symbols",
        SPEC_INTER_BLOCK_MODE_INFO
    )
}

#[cfg(test)]
#[path = "compound_tests.rs"]
mod tests;
