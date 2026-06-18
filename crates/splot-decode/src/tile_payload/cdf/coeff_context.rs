// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 8.3.2 coefficient-symbol CDF context derivation.
//!
//! This module derives the per-symbol `ctx` index that selects a coefficient
//! CDF row in the § 8.3.2 Cdf selection process
//! (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`). It is the coefficient
//! counterpart of [`super::block_context`] (which derives the block-mode
//! contexts).
//!
//! Feature tracking: `DECODE-TILE-CDF-SELECTION-BOUNDARY`.
//!
//! Scope:
//!
//! - the two **position-only** coefficient base contexts — `coeff_base_eob` (the
//!   end-of-block base context, keyed on the scan position relative to the
//!   transform block size) and `coeff_base_bob` (the begin-of-block base context,
//!   keyed on the begin position relative to the segment end-of-block) — pure
//!   functions of caller-supplied scan/segment scalars and caller-resolved
//!   geometry, needing no `Level[]` magnitude buffer; and
//! - the `coeff_br` coefficient base-range context ([`CoeffBrContext`]), the first
//!   context that reads the per-transform-block `Level[]` magnitudes, over a
//!   caller-provided level slice.
//!
//! The remaining `Level[]`-dependent coefficient contexts (`coeff_base`, the IDTX
//! variants) and the sign contexts (`dc_sign`, `idtx_sign`) are derived by future
//! increments once the full per-transform-block level/sign buffers and the § 5.20
//! coefficient decode loop exist. Nothing here is wired into a decode path yet, so
//! it is no-output-change (the derivations are exercised by compile-time
//! spec-contract `const` checks and unit tests, not by any decode stage).

use splot_recon::TransformClass;

/// AV2 § 3 `SIG_COEF_CONTEXTS_EOB`: the number of `coeff_base_eob` contexts
/// (`03-symbols.md`); the four contexts are `SIG_COEF_CONTEXTS_EOB - 4 ..=
/// SIG_COEF_CONTEXTS_EOB - 1`, i.e. `0..=3`.
const SIG_COEF_CONTEXTS_EOB: usize = 4;

/// AV2 § 3 `MAX_BASE_BR_RANGE` = `COEFF_BASE_RANGE (3) + NUM_BASE_LEVELS (2) + 1`
/// (`03-symbols.md`); the `coeff_br` magnitude sum clamps each neighbour level to
/// `MAX_BASE_BR_RANGE - 1`.
const MAX_BASE_BR_RANGE: u32 = 6;

/// AV2 § 8.3.2 `Mag_Ref_Offset_With_Tx_Class[txClass][idx][rowOrCol]`
/// (`08-parsing-process.md#s-8-3-2`): the up-to-three neighbour `(dRow, dCol)`
/// offsets the `coeff_br` magnitude sum reads, per transform class. Indexed by the
/// spec `txClass` value (`TX_CLASS_2D` = 0, `TX_CLASS_HORIZ` = 1,
/// `TX_CLASS_VERT` = 2).
const MAG_REF_OFFSET_WITH_TX_CLASS: [[[usize; 2]; 3]; 3] = [
    [[0, 1], [1, 0], [1, 1]], // TX_CLASS_2D
    [[0, 1], [1, 0], [0, 2]], // TX_CLASS_HORIZ
    [[0, 1], [1, 0], [2, 0]], // TX_CLASS_VERT
];

