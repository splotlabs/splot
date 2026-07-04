// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.15.4 inverse-transform parameter derivations.
//!
//! This module resolves the per-pass parameters the § 7.15.4 outer 2D inverse
//! transform ([`inverse_transform_2d_outer`](crate::inverse_transform_2d_outer))
//! consumes but leaves caller-resolved. It provides the [`transform_shift`]
//! lookup (the `rowShift` / `colShift` down-shifts), the
//! [`get_transform_1d_type`] row/column transform-type derivation, and the
//! [`dpcm_direction`] § 7.15.4 DPCM cumulative-sum direction selection. The
//! combined resolve helper that ties the shifts and types together is the
//! [`InverseTransform2dOuter::resolve`](crate::InverseTransform2dOuter::resolve)
//! constructor.
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
//! Feature tracking: `RECON-TRANSFORM-SHIFT-LOOKUP`,
//! `RECON-GET-TRANSFORM-1D-TYPE`, `RECON-DPCM-DIRECTION`.

use crate::inverse_transform::InverseTransform1dType;
use crate::inverse_transform_2d::InverseTransform2dDim;
use crate::inverse_transform_2d_outer::DpcmDirection;
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
/// This is a `const fn` so callers can resolve a fixed transform shape's shifts
/// at compile time; the manual `while`/index loop (rather than an iterator
/// combinator) keeps the body const-compatible.
///
/// # Errors
/// Returns [`ReconError::InvalidTransformShiftShape`] if `(log2_width,
/// log2_height)` is not one of the 25 AV2 `TX_SIZES_ALL` transform shapes.
pub const fn transform_shift(log2_width: u32, log2_height: u32) -> Result<(u8, u8)> {
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

/// The `TX_SIZES_ALL` ordinal `txSz` whose `(Tx_Width_Log2, Tx_Height_Log2)` equals
/// `(log2_width, log2_height)`, for indexing per-`txSz` § 9 tables such as
/// `Qm_Offset[txSz]`. The lookup is exact because `(log2W, log2H)` uniquely
/// identifies every ordinal.
///
/// # Errors
/// Returns [`ReconError::InvalidTransformShiftShape`] if `(log2_width,
/// log2_height)` is not one of the 25 AV2 `TX_SIZES_ALL` transform shapes.
pub const fn tx_size_index(log2_width: u32, log2_height: u32) -> Result<usize> {
    let mut i = 0;
    while i < TX_SIZE_LOG2_DIMS.len() {
        let (w, h) = TX_SIZE_LOG2_DIMS[i];
        if w == log2_width && h == log2_height {
            return Ok(i);
        }
        i += 1;
    }
    Err(ReconError::InvalidTransformShiftShape {
        log2_width,
        log2_height,
    })
}

/// Which § 7.15.4.1 transform pass a [`get_transform_1d_type`] query is for: the
/// row pass (`get_transform_1d_type(0, w)`) or the column pass
/// (`get_transform_1d_type(1, h)`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransformPass {
    /// The first (row) pass — spec `dir = 0`, sized by the transform width.
    Row,
    /// The second (column) pass — spec `dir = 1`, sized by the transform height.
    Col,
}

impl TransformPass {
    /// The spec `dir` table index (`Row = 0`, `Col = 1`).
    const fn dir_index(self) -> usize {
        match self {
            Self::Row => 0,
            Self::Col => 1,
        }
    }
}

const DCT: InverseTransform2dDim = InverseTransform2dDim::Kernel(InverseTransform1dType::Dct);
const ADST: InverseTransform2dDim = InverseTransform2dDim::Kernel(InverseTransform1dType::Adst);
const FDST: InverseTransform2dDim = InverseTransform2dDim::Kernel(InverseTransform1dType::Fdst);
const IDT: InverseTransform2dDim = InverseTransform2dDim::Identity;

/// AV2 § 7.15.4 `Transform_1d_Type[TX_TYPES][2]` = `(rowType, colType)` per
/// `PlaneTxType`, transcribed verbatim from the spec process body
/// (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-15-4`, the constant table at
/// lines 10679-10696). Like `Transform_Shift`, this is a § 7.15.4 process-body
/// constant that is **not** part of the generated `all_tables.h` § 9 attachment,
/// so it is a hand-written, spec-cited constant rather than a `gen-tables`
/// output. `IDT` maps to [`InverseTransform2dDim::Identity`]; the kernel types
/// map to [`InverseTransform2dDim::Kernel`].
const TRANSFORM_1D_TYPE: [[InverseTransform2dDim; 2]; 16] = [
    [DCT, DCT],   // 0  DCT_DCT
    [DCT, ADST],  // 1  ADST_DCT
    [ADST, DCT],  // 2  DCT_ADST
    [ADST, ADST], // 3  ADST_ADST
    [DCT, FDST],  // 4  FDST_DCT
    [FDST, DCT],  // 5  DCT_FDST
    [FDST, FDST], // 6  FDST_FDST
    [FDST, ADST], // 7  ADST_FDST
    [ADST, FDST], // 8  FDST_ADST
    [IDT, IDT],   // 9  IDTX
    [IDT, DCT],   // 10 V_DCT
    [DCT, IDT],   // 11 H_DCT
    [IDT, ADST],  // 12 V_ADST
    [ADST, IDT],  // 13 H_ADST
    [IDT, FDST],  // 14 V_FDST
    [FDST, IDT],  // 15 H_FDST
];

