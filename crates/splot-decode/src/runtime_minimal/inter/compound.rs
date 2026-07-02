// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;

use super::{Mv, unsupported_compound_at};
use crate::Result;
use crate::tile_payload::{TileCdfSelector, TileCdfSubset};

const COMPOUND_MODE_NEAR_NEARMV: u8 = 0;
const COMP_REF_CTX_NO_NEIGHBOUR: usize = 1;
const COMP_REF1_BIT_TYPE_SAME_SIDE: usize = 0;
const FULL_SB_N4: usize = 16;
const SPEC_READ_REF_FRAMES: &str = "5.20.7.10";
const SPEC_INTER_BLOCK_MODE_INFO: &str = "5.20.7.6";

/// Inputs needed to read the compound-average syntax subset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CompoundParseInput {
    /// § 6.19.7.11 `NumTotalRefs`.
    pub(super) num_total_refs: usize,
    /// Sequence § 5.4.6 `NumSameRefCompound`.
    pub(super) num_same_ref_compound: u8,
    /// Whether this block has decoded spatial neighbours.
    pub(super) has_neighbour: bool,
    /// § 8.3.2 `is_joint` context.
    pub(super) is_joint_ctx: usize,
    /// Decoded § 5.20.5.10 `skip` flag.
    pub(super) skip: u8,
    /// Block width in 4x4 MI units.
    pub(super) n4w: usize,
    /// Block height in 4x4 MI units.
    pub(super) n4h: usize,
    /// Block top-left row in 4x4 MI units.
    pub(super) mi_row: usize,
    /// Block top-left column in 4x4 MI units.
    pub(super) mi_col: usize,
    /// Frame height in 4x4 MI units.
    pub(super) mi_rows: usize,
    /// Frame width in 4x4 MI units.
    pub(super) mi_cols: usize,
}

/// The parsed compound syntax handed to motion compensation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CompoundBlockSyntax {
    /// § 5.20.7.11 `RefFrame[0]`.
    pub(super) ref_frame0: i8,
    /// § 5.20.7.11 `RefFrame[1]`.
    pub(super) ref_frame1: i8,
    /// List-0 § 7.11 motion vector.
    pub(super) mv0: Mv,
    /// List-1 § 7.11 motion vector.
    pub(super) mv1: Mv,
}

/// Reads the §5.20.7.11 compound reference pair (`read_ref_frames` tail) for
/// COMPOUND_AVERAGE and returns the selected `(RefFrame[0], RefFrame[1])`.
///
/// Per §5.20.7.6 the caller derives the block mode context from the returned
/// pair before reading any mode symbol via [`read_compound_mode_syntax`].
pub(super) fn read_compound_reference_pair(
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

    if input.num_same_ref_compound > 1 {
        let comp_ref0 = read_symbol(TileCdfSelector::CompRef0 {
            ctx: COMP_REF_CTX_NO_NEIGHBOUR,
            ref_idx: 0,
        })?;
        if comp_ref0 != 1 {
            return Err(compound_cap!(
                "compound_block_missing_first_ref",
                tile_offset,
                "inter.compound.comp_ref0 != 1",
                SPEC_READ_REF_FRAMES
            ));
        }
    }
    if input.num_same_ref_compound > 0 {
        let comp_ref1 = read_symbol(TileCdfSelector::CompRef1 {
            ctx: COMP_REF_CTX_NO_NEIGHBOUR,
            bit_type: COMP_REF1_BIT_TYPE_SAME_SIDE,
            ref_idx: 0,
        })?;
        if comp_ref1 != 0 {
            return Err(compound_cap!(
                "compound_block_same_ref_pair",
                tile_offset,
                "inter.compound.comp_ref1 != 0",
                SPEC_READ_REF_FRAMES
            ));
        }
    }

    Ok((0, 1))
}

