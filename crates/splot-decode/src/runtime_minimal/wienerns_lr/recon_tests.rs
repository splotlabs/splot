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
/// §3 `TX_TYPES` index for `DCT_DCT` (`Transform_1d_Type[0] == (DCT, DCT)`).
const DCT_DCT: usize = 0;
/// §3 `TX_TYPES` index for `ADST_ADST` (`Transform_1d_Type[3] == (ADST, ADST)`).
const ADST_ADST: usize = 3;

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
    // 64x64 luma frame (a positive multiple of 64), 10-bit 4:2:0 — matching
    // the ac0ej3 sample type. `quant_reconstructable = true` (no delta-q / qm).
    // `enable_ibp = false` keeps these flat-DC gate tests on the §7.13.2.10
    // prediction; the §7.13.2.12 IBP DC path has its own focused test.
    // `enable_intra_edge_filter = false`: these tests exercise the DC / cardinal
    // subset, which never reaches the §7.13.2.7 edge-filter gate.
    WienerNsLrReconSink::<u16>::new(64, 64, BitDepth::Ten, true, false, false).unwrap()
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
    // §7.13.2.1 no-neighbour DC fallback for 10-bit is `1 << (10 - 1)` == 512.
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 512);
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 15, 15).unwrap(), 512);
    let (luma4x4, _chroma4x4) = sink.reconstructed_counts();
    // TX_16X16 == 4x4 luma 4x4 units.
    assert_eq!(luma4x4, 16);
}

#[test]
fn non_dc_luma_mode_leaves_the_region_unreconstructed() {
    let mut sink = sink();
    // A leaf without a DC_PRED luma mode (here `None`, an SDP chroma / inter
    // leaf) is deferred: only DC_PRED luma is in the verified subset.
    recon_luma(&mut sink, 0, 0, TX_16X16, &zero_block(), None, false);
    // The default 10-bit workspace fill is 0 (not the DC fallback): the sink
    // did not write the non-DC block, so the region stays at the fill value.
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 0);
    assert_eq!(sink.reconstructed_counts().0, 0);
}

// SMOOTH chroma is DEFERRED, never reconstructed. This is load-bearing for the
// ac0ej3 mission: splot resolves the SB0 chroma leaf (and every reachable chroma
// leaf past the first BLOCK_16X64 luma column) as `SMOOTH`, but AVM's mode oracle
// resolves them as `DC` / `H` / `CfL` and its prediction-only buffer is flat 512
// (no-neighbour DC). The §7.13.2.13 SMOOTH primitive over the §7.13.2.1
// no-neighbour fallback edges (above 511, left 513) instead produces a 511..513
// gradient, so admitting SMOOTH here would write confidently-wrong samples. The
// sink DEFERS until the upstream mode resolution is reconciled with AVM.
#[test]
fn dc_chroma_non_dc_mode_leaves_the_region_unreconstructed() {
    let mut sink = sink();
    // SMOOTH chroma is not in the verified DC subset, so it is deferred.
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
    // DC chroma reconstructs the bare DC fallback.
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
    // First block at (0,0): no-neighbour DC -> 512.
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X16,
        &zero_block(),
        Some(IntraYMode::DC_PRED),
        false,
    );
    // Second block to the right at mi_col=4 (x=16): its DC reads the left
    // neighbour (the reconstructed 512 column), so the flat DC is again 512 —
    // proving the neighbour read path runs over the partially-built frame.
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

// A non-`all_zero`, non-square DC leaf (e.g. TX_16X64) is now ADMITTED: the
// §7.15.4 outer process drives the rectangular-residual inverse transform
// (proven bit-exact for the ac0ej3 mi(4,0) leaf). The frame-origin no-neighbour
// DC fallback (512) plus a flat DC-only residual reconstructs a flat block.
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
    // The origin block has no reconstructed neighbour (off-frame edges), so the
    // §7.13.2.1 DC fallback is 512 (10-bit); the DC-only residual is applied
    // over the whole 16x64 block, so the sink wrote a real (non-fill) region.
    assert!(sink.reconstructed_counts().0 > 0);
    // 16x64 == 4 MI cols x 16 MI rows == 64 4x4 units.
    assert_eq!(sink.reconstructed_counts().0, 64);
}