/// Returns the AV2 § 8.3.2 `coeff_base_eob` CDF context for the scan position
/// `c` in a transform block of caller-resolved adjusted geometry
/// (`08-parsing-process.md#s-8-3-2`).
///
/// The context partitions the scan position by the adjusted transform block's
/// coefficient count `numCoeffs = height << bwl` (the spec's
/// `Tx_Height[adjTxSz] << Tx_Width_Log2[adjTxSz]`):
///
/// - `c == 0` → `SIG_COEF_CONTEXTS_EOB - 4` (`0`)
/// - `c <= numCoeffs / 8` → `SIG_COEF_CONTEXTS_EOB - 3` (`1`)
/// - `c <= numCoeffs / 4` → `SIG_COEF_CONTEXTS_EOB - 2` (`2`)
/// - otherwise → `SIG_COEF_CONTEXTS_EOB - 1` (`3`)
///
/// `bwl` is `Tx_Width_Log2[adjTxSz]` and `height` is `Tx_Height[adjTxSz]`, both
/// caller-resolved from the adjusted transform size (this module does not model
/// the § 9.2 conversion tables). The result is in `0..=3`. The `numCoeffs`
/// shift is computed total: an out-of-range `bwl` saturates to `usize::MAX`
/// rather than overflowing, so the function never panics.
pub(crate) const fn coeff_base_eob_ctx(c: usize, bwl: u32, height: usize) -> usize {
    // numCoeffs = height << bwl, computed without a panic on a bad shift width.
    let num_coeffs = match height.checked_shl(bwl) {
        Some(v) => v,
        None => usize::MAX,
    };
    if c == 0 {
        SIG_COEF_CONTEXTS_EOB - 4
    } else if c <= num_coeffs / 8 {
        SIG_COEF_CONTEXTS_EOB - 3
    } else if c <= num_coeffs / 4 {
        SIG_COEF_CONTEXTS_EOB - 2
    } else {
        SIG_COEF_CONTEXTS_EOB - 1
    }
}

/// Returns the AV2 § 8.3.2 `coeff_base_bob` CDF context for the begin-of-block
/// position `bob` relative to the segment end-of-block `seg_eob`
/// (`08-parsing-process.md#s-8-3-2`).
///
/// - `bob <= seg_eob >> 3` → `0`
/// - `bob <= seg_eob >> 2` → `1`
/// - otherwise → `2`
///
/// The result is in `0..=2`. Pure function of the two caller-supplied scalars;
/// total and panic-free.
pub(crate) const fn coeff_base_bob_ctx(bob: usize, seg_eob: usize) -> usize {
    if bob <= seg_eob >> 3 {
        0
    } else if bob <= seg_eob >> 2 {
        1
    } else {
        2
    }
}

/// AV2 § 8.3.2 `coeff_br` coefficient base-range CDF context derivation.
///
/// `coeff_br` selects `TileCoeffBrUvCdf[ctx]` (chroma), `TileCoeffBrLfCdf[ctx]`
/// (low-frequency luma), or `TileCoeffBrCdf[ctx]` (luma) from the `ctx` derived
/// here (`08-parsing-process.md#s-8-3-2`). The context sums up to three
/// neighbouring `Level[]` magnitudes (each clamped to `MAX_BASE_BR_RANGE - 1`) at
/// the transform-class-specific offsets, halves and clamps the sum to `0..=6`,
/// then offsets it by plane / DC-position / low-frequency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffBrContext {
    /// The coefficient scan position `pos` within the adjusted transform block.
    pub(crate) pos: usize,
    /// `Tx_Width_Log2[adjTxSz]` — the adjusted block width log2 (`row`/`col`
    /// split of `pos`).
    pub(crate) bwl: u32,
    /// `Tx_Width[adjTxSz]` — the adjusted block width (column bound + row stride).
    pub(crate) txw: usize,
    /// `Tx_Height[adjTxSz]` — the adjusted block height (row bound).
    pub(crate) txh: usize,
    /// The plane index (`0` luma, `> 0` chroma).
    pub(crate) plane: usize,
    /// Whether this transform block is low-frequency (`isLf`).
    pub(crate) is_lf: bool,
    /// The transform class (`get_tx_class(PlaneTxType)`).
    pub(crate) tx_class: TransformClass,
}

impl CoeffBrContext {
    /// The spec `txClass` index into [`MAG_REF_OFFSET_WITH_TX_CLASS`]
    /// (`TX_CLASS_2D` = 0, `TX_CLASS_HORIZ` = 1, `TX_CLASS_VERT` = 2).
    const fn tx_class_index(self) -> usize {
        match self.tx_class {
            TransformClass::TwoD => 0,
            TransformClass::Horizontal => 1,
            TransformClass::Vertical => 2,
        }
    }

