// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::super::max_level::coeff_is_low_frequency;
use super::*;

#[test]
fn low_frequency_positions_are_a_scan_prefix() {
    use splot_recon::{TransformClass, coefficient_scan_slice};
    fn check<const CLASS: usize, const LUMA: bool>(class: TransformClass) {
        let tx_class = [
            CoeffTransformClass::TwoD,
            CoeffTransformClass::Horizontal,
            CoeffTransformClass::Vertical,
        ][CLASS];
        for width_log2 in 2..=5u32 {
            for height in [4, 8, 16, 32] {
                let width = 1usize << width_log2;
                let scan = coefficient_scan_slice(width, height, class).unwrap();
                let end = lf_scan_end::<CLASS, LUMA>(width, height);
                for (scan_index, &pos) in scan.iter().enumerate() {
                    let pos = usize::from(pos);
                    let entry =
                        CoeffScanEntry::new(scan_index, pos, pos >> width_log2, pos % width);
                    let is_lf = coeff_is_low_frequency(entry, usize::from(!LUMA), tx_class);
                    assert_eq!(is_lf, scan_index < end, "{CLASS} {LUMA} {width}x{height}");
                }
            }
        }
    }
    check::<0, true>(TransformClass::TwoD);
    check::<0, false>(TransformClass::TwoD);
    check::<1, true>(TransformClass::Horizontal);
    check::<1, false>(TransformClass::Horizontal);
    check::<2, true>(TransformClass::Vertical);
    check::<2, false>(TransformClass::Vertical);
}

#[test]
fn grid_scans_hold_each_shape_scan_with_its_grid_index() {
    for (class, recon_class) in [
        (CoeffTransformClass::TwoD, TransformClass::TwoD),
        (CoeffTransformClass::Horizontal, TransformClass::Horizontal),
        (CoeffTransformClass::Vertical, TransformClass::Vertical),
    ] {
        for width in [4, 8, 16, 32] {
            for height in [4, 8, 16, 32] {
                let scan = splot_recon::coefficient_scan_slice(width, height, recon_class).unwrap();
                let grid = grid_scan(class, width, height, scan.len()).unwrap();
                for (&pos, &entry) in scan.iter().zip(grid) {
                    let pos = usize::from(pos);
                    assert_eq!(entry >> 16, pos as u32);
                    let expected = (pos / width) * LEVEL_STRIDE + pos % width;
                    assert_eq!(entry_grid(entry), expected);
                }
            }
        }
    }
}

#[test]
fn scan_extent_bounds_every_scan_prefix() {
    for (class, recon_class) in [
        (CoeffTransformClass::TwoD, TransformClass::TwoD),
        (CoeffTransformClass::Horizontal, TransformClass::Horizontal),
        (CoeffTransformClass::Vertical, TransformClass::Vertical),
    ] {
        for width in [4, 8, 16, 32] {
            for height in [4, 8, 16, 32] {
                let scan = splot_recon::coefficient_scan_slice(width, height, recon_class).unwrap();
                let mut extent = 0;
                let log2 = (
                    width.trailing_zeros() as usize,
                    height.trailing_zeros() as usize,
                );
                for (eob, &pos) in (1..).zip(scan) {
                    extent = extent.max(usize::from(pos) + 1);
                    assert_eq!(scan_extent(class, log2, eob), extent);
                }
            }
        }
    }
    let class = CoeffTransformClass::TwoD;
    assert_eq!(scan_extent(class, (2, 2), 0), 16);
    assert_eq!(scan_extent(class, (2, 2), 17), 16);
    assert_eq!(scan_extent(class, (6, 6), 1), 1);
    assert_eq!(scan_extent(class, (6, 6), 1025), 1024);
}

#[test]
fn grid_scan_rejects_bad_shapes_and_eobs() {
    let class = CoeffTransformClass::TwoD;
    assert!(grid_scan(class, 12, 4, 1).is_err());
    assert!(grid_scan(class, 4, 64, 1).is_err());
    assert!(grid_scan(class, 4, 4, 0).is_err());
    assert!(grid_scan(class, 4, 4, 17).is_err());
    assert_eq!(grid_scan(class, 4, 4, 16).unwrap().len(), 16);
}

#[test]
fn scan_entry_context_classes_give_the_luma_2d_base_contexts() {
    for width in [4, 8, 16, 32] {
        for height in [4, 8, 16, 32] {
            let len = width * height;
            let scan = grid_scan(CoeffTransformClass::TwoD, width, height, len).unwrap();
            for (scan_index, &entry) in scan.iter().enumerate().skip(1) {
                let pos = (entry >> 16) as usize;
                let diagonal = pos / width + pos % width;
                let class = ctx_class_of(entry);
                let (hf, lf) = (&CTX_2D[0][class], &CTX_2D[1][class]);
                for (mag, (&hf, &lf)) in hf.iter().zip(lf).enumerate() {
                    let half = (mag + 1) >> 1;
                    if scan_index < lf_scan_end::<0, true>(width, height) {
                        let (cap, offset) =
                            LF_2D_CONTEXT_CAP_AND_OFFSET[1 + usize::from(diagonal >= 2)];
                        assert_eq!(usize::from(lf), half.min(cap) + offset);
                    } else {
                        let bucket = usize::from(diagonal >= 6) + usize::from(diagonal >= 8);
                        let ctx = half.min(4) + HF_2D_CONTEXT_OFFSET[bucket];
                        assert_eq!(usize::from(hf), 2 * ctx);
                    }
                }
            }
        }
    }
}

#[test]
fn tx_geometry_matches_the_transform_size_tables() {
    use splot_core::tables::conversion::{
        ADJUSTED_TX_SIZE, TX_HEIGHT, TX_HEIGHT_LOG2, TX_WIDTH, TX_WIDTH_LOG2,
    };

    use super::super::super::general_intra_residual::txb_skip_tx_size_ctx;
    use super::super::{NonZeroCoeffEobContextInput, TX_GEOMETRY, nonzero_coeff_eob_symbol_input};

    for (tx_size, tx) in TX_GEOMETRY.iter().enumerate() {
        let (w4, h4) = (
            TX_WIDTH[tx_size] as usize / 4,
            TX_HEIGHT[tx_size] as usize / 4,
        );
        assert_eq!((usize::from(tx.w4), usize::from(tx.h4)), (w4, h4));
        let adjusted = ADJUSTED_TX_SIZE[tx_size] as usize;
        let (width, height) = (TX_WIDTH[adjusted] as usize, TX_HEIGHT[adjusted] as usize);
        assert_eq!(
            (usize::from(tx.width), usize::from(tx.height)),
            (width, height)
        );
        assert_eq!((1 << tx.width_log2, 1 << tx.height_log2), (width, height));
        assert_eq!(usize::from(tx.size_ctx), txb_skip_tx_size_ctx(tx_size));
        let eob = nonzero_coeff_eob_symbol_input(NonZeroCoeffEobContextInput {
            plane: 0,
            is_inter: false,
            tx_width_log2: TX_WIDTH_LOG2[tx_size] as usize,
            tx_height_log2: TX_HEIGHT_LOG2[tx_size] as usize,
        })
        .unwrap();
        assert_eq!(tx.eob_size, eob.size, "tx_size {tx_size}");
    }
}
