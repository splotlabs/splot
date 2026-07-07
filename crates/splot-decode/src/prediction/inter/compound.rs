// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;

use super::{Mv, unsupported_compound_at};
use crate::Result;
use crate::bitstream::tile_payload::{TileCdfSelector, TileCdfSubset};

const COMPOUND_MODE_NEAR_NEARMV: u8 = 0;
const COMPOUND_MODE_SAME_REF_NEW_NEWMV: u8 = 3;
const RANKED_REF0_TO_PRUNE: usize = 3;
const MAX_REFS_PER_FRAME: usize = 7;
const SPEC_READ_REF_FRAMES: &str = "5.20.7.10";
const SPEC_INTER_BLOCK_MODE_INFO: &str = "5.20.7.6";

/// Inputs needed to read the compound-average syntax subset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CompoundParseInput {
    /// § 6.19.7.11 `NumTotalRefs`.
    pub(crate) num_total_refs: usize,
    /// Frame-capped sequence § 5.4.6 `NumSameRefCompound`.
    pub(crate) num_same_ref_compound: u8,
    /// § 8.3.2 reference-prediction contexts indexed by candidate reference.
    pub(crate) ref_contexts: [usize; MAX_REFS_PER_FRAME],
    /// Whether each ranked reference is at non-negative display distance from the current frame.
    pub(crate) ref_distance_nonnegative: [bool; MAX_REFS_PER_FRAME],
    /// § 8.3.2 `is_joint` context for different-reference compound.
    pub(crate) is_joint_ctx: Option<usize>,
}

/// Compound AV2 §5.20.7.6 `YMode` values currently admitted by the decoder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompoundYMode {
    /// `NEAR_NEARMV`.
    NearNear,
    /// `NEAR_NEWMV`.
    NearNew,
    /// `NEW_NEWMV`.
    NewNew,
}

impl CompoundYMode {
    pub(crate) const fn has_newmv(self) -> bool {
        match self {
            Self::NearNear => false,
            Self::NearNew | Self::NewNew => true,
        }
    }

    pub(crate) const fn has_nearmv(self) -> bool {
        match self {
            Self::NearNear | Self::NearNew => true,
            Self::NewNew => false,
        }
    }

    pub(crate) const fn reads_drl_idx(self) -> bool {
        self.has_newmv() || self.has_nearmv()
    }

    pub(crate) const fn has_second_drl(self, skip_mode_present: bool) -> bool {
        match self {
            Self::NearNear | Self::NearNew => !skip_mode_present,
            Self::NewNew => false,
        }
    }

    pub(crate) const fn mvd_sign_derivation_threshold(self) -> usize {
        match self {
            Self::NearNear | Self::NearNew => 1,
            Self::NewNew => 4,
        }
    }

    pub(crate) const fn use_amvd_index(self) -> Option<usize> {
        match self {
            Self::NearNear => None,
            Self::NearNew => Some(0),
            Self::NewNew => Some(7),
        }
    }

    pub(crate) const fn list0_is_newmv(self) -> bool {
        match self {
            Self::NearNear | Self::NearNew => false,
            Self::NewNew => true,
        }
    }

    pub(crate) const fn list1_is_newmv(self) -> bool {
        match self {
            Self::NearNear => false,
            Self::NearNew | Self::NewNew => true,
        }
    }
}

/// The parsed compound syntax handed to motion compensation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CompoundBlockSyntax {
    /// § 5.20.7.6 compound `YMode`.
    pub(crate) y_mode: CompoundYMode,
    /// § 5.20.7.11 `RefFrame[0]`.
    pub(crate) ref_frame0: i8,
    /// § 5.20.7.11 `RefFrame[1]`.
    pub(crate) ref_frame1: i8,
    /// List-0 § 7.11 motion vector.
    pub(crate) mv0: Mv,
    /// List-1 § 7.11 motion vector.
    pub(crate) mv1: Mv,
}