// Finding #1: a non-`all_zero` DC leaf carrying §5.20.7.29 IST secondary
// transform syntax is DEFERRED (the primitive is DCT_DCT-only). A REAL IST
// (`sec_tx_type != 0`) stays deferred even after no-op IST is admitted.
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

// A non-`all_zero` DC leaf carrying §5.20.7.29 IST syntax with `sec_tx_type == 0`
// applies NO §7.15.3 secondary transform — it reconstructs through the identical
// DCT_DCT residual path as a non-IST leaf, so it is ADMITTED when it also
// satisfies the normal residual + proven-neighbour conditions. It must produce
// the byte-identical result to the same block WITHOUT IST syntax.
#[test]
fn ist_noop_sec_tx_dc_leaf_reconstructs_identically_to_non_ist() {
    // Reference: the same DC-only square leaf with NO IST syntax.
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

    // The same leaf carrying a `sec_tx_type == 0` no-op IST: admitted, identical.
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

// A `sec_tx_type == 0` no-op IST leaf is still subject to EVERY other residual
// gate. An FSC no-op-IST leaf still DEFERS (the non-FSC primitive), proving the
// IST relaxation did not widen the FSC gate.
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

// A non-`all_zero`, NON-square DC leaf with `eob > 1` is now ADMITTED: the
// retained `block.plane_tx_type` flows to the §7.15.4 primary inverse, so the
// former "unretained non-`DCT_DCT` type" defer is gone. The whole 16x64 block
// reconstructs (64 4x4 units), and the non-DC coefficient produces a real (non
// flat-DC) residual variation that distinguishes the tx-type — see
// `non_square_eob_gt1_threads_real_tx_type` for the per-type correctness proof.
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
    // 16x64 == 4 MI cols x 16 MI rows == 64 4x4 units, all reconstructed.
    assert_eq!(sink.reconstructed_counts().0, 64);
}

/// Reconstructs `base` (its asymmetric `eob > 1` coefficients) twice — once as
/// `DCT_DCT` and once as `ADST_ADST` — over the no-neighbour frame origin, and
/// asserts both fully reconstruct (`expected_count` 4x4 units) AND the two
/// reconstructions DIFFER somewhere over the `width x height` block. Under the
/// former hardcoded `DCT_DCT` argument BOTH types reconstructed identically (as
/// `DCT_DCT`), so a per-sample difference proves the retained `plane_tx_type`
/// now drives the §7.15.4 inverse — the latent confident-wrong is removed.
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

// Correctness proof (removes the latent confident-wrong): an `eob > 1` leaf —
// both the SQUARE (`TX_16X16`) and NON-square (`TX_16X64`) case — reconstructs
// with its REAL tx-type, NOT the former hardcoded `DCT_DCT`. Under the old
// hardcoded argument an `ADST_ADST` leaf reconstructed identically to `DCT_DCT`
// (a latent confident-wrong, safe only because every prior-admitted block
// happened to be `DCT_DCT`). Asymmetric coefficients (per the decode-verify
// lesson: symmetric/zero values can mask a kernel difference) sit in distinct
// row/col positions so both 1-D types matter.
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

// Finding #1: an FSC DC leaf is DEFERRED (non-FSC primitive).
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

