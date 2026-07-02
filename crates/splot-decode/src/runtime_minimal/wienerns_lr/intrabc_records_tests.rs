// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unit tests for the bounded IntrABC syntax handoff ([`super`]).

use splot_core::headers::frame::{
    FrameSize, InterControl, IntrabcParams, MvPrecision, TxMode, build_minimal_intra_clk_core,
    build_minimal_intra_sequence_header,
};
use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoder, SymbolDecoderConfig};
use splot_core::symbol_encoder::SymbolEncoder;

use crate::error::DecodeError;
use crate::tile_payload::{FrameCdfSubset, MvCdfSelector};

use super::*;

const BLOCK_16X16: usize = 6;

fn no_off() -> ByteOffset {
    ByteOffset::new(0)
}

/// A `skip` IntrABC neighbour prelude with an integer block vector, for
/// populating the neighbour grid / ref-MV bank in tests.
fn ac0ej3_skip_neighbour() -> IntrabcBlockPrelude {
    IntrabcBlockPrelude {
        use_intrabc: true,
        is_inter: true,
        skip_flag: true,
        intrabc: Some(IntrabcInfo {
            intrabc_mode: 1,
            ref_mv_idx: 0,
            mv_precision: MV_PRECISION_QUARTER_PEL,
            block_mv: IntrabcBlockVector { row: -512, col: 0 },
        }),
    }
}

fn selectable_fixture() -> (SequenceHeader, FrameHeaderCore) {
    let mut sequence = build_minimal_intra_sequence_header().unwrap();
    let (mut core, _) = build_minimal_intra_clk_core().unwrap();
    sequence
        .inter
        .as_mut()
        .unwrap()
        .seq_max_bvp_drl_bits_minus_1 = 0;
    sequence.inter.as_mut().unwrap().enable_bawp = false;
    core.intra_tail.as_mut().unwrap().tx_mode = TxMode::Select;
    core.allow_intrabc = Some(true);
    core.intrabc = Some(IntrabcParams {
        allow_intrabc: true,
        allow_global_intrabc: Some(false),
        allow_local_intrabc: None,
        change_bvp_drl: Some(false),
        max_bvp_drl_bits_minus_1: None,
    });
    core.force_integer_mv = Some(false);
    (sequence, core)
}

fn selectable_large_frame_fixture() -> (SequenceHeader, FrameHeaderCore) {
    let (sequence, mut core) = selectable_fixture();
    core.frame_size = Some(FrameSize::new(128, 128));
    let tile_info = core.tile_info.as_mut().unwrap();
    tile_info.mi_col_starts = vec![0, 32];
    tile_info.mi_row_starts = vec![0, 32];
    (sequence, core)
}

fn unsupported_reason(error: DecodeError) -> &'static str {
    match error {
        DecodeError::UnsupportedFeature { unsupported } => unsupported.reason(),
        other => panic!("unexpected decode error: {other:?}"),
    }
}

fn encode_steps(steps: &[(Option<TileCdfSelector>, u32)]) -> Vec<u8> {
    let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::new();
    for &(selector, value) in steps {
        if let Some(selector) = selector {
            cdfs.with_row_mut(selector, |row| {
                encoder.write_symbol(row, Symbol::new(u8::try_from(value).unwrap()))
            })
            .unwrap()
            .unwrap();
        } else {
            encoder.write_literal(value, 1).unwrap();
        }
    }
    encoder.finish().unwrap().into_bytes()
}

fn decoder(payload: &[u8]) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    )
    .unwrap()
}

fn state() -> TileIntrabcPreludeState {
    let (sequence, _) = selectable_fixture();
    TileIntrabcPreludeState::new(64, 64, &sequence, ByteOffset::new(0)).unwrap()
}

#[test]
fn resolve_force_integer_mv_from_inter_mv_precision_when_flat_mirror_missing() {
    let (_, mut core) = selectable_fixture();
    core.force_integer_mv = None;
    let mut inter = InterControl::default();
    inter.mv_precision = Some(MvPrecision::QuarterPel);
    core.inter = Some(inter);

    assert!(!resolve_intrabc_force_integer_mv(&core, no_off()).unwrap());
}

#[test]
fn inter_non_mixed_region_does_not_code_intrabc_use() {
    let (_, mut core) = selectable_fixture();
    core.frame_is_intra = Some(false);
    core.allow_intrabc = None;
    let mut inter = InterControl::default();
    inter.allow_intrabc = Some(true);
    core.inter = Some(inter);
    let block = IntrabcBlockContext::new_with_mixed_region(0, 0, 2, false, false);

    assert!(!intrabc_use_is_coded(&core, block, 4, 4));
}