/// Reads the §5.20.7.11 compound reference pair (`read_ref_frames` tail) for
/// COMPOUND_AVERAGE and returns the selected `(RefFrame[0], RefFrame[1])`.
///
/// Per §5.20.7.6 the caller derives the block mode context from the returned
/// pair before reading any mode symbol via [`read_compound_mode_syntax`].
pub(crate) fn read_compound_reference_pair(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CompoundParseInput,
    tile_offset: ByteOffset,
) -> Result<(i8, i8)> {
    gate_compound_subset(input, tile_offset)?;

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

/// Reads the §5.20.7.6 compound mode symbols with the mode context derived
/// from the selected reference pair.
pub(crate) fn read_compound_mode_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    pair: (i8, i8),
    new_mv_context: usize,
    is_joint_ctx: Option<usize>,
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
            ref_frame0: pair.0,
            ref_frame1: pair.1,
            mv0: Mv::ZERO,
            mv1: Mv::ZERO,
        });
    }

    let is_joint_ctx = is_joint_ctx.ok_or_else(|| {
        compound_missing!(
            "compound_missing_is_joint_context",
            tile_offset,
            "inter.compound.is_joint_context",
            SPEC_INTER_BLOCK_MODE_INFO
        )
    })?;
    let is_joint = read_symbol(TileCdfSelector::IsJoint { ctx: is_joint_ctx })?;
    if is_joint != 0 {
        return Err(compound_cap!(
            "compound_block_joint_mode",
            tile_offset,
            "inter.compound.is_joint",
            SPEC_INTER_BLOCK_MODE_INFO
        ));
    }

    let compound_mode = read_symbol(TileCdfSelector::CompoundModeNonJoint {
        ctx: new_mv_context,
    })?;
    if compound_mode != COMPOUND_MODE_NEAR_NEARMV {
        return Err(compound_cap!(
            "compound_block_unsupported_mode",
            tile_offset,
            "inter.compound.mode != NEAR_NEARMV",
            SPEC_INTER_BLOCK_MODE_INFO
        ));
    }

    Ok(CompoundBlockSyntax {
        y_mode: CompoundYMode::NearNear,
        ref_frame0: pair.0,
        ref_frame1: pair.1,
        mv0: Mv::ZERO,
        mv1: Mv::ZERO,
    })
}