// Finding #2: a DC block bordering a DEFERRED (skipped) neighbour is deferred —
// its DC prediction would read the workspace fill value, not reconstruction.
#[test]
fn dc_block_with_deferred_neighbour_is_deferred() {
    let mut sink = sink();
    // Block at (0,0) is deferred (non-DC leaf -> `None`). It is NOT reconstructed.
    recon_luma(&mut sink, 0, 0, TX_16X16, &zero_block(), None, false);
    assert_eq!(sink.reconstructed_counts().0, 0);
    // Block at (4,0) is DC_PRED but its LEFT neighbour (0,0) exists on-grid and
    // was deferred, so this block defers too (no wrong prediction from fill).
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

// Finding #2 (re-review): U and V chroma coverage are tracked SEPARATELY. A
// reconstructed U block must not let a deferred-neighbour V block pass the
// DC-edge guard (4:2:0 U and V share MI dimensions but not reconstruction
// state); otherwise V would predict from its own workspace fill value.
#[test]
fn chroma_u_coverage_does_not_satisfy_v_edge_guard() {
    let mut sink = sink();
    // A U DC block at the chroma origin reconstructs (off-grid edges) and marks
    // U coverage across MI columns 0..4.
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
    // A V DC block whose left neighbour (MI column 3) is covered ONLY on the U
    // plane must DEFER — it cannot borrow U's coverage to satisfy its own guard.
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

// Finding #3: when the frame signals a non-zero quantizer delta / qmatrix
// (`quant_reconstructable == false`), the sink reconstructs NOTHING.
#[test]
fn non_reconstructable_quant_defers_everything() {
    let mut sink =
        WienerNsLrReconSink::<u16>::new(64, 64, BitDepth::Ten, false, false, false).unwrap();
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

// Cardinal H_PRED (§7.13.2.8 step 5, pAngle 180) over a REAL reconstructed left
// column: a pure horizontal copy `pred[i][j] = LeftCol[i]`. The first DC block
// at (0,0) reconstructs the flat `512` no-neighbour fallback; an `all_zero`
// H_PRED block to its right copies that left column, so it is again flat `512` —
// proving the cardinal copy reads the partially-built frame's real neighbour.
#[test]
fn cardinal_hpred_copies_reconstructed_left_column() {
    let mut sink = sink();
    // Left DC neighbour at (0,0): flat 512.
    recon_luma(
        &mut sink,
        0,
        0,
        TX_16X16,
        &zero_block(),
        Some(IntraYMode::DC_PRED),
        false,
    );
    // H_PRED block to the right at mi_col=4 (x=16): its left column (x=15) is the
    // reconstructed 512, so the horizontal copy is flat 512.
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
    // 16 (left DC) + 16 (H_PRED) == 32 4x4 units.
    assert_eq!(sink.reconstructed_counts().0, 32);
}

// Cardinal V_PRED (§7.13.2.8 step 4, pAngle 90) over a REAL reconstructed above
// row: a pure vertical copy `pred[i][j] = AboveRow[j]`. The first DC block at
// (0,0) reconstructs flat `512`; a V_PRED block below it copies that above row,
// flat `512` — proving the cardinal copy reads the real above neighbour.
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
    // V_PRED block below at mi_row=4 (y=16): its above row (y=15) is the
    // reconstructed 512, so the vertical copy is flat 512.
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

// A cardinal block at the frame ORIGIN has no required edge to copy — H_PRED
// has no left column (mi_col == 0), V_PRED has no above row (mi_row == 0) — so
// the sink DEFERS both (the §7.13.2.1 no-neighbour fallback is a separate,
// here-unmodelled path; never predict from the fill value). The V_PRED case
// pins the exact ac0ej3 SB-column-4 V_PRED-at-y=0 deferral.
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

// A cardinal H_PRED block whose LEFT neighbour exists on-grid but was DEFERRED
// (still the fill value) must defer too — never copy a fill-value left column.
#[test]
fn cardinal_hpred_with_deferred_left_neighbour_is_deferred() {
    let mut sink = sink();
    // (0,0) is deferred (non-DC `None` leaf), so it is NOT reconstructed.
    recon_luma(&mut sink, 0, 0, TX_16X16, &zero_block(), None, false);
    assert_eq!(sink.reconstructed_counts().0, 0);
    // H_PRED at mi_col=4: its left neighbour (0,0) is on-grid but uncovered.
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

// A NON-SQUARE cardinal transform is DEFERRED: the cardinal recon primitive is
// square-only. (TX_16X64 H_PRED with a covered left column still defers.)
#[test]
fn cardinal_nonsquare_transform_is_deferred() {
    let mut sink = sink();
    // Reconstruct a left DC neighbour column first so coverage is not the gate.
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
    // Non-square cardinal deferred: count unchanged, region stays fill.
    assert_eq!(sink.reconstructed_counts().0, before);
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 16, 0).unwrap(), 0);
}

// An ANGULAR directional mode (e.g. D135) is DEFERRED by the sink even when its
// neighbours are covered: only the cardinal V/H copy subset is admitted here.
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

// Finding 1: a cardinal H_PRED leaf using a §5.20.5.5 multi-reference line
// (`mrl_index > 0`) is DEFERRED. The cardinal recon primitive copies the
// IMMEDIATE left/above edge (`MrlIndex == 0`); for `mrl_index > 0` it would
// copy the adjacent samples instead of the selected reference line — wrong.
// The left neighbour is covered, so only the MRL gate causes the deferral.
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
    // H_PRED at mi_col=4 with a covered left column but `mrl_index == 1`: defer.
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

// A cardinal H_PRED leaf with a NON-`all_zero`, non-DC-only residual (`eob > 1`)
// is now ADMITTED: the retained `block.plane_tx_type` flows to the §7.15.4
// primary inverse, so the cardinal recon primitive applies the REAL tx-type
// residual rather than a hardcoded `DCT_DCT`. The left neighbour is covered
// (the same-size DC block at (0,0)), so the block reconstructs.
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
    // A non-`all_zero` block with two decoded coefficients (`eob == 2`).
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
    // TX_16X16 == 16 4x4 units, now reconstructed on top of the DC block.
    assert_eq!(sink.reconstructed_counts().0, before + 16);
}

/// A §7.13.3.18 IntrABC integer-vector skip copy whose source rectangle is fully
/// reconstructed copies the source samples into the target (target == source).
#[test]
fn intrabc_integer_skip_copy_reconstructs_target_from_reconstructed_source() {
    let mut sink = sink();
    // Reconstruct a DC source block at the origin (16x16, flat 512), then copy a
    // 16x16 region of it down to a non-overlapping target.
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
    sink.reconstruct_intrabc_block(source, target, true, ByteOffset::new(0))
        .unwrap();
    // The whole 16x16 target now carries the copied source samples (flat 512).
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 32).unwrap(), 512);
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 15, 47).unwrap(), 512);
    // 16x16 == 16 4x4 luma units added.
    assert_eq!(sink.reconstructed_counts().0, before + 16);
}

