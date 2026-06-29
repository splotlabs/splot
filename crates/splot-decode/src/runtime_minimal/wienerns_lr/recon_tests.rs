// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unit tests for the ac0ej3 selectable reconstruction sink ([`super`]).

#![allow(clippy::unwrap_used)]

use super::*;
use splot_core::span::ByteOffset;

/// §3 `TxSize` index for TX_16X16 (`Tx_Width[2] == Tx_Height[2] == 16`).
const TX_16X16: usize = 2;
/// §3 `TxSize` index for TX_16X64 (`Tx_Width[17] == 16`, `Tx_Height[17] == 64`):
/// a NON-SQUARE transform.
const TX_16X64: usize = 17;
/// §3 `TxSize` index for TX_16X4 (`Tx_Width[14] == 16`, `Tx_Height[14] == 4`): a
/// single-MI-row-tall block, used to build a PARTIALLY covered reference column.
const TX_16X4: usize = 14;
/// §3 `TxSize` index for TX_16X32 (`Tx_Width[9] == 16`, `Tx_Height[9] == 32`): an
/// 8-MI-row-tall block whose left reference column spans 8 MI rows.
const TX_16X32: usize = 9;
/// §3 `TX_TYPES` index for `DCT_DCT` (`Transform_1d_Type[0] == (DCT, DCT)`).
const DCT_DCT: usize = 0;
/// §3 `TX_TYPES` index for `ADST_ADST` (`Transform_1d_Type[3] == (ADST, ADST)`).
const ADST_ADST: usize = 3;

/// A placeholder §7.13.3.17 scaling for the INTEGER-DV copy tests: the copy path
/// (`source.size() == target.size()`) never reads it (only the fractional-DV
/// bilinear path consumes `start`/`step`), so a zero scaling is inert here.
fn unused_scaling() -> PlaneScaling {
    PlaneScaling {
        start_x: 0,
        start_y: 0,
        step_x: 1024,
        step_y: 1024,
        first_x: 0,
        first_y: 0,
        last_x: 0,
        last_y: 0,
    }
}

/// An `all_zero` (`txb_skip`) DC block: reconstruction writes the bare §7.13.2
/// DC prediction (zero residual).
fn zero_block() -> LumaCoeffBlock {
    LumaCoeffBlock {
        all_zero: true,
        eob: 0,
        quant: Vec::new(),
        intra_ist: None,
        plane_tx_type: DCT_DCT,
    }
}

/// A non-`all_zero` `DCT_DCT` block with a single decoded DC coefficient `dc`
/// over an `adjusted`-entry coefficient grid (the §7.15.4.1 `Min(w,32) x
/// Min(h,32)` adjusted-size), used to exercise the non-skip reconstruction path.
fn dc_coeff_block(adjusted: usize, dc: i32) -> LumaCoeffBlock {
    let mut quant = vec![0i32; adjusted];
    quant[0] = dc;
    LumaCoeffBlock {
        all_zero: false,
        eob: 1,
        quant,
        intra_ist: None,
        plane_tx_type: DCT_DCT,
    }
}

/// A non-`all_zero` block sized for a 16x16 adjusted transform (256 entries).
fn coeff_block_16x16() -> LumaCoeffBlock {
    dc_coeff_block(256, -355)
}

fn sink() -> WienerNsLrReconSink<u16> {
    WienerNsLrReconSink::<u16>::new(64, 64, BitDepth::Ten, true, false, false, 16).unwrap()
}

#[allow(clippy::too_many_arguments)]
fn recon_luma(
    sink: &mut WienerNsLrReconSink<u16>,
    mi_col: usize,
    mi_row: usize,
    tx_size: usize,
    block: &LumaCoeffBlock,
    mode: Option<IntraYMode>,
    fsc_mode: bool,
) {
    sink.reconstruct_luma_transform(
        mi_col,
        mi_row,
        tx_size,
        block,
        mode,
        None,
        0,
        0,
        149,
        true,
        fsc_mode,
        false,
        ByteOffset::new(0),
    )
    .unwrap();
}

/// Drives a CARDINAL (`H_PRED` / `V_PRED`) directional luma transform through
/// the sink: `leaf_y_mode` is the directional mode, `directional` the resolved
/// cardinal predictor, and `mrl_index` the §5.20.5.5 multi-reference-line index
/// (`0` for the immediate edge the cardinal primitive reads).
#[allow(clippy::too_many_arguments)]
fn recon_luma_cardinal(
    sink: &mut WienerNsLrReconSink<u16>,
    mi_col: usize,
    mi_row: usize,
    tx_size: usize,
    block: &LumaCoeffBlock,
    mode: IntraYMode,
    directional: SupportedDirectionalLumaMode,
    mrl_index: u8,
) {
    sink.reconstruct_luma_transform(
        mi_col,
        mi_row,
        tx_size,
        block,
        Some(mode),
        Some(directional),
        mrl_index,
        0,
        149,
        true,
        false,
        false,
        ByteOffset::new(0),
    )
    .unwrap();
}

#[test]
fn dc_all_zero_top_left_writes_the_10bit_no_neighbour_fallback() {
    let mut sink = sink();
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X16,
        &zero_block(),
        Some(IntraYMode::DC_PRED),
        false,
    );
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 512);
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 15, 15).unwrap(), 512);
    let (luma4x4, _chroma4x4) = sink.reconstructed_counts();
    assert_eq!(luma4x4, 16);
}

