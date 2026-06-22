// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.7.27 GENERAL coefficient-tokenization walk for a 16x16 DCT_DCT luma
//! block in the BASE PASS (`ENC-COEFF-TOKENIZE-16X16-BASE`).
//!
//! This is the size-generic [`super::general_walk::tokenize_general_luma_block`]
//! specialized to a `Quant[256]` block with [`TxGeom::TX_16X16`] and the base-pass eob
//! window (eob `1..=32`, eobPt `<= 6`, the `eob_pt_256` symbol `<= 5`). It reuses ONE
//! codepath with the 4x4 walk: only the [`TxGeom`] descriptor differs (coeff_count
//! 256, `bwl = 4`, the § 8.3.2 `coeff_base_eob_ctx` band breaks at `numCoeffs / 8 = 32`
//! & `numCoeffs / 4 = 64`, the 16x16 2D scan order, the `eob_pt_256` size class, and
//! the `TX_SIZE_16X16_CTX` `txSzCtx`). The LF/HF boundary `row + col < 4` is
//! SIZE-INDEPENDENT.
//!
//! BASE PASS ONLY: the brick scope is eob `1..=32` (eobPt `1..=6`, the `eob_pt_256`
//! symbol `0..=5`). An eob `> 32` is rejected here with a typed error as the chosen
//! base-pass boundary — the higher eob (eobPt `>= 7`, and ultimately the `eob_pt_256`
//! symbol-7 `eob_pt_extra` refinement for eob `>= 65`) is a later brick. Within the
//! base pass the `eob_extra` CDF flag + `eob_extra_bit` literals layer is identical to
//! the 4x4 walk (it keys on `eobPt`, not the size class); eobPt 6 carries
//! `eobPt - 3 = 3` `eob_extra_bit` literals.
//!
//! HONESTY: the [`super::general_walk_recover::recover_quant_from_tokens_geom`] proof
//! is § 8.2 SELF-CONSISTENCY — the same code authored the emission and its inverse, so
//! it proves the emitted (level, sign, position) triples are internally reversible and
//! that every reached § 8.3.2 context routes to a real generated default row. It does
//! NOT validate the § 8.3.2 CDF contexts against a real decoder; context conformance is
//! deferred to the splot-decode cross-check brick.

use super::general_walk::{TxGeom, tokenize_general_luma_block};
use crate::block_symbol_trace::BlockSymbolToken;
use crate::error::Result;

/// 16x16 DCT_DCT coefficient count (`Quant[256]`).
const TX_16X16_COEFF_COUNT: usize = 256;
const _ASSERT_COEFF_COUNT: () = assert!(TX_16X16_COEFF_COUNT == TxGeom::TX_16X16.coeff_count);

/// The largest eobPt the 16x16 base pass admits (eob `1..=32` → eobPt `1..=6`). The
/// brick scope is eob `1..=32`: eob 17..=32 → eobPt 6 (base 17). The `eob_pt_256`
/// symbol carries `eobPt - 1`, so eobPt 6 → symbol 5, which is `< 7` and does NOT
/// trigger the symbol-7 `eob_pt_extra` refinement. So the whole eob `1..=32` base
/// window is reachable with the plain `eob_pt_256` symbol + the existing `eob_extra` /
/// `eob_extra_bit` layer (eobPt 6 carries `eobPt - 3 = 3` `eob_extra_bit` literals).
/// eob `>= 33` (eobPt `>= 7`) is a later brick and is rejected with a typed error.
const MAX_16X16_BASE_EOB_PT: usize = 6;

/// Tokenizes an arbitrary 16x16 DCT_DCT luma `Quant[256]` block in the base pass (eob
/// `1..=32`) into the ordered AV2 § 5.20.7.27 block-symbol trace: the luma `txb_skip`
/// at the `TX_16X16` `txSzCtx`, the `eob_pt_256` size-class EOB symbol (plus the
/// `eob_extra` / `eob_extra_bit` refinement for eobPt `>= 3`), the reverse-scan
/// `coeff_base_eob` / `coeff_base` base pass with 16x16 LF/HF § 8.3.2 contexts, an
/// interleaved `coeff_br` for any base-range magnitude, the reverse-scan sign pass
/// (`dc_sign` for the DC, `sign_bit` bypass for the AC), the § 5.20.7.28 golomb tail
/// for any golomb-range coefficient (golomb is size-independent), and the all-zero
/// chroma U/V `txb_skip` tail.
///
/// `quant` is the row-major (raster) signed quantized 16x16 block; `coeff_cdf_q_ctx` is
/// the caller-resolved coefficient-CDF q-context. An all-zero block emits exactly one
/// luma `all_zero == 1` token (no chroma tail). Errors:
///
/// - [`crate::error::Error::CoefficientTokenizationUnsupportedEob`] when eob `> 32`
///   (eobPt `> 6`, the `eob_pt_256` symbol-7 `eob_pt_extra` refinement — the next
///   brick), and
/// - [`crate::error::Error::CoefficientTokenizationUnsupportedMagnitude`] when a
///   golomb-range coefficient's § 5.20.7.28 extension exceeds the per-`m` golomb cap.
///
/// # Preconditions
/// Assumes **TCQ is off** (`allow_tcq == 0`), as the minimal/general intra encoder path
/// is. Do not reuse on a TCQ-enabled block (the `read_quant` threshold drops by 1).
pub(crate) fn tokenize_general_16x16_luma_block(
    quant: &[i32; TX_16X16_COEFF_COUNT],
    coeff_cdf_q_ctx: usize,
) -> Result<Vec<BlockSymbolToken>> {
    tokenize_general_luma_block(
        quant,
        TxGeom::TX_16X16,
        MAX_16X16_BASE_EOB_PT,
        coeff_cdf_q_ctx,
    )
}

#[cfg(test)]
#[path = "general_walk_16x16_tests.rs"]
mod tests;