/// A §7.13.3.18 IntrABC block whose source rectangle is NOT fully reconstructed
/// is DEFERRED — never copies a workspace fill value as if it were a real sample.
#[test]
fn intrabc_copy_with_unreconstructed_source_is_deferred() {
    let mut sink = sink();
    // No block reconstructed yet: the source region (0,0,16,16) is all fill.
    let source = PlaneRect::new(0, 0, 16, 16).unwrap();
    let target = PlaneRect::new(0, 32, 16, 16).unwrap();
    sink.reconstruct_intrabc_block(source, target, true, ByteOffset::new(0))
        .unwrap();
    // The target stays at the unreconstructed fill value, and nothing is counted.
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 32).unwrap(), 0);
    assert_eq!(sink.reconstructed_counts().0, 0);
}

/// Codex finding 1: a NON-4x4-aligned integer source whose CEIL'd MI span includes
/// an unreconstructed trailing MI is DEFERRED. The covered-MI span must be computed
/// from the source's actual sample extent (`ceil((x+width)/4) - floor(x/4)`), not a
/// floored `width / 4` that would drop the trailing partial MI and copy its fill.
#[test]
fn intrabc_unaligned_source_with_uncovered_trailing_mi_is_deferred() {
    let mut sink = sink();
    // Reconstruct a 16x16 DC block at the origin: covers luma x[0,16) == MI cols
    // 0..4. MI col 4 (x[16,20)) stays unreconstructed.
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
    // A 16px source at x=2 spans x[2,18) == MI cols 0..=4 (ceil(18/4)==5): MI col 4
    // is uncovered, so the copy must DEFER. (A floored `16/4==4` span would wrongly
    // see only cols 0..4 and copy the trailing fill.)
    let source = PlaneRect::new(2, 0, 16, 16).unwrap();
    let target = PlaneRect::new(2, 32, 16, 16).unwrap();
    sink.reconstruct_intrabc_block(source, target, true, ByteOffset::new(0))
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
    sink.reconstruct_intrabc_block(source, target, false, ByteOffset::new(0))
        .unwrap();
    // The copy wrote the predictor (the flat 512 source) into the target, but the
    // count is unchanged: a non-skip block's coverage is owned by its residual leaf.
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
    // Reference: a DC_PRED leaf at the frame origin over a flat-512 no-neighbour
    // prediction with the same asymmetric residual. (DC_PRED at (0,0) is the flat
    // 512 fallback, identical to the IntrABC predictor copied from a flat-512
    // source, so the two reconstructions must agree sample-for-sample.)
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

    // IntrABC: reconstruct a flat-512 source, copy it down to a target (the
    // prediction), then add the SAME residual via the IntrABC residual leaf.
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
    sink.reconstruct_intrabc_block(source, target, false, ByteOffset::new(0))
        .unwrap();
    // The residual leaf at the target (mi 0,8 == x0,y32) adds the residual.
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
    // The residual leaf marked the 16x16 == 16 4x4 units.
    assert_eq!(sink.reconstructed_counts().0, before + 16);
    // Every sample equals the reference DC reconstruction over the flat-512 pred.
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
    // No `reconstruct_intrabc_block` ran, so no pending prediction is recorded.
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
    sink.reconstruct_intrabc_block(source, target, false, ByteOffset::new(0))
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
    // The real-IST residual deferred: the target keeps the bare prediction copy
    // (flat 512) and the count is unchanged (no residual leaf coverage marked).
    assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 32).unwrap(), 512);
    assert_eq!(sink.reconstructed_counts().0, before);
}

