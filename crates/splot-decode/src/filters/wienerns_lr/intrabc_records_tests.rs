// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unit tests for the bounded IntrABC syntax handoff ([`super`]).

use splot_core::headers::frame::{
    FrameSize, InterControl, IntrabcParams, MvPrecision, TxMode, build_minimal_intra_clk_core,
    build_minimal_intra_sequence_header,
};
use splot_core::headers::sequence::DrlReorder;
use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoder, SymbolDecoderConfig};
use splot_core::symbol_encoder::SymbolEncoder;

use super::*;
use crate::bitstream::tile_payload::{FrameCdfSubset, MvCdfSelector};
use crate::error::DecodeError;
use crate::filters::wienerns_lr::intrabc_ref_mv_stack::{
    DrlReorderMode, IntrabcStackAdmission, IntrabcStackGeometry,
    build_intrabc_ref_mv_stack_from_candidates, intrabc_ref_stack_admission_from_candidates,
};

const BLOCK_16X16: usize = 6;

impl IntrabcBlockContext {
    const fn new(row: usize, col: usize, b_size: usize, is_chroma_part: bool) -> Self {
        Self {
            row,
            col,
            b_size,
            is_chroma_part,
            mixed_region: true,
        }
    }

    const fn new_with_mixed_region(
        row: usize,
        col: usize,
        b_size: usize,
        is_chroma_part: bool,
        mixed_region: bool,
    ) -> Self {
        Self {
            row,
            col,
            b_size,
            is_chroma_part,
            mixed_region,
        }
    }
}

impl IntrabcBlockGeometry {
    const fn new(block: IntrabcBlockContext, n4w: usize, n4h: usize) -> Self {
        Self { block, n4w, n4h }
    }
}

fn read_intrabc_info_record(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    state: &TileIntrabcPreludeState,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    geometry: IntrabcBlockGeometry,
    tile_offset: ByteOffset,
) -> Result<IntrabcInfo> {
    let pending =
        read_pending_intrabc_info(cdfs, symbols, state, sequence, core, geometry, tile_offset)?;
    let stack_geometry = IntrabcStackGeometry {
        mi_row: geometry.block.row,
        mi_col: geometry.block.col,
        n4w: geometry.n4w,
        n4h: geometry.n4h,
        sb_samples: i32::try_from(state.sb_size4.saturating_mul(MI_SIZE)).unwrap_or(i32::MAX),
        frame_w: i32::try_from(state.mi_cols.saturating_mul(MI_SIZE)).unwrap_or(i32::MAX),
        frame_h: i32::try_from(state.mi_rows.saturating_mul(MI_SIZE)).unwrap_or(i32::MAX),
        max_bvp_drl_bits_minus_1: pending.max_bvp_drl_bits_minus_1(),
    };
    let spatial = state
        .capture_spatial_intrabc_probes(geometry)
        .resolve(|_, _| None);
    let drl_reorder = match sequence.inter.as_ref().map(|inter| inter.drl_reorder) {
        Some(DrlReorder::Always) => DrlReorderMode::Always,
        Some(DrlReorder::Constraint) => DrlReorderMode::Constraint,
        Some(DrlReorder::Disabled) | None => DrlReorderMode::Disabled,
    };
    let enable_refmvbank = sequence
        .inter
        .as_ref()
        .is_some_and(|inter| inter.enable_refmvbank);
    let pred_mv = match intrabc_ref_stack_admission_from_candidates(
        &[],
        stack_geometry,
        &spatial,
        enable_refmvbank,
        drl_reorder,
        pending.ref_mv_idx(),
    ) {
        IntrabcStackAdmission::Admit { selected } => selected,
        IntrabcStackAdmission::Defer => {
            return Err(
                crate::error::DecodeHeaderStateError::InvalidSelectableTransformRecords.into(),
            );
        }
    };
    Ok(resolve_pending_intrabc_info(pending, pred_mv))
}