/// Runs the live `read_intrabc_use_and_skip` → `read_intrabc_info` sequence over
/// `steps` on the large-frame fixture, returning the decoded `use_skip`, the
/// `read_intrabc_info` result (`Ok(info)` for a `skip` leaf that advances, `Err`
/// for a fail-closed leaf), and the symbol count consumed. `skip_flag` is the
/// leaf's §5.20.5.3 `skip` carried into `read_intrabc_info`.
fn run_intrabc_prelude(
    steps: &[(Option<TileCdfSelector>, u32)],
    skip_flag: bool,
) -> (IntrabcUseSkip, Result<IntrabcInfo>, u64) {
    let (sequence, core) = selectable_large_frame_fixture();
    let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let payload = encode_steps(steps);
    let mut symbols = decoder(&payload);
    let state = state();
    let block = IntrabcBlockContext::new(20, 0, 2, false);
    let geometry = IntrabcBlockGeometry::new(block, 4, 4);

    let use_skip = read_intrabc_use_and_skip(
        &mut cdfs,
        &mut symbols,
        &state,
        &core,
        geometry,
        ByteOffset::new(20),
    )
    .unwrap();
    let info = read_intrabc_info(
        &mut cdfs,
        &mut symbols,
        &state,
        &sequence,
        &core,
        geometry,
        skip_flag,
        None,
        ByteOffset::new(20),
    );
    (use_skip, info, symbols.symbol_count())
}

#[test]
fn active_intrabc_nearmv_skip_reads_use_skip_mode_and_drl_then_advances() {
    let (use_skip, info, symbol_count) = run_intrabc_prelude(
        &[
            (Some(TileCdfSelector::Intrabc { ctx: 0 }), 1),
            (Some(TileCdfSelector::Skip { ctx: 0 }), 1),
            (Some(TileCdfSelector::IntrabcMode), 1),
            (None, 0),
        ],
        true,
    );

    assert_eq!(
        use_skip,
        IntrabcUseSkip {
            use_intrabc: true,
            skip_flag: true,
        }
    );
    assert_eq!(info.unwrap().intrabc_mode, 1);
    assert_eq!(symbol_count, 4);
}

#[test]
fn active_intrabc_newmv_nonskip_reads_block_vector_and_returns_info_for_residual() {
    let (use_skip, info, symbol_count) = run_intrabc_prelude(
        &[
            (Some(TileCdfSelector::Intrabc { ctx: 0 }), 1),
            (Some(TileCdfSelector::Skip { ctx: 0 }), 0),
            (Some(TileCdfSelector::IntrabcMode), 0),
            (None, 0),
            (Some(TileCdfSelector::IntrabcPrecision), 1),
            (
                Some(TileCdfSelector::ReadMv(MvCdfSelector::JointShellSet {
                    mv_ctx: 1,
                })),
                0,
            ),
            (
                Some(TileCdfSelector::ReadMv(MvCdfSelector::JointShellClass {
                    precision: usize::from(MV_PRECISION_QUARTER_PEL),
                    shell_set: 0,
                    mv_ctx: 1,
                })),
                0,
            ),
            (
                Some(TileCdfSelector::ReadMv(
                    MvCdfSelector::ShellOffsetLowClass {
                        mv_ctx: 1,
                        shell_class: 0,
                    },
                )),
                0,
            ),
        ],
        false,
    );

    assert_eq!(
        use_skip,
        IntrabcUseSkip {
            use_intrabc: true,
            skip_flag: false,
        }
    );
    let info = info.expect("non-skip IntrABC prelude returns parsed mode-info");
    assert_eq!(info.intrabc_mode, 0);
    assert_eq!(info.block_mv, IntrabcBlockVector { row: -512, col: 0 });
    assert_eq!(symbol_count, 8);
}

#[test]
fn active_intrabc_ref_stack_admits_two_distinct_spatial_candidates() {
    let (mut sequence, core) = selectable_large_frame_fixture();
    sequence.inter.as_mut().unwrap().drl_reorder = DrlReorder::Always;
    let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let payload = encode_steps(&[
        (Some(TileCdfSelector::Intrabc { ctx: 1 }), 1),
        (Some(TileCdfSelector::Skip { ctx: 0 }), 0),
        (Some(TileCdfSelector::IntrabcMode), 1),
        (None, 0),
    ]);
    let mut symbols = decoder(&payload);
    let mut state = TileIntrabcPreludeState::new(64, 64, &sequence, no_off()).unwrap();
    let neighbour = |state: &mut TileIntrabcPreludeState, row, col, mv| {
        state
            .record_block(
                row,
                col,
                1,
                1,
                IntrabcBlockPrelude {
                    use_intrabc: true,
                    is_inter: true,
                    skip_flag: false,
                    intrabc: Some(IntrabcInfo {
                        intrabc_mode: 1,
                        ref_mv_idx: 0,
                        mv_precision: MV_PRECISION_QUARTER_PEL,
                        block_mv: mv,
                    }),
                },
                ByteOffset::new(0),
            )
            .unwrap();
    };
    neighbour(&mut state, 23, 7, IntrabcBlockVector { row: -512, col: 0 });
    neighbour(&mut state, 19, 8, IntrabcBlockVector { row: 0, col: -3072 });
    let block = IntrabcBlockContext::new(20, 8, 2, false);
    let geometry = IntrabcBlockGeometry::new(block, 4, 4);

    let use_skip = read_intrabc_use_and_skip(
        &mut cdfs,
        &mut symbols,
        &state,
        &core,
        geometry,
        ByteOffset::new(20),
    )
    .unwrap();
    let info = read_intrabc_info(
        &mut cdfs,
        &mut symbols,
        &state,
        &sequence,
        &core,
        geometry,
        false,
        None,
        ByteOffset::new(20),
    )
    .unwrap();

    assert_eq!(
        use_skip,
        IntrabcUseSkip {
            use_intrabc: true,
            skip_flag: false,
        }
    );
    assert_eq!(info.block_mv, IntrabcBlockVector { row: -512, col: 0 });
}