/// A fractional-vector IntrABC block (source and target differ in shape — the
/// BILINEAR border) is DEFERRED: the copy primitive only models the integer copy.
#[test]
fn intrabc_fractional_vector_block_is_deferred() {
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
    // A fractional vector widens the source by a one-sample BILINEAR border, so
    // source.size() != target.size().
    let source = PlaneRect::new(0, 0, 17, 17).unwrap();
    let target = PlaneRect::new(0, 32, 16, 16).unwrap();
    sink.reconstruct_intrabc_block(source, target, true, ByteOffset::new(0))
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
    // Square: never remapped, regardless of angle (asymmetric probe angles).
    assert_eq!(wide_angle_mapping(16, 16, 35), 35);
    assert_eq!(wide_angle_mapping(16, 16, 215), 215);

    // Tall `h == 2*w` (BLOCK_8X16): the `h == 2*w && pAngle < WAIP_WH_RATIO_2_THRES
    // (61)` branch adds 180. pAngle 58 wraps to 238; pAngle 81 (>= 61) does NOT.
    assert_eq!(wide_angle_mapping(8, 16, 58), 58 + 180);
    assert_eq!(wide_angle_mapping(8, 16, 81), 81);
    // Tall `h == 4*w` (BLOCK_8X32): threshold WAIP_WH_RATIO_4_THRES (73). pAngle 70
    // wraps; pAngle 76 (>= 73) does not.
    assert_eq!(wide_angle_mapping(8, 32, 70), 70 + 180);
    assert_eq!(wide_angle_mapping(8, 32, 76), 76);

    // Wide `w == 2*h` (BLOCK_16X8): the `w == 2*h && pAngle > 270 - WAIP_WH_RATIO_2
    // (209)` branch subtracts 180. pAngle 212 wraps to 32; pAngle 189 (<= 209) does
    // not.
    assert_eq!(wide_angle_mapping(16, 8, 212), 212 - 180);
    assert_eq!(wide_angle_mapping(16, 8, 189), 189);
    // Wide `w == 4*h` (BLOCK_32X8): threshold 270 - WAIP_WH_RATIO_4 (197). pAngle 200
    // wraps; pAngle 194 (<= 197) does not.
    assert_eq!(wide_angle_mapping(32, 8, 200), 200 - 180);
    assert_eq!(wide_angle_mapping(32, 8, 194), 194);
}
