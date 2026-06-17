// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.15.4 inverse-transform parameter derivations.
//!
//! This module resolves the per-pass parameters the § 7.15.4 outer 2D inverse
//! transform ([`inverse_transform_2d_outer`](crate::inverse_transform_2d_outer))
//! consumes but leaves caller-resolved. It currently provides the
//! [`transform_shift`] lookup (the `rowShift` / `colShift` down-shifts); the
//! `get_transform_1d_type` row/column transform-type derivation and the DPCM
//! direction selection are future rows.
//!
//! Like the rest of `splot-recon`, transform shape is keyed by the original
//! (unadjusted) `(log2W, log2H)` base-2 log dimensions rather than a `txSz`
//! enum: `splot-recon` cannot depend on `splot-core`'s § 9.2 conversion tables
//! (the one-way dependency rule), so callers resolve `txSz`-derived values and
//! pass the log2 dimensions, exactly as
//! [`InverseTransform2dOuter`](crate::InverseTransform2dOuter) already does. The
//! spec `Tx_Width_Log2` / `Tx_Height_Log2` tables prove `(log2W, log2H)`
//! uniquely identifies every `TX_SIZES_ALL` ordinal, so the lookup is exact.
//!
//! Feature tracking: `RECON-TRANSFORM-SHIFT-LOOKUP`.

use crate::{ReconError, Result};

/// AV2 § 7.15.4 `Transform_Shift[TX_SIZES_ALL][2]` = `(rowShift, colShift)` per
/// transform-size ordinal, transcribed verbatim from the spec process body
/// (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-15-4`, the constant table at
/// lines 10610-10636). This is a § 7.15.4 process-body constant and is **not**
/// part of the generated `all_tables.h` § 9 attachment, so it is a hand-written,
/// spec-cited constant rather than a `cargo xtask gen-tables` output.
const TRANSFORM_SHIFT: [(u8, u8); 25] = [
    (7, 10), // 0  TX_4X4
    (7, 11), // 1  TX_8X8
    (6, 13), // 2  TX_16X16
    (6, 13), // 3  TX_32X32
    (6, 13), // 4  TX_64X64
    (7, 10), // 5  TX_4X8
    (7, 10), // 6  TX_8X4
    (7, 11), // 7  TX_8X16
    (7, 11), // 8  TX_16X8
    (6, 12), // 9  TX_16X32
    (6, 12), // 10 TX_32X16
    (6, 12), // 11 TX_32X64
    (6, 12), // 12 TX_64X32
    (6, 12), // 13 TX_4X16
    (6, 12), // 14 TX_16X4
    (6, 13), // 15 TX_8X32
    (6, 13), // 16 TX_32X8
    (6, 13), // 17 TX_16X64
    (6, 13), // 18 TX_64X16
    (7, 11), // 19 TX_4X32
    (7, 11), // 20 TX_32X4
    (6, 12), // 21 TX_8X64
    (6, 12), // 22 TX_64X8
    (6, 13), // 23 TX_4X64
    (6, 13), // 24 TX_64X4
];

/// AV2 § 9 `(Tx_Width_Log2[txSz], Tx_Height_Log2[txSz])` per transform-size
/// ordinal, parallel to [`TRANSFORM_SHIFT`] (same `txSz` index). Mirrored from
/// the generated `all_tables.h` § 9.2 values (`Tx_Width_Log2` / `Tx_Height_Log2`),
/// which `splot-recon` cannot reach through `splot-core`. Used only to key the
/// shift lookup by the original `(log2W, log2H)` shape; the
/// `tx_size_log2_dims_keys_are_distinct` test pins the uniqueness invariant the
/// search relies on, and the spot-check tests pin individual values against the
/// spec.
const TX_SIZE_LOG2_DIMS: [(u32, u32); 25] = [
    (2, 2), // 0  TX_4X4
    (3, 3), // 1  TX_8X8
    (4, 4), // 2  TX_16X16
    (5, 5), // 3  TX_32X32
    (6, 6), // 4  TX_64X64
    (2, 3), // 5  TX_4X8
    (3, 2), // 6  TX_8X4
    (3, 4), // 7  TX_8X16
    (4, 3), // 8  TX_16X8
    (4, 5), // 9  TX_16X32
    (5, 4), // 10 TX_32X16
    (5, 6), // 11 TX_32X64
    (6, 5), // 12 TX_64X32
    (2, 4), // 13 TX_4X16
    (4, 2), // 14 TX_16X4
    (3, 5), // 15 TX_8X32
    (5, 3), // 16 TX_32X8
    (4, 6), // 17 TX_16X64
    (6, 4), // 18 TX_64X16
    (2, 5), // 19 TX_4X32
    (5, 2), // 20 TX_32X4
    (3, 6), // 21 TX_8X64
    (6, 3), // 22 TX_64X8
    (2, 6), // 23 TX_4X64
    (6, 2), // 24 TX_64X4
];