#[test]
fn intrabc_ref_stack_admits_ac0ej3_mi_0_112_default_only_stack() {
    let (mut sequence, _core) = selectable_large_frame_fixture();
    sequence
        .inter
        .as_mut()
        .unwrap()
        .seq_max_bvp_drl_bits_minus_1 = 2;
    let mut state = TileIntrabcPreludeState::new(128, 256, &sequence, no_off()).unwrap();
    state
        .record_block(16, 56, 8, 16, ac0ej3_skip_neighbour(), no_off())
        .unwrap();
    let geometry =
        IntrabcBlockGeometry::new(IntrabcBlockContext::new(0, 112, BLOCK_16X16, false), 8, 16);
    let syntax = IntrabcInfoSyntax {
        intrabc_mode: 1,
        ref_mv_idx: 3,
        mv_precision: MV_PRECISION_QUARTER_PEL,
        max_bvp_drl_bits_minus_1: 2,
    };

    let pred_mv =
        ensure_intrabc_ref_stack_supported(&state, &sequence, geometry, syntax, no_off()).unwrap();
    assert_eq!(pred_mv, Mv { row: 0, col: -256 });
}

#[test]
fn spatial_scan_admits_sb_border_col_minus_two_neighbour() {
    let (sequence, _core) = selectable_large_frame_fixture();
    let neighbour = ac0ej3_skip_neighbour(); // BV (-512, 0).
    let mut sb_border = TileIntrabcPreludeState::new(64, 64, &sequence, no_off()).unwrap();
    sb_border
        .record_block(15, 54, 1, 1, neighbour, no_off())
        .unwrap();
    let at_border =
        IntrabcBlockGeometry::new(IntrabcBlockContext::new(16, 56, BLOCK_16X16, false), 8, 16);
    let scan = sb_border.spatial_intrabc_scan(at_border);
    assert_eq!(
        scan.candidates,
        vec![super::super::intrabc_ref_mv_stack::WeightedBv {
            mv: Mv { row: -512, col: 0 },
            weight: 0,
        }]
    );
    assert!(!scan.defer);

    let mut interior = TileIntrabcPreludeState::new(64, 64, &sequence, no_off()).unwrap();
    interior
        .record_block(19, 54, 1, 1, neighbour, no_off())
        .unwrap();
    let at_interior =
        IntrabcBlockGeometry::new(IntrabcBlockContext::new(20, 56, BLOCK_16X16, false), 8, 16);
    let control = interior.spatial_intrabc_scan(at_interior);
    assert!(control.candidates.is_empty());
    assert!(!control.defer);
}

#[test]
fn intrabc_newmv_geometry_derives_integer_luma_copy_rectangles() {
    let (sequence, mut core) = selectable_large_frame_fixture();
    core.force_integer_mv = Some(true);
    let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let payload = encode_steps(&[
        (Some(TileCdfSelector::IntrabcMode), 0),
        (None, 0),
        (
            Some(TileCdfSelector::ReadMv(MvCdfSelector::JointShellSet {
                mv_ctx: 1,
            })),
            0,
        ),
        (
            Some(TileCdfSelector::ReadMv(MvCdfSelector::JointShellClass {
                precision: usize::from(MV_PRECISION_ONE_PEL),
                shell_set: 0,
                mv_ctx: 1,
            })),
            0,
        ),
        (
            Some(TileCdfSelector::ReadMv(
                MvCdfSelector::ShellOffsetLowClass {
                    mv_ctx: 1,
                    shell_class: 0,
                },
            )),
            0,
        ),
    ]);
    let mut symbols = decoder(&payload);
    let block = IntrabcBlockContext::new(20, 0, 2, false);
    let geometry = IntrabcBlockGeometry::new(block, 4, 4);

    let info = read_intrabc_info_record(
        &mut cdfs,
        &mut symbols,
        &sequence,
        &core,
        geometry,
        ByteOffset::new(20),
    )
    .unwrap();
    let prediction =
        derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
            .unwrap();

    assert_eq!(prediction.target, PlaneRect::new(0, 80, 16, 16).unwrap());
    assert_eq!(prediction.source, PlaneRect::new(0, 16, 16, 16).unwrap());
    assert!(!prediction.fractional);
    assert_eq!(prediction.scaling.start_x >> 10, 0);
    assert_eq!(prediction.scaling.start_y >> 10, 16);
    assert_eq!((prediction.scaling.start_x >> 6) & 15, 0);
    assert_eq!((prediction.scaling.start_y >> 6) & 15, 0);
}

