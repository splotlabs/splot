// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoderConfig};
use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};

use super::super::cdf::FrameCdfSubset;
use super::super::encode_symbol_sequence;
use super::super::partition_allowed::PartitionFeatureFlags;
use super::super::partition_traversal::tests::make_work_unit as make_test_work_unit;
use super::super::partition_traversal::{
    TilePartitionBruState, TilePartitionContextState, TilePartitionFrameFacts,
    TilePartitionLoopRestorationState, TilePartitionTraversalInput,
    plan_tile_partition_traversal_cursor,
};
use super::*;
use crate::DecodeLimits;

const BLOCK_16X16: usize = 6;
const BLOCK_32X16: usize = 8;
const BLOCK_64X64: usize = 12;
const BLOCK_256X256: usize = 18;
const BLOCK_4X16: usize = 19;
const CLEAR_PARTITION_CONTEXT: usize = 0;
const PAYLOAD: [u8; 2] = [0x12, 0xFB];

fn make_work_unit(payload: &[u8]) -> DecodeTileWorkUnit<'_> {
    make_test_work_unit(payload, CdfUpdateMode::Disabled)
}

fn symbols_at_block_start<'payload>(
    work_unit: &mut DecodeTileWorkUnit<'payload>,
) -> SymbolDecoder<'payload> {
    let rows: Vec<Vec<usize>> = (0..16).map(|_| vec![BLOCK_256X256; 16]).collect();
    let mi0_rows: Vec<&[usize]> = rows.iter().map(Vec::as_slice).collect();
    let mi1_rows: Vec<&[usize]> = rows.iter().map(Vec::as_slice).collect();
    let edge = [CLEAR_PARTITION_CONTEXT; 16];
    let context =
        TilePartitionContextState::new([&mi0_rows, &mi1_rows], [&edge, &edge], [&edge, &edge]);
    let frame = TilePartitionFrameFacts::new(
        16,
        16,
        BLOCK_64X64,
        3,
        true,
        true,
        true,
        true,
        false,
        false,
        TilePartitionLoopRestorationState::NoSyntax,
        PartitionFeatureFlags::new(true, true),
        4,
        true,
        TilePartitionBruState::Active,
    )
    .unwrap();
    let cursor = plan_tile_partition_traversal_cursor(TilePartitionTraversalInput::new(
        work_unit,
        frame,
        context,
        DecodeLimits::DEFAULT,
    ))
    .unwrap();
    let (_plan, symbols) = cursor.into_parts();
    symbols
}

fn symbol_decoder(payload: &[u8]) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
    )
    .unwrap()
}

const SB_N4: usize = 16;
const D135_JOINT_MODE: u8 = 36;
const SMOOTH_V_JOINT_MODE: u8 = 2;

fn empty_joint_modes() -> TileIntraJointModeState {
    TileIntraJointModeState::new(SB_N4, 2 * SB_N4).unwrap()
}

fn empty_uses_mrls() -> TileUsesMrlsState {
    TileUsesMrlsState::new(SB_N4, 2 * SB_N4, SB_N4).unwrap()
}

fn empty_fsc_modes() -> TileFscModeState {
    TileFscModeState::new(SB_N4, 2 * SB_N4, SB_N4).unwrap()
}

fn empty_palette_state() -> TileLumaPaletteState {
    TileLumaPaletteState::new(SB_N4, 2 * SB_N4, SB_N4).unwrap()
}

#[allow(clippy::too_many_arguments)]
fn decode_general_intra_luma_block_mode(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    chroma_tools: GeneralIntraChromaToolConfig,
    joint_modes: &TileIntraJointModeState,
    uses_mrls: &TileUsesMrlsState,
    fsc_modes: &TileFscModeState,
    block_size_index: usize,
    block_r: usize,
    block_c: usize,
    block_n4w: usize,
    block_n4h: usize,
) -> Result<GeneralIntraLumaBlockMode, GeneralIntraBlockModeError> {
    decode_general_intra_luma_block_mode_with_fsc_context(
        work_unit,
        symbols,
        chroma_tools,
        joint_modes,
        uses_mrls,
        fsc_modes,
        true,
        block_size_index,
        block_r,
        block_c,
        block_n4w,
        block_n4h,
    )
}

