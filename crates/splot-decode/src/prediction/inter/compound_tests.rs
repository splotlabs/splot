// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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
    }
}

fn read_compound_average_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CompoundParseInput,
    is_joint_ctx: usize,
    tile_offset: ByteOffset,
) -> Result<CompoundBlockSyntax> {
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
        1,
        ByteOffset::new(0),
    )
    .unwrap();

    assert_eq!(
        syntax,
        CompoundBlockSyntax {
            y_mode: CompoundYMode::NearNear,
            use_optflow: false,
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
fn non_joint_new_mv_modes_roundtrip_through_symbol_encoder() {
    for (mode, y_mode) in [
        (COMPOUND_MODE_NEAR_NEWMV, CompoundYMode::NearNew),
        (COMPOUND_MODE_NEW_NEARMV, CompoundYMode::NewNear),
        (COMPOUND_MODE_NEW_NEWMV, CompoundYMode::NewNew),
    ] {
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
            mode,
        );
        let bytes = encoder.finish().unwrap().into_bytes();

        let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut symbols = symbol_decoder(&bytes);
        let syntax = read_compound_average_syntax(
            &mut dec_tile,
            &mut symbols,
            default_input(),
            1,
            ByteOffset::new(0),
        )
        .unwrap();

        assert_eq!(syntax.y_mode, y_mode);
        symbols.exit_symbol().unwrap();
    }
}

#[test]
fn compound_opfl_modes_use_opfl_amvd_contexts() {
    assert_eq!(CompoundYMode::NearNew.use_amvd_index(false), Some(0));
    assert_eq!(CompoundYMode::NearNew.use_amvd_index(true), Some(2));
    assert_eq!(CompoundYMode::NewNear.use_amvd_index(false), Some(1));
    assert_eq!(CompoundYMode::NewNear.use_amvd_index(true), Some(3));
    assert_eq!(CompoundYMode::JointNew.use_amvd_index(false), Some(5));
    assert_eq!(CompoundYMode::JointNew.use_amvd_index(true), Some(6));
    assert_eq!(CompoundYMode::NewNew.use_amvd_index(false), Some(7));
    assert_eq!(CompoundYMode::NewNew.use_amvd_index(true), Some(8));
    assert_eq!(CompoundYMode::NearNear.use_amvd_index(true), None);
}

#[test]
fn compound_mode_predicates_keep_per_list_roles() {
    let cases = [
        (CompoundYMode::NearNear, true, false, false),
        (CompoundYMode::NearNew, true, false, true),
        (CompoundYMode::NewNear, false, true, false),
        (CompoundYMode::JointNew, false, true, true),
        (CompoundYMode::NewNew, false, true, true),
    ];

    for (mode, second_drl, list0_newmv, list1_newmv) in cases {
        assert_eq!(mode.has_second_drl(), second_drl, "{mode:?}");
        assert_eq!(mode.list0_is_newmv(), list0_newmv, "{mode:?}");
        assert_eq!(mode.list1_is_newmv(), list1_newmv, "{mode:?}");
    }
}

#[test]
fn compound_average_reads_is_joint_context_zero() {
    let mut enc_tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::new();
    encode_symbol(
        &mut enc_tile,
        &mut encoder,
        TileCdfSelector::IsJoint { ctx: 0 },
        0,
    );
    encode_symbol(
        &mut enc_tile,
        &mut encoder,
        TileCdfSelector::CompoundModeNonJoint { ctx: 0 },
        COMPOUND_MODE_NEAR_NEARMV,
    );
    let bytes = encoder.finish().unwrap().into_bytes();

    let input = default_input();
    let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&bytes);
    let syntax =
        read_compound_average_syntax(&mut dec_tile, &mut symbols, input, 0, ByteOffset::new(0))
            .unwrap();

    assert_eq!(syntax.y_mode, CompoundYMode::NearNear);
    symbols.exit_symbol().unwrap();
    for selector in [
        TileCdfSelector::IsJoint { ctx: 0 },
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
        read_compound_average_syntax(&mut dec_tile, &mut symbols, input, 1, ByteOffset::new(0))
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
fn compound_reference_pair_roundtrips_three_ranked_refs() {
    let selectors = [
        (TileCdfSelector::CompRef0 { ctx: 1, ref_idx: 0 }, 1),
        (
            TileCdfSelector::CompRef1 {
                ctx: 1,
                bit_type: 0,
                ref_idx: 0,
            },
            0,
        ),
        (
            TileCdfSelector::CompRef1 {
                ctx: 1,
                bit_type: 0,
                ref_idx: 1,
            },
            1,
        ),
    ];
    let mut enc_tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::new();
    for (selector, value) in selectors {
        encode_symbol(&mut enc_tile, &mut encoder, selector, value);
    }
    let bytes = encoder.finish().unwrap().into_bytes();

    let mut input = default_input();
    input.num_total_refs = 3;
    input.num_same_ref_compound = 2;
    let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&bytes);
    let pair = read_compound_reference_pair(&mut dec_tile, &mut symbols, input, ByteOffset::new(0))
        .unwrap();

    assert_eq!(pair, (0, 1));
    symbols.exit_symbol().unwrap();
    for (selector, _) in selectors {
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
        read_compound_average_syntax(&mut dec_tile, &mut symbols, input, 1, ByteOffset::new(0))
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
fn compound_average_accepts_joint_mode_without_non_joint_symbol() {
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
    let syntax = read_compound_average_syntax(
        &mut dec_tile,
        &mut symbols,
        default_input(),
        1,
        ByteOffset::new(0),
    )
    .unwrap();

    assert_eq!(syntax.y_mode, CompoundYMode::JointNew);
    assert_eq!(syntax.ref_frame0, 0);
    assert_eq!(syntax.ref_frame1, 1);
    symbols.exit_symbol().unwrap();
}

#[test]
fn compound_average_rejects_global_global_mode() {
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
        COMPOUND_MODE_GLOBAL_GLOBALMV,
    );
    let bytes = encoder.finish().unwrap().into_bytes();

    let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&bytes);
    assert!(
        read_compound_average_syntax(
            &mut dec_tile,
            &mut symbols,
            default_input(),
            1,
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
        1,
        ByteOffset::new(0),
    );

    assert!(result.is_ok());
    assert!(symbols.consumed_bits() > before);
}