#[test]
fn intrabc_nearmv_geometry_derives_integer_luma_copy_rectangles() {
    let (sequence, core) = selectable_large_frame_fixture();
    let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let payload = encode_steps(&[(Some(TileCdfSelector::IntrabcMode), 1), (None, 0)]);
    let mut symbols = decoder(&payload);
    let block = IntrabcBlockContext::new(20, 0, 2, false);
    let geometry = IntrabcBlockGeometry::new(block, 4, 4);

    let info = read_intrabc_info_record(
        &mut cdfs,
        &mut symbols,
        &sequence,
        &core,
        geometry,
        ByteOffset::new(20),
    )
    .unwrap();
    let prediction =
        derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
            .unwrap();

    assert_eq!(prediction.target, PlaneRect::new(0, 80, 16, 16).unwrap());
    assert_eq!(prediction.source, PlaneRect::new(0, 16, 16, 16).unwrap());
    assert!(!prediction.fractional);
    assert_eq!(prediction.scaling.start_x >> 10, 0);
    assert_eq!(prediction.scaling.start_y >> 10, 16);
    assert_eq!((prediction.scaling.start_x >> 6) & 15, 0);
    assert_eq!((prediction.scaling.start_y >> 6) & 15, 0);
}

#[test]
fn intrabc_geometry_derives_bilinear_fractional_luma_prediction_region() {
    let (_, core) = selectable_large_frame_fixture();
    let block = IntrabcBlockContext::new(8, 8, 2, false);
    let geometry = IntrabcBlockGeometry::new(block, 4, 4);
    let info = IntrabcInfo {
        intrabc_mode: 0,
        ref_mv_idx: 0,
        mv_precision: MV_PRECISION_QUARTER_PEL,
        block_mv: IntrabcBlockVector { row: -132, col: 0 },
    };

    let prediction =
        derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
            .unwrap();

    assert_eq!(prediction.target, PlaneRect::new(32, 32, 16, 16).unwrap());
    assert_eq!(prediction.source, prediction.target);
    assert!(prediction.fractional);
    assert_eq!(prediction.scaling.start_x >> 10, 32);
    assert_eq!(prediction.scaling.start_y >> 10, 15);
    assert_eq!((prediction.scaling.start_x >> 6) & 15, 0);
    assert_ne!((prediction.scaling.start_y >> 6) & 15, 0);
}

#[test]
fn intrabc_geometry_admits_fractional_border_extension_at_frame_top() {
    let (_, core) = selectable_large_frame_fixture();
    let block = IntrabcBlockContext::new(0, 8, 2, false);
    let geometry = IntrabcBlockGeometry::new(block, 4, 4);
    let info = IntrabcInfo {
        intrabc_mode: 0,
        ref_mv_idx: 0,
        mv_precision: MV_PRECISION_QUARTER_PEL,
        block_mv: IntrabcBlockVector { row: -4, col: 0 },
    };

    let prediction =
        derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
            .unwrap();

    assert_eq!(prediction.target, PlaneRect::new(32, 0, 16, 16).unwrap());
    assert_eq!(prediction.source, prediction.target);
    assert!(prediction.fractional);
    assert_eq!(prediction.scaling.start_x >> 10, 32);
    assert_eq!(prediction.scaling.start_y >> 10, -1);
    assert_ne!((prediction.scaling.start_y >> 6) & 15, 0);
}

#[test]
fn intrabc_geometry_uses_mi_domain_for_partial_edge_frame() {
    let (_, mut core) = selectable_fixture();
    core.frame_size = Some(FrameSize::new(10, 10));
    let tile_info = core.tile_info.as_mut().unwrap();
    tile_info.mi_col_starts = vec![0, 4];
    tile_info.mi_row_starts = vec![0, 4];
    let block = IntrabcBlockContext::new(2, 0, 2, false);
    let geometry = IntrabcBlockGeometry::new(block, 4, 2);
    let info = IntrabcInfo {
        intrabc_mode: 1,
        ref_mv_idx: 0,
        mv_precision: MV_PRECISION_QUARTER_PEL,
        block_mv: IntrabcBlockVector { row: -64, col: 0 },
    };

    let prediction =
        derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
            .unwrap();

    assert_eq!(prediction.target, PlaneRect::new(0, 8, 16, 8).unwrap());
    assert_eq!(prediction.source, PlaneRect::new(0, 0, 16, 8).unwrap());
    assert!(!prediction.fractional);
    assert_eq!(prediction.scaling.start_x >> 10, 0);
    assert_eq!(prediction.scaling.start_y >> 10, 0);
}

#[test]
fn intrabc_geometry_rejects_source_outside_current_tile() {
    let (_, mut core) = selectable_large_frame_fixture();
    let tile_info = core.tile_info.as_mut().unwrap();
    tile_info.mi_col_starts = vec![0, 4, 8];
    tile_info.mi_row_starts = vec![0, 8];
    let block = IntrabcBlockContext::new(4, 4, 2, false);
    let geometry = IntrabcBlockGeometry::new(block, 4, 4);
    let info = IntrabcInfo {
        intrabc_mode: 1,
        ref_mv_idx: 0,
        mv_precision: MV_PRECISION_QUARTER_PEL,
        block_mv: IntrabcBlockVector { row: 0, col: -128 },
    };

    let error = derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
        .unwrap_err();

    assert_eq!(
        unsupported_reason(error),
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_source_bounds"
    );
}