fn gate_compound_subset(input: CompoundParseInput, tile_offset: ByteOffset) -> Result<()> {
    if input.num_total_refs == 1 && input.num_same_ref_compound > 0 {
        return Ok(());
    }

    for (missing, reason, message, spec_section) in [
        (
            input.num_total_refs != 2,
            "compound_num_total_refs",
            "unsupported capability: inter.compound.num_total_refs != 2",
            SPEC_READ_REF_FRAMES,
        ),
        (
            input.is_joint_ctx != Some(1),
            "compound_is_joint_context",
            "unsupported capability: inter.compound.is_joint_ctx != 1",
            SPEC_INTER_BLOCK_MODE_INFO,
        ),
    ] {
        if missing {
            return Err(unsupported_compound_at(
                reason,
                tile_offset,
                message,
                spec_section,
            ));
        }
    }

    Ok(())
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use splot_core::span::ByteOffset;
    use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoder, SymbolDecoderConfig};
    use splot_core::symbol_encoder::SymbolEncoder;

    use super::*;
    use crate::bitstream::tile_payload::{FrameCdfSubset, TileCdfSelector, TileCdfSubset};

    fn symbol_decoder(payload: &[u8]) -> SymbolDecoder<'_> {
        SymbolDecoder::with_base_and_config(
            payload,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
        )
        .unwrap()
    }

    fn encode_symbol(
        tile: &mut TileCdfSubset,
        encoder: &mut SymbolEncoder,
        selector: TileCdfSelector,
        value: u8,
    ) {
        tile.with_row_mut(selector, |row| {
            encoder.write_symbol(row, Symbol::new(value))
        })
        .unwrap()
        .unwrap();
    }

    fn default_input() -> CompoundParseInput {
        CompoundParseInput {
            num_total_refs: 2,
            num_same_ref_compound: 0,
            ref_contexts: [1; MAX_REFS_PER_FRAME],
            ref_distance_nonnegative: [true; MAX_REFS_PER_FRAME],
            is_joint_ctx: Some(1),
        }
    }

    fn read_compound_average_syntax(
        cdfs: &mut TileCdfSubset,
        symbols: &mut SymbolDecoder<'_>,
        input: CompoundParseInput,
        tile_offset: ByteOffset,
    ) -> Result<CompoundBlockSyntax> {
        let is_joint_ctx = input.is_joint_ctx;
        let pair = read_compound_reference_pair(cdfs, symbols, input, tile_offset)?;
        read_compound_mode_syntax(cdfs, symbols, pair, 0, is_joint_ctx, tile_offset)
    }

    #[test]
    fn compound_average_syntax_roundtrips_through_symbol_encoder() {
        let mut enc_tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut encoder = SymbolEncoder::new();
        encode_symbol(
            &mut enc_tile,
            &mut encoder,
            TileCdfSelector::IsJoint { ctx: 1 },
            0,
        );
        encode_symbol(
            &mut enc_tile,
            &mut encoder,
            TileCdfSelector::CompoundModeNonJoint { ctx: 0 },
            COMPOUND_MODE_NEAR_NEARMV,
        );
        let bytes = encoder.finish().unwrap().into_bytes();

        let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut symbols = symbol_decoder(&bytes);
        let syntax = read_compound_average_syntax(
            &mut dec_tile,
            &mut symbols,
            default_input(),
            ByteOffset::new(0),
        )
        .unwrap();

        assert_eq!(
            syntax,
            CompoundBlockSyntax {
                y_mode: CompoundYMode::NearNear,
                ref_frame0: 0,
                ref_frame1: 1,
                mv0: Mv::ZERO,
                mv1: Mv::ZERO,
            }
        );
        symbols.exit_symbol().unwrap();
        for selector in [
            TileCdfSelector::IsJoint { ctx: 1 },
            TileCdfSelector::CompoundModeNonJoint { ctx: 0 },
        ] {
            assert_eq!(
                enc_tile.row(selector).unwrap(),
                dec_tile.row(selector).unwrap()
            );
        }
    }

    #[test]
    fn compound_average_reads_same_ref_compound_ref_symbols() {
        let mut enc_tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut encoder = SymbolEncoder::new();
        encode_symbol(
            &mut enc_tile,
            &mut encoder,
            TileCdfSelector::CompRef0 { ctx: 1, ref_idx: 0 },
            1,
        );
        encode_symbol(
            &mut enc_tile,
            &mut encoder,
            TileCdfSelector::CompRef1 {
                ctx: 1,
                bit_type: 0,
                ref_idx: 0,
            },
            0,
        );
        encode_symbol(
            &mut enc_tile,
            &mut encoder,
            TileCdfSelector::IsJoint { ctx: 1 },
            0,
        );
        encode_symbol(
            &mut enc_tile,
            &mut encoder,
            TileCdfSelector::CompoundModeNonJoint { ctx: 0 },
            COMPOUND_MODE_NEAR_NEARMV,
        );
        let bytes = encoder.finish().unwrap().into_bytes();

        let mut input = default_input();
        input.num_same_ref_compound = 2;
        let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut symbols = symbol_decoder(&bytes);
        let syntax =
            read_compound_average_syntax(&mut dec_tile, &mut symbols, input, ByteOffset::new(0))
                .unwrap();

        assert_eq!(syntax.ref_frame0, 0);
        assert_eq!(syntax.ref_frame1, 1);
        symbols.exit_symbol().unwrap();
        for selector in [
            TileCdfSelector::CompRef0 { ctx: 1, ref_idx: 0 },
            TileCdfSelector::CompRef1 {
                ctx: 1,
                bit_type: 0,
                ref_idx: 0,
            },
        ] {
            assert_eq!(
                enc_tile.row(selector).unwrap(),
                dec_tile.row(selector).unwrap()
            );
        }
    }

    #[test]
    fn compound_average_reads_same_reference_mode_symbol() {
        let mut enc_tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut encoder = SymbolEncoder::new();
        encode_symbol(
            &mut enc_tile,
            &mut encoder,
            TileCdfSelector::CompoundModeSameRefs { ctx: 0 },
            COMPOUND_MODE_NEAR_NEARMV,
        );
        let bytes = encoder.finish().unwrap().into_bytes();

        let mut input = default_input();
        input.num_total_refs = 1;
        input.num_same_ref_compound = 2;
        let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut symbols = symbol_decoder(&bytes);
        let syntax =
            read_compound_average_syntax(&mut dec_tile, &mut symbols, input, ByteOffset::new(0))
                .unwrap();

        assert_eq!(syntax.ref_frame0, 0);
        assert_eq!(syntax.ref_frame1, 0);
        symbols.exit_symbol().unwrap();
        assert_eq!(
            enc_tile
                .row(TileCdfSelector::CompoundModeSameRefs { ctx: 0 })
                .unwrap(),
            dec_tile
                .row(TileCdfSelector::CompoundModeSameRefs { ctx: 0 })
                .unwrap()
        );
    }

    #[test]
    fn compound_average_rejects_joint_mode() {
        assert_compound_average_rejects_is_joint_symbol(1);
    }

    #[test]
    fn compound_average_rejects_short_payload() {
        assert_compound_average_rejects_is_joint_symbol(0);
    }

    fn assert_compound_average_rejects_is_joint_symbol(symbol: u8) {
        let mut enc_tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut encoder = SymbolEncoder::new();
        encode_symbol(
            &mut enc_tile,
            &mut encoder,
            TileCdfSelector::IsJoint { ctx: 1 },
            symbol,
        );
        let bytes = encoder.finish().unwrap().into_bytes();

        let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut symbols = symbol_decoder(&bytes);
        assert!(
            read_compound_average_syntax(
                &mut dec_tile,
                &mut symbols,
                default_input(),
                ByteOffset::new(0),
            )
            .is_err()
        );
    }

    #[test]
    fn compound_average_does_not_gate_residual_geometry_before_reading_symbols() {
        let mut enc_tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut encoder = SymbolEncoder::new();
        encode_symbol(
            &mut enc_tile,
            &mut encoder,
            TileCdfSelector::IsJoint { ctx: 1 },
            0,
        );
        encode_symbol(
            &mut enc_tile,
            &mut encoder,
            TileCdfSelector::CompoundModeNonJoint { ctx: 0 },
            COMPOUND_MODE_NEAR_NEARMV,
        );
        let bytes = encoder.finish().unwrap().into_bytes();

        let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut symbols = symbol_decoder(&bytes);
        let before = symbols.consumed_bits();
        let result = read_compound_average_syntax(
            &mut dec_tile,
            &mut symbols,
            default_input(),
            ByteOffset::new(0),
        );

        assert!(result.is_ok());
        assert!(symbols.consumed_bits() > before);
    }
}