/// Returns the AV2 § 7.15.4 1D transform type for the `pass` pass of a transform
/// block, i.e. `get_transform_1d_type(dir, sz)` =
/// `Transform_1d_Type[PlaneTxType][dir]` with the `useDdt` substitution
/// (`07-decoding-process.md#s-7-15-4`).
///
/// `plane_tx_type` is `PlaneTxType` (`0..TX_TYPES`), `pass` selects the table
/// column (`dir`), and `size` is the pass dimension's adjusted sample size (`w`
/// for [`TransformPass::Row`], `h` for [`TransformPass::Col`]). When `use_ddt`
/// (the caller-resolved `enable_inter_ddt && !use_intrabc && is_inter`) is set
/// and the base type is `ADST` or `FDST` and `size != 4`, the type is replaced by
/// `DDTX` or `FDDT` respectively. The result drops into the `row_type` /
/// `col_type` fields of [`InverseTransform2dOuter`](crate::InverseTransform2dOuter).
///
/// # Errors
/// Returns [`ReconError::InvalidPlaneTxType`] if `plane_tx_type` is not a valid
/// `TX_TYPES` index (`0..16`).
pub const fn get_transform_1d_type(
    plane_tx_type: usize,
    pass: TransformPass,
    size: usize,
    use_ddt: bool,
) -> Result<InverseTransform2dDim> {
    if plane_tx_type >= TRANSFORM_1D_TYPE.len() {
        return Err(ReconError::InvalidPlaneTxType { plane_tx_type });
    }
    let base = TRANSFORM_1D_TYPE[plane_tx_type][pass.dir_index()];
    if use_ddt && size != 4 {
        match base {
            InverseTransform2dDim::Kernel(InverseTransform1dType::Adst) => {
                return Ok(InverseTransform2dDim::Kernel(InverseTransform1dType::Ddtx));
            }
            InverseTransform2dDim::Kernel(InverseTransform1dType::Fdst) => {
                return Ok(InverseTransform2dDim::Kernel(InverseTransform1dType::Fddt));
            }
            _ => {}
        }
    }
    Ok(base)
}