#[test]
fn intrabc_geometry_rejects_self_referential_source() {
    let (_, core) = selectable_large_frame_fixture();
    let block = IntrabcBlockContext::new(8, 8, 2, false);
    let geometry = IntrabcBlockGeometry::new(block, 4, 4);
    let info = IntrabcInfo {
        intrabc_mode: 1,
        ref_mv_idx: 0,
        mv_precision: MV_PRECISION_QUARTER_PEL,
        block_mv: IntrabcBlockVector { row: 0, col: 0 },
    };

    let error = derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
        .unwrap_err();

    assert_eq!(
        unsupported_reason(error),
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_mv_validity"
    );
}

#[test]
fn intrabc_geometry_rejects_out_of_frame_source() {
    let (_, core) = selectable_large_frame_fixture();
    let block = IntrabcBlockContext::new(0, 0, 2, false);
    let geometry = IntrabcBlockGeometry::new(block, 4, 4);
    let info = IntrabcInfo {
        intrabc_mode: 1,
        ref_mv_idx: 0,
        mv_precision: MV_PRECISION_QUARTER_PEL,
        block_mv: IntrabcBlockVector { row: -512, col: 0 },
    };

    let error = derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
        .unwrap_err();

    assert_eq!(
        unsupported_reason(error),
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_source_bounds"
    );
}

#[test]
fn intrabc_geometry_rejects_out_of_frame_target() {
    let (_, core) = selectable_large_frame_fixture();
    let block = IntrabcBlockContext::new(32, 0, 2, false);
    let geometry = IntrabcBlockGeometry::new(block, 4, 4);
    let info = IntrabcInfo {
        intrabc_mode: 1,
        ref_mv_idx: 0,
        mv_precision: MV_PRECISION_QUARTER_PEL,
        block_mv: IntrabcBlockVector { row: 0, col: 0 },
    };

    let error = derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
        .unwrap_err();

    assert_eq!(
        unsupported_reason(error),
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_target_bounds"
    );
}

/// A bottom-edge IntrABC block whose NOMINAL target footprint overhangs the frame
/// bottom edge (the §6.19.7.12 `intrabc_target_bounds` frontier) is now ADMITTED
/// with an EFFECTIVE in-frame target clamped to the visible region, modelling AVM
/// §5.20.3.2 `block_coded`. The frame is 10x10 (4x4 MI, 16x16 luma storage); a
/// `BLOCK_16X64`-shaped block (`n4h == 4`, 16 luma rows nominal) at MI(row=2,col=0)
/// has its nominal y[8,24) target clamped to the in-frame y[8,16) — an EFFECTIVE
/// 16x8 target — and an integer DV(row=-64) gives a CONGRUENT in-frame 16x8 source
/// at y[0,8). The block is NOT rejected `intrabc_target_bounds`; the clamp only
/// shrinks (source/target stay the same congruent shape).
#[test]
fn intrabc_geometry_clamps_bottom_edge_overhang_target_to_visible_region() {
    let (_, mut core) = selectable_fixture();
    core.frame_size = Some(FrameSize::new(10, 10));
    let tile_info = core.tile_info.as_mut().unwrap();
    tile_info.mi_col_starts = vec![0, 4];
    tile_info.mi_row_starts = vec![0, 4];
    let block = IntrabcBlockContext::new(2, 0, 2, false);
    let geometry = IntrabcBlockGeometry::new(block, 4, 4);
    let info = IntrabcInfo {
        intrabc_mode: 1,
        ref_mv_idx: 0,
        mv_precision: MV_PRECISION_QUARTER_PEL,
        block_mv: IntrabcBlockVector { row: -64, col: 0 },
    };

    let prediction =
        derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
            .unwrap();

    assert_eq!(prediction.target, PlaneRect::new(0, 8, 16, 8).unwrap());
    assert_eq!(prediction.source, PlaneRect::new(0, 0, 16, 8).unwrap());
    assert!(!prediction.fractional);
    assert_eq!(prediction.target.size(), prediction.source.size());
}

/// A genuinely off-frame IntrABC block whose TOP-LEFT MI row is at/after the frame
/// MI-row count (no visible samples at all, not an overhang) is still rejected
/// `intrabc_target_bounds` — the clamp admits overhangs, never an off-frame top-left.
#[test]
fn intrabc_geometry_rejects_off_frame_top_left_block() {
    let (_, mut core) = selectable_fixture();
    core.frame_size = Some(FrameSize::new(10, 10));
    let tile_info = core.tile_info.as_mut().unwrap();
    tile_info.mi_col_starts = vec![0, 4];
    tile_info.mi_row_starts = vec![0, 4];
    let block = IntrabcBlockContext::new(4, 0, 2, false);
    let geometry = IntrabcBlockGeometry::new(block, 4, 4);
    let info = IntrabcInfo {
        intrabc_mode: 1,
        ref_mv_idx: 0,
        mv_precision: MV_PRECISION_QUARTER_PEL,
        block_mv: IntrabcBlockVector { row: -64, col: 0 },
    };

    let error = derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
        .unwrap_err();

    assert_eq!(
        unsupported_reason(error),
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_target_bounds"
    );
}

#[test]
fn intrabc_geometry_rejects_missing_frame_size() {
    let (_, mut core) = selectable_large_frame_fixture();
    core.frame_size = None;
    let block = IntrabcBlockContext::new(8, 8, 2, false);
    let geometry = IntrabcBlockGeometry::new(block, 4, 4);
    let info = IntrabcInfo {
        intrabc_mode: 1,
        ref_mv_idx: 0,
        mv_precision: MV_PRECISION_QUARTER_PEL,
        block_mv: IntrabcBlockVector { row: 0, col: 0 },
    };

    let error = derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
        .unwrap_err();

    assert_eq!(
        unsupported_reason(error),
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_frame_size"
    );
}

