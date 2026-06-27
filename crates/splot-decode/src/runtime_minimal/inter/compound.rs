// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal compound-reference `mode_info` syntax for the first source-backed
//! COMPOUND_AVERAGE fixture.
//!
//! Feature tracking: `DECODE-INTER-COMPOUND-AVERAGE`.

use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;

use super::{Mv, unsupported_compound_at};
use crate::Result;
use crate::tile_payload::{TileCdfSelector, TileCdfSubset};

/// AV2 reference mode `COMPOUND_REFERENCE` selected by § 5.20.7.10 `comp_mode`.
const COMPOUND_REFERENCE: u8 = 1;

/// AV2 compound mode offset 0: `YMode = NEAR_NEARMV`.
const COMPOUND_MODE_NEAR_NEARMV: u8 = 0;

/// No-neighbour § 8.3.2 `comp_mode` context (`NNumBuf == 0`).
const COMP_MODE_CTX_NO_NEIGHBOUR: usize = 1;
/// No-neighbour § 8.3.2 `comp_ref` context for the verified second-reference
/// decision (`count_refs(0) == count_refs(1) == 0`).
const COMP_REF_CTX_NO_NEIGHBOUR: usize = 1;
/// Same-side bit type for the forced first ref and candidate ref 0 in the
/// `NumTotalRefs == 2`, `NumSameRefCompound == 1` path.
const COMP_REF1_BIT_TYPE_SAME_SIDE: usize = 0;
const SPEC_READ_REF_FRAMES: &str = "5.20.7.10";
const SPEC_INTER_BLOCK_MODE_INFO: &str = "5.20.7.6";

/// Inputs needed to read the narrow compound syntax subset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CompoundParseInput {
    /// § 6.19.7.11 `NumTotalRefs`.
    pub(super) num_total_refs: usize,
    /// Sequence § 5.4.6 `NumSameRefCompound`.
    pub(super) num_same_ref_compound: u8,
    /// Whether this block has decoded spatial neighbours.
    pub(super) has_neighbour: bool,
    /// § 7.11.2 / § 8.3.2 `NewMvContext` for `compound_mode_non_joint`.
    pub(super) new_mv_context: usize,
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

/// Reads § 5.20.7.10 / § 5.20.7.11 / § 5.20.7.6 for the verified compound subset:
/// `comp_mode == COMPOUND_REFERENCE`, implicit two-reference selection
/// `RefFrame == [0, 1]`, `is_joint == 0`, and `compound_mode_non_joint == 0`
/// (`NEAR_NEARMV`). The admitted block must be a skipped, no-neighbour, full-frame
/// 64x64 leaf; the caller then reads the two § 5.20.7.8 DRL symbols in spec order
/// and gates compound type/CWP tools through the sequence-level unsupported checks.
pub(super) fn read_compound_average_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CompoundParseInput,
    tile_offset: ByteOffset,
) -> Result<CompoundBlockSyntax> {
    gate_compound_subset(input, tile_offset)?;

    let comp_mode = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::CompMode {
                ctx: COMP_MODE_CTX_NO_NEIGHBOUR,
            },
            symbols,
        )
        .map_err(|_| compound_symbol_read_error(tile_offset))?;
    if comp_mode.get() != COMPOUND_REFERENCE {
        return Err(unsupported_compound_at(
            "compound_block_single_reference",
            tile_offset,
            "minimal compound-average decode expected §5.20.7.10 comp_mode == COMPOUND_REFERENCE for the reference-select fixture",
            SPEC_READ_REF_FRAMES,
        ));
    }

    // §5.20.7.11 read_compound_ref: with NumTotalRefs == 2 and
    // NumSameRefCompound == 0, no comp_ref symbol is read. When
    // NumSameRefCompound > 1, the first `comp_ref` is signalled through CompRef0
    // and must select RefFrame[0] == 0. When NumSameRefCompound > 0, the repeated
    // ref loop then reads one CompRef1 symbol; the verified fixture takes
    // comp_ref == 0, leaving RefFrame[1] at the default NumTotalRefs - 1 == 1.
    if input.num_same_ref_compound > 1 {
        let comp_ref0 = cdfs
            .read_block_symbol_trace(
                TileCdfSelector::CompRef0 {
                    ctx: COMP_REF_CTX_NO_NEIGHBOUR,
                    ref_idx: 0,
                },
                symbols,
            )
            .map_err(|_| compound_symbol_read_error(tile_offset))?;
        if comp_ref0.get() != 1 {
            return Err(unsupported_compound_at(
                "compound_block_missing_first_ref",
                tile_offset,
                "minimal compound-average decode requires the NumSameRefCompound first-reference comp_ref symbol to select RefFrame[0] == 0",
                SPEC_READ_REF_FRAMES,
            ));
        }
    }
    if input.num_same_ref_compound > 0 {
        let comp_ref1 = cdfs
            .read_block_symbol_trace(
                TileCdfSelector::CompRef1 {
                    ctx: COMP_REF_CTX_NO_NEIGHBOUR,
                    bit_type: COMP_REF1_BIT_TYPE_SAME_SIDE,
                    ref_idx: 0,
                },
                symbols,
            )
            .map_err(|_| compound_symbol_read_error(tile_offset))?;
        if comp_ref1.get() != 0 {
            return Err(unsupported_compound_at(
                "compound_block_same_ref_pair",
                tile_offset,
                "minimal compound-average decode requires the NumSameRefCompound second-reference comp_ref symbol to select the distinct RefFrame[1] == 1 path",
                SPEC_READ_REF_FRAMES,
            ));
        }
    }

    let is_joint = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::IsJoint {
                ctx: input.is_joint_ctx,
            },
            symbols,
        )
        .map_err(|_| compound_symbol_read_error(tile_offset))?;
    if is_joint.get() != 0 {
        return Err(unsupported_compound_at(
            "compound_block_joint_mode",
            tile_offset,
            "minimal compound-average decode only supports non-joint compound mode syntax (is_joint == 0)",
            SPEC_INTER_BLOCK_MODE_INFO,
        ));
    }

    let compound_mode = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::CompoundModeNonJoint {
                ctx: input.new_mv_context,
            },
            symbols,
        )
        .map_err(|_| compound_symbol_read_error(tile_offset))?;
    if compound_mode.get() != COMPOUND_MODE_NEAR_NEARMV {
        return Err(unsupported_compound_at(
            "compound_block_unsupported_mode",
            tile_offset,
            "minimal compound-average decode only supports compound_mode_non_joint == 0 (NEAR_NEARMV)",
            SPEC_INTER_BLOCK_MODE_INFO,
        ));
    }

    Ok(CompoundBlockSyntax {
        ref_frame0: 0,
        ref_frame1: 1,
        mv0: Mv::ZERO,
        mv1: Mv::ZERO,
    })
}

