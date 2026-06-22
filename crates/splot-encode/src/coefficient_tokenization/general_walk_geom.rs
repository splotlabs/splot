// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The size-generic transform-geometry descriptor ([`TxGeom`]) for the GENERAL
//! coefficient-tokenization walk ([`super::general_walk`]). Split out so the SAME
//! reverse-scan base/sign codepath tokenizes both the 4x4 DCT_DCT luma block
//! (`ENC-COEFF-GENERAL-WALK-LF-BASE` and its extensions) and the 16x16 DCT_DCT luma
//! block base pass (`ENC-COEFF-TOKENIZE-16X16-BASE`), reading the AV2 § 8.3.2
//! contexts parameterized by `bwl`/`txw`/`txh` rather than 4x4 literals.
//!
//! The LF/HF predicate (the decoder `get_lf_limits` for `TX_CLASS_2D` luma,
//! `row + col < 4`) is SIZE-INDEPENDENT (see
//! `crates/splot-decode/src/tile_payload/coeff_loop/max_level.rs`); only the
//! coefficient count, the `eob_pt` size class / `coeff_base_eob` band breaks
//! (`numCoeffs / 8` & `numCoeffs / 4`), the scan order, and the `txSzCtx` differ
//! between sizes. The `bwl`/`txw`/`txh`/`max_scan_index`/`num_coeffs` of this struct
//! feed the size-generic § 8.3.2 mirror functions exactly as the decoder does.

/// Which `eob_pt_*` size-class CDF the EOB-point symbol of a transform block reads
/// (AV2 § 5.20.7.27). `eob_pt_16` is the 16-position class (4x4); `eob_pt_256` is the
/// 256-position class (16x16, `eobMultisize == 4`). The level/`eob_extra`
/// refinement arithmetic is otherwise identical (it keys on `eobPt`, not the size
/// class), so only the CDF bank the symbol reads differs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EobPtKind {
    /// `eob_pt_16` — the `TX_4X4` EOB-point size class.
    Pt16,
    /// `eob_pt_256` — the `TX_16X16` EOB-point size class.
    Pt256,
}

/// The AV2 § 8.3.2 transform geometry the general walk threads through its
/// size-generic base/sign passes and § 8.3.2 context derivations. A descriptor, not
/// a table: every field is a caller-resolved scalar the decoder also resolves from
/// the transform size (`Tx_Width`/`Tx_Height`/`Tx_Width_Log2`/`txSzCtx`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TxGeom {
    /// `Tx_Width[txSz]` — the transform width (column bound + raster row stride).
    pub(super) width: usize,
    /// `Tx_Height[txSz]` — the transform height (row bound).
    pub(super) height: usize,
    /// `Tx_Width_Log2[txSz]` — the `row`/`col` split of a raster position
    /// (`row = pos >> bwl`, `col = pos - (row << bwl)`).
    pub(super) bwl: u32,
    /// `width * height` — the coefficient count (`Quant[coeff_count]`).
    pub(super) coeff_count: usize,
    /// The largest in-window nonzero scan index (`coeff_count - 1`, i.e. eob
    /// `<= coeff_count`). A nonzero past it is rejected.
    pub(super) max_scan_index: usize,
    /// `numCoeffs = height << bwl` used by the § 8.3.2 `coeff_base_eob_ctx` band
    /// breaks (`numCoeffs / 8`, `numCoeffs / 4`). Equals `coeff_count` for a square
    /// transform, but the decoder spells it `Tx_Height[txSz] << Tx_Width_Log2[txSz]`,
    /// so it is carried explicitly to mirror the decoder verbatim.
    pub(super) num_coeffs: usize,
    /// The § 8.3.2 `txSzCtx` for the `txb_skip` / `coeff_base` / `coeff_base_eob`
    /// CDF selectors of this transform size (`TX_SIZE_4X4_CTX == 0`,
    /// `TX_SIZE_16X16_CTX == 2`).
    pub(super) tx_size_ctx: usize,
    /// Which `eob_pt_*` size-class CDF the EOB-point symbol reads.
    pub(super) eob_pt_kind: EobPtKind,
}

impl TxGeom {
    /// The 4x4 DCT_DCT luma geometry (`bwl = 2`, `numCoeffs = 16`, the `eob_pt_16`
    /// size class, the `TX_SIZE_4X4_CTX` selector). The general 4x4 entry delegates
    /// with this descriptor so its emitted stream stays byte-identical.
    pub(super) const TX_4X4: Self = Self {
        width: 4,
        height: 4,
        bwl: 2,
        coeff_count: 16,
        max_scan_index: 15,
        num_coeffs: 4 << 2,
        tx_size_ctx: super::TX_SIZE_4X4_CTX,
        eob_pt_kind: EobPtKind::Pt16,
    };

    /// The 16x16 DCT_DCT luma geometry (`bwl = 4`, `numCoeffs = 256`, the
    /// `eob_pt_256` size class, the `TX_SIZE_16X16_CTX` selector). The 16x16 base
    /// pass entry delegates with this descriptor.
    pub(super) const TX_16X16: Self = Self {
        width: 16,
        height: 16,
        bwl: 4,
        coeff_count: 256,
        max_scan_index: 255,
        num_coeffs: 16 << 4,
        tx_size_ctx: super::TX_SIZE_16X16_CTX,
        eob_pt_kind: EobPtKind::Pt256,
    };
}