/// Selects the AV2 § 7.15.4 DPCM cumulative-sum direction for a transform block,
/// or `None` when DPCM is not applied
/// (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-15-4`).
///
/// § 7.15.4 sets `useDpcm = (plane == 0 ? use_dpcm_y : use_dpcm_uv)` and
/// `mode = (plane == 0 ? YMode : UVMode)`; when `useDpcm` is 1 the cumulative sum
/// runs down each column for `V_PRED` ([`DpcmDirection::Vertical`]) and across
/// each row otherwise ([`DpcmDirection::Horizontal`]). `use_dpcm` is the
/// plane-selected `useDpcm` flag and `mode_is_v_pred` is whether the
/// plane-selected prediction `mode` equals `V_PRED`, both caller-resolved so
/// `splot-recon` holds no frame state or prediction-mode enum. The result drops
/// into the `dpcm` field of
/// [`InverseTransform2dOuter`](crate::InverseTransform2dOuter), which applies the
/// cumulative sum after the inverse transform.
///
/// This is a `const fn` and is total: every `(use_dpcm, mode_is_v_pred)`
/// combination maps to a defined result with no error path.
pub const fn dpcm_direction(use_dpcm: bool, mode_is_v_pred: bool) -> Option<DpcmDirection> {
    if !use_dpcm {
        None
    } else if mode_is_v_pred {
        Some(DpcmDirection::Vertical)
    } else {
        Some(DpcmDirection::Horizontal)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const _DPCM_NONE: () = assert!(dpcm_direction(false, true).is_none());
    const _DPCM_VERTICAL: () = assert!(matches!(
        dpcm_direction(true, true),
        Some(DpcmDirection::Vertical)
    ));
    const _DPCM_HORIZONTAL: () = assert!(matches!(
        dpcm_direction(true, false),
        Some(DpcmDirection::Horizontal)
    ));

    const _CONST_EVAL_CHECK: () = assert!(matches!(transform_shift(2, 2), Ok((7, 10))));

    #[test]
    fn transform_shift_returns_the_parallel_table_entry_for_every_shape() {
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
        for (i, &a) in TX_SIZE_LOG2_DIMS.iter().enumerate() {
            for (j, &b) in TX_SIZE_LOG2_DIMS.iter().enumerate().skip(i + 1) {
                assert_ne!(a, b, "duplicate key at txSz {i} and {j}");
            }
        }
    }

    #[test]
    fn transform_shift_matches_independently_transcribed_spec_values() {
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
        assert!(matches!(
            transform_shift(3, 7),
            Err(ReconError::InvalidTransformShiftShape { .. })
        ));
    }

    use InverseTransform1dType::{Adst, Dct, Ddtx, Fddt, Fdst};
    use InverseTransform2dDim::{Identity, Kernel};

    const _CONST_TYPE_CHECK: () = assert!(matches!(
        get_transform_1d_type(1, TransformPass::Row, 8, false),
        Ok(Kernel(Dct))
    ));

    #[test]
    fn get_transform_1d_type_matches_the_spec_table_without_ddt() {
        let expected: [(InverseTransform2dDim, InverseTransform2dDim); 16] = [
            (Kernel(Dct), Kernel(Dct)),
            (Kernel(Dct), Kernel(Adst)),
            (Kernel(Adst), Kernel(Dct)),
            (Kernel(Adst), Kernel(Adst)),
            (Kernel(Dct), Kernel(Fdst)),
            (Kernel(Fdst), Kernel(Dct)),
            (Kernel(Fdst), Kernel(Fdst)),
            (Kernel(Fdst), Kernel(Adst)),
            (Kernel(Adst), Kernel(Fdst)),
            (Identity, Identity),
            (Identity, Kernel(Dct)),
            (Kernel(Dct), Identity),
            (Identity, Kernel(Adst)),
            (Kernel(Adst), Identity),
            (Identity, Kernel(Fdst)),
            (Kernel(Fdst), Identity),
        ];
        for (ptt, &(row, col)) in expected.iter().enumerate() {
            assert_eq!(
                get_transform_1d_type(ptt, TransformPass::Row, 8, false).unwrap(),
                row,
                "PlaneTxType {ptt} rowType"
            );
            assert_eq!(
                get_transform_1d_type(ptt, TransformPass::Col, 8, false).unwrap(),
                col,
                "PlaneTxType {ptt} colType"
            );
        }
    }

    #[test]
    fn get_transform_1d_type_applies_ddt_substitution_only_when_eligible() {
        assert_eq!(
            get_transform_1d_type(3, TransformPass::Row, 8, true).unwrap(),
            Kernel(Ddtx)
        );
        assert_eq!(
            get_transform_1d_type(6, TransformPass::Col, 16, true).unwrap(),
            Kernel(Fddt)
        );
        assert_eq!(
            get_transform_1d_type(3, TransformPass::Row, 4, true).unwrap(),
            Kernel(Adst)
        );
        assert_eq!(
            get_transform_1d_type(6, TransformPass::Row, 16, false).unwrap(),
            Kernel(Fdst)
        );
        assert_eq!(
            get_transform_1d_type(0, TransformPass::Row, 16, true).unwrap(),
            Kernel(Dct)
        );
        assert_eq!(
            get_transform_1d_type(9, TransformPass::Col, 16, true).unwrap(),
            Identity
        );
    }

    #[test]
    fn get_transform_1d_type_rejects_out_of_range_plane_tx_type() {
        assert!(matches!(
            get_transform_1d_type(16, TransformPass::Row, 8, false),
            Err(ReconError::InvalidPlaneTxType { plane_tx_type: 16 })
        ));
        assert!(matches!(
            get_transform_1d_type(usize::MAX, TransformPass::Col, 8, true),
            Err(ReconError::InvalidPlaneTxType { .. })
        ));
    }

    #[test]
    fn dpcm_direction_maps_the_four_spec_cases() {
        assert_eq!(dpcm_direction(false, false), None);
        assert_eq!(dpcm_direction(false, true), None);
        assert_eq!(dpcm_direction(true, true), Some(DpcmDirection::Vertical));
        assert_eq!(dpcm_direction(true, false), Some(DpcmDirection::Horizontal));
    }

    #[test]
    fn selected_direction_drives_the_outer_cumulative_sum() {
        use crate::{BitDepth, InverseTransform2dOuter, inverse_transform_2d_outer};

        let dequant = [8i32; 16];
        let resolve = |dpcm| {
            InverseTransform2dOuter::resolve(9, 2, 2, false, true, BitDepth::Eight, dpcm).unwrap()
        };

        let mut vertical = [0i32; 16];
        inverse_transform_2d_outer(
            &resolve(dpcm_direction(true, true)),
            &dequant,
            &mut vertical,
        )
        .unwrap();
        assert_eq!(
            vertical,
            [1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4],
            "Vertical DPCM accumulates down each column"
        );

        let mut none = [0i32; 16];
        inverse_transform_2d_outer(&resolve(dpcm_direction(false, true)), &dequant, &mut none)
            .unwrap();
        assert_eq!(none, [1i32; 16], "no DPCM leaves the residual unchanged");
    }
}