#[test]
fn non_dc_luma_mode_leaves_the_region_unreconstructed() {
    let mut sink = sink();
    recon_luma(&mut sink, 0, 0, TX_16X16, &zero_block(), None, false);
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 0);
    assert_eq!(sink.reconstructed_counts().0, 0);
}

#[test]
fn dc_chroma_non_dc_mode_leaves_the_region_unreconstructed() {
    let mut sink = sink();
    sink.reconstruct_chroma_transform(
        PlaneId::U,
        TX_16X16,
        0,
        0,
        &zero_block(),
        Some(SupportedChromaMode::Smooth),
        149,
        ByteOffset::new(0),
    )
    .unwrap();
    assert_eq!(sink.reconstructed_sample(PlaneId::U, 0, 0).unwrap(), 0);
    assert_eq!(sink.reconstructed_counts().1, 0);
    sink.reconstruct_chroma_transform(
        PlaneId::U,
        TX_16X16,
        0,
        0,
        &zero_block(),
        Some(SupportedChromaMode::Dc),
        149,
        ByteOffset::new(0),
    )
    .unwrap();
    assert_eq!(sink.reconstructed_sample(PlaneId::U, 0, 0).unwrap(), 512);
    assert_eq!(sink.reconstructed_counts().1, 16);
}

#[test]
fn second_block_dc_reads_first_block_reconstructed_neighbour() {
    let mut sink = sink();
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X16,
        &zero_block(),
        Some(IntraYMode::DC_PRED),
        false,
    );
    recon_luma(
        &mut sink,
        4,
        0,
        TX_16X16,
        &zero_block(),
        Some(IntraYMode::DC_PRED),
        false,
    );
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 16, 0).unwrap(), 512);
    assert_eq!(sink.reconstructed_counts().0, 32);
}

#[test]
fn out_of_range_tx_size_leaves_the_region_unreconstructed() {
    let mut sink = sink();
    recon_luma(
        &mut sink,
        0,
        0,
        999,
        &zero_block(),
        Some(IntraYMode::DC_PRED),
        false,
    );
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 0);
    assert_eq!(sink.reconstructed_counts().0, 0);
}

/// A non-`all_zero` DC block sized for a 16x64 adjusted transform (the
/// `Min(16,32) x Min(64,32) == 16x32 == 512`-entry coefficient grid), with a
/// single DC coefficient, used to exercise the non-square residual path.
fn coeff_block_16x64() -> LumaCoeffBlock {
    dc_coeff_block(512, -2)
}

#[test]
fn non_square_nonzero_dc_leaf_is_reconstructed() {
    let mut sink = sink();
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X64,
        &coeff_block_16x64(),
        Some(IntraYMode::DC_PRED),
        false,
    );
    assert!(sink.reconstructed_counts().0 > 0);
    assert_eq!(sink.reconstructed_counts().0, 64);
}

#[test]
fn ist_nonzero_dc_leaf_is_deferred() {
    let mut sink = sink();
    let mut block = coeff_block_16x16();
    block.intra_ist = Some(crate::tile_payload::IntraIstSyntax {
        sec_tx_type: 1,
        most_probable_stx_set: Some(0),
    });
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X16,
        &block,
        Some(IntraYMode::DC_PRED),
        false,
    );
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 0);
    assert_eq!(sink.reconstructed_counts().0, 0);
}

#[test]
fn ist_noop_sec_tx_dc_leaf_reconstructs_identically_to_non_ist() {
    let mut reference_sink = sink();
    recon_luma(
        &mut reference_sink,
        0,
        0,
        TX_16X16,
        &coeff_block_16x16(),
        Some(IntraYMode::DC_PRED),
        false,
    );
    let reference = reference_sink
        .reconstructed_sample(PlaneId::Y, 0, 0)
        .unwrap();
    assert!(
        reference_sink.reconstructed_counts().0 > 0,
        "the non-IST reference leaf must reconstruct"
    );

    let mut sink = sink();
    let mut block = coeff_block_16x16();
    block.intra_ist = Some(crate::tile_payload::IntraIstSyntax {
        sec_tx_type: 0,
        most_probable_stx_set: None,
    });
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X16,
        &block,
        Some(IntraYMode::DC_PRED),
        false,
    );
    assert_eq!(
        sink.reconstructed_counts().0,
        reference_sink.reconstructed_counts().0,
        "a no-op-IST leaf must reconstruct the same sample count as its non-IST twin"
    );
    assert_eq!(
        sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(),
        reference,
        "a no-op-IST leaf must reconstruct byte-identically to its non-IST twin"
    );
}

#[test]
fn ist_noop_sec_tx_fsc_leaf_still_defers() {
    let mut sink = sink();
    let mut block = coeff_block_16x16();
    block.intra_ist = Some(crate::tile_payload::IntraIstSyntax {
        sec_tx_type: 0,
        most_probable_stx_set: None,
    });
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X16,
        &block,
        Some(IntraYMode::DC_PRED),
        true,
    );
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 0);
    assert_eq!(sink.reconstructed_counts().0, 0);
}