#[allow(clippy::too_many_arguments)]
fn decode_general_intra_block_modes(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    chroma_tools: GeneralIntraChromaToolConfig,
    joint_modes: &TileIntraJointModeState,
    uses_mrls: &TileUsesMrlsState,
    fsc_modes: &TileFscModeState,
    palette_state: &TileLumaPaletteState,
    is_cfl_ctx: usize,
    block_size_index: usize,
    block_r: usize,
    block_c: usize,
    block_n4w: usize,
    block_n4h: usize,
    bit_depth_bits: u32,
) -> Result<GeneralIntraBlockModes, GeneralIntraBlockModeError> {
    decode_general_intra_block_modes_with_fsc_context(
        work_unit,
        symbols,
        chroma_tools,
        joint_modes,
        uses_mrls,
        fsc_modes,
        true,
        palette_state,
        is_cfl_ctx,
        block_size_index,
        block_r,
        block_c,
        block_n4w,
        block_n4h,
        block_size_index,
        block_n4w,
        block_n4h,
        bit_depth_bits,
    )
}

#[test]
fn cfl_allowed_420_uses_chroma_plane_64_sample_limit() {
    let tools = GeneralIntraChromaToolConfig::new(true, false);

    assert!(cfl_allowed_for_non_lossless_420(tools, 16, 16));
    assert!(cfl_allowed_for_non_lossless_420(tools, 32, 16));
    assert!(cfl_allowed_for_non_lossless_420(tools, 32, 32));
    assert!(!cfl_allowed_for_non_lossless_420(tools, 33, 32));
    assert!(!cfl_allowed_for_non_lossless_420(tools, 32, 33));
    assert!(!cfl_allowed_for_non_lossless_420(tools, 64, 32));
    assert!(!cfl_allowed_for_non_lossless_420(tools, 32, 64));
    assert!(!cfl_allowed_for_non_lossless_420(
        GeneralIntraChromaToolConfig::new(false, false),
        16,
        16,
    ));
}

#[test]
fn decodes_dc_luma_mode_and_a_chroma_mode_in_spec_order() {
    let mut work_unit = make_work_unit(&PAYLOAD);
    let mut symbols = symbols_at_block_start(&mut work_unit);
    let joint_modes = empty_joint_modes();
    let uses_mrls = empty_uses_mrls();

    let modes = decode_general_intra_block_modes(
        &mut work_unit,
        &mut symbols,
        GeneralIntraChromaToolConfig::disabled(),
        &joint_modes,
        &uses_mrls,
        &empty_fsc_modes(),
        &empty_palette_state(),
        0,
        BLOCK_64X64,
        0,
        0,
        SB_N4,
        SB_N4,
        8,
    )
    .unwrap();

    assert_eq!(modes.y_mode, IntraYMode::DC_PRED);
    assert_eq!(modes.intra_joint_mode, 0);
    assert!(
        modes.uv_mode < UV_INTRA_MODES_CFL_NOT_ALLOWED,
        "uv_mode {} out of range",
        modes.uv_mode
    );
}

#[test]
fn non_directional_left_neighbour_keeps_ctx_zero_and_decodes() {
    let mut work_unit = make_work_unit(&PAYLOAD);
    let mut symbols = symbols_at_block_start(&mut work_unit);
    let mut joint_modes = empty_joint_modes();
    let uses_mrls = empty_uses_mrls();
    joint_modes.record_block(0, 0, SB_N4, SB_N4, SMOOTH_V_JOINT_MODE);

    let modes = decode_general_intra_block_modes(
        &mut work_unit,
        &mut symbols,
        GeneralIntraChromaToolConfig::disabled(),
        &joint_modes,
        &uses_mrls,
        &empty_fsc_modes(),
        &empty_palette_state(),
        0,
        BLOCK_64X64,
        0,
        SB_N4,
        SB_N4,
        SB_N4,
        8,
    )
    .unwrap();
    assert_eq!(modes.y_mode, IntraYMode::DC_PRED);
}

