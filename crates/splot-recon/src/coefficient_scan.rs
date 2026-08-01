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

/// The four legal operating dimensions `Min(Tx_Width[txSz], 32)` /
/// `Min(Tx_Height[txSz], 32)`.
const SCAN_DIMS: [usize; 4] = [4, 8, 16, 32];

/// Total scan positions over every `(w, h)` shape: `(4 + 8 + 16 + 32)^2`.
const SCAN_TABLE_LEN: usize = 3600;

/// The largest single scan, `32 * 32`.
const MAX_SCAN_LEN: usize = 1024;

/// Start of each `(w, h)` shape's run inside a packed per-class scan table.
const SCAN_OFFSETS: [[usize; 4]; 4] = build_scan_offsets();

/// `TX_CLASS_VERT` is the identity permutation, so every shape's scan is the
/// `w * h` prefix of one shared table rather than its own packed run.
const VERTICAL_SCAN: [u16; MAX_SCAN_LEN] = build_vertical_scan();

/// Packed `TX_CLASS_HORIZ` scans for every `(w, h)` shape.
const HORIZONTAL_SCAN: [u16; SCAN_TABLE_LEN] = build_horizontal_scan();

/// Packed `TX_CLASS_2D` scans for every `(w, h)` shape.
const TWO_D_SCAN: [u16; SCAN_TABLE_LEN] = build_two_d_scan();

const fn build_scan_offsets() -> [[usize; 4]; 4] {
    let mut offsets = [[0usize; 4]; 4];
    let mut offset = 0;
    let mut wi = 0;
    while wi < 4 {
        let mut hi = 0;
        while hi < 4 {
            offsets[wi][hi] = offset;
            offset += SCAN_DIMS[wi] * SCAN_DIMS[hi];
            hi += 1;
        }
        wi += 1;
    }
    offsets
}

const fn build_vertical_scan() -> [u16; MAX_SCAN_LEN] {
    let mut out = [0u16; MAX_SCAN_LEN];
    let mut c = 0;
    while c < MAX_SCAN_LEN {
        out[c] = c as u16;
        c += 1;
    }
    out
}

#[allow(clippy::many_single_char_names)]
const fn build_horizontal_scan() -> [u16; SCAN_TABLE_LEN] {
    let mut out = [0u16; SCAN_TABLE_LEN];
    let mut wi = 0;
    while wi < 4 {
        let mut hi = 0;
        while hi < 4 {
            let (w, h, base) = (SCAN_DIMS[wi], SCAN_DIMS[hi], SCAN_OFFSETS[wi][hi]);
            let mut c = 0;
            let mut x = 0;
            while x < w {
                let mut y = 0;
                while y < h {
                    out[base + c] = (y * w + x) as u16;
                    c += 1;
                    y += 1;
                }
                x += 1;
            }
            hi += 1;
        }
        wi += 1;
    }
    out
}

#[allow(clippy::many_single_char_names)]
const fn build_two_d_scan() -> [u16; SCAN_TABLE_LEN] {
    let mut out = [0u16; SCAN_TABLE_LEN];
    let mut wi = 0;
    while wi < 4 {
        let mut hi = 0;
        while hi < 4 {
            let base = SCAN_OFFSETS[wi][hi];
            let (w, h) = (SCAN_DIMS[wi] as i32, SCAN_DIMS[hi] as i32);
            let len = (w * h) as usize;
            let (mut x, mut y) = (0i32, 0i32);
            let mut c = 0;
            while c < len {
                out[base + c] = (y * w + x) as u16;
                x += 1;
                y -= 1;
                if y < 0 || x >= w {
                    x += 1;
                    let span = if x < h - 1 - y { x } else { h - 1 - y };
                    x -= span;
                    y += span;
                }
                c += 1;
            }
            hi += 1;
        }
        wi += 1;
    }
    out
}

const fn scan_dim_index(d: usize) -> Option<usize> {
    match d {
        4 => Some(0),
        8 => Some(1),
        16 => Some(2),
        32 => Some(3),
        _ => None,
    }
}