#[test]
fn non_square_multi_coeff_dc_leaf_is_reconstructed() {
    let mut sink = sink();
    let mut block = coeff_block_16x64();
    block.eob = 2;
    block.quant[1] = 7;
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X64,
        &block,
        Some(IntraYMode::DC_PRED),
        false,
    );
    assert_eq!(sink.reconstructed_counts().0, 64);
}

/// Reconstructs `base` (its asymmetric `eob > 1` coefficients) twice — once as
/// `DCT_DCT` and once as `ADST_ADST` — over the no-neighbour frame origin, and
/// asserts both fully reconstruct (`expected_count` 4x4 units) AND the two
/// reconstructions DIFFER somewhere over the `width x height` block, proving the
/// retained `plane_tx_type` drives the §7.15.4 inverse.
fn assert_tx_type_threads(
    base: &LumaCoeffBlock,
    tx_size: usize,
    (width, height): (usize, usize),
    expected_count: usize,
) {
    let recon_with = |tx_type: usize| {
        let mut block = base.clone();
        block.plane_tx_type = tx_type;
        let mut sink = sink();
        recon_luma(
            &mut sink,
            0,
            0,
            tx_size,
            &block,
            Some(IntraYMode::DC_PRED),
            false,
        );
        sink
    };
    let sink_dct = recon_with(DCT_DCT);
    let sink_adst = recon_with(ADST_ADST);
    assert_eq!(sink_dct.reconstructed_counts().0, expected_count);
    assert_eq!(sink_adst.reconstructed_counts().0, expected_count);
    let differs = (0..height).any(|y| {
        (0..width).any(|x| {
            sink_dct.reconstructed_sample(PlaneId::Y, x, y).unwrap()
                != sink_adst.reconstructed_sample(PlaneId::Y, x, y).unwrap()
        })
    });
    assert!(
        differs,
        "ADST_ADST must reconstruct differently from DCT_DCT for the same eob>1 coeffs (tx {tx_size})"
    );
}

#[test]
fn eob_gt1_threads_real_tx_type_square_and_non_square() {
    for (mut block, tx_size, dims, count) in [
        (coeff_block_16x16(), TX_16X16, (16, 16), 16),
        (coeff_block_16x64(), TX_16X64, (16, 64), 64),
    ] {
        block.quant[0] = -3;
        block.quant[1] = 9;
        block.quant[16] = -7;
        block.eob = 3;
        assert_tx_type_threads(&block, tx_size, dims, count);
    }
}

#[test]
fn fsc_nonzero_dc_leaf_is_deferred() {
    let mut sink = sink();
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X16,
        &coeff_block_16x16(),
        Some(IntraYMode::DC_PRED),
        true,
    );
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 0);
    assert_eq!(sink.reconstructed_counts().0, 0);
}

#[test]
fn dc_block_with_deferred_neighbour_is_deferred() {
    let mut sink = sink();
    recon_luma(&mut sink, 0, 0, TX_16X16, &zero_block(), None, false);
    assert_eq!(sink.reconstructed_counts().0, 0);
    recon_luma(
        &mut sink,
        4,
        0,
        TX_16X16,
        &zero_block(),
        Some(IntraYMode::DC_PRED),
        false,
    );
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 16, 0).unwrap(), 0);
    assert_eq!(sink.reconstructed_counts().0, 0);
}

#[test]
fn chroma_u_coverage_does_not_satisfy_v_edge_guard() {
    let mut sink = sink();
    sink.reconstruct_chroma_transform(
        PlaneId::U,
        TX_16X16,
        0,
        0,
        &zero_block(),
        Some(SupportedChromaMode::Dc),
        149,
        ByteOffset::new(0),
    )
    .unwrap();
    let chroma_after_u = sink.reconstructed_counts().1;
    assert!(chroma_after_u > 0, "U origin block should reconstruct");
    sink.reconstruct_chroma_transform(
        PlaneId::V,
        TX_16X16,
        16,
        0,
        &zero_block(),
        Some(SupportedChromaMode::Dc),
        149,
        ByteOffset::new(0),
    )
    .unwrap();
    assert_eq!(
        sink.reconstructed_counts().1,
        chroma_after_u,
        "deferred-neighbour V block must not reconstruct via U's coverage",
    );
}

#[test]
fn non_reconstructable_quant_defers_everything() {
    let mut sink =
        WienerNsLrReconSink::<u16>::new(64, 64, BitDepth::Ten, false, false, false, 16).unwrap();
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X16,
        &zero_block(),
        Some(IntraYMode::DC_PRED),
        false,
    );
    sink.reconstruct_chroma_transform(
        PlaneId::U,
        TX_16X16,
        0,
        0,
        &zero_block(),
        Some(SupportedChromaMode::Dc),
        149,
        ByteOffset::new(0),
    )
    .unwrap();
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 0);
    assert_eq!(sink.reconstructed_sample(PlaneId::U, 0, 0).unwrap(), 0);
    assert_eq!(sink.reconstructed_counts(), (0, 0));
}