#[test]
fn directional_neighbour_ctx_reads_with_the_real_context() {
    let mut work_unit = make_work_unit(&PAYLOAD);
    let mut symbols = symbols_at_block_start(&mut work_unit);
    let symbol_count_before = symbols.symbol_count();
    let mut joint_modes = empty_joint_modes();
    let uses_mrls = empty_uses_mrls();
    joint_modes.record_block(0, 0, SB_N4, SB_N4, D135_JOINT_MODE);

    let modes = decode_general_intra_block_modes(
        &mut work_unit,
        &mut symbols,
        GeneralIntraChromaToolConfig::disabled(),
        &joint_modes,
        &uses_mrls,
        &empty_fsc_modes(),
        &empty_palette_state(),
        0,
        BLOCK_64X64,
        0,
        SB_N4,
        SB_N4,
        SB_N4,
        8,
    )
    .unwrap();

    assert!(symbols.symbol_count() > symbol_count_before);
    assert!(!modes.y_mode.is_directional());
}

#[test]
fn directional_luma_mrl_zero_is_consumed_when_mrls_are_enabled() {
    let payload = encode_symbol_sequence(&[
        (TileCdfSelector::YModeSet, 0),
        (TileCdfSelector::YModeIndex { ctx: 0 }, 5),
        (TileCdfSelector::MrlIndex { ctx: 0 }, 0),
    ]);
    let mut work_unit = make_work_unit(&payload);
    let mut symbols = symbol_decoder(&payload);
    let joint_modes = empty_joint_modes();
    let uses_mrls = empty_uses_mrls();

    let luma = decode_general_intra_luma_block_mode(
        &mut work_unit,
        &mut symbols,
        GeneralIntraChromaToolConfig::disabled().with_enable_mrls(true),
        &joint_modes,
        &uses_mrls,
        &empty_fsc_modes(),
        BLOCK_64X64,
        0,
        0,
        SB_N4,
        SB_N4,
    )
    .unwrap();

    assert!(luma.y_mode.is_directional());
    assert_eq!(luma.mrl_index, 0);
    assert_eq!(luma.mrl_sec_index, None);
    assert_eq!(luma.uses_mrls, 0);
    assert_eq!(symbols.symbol_count(), 3);
    assert_eq!(symbols.finish().unwrap().symbol_count, 3);
}

#[test]
fn active_mrl_metadata_is_retained_after_mrl_sec_index_is_consumed() {
    let payload = encode_symbol_sequence(&[
        (TileCdfSelector::YModeSet, 0),
        (TileCdfSelector::YModeIndex { ctx: 0 }, 5),
        (TileCdfSelector::MrlIndex { ctx: 0 }, 1),
        (TileCdfSelector::MrlSecIndex { ctx: 0 }, 0),
    ]);
    let mut work_unit = make_work_unit(&payload);
    let mut symbols = symbol_decoder(&payload);
    let joint_modes = empty_joint_modes();
    let uses_mrls = empty_uses_mrls();

    let luma = decode_general_intra_luma_block_mode(
        &mut work_unit,
        &mut symbols,
        GeneralIntraChromaToolConfig::disabled().with_enable_mrls(true),
        &joint_modes,
        &uses_mrls,
        &empty_fsc_modes(),
        BLOCK_64X64,
        0,
        0,
        SB_N4,
        SB_N4,
    )
    .unwrap();

    assert_eq!(luma.mrl_index, 1);
    assert_eq!(luma.mrl_sec_index, Some(0));
    assert_eq!(luma.uses_mrls, 1);
    assert_eq!(symbols.symbol_count(), 4);
}

