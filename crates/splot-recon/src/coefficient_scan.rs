// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.7.30 `get_scan` coefficient scan order.
//!
//! Computes the order in which transform coefficients are scanned for a `w * h`
//! transform block ([`05-syntax-structures.md`](../../../docs/spec/av2/1.0.0/05-syntax-structures.md)
//! `#s-5-20-7-30`). The coefficient decode loop (§ 5.20.7) and the § 7.14.4
//! coefficient placement consume this order; both will call it once the loop is
//! wired. It is a pure permutation of `0..w*h`:
//!
//! - `TX_CLASS_VERT`: row-major raster order (identity).
//! - `TX_CLASS_HORIZ`: column-major (transpose) order.
//! - `TX_CLASS_2D`: the AV2 anti-diagonal scan — each anti-diagonal `x + y`
//!   traversed from high `y` (low `x`) to low `y` (high `x`).
//!
//! Like the rest of `splot-recon`, the block shape is caller-resolved: callers
//! pass `w = Min(Tx_Width[txSz], 32)` and `h = Min(Tx_Height[txSz], 32)` rather
//! than a `txSz` enum (`splot-recon` cannot reach `splot-core`'s § 9.2 tables).
//!
//! It also provides [`tx_class`], the AV2 `get_tx_class` mapping from a
//! `PlaneTxType` to its [`TransformClass`] (the class then selects the scan via
//! [`coefficient_scan_order`]).
//!
//! Feature tracking: `RECON-COEFFICIENT-SCAN-ORDER`, `RECON-GET-TX-CLASS`.

use crate::{ReconError, Result};

/// AV2 transform class (`03-symbols.md`): selects the § 5.20.7.30 scan pattern.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransformClass {
    /// `TX_CLASS_2D` (value 0): the anti-diagonal scan.
    TwoD,
    /// `TX_CLASS_HORIZ` (value 1): column-major (transpose) scan.
    Horizontal,
    /// `TX_CLASS_VERT` (value 2): row-major (raster) scan.
    Vertical,
}

/// Writes the AV2 § 5.20.7.30 `get_scan(txSz, txClass)` coefficient scan order
/// for a `w * h` transform block into `out`. `out[c]` is the flattened
/// `y * w + x` position of the `c`-th scanned coefficient.
///
/// `w` and `h` are the operating dimensions `Min(Tx_Width[txSz], 32)` /
/// `Min(Tx_Height[txSz], 32)`, each 4, 8, 16, or 32. Scan positions never exceed
/// `w * h - 1 <= 1023`, so they fit `u16`.
///
/// # Errors
/// Returns [`ReconError::InvalidScanShape`] if `w` / `h` are not each 4/8/16/32,
/// and [`ReconError::ScanLengthMismatch`] if `out` is not exactly `w * h` long.
#[allow(clippy::many_single_char_names)]
pub fn coefficient_scan_order(
    w: usize,
    h: usize,
    class: TransformClass,
    out: &mut [u16],
) -> Result<()> {
    if !matches!(w, 4 | 8 | 16 | 32) || !matches!(h, 4 | 8 | 16 | 32) {
        return Err(ReconError::InvalidScanShape { w, h });
    }
    let expected = w * h;
    if out.len() != expected {
        return Err(ReconError::ScanLengthMismatch {
            expected,
            out_len: out.len(),
        });
    }
    match class {
        TransformClass::Vertical => {
            let mut c = 0;
            for y in 0..h {
                for x in 0..w {
                    out[c] = (y * w + x) as u16;
                    c += 1;
                }
            }
        }
        TransformClass::Horizontal => {
            let mut c = 0;
            for x in 0..w {
                for y in 0..h {
                    out[c] = (y * w + x) as u16;
                    c += 1;
                }
            }
        }
        TransformClass::TwoD => {
            let (wi, hi) = (w as i32, h as i32);
            let (mut x, mut y) = (0i32, 0i32);
            for slot in out.iter_mut() {
                *slot = (y * wi + x) as u16;
                x += 1;
                y -= 1;
                if y < 0 || x >= wi {
                    x += 1;
                    let s = x.min(hi - 1 - y);
                    x -= s;
                    y += s;
                }
            }
        }
    }
    Ok(())
}