#[test]
fn cardinal_hpred_copies_reconstructed_left_column() {
    let mut sink = sink();
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X16,
        &zero_block(),
        Some(IntraYMode::DC_PRED),
        false,
    );
    recon_luma_cardinal(
        &mut sink,
        4,
        0,
        TX_16X16,
        &zero_block(),
        IntraYMode::H_PRED_FOR_TEST,
        SupportedDirectionalLumaMode::Horizontal,
        0,
    );
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 16, 0).unwrap(), 512);
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 31, 15).unwrap(), 512);
    assert_eq!(sink.reconstructed_counts().0, 32);
}

#[test]
fn cardinal_vpred_copies_reconstructed_above_row() {
    let mut sink = sink();
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X16,
        &zero_block(),
        Some(IntraYMode::DC_PRED),
        false,
    );
    recon_luma_cardinal(
        &mut sink,
        0,
        4,
        TX_16X16,
        &zero_block(),
        IntraYMode::V_PRED_FOR_TEST,
        SupportedDirectionalLumaMode::Vertical,
        0,
    );
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 16).unwrap(), 512);
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 15, 31).unwrap(), 512);
    assert_eq!(sink.reconstructed_counts().0, 32);
}

#[test]
fn cardinal_at_frame_edge_with_no_required_neighbour_is_deferred() {
    for (mode, direction) in [
        (
            IntraYMode::H_PRED_FOR_TEST,
            SupportedDirectionalLumaMode::Horizontal,
        ),
        (
            IntraYMode::V_PRED_FOR_TEST,
            SupportedDirectionalLumaMode::Vertical,
        ),
    ] {
        let mut sink = sink();
        recon_luma_cardinal(&mut sink, 0, 0, TX_16X16, &zero_block(), mode, direction, 0);
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 0);
        assert_eq!(sink.reconstructed_counts().0, 0);
    }
}

/// An ASYMMETRIC partially-covered edge: the §5.20.2.3 contiguous-availability scan
/// counts the leading covered run and BREAKS at the first hole (a covered cell after
/// a hole is NOT skipped into the run).
#[test]
fn covered_run_len_counts_the_leading_run_and_breaks_on_the_first_hole() {
    let mut coverage = PlaneCoverage::new(64, 64);
    coverage.mark(0, 0, 1, 1);
    coverage.mark(0, 1, 1, 1);
    coverage.mark(0, 4, 1, 1);
    assert_eq!(coverage.covered_run_len(0, 0, 0, 1, 8), 2);
    assert_eq!(coverage.covered_run_len(0, 0, 0, 1, 1), 1);
    assert_eq!(coverage.covered_run_len(0, 2, 0, 1, 8), 0);
    coverage.mark(0, 0, 3, 1);
    assert_eq!(coverage.covered_run_len(0, 0, 1, 0, 5), 3);
}

/// The §7.13.2.1 single-neighbour cardinal fallback (V_PRED, `haveAbove == 0`) fills
/// the whole block with the ORIGIN-ADJACENT `left_ref[0]` (AVM `reconintra.c:1150`),
/// not a deeper sample. Row 0 of the left column holds a value DISTINCT from row 1
/// (so a deeper read would be detectable), rows 2-7 deferred: the block must be flat
/// to the row-0 value.
#[test]
fn cardinal_vpred_partial_left_fallback_reads_origin_adjacent_sample_not_deeper() {
    let mut sink = sink();
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X4,
        &zero_block(),
        Some(IntraYMode::DC_PRED),
        false,
    );
    recon_luma(
        &mut sink,
        0,
        1,
        TX_16X4,
        &dc_coeff_block(64, 200),
        Some(IntraYMode::DC_PRED),
        false,
    );
    let row0 = sink.reconstructed_sample(PlaneId::Y, 15, 0).unwrap();
    let row1 = sink.reconstructed_sample(PlaneId::Y, 15, 4).unwrap();
    assert_ne!(
        row0, row1,
        "the two left-column rows must differ to discriminate origin-adjacent vs deeper reads"
    );
    let before = sink.reconstructed_counts().0;
    recon_luma_cardinal(
        &mut sink,
        4,
        0,
        TX_16X32,
        &zero_block(),
        IntraYMode::V_PRED_FOR_TEST,
        SupportedDirectionalLumaMode::Vertical,
        0,
    );
    assert!(
        sink.reconstructed_counts().0 > before,
        "the partial-left V_PRED fallback must be ADMITTED, not deferred"
    );
    for y in [0usize, 16, 31] {
        for x in [16usize, 24, 31] {
            assert_eq!(
                sink.reconstructed_sample(PlaneId::Y, x, y).unwrap(),
                row0,
                "V_PRED partial fallback ({x},{y}) must replicate the origin-adjacent left sample"
            );
        }
    }
}

/// The partial-vs-midpoint boundary: when the ORIGIN-ADJACENT left cell of the
/// §7.13.2.1 fallback is itself uncovered (the no-neighbour midpoint case), the
/// cardinal block must DEFER — the gate admits only a leading covered run >= 1.
#[test]
fn cardinal_vpred_fallback_with_uncovered_origin_left_cell_is_deferred() {
    let mut sink = sink();
    let before = sink.reconstructed_counts().0;
    recon_luma_cardinal(
        &mut sink,
        4,
        0,
        TX_16X16,
        &zero_block(),
        IntraYMode::V_PRED_FOR_TEST,
        SupportedDirectionalLumaMode::Vertical,
        0,
    );
    assert_eq!(
        sink.reconstructed_counts().0,
        before,
        "a V_PRED fallback with an uncovered origin-adjacent left cell must DEFER"
    );
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 16, 0).unwrap(), 0);
}