#[test]
fn active_fsc_mode_metadata_is_retained() {
    let payload = encode_symbol_sequence(&[
        (TileCdfSelector::YModeSet, 0),
        (TileCdfSelector::YModeIndex { ctx: 0 }, 0),
        (
            TileCdfSelector::FscMode {
                ctx: 0,
                bsize_group: fsc_bsize_group(BLOCK_16X16).unwrap(),
            },
            1,
        ),
    ]);
    let mut work_unit = make_work_unit(&payload);
    let mut symbols = symbol_decoder(&payload);
    let joint_modes = empty_joint_modes();
    let uses_mrls = empty_uses_mrls();

    let luma = decode_general_intra_luma_block_mode(
        &mut work_unit,
        &mut symbols,
        GeneralIntraChromaToolConfig::disabled().with_enable_idtx_intra(true),
        &joint_modes,
        &uses_mrls,
        &empty_fsc_modes(),
        BLOCK_16X16,
        0,
        0,
        4,
        4,
    )
    .unwrap();

    assert_eq!(luma.y_mode, IntraYMode::DC_PRED);
    assert_eq!(luma.fsc_mode, 1);
    assert_eq!(symbols.symbol_count(), 3);
}

#[test]
fn mixed_region_fsc_mode_uses_inter_context() {
    let bsize_group = fsc_bsize_group(BLOCK_16X16).unwrap();
    let payload = encode_symbol_sequence(&[
        (TileCdfSelector::YModeSet, 0),
        (TileCdfSelector::YModeIndex { ctx: 0 }, 0),
        (
            TileCdfSelector::FscMode {
                ctx: INTER_FSC_MODE_CONTEXT,
                bsize_group,
            },
            1,
        ),
    ]);
    let mut work_unit = make_work_unit(&payload);
    let mut symbols = symbol_decoder(&payload);
    let joint_modes = empty_joint_modes();
    let uses_mrls = empty_uses_mrls();
    let mut fsc_modes = TileFscModeState::new(2 * SB_N4, 2 * SB_N4, SB_N4).unwrap();
    fsc_modes.record_block(7, 11, 1, 1, 1);
    fsc_modes.record_block(11, 7, 1, 1, 1);

    let luma = decode_general_intra_luma_block_mode_with_fsc_context(
        &mut work_unit,
        &mut symbols,
        GeneralIntraChromaToolConfig::disabled().with_enable_idtx_intra(true),
        &joint_modes,
        &uses_mrls,
        &fsc_modes,
        false,
        BLOCK_16X16,
        8,
        8,
        4,
        4,
    )
    .unwrap();

    assert_eq!(luma.fsc_mode, 1);
    assert_eq!(symbols.symbol_count(), 3);
}

#[test]
fn inactive_palette_y_mode_is_consumed_after_chroma_mode() {
    let payload = encode_symbol_sequence(&[
        (TileCdfSelector::YModeSet, 0),
        (TileCdfSelector::YModeIndex { ctx: 0 }, 0),
        (TileCdfSelector::UvModeCflNotAllowed { ctx: 0 }, 1),
        (TileCdfSelector::PaletteYMode, 0),
    ]);
    let mut work_unit = make_work_unit(&payload);
    let mut symbols = symbol_decoder(&payload);
    let joint_modes = empty_joint_modes();
    let uses_mrls = empty_uses_mrls();

    let modes = decode_general_intra_block_modes(
        &mut work_unit,
        &mut symbols,
        GeneralIntraChromaToolConfig::disabled().with_allow_screen_content_tools(true),
        &joint_modes,
        &uses_mrls,
        &empty_fsc_modes(),
        &empty_palette_state(),
        0,
        BLOCK_16X16,
        0,
        0,
        4,
        4,
        8,
    )
    .unwrap();

    assert_eq!(modes.y_mode, IntraYMode::DC_PRED);
    assert_eq!(modes.uv_mode, 1);
    assert_eq!(symbols.symbol_count(), 4);
    assert_eq!(symbols.finish().unwrap().symbol_count, 4);
}