#[test]
fn intrabc_newmv_one_pel_record_shifts_shell_delta() {
    let (sequence, mut core) = selectable_fixture();
    core.force_integer_mv = Some(true);
    let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let payload = encode_steps(&[
        (Some(TileCdfSelector::IntrabcMode), 0),
        (None, 0),
        (
            Some(TileCdfSelector::ReadMv(MvCdfSelector::JointShellSet {
                mv_ctx: 1,
            })),
            0,
        ),
        (
            Some(TileCdfSelector::ReadMv(MvCdfSelector::JointShellClass {
                precision: usize::from(MV_PRECISION_ONE_PEL),
                shell_set: 0,
                mv_ctx: 1,
            })),
            0,
        ),
        (
            Some(TileCdfSelector::ReadMv(
                MvCdfSelector::ShellOffsetLowClass {
                    mv_ctx: 1,
                    shell_class: 0,
                },
            )),
            1,
        ),
        (
            Some(TileCdfSelector::ReadMv(MvCdfSelector::ColMvIndex {
                mv_ctx: 1,
                ctx: 0,
            })),
            0,
        ),
        (None, 0),
    ]);
    let mut symbols = decoder(&payload);
    let block = IntrabcBlockContext::new(0, 0, 2, false);
    let geometry = IntrabcBlockGeometry::new(block, 4, 4);

    let info = read_intrabc_info_record(
        &mut cdfs,
        &mut symbols,
        &sequence,
        &core,
        geometry,
        ByteOffset::new(20),
    )
    .unwrap();

    assert_eq!(
        info,
        IntrabcInfo {
            intrabc_mode: 0,
            ref_mv_idx: 0,
            mv_precision: MV_PRECISION_ONE_PEL,
            block_mv: IntrabcBlockVector { row: -504, col: 0 },
        }
    );
    assert_eq!(symbols.symbol_count(), 6);
}

#[test]
fn intrabc_newmv_read_errors_use_intrabc_frontier_diagnostic() {
    let (_sequence, _) = selectable_fixture();
    let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let payload = [];
    let mut symbols = decoder(&payload);

    let error = assign_intrabc_mv(
        &mut cdfs,
        &mut symbols,
        0,
        crate::runtime_minimal::inter::read_mv::MV_PRECISION_TWO_PEL,
        Mv { row: 0, col: 0 },
        ByteOffset::new(20),
    )
    .unwrap_err();

    assert_eq!(
        unsupported_reason(error),
        "unsupported_wienerns_lr_selectable_transform_records_intrabc_newmv"
    );
}

#[test]
fn non_intrabc_path_reads_only_use_intrabc_symbol() {
    let (_, core) = selectable_fixture();
    let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let payload = encode_steps(&[(Some(TileCdfSelector::Intrabc { ctx: 0 }), 0)]);
    let mut symbols = decoder(&payload);
    let state = state();
    let block = IntrabcBlockContext::new(0, 0, 2, false);
    let geometry = IntrabcBlockGeometry::new(block, 4, 4);

    let use_skip = read_intrabc_use_and_skip(
        &mut cdfs,
        &mut symbols,
        &state,
        &core,
        geometry,
        ByteOffset::new(20),
    )
    .unwrap();

    assert_eq!(
        use_skip,
        IntrabcUseSkip {
            use_intrabc: false,
            skip_flag: false,
        }
    );
    assert_eq!(symbols.symbol_count(), 1);
}

#[test]
fn contexts_use_intrabc_npos_and_skip_nposbuf_boundaries() {
    let mut state = state();
    let ordinary = IntrabcBlockPrelude {
        use_intrabc: false,
        is_inter: false,
        skip_flag: false,
        intrabc: None,
    };
    let intrabc_skip = ac0ej3_skip_neighbour();
    state
        .record_block(15, 4, 4, 1, intrabc_skip, ByteOffset::new(0))
        .unwrap();
    state
        .record_block(15, 8, 4, 1, intrabc_skip, ByteOffset::new(0))
        .unwrap();
    state
        .record_block(16, 3, 1, 4, ordinary, ByteOffset::new(0))
        .unwrap();
    state
        .record_block(16, 4, 4, 4, intrabc_skip, ByteOffset::new(0))
        .unwrap();

    assert_eq!(
        state.intrabc_ctx(16, 8, 4, 4, ByteOffset::new(0)).unwrap(),
        2
    );
    assert_eq!(state.skip_ctx(16, 8, 4, 4, ByteOffset::new(0)).unwrap(), 2);
}