#[test]
fn cardinal_hpred_with_deferred_left_neighbour_is_deferred() {
    let mut sink = sink();
    recon_luma(&mut sink, 0, 0, TX_16X16, &zero_block(), None, false);
    assert_eq!(sink.reconstructed_counts().0, 0);
    recon_luma_cardinal(
        &mut sink,
        4,
        0,
        TX_16X16,
        &zero_block(),
        IntraYMode::H_PRED_FOR_TEST,
        SupportedDirectionalLumaMode::Horizontal,
        0,
    );
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 16, 0).unwrap(), 0);
    assert_eq!(sink.reconstructed_counts().0, 0);
}

#[test]
fn cardinal_nonsquare_transform_is_deferred() {
    let mut sink = sink();
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X16,
        &zero_block(),
        Some(IntraYMode::DC_PRED),
        false,
    );
    let before = sink.reconstructed_counts().0;
    recon_luma_cardinal(
        &mut sink,
        4,
        0,
        TX_16X64,
        &zero_block(),
        IntraYMode::H_PRED_FOR_TEST,
        SupportedDirectionalLumaMode::Horizontal,
        0,
    );
    assert_eq!(sink.reconstructed_counts().0, before);
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 16, 0).unwrap(), 0);
}

#[test]
fn angular_directional_mode_is_deferred() {
    let mut sink = sink();
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X16,
        &zero_block(),
        Some(IntraYMode::DC_PRED),
        false,
    );
    let before = sink.reconstructed_counts().0;
    recon_luma_cardinal(
        &mut sink,
        4,
        0,
        TX_16X16,
        &zero_block(),
        IntraYMode::D135_PRED_FOR_TEST,
        SupportedDirectionalLumaMode::D135,
        0,
    );
    assert_eq!(sink.reconstructed_counts().0, before);
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 16, 0).unwrap(), 0);
}

#[test]
fn cardinal_with_active_mrl_index_is_deferred() {
    let mut sink = sink();
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X16,
        &zero_block(),
        Some(IntraYMode::DC_PRED),
        false,
    );
    let before = sink.reconstructed_counts().0;
    recon_luma_cardinal(
        &mut sink,
        4,
        0,
        TX_16X16,
        &zero_block(),
        IntraYMode::H_PRED_FOR_TEST,
        SupportedDirectionalLumaMode::Horizontal,
        1,
    );
    assert_eq!(sink.reconstructed_counts().0, before);
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 16, 0).unwrap(), 0);
}

#[test]
fn cardinal_with_multi_coeff_residual_is_reconstructed() {
    let mut sink = sink();
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X16,
        &zero_block(),
        Some(IntraYMode::DC_PRED),
        false,
    );
    let before = sink.reconstructed_counts().0;
    let mut block = coeff_block_16x16();
    block.eob = 2;
    block.quant[1] = 7;
    recon_luma_cardinal(
        &mut sink,
        4,
        0,
        TX_16X16,
        &block,
        IntraYMode::H_PRED_FOR_TEST,
        SupportedDirectionalLumaMode::Horizontal,
        0,
    );
    assert_eq!(sink.reconstructed_counts().0, before + 16);
}

/// A §7.13.2.2 PAETH leaf with a NON-`all_zero` residual reconstructs when its
/// `haveAbove && haveLeft` neighbours (above row, left column, AND the diagonal
/// corner unit) are all covered: the predictor reads the real reconstructed edges
/// and the §5.20.7.27 residual is added on top. Proven by the count growing AND the
/// sample moving off the would-be flat `512` Paeth prediction (the residual fired).
#[test]
fn paeth_with_residual_reconstructs_when_neighbours_covered() {
    let mut sink = sink();
    for col in 0..8 {
        recon_luma(
            &mut sink,
            col,
            0,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
    }
    for row in 0..8 {
        recon_luma(
            &mut sink,
            0,
            row,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
    }
    let before = sink.reconstructed_counts().0;
    let mut block = coeff_block_16x16();
    block.eob = 2;
    block.quant[1] = 9;
    recon_luma(
        &mut sink,
        4,
        4,
        TX_16X16,
        &block,
        Some(IntraYMode::PAETH_PRED_FOR_TEST),
        false,
    );
    assert_eq!(sink.reconstructed_counts().0, before + 16);
    let mut moved = false;
    for dy in 0..16 {
        for dx in 0..16 {
            if sink
                .reconstructed_sample(PlaneId::Y, 16 + dx, 16 + dy)
                .unwrap()
                != 512
            {
                moved = true;
            }
        }
    }
    assert!(
        moved,
        "the PAETH residual must move samples off the flat prediction"
    );
}

/// A §7.13.2.2 PAETH leaf whose diagonal CORNER unit is NOT covered is DEFERRED
/// (the corner `AboveRow[-1]` is load-bearing for Paeth, so an uncovered corner must
/// not fall back to a fill value).
#[test]
fn paeth_with_uncovered_corner_is_deferred() {
    let mut sink = sink();
    for col in 1..8 {
        recon_luma(
            &mut sink,
            col,
            0,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
    }
    for row in 1..8 {
        recon_luma(
            &mut sink,
            0,
            row,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
    }
    let before = sink.reconstructed_counts().0;
    recon_luma(
        &mut sink,
        4,
        4,
        TX_16X16,
        &zero_block(),
        Some(IntraYMode::PAETH_PRED_FOR_TEST),
        false,
    );
    assert_eq!(
        sink.reconstructed_counts().0,
        before,
        "uncovered corner defers"
    );
}

/// A sink seeded with a reconstructed 16x16 `DC_PRED` block at the frame origin
/// (the flat-`512` source the IntrABC integer-copy tests displace), plus the 4x4
/// count captured BEFORE the copy under test.
fn sink_with_origin_dc() -> (WienerNsLrReconSink<u16>, usize) {
    let mut sink = sink();
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X16,
        &zero_block(),
        Some(IntraYMode::DC_PRED),
        false,
    );
    let before = sink.reconstructed_counts().0;
    (sink, before)
}

/// A §7.13.3.18 IntrABC integer-vector skip copy whose source rectangle is fully
/// reconstructed copies the source samples into the target (target == source).
#[test]
fn intrabc_integer_skip_copy_reconstructs_target_from_reconstructed_source() {
    let (mut sink, before) = sink_with_origin_dc();
    let source = PlaneRect::new(0, 0, 16, 16).unwrap();
    let target = PlaneRect::new(0, 32, 16, 16).unwrap();
    sink.reconstruct_intrabc_block(source, target, unused_scaling(), true, ByteOffset::new(0))
        .unwrap();
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 32).unwrap(), 512);
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 15, 47).unwrap(), 512);
    assert_eq!(sink.reconstructed_counts().0, before + 16);
}