#[test]
fn active_palette_y_mode_reads_size_and_literal_colors() {
    let mut tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::with_config(
        SymbolEncoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
    );
    for (selector, value) in [
        (TileCdfSelector::YModeSet, 0),
        (TileCdfSelector::YModeIndex { ctx: 0 }, 0),
        (TileCdfSelector::UvModeCflNotAllowed { ctx: 0 }, 1),
        (TileCdfSelector::PaletteYMode, 1),
        (TileCdfSelector::PaletteYSize, 0),
    ] {
        tile.with_row_mut(selector, |row| {
            encoder.write_symbol(row, Symbol::new(value))
        })
        .unwrap()
        .unwrap();
    }
    encoder.write_literal(10, 8).unwrap();
    encoder.write_literal(0, 2).unwrap();
    encoder.write_literal(3, 5).unwrap();
    let payload = encoder.finish().unwrap().into_bytes();
    let mut work_unit = make_work_unit(&payload);
    let mut symbols = symbol_decoder(&payload);
    let joint_modes = empty_joint_modes();
    let uses_mrls = empty_uses_mrls();

    let modes = decode_general_intra_block_modes(
        &mut work_unit,
        &mut symbols,
        GeneralIntraChromaToolConfig::disabled().with_allow_screen_content_tools(true),
        &joint_modes,
        &uses_mrls,
        &empty_fsc_modes(),
        &empty_palette_state(),
        0,
        BLOCK_16X16,
        0,
        0,
        4,
        4,
        8,
    )
    .unwrap();

    let palette = modes.palette_y().expect("active palette");
    assert_eq!(palette.size(), 2);
    assert_eq!(&palette.colors()[..2], &[10, 14]);
    assert_eq!(symbols.symbol_count(), 20);
}

#[test]
fn mrl_symbols_use_retained_neighbour_contexts() {
    let payload = encode_symbol_sequence(&[
        (TileCdfSelector::YModeSet, 0),
        (TileCdfSelector::YModeIndex { ctx: 0 }, 5),
        (TileCdfSelector::MrlIndex { ctx: 2 }, 1),
        (TileCdfSelector::MrlSecIndex { ctx: 1 }, 1),
    ]);
    let mut work_unit = make_work_unit(&payload);
    let mut symbols = symbol_decoder(&payload);
    let joint_modes = TileIntraJointModeState::new(2 * SB_N4, 2 * SB_N4).unwrap();
    let mut uses_mrls = TileUsesMrlsState::new(2 * SB_N4, 2 * SB_N4, SB_N4).unwrap();
    uses_mrls.record_block(7, 11, 1, 1, 2);
    uses_mrls.record_block(11, 7, 1, 1, 1);

    let luma = decode_general_intra_luma_block_mode(
        &mut work_unit,
        &mut symbols,
        GeneralIntraChromaToolConfig::disabled().with_enable_mrls(true),
        &joint_modes,
        &uses_mrls,
        &TileFscModeState::new(2 * SB_N4, 2 * SB_N4, SB_N4).unwrap(),
        BLOCK_16X16,
        8,
        8,
        4,
        4,
    )
    .unwrap();

    assert_eq!(luma.mrl_index, 1);
    assert_eq!(luma.mrl_sec_index, Some(1));
    assert_eq!(luma.uses_mrls, 2);
    assert_eq!(symbols.symbol_count(), 4);
}

#[test]
fn mhccp_allowed_follows_current_non_lossless_420_bounds() {
    let mhccp = GeneralIntraChromaToolConfig::new(false, true);
    assert!(mhccp_allowed_for_non_lossless_420(mhccp, 4, 4));
    assert!(mhccp_allowed_for_non_lossless_420(mhccp, 16, 16));
    assert!(!mhccp_allowed_for_non_lossless_420(mhccp, 2, 2));
    assert!(!mhccp_allowed_for_non_lossless_420(mhccp, 17, 16));
    assert!(!mhccp_allowed_for_non_lossless_420(
        GeneralIntraChromaToolConfig::disabled(),
        4,
        4
    ));
}