fn gate_compound_subset(input: CompoundParseInput, tile_offset: ByteOffset) -> Result<()> {
    if input.num_total_refs != 2 {
        return Err(unsupported_compound_at(
            "compound_num_total_refs",
            tile_offset,
            "minimal compound-average decode requires NumTotalRefs == 2 so read_compound_ref selects implicit RefFrame[0,1] without comp_ref symbols",
            SPEC_READ_REF_FRAMES,
        ));
    }
    if input.has_neighbour || input.new_mv_context != 0 {
        return Err(unsupported_compound_at(
            "compound_block_neighbour_context",
            tile_offset,
            "minimal compound-average decode is verified only for a no-neighbour NEAR_NEARMV block (NewMvContext == 0)",
            SPEC_INTER_BLOCK_MODE_INFO,
        ));
    }
    if input.is_joint_ctx != 1 {
        return Err(unsupported_compound_at(
            "compound_is_joint_context",
            tile_offset,
            "minimal compound-average decode is verified only for the same-side or unequal-distance §8.3.2 is_joint context 1 fixture",
            SPEC_INTER_BLOCK_MODE_INFO,
        ));
    }
    if input.skip != 1 {
        return Err(unsupported_compound_at(
            "compound_block_residual",
            tile_offset,
            "minimal compound-average decode only supports a skipped compound block (no residual syntax)",
            SPEC_INTER_BLOCK_MODE_INFO,
        ));
    }

    // Local const kept beside its guard so the magic-value documentation stays in context.
    #[allow(clippy::items_after_statements)]
    const FULL_SB_N4: usize = 16;
    if input.n4w != FULL_SB_N4
        || input.n4h != FULL_SB_N4
        || input.mi_row != 0
        || input.mi_col != 0
        || input.mi_rows != FULL_SB_N4
        || input.mi_cols != FULL_SB_N4
    {
        return Err(unsupported_compound_at(
            "compound_block_geometry",
            tile_offset,
            "minimal compound-average decode is verified only for one top-left 64x64 compound leaf covering a single-superblock frame",
            SPEC_INTER_BLOCK_MODE_INFO,
        ));
    }

    Ok(())
}

fn compound_symbol_read_error(tile_offset: ByteOffset) -> super::super::DecodeError {
    unsupported_compound_at(
        "compound_block_mode_parse",
        tile_offset,
        "minimal compound-average block mode-info syntax could not be parsed from the tile payload",
        SPEC_INTER_BLOCK_MODE_INFO,
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
            new_mv_context: 0,
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

    #[test]
    fn compound_average_syntax_roundtrips_through_symbol_encoder() {
        let mut enc_tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut encoder = SymbolEncoder::new();
        encode_symbol(
            &mut enc_tile,
            &mut encoder,
            TileCdfSelector::CompMode {
                ctx: COMP_MODE_CTX_NO_NEIGHBOUR,
            },
            COMPOUND_REFERENCE,
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
            TileCdfSelector::CompMode {
                ctx: COMP_MODE_CTX_NO_NEIGHBOUR,
            },
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
    fn compound_average_rejects_single_reference_comp_mode() {
        let mut enc_tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut encoder = SymbolEncoder::new();
        encode_symbol(
            &mut enc_tile,
            &mut encoder,
            TileCdfSelector::CompMode {
                ctx: COMP_MODE_CTX_NO_NEIGHBOUR,
            },
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
    fn compound_average_reads_same_ref_compound_ref_symbols() {
        let mut enc_tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut encoder = SymbolEncoder::new();
        encode_symbol(
            &mut enc_tile,
            &mut encoder,
            TileCdfSelector::CompMode {
                ctx: COMP_MODE_CTX_NO_NEIGHBOUR,
            },
            COMPOUND_REFERENCE,
        );
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
            TileCdfSelector::CompMode {
                ctx: COMP_MODE_CTX_NO_NEIGHBOUR,
            },
            COMPOUND_REFERENCE,
        );
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
            TileCdfSelector::CompMode {
                ctx: COMP_MODE_CTX_NO_NEIGHBOUR,
            },
            COMPOUND_REFERENCE,
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
    fn compound_average_rejects_unverified_geometry_before_reading_symbols() {
        let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut symbols = symbol_decoder(&[0x80, 0x00]);
        let mut input = default_input();
        input.n4w = 8;
        let before = symbols.consumed_bits();
        assert!(
            read_compound_average_syntax(&mut dec_tile, &mut symbols, input, ByteOffset::new(0),)
                .is_err()
        );
        assert_eq!(symbols.consumed_bits(), before);
    }
}