/// A §7.13.3.18 FRACTIONAL-DV IntrABC block runs the BILINEAR sub-pel predictor
/// (NOT a rect copy) over the reconstructed `CurrFrame` luma plane, in full-recon
/// mode. The reference is computed INDEPENDENTLY with the proven
/// [`splot_recon::subpel_predict_block`] primitive over the same §7.13.3.17 scaling
/// (from [`super::PlaneScaling`] via the production `derive_plane_scaling`) and the
/// same reference plane: the sink must produce byte-identical samples, proving the
/// scaling-threading, reference-plane construction, and target write. The source
/// pattern is ASYMMETRIC (a horizontal ramp), so the half-pel `col = -4` DV
/// (phase-8 `{64, 64}` weights) genuinely averages two distinct neighbours.
#[test]
fn intrabc_fractional_dv_runs_bilinear_subpel_predictor() {
    use super::full_recon::intrabc_bilinear_params;
    use crate::runtime_minimal::inter::mv_scaling::derive_plane_scaling;
    use splot_recon::{ReferencePlaneView, subpel_predict_block};

    let mut sink = WienerNsLrReconSink::<u16>::new(128, 128, BitDepth::Ten, true, false, false, 16)
        .unwrap()
        .into_full_recon();
    for col in 0..4 {
        let dc = 100 + 80 * col as i32;
        recon_luma(
            &mut sink,
            col,
            8,
            TX_16X16,
            &dc_coeff_block(256, dc),
            Some(IntraYMode::DC_PRED),
            false,
        );
    }

    let (w, h) = (16usize, 16usize);
    let (tgt_x, tgt_y) = (48usize, 64usize);
    let (mv_row, mv_col) = (-256i32, -260i32); // up 32 samples; left 32.5 (half-pel)
    let scaling = derive_plane_scaling(
        tgt_x as i64,
        tgt_y as i64,
        i64::from(mv_row),
        i64::from(mv_col),
        0,
        0,
        32,
        32,
        w as i64,
        h as i64,
    );

    let storage = 128usize;
    let mut reference = vec![0u16; storage * storage];
    for y in 0..storage {
        for x in 0..storage {
            reference[y * storage + x] = sink.reconstructed_sample(PlaneId::Y, x, y).unwrap();
        }
    }
    let view = ReferencePlaneView::new(&reference, storage, storage).unwrap();
    let params = intrabc_bilinear_params(scaling, w, h, BitDepth::Ten);
    let expected = subpel_predict_block(&view, &params).unwrap();

    let source = PlaneRect::new(0, 32, w + 1, h).unwrap(); // fractional: +1 right border
    let target = PlaneRect::new(tgt_x, tgt_y, w, h).unwrap();
    sink.reconstruct_intrabc_block(source, target, scaling, true, ByteOffset::new(0))
        .unwrap();

    for dy in 0..h {
        for dx in 0..w {
            assert_eq!(
                sink.reconstructed_sample(PlaneId::Y, tgt_x + dx, tgt_y + dy)
                    .unwrap(),
                expected[dy * w + dx],
                "fractional IntrABC sample ({dx},{dy}) must match the bilinear reference"
            );
        }
    }
    assert!(
        expected.iter().any(|&s| s != expected[0]),
        "the bilinear prediction over the ramp must vary across the block"
    );
}