#[test]
fn active_cfl_chroma_mode_returns_typed_uv_cfl_pred() {
    let payload = encode_symbol_sequence(&[
        (TileCdfSelector::IsCfl { ctx: 0 }, 1),
        (TileCdfSelector::CflIndex, 1),
    ]);
    let mut work_unit = make_work_unit(&payload);
    let mut symbols = symbol_decoder(&payload);

    let mode = decode_general_intra_chroma_block_mode(
        &mut work_unit,
        &mut symbols,
        GeneralIntraChromaToolConfig::new(true, false),
        GeneralIntraChromaModeContext::shared_or_non_sdp(0),
        IntraYMode::DC_PRED,
        BLOCK_64X64,
        SB_N4,
        SB_N4,
    )
    .unwrap();

    assert!(mode.is_cfl());
    assert_eq!(mode.uv_mode(), UV_CFL_PRED_MODE);
    assert_eq!(mode.coeff_uv_mode(), usize::from(UV_CFL_PRED_MODE));
    assert_eq!(
        mode.cfl_params(),
        Some(CflParams {
            index: CflIndex::DerivedAlpha,
            alpha_u: 0,
            alpha_v: 0,
            mh_dir: None
        })
    );
    assert_eq!(symbols.symbol_count(), 2);
    assert_eq!(symbols.finish().unwrap().symbol_count, 2);
}

#[test]
fn active_mhccp_chroma_mode_is_admitted_when_cfl_is_disabled() {
    let size_group = cfl_mh_dir_size_group(BLOCK_16X16).unwrap();
    let payload = encode_symbol_sequence(&[
        (TileCdfSelector::IsCfl { ctx: 0 }, 1),
        (TileCdfSelector::CflMhDir { size_group }, 2),
    ]);
    let mut work_unit = make_work_unit(&payload);
    let mut symbols = symbol_decoder(&payload);

    let mode = decode_general_intra_chroma_block_mode(
        &mut work_unit,
        &mut symbols,
        GeneralIntraChromaToolConfig::new(false, true),
        GeneralIntraChromaModeContext::shared_or_non_sdp(0),
        IntraYMode::DC_PRED,
        BLOCK_16X16,
        4,
        4,
    )
    .unwrap();

    assert!(mode.is_cfl());
    assert_eq!(mode.uv_mode(), UV_CFL_PRED_MODE);
    assert_eq!(
        mode.cfl_params(),
        Some(CflParams {
            index: CflIndex::Multi,
            alpha_u: 0,
            alpha_v: 0,
            mh_dir: Some(2)
        })
    );
    assert_eq!(symbols.symbol_count(), 2);
    assert_eq!(symbols.finish().unwrap().symbol_count, 2);
}

#[test]
fn shared_chroma_mhccp_dir_uses_luma_syntax_block_size() {
    let syntax_size_group = cfl_mh_dir_size_group(BLOCK_4X16).unwrap();
    let inherited_chroma_size_group = cfl_mh_dir_size_group(BLOCK_32X16).unwrap();
    assert_ne!(syntax_size_group, inherited_chroma_size_group);
    let payload = encode_symbol_sequence(&[
        (TileCdfSelector::YModeSet, 0),
        (TileCdfSelector::YModeIndex { ctx: 0 }, 0),
        (TileCdfSelector::IsCfl { ctx: 0 }, 1),
        (
            TileCdfSelector::CflMhDir {
                size_group: syntax_size_group,
            },
            2,
        ),
    ]);
    let mut work_unit = make_work_unit(&payload);
    let mut symbols = symbol_decoder(&payload);
    let joint_modes = empty_joint_modes();
    let uses_mrls = empty_uses_mrls();

    let modes = decode_general_intra_block_modes_with_fsc_context(
        &mut work_unit,
        &mut symbols,
        GeneralIntraChromaToolConfig::new(false, true),
        &joint_modes,
        &uses_mrls,
        &empty_fsc_modes(),
        true,
        &empty_palette_state(),
        0,
        BLOCK_4X16,
        0,
        15,
        1,
        4,
        BLOCK_32X16,
        8,
        4,
        8,
    )
    .unwrap();

    assert!(modes.is_cfl());
    assert_eq!(
        modes.cfl_params(),
        Some(CflParams {
            index: CflIndex::Multi,
            alpha_u: 0,
            alpha_v: 0,
            mh_dir: Some(2),
        })
    );
    assert_eq!(symbols.symbol_count(), 4);
    assert_eq!(symbols.finish().unwrap().symbol_count, 4);
}