/// Entropy-neighbour facts for a skipped IntrABC block; spatial probes resolve
/// their block vector separately.
fn frontier_skip_neighbour() -> IntrabcBlockPrelude {
    IntrabcBlockPrelude {
        use_intrabc: true,
        is_inter: true,
        skip_flag: true,
        morph_pred: false,
    }
}

fn ordinary_neighbour() -> IntrabcBlockPrelude {
    IntrabcBlockPrelude {
        use_intrabc: false,
        is_inter: false,
        skip_flag: false,
        morph_pred: false,
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

fn selectable_morph_fixture() -> (SequenceHeader, FrameHeaderCore) {
    let (mut sequence, mut core) = selectable_fixture();
    sequence.inter.as_mut().unwrap().enable_bawp = true;
    core.allow_screen_content_tools = Some(true);
    (sequence, core)
}

fn assert_invalid_block_vector_error(error: &DecodeError) {
    match error {
        DecodeError::MalformedSource { issue } => {
            assert_eq!(issue.spec_section(), Some("6.19.7.12"));
        }
        other => panic!("unexpected decode error: {other:?}"),
    }
}

fn assert_state_error(error: &DecodeError) {
    assert!(matches!(
        error,
        DecodeError::HeaderState {
            source: crate::error::DecodeHeaderStateError::InvalidSelectableTransformRecords,
        }
    ));
}

fn encode_steps(steps: &[(Option<TileCdfSelector>, u32)]) -> Vec<u8> {
    let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::new();
    for &(selector, value) in steps {
        if let Some(selector) = selector {
            cdfs.with_row_mut(selector, |row| {
                encoder.write_symbol_u16(row, Symbol::new(u8::try_from(value).unwrap()))
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

fn full_frame_state(
    mi_rows: usize,
    mi_cols: usize,
    sequence: &SequenceHeader,
    frame_is_intra_only: bool,
) -> crate::Result<TileIntrabcPreludeState> {
    TileIntrabcPreludeState::new_for_tile(
        (mi_rows, mi_cols),
        0..mi_rows,
        0..mi_cols,
        sequence,
        frame_is_intra_only,
        true,
    )
}

fn state() -> TileIntrabcPreludeState {
    let (sequence, _) = selectable_fixture();
    full_frame_state(64, 64, &sequence, true).unwrap()
}

#[test]
fn resolve_force_integer_mv_from_inter_mv_precision_when_flat_mirror_missing() {
    let (_, mut core) = selectable_fixture();
    core.force_integer_mv = None;
    let mut inter = InterControl::default();
    inter.mv_precision = Some(MvPrecision::QuarterPel);
    core.inter = Some(inter);

    assert!(!resolve_intrabc_force_integer_mv(&core).unwrap());
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

/// Runs the live use/skip read, pending-info read, reference-stack admission, and
/// final resolution over `steps` on the large-frame fixture, returning the
/// decoded `use_skip`, resolved info result, and symbol count consumed.
fn run_intrabc_prelude(
    steps: &[(Option<TileCdfSelector>, u32)],
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
    let info = read_intrabc_info_record(
        &mut cdfs,
        &mut symbols,
        &state,
        &sequence,
        &core,
        geometry,
        ByteOffset::new(20),
    );
    (use_skip, info, symbols.symbol_count())
}

#[test]
fn active_intrabc_nearmv_skip_reads_use_skip_mode_and_drl_then_advances() {
    let (use_skip, info, symbol_count) = run_intrabc_prelude(&[
        (Some(TileCdfSelector::Intrabc { ctx: 0 }), 1),
        (Some(TileCdfSelector::Skip { ctx: 0 }), 1),
        (Some(TileCdfSelector::IntrabcMode), 1),
        (None, 0),
    ]);

    assert_eq!(
        use_skip,
        IntrabcUseSkip {
            use_intrabc: true,
            skip_flag: true,
        }
    );
    assert_eq!(
        info.unwrap().block_mv,
        IntrabcBlockVector { row: -512, col: 0 }
    );
    assert_eq!(symbol_count, 4);
}

#[test]
fn active_intrabc_newmv_nonskip_reads_block_vector_and_returns_info_for_residual() {
    let (use_skip, info, symbol_count) = run_intrabc_prelude(&[
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
    ]);

    assert_eq!(
        use_skip,
        IntrabcUseSkip {
            use_intrabc: true,
            skip_flag: false,
        }
    );
    let info = info.expect("non-skip IntrABC prelude returns parsed mode-info");
    assert_eq!(info.block_mv, IntrabcBlockVector { row: -512, col: 0 });
    assert_eq!(symbol_count, 8);
}

#[test]
fn intrabc_morph_pred_zero_reads_symbol_and_advances() {
    let (sequence, core) = selectable_morph_fixture();
    let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let payload = encode_steps(&[
        (Some(TileCdfSelector::IntrabcMode), 1),
        (None, 0),
        (Some(TileCdfSelector::MorphPred { ctx: 0 }), 0),
    ]);
    let mut symbols = decoder(&payload);
    let state = full_frame_state(64, 64, &sequence, true).unwrap();
    let geometry = IntrabcBlockGeometry::new(IntrabcBlockContext::new(20, 0, 2, false), 4, 4);

    let info = read_intrabc_info_record(
        &mut cdfs,
        &mut symbols,
        &state,
        &sequence,
        &core,
        geometry,
        ByteOffset::new(20),
    )
    .unwrap();

    assert!(!info.morph_pred);
    assert_eq!(info.block_mv, IntrabcBlockVector { row: -512, col: 0 });
    assert_eq!(symbols.symbol_count(), 3);
}

#[test]
fn intrabc_morph_pred_one_is_retained_for_reconstruction() {
    let (sequence, core) = selectable_morph_fixture();
    let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let payload = encode_steps(&[
        (Some(TileCdfSelector::IntrabcMode), 1),
        (None, 0),
        (Some(TileCdfSelector::MorphPred { ctx: 0 }), 1),
    ]);
    let mut symbols = decoder(&payload);
    let state = full_frame_state(64, 64, &sequence, true).unwrap();
    let geometry = IntrabcBlockGeometry::new(IntrabcBlockContext::new(20, 0, 2, false), 4, 4);

    let info = read_intrabc_info_record(
        &mut cdfs,
        &mut symbols,
        &state,
        &sequence,
        &core,
        geometry,
        ByteOffset::new(20),
    )
    .unwrap();

    assert!(info.morph_pred);
    assert_eq!(info.block_mv, IntrabcBlockVector { row: -512, col: 0 });
    assert_eq!(symbols.symbol_count(), 3);
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
    let mut state = full_frame_state(64, 64, &sequence, true).unwrap();
    let neighbour = |state: &mut TileIntrabcPreludeState, row, col| {
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
                    morph_pred: false,
                },
            )
            .unwrap();
    };
    neighbour(&mut state, 23, 7);
    neighbour(&mut state, 19, 8);
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
    let pending = read_pending_intrabc_info(
        &mut cdfs,
        &mut symbols,
        &state,
        &sequence,
        &core,
        geometry,
        ByteOffset::new(20),
    )
    .unwrap();
    let spatial = state
        .capture_spatial_intrabc_probes(geometry)
        .resolve(|row, col| match (row, col) {
            (23, 7) => Some(Mv { row: -512, col: 0 }),
            (19, 8) => Some(Mv { row: 0, col: -3072 }),
            _ => None,
        });
    let admission = intrabc_ref_stack_admission_from_candidates(
        &[],
        IntrabcStackGeometry {
            mi_row: geometry.block.row,
            mi_col: geometry.block.col,
            n4w: geometry.n4w,
            n4h: geometry.n4h,
            sb_samples: i32::try_from(state.sb_size4 * MI_SIZE).unwrap(),
            frame_w: i32::try_from(state.mi_cols * MI_SIZE).unwrap(),
            frame_h: i32::try_from(state.mi_rows * MI_SIZE).unwrap(),
            max_bvp_drl_bits_minus_1: pending.max_bvp_drl_bits_minus_1(),
        },
        &spatial,
        true,
        DrlReorderMode::Always,
        pending.ref_mv_idx(),
    );
    let IntrabcStackAdmission::Admit { selected } = admission else {
        panic!("two live spatial candidates must admit the requested IntrABC reference")
    };
    let info = resolve_pending_intrabc_info(pending, selected);

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
fn spatial_scan_admits_sb_border_col_minus_two_neighbour() {
    let (sequence, _) = selectable_large_frame_fixture();
    let neighbour = frontier_skip_neighbour(); // BV (-512, 0).
    let mut sb_border = full_frame_state(64, 64, &sequence, true).unwrap();
    sb_border.record_block(15, 54, 1, 1, neighbour).unwrap();
    let at_border =
        IntrabcBlockGeometry::new(IntrabcBlockContext::new(16, 56, BLOCK_16X16, false), 8, 16);
    let scan = sb_border
        .capture_spatial_intrabc_probes(at_border)
        .resolve(|row, col| (row == 15 && col == 54).then_some(Mv { row: -512, col: 0 }));
    assert_eq!(
        scan.candidates,
        vec![super::super::intrabc_ref_mv_stack::WeightedBv {
            mv: Mv { row: -512, col: 0 },
            weight: 0,
        }]
    );

    let mut interior = full_frame_state(64, 64, &sequence, true).unwrap();
    interior.record_block(19, 54, 1, 1, neighbour).unwrap();
    let at_interior =
        IntrabcBlockGeometry::new(IntrabcBlockContext::new(20, 56, BLOCK_16X16, false), 8, 16);
    let control = interior
        .capture_spatial_intrabc_probes(at_interior)
        .resolve(|row, col| (row == 19 && col == 54).then_some(Mv { row: -512, col: 0 }));
    assert!(control.candidates.is_empty());
}

#[test]
fn sequence_256_intrabc_context_uses_the_frame_superblock_size() {
    let (mut sequence, _) = selectable_large_frame_fixture();
    let partition = sequence.partition.as_mut().unwrap();
    partition.use_256x256_superblock = true;
    partition.use_128x128_superblock = false;
    let neighbour = frontier_skip_neighbour();
    let mut intra = full_frame_state(64, 64, &sequence, true).unwrap();
    let mut inter = full_frame_state(64, 64, &sequence, false).unwrap();
    intra.record_block(31, 11, 1, 1, neighbour).unwrap();
    inter.record_block(31, 11, 1, 1, neighbour).unwrap();

    assert_eq!(intra.sb_size4, 32);
    assert_eq!(inter.sb_size4, 64);
    assert_eq!(intra.intrabc_ctx(32, 8, 4, 4).unwrap(), 0);
    assert_eq!(inter.intrabc_ctx(32, 8, 4, 4).unwrap(), 1);
    assert_eq!(intra.skip_ctx(32, 8, 4, 4).unwrap(), 1);
    assert_eq!(inter.skip_ctx(32, 8, 4, 4).unwrap(), 1);
}

#[test]
fn sequence_256_intrabc_spatial_probe_uses_the_frame_superblock_size() {
    let (mut sequence, _) = selectable_large_frame_fixture();
    let partition = sequence.partition.as_mut().unwrap();
    partition.use_256x256_superblock = true;
    partition.use_128x128_superblock = false;
    let neighbour = frontier_skip_neighbour();
    let mut intra = full_frame_state(64, 64, &sequence, true).unwrap();
    let mut inter = full_frame_state(64, 64, &sequence, false).unwrap();
    intra.record_block(31, 11, 1, 1, neighbour).unwrap();
    inter.record_block(31, 11, 1, 1, neighbour).unwrap();
    let geometry =
        IntrabcBlockGeometry::new(IntrabcBlockContext::new(32, 8, BLOCK_16X16, false), 4, 4);

    let intra_scan = intra
        .capture_spatial_intrabc_probes(geometry)
        .resolve(|row, col| (row == 31 && col == 11).then_some(Mv { row: -512, col: 0 }));
    let inter_scan = inter
        .capture_spatial_intrabc_probes(geometry)
        .resolve(|row, col| (row == 31 && col == 11).then_some(Mv { row: -512, col: 0 }));

    assert!(intra_scan.candidates.is_empty());
    assert_eq!(
        inter_scan.candidates,
        vec![super::super::intrabc_ref_mv_stack::WeightedBv {
            mv: Mv { row: -512, col: 0 },
            weight: 1,
        }]
    );
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
    let state = full_frame_state(32, 32, &sequence, true).unwrap();

    let info = read_intrabc_info_record(
        &mut cdfs,
        &mut symbols,
        &state,
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
    let state = full_frame_state(32, 32, &sequence, true).unwrap();

    let info = read_intrabc_info_record(
        &mut cdfs,
        &mut symbols,
        &state,
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
        morph_pred: false,
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
        morph_pred: false,
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
        morph_pred: false,
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
        morph_pred: false,
        block_mv: IntrabcBlockVector { row: 0, col: -128 },
    };

    let error = derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
        .unwrap_err();

    assert_invalid_block_vector_error(&error);
}

#[test]
fn intrabc_geometry_derives_overlapping_integer_copy_region() {
    let (_, core) = selectable_large_frame_fixture();
    let block = IntrabcBlockContext::new(8, 8, 2, false);
    let geometry = IntrabcBlockGeometry::new(block, 4, 4);
    let info = IntrabcInfo {
        morph_pred: false,
        block_mv: IntrabcBlockVector { row: 0, col: 0 },
    };

    let prediction =
        derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
            .unwrap();

    assert_eq!(prediction.source, prediction.target);
    assert!(!prediction.fractional);
}

#[test]
fn intrabc_geometry_rejects_out_of_frame_source() {
    let (_, core) = selectable_large_frame_fixture();
    let block = IntrabcBlockContext::new(0, 0, 2, false);
    let geometry = IntrabcBlockGeometry::new(block, 4, 4);
    let info = IntrabcInfo {
        morph_pred: false,
        block_mv: IntrabcBlockVector { row: -512, col: 0 },
    };

    let error = derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
        .unwrap_err();

    assert_invalid_block_vector_error(&error);
}

#[test]
fn intrabc_geometry_rejects_out_of_frame_target() {
    let (_, core) = selectable_large_frame_fixture();
    let block = IntrabcBlockContext::new(32, 0, 2, false);
    let geometry = IntrabcBlockGeometry::new(block, 4, 4);
    let info = IntrabcInfo {
        morph_pred: false,
        block_mv: IntrabcBlockVector { row: 0, col: 0 },
    };

    let error = derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
        .unwrap_err();

    assert_state_error(&error);
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
        morph_pred: false,
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
        morph_pred: false,
        block_mv: IntrabcBlockVector { row: -64, col: 0 },
    };

    let error = derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
        .unwrap_err();

    assert_state_error(&error);
}

#[test]
fn intrabc_geometry_rejects_missing_frame_size() {
    let (_, mut core) = selectable_large_frame_fixture();
    core.frame_size = None;
    let block = IntrabcBlockContext::new(8, 8, 2, false);
    let geometry = IntrabcBlockGeometry::new(block, 4, 4);
    let info = IntrabcInfo {
        morph_pred: false,
        block_mv: IntrabcBlockVector { row: 0, col: 0 },
    };

    let error = derive_intrabc_luma_prediction_geometry(&core, geometry, info, ByteOffset::new(20))
        .unwrap_err();

    assert_state_error(&error);
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
    let state = full_frame_state(64, 64, &sequence, true).unwrap();

    let info = read_intrabc_info_record(
        &mut cdfs,
        &mut symbols,
        &state,
        &sequence,
        &core,
        geometry,
        ByteOffset::new(20),
    )
    .unwrap();

    assert_eq!(
        info,
        IntrabcInfo {
            morph_pred: false,
            block_mv: IntrabcBlockVector { row: -504, col: 0 },
        }
    );
    assert_eq!(symbols.symbol_count(), 6);
}

#[test]
fn intrabc_newmv_one_pel_lowers_predictor_before_delta() {
    let (mut sequence, mut core) = selectable_fixture();
    sequence.inter.as_mut().unwrap().enable_refmvbank = true;
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
    let geometry = IntrabcBlockGeometry::new(IntrabcBlockContext::new(20, 20, 2, false), 4, 4);
    let state = full_frame_state(64, 64, &sequence, true).unwrap();

    let pending = read_pending_intrabc_info(
        &mut cdfs,
        &mut symbols,
        &state,
        &sequence,
        &core,
        geometry,
        ByteOffset::new(20),
    )
    .unwrap();
    let spatial = state
        .capture_spatial_intrabc_probes(geometry)
        .resolve(|_, _| None);
    let admission = intrabc_ref_stack_admission_from_candidates(
        &[Mv { row: 4, col: -316 }],
        IntrabcStackGeometry {
            mi_row: geometry.block.row,
            mi_col: geometry.block.col,
            n4w: geometry.n4w,
            n4h: geometry.n4h,
            sb_samples: i32::try_from(state.sb_size4 * MI_SIZE).unwrap(),
            frame_w: i32::try_from(state.mi_cols * MI_SIZE).unwrap(),
            frame_h: i32::try_from(state.mi_rows * MI_SIZE).unwrap(),
            max_bvp_drl_bits_minus_1: pending.max_bvp_drl_bits_minus_1(),
        },
        &spatial,
        true,
        DrlReorderMode::Disabled,
        pending.ref_mv_idx(),
    );
    let IntrabcStackAdmission::Admit { selected } = admission else {
        panic!("the live bank candidate must admit the requested IntrABC reference")
    };
    let info = resolve_pending_intrabc_info(pending, selected);

    assert_eq!(info.block_mv, IntrabcBlockVector { row: 0, col: -312 });
    assert_eq!(symbols.symbol_count(), 5);
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

fn assert_neighbour_contexts(
    blocks: &[(usize, usize, usize, usize, IntrabcBlockPrelude)],
    probe: (usize, usize, usize, usize),
    expected: usize,
) {
    let mut state = state();
    for &(row, col, n4w, n4h, prelude) in blocks {
        state.record_block(row, col, n4w, n4h, prelude).unwrap();
    }
    let (row, col, n4w, n4h) = probe;
    assert_eq!(state.intrabc_ctx(row, col, n4w, n4h).unwrap(), expected);
    assert_eq!(state.skip_ctx(row, col, n4w, n4h).unwrap(), expected);
}

#[test]
fn contexts_use_intrabc_npos_and_skip_nposbuf_boundaries() {
    let ordinary = ordinary_neighbour();
    let intrabc_skip = frontier_skip_neighbour();
    assert_neighbour_contexts(
        &[
            (15, 4, 4, 1, intrabc_skip),
            (15, 8, 4, 1, intrabc_skip),
            (16, 3, 1, 4, ordinary),
            (16, 4, 4, 4, intrabc_skip),
        ],
        (16, 8, 4, 4),
        2,
    );
}

#[test]
fn contexts_stop_after_first_two_valid_neighbour_candidates() {
    let ordinary = ordinary_neighbour();
    let intrabc_skip = frontier_skip_neighbour();
    assert_neighbour_contexts(
        &[
            (23, 7, 1, 1, ordinary),
            (19, 11, 1, 1, ordinary),
            (20, 7, 1, 1, intrabc_skip),
            (19, 8, 1, 1, intrabc_skip),
        ],
        (20, 8, 4, 4),
        0,
    );
}

#[test]
fn contexts_preserve_duplicate_neighbour_slots_before_cap() {
    assert_neighbour_contexts(&[(0, 7, 1, 1, frontier_skip_neighbour())], (0, 8, 4, 1), 2);
}

#[test]
fn intrabc_ref_stack_caps_256_sequence_superblocks_to_intra_sb_size() {
    let (mut sequence, _) = selectable_fixture();
    let partition = sequence.partition.as_mut().unwrap();
    partition.use_256x256_superblock = true;
    partition.use_128x128_superblock = false;
    let stack_geometry = IntrabcStackGeometry {
        mi_row: 0,
        mi_col: 0,
        n4w: 4,
        n4h: 4,
        sb_samples: 128,
        frame_w: i32::MAX,
        frame_h: i32::MAX,
        max_bvp_drl_bits_minus_1: 2,
    };
    let candidates = build_intrabc_ref_mv_stack_from_candidates(&[], stack_geometry, false, &[], 0);

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
fn record_block_clamps_bottom_edge_overhang_to_in_frame_cells() {
    let (sequence, _) = selectable_fixture();
    let mut state = full_frame_state(4, 4, &sequence, true).unwrap();
    state
        .record_block(2, 0, 2, 4, frontier_skip_neighbour())
        .unwrap();

    for r in 2..4 {
        for c in 0..2 {
            assert!(
                state.value(r, c).unwrap().is_some(),
                "in-frame MI cell ({r},{c}) must record the block"
            );
        }
    }
    assert!(state.value(0, 0).unwrap().is_none());
    assert!(state.value(3, 3).unwrap().is_none());
}

#[test]
fn record_block_clamps_right_edge_overhang_to_in_frame_cells() {
    let (sequence, _) = selectable_fixture();
    let mut state = full_frame_state(4, 4, &sequence, true).unwrap();
    state
        .record_block(0, 2, 4, 1, frontier_skip_neighbour())
        .unwrap();

    for c in 2..4 {
        assert!(
            state.value(0, c).unwrap().is_some(),
            "in-frame MI cell (0,{c}) must record the block"
        );
    }
    assert!(state.value(0, 1).unwrap().is_none());
    assert!(state.value(1, 2).unwrap().is_none());
}

#[test]
fn tile_local_state_translates_absolute_coordinates() {
    let (sequence, _) = selectable_fixture();
    let mut state =
        TileIntrabcPreludeState::new_for_tile((12, 16), 4..8, 8..12, &sequence, true, true)
            .unwrap();
    assert_eq!(state.values.len(), 16);

    state
        .record_block(5, 9, 2, 2, frontier_skip_neighbour())
        .unwrap();

    for row in 5..7 {
        for col in 9..11 {
            assert!(state.value(row, col).unwrap().is_some());
        }
    }
    assert!(state.facts_at(4, 8).is_none());
    assert!(state.facts_at(5, 8).is_none());
    assert!(state.value(3, 9).is_err());
    assert!(state.value(5, 12).is_err());
}

#[test]
fn disabled_intrabc_state_skips_the_tile_grid() {
    let (sequence, _) = selectable_fixture();
    let mut state =
        TileIntrabcPreludeState::new_for_tile((270, 480), 0..270, 0..480, &sequence, false, false)
            .unwrap();

    assert!(state.values.is_empty());
    state
        .record_block(0, 0, 16, 16, frontier_skip_neighbour())
        .unwrap();
    assert!(state.values.is_empty());
}

#[test]
fn intrabc_grid_cell_stays_compact() {
    assert_eq!(core::mem::size_of::<IntrabcGridCell>(), 8);
}