#[test]
fn contexts_stop_after_first_two_valid_neighbour_candidates() {
    let mut state = state();
    let ordinary = IntrabcBlockPrelude {
        use_intrabc: false,
        is_inter: false,
        skip_flag: false,
        intrabc: None,
    };
    let intrabc_skip = ac0ej3_skip_neighbour();
    state
        .record_block(23, 7, 1, 1, ordinary, ByteOffset::new(0))
        .unwrap();
    state
        .record_block(19, 11, 1, 1, ordinary, ByteOffset::new(0))
        .unwrap();
    state
        .record_block(20, 7, 1, 1, intrabc_skip, ByteOffset::new(0))
        .unwrap();
    state
        .record_block(19, 8, 1, 1, intrabc_skip, ByteOffset::new(0))
        .unwrap();

    assert_eq!(
        state.intrabc_ctx(20, 8, 4, 4, ByteOffset::new(0)).unwrap(),
        0
    );
    assert_eq!(state.skip_ctx(20, 8, 4, 4, ByteOffset::new(0)).unwrap(), 0);
}

#[test]
fn contexts_preserve_duplicate_neighbour_slots_before_cap() {
    let mut state = state();
    let intrabc_skip = ac0ej3_skip_neighbour();
    state
        .record_block(0, 7, 1, 1, intrabc_skip, ByteOffset::new(0))
        .unwrap();

    assert_eq!(
        state.intrabc_ctx(0, 8, 4, 1, ByteOffset::new(0)).unwrap(),
        2
    );
    assert_eq!(state.skip_ctx(0, 8, 4, 1, ByteOffset::new(0)).unwrap(), 2);
}

#[test]
fn intrabc_ref_stack_caps_256_sequence_superblocks_to_intra_sb_size() {
    use super::super::intrabc_ref_mv_stack::build_intrabc_ref_mv_stack;
    let (mut sequence, _) = selectable_fixture();
    let partition = sequence.partition.as_mut().unwrap();
    partition.use_256x256_superblock = true;
    partition.use_128x128_superblock = false;
    let stack_geometry = IntrabcStackGeometry {
        mi_row: 0,
        mi_col: 0,
        n4w: 4,
        n4h: 4,
        sb_samples: superblock_samples(&sequence, no_off()).unwrap(),
        frame_w: i32::MAX,
        frame_h: i32::MAX,
        max_bvp_drl_bits_minus_1: 2,
    };
    let candidates =
        build_intrabc_ref_mv_stack(&IntrabcRefMvBank::new(0), stack_geometry, false, &[]);

    assert_eq!(
        candidates,
        vec![
            Mv { row: -1024, col: 0 },
            Mv { row: 0, col: -3072 },
            Mv { row: -128, col: 0 },
            Mv { row: 0, col: -128 },
        ]
    );
}

#[test]
fn local_intrabc_range_admits_ac0ej3_first_block() {
    assert!(local_intrabc_range_valid(IntrabcLocalRangeInputs {
        mi_row: 16,
        mi_col: 56,
        block_w: 32,
        block_h: 64,
        dv_row: -512,
        dv_col: 0,
        sb_size: 128,
    }));
}

#[test]
fn local_intrabc_range_rejects_uncoded_bottom_right_source() {
    assert!(!local_intrabc_range_valid(IntrabcLocalRangeInputs {
        mi_row: 16,
        mi_col: 56,
        block_w: 32,
        block_h: 64,
        dv_row: 8,
        dv_col: 8,
        sb_size: 128,
    }));
}

#[test]
fn local_intrabc_range_rejects_source_beyond_left_buffer_window() {
    assert!(!local_intrabc_range_valid(IntrabcLocalRangeInputs {
        mi_row: 64,
        mi_col: 64,
        block_w: 32,
        block_h: 32,
        dv_row: 0,
        dv_col: -3072,
        sb_size: 128,
    }));
}

#[test]
fn local_intrabc_range_rejects_previous_superblock_buffer_collision_source() {
    assert!(!local_intrabc_range_valid(IntrabcLocalRangeInputs {
        mi_row: 0,
        mi_col: 68,
        block_w: 32,
        block_h: 32,
        dv_row: 0,
        dv_col: -128 * 8,
        sb_size: 128,
    }));
}

#[test]
fn proven_valid_defers_when_local_intrabc_disabled() {
    let (sequence, mut core) = selectable_large_frame_fixture();
    core.intrabc = Some(IntrabcParams {
        allow_intrabc: true,
        allow_global_intrabc: Some(true),
        allow_local_intrabc: Some(false),
        change_bvp_drl: Some(false),
        max_bvp_drl_bits_minus_1: None,
    });
    let geometry = IntrabcBlockGeometry::new(IntrabcBlockContext::new(20, 0, 2, false), 4, 4);
    let info = IntrabcInfo {
        intrabc_mode: 1,
        ref_mv_idx: 0,
        mv_precision: MV_PRECISION_QUARTER_PEL,
        block_mv: IntrabcBlockVector { row: -512, col: 0 },
    };
    assert!(
        !intrabc_dv_proven_valid(&sequence, &core, geometry, info, ByteOffset::new(20)).unwrap()
    );
}