/// Returns the AV2 § 8.3.2 `get_tx_class(txType)` transform class for a
/// `PlaneTxType`
/// ([`08-parsing-process.md`](../../../docs/spec/av2/1.0.0/08-parsing-process.md)
/// `#s-8-3-2`): the vertical-only transforms map to
/// [`TransformClass::Vertical`], the horizontal-only transforms to
/// [`TransformClass::Horizontal`], and every other transform (including all 2D
/// and identity transforms) to [`TransformClass::TwoD`].
///
/// The vertical types are `V_DCT` (10), `V_ADST` (12), and `V_FLIPADST` (14); the
/// horizontal types are `H_DCT` (11), `H_ADST` (13), and `H_FLIPADST` (15)
/// (`03-symbols.md` `TX_TYPE` values). The result selects the scan in
/// [`coefficient_scan_order`]. Total over all inputs: the spec's `else` branch
/// maps any non-directional value to `TX_CLASS_2D`.
#[must_use]
pub const fn tx_class(plane_tx_type: usize) -> TransformClass {
    match plane_tx_type {
        10 | 12 | 14 => TransformClass::Vertical,
        11 | 13 | 15 => TransformClass::Horizontal,
        _ => TransformClass::TwoD,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const _CONST_CLASS_CHECK: () = assert!(matches!(tx_class(10), TransformClass::Vertical));

    fn scan(w: usize, h: usize, class: TransformClass) -> Vec<u16> {
        let mut out = vec![0u16; w * h];
        coefficient_scan_order(w, h, class, &mut out).unwrap();
        out
    }

    fn assert_is_permutation(order: &[u16], n: usize) {
        let mut seen = vec![false; n];
        for &p in order {
            let p = usize::from(p);
            assert!(p < n, "position {p} out of range for {n}");
            assert!(!seen[p], "position {p} repeated");
            seen[p] = true;
        }
        assert!(seen.iter().all(|&s| s), "not all positions covered");
    }

    #[test]
    fn two_d_4x4_matches_the_spec_anti_diagonal_scan() {
        let expected: [u16; 16] = [0, 4, 1, 8, 5, 2, 12, 9, 6, 3, 13, 10, 7, 14, 11, 15];
        assert_eq!(scan(4, 4, TransformClass::TwoD).as_slice(), &expected);
    }

    #[test]
    fn vertical_scan_is_row_major_identity() {
        let order = scan(8, 4, TransformClass::Vertical);
        for (c, &p) in order.iter().enumerate() {
            assert_eq!(usize::from(p), c, "vertical scan slot {c}");
        }
    }

    #[test]
    fn horizontal_scan_is_column_major_transpose() {
        let expected: [u16; 16] = [0, 4, 8, 12, 1, 5, 9, 13, 2, 6, 10, 14, 3, 7, 11, 15];
        assert_eq!(scan(4, 4, TransformClass::Horizontal).as_slice(), &expected);
    }

    #[test]
    fn every_shape_and_class_is_a_valid_permutation() {
        for &w in &[4usize, 8, 16, 32] {
            for &h in &[4usize, 8, 16, 32] {
                for class in [
                    TransformClass::TwoD,
                    TransformClass::Horizontal,
                    TransformClass::Vertical,
                ] {
                    assert_is_permutation(&scan(w, h, class), w * h);
                }
            }
        }
    }

    #[test]
    fn rejects_invalid_shape_and_length() {
        let mut out = [0u16; 20];
        assert!(matches!(
            coefficient_scan_order(5, 4, TransformClass::TwoD, &mut out),
            Err(ReconError::InvalidScanShape { w: 5, h: 4 })
        ));
        let mut short = [0u16; 15];
        assert!(matches!(
            coefficient_scan_order(4, 4, TransformClass::TwoD, &mut short),
            Err(ReconError::ScanLengthMismatch {
                expected: 16,
                out_len: 15
            })
        ));
    }

    #[test]
    fn tx_class_maps_every_plane_tx_type() {
        for t in [10usize, 12, 14] {
            assert_eq!(tx_class(t), TransformClass::Vertical, "txType {t}");
        }
        for t in [11usize, 13, 15] {
            assert_eq!(tx_class(t), TransformClass::Horizontal, "txType {t}");
        }
        for t in 0..=9usize {
            assert_eq!(tx_class(t), TransformClass::TwoD, "txType {t}");
        }
        assert_eq!(tx_class(16), TransformClass::TwoD);
        assert_eq!(tx_class(usize::MAX), TransformClass::TwoD);
    }
}