    /// Returns the AV2 § 8.3.2 `coeff_br` CDF context, reading the
    /// per-transform-block `Level[]` magnitudes from `level` (row-major,
    /// `txw`-wide; `level[row * txw + col]`).
    ///
    /// Neighbour reads outside the block bounds (`refRow >= txh` or
    /// `refCol >= txw`) or past the `level` slice contribute `0`, exactly matching
    /// the spec's `refRow < txh && refCol < txw` guard, so the function is total
    /// and never panics. The result is the spec `ctx`: `0..=13` (luma /
    /// low-frequency) or `0..=3` (chroma).
    pub(crate) const fn ctx(self, level: &[u32]) -> usize {
        let row = self.pos >> self.bwl;
        let col = self.pos - (row << self.bwl);
        let class_idx = self.tx_class_index();
        // num = 3, or 2 for non-2D chroma (§ 8.3.2).
        let num = if class_idx != 0 && self.plane > 0 {
            2
        } else {
            3
        };
        let clamp = MAX_BASE_BR_RANGE - 1;
        let mut mag: u32 = 0;
        let mut idx = 0;
        // `while` (not `for`): iterators are not permitted in a `const fn`.
        while idx < num {
            let ref_row = row + MAG_REF_OFFSET_WITH_TX_CLASS[class_idx][idx][0];
            let ref_col = col + MAG_REF_OFFSET_WITH_TX_CLASS[class_idx][idx][1];
            if ref_row < self.txh && ref_col < self.txw {
                let flat = ref_row * self.txw + ref_col;
                if flat < level.len() {
                    let lvl = level[flat];
                    mag += if lvl < clamp { lvl } else { clamp };
                }
            }
            idx += 1;
        }
        // mag = Min((mag + 1) >> 1, 6)
        let halved = (mag + 1) >> 1;
        let mag = (if halved < 6 { halved } else { 6 }) as usize;
        if self.plane > 0 {
            if mag < 3 { mag } else { 3 }
        } else if self.pos == 0 {
            if class_idx != 0 { mag + 7 } else { mag }
        } else if self.is_lf {
            mag + 7
        } else {
            mag
        }
    }
}