/// Returns the AV2 § 7.15.4 `(rowShift, colShift)` down-shifts for a transform
/// block of original (unadjusted) base-2 log dimensions `(log2_width,
/// log2_height)` — i.e. `(Transform_Shift[txSz][0], Transform_Shift[txSz][1])`
/// for the `txSz` whose `(Tx_Width_Log2, Tx_Height_Log2)` equals the requested
/// shape (`07-decoding-process.md#s-7-15-4`).
///
/// The result drops straight into the `row_shift` / `col_shift` fields of
/// [`InverseTransform2dOuter`](crate::InverseTransform2dOuter).
///
/// # Errors
/// Returns [`ReconError::InvalidTransformShiftShape`] if `(log2_width,
/// log2_height)` is not one of the 25 AV2 `TX_SIZES_ALL` transform shapes.
pub fn transform_shift(log2_width: u32, log2_height: u32) -> Result<(u8, u8)> {
    let mut i = 0;
    while i < TX_SIZE_LOG2_DIMS.len() {
        let (w, h) = TX_SIZE_LOG2_DIMS[i];
        if w == log2_width && h == log2_height {
            return Ok(TRANSFORM_SHIFT[i]);
        }
        i += 1;
    }
    Err(ReconError::InvalidTransformShiftShape {
        log2_width,
        log2_height,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn transform_shift_returns_the_parallel_table_entry_for_every_shape() {
        // Each (log2W, log2H) key maps to the Transform_Shift row at the same
        // txSz index — the search must find each of the 25 ordinals exactly.
        for (i, &(w, h)) in TX_SIZE_LOG2_DIMS.iter().enumerate() {
            assert_eq!(
                transform_shift(w, h).unwrap(),
                TRANSFORM_SHIFT[i],
                "shape ({w},{h}) at txSz {i}"
            );
        }
    }

    #[test]
    fn tx_size_log2_dims_keys_are_distinct() {
        // The (log2W, log2H) lookup key is only valid if it uniquely identifies
        // each TX_SIZES_ALL ordinal (Tx_Width_Log2/Tx_Height_Log2 prove this).
        for (i, &a) in TX_SIZE_LOG2_DIMS.iter().enumerate() {
            for (j, &b) in TX_SIZE_LOG2_DIMS.iter().enumerate().skip(i + 1) {
                assert_ne!(a, b, "duplicate key at txSz {i} and {j}");
            }
        }
    }

    #[test]
    fn transform_shift_matches_independently_transcribed_spec_values() {
        // Spot values transcribed directly from the spec table (NOT from the
        // module constants): 07-decoding-process.md#s-7-15-4 lines 10610-10636,
        // paired with Tx_Width_Log2/Tx_Height_Log2. (rowShift = [0], colShift = [1].)
        assert_eq!(transform_shift(2, 2).unwrap(), (7, 10)); // TX_4X4
        assert_eq!(transform_shift(3, 3).unwrap(), (7, 11)); // TX_8X8
        assert_eq!(transform_shift(4, 4).unwrap(), (6, 13)); // TX_16X16
        assert_eq!(transform_shift(6, 6).unwrap(), (6, 13)); // TX_64X64
        assert_eq!(transform_shift(4, 5).unwrap(), (6, 12)); // TX_16X32
        assert_eq!(transform_shift(2, 5).unwrap(), (7, 11)); // TX_4X32
        assert_eq!(transform_shift(6, 2).unwrap(), (6, 13)); // TX_64X4
    }

    #[test]
    fn transform_shift_is_symmetric_under_transpose() {
        // Every TX_WxH and its TX_HxW transpose share the same shifts (the spec
        // table is transpose-symmetric); verify on the rectangular shapes.
        for &(w, h) in &TX_SIZE_LOG2_DIMS {
            if w != h {
                assert_eq!(
                    transform_shift(w, h).unwrap(),
                    transform_shift(h, w).unwrap(),
                    "({w},{h}) vs transpose"
                );
            }
        }
    }

    #[test]
    fn transform_shift_rejects_non_av2_shapes() {
        // A (log2W, log2H) that is not one of the 25 AV2 transform shapes is a
        // typed error, never a panic.
        assert!(matches!(
            transform_shift(1, 1),
            Err(ReconError::InvalidTransformShiftShape {
                log2_width: 1,
                log2_height: 1
            })
        ));
        assert!(matches!(
            transform_shift(7, 7),
            Err(ReconError::InvalidTransformShiftShape {
                log2_width: 7,
                log2_height: 7
            })
        ));
        // A 3x6/6x3 exists, but 3x7 does not.
        assert!(matches!(
            transform_shift(3, 7),
            Err(ReconError::InvalidTransformShiftShape { .. })
        ));
    }
}