/// A §7.13.3.18 IntrABC block whose source rectangle is NOT fully reconstructed
/// is DEFERRED — never copies a workspace fill value as if it were a real sample.
#[test]
fn intrabc_copy_with_unreconstructed_source_is_deferred() {
    let mut sink = sink();
    let source = PlaneRect::new(0, 0, 16, 16).unwrap();
    let target = PlaneRect::new(0, 32, 16, 16).unwrap();
    sink.reconstruct_intrabc_block(source, target, unused_scaling(), true, ByteOffset::new(0))
        .unwrap();
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 32).unwrap(), 0);
    assert_eq!(sink.reconstructed_counts().0, 0);
}

/// Codex finding 1: a NON-4x4-aligned integer source whose CEIL'd MI span includes
/// an unreconstructed trailing MI is DEFERRED. The covered-MI span must be computed
/// from the source's actual sample extent (`ceil((x+width)/4) - floor(x/4)`), not a
/// floored `width / 4` that would drop the trailing partial MI and copy its fill.
#[test]
fn intrabc_unaligned_source_with_uncovered_trailing_mi_is_deferred() {
    let (mut sink, before) = sink_with_origin_dc();
    let source = PlaneRect::new(2, 0, 16, 16).unwrap();
    let target = PlaneRect::new(2, 32, 16, 16).unwrap();
    sink.reconstruct_intrabc_block(source, target, unused_scaling(), true, ByteOffset::new(0))
        .unwrap();
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 2, 32).unwrap(), 0);
    assert_eq!(sink.reconstructed_counts().0, before);
}

/// A non-skip IntrABC block's displaced copy writes the §7.13.2 PREDICTION into
/// the workspace target but does NOT mark coverage: the §5.20.7.27 residual leaf
/// (decoded after this prelude, in decode order) adds the residual onto the copied
/// predictor and marks coverage. So after the copy alone, the target carries the
/// copied source samples (the prediction) but the 4x4 count is unchanged — only
/// the residual leaf finalises the block.
#[test]
fn intrabc_non_skip_copy_writes_prediction_without_marking_coverage() {
    let (mut sink, before) = sink_with_origin_dc();
    let source = PlaneRect::new(0, 0, 16, 16).unwrap();
    let target = PlaneRect::new(0, 32, 16, 16).unwrap();
    sink.reconstruct_intrabc_block(source, target, unused_scaling(), false, ByteOffset::new(0))
        .unwrap();
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 32).unwrap(), 512);
    assert_eq!(sink.reconstructed_counts().0, before);
}

/// The full non-skip IntrABC residual path: the displaced copy writes the
/// prediction (the flat 512 source), then the residual leaf adds an ASYMMETRIC
/// `eob > 1` residual onto it and marks coverage. The reconstruction must equal
/// `Clip1(prediction + inverse-transform(residual))`, verified against the
/// equivalent intra `DC_PRED` reconstruction over the SAME flat-512 prediction.
#[test]
fn intrabc_non_skip_residual_leaf_adds_residual_onto_copied_prediction() {
    let mut residual = coeff_block_16x16();
    residual.quant[0] = -3;
    residual.quant[1] = 9;
    residual.quant[16] = -7;
    residual.eob = 3;

    let mut reference_sink = sink();
    recon_luma(
        &mut reference_sink,
        0,
        0,
        TX_16X16,
        &residual,
        Some(IntraYMode::DC_PRED),
        false,
    );

    let mut sink = sink();
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X16,
        &zero_block(),
        Some(IntraYMode::DC_PRED),
        false,
    );
    let before = sink.reconstructed_counts().0;
    let source = PlaneRect::new(0, 0, 16, 16).unwrap();
    let target = PlaneRect::new(0, 32, 16, 16).unwrap();
    sink.reconstruct_intrabc_block(source, target, unused_scaling(), false, ByteOffset::new(0))
        .unwrap();
    sink.reconstruct_luma_transform(
        0,
        8,
        TX_16X16,
        &residual,
        Some(IntraYMode::DC_PRED),
        None,
        0,
        0,
        149,
        true,
        false,
        true, // is_intrabc
        ByteOffset::new(0),
    )
    .unwrap();
    assert_eq!(sink.reconstructed_counts().0, before + 16);
    for y in 0..16 {
        for x in 0..16 {
            let got = sink.reconstructed_sample(PlaneId::Y, x, 32 + y).unwrap();
            let want = reference_sink
                .reconstructed_sample(PlaneId::Y, x, y)
                .unwrap();
            assert_eq!(
                got,
                want,
                "IntrABC residual ({x},{}) must equal pred+residual {want}, got {got}",
                32 + y
            );
        }
    }
}

/// A non-skip IntrABC residual leaf whose transform rect is NOT inside any pending
/// IntrABC prediction (the whole-block copy was deferred — fractional DV / uncovered
/// source / non-reconstructable quant) is DEFERRED: never adds a residual onto the
/// workspace fill value.
#[test]
fn intrabc_non_skip_residual_leaf_without_pending_prediction_is_deferred() {
    let mut sink = sink();
    let mut residual = coeff_block_16x16();
    residual.eob = 2;
    residual.quant[1] = 7;
    sink.reconstruct_luma_transform(
        0,
        0,
        TX_16X16,
        &residual,
        Some(IntraYMode::DC_PRED),
        None,
        0,
        0,
        149,
        true,
        false,
        true, // is_intrabc
        ByteOffset::new(0),
    )
    .unwrap();
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 0);
    assert_eq!(sink.reconstructed_counts().0, 0);
}