/// Returns the AV2 § 5.20.7.30 `get_scan(txSz, txClass)` coefficient scan order
/// for a `w * h` transform block as a borrowed table
/// ([`05-syntax-structures.md`](../../../docs/spec/av2/1.0.0/05-syntax-structures.md)
/// `#s-5-20-7-30`). `scan[c]` is the flattened `y * w + x` position of the `c`-th
/// scanned coefficient.
///
/// The scan is a pure function of `(w, h, class)`, so the 48 permutations are
/// built once at compile time and every caller borrows one. Callers that only
/// read the order should prefer this over [`coefficient_scan_order`], which
/// copies into caller storage.
///
/// `w` and `h` are the operating dimensions `Min(Tx_Width[txSz], 32)` /
/// `Min(Tx_Height[txSz], 32)`, each 4, 8, 16, or 32.
///
/// # Errors
/// Returns [`ReconError::InvalidScanShape`] if `w` / `h` are not each 4/8/16/32.
pub fn coefficient_scan_slice(w: usize, h: usize, class: TransformClass) -> Result<&'static [u16]> {
    let invalid = || ReconError::InvalidScanShape { w, h };
    let wi = scan_dim_index(w).ok_or_else(invalid)?;
    let hi = scan_dim_index(h).ok_or_else(invalid)?;
    let len = w * h;
    let (table, base) = match class {
        TransformClass::Vertical => (VERTICAL_SCAN.as_slice(), 0),
        TransformClass::Horizontal => (HORIZONTAL_SCAN.as_slice(), SCAN_OFFSETS[wi][hi]),
        TransformClass::TwoD => (TWO_D_SCAN.as_slice(), SCAN_OFFSETS[wi][hi]),
    };
    table.get(base..base + len).ok_or_else(invalid)
}

/// Writes the AV2 § 5.20.7.30 `get_scan(txSz, txClass)` coefficient scan order
/// for a `w * h` transform block into `out`. `out[c]` is the flattened
/// `y * w + x` position of the `c`-th scanned coefficient.
///
/// `w` and `h` are the operating dimensions `Min(Tx_Width[txSz], 32)` /
/// `Min(Tx_Height[txSz], 32)`, each 4, 8, 16, or 32. Scan positions never exceed
/// `w * h - 1 <= 1023`, so they fit `u16`.
///
/// Callers that only read the order should use [`coefficient_scan_slice`], which
/// borrows the same table and copies nothing. This entry point exists for
/// callers that already own scan storage.
///
/// # Errors
/// Returns [`ReconError::InvalidScanShape`] if `w` / `h` are not each 4/8/16/32,
/// and [`ReconError::ScanLengthMismatch`] if `out` is not exactly `w * h` long.
pub fn coefficient_scan_order(
    w: usize,
    h: usize,
    class: TransformClass,
    out: &mut [u16],
) -> Result<()> {
    let scan = coefficient_scan_slice(w, h, class)?;
    if out.len() != scan.len() {
        return Err(ReconError::ScanLengthMismatch {
            expected: scan.len(),
            out_len: out.len(),
        });
    }
    out.copy_from_slice(scan); // splot-copy-ok: fill caller-owned scan storage
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

    /// The § 5.20.7.30 walk written straight from the spec text, independent of
    /// the compile-time tables it checks.
    #[allow(clippy::many_single_char_names)]
    fn reference_scan(w: usize, h: usize, class: TransformClass) -> Vec<u16> {
        let mut out = vec![0u16; w * h];
        match class {
            TransformClass::Vertical => {
                for (c, slot) in out.iter_mut().enumerate() {
                    *slot = c as u16;
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
                for slot in &mut out {
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
        out
    }

    #[test]
    fn compile_time_tables_match_the_spec_walk_for_every_shape_and_class() {
        for &w in &[4usize, 8, 16, 32] {
            for &h in &[4usize, 8, 16, 32] {
                for class in [
                    TransformClass::TwoD,
                    TransformClass::Horizontal,
                    TransformClass::Vertical,
                ] {
                    let table = coefficient_scan_slice(w, h, class).unwrap();
                    assert_eq!(table.len(), w * h, "{w}x{h} {class:?} length");
                    assert_eq!(
                        table,
                        reference_scan(w, h, class).as_slice(),
                        "{w}x{h} {class:?} scan"
                    );
                }
            }
        }
    }

    #[test]
    fn scan_slice_rejects_an_invalid_shape() {
        assert!(matches!(
            coefficient_scan_slice(5, 4, TransformClass::TwoD),
            Err(ReconError::InvalidScanShape { w: 5, h: 4 })
        ));
        assert!(matches!(
            coefficient_scan_slice(4, 64, TransformClass::Vertical),
            Err(ReconError::InvalidScanShape { w: 4, h: 64 })
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