#[test]
fn proven_valid_admits_inferred_local_intrabc_frame() {
    let (sequence, mut core) = selectable_large_frame_fixture();
    core.intrabc = Some(IntrabcParams {
        allow_intrabc: true,
        allow_global_intrabc: Some(false),
        allow_local_intrabc: None,
        change_bvp_drl: Some(false),
        max_bvp_drl_bits_minus_1: None,
    });
    let geometry = IntrabcBlockGeometry::new(IntrabcBlockContext::new(4, 4, 2, false), 4, 4);
    let info = IntrabcInfo {
        intrabc_mode: 1,
        ref_mv_idx: 0,
        mv_precision: MV_PRECISION_QUARTER_PEL,
        block_mv: IntrabcBlockVector { row: -128, col: 0 },
    };
    assert!(
        intrabc_dv_proven_valid(&sequence, &core, geometry, info, ByteOffset::new(20)).unwrap()
    );
}

#[test]
fn resolve_allow_global_intrabc_only_admits_explicit_true() {
    fn check(allow_intrabc: bool, global: Option<bool>) -> bool {
        let (_, mut core) = selectable_large_frame_fixture();
        core.intrabc = Some(IntrabcParams {
            allow_intrabc,
            allow_global_intrabc: global,
            allow_local_intrabc: None,
            change_bvp_drl: Some(false),
            max_bvp_drl_bits_minus_1: None,
        });
        resolve_allow_global_intrabc(&core)
    }
    assert!(check(true, Some(true)));
    assert!(!check(true, Some(false)));
    assert!(!check(true, None));
    assert!(!check(false, Some(true)));
}

#[test]
fn global_superblock_samples_returns_true_sb_size() {
    let (sequence, _core) = selectable_fixture();
    assert_eq!(global_superblock_samples(&sequence, no_off()).unwrap(), 64);
}

#[test]
fn intrabc_tile_total_sb64_per_row_matches_avm() {
    let (_, mut core) = selectable_large_frame_fixture();
    let tile_info = core.tile_info.as_mut().unwrap();
    tile_info.mi_col_starts = vec![0, 480];
    tile_info.mi_row_starts = vec![0, 272];
    let geometry =
        IntrabcBlockGeometry::new(IntrabcBlockContext::new(256, 56, BLOCK_16X16, false), 4, 16);
    assert_eq!(
        intrabc_tile_total_sb64_per_row(&core, geometry, no_off()).unwrap(),
        30
    );
}

#[test]
fn global_intrabc_range_matches_avm_wavefront_vectors() {
    type Vector = (
        &'static str,
        (usize, usize),
        (usize, usize),
        (i32, i32),
        usize,
        bool,
    );
    let vectors: &[Vector] = &[
        (
            "prev-SB-row wavefront source",
            (256, 56),
            (16, 64),
            (-1024, 0),
            128,
            true,
        ),
        (
            "same-row far-enough source",
            (0, 256),
            (16, 16),
            (0, -2560),
            128,
            true,
        ),
        (
            "one SB64 too close",
            (0, 256),
            (16, 16),
            (0, -2048),
            128,
            false,
        ),
        (
            "source in a later SB row",
            (0, 256),
            (16, 16),
            (1024, -2560),
            128,
            false,
        ),
        (
            "bottom-left-128 residual-sensitive",
            (16, 0),
            (16, 16),
            (0, -2168),
            128,
            false,
        ),
        (
            "bottom-left-128 both horizons clear",
            (16, 0),
            (16, 16),
            (0, -3584),
            128,
            true,
        ),
        // A non-{64,128,256} SB size is rejected fail-closed.
        (
            "unsupported SB size",
            (0, 256),
            (16, 16),
            (0, -2560),
            96,
            false,
        ),
    ];
    for &(label, (mi_row, mi_col), (block_w, block_h), (dv_row, dv_col), sb_size, expect) in vectors
    {
        let got = global_intrabc_range_valid(IntrabcGlobalRangeInputs {
            mi_row,
            mi_col,
            block_w,
            block_h,
            dv_row,
            dv_col,
            sb_size,
            total_sb64_per_row: 30,
        });
        assert_eq!(
            got, expect,
            "global wavefront vector {label}: want {expect}, got {got}"
        );
    }
}

#[test]
fn record_block_clamps_bottom_edge_overhang_to_in_frame_cells() {
    let (sequence, _core) = selectable_fixture();
    let mut state = TileIntrabcPreludeState::new(4, 4, &sequence, no_off()).unwrap();
    state
        .record_block(2, 0, 2, 4, ac0ej3_skip_neighbour(), no_off())
        .unwrap();

    for r in 2..4 {
        for c in 0..2 {
            assert!(
                state.value(r, c, no_off()).unwrap().is_some(),
                "in-frame MI cell ({r},{c}) must record the block"
            );
        }
    }
    assert!(state.value(0, 0, no_off()).unwrap().is_none());
    assert!(state.value(3, 3, no_off()).unwrap().is_none());
}

#[test]
fn record_block_clamps_right_edge_overhang_to_in_frame_cells() {
    let (sequence, _core) = selectable_fixture();
    let mut state = TileIntrabcPreludeState::new(4, 4, &sequence, no_off()).unwrap();
    state
        .record_block(0, 2, 4, 1, ac0ej3_skip_neighbour(), no_off())
        .unwrap();

    for c in 2..4 {
        assert!(
            state.value(0, c, no_off()).unwrap().is_some(),
            "in-frame MI cell (0,{c}) must record the block"
        );
    }
    assert!(state.value(0, 1, no_off()).unwrap().is_none());
    assert!(state.value(1, 2, no_off()).unwrap().is_none());
}