/// A non-skip IntrABC residual leaf carrying a REAL §5.20.7.29 IST secondary
/// transform (`sec_tx_type != 0`) is DEFERRED even with a pending prediction: the
/// §7.15.3 secondary transform is unmodelled, so the residual is not proven.
#[test]
fn intrabc_non_skip_residual_leaf_with_real_ist_is_deferred() {
    let mut sink = sink();
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X16,
        &zero_block(),
        Some(IntraYMode::DC_PRED),
        false,
    );
    let before = sink.reconstructed_counts().0;
    let source = PlaneRect::new(0, 0, 16, 16).unwrap();
    let target = PlaneRect::new(0, 32, 16, 16).unwrap();
    sink.reconstruct_intrabc_block(source, target, unused_scaling(), false, ByteOffset::new(0))
        .unwrap();
    let mut residual = coeff_block_16x16();
    residual.intra_ist = Some(crate::tile_payload::IntraIstSyntax {
        sec_tx_type: 1,
        most_probable_stx_set: Some(0),
    });
    sink.reconstruct_luma_transform(
        0,
        8,
        TX_16X16,
        &residual,
        Some(IntraYMode::DC_PRED),
        None,
        0,
        0,
        149,
        true,
        false,
        true, // is_intrabc
        ByteOffset::new(0),
    )
    .unwrap();
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 32).unwrap(), 512);
    assert_eq!(sink.reconstructed_counts().0, before);
}

/// A fractional-vector IntrABC block (source and target differ in shape — the
/// BILINEAR border) is DEFERRED: the copy primitive only models the integer copy.
#[test]
fn intrabc_fractional_vector_block_is_deferred() {
    let (mut sink, before) = sink_with_origin_dc();
    let source = PlaneRect::new(0, 0, 17, 17).unwrap();
    let target = PlaneRect::new(0, 32, 16, 16).unwrap();
    sink.reconstruct_intrabc_block(source, target, unused_scaling(), true, ByteOffset::new(0))
        .unwrap();
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 32).unwrap(), 0);
    assert_eq!(sink.reconstructed_counts().0, before);
}

/// The §5.20.7.29 `wide_angle_mapping` wrap branches FIRE only for non-square
/// transforms (`h == k*w` tall / `w == k*h` wide), and are inert for square blocks.
/// This pins the wrap behaviour VERBATIM against AVM `wide_angle_mapping`
/// (`reconintra.h`): a tall block (`h == k*w` with `pAngle < WAIP_WH_RATIO_k_THRES`)
/// adds 180 (zone-1 → zone-3 D203 wrap); a wide block (`w == k*h` with `pAngle >
/// 270 - WAIP_WH_RATIO_k_THRES`) subtracts 180 (zone-3 → zone-1 D45 wrap); a square
/// block never remaps; and an out-of-threshold non-square angle is unchanged.
#[test]
fn wide_angle_mapping_wraps_non_square_blocks_verbatim_vs_avm() {
    assert_eq!(wide_angle_mapping(16, 16, 35), 35);
    assert_eq!(wide_angle_mapping(16, 16, 215), 215);

    assert_eq!(wide_angle_mapping(8, 16, 58), 58 + 180);
    assert_eq!(wide_angle_mapping(8, 16, 81), 81);
    assert_eq!(wide_angle_mapping(8, 32, 70), 70 + 180);
    assert_eq!(wide_angle_mapping(8, 32, 76), 76);

    assert_eq!(wide_angle_mapping(16, 8, 212), 212 - 180);
    assert_eq!(wide_angle_mapping(16, 8, 189), 189);
    assert_eq!(wide_angle_mapping(32, 8, 200), 200 - 180);
    assert_eq!(wide_angle_mapping(32, 8, 194), 194);
}

/// The threaded AV2 §5.20.2.3 `BlockDecoded` far-edge availability infra: the sink
/// records a block's §7.13.2.1 `num4AboveRight` / `num4BelowLeft` (in luma 4x4
/// units) over the transform's MI footprint and exposes them through
/// [`WienerNsLrReconSink::block_decoded_far_edge`]. A unit not written to (no block
/// decoded there yet) reads `None`. This pins the durable threading wiring
/// independently of the conservative coverage gates (which are unchanged).
#[test]
fn block_decoded_far_edge_records_and_queries_threaded_availability() {
    let mut sink = sink();
    sink.record_block_decoded_far_edge(0, 0, TX_16X16, 3, 2);
    assert_eq!(sink.block_decoded_far_edge(0, 0), Some((3, 2)));
    assert_eq!(sink.block_decoded_far_edge(3, 3), Some((3, 2)));
    assert_eq!(sink.block_decoded_far_edge(4, 0), None);
    assert_eq!(sink.block_decoded_far_edge(0, 4), None);
    sink.record_block_decoded_far_edge(4, 0, TX_16X16, 0, 0);
    assert_eq!(sink.block_decoded_far_edge(4, 0), Some((0, 0)));
    assert_eq!(sink.block_decoded_far_edge(0, 0), Some((3, 2)));
}