/// Reads the §5.20.7.6 compound mode symbols with the mode context derived
/// from the selected reference pair.
pub(super) fn read_compound_mode_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    pair: (i8, i8),
    new_mv_context: usize,
    is_joint_ctx: usize,
    tile_offset: ByteOffset,
) -> Result<CompoundBlockSyntax> {
    if new_mv_context != 0 {
        return Err(compound_cap!(
            "compound_block_neighbour_context",
            tile_offset,
            "inter.compound.neighbour_context",
            SPEC_INTER_BLOCK_MODE_INFO
        ));
    }
    let mut read_symbol = |selector| {
        cdfs.read_block_symbol_trace(selector, symbols)
            .map(splot_core::symbol::Symbol::get)
            .map_err(|_| compound_symbol_read_error(tile_offset))
    };

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
        ref_frame0: pair.0,
        ref_frame1: pair.1,
        mv0: Mv::ZERO,
        mv1: Mv::ZERO,
    })
}

fn gate_compound_subset(input: CompoundParseInput, tile_offset: ByteOffset) -> Result<()> {
    let full_sb_geometry = input.n4w == FULL_SB_N4
        && input.n4h == FULL_SB_N4
        && input.mi_row == 0
        && input.mi_col == 0
        && input.mi_rows == FULL_SB_N4
        && input.mi_cols == FULL_SB_N4;
    for (missing, reason, message, spec_section) in [
        (
            input.num_total_refs != 2,
            "compound_num_total_refs",
            "unsupported capability: inter.compound.num_total_refs != 2",
            SPEC_READ_REF_FRAMES,
        ),
        (
            input.has_neighbour,
            "compound_block_neighbour_context",
            "unsupported capability: inter.compound.neighbour_context",
            SPEC_INTER_BLOCK_MODE_INFO,
        ),
        (
            input.is_joint_ctx != 1,
            "compound_is_joint_context",
            "unsupported capability: inter.compound.is_joint_ctx != 1",
            SPEC_INTER_BLOCK_MODE_INFO,
        ),
        (
            input.skip != 1,
            "compound_block_residual",
            "unsupported capability: inter.compound.residual",
            SPEC_INTER_BLOCK_MODE_INFO,
        ),
        (
            !full_sb_geometry,
            "compound_block_geometry",
            "unsupported capability: inter.compound.block_geometry",
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

fn compound_symbol_read_error(tile_offset: ByteOffset) -> super::super::DecodeError {
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
    use crate::tile_payload::{FrameCdfSubset, TileCdfSelector, TileCdfSubset};

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
            has_neighbour: false,
            is_joint_ctx: 1,
            skip: 1,
            n4w: 16,
            n4h: 16,
            mi_row: 0,
            mi_col: 0,
            mi_rows: 16,
            mi_cols: 16,
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
            TileCdfSelector::CompRef0 {
                ctx: COMP_REF_CTX_NO_NEIGHBOUR,
                ref_idx: 0,
            },
            1,
        );
        encode_symbol(
            &mut enc_tile,
            &mut encoder,
            TileCdfSelector::CompRef1 {
                ctx: COMP_REF_CTX_NO_NEIGHBOUR,
                bit_type: COMP_REF1_BIT_TYPE_SAME_SIDE,
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
            TileCdfSelector::CompRef0 {
                ctx: COMP_REF_CTX_NO_NEIGHBOUR,
                ref_idx: 0,
            },
            TileCdfSelector::CompRef1 {
                ctx: COMP_REF_CTX_NO_NEIGHBOUR,
                bit_type: COMP_REF1_BIT_TYPE_SAME_SIDE,
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
    fn compound_average_rejects_joint_mode() {
        let mut enc_tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut encoder = SymbolEncoder::new();
        encode_symbol(
            &mut enc_tile,
            &mut encoder,
            TileCdfSelector::IsJoint { ctx: 1 },
            1,
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
    fn compound_average_rejects_short_payload() {
        let mut enc_tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut encoder = SymbolEncoder::new();
        encode_symbol(
            &mut enc_tile,
            &mut encoder,
            TileCdfSelector::IsJoint { ctx: 1 },
            0,
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
    fn compound_average_rejects_unsupported_geometry_before_reading_symbols() {
        let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut symbols = symbol_decoder(&[0x80, 0x00]);
        let mut input = default_input();
        input.n4w = 8;
        let before = symbols.consumed_bits();
        let result =
            read_compound_average_syntax(&mut dec_tile, &mut symbols, input, ByteOffset::new(0));

        assert!(result.is_err());
        assert_eq!(before, symbols.consumed_bits());
    }
}