#[test]
fn sdp_chroma_part_cfl_disallowed_reads_uv_mode_without_is_cfl() {
    let payload = encode_symbol_sequence(&[(TileCdfSelector::UvModeCflNotAllowed { ctx: 0 }, 0)]);
    let mut work_unit = make_work_unit(&payload);
    let mut symbols = symbol_decoder(&payload);

    let mode = decode_general_intra_chroma_block_mode(
        &mut work_unit,
        &mut symbols,
        GeneralIntraChromaToolConfig::new(true, true),
        GeneralIntraChromaModeContext::sdp_chroma_part(false, 0),
        IntraYMode::DC_PRED,
        BLOCK_64X64,
        SB_N4,
        SB_N4,
    )
    .unwrap();

    assert!(!mode.is_cfl());
    assert_eq!(mode.uv_mode(), 0);
    assert_eq!(symbols.symbol_count(), 1);
    assert_eq!(symbols.finish().unwrap().symbol_count, 1);
}

#[test]
fn lossless_large_chroma_reads_uv_mode_without_is_cfl() {
    let payload = encode_symbol_sequence(&[
        (TileCdfSelector::UseDpcmUv, 0),
        (TileCdfSelector::UvModeCflNotAllowed { ctx: 0 }, 0),
    ]);
    let mut work_unit = make_work_unit(&payload);
    let mut symbols = symbol_decoder(&payload);

    let mode = decode_general_intra_chroma_block_mode(
        &mut work_unit,
        &mut symbols,
        GeneralIntraChromaToolConfig::new(true, true).with_lossless(true),
        GeneralIntraChromaModeContext::shared_or_non_sdp(0),
        IntraYMode::DC_PRED,
        BLOCK_16X16,
        4,
        4,
    )
    .unwrap();

    assert!(!mode.is_cfl());
    assert_eq!(mode.uv_mode(), 0);
    assert_eq!(symbols.symbol_count(), 2);
    assert_eq!(symbols.finish().unwrap().symbol_count, 2);
}

#[test]
fn chroma_dpcm_modes_resolve_direction_and_coeff_mode() {
    let luma = GeneralIntraLumaBlockMode {
        y_mode: IntraYMode::DC_PRED,
        angle_delta_y: 0,
        intra_joint_mode: 0,
        mrl_index: 0,
        mrl_sec_index: None,
        fsc_mode: 0,
        uses_mrls: 0,
        use_dpcm_y: 0,
        dpcm_mode_y: 0,
    };

    let vertical = GeneralIntraChromaBlockMode::dpcm(0);
    assert_eq!(
        vertical.chroma_dpcm_direction(),
        Some(DpcmDirection::Vertical)
    );
    assert_eq!(
        vertical.supported_chroma_mode(IntraYMode::DC_PRED),
        Some(SupportedChromaMode::Vertical)
    );
    let modes = GeneralIntraBlockModes::from_luma_chroma_palette(luma, vertical, None);
    assert_eq!(modes.chroma_dpcm_direction(), Some(DpcmDirection::Vertical));
    assert_eq!(
        modes.supported_chroma_mode(),
        Some(SupportedChromaMode::Vertical)
    );

    let horizontal = GeneralIntraChromaBlockMode::dpcm(1);
    assert_eq!(
        horizontal.chroma_dpcm_direction(),
        Some(DpcmDirection::Horizontal)
    );
    assert_eq!(
        horizontal.supported_chroma_mode(IntraYMode::DC_PRED),
        Some(SupportedChromaMode::Horizontal)
    );
    assert_eq!(
        GeneralIntraBlockModes::from_luma_chroma_palette(luma, horizontal, None)
            .supported_chroma_mode(),
        Some(SupportedChromaMode::Horizontal)
    );
}