// Compile-time spec-contract checks. These `const` items are the non-test
// consumer of the context derivations until the §5.20.7.27 `coeffs()` decode
// loop wires them: they pin the §8.3.2 contract at the four/three boundaries
// (TX_32X32 geometry: numCoeffs = 32 << 5 = 1024, so thresholds 128 and 256;
// segEob = 64, so thresholds 8 and 16) so any drift fails the build.
const _COEFF_BASE_EOB_CONTRACT: () = {
    assert!(coeff_base_eob_ctx(0, 5, 32) == 0);
    assert!(coeff_base_eob_ctx(128, 5, 32) == 1);
    assert!(coeff_base_eob_ctx(256, 5, 32) == 2);
    assert!(coeff_base_eob_ctx(257, 5, 32) == 3);
};
const _COEFF_BASE_BOB_CONTRACT: () = {
    assert!(coeff_base_bob_ctx(0, 64) == 0);
    assert!(coeff_base_bob_ctx(16, 64) == 1);
    assert!(coeff_base_bob_ctx(17, 64) == 2);
};
const _COEFF_BR_CONTRACT: () = {
    // TX_4X4 (bwl 2, txw/txh 4). All-zero neighbours, DC luma 2D -> ctx 0.
    let zero = [0u32; 16];
    let dc_2d = CoeffBrContext {
        pos: 0,
        bwl: 2,
        txw: 4,
        txh: 4,
        plane: 0,
        is_lf: false,
        tx_class: TransformClass::TwoD,
    };
    assert!(dc_2d.ctx(&zero) == 0);
    // Three saturating neighbours at (0,1),(1,0),(1,1): mag = 4+4+4 = 12 ->
    // (12 + 1) >> 1 = 6 -> DC luma 2D -> ctx 6.
    let mags = [0u32, 4, 0, 0, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert!(dc_2d.ctx(&mags) == 6);
    // DC luma non-2D (VERT): mag 0, pos == 0, txClass != 2D -> mag + 7 = 7.
    let dc_vert = CoeffBrContext {
        pos: 0,
        bwl: 2,
        txw: 4,
        txh: 4,
        plane: 0,
        is_lf: false,
        tx_class: TransformClass::Vertical,
    };
    assert!(dc_vert.ctx(&zero) == 7);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coeff_base_eob_partitions_the_scan_position() {
        // TX_32X32 adjusted: bwl = Tx_Width_Log2 = 5, height = 32, so
        // numCoeffs = 32 << 5 = 1024; thresholds 1024/8 = 128 and 1024/4 = 256.
        let (bwl, height) = (5u32, 32usize);
        assert_eq!(coeff_base_eob_ctx(0, bwl, height), 0, "c == 0");
        // 1 ..= 128 -> ctx 1.
        assert_eq!(coeff_base_eob_ctx(1, bwl, height), 1);
        assert_eq!(
            coeff_base_eob_ctx(128, bwl, height),
            1,
            "boundary numCoeffs/8"
        );
        // 129 ..= 256 -> ctx 2.
        assert_eq!(coeff_base_eob_ctx(129, bwl, height), 2);
        assert_eq!(
            coeff_base_eob_ctx(256, bwl, height),
            2,
            "boundary numCoeffs/4"
        );
        // > 256 -> ctx 3.
        assert_eq!(coeff_base_eob_ctx(257, bwl, height), 3);
        assert_eq!(coeff_base_eob_ctx(1023, bwl, height), 3, "last position");
    }

    #[test]
    fn coeff_base_eob_smallest_block() {
        // TX_4X4 adjusted: bwl = 2, height = 4, numCoeffs = 4 << 2 = 16;
        // thresholds 16/8 = 2 and 16/4 = 4.
        let (bwl, height) = (2u32, 4usize);
        assert_eq!(coeff_base_eob_ctx(0, bwl, height), 0);
        assert_eq!(coeff_base_eob_ctx(1, bwl, height), 1);
        assert_eq!(coeff_base_eob_ctx(2, bwl, height), 1, "boundary 16/8");
        assert_eq!(coeff_base_eob_ctx(3, bwl, height), 2);
        assert_eq!(coeff_base_eob_ctx(4, bwl, height), 2, "boundary 16/4");
        assert_eq!(coeff_base_eob_ctx(5, bwl, height), 3);
    }

    #[test]
    fn coeff_base_eob_is_total_for_out_of_range_shift() {
        // A pathological bwl must not panic; numCoeffs saturates so any non-zero
        // c lands in ctx 1 (c <= usize::MAX / 8).
        assert_eq!(coeff_base_eob_ctx(0, u32::MAX, 32), 0);
        assert_eq!(coeff_base_eob_ctx(1, u32::MAX, 32), 1);
    }

    #[test]
    fn coeff_base_bob_partitions_the_begin_position() {
        // segEob = 64: thresholds 64 >> 3 = 8 and 64 >> 2 = 16.
        let seg_eob = 64usize;
        assert_eq!(coeff_base_bob_ctx(0, seg_eob), 0);
        assert_eq!(coeff_base_bob_ctx(8, seg_eob), 0, "boundary segEob>>3");
        assert_eq!(coeff_base_bob_ctx(9, seg_eob), 1);
        assert_eq!(coeff_base_bob_ctx(16, seg_eob), 1, "boundary segEob>>2");
        assert_eq!(coeff_base_bob_ctx(17, seg_eob), 2);
        assert_eq!(coeff_base_bob_ctx(64, seg_eob), 2, "bob == segEob");
    }

    #[test]
    fn coeff_base_bob_zero_segment_eob() {
        // segEob = 0: both thresholds are 0, so only bob == 0 takes ctx 0.
        assert_eq!(coeff_base_bob_ctx(0, 0), 0);
        assert_eq!(coeff_base_bob_ctx(1, 0), 2);
    }

    /// A `coeff_br` context over TX_4X4 adjusted geometry (bwl 2, txw/txh 4).
    fn br(pos: usize, plane: usize, is_lf: bool, tx_class: TransformClass) -> CoeffBrContext {
        CoeffBrContext {
            pos,
            bwl: 2,
            txw: 4,
            txh: 4,
            plane,
            is_lf,
            tx_class,
        }
    }

    #[test]
    fn coeff_br_dc_luma_2d_sums_clamped_neighbours() {
        // pos 0 -> (row,col) = (0,0); 2D offsets (0,1),(1,0),(1,1) = flat 1,4,5.
        // levels 7,2,10 -> clamped to 5,2,5 (MAX_BASE_BR_RANGE-1) -> mag 12 ->
        // (12 + 1) >> 1 = 6; luma DC 2D -> ctx = mag = 6.
        let mut level = [0u32; 16];
        level[1] = 7;
        level[4] = 2;
        level[5] = 10;
        assert_eq!(br(0, 0, false, TransformClass::TwoD).ctx(&level), 6);
    }

    #[test]
    fn coeff_br_clamps_halved_magnitude_to_six() {
        // pos 5 -> (row,col) = (1,1); 2D offsets -> flat 6,9,10, all level 5 ->
        // mag 15 -> (15 + 1) >> 1 = 8 -> clamped to 6; luma non-DC -> ctx 6.
        let mut level = [0u32; 16];
        level[6] = 5;
        level[9] = 5;
        level[10] = 5;
        assert_eq!(br(5, 0, false, TransformClass::TwoD).ctx(&level), 6);
    }

    #[test]
    fn coeff_br_dc_non_2d_and_low_frequency_add_seven() {
        let zero = [0u32; 16];
        // DC (pos 0) luma, non-2D (VERT): mag 0 + 7 -> ctx 7.
        assert_eq!(br(0, 0, false, TransformClass::Vertical).ctx(&zero), 7);
        // Non-DC (pos 5) luma, low-frequency: mag 0 + 7 -> ctx 7.
        assert_eq!(br(5, 0, true, TransformClass::TwoD).ctx(&zero), 7);
        // Non-DC (pos 5) luma, non-LF: mag 0 -> ctx 0.
        assert_eq!(br(5, 0, false, TransformClass::TwoD).ctx(&zero), 0);
    }

    #[test]
    fn coeff_br_chroma_clamps_to_three() {
        // Chroma 2D (num 3): flat 1,4,5 all 5 -> mag 15 -> 8 -> 6; chroma
        // Min(6, 3) -> ctx 3.
        let mut level = [0u32; 16];
        level[1] = 5;
        level[4] = 5;
        level[5] = 5;
        assert_eq!(br(0, 1, false, TransformClass::TwoD).ctx(&level), 3);
    }

    #[test]
    fn coeff_br_non_2d_chroma_reads_only_two_neighbours() {
        // Chroma VERT at pos 5 -> num 2: reads offsets (0,1),(1,0) = flat 6,9 but
        // NOT the third VERT offset (2,0) = flat 13. levels 1,1 at 6,9 and 4 at 13
        // -> mag 2 -> (3 >> 1) = 1 -> chroma Min(1,3) = 1. (num 3 would read flat
        // 13 too -> mag 6 -> 3 -> Min(3,3) = 3, so ctx 1 proves only two are read.)
        let mut level = [0u32; 16];
        level[6] = 1;
        level[9] = 1;
        level[13] = 4;
        assert_eq!(br(5, 1, false, TransformClass::Vertical).ctx(&level), 1);
    }

    #[test]
    fn coeff_br_is_total_for_out_of_bounds_and_short_slices() {
        // pos 15 -> (row,col) = (3,3); every 2D offset leaves the 4x4 block, so
        // no neighbour is read -> mag 0 -> luma non-DC ctx 0 (no panic).
        let full = [9u32; 16];
        assert_eq!(br(15, 0, false, TransformClass::TwoD).ctx(&full), 0);
        // A short slice: only flat 1 is in range (flat 4,5 are past len 4) -> mag
        // = Min(5, 5) = 5 -> (6 >> 1) = 3 -> ctx 3 (no panic).
        let short = [0u32, 9, 0, 0];
        assert_eq!(br(0, 0, false, TransformClass::TwoD).ctx(&short), 3);
    }
}