#[test]
fn read_cfl_alphas_consumes_explicit_sign_and_alpha_contexts() {
    let payload = encode_symbol_sequence(&[
        (TileCdfSelector::CflIndex, CFL_EXPLICIT),
        (TileCdfSelector::CflSign, 7),
        (TileCdfSelector::CflAlpha { ctx: 5 }, 3),
        (TileCdfSelector::CflAlpha { ctx: 5 }, 4),
    ]);
    let mut work_unit = make_work_unit(&payload);
    let mut symbols = symbol_decoder(&payload);

    let params = read_cfl_alphas(
        &mut work_unit,
        &mut symbols,
        GeneralIntraChromaToolConfig::new(true, false),
        BLOCK_64X64,
        SB_N4,
        SB_N4,
    )
    .unwrap();

    assert_eq!(
        params,
        CflParams {
            index: CflIndex::Explicit,
            alpha_u: 4,
            alpha_v: 5,
            mh_dir: None
        }
    );
    assert_eq!(symbols.symbol_count(), 4);
    assert_eq!(symbols.finish().unwrap().symbol_count, 4);
}

#[test]
fn read_cfl_alphas_empty_payload_fails_exit_symbol_validation() {
    let payload: [u8; 0] = [];
    let mut work_unit = make_work_unit(&payload);
    let mut symbols = symbol_decoder(&payload);

    read_cfl_alphas(
        &mut work_unit,
        &mut symbols,
        GeneralIntraChromaToolConfig::new(true, false),
        BLOCK_64X64,
        SB_N4,
        SB_N4,
    )
    .unwrap();

    assert!(symbols.finish().is_err());
}

#[test]
fn cfl_alpha_contexts_match_spec_tables() {
    let u_contexts: Vec<_> = (0..=7)
        .filter_map(|alpha_signs| {
            let sign_u = (alpha_signs + 1) / 3;
            let sign_v = (alpha_signs + 1) % 3;
            (sign_u != CFL_SIGN_ZERO).then(|| (alpha_signs, cfl_alpha_u_ctx(sign_u, sign_v)))
        })
        .collect();
    let v_contexts: Vec<_> = (0..=7)
        .filter_map(|alpha_signs| {
            let sign_u = (alpha_signs + 1) / 3;
            let sign_v = (alpha_signs + 1) % 3;
            (sign_v != CFL_SIGN_ZERO).then(|| (alpha_signs, cfl_alpha_v_ctx(sign_u, sign_v)))
        })
        .collect();

    assert_eq!(
        u_contexts,
        vec![(2, 0), (3, 1), (4, 2), (5, 3), (6, 4), (7, 5)]
    );
    assert_eq!(
        v_contexts,
        vec![(0, 0), (1, 3), (3, 1), (4, 4), (6, 2), (7, 5)]
    );
}

#[test]
fn cfl_mh_dir_size_group_uses_generated_size_group() {
    assert_eq!(cfl_mh_dir_size_group(BLOCK_64X64).unwrap(), 3);

    let invalid = splot_core::tables::conversion::SIZE_GROUP.len();
    let err = cfl_mh_dir_size_group(invalid).unwrap_err();
    assert!(matches!(
        err,
        GeneralIntraBlockModeError::InvalidCflMhDirBlockSizeIndex {
            block_size_index
        } if block_size_index == invalid
    ));
}
