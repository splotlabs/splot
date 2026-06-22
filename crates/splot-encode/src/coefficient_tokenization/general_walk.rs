// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.7.27 GENERAL coefficient-tokenization walk: the size-generic
//! reverse-scan base/sign codepath. It tokenizes an arbitrary quantized DCT_DCT luma
//! block whose nonzero coefficients sit anywhere in the scan — the low-frequency
//! region plus the entire high-frequency tail — and emits the ordered § 5.20.7.27
//! coefficient token stream the decoder coefficient loop reads: the luma `txb_skip`,
//! the size-class `eob_pt_*` symbol, an optional `eob_extra` CDF flag and
//! `eob_extra_bit` bypass literals (read only when eobPt `>= 3`), the reverse-scan
//! `coeff_base_eob` / `coeff_base` base pass (with the running-`Level[]` § 8.3.2 LF
//! luma context from [`super::coeff_base_lf_luma_context`] for low-frequency
//! coefficients and the HF luma context from [`super::coeff_base_hf_luma_context`] for
//! high-frequency ones), the reverse-scan interleaved sign pass (`dc_sign` CDF for the
//! DC, `sign_bit` § 8.2.5 bypass for the AC), and the all-zero chroma U/V `txb_skip`
//! tail. It reuses the existing token constructors and CDF routing; it never invents
//! AV2 CDF values or contexts.
//!
//! SIZE-GENERIC ([`TxGeom`]): the SAME codepath tokenizes the 4x4 block
//! (`ENC-COEFF-GENERAL-WALK-LF-BASE` + HF/golomb extensions, via
//! [`tokenize_general_lf_luma_block`]) and the 16x16 base pass
//! (`ENC-COEFF-TOKENIZE-16X16-BASE`, via
//! [`super::general_walk_16x16::tokenize_general_16x16_luma_block`]). The decoder's
//! § 8.3.2 context functions are ALREADY size-generic (parameterized by
//! `bwl`/`txw`/`txh`/`height`), and so are the encoder's mirror functions
//! ([`super::coeff_base_lf_luma_context`] / [`super::coeff_base_hf_luma_context`] /
//! [`super::coeff_br_lf_luma_context`]). This walk threads the [`TxGeom`] descriptor
//! through them instead of the 4x4 literals; the LF/HF boundary `row + col < 4` is
//! SIZE-INDEPENDENT (the decoder `get_lf_limits` for `TX_CLASS_2D` luma). The 16x16
//! deltas are: coeff_count 16 → 256; the § 8.3.2 `coeff_base_eob_ctx` band breaks
//! `numCoeffs / 8` & `numCoeffs / 4` (2,4 → 32,64); `coefficient_scan_order(16,16,2D)`
//! instead of (4,4); the `eob_pt_256` size class instead of `eob_pt_16`; and the
//! `TX_SIZE_16X16_CTX` `txSzCtx` instead of `TX_SIZE_4X4_CTX` in the token selectors.
//!
//! LF REGION BOUNDARY: for luma `TX_CLASS_2D` the decoder
//! `get_lf_limits(row, col, txClass, plane)`
//! (`crates/splot-decode/src/tile_payload/coeff_loop/max_level.rs`) marks a
//! coefficient low-frequency iff `row + col < 4` — NOT by scan index, and NOT by
//! transform size. Each coefficient's LF/HF predicate is derived from its OWN raster
//! `row + col < 4` (see [`is_lf_position`]).
//!
//! HF EOB / NON-EOB COEFFICIENTS: the EOB and non-EOB high-frequency coefficients use
//! DIFFERENT § 8.3.2 CDF tables than the low-frequency ones — verified against the
//! decoder and the generated default tables:
//!
//! - `coeff_base_eob` reads the 4-symbol HF `DEFAULT_COEFF_BASE_EOB_CDF`
//!   (`[q][tx_size][ctx][row]`), NOT the 6-symbol LF `DEFAULT_COEFF_BASE_LF_EOB_CDF`.
//!   The `coeff_base_eob` *context* is shared (scan-band based,
//!   [`coeff_base_eob_ctx`]). The HF EOB token level mapping uses the HF base-level cap
//!   (`eob_level = min(mag, NUM_BASE_LEVELS + 1) == min(mag, 3)`, NOT the LF
//!   `LF_NUM_BASE_LEVELS + 1 == 5`).
//! - When the HF EOB coeff magnitude exceeds `NUM_BASE_LEVELS`, its `coeff_br` reads
//!   the HF `DEFAULT_COEFF_BR_CDF` (`[q][ctx][row]`, NO transform-size dimension), NOT
//!   the LF `DEFAULT_COEFF_BR_LF_CDF`. For the EOB coefficient (visited first in
//!   reverse scan, empty `Level[]`) the neighbour sum is `0` → HF `coeff_br` ctx `== 0`
//!   ([`HF_COEFF_BR_CTX_EOB`]).
//! - A non-EOB high-frequency coefficient uses the 4-symbol HF `DEFAULT_COEFF_BASE_CDF`
//!   (`[q][tx_size][ctx][tcq][row]`) at the § 8.3.2 HF luma context
//!   ([`super::coeff_base_hf_luma_context`]) — `magLimit = 3` for EVERY neighbour, no
//!   DC band; `ctx2 = min((mag+1)>>1, 4)`, 2D band `row+col < 6 -> ctx2`, `< 8 ->
//!   ctx2 + 5`, else `ctx2 + 10`. The HF base saturates at `NUM_BASE_LEVELS + 1 == 3`,
//!   and the HF `coeff_br` (the no-`+7` branch) refines up to magnitude
//!   `MAX_HF_BASE_BR_MAGNITUDE == 5`.
//!
//! EOB SIGNALING (mirrors the decoder `nonzero_coeff_eob` arithmetic and the
//! `read_nonzero_coeff_eob` read sequence in
//! `crates/splot-decode/src/tile_payload/coeff_loop.rs`, and the § 5.20.7.27 eob
//! refinement loop at `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`): the
//! size-class `eob_pt_*` symbol carries `eobPt - 1`. eob 1 → eobPt 1, eob 2 → eobPt 2
//! (both eobPt `< 3`, NO refinement). The eob→eobPt mapping (decoder base =
//! `(1 << (eobPt-2)) + 1`): eob 3..=4 → eobPt 3 (base 3), eob 5..=8 → eobPt 4 (base 5),
//! eob 9..=16 → eobPt 5 (base 9), eob 17..=32 → eobPt 6 (base 17). For eobPt `>= 3` the
//! decoder reads `eob_extra` (a CDF flag = the HIGH refinement bit) then `eobPt - 3`
//! `eob_extra_bit` literals (the LOW refinement bits) and computes
//! `eob = base + (eob_extra << (eobPt - 3)) + eob_extra_bits`. From the input eob this
//! brick derives `offset = eob - base`, `eob_extra = (offset >> (eobPt - 3)) & 1`, and
//! `eob_extra_bits = offset & ((1 << (eobPt - 3)) - 1)`. NOTE: for the `eob_pt_256` /
//! `eob_pt_1024` size classes a `eob_pt_*` symbol of 7 instead carries an
//! `eob_pt_extra` refinement to eobPt 7..=8 — that refinement (eobPt `> 6`) is OUT OF
//! SCOPE for the 16x16 base pass (eob `<= 32`, eobPt `<= 5`, symbol `<= 4`).
//!
//! `eob_extra_bit` BIT ORDER (load-bearing, mirrored from decoder/spec — the § 8.2
//! roundtrip CANNOT catch a bit-order error): the spec loop emits the bit for
//! `i = eobPt - 4` (the MSB of `eob_extra_bits`) FIRST, down to `i = 0` (the LSB) LAST.
//! The decoder reads them as one `read_literal(width)`, which is MSB-first. So this
//! tokenizer emits the `eob_extra_bit` bypass literals MSB-first;
//! [`recover_quant_from_tokens`] reads them back in the SAME MSB-first order.
//!
//! Every coefficient may carry a base-range magnitude (a `coeff_br` token right after
//! its `coeff_base_eob` / `coeff_base`). A magnitude at-or-above its position
//! `maxLevel` (LF `8`, HF `6`) is a § 5.20.7.28 `read_quant` GOLOMB coefficient: its
//! base+`coeff_br` level saturates at `maxLevel` and the extension `x = magnitude -
//! maxLevel` is coded in the golomb tail (sign pass). MULTIPLE golomb coefficients per
//! block are supported — the running `hrLevelAvg` predictor is threaded across them in
//! reverse scan.
//!
//! HONESTY: the [`recover_quant_from_tokens`] proof is § 8.2 SELF-CONSISTENCY. The
//! same code authored the emission and its inverse, so it proves the encoder's emitted
//! (level, sign, position) triples are internally reversible — with asymmetric values
//! it catches a swapped sign order (AC-before-DC) or a level/position transposition. It
//! does NOT validate the § 8.3.2 CDF contexts against a real decoder; context
//! conformance is deferred to the splot-decode cross-check brick.

use splot_recon::{PlaneId, PlaneRect, TransformClass, coefficient_scan_order};

pub(super) use super::general_walk_geom::TxGeom;
use super::general_walk_golomb::{
    golomb_params_from_hr_level_avg, golomb_x_max, next_hr_level_avg, push_read_quant_golomb_tail,
    read_quant_golomb_tail_len,
};
use super::{
    COEFF_BASE_RANGE, CoefficientEntropyToken, EOB_CTX_LUMA_INTRA, LF_NUM_BASE_LEVELS,
    MAX_BASE_BR_MAGNITUDE, NUM_BASE_LEVELS, chroma_u_all_zero_token, chroma_v_all_zero_token,
    coded_luma_all_zero_token, coeff_base_hf_eob_token_sized, coeff_base_hf_luma_context,
    coeff_base_hf_token_sized, coeff_base_lf_eob_token_sized, coeff_base_lf_luma_context,
    coeff_base_lf_token_sized, coeff_br_hf_token, coeff_br_lf_luma_context, coeff_br_lf_token,
    eob_extra_token, eob_pt_token, luma_all_zero_token, luma_dc_sign_token,
};
// Re-exported only for the sibling 4x4 test modules' `eob_pt_16_token` references; the
// size-generic walk uses `eob_pt_token` instead.
#[cfg(test)]
#[allow(unused_imports)]
use super::eob_pt_16_token;
use crate::block_symbol_trace::BlockSymbolToken;
use crate::error::{Error, Result};

// The § 8.2 self-consistency recovery inverse moved to `general_walk_recover`; it is
// re-exported here so the sibling test modules resolve it through `super::*`. Only the
// test modules reference it, so the lib-only build sees it as unused.
#[allow(unused_imports)]
pub(super) use super::general_walk_recover::recover_quant_from_tokens;

/// The luma plane identity for general-walk diagnostics.
const PLANE_Y: PlaneId = PlaneId::Y;
/// `TX_CLASS_2D` index for the § 8.3.2 context mirrors.
const TRANSFORM_CLASS_2D: usize = 0;

/// `tcq_ctx = (tcqState >> 1) & 1` is 0 when TCQ is off.
const COEFF_BASE_LF_TCQ_CTX_NEUTRAL: usize = 0;
/// The smallest eob (eobPt `>= 3`) that carries the § 5.20.7.27 `eob_extra` CDF flag.
/// The decoder base for eobPt 3 is `(1 << (3 - 2)) + 1 == 3`. Shared with
/// [`super::general_walk_recover`].
pub(super) const MIN_EOB_WITH_EXTRA: usize = 3;
/// The largest eobPt the size-generic recovery accepts: eobPt 6 (eob 17..=32, the
/// 16x16 base-pass cap). It carries `eobPt - 3 == 3` `eob_extra_bit` literals. The 4x4
/// walk never produces an eob past 16 (eobPt 5), so this is the recovery upper bound
/// shared by both sizes. Shared with [`super::general_walk_recover`].
pub(super) const MAX_GENERAL_EOB_PT: usize = 6;
/// The neutral V-plane `txb_skip` context for an all-zero U/V tail (no `EobU`).
const V_TXB_SKIP_CTX_NEUTRAL: usize = 0;
/// The § 8.3.2 `coeff_br` low-frequency luma context for the EOB coefficient at the DC
/// raster position 0 (eob == 1): with an empty `Level[]` the decoder
/// `CoeffBrContext::ctx` reduces to `mag == 0` at `pos == 0`.
const COEFF_BR_LF_CTX_EOB_DC: usize = 0;
/// The § 8.3.2 `coeff_br` low-frequency luma context for the EOB coefficient at a
/// non-zero raster position (any AC, eob `>= 2`): with the empty `Level[]` the
/// `self.is_lf` branch yields `mag + 7 == 7`.
const COEFF_BR_LF_CTX_EOB_AC: usize = 7;
/// The § 8.3.2 `coeff_br` HIGH-frequency luma context for the EOB coefficient at an HF
/// raster position: the non-DC HF `else { mag }` branch with an empty `Level[]`
/// (`mag == 0`) yields `0` — NO `+7` offset (contrast [`COEFF_BR_LF_CTX_EOB_AC`]).
const HF_COEFF_BR_CTX_EOB: usize = 0;
/// The LF/HF boundary diagonal for a luma 2D block: a coefficient at raster
/// `(row, col)` is low-frequency iff `row + col < LF_DIAGONAL_LIMIT` (`4`), mirroring
/// the decoder `get_lf_limits` for `TX_CLASS_2D` luma. SIZE-INDEPENDENT.
const LF_DIAGONAL_LIMIT: usize = 4;
/// The largest magnitude a HIGH-frequency luma coefficient codes with one HF
/// `coeff_base`/`coeff_base_eob` and one HF `coeff_br` before the § 5.20.7.28
/// `read_quant` golomb tail (`NUM_BASE_LEVELS + COEFF_BASE_RANGE = 5`; HF
/// `maxLevel = 6`).
const MAX_HF_BASE_BR_MAGNITUDE: u32 = NUM_BASE_LEVELS + COEFF_BASE_RANGE;
/// The LF luma `maxLevel` (`LF_NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1 = 8`).
const LF_GOLOMB_MAX_LEVEL: u32 = MAX_BASE_BR_MAGNITUDE + 1;
/// The HF luma `maxLevel` (`NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1 = 6`).
const HF_GOLOMB_MAX_LEVEL: u32 = MAX_HF_BASE_BR_MAGNITUDE + 1;

/// 4x4 DCT_DCT coefficient count (`Quant[16]`); the public 4x4 entry's array length.
const TX_4X4_COEFF_COUNT: usize = 16;
/// The largest eobPt the 4x4 walk reaches (eob 9..=16 → eobPt 5). A 4x4 block cannot
/// reach eobPt 6 (its base 17 > 16).
const MAX_4X4_EOB_PT: usize = 5;

// 4x4 geometry constants, kept for the sibling 4x4 test modules (they pass these to
// the size-generic § 8.3.2 context mirrors directly). The size-generic walk reads
// them from `TxGeom::TX_4X4` instead.
#[cfg(test)]
const TX_4X4_WIDTH: usize = 4;
#[cfg(test)]
const TX_4X4_HEIGHT: usize = 4;
#[cfg(test)]
const TX_4X4_BWL: u32 = 2;
/// The largest in-window nonzero scan index for the 4x4 walk (eob `<= 16`). Kept for
/// the sibling 4x4 test modules' `MAX_GENERAL_SCAN_INDEX` references.
#[cfg(test)]
const MAX_GENERAL_SCAN_INDEX: usize = 15;

/// Tokenizes an arbitrary 4x4 DCT_DCT luma `Quant[16]` block in the general walk
/// window (eob `<= 16`: the full 4x4 scan — the low-frequency region scan `0..=9` plus
/// the entire high-frequency tail scan `10..=15`) into the ordered AV2 § 5.20.7.27
/// block-symbol trace (luma coefficients followed by the all-zero chroma U/V tail).
/// EVERY low-frequency coefficient may have a base-range magnitude
/// `1..=MAX_BASE_BR_MAGNITUDE` (`7`, adding `coeff_br`); a high-frequency coefficient
/// (EOB or non-EOB) caps at `MAX_HF_BASE_BR_MAGNITUDE` (`5`). A magnitude at or above
/// its position `maxLevel` carries the § 5.20.7.28 `read_quant` golomb tail.
///
/// `quant` is the row-major (raster) signed quantized block; `coeff_cdf_q_ctx` is the
/// caller-resolved coefficient-CDF q-context. Delegates to the size-generic
/// [`tokenize_general_luma_block`] with [`TxGeom::TX_4X4`], so its emitted stream is
/// byte-identical to before this refactor.
///
/// # Preconditions
/// Assumes **TCQ is off** (`allow_tcq == 0`). The § 5.20.7.28 `read_quant` threshold
/// is `level >= maxLevel - allow_tcq`; under TCQ the threshold drops by 1 and this
/// tokenizer's stream would desynchronize a TCQ-enabled decoder. Do not reuse on a
/// TCQ-enabled block.
pub(crate) fn tokenize_general_lf_luma_block(
    quant: &[i32; TX_4X4_COEFF_COUNT],
    coeff_cdf_q_ctx: usize,
) -> Result<Vec<BlockSymbolToken>> {
    tokenize_general_luma_block(quant, TxGeom::TX_4X4, MAX_4X4_EOB_PT, coeff_cdf_q_ctx)
}

/// Tokenizes an arbitrary DCT_DCT luma block described by `geom` into the ordered AV2
/// § 5.20.7.27 block-symbol trace. The SAME reverse-scan base/sign codepath the 4x4
/// entry uses, parameterized by [`TxGeom`] (so the 16x16 base pass reuses it).
///
/// `quant` is the row-major signed quantized block (`quant.len() == geom.coeff_count`);
/// `max_eob_pt` is the largest eobPt this caller admits (5 for the 4x4 full walk, 5
/// for the 16x16 base pass). An eob whose eobPt exceeds `max_eob_pt` is rejected with
/// [`Error::CoefficientTokenizationUnsupportedEob`] (the eob `>= 33` 16x16 case needs
/// the `eob_pt_256` symbol-7 `eob_pt_extra` refinement, the next brick). Errors:
///
/// - [`Error::CoefficientTokenizationUnsupportedEob`] when a nonzero sits at a scan
///   index `> geom.max_scan_index` (eob `> geom.coeff_count`) or the eob's eobPt
///   exceeds `max_eob_pt`, and
/// - [`Error::CoefficientTokenizationUnsupportedMagnitude`] when a golomb-range
///   coefficient's § 5.20.7.28 extension exceeds the per-`m` golomb cap.
pub(super) fn tokenize_general_luma_block(
    quant: &[i32],
    geom: TxGeom,
    max_eob_pt: usize,
    coeff_cdf_q_ctx: usize,
) -> Result<Vec<BlockSymbolToken>> {
    if quant.len() != geom.coeff_count {
        return Err(Error::CoefficientTokenizationAllocationFailed {
            context: "general walk quant length mismatch",
        });
    }
    let scan = build_scan(geom)?;
    let eob = end_of_block(quant, &scan);

    if eob == 0 {
        let mut trace = Vec::new();
        trace
            .try_reserve_exact(1)
            .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
                context: "general walk all-zero block trace",
            })?;
        trace.push(BlockSymbolToken::Coeff(luma_all_zero_token(
            coeff_cdf_q_ctx,
        )));
        return Ok(trace);
    }

    validate_general_scope(quant, &scan, eob, geom, max_eob_pt)?;

    let base_pass = compose_base_pass(quant, &scan, eob, geom, coeff_cdf_q_ctx)?;
    let sign_pass = compose_sign_pass(quant, &scan, eob, geom, coeff_cdf_q_ctx)?;

    // An eobPt-`>=3` block (eob `>= 3`) carries an `eob_extra` CDF token after the
    // `eob_pt_*` symbol, followed by `eobPt - 3` `eob_extra_bit` bypass literals.
    let eob_pt = eob_pt_from_eob(eob);
    let has_eob_extra = eob >= MIN_EOB_WITH_EXTRA;
    let (eob_extra_flag, eob_extra_bits, eob_extra_width) = if has_eob_extra {
        eob_refinement(eob, eob_pt)
    } else {
        (false, 0, 0)
    };
    // Header: luma all_zero + eob_pt_*, plus (when refined) the `eob_extra` flag and
    // `eob_extra_width` `eob_extra_bit` bypass literals.
    let header_len = 2usize
        + usize::from(has_eob_extra)
        + if has_eob_extra {
            eob_extra_width as usize
        } else {
            0
        };

    // The all-zero chroma tail appended after the sign pass: the U and V `txb_skip`
    // tokens (this brick's blocks have no coded chroma coefficients).
    const CHROMA_ALL_ZERO_TAIL_LEN: usize = 2;

    let total = base_pass
        .len()
        .checked_add(sign_pass.len())
        .and_then(|n| n.checked_add(header_len))
        .and_then(|n| n.checked_add(CHROMA_ALL_ZERO_TAIL_LEN))
        .ok_or(Error::CoefficientTokenizationAllocationFailed {
            context: "general walk coded block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
            context: "general walk coded block trace",
        })?;

    trace.push(BlockSymbolToken::Coeff(coded_luma_all_zero_token_sized(
        coeff_cdf_q_ctx,
        geom,
    )));
    trace.push(BlockSymbolToken::Coeff(eob_pt_token(
        geom.eob_pt_kind,
        coeff_cdf_q_ctx,
        EOB_CTX_LUMA_INTRA,
        eob_pt_symbol(eob),
    )));
    if has_eob_extra {
        // Emit the `eob_extra` CDF flag (the HIGH refinement bit), then the
        // `eob_extra_width` `eob_extra_bit` bypass literals (the LOW refinement bits)
        // MSB-first — the order the decoder reads them into `eob_extra_bits` via one
        // `read_literal(width)` (see the module `eob_extra_bit` BIT ORDER note).
        trace.push(BlockSymbolToken::Coeff(eob_extra_token(
            coeff_cdf_q_ctx,
            eob_extra_flag,
        )));
        for i in (0..eob_extra_width).rev() {
            let bit = (eob_extra_bits >> i) & 1;
            trace.push(BlockSymbolToken::bypass(1, bit));
        }
    }
    trace.extend(base_pass.into_iter().map(BlockSymbolToken::Coeff));
    trace.extend(sign_pass);
    trace.push(BlockSymbolToken::Coeff(chroma_u_all_zero_token(
        coeff_cdf_q_ctx,
    )));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        coeff_cdf_q_ctx,
        V_TXB_SKIP_CTX_NEUTRAL,
    )));
    Ok(trace)
}

/// Returns the coded luma `all_zero == 0` token at the block's `txSzCtx`. The 4x4
/// `coded_luma_all_zero_token` uses `TX_SIZE_4X4_CTX`; the recovery side checks the
/// 4x4 form, so the 4x4 geom MUST produce that exact token. The 16x16 base pass uses
/// its own `txb_skip` row, so it builds the token directly with `geom.tx_size_ctx`.
fn coded_luma_all_zero_token_sized(
    coeff_cdf_q_ctx: usize,
    geom: TxGeom,
) -> CoefficientEntropyToken {
    if geom.tx_size_ctx == TxGeom::TX_4X4.tx_size_ctx {
        coded_luma_all_zero_token(coeff_cdf_q_ctx)
    } else {
        super::coded_luma_all_zero_token_sized(coeff_cdf_q_ctx, geom.tx_size_ctx)
    }
}

/// Builds the AV2 2D scan order for the block described by `geom`. Shared with
/// [`super::general_walk_recover`].
pub(super) fn build_scan(geom: TxGeom) -> Result<Vec<u16>> {
    let mut scan = Vec::new();
    scan.try_reserve_exact(geom.coeff_count).map_err(|_| {
        Error::CoefficientTokenizationAllocationFailed {
            context: "general walk 2D scan order allocation",
        }
    })?;
    scan.resize(geom.coeff_count, 0u16);
    coefficient_scan_order(geom.width, geom.height, TransformClass::TwoD, &mut scan).map_err(
        |_| Error::CoefficientTokenizationAllocationFailed {
            context: "general walk 2D scan order",
        },
    )?;
    Ok(scan)
}

/// Returns the fixed block rectangle for general-walk diagnostics. The dimensions are
/// nonzero, so `PlaneRect::new` cannot fail; the error is mapped to a typed
/// tokenization error rather than panicking.
fn block_rect(geom: TxGeom) -> Result<PlaneRect> {
    PlaneRect::new(0, 0, geom.width, geom.height).map_err(|_| {
        Error::CoefficientTokenizationAllocationFailed {
            context: "general walk block rectangle",
        }
    })
}

/// Returns `eob = (max scan index c with Quant[scan[c]] != 0) + 1`, or 0 for an
/// all-zero block.
fn end_of_block(quant: &[i32], scan: &[u16]) -> usize {
    let mut eob = 0;
    for (c, &raster) in scan.iter().enumerate() {
        if quant.get(raster as usize).copied().unwrap_or(0) != 0 {
            eob = c + 1;
        }
    }
    eob
}

/// Rejects any nonzero outside the supported window (scan indices
/// `0..=geom.max_scan_index`, eob `<= geom.coeff_count`), any eob whose eobPt exceeds
/// `max_eob_pt`, or any golomb extension beyond the per-`m` cap. The window is the
/// FULL 2D scan: the whole low-frequency region plus the entire high-frequency tail.
///
/// MULTIPLE golomb coefficients per block are supported: the running `hrLevelAvg`
/// predictor is threaded across them in reverse scan, so each golomb coefficient's
/// `m = Clip3(1, 6, GetMsb(hrLevelAvg))` (and therefore its golomb cap) varies. This
/// validation walks the SAME reverse-scan order as the emission/recovery.
fn validate_general_scope(
    quant: &[i32],
    scan: &[u16],
    eob: usize,
    geom: TxGeom,
    max_eob_pt: usize,
) -> Result<()> {
    // Reject an eob whose eobPt exceeds this caller's window (e.g. eob >= 33 for the
    // 16x16 base pass, which needs the `eob_pt_256` symbol-7 `eob_pt_extra`
    // refinement, the next brick). Reported via the EOB error using the last in-window
    // scan index as the max.
    if eob_pt_from_eob(eob) > max_eob_pt {
        let c = eob - 1;
        let raster = scan_pos(scan, c)?;
        return Err(Error::CoefficientTokenizationUnsupportedEob {
            scan_index: c,
            position: raster,
            value: quant.get(raster).copied().unwrap_or(0),
            max_scan_index: max_eob_pt_scan_index(geom, max_eob_pt),
        });
    }

    // Reject any nonzero outside the supported scan window (forward scan, so the
    // smallest offending scan index is reported).
    for (c, &raster) in scan.iter().enumerate() {
        let value = quant.get(raster as usize).copied().unwrap_or(0);
        if value == 0 {
            continue;
        }
        if c > geom.max_scan_index {
            return Err(Error::CoefficientTokenizationUnsupportedEob {
                scan_index: c,
                position: raster as usize,
                value,
                max_scan_index: geom.max_scan_index,
            });
        }
    }

    // Thread the running `hrLevelAvg` across the golomb coefficients in reverse scan.
    let mut hr_level_avg = 0u32;
    for offset in 0..eob {
        let c = eob - 1 - offset;
        let pos = scan_pos(scan, c)?;
        let value = quant.get(pos).copied().unwrap_or(0);
        if value == 0 {
            continue;
        }
        let magnitude = value.unsigned_abs();
        let max_level = general_walk_max_level_for_pos(pos, geom);
        if magnitude < max_level {
            continue;
        }
        let params = golomb_params_from_hr_level_avg(hr_level_avg);
        let x = magnitude - max_level;
        let x_max = golomb_x_max(params);
        if x > x_max {
            return Err(Error::CoefficientTokenizationUnsupportedMagnitude {
                plane: PLANE_Y,
                block: block_rect(geom)?,
                coefficient_index: pos,
                magnitude,
                max_magnitude: max_level + x_max,
            });
        }
        hr_level_avg = next_hr_level_avg(x, hr_level_avg);
    }
    Ok(())
}

/// Returns the largest in-window nonzero scan index for an eobPt-bounded window: the
/// `eob - 1` of the largest eob whose eobPt is `<= max_eob_pt` (clamped to the
/// block's `max_scan_index`). Used only to populate the rejection error's
/// `max_scan_index` field for the eobPt-too-large case.
fn max_eob_pt_scan_index(geom: TxGeom, max_eob_pt: usize) -> usize {
    // The largest eob for eobPt `<= max_eob_pt` is the base of `max_eob_pt + 1` minus
    // one (the eob just below the next eobPt's base), clamped to the block window.
    let next_base = eob_base_for_pt(max_eob_pt + 1);
    next_base.saturating_sub(2).min(geom.max_scan_index)
}

/// Returns whether the luma 2D coefficient at raster position `pos` is in the
/// low-frequency region (`row + col < LF_DIAGONAL_LIMIT`, i.e. `< 4`), mirroring the
/// decoder `get_lf_limits` for `TX_CLASS_2D` luma. SIZE-INDEPENDENT predicate keyed on
/// the block geometry's `bwl`.
fn is_lf_position_geom(pos: usize, geom: TxGeom) -> bool {
    let row = pos.checked_shr(geom.bwl).unwrap_or(0);
    let col = pos - row.checked_shl(geom.bwl).unwrap_or(0);
    row + col < LF_DIAGONAL_LIMIT
}

/// The 4x4 form of [`is_lf_position_geom`] (`TxGeom::TX_4X4`). Kept for the sibling
/// 4x4 test modules' `is_lf_position(pos)` calls.
#[cfg(test)]
fn is_lf_position(pos: usize) -> bool {
    is_lf_position_geom(pos, TxGeom::TX_4X4)
}

/// Returns the AV2 § 5.20.7.27/§ 5.20.7.28 `maxLevel` for the luma 2D coefficient at
/// raster position `pos` — the level at which `read_quant` fires (TCQ off). A
/// low-frequency coefficient saturates at `MAX_BASE_BR_MAGNITUDE` (`7`), so its
/// `maxLevel` is `8`; a high-frequency coefficient saturates at
/// `MAX_HF_BASE_BR_MAGNITUDE` (`5`), so its `maxLevel` is `6`. Mirrors the decoder
/// `derive_coeff_max_level` per region. Shared with [`super::general_walk_recover`].
pub(super) fn general_walk_max_level_for_pos(pos: usize, geom: TxGeom) -> u32 {
    if is_lf_position_geom(pos, geom) {
        LF_GOLOMB_MAX_LEVEL
    } else {
        HF_GOLOMB_MAX_LEVEL
    }
}

/// Composes the reverse-scan base pass over `c = eob - 1 .. 0` using a running
/// `Level[]` for the § 8.3.2 luma `coeff_base` / `coeff_br` contexts of the non-EOB
/// coefficients. The EOB coefficient (visited first) emits its `coeff_base_eob` and,
/// when its magnitude exceeds its position base-level threshold, an interleaved
/// `coeff_br` at the constant empty-`Level[]` context. Each non-EOB coefficient emits
/// its `coeff_base` and, when refined, an interleaved `coeff_br` whose context is
/// derived from the running `Level[]`.
///
/// LF/HF SELECTION: each coefficient's low-frequency predicate is derived from its OWN
/// raster `row + col < LF_DIAGONAL_LIMIT` (`4`). A low-frequency coefficient emits the
/// 6-symbol LF table (LF context, cap `LF_NUM_BASE_LEVELS`); a high-frequency one
/// emits the 4-symbol HF table (the `coeff_base_hf_luma_context` band, cap
/// `NUM_BASE_LEVELS`). All token constructors carry the block's `geom.tx_size_ctx`.
fn compose_base_pass(
    quant: &[i32],
    scan: &[u16],
    eob: usize,
    geom: TxGeom,
    coeff_cdf_q_ctx: usize,
) -> Result<Vec<CoefficientEntropyToken>> {
    let mut tokens = Vec::new();
    // One `coeff_base`/`coeff_base_eob` token per coefficient, plus at most one
    // `coeff_br` per coefficient.
    let capacity = eob
        .checked_mul(2)
        .ok_or(Error::CoefficientTokenizationAllocationFailed {
            context: "general walk base pass token capacity",
        })?;
    tokens.try_reserve_exact(capacity).map_err(|_| {
        Error::CoefficientTokenizationAllocationFailed {
            context: "general walk base pass tokens",
        }
    })?;
    // The running `Level[]` is sized to the block coefficient count, allocated checked.
    let mut level = Vec::new();
    level.try_reserve_exact(geom.coeff_count).map_err(|_| {
        Error::CoefficientTokenizationAllocationFailed {
            context: "general walk base pass level state",
        }
    })?;
    level.resize(geom.coeff_count, 0u32);

    for offset in 0..eob {
        let c = eob - 1 - offset;
        let pos = scan_pos(scan, c)?;
        let magnitude = quant.get(pos).copied().unwrap_or(0).unsigned_abs();
        if offset == 0 {
            // EOB coefficient: `coeff_base_eob`; `level = coeff_base_eob + 1`. The base
            // level saturates at `base_levels + 1` (LF `LF_NUM_BASE_LEVELS`, HF
            // `NUM_BASE_LEVELS`) and `coeff_br` is read when the level exceeds
            // `base_levels`.
            if is_lf_position_geom(pos, geom) {
                let eob_level = magnitude.min(LF_NUM_BASE_LEVELS + 1) as u8;
                tokens.push(coeff_base_lf_eob_token_sized(
                    coeff_cdf_q_ctx,
                    geom.tx_size_ctx,
                    coeff_base_eob_ctx_geom(c, geom),
                    eob_level,
                ));
                if magnitude > LF_NUM_BASE_LEVELS {
                    let br_symbol =
                        (magnitude - (LF_NUM_BASE_LEVELS + 1)).min(COEFF_BASE_RANGE) as u8;
                    tokens.push(coeff_br_lf_token(
                        coeff_cdf_q_ctx,
                        eob_coeff_br_ctx(pos),
                        br_symbol,
                    ));
                }
            } else {
                let eob_level = magnitude.min(NUM_BASE_LEVELS + 1) as u8;
                tokens.push(coeff_base_hf_eob_token_sized(
                    coeff_cdf_q_ctx,
                    geom.tx_size_ctx,
                    coeff_base_eob_ctx_geom(c, geom),
                    eob_level,
                ));
                if magnitude > NUM_BASE_LEVELS {
                    let br_symbol = (magnitude - (NUM_BASE_LEVELS + 1)).min(COEFF_BASE_RANGE) as u8;
                    tokens.push(coeff_br_hf_token(
                        coeff_cdf_q_ctx,
                        HF_COEFF_BR_CTX_EOB,
                        br_symbol,
                    ));
                }
            }
        } else if is_lf_position_geom(pos, geom) {
            // Non-EOB LOW-frequency coefficient: the § 8.3.2 LF luma `coeff_base`
            // context derived from the partially-built `Level[]`. A non-EOB
            // `coeff_base` symbol is `min(mag, LF_NUM_BASE_LEVELS + 1)`.
            let ctx = coeff_base_lf_luma_context(
                pos,
                geom.bwl,
                geom.width,
                geom.height,
                TRANSFORM_CLASS_2D,
                c,
                &level,
            );
            let base_symbol = magnitude.min(LF_NUM_BASE_LEVELS + 1) as u8;
            tokens.push(coeff_base_lf_token_sized(
                coeff_cdf_q_ctx,
                geom.tx_size_ctx,
                ctx,
                COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
                base_symbol,
            ));
            if magnitude > LF_NUM_BASE_LEVELS {
                let br_ctx = coeff_br_lf_luma_context(
                    pos,
                    geom.bwl,
                    geom.width,
                    geom.height,
                    TRANSFORM_CLASS_2D,
                    true,
                    &level,
                );
                let br_symbol = (magnitude - (LF_NUM_BASE_LEVELS + 1)).min(COEFF_BASE_RANGE) as u8;
                tokens.push(coeff_br_lf_token(coeff_cdf_q_ctx, br_ctx, br_symbol));
            }
        } else {
            // Non-EOB HIGH-frequency coefficient: the § 8.3.2 HF luma `coeff_base`
            // context derived from the partially-built `Level[]` via
            // `coeff_base_hf_luma_context`. The HF base level saturates at
            // `NUM_BASE_LEVELS + 1 == 3` (a 4-symbol CDF) and `coeff_br` (the HF
            // `coeff_br`, no `+7`) refines when the magnitude exceeds `NUM_BASE_LEVELS`.
            let ctx = coeff_base_hf_luma_context(
                pos,
                geom.bwl,
                geom.width,
                geom.height,
                TRANSFORM_CLASS_2D,
                &level,
            );
            let base_symbol = magnitude.min(NUM_BASE_LEVELS + 1) as u8;
            tokens.push(coeff_base_hf_token_sized(
                coeff_cdf_q_ctx,
                geom.tx_size_ctx,
                ctx,
                COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
                base_symbol,
            ));
            if magnitude > NUM_BASE_LEVELS {
                let br_ctx = coeff_br_lf_luma_context(
                    pos,
                    geom.bwl,
                    geom.width,
                    geom.height,
                    TRANSFORM_CLASS_2D,
                    false,
                    &level,
                );
                let br_symbol = (magnitude - (NUM_BASE_LEVELS + 1)).min(COEFF_BASE_RANGE) as u8;
                tokens.push(coeff_br_hf_token(coeff_cdf_q_ctx, br_ctx, br_symbol));
            }
        }
        // Write `Level[pos] = mag` before deriving the next (lower-c) context.
        if let Some(slot) = level.get_mut(pos) {
            *slot = magnitude;
        }
    }
    Ok(tokens)
}

/// Composes the reverse-scan, interleaved sign+quant pass over `c = eob - 1 .. 0`: a
/// `dc_sign` CDF token for the DC at raster position 0, a `sign_bit` bypass for every
/// other coefficient, no sign for a zero coefficient, and — for a coefficient whose
/// magnitude reaches its position `maxLevel` — the § 5.20.7.28 `read_quant` golomb
/// tail emitted RIGHT AFTER its sign token. MULTIPLE golomb coefficients are supported
/// via the running `hrLevelAvg` predictor.
fn compose_sign_pass(
    quant: &[i32],
    scan: &[u16],
    eob: usize,
    geom: TxGeom,
    coeff_cdf_q_ctx: usize,
) -> Result<Vec<BlockSymbolToken>> {
    // Reserve one sign token per nonzero coefficient plus, for every golomb
    // coefficient, its golomb-tail bypass literals (an exact upper bound).
    let mut reserve = eob;
    let mut hr_level_avg = 0u32;
    for offset in 0..eob {
        let c = eob - 1 - offset;
        let pos = scan_pos(scan, c)?;
        let magnitude = quant.get(pos).copied().unwrap_or(0).unsigned_abs();
        let max_level = general_walk_max_level_for_pos(pos, geom);
        if magnitude >= max_level {
            let x = magnitude - max_level;
            let params = golomb_params_from_hr_level_avg(hr_level_avg);
            reserve = reserve
                .checked_add(read_quant_golomb_tail_len(x, params))
                .ok_or(Error::CoefficientTokenizationAllocationFailed {
                    context: "general walk sign pass golomb reservation",
                })?;
            hr_level_avg = next_hr_level_avg(x, hr_level_avg);
        }
    }
    let mut tokens = Vec::new();
    tokens.try_reserve_exact(reserve).map_err(|_| {
        Error::CoefficientTokenizationAllocationFailed {
            context: "general walk sign pass tokens",
        }
    })?;

    // Reset the predictor for the emission loop (it walks the same reverse-scan order).
    let mut hr_level_avg = 0u32;
    for offset in 0..eob {
        let c = eob - 1 - offset;
        let pos = scan_pos(scan, c)?;
        let value = quant.get(pos).copied().unwrap_or(0);
        if value == 0 {
            continue;
        }
        let negative = value < 0;
        let row = pos.checked_shr(geom.bwl).unwrap_or(0);
        let col = pos - row.checked_shl(geom.bwl).unwrap_or(0);
        if row == 0 && col == 0 {
            // The luma DC sign is CDF-coded (`dc_sign`).
            tokens.push(BlockSymbolToken::Coeff(luma_dc_sign_token(
                coeff_cdf_q_ctx,
                negative,
            )));
        } else {
            // Every non-DC luma sign under TX_CLASS_2D is a § 8.2.5 `sign_bit` bypass.
            tokens.push(BlockSymbolToken::bypass(1, u32::from(negative)));
        }
        let magnitude = value.unsigned_abs();
        let max_level = general_walk_max_level_for_pos(pos, geom);
        if magnitude >= max_level {
            let x = magnitude - max_level;
            let params = golomb_params_from_hr_level_avg(hr_level_avg);
            push_read_quant_golomb_tail(&mut tokens, x, params)?;
            hr_level_avg = next_hr_level_avg(x, hr_level_avg);
        }
    }
    Ok(tokens)
}

/// Returns the AV2 § 8.3.2 `coeff_base_eob` context for a luma EOB coefficient at scan
/// index `c`: `c == 0 -> 0`, `c <= numCoeffs/8 -> 1`, `c <= numCoeffs/4 -> 2`,
/// otherwise `3`. `numCoeffs = geom.num_coeffs` (`Tx_Height << Tx_Width_Log2`): 16 for
/// 4x4 (breaks at 2, 4), 256 for 16x16 (breaks at 32, 64). Mirrors the decoder
/// `coeff_base_eob_ctx(c, bwl, height)`
/// (`crates/splot-decode/src/tile_payload/cdf/coeff_context.rs`).
pub(super) fn coeff_base_eob_ctx_geom(c: usize, geom: TxGeom) -> usize {
    let num_coeffs = geom.num_coeffs;
    if c == 0 {
        0
    } else if c <= num_coeffs / 8 {
        1
    } else if c <= num_coeffs / 4 {
        2
    } else {
        3
    }
}

/// The 4x4 form of [`coeff_base_eob_ctx_geom`] (`TxGeom::TX_4X4`, `numCoeffs = 16`,
/// breaks at 2 & 4). Kept for the sibling 4x4 test modules' `coeff_base_eob_ctx(c)`
/// calls.
#[cfg(test)]
fn coeff_base_eob_ctx(c: usize) -> usize {
    coeff_base_eob_ctx_geom(c, TxGeom::TX_4X4)
}

/// Returns the AV2 § 5.20.7.27 `eobPt` for an eob (the inverse of the decoder base
/// `(eobPt < 2) ? eobPt : (1 << (eobPt - 2)) + 1`): eob 1 → 1, eob 2 → 2, eob 3..=4 → 3,
/// eob 5..=8 → 4, eob 9..=16 → 5, eob 17..=32 → 6, eob 33..=64 → 7, eob 65..=128 → 8,
/// eob `>= 129` → 9 (the cap for any modeled transform). The general walk admits only
/// the bounded eobPt window per [`tokenize_general_luma_block`]'s `max_eob_pt`; a
/// larger eobPt is rejected upstream by [`validate_general_scope`]. Total over any eob.
const fn eob_pt_from_eob(eob: usize) -> usize {
    if eob < 2 {
        1
    } else if eob <= 2 {
        2
    } else if eob <= 4 {
        3
    } else if eob <= 8 {
        4
    } else if eob <= 16 {
        5
    } else if eob <= 32 {
        6
    } else if eob <= 64 {
        7
    } else if eob <= 128 {
        8
    } else {
        9
    }
}

/// Returns the decoder eob base for `eobPt`: `(eobPt < 2) ? eobPt :
/// (1 << (eobPt - 2)) + 1` (eobPt 1 → 1, 2 → 2, 3 → 3, 4 → 5, 5 → 9, 6 → 17). Mirrors
/// `nonzero_coeff_eob` (`crates/splot-decode/src/tile_payload/coeff_loop.rs`). Shared
/// with [`super::general_walk_recover`].
pub(super) const fn eob_base_for_pt(eob_pt: usize) -> usize {
    if eob_pt < 2 {
        eob_pt
    } else {
        (1 << (eob_pt - 2)) + 1
    }
}

/// Returns the `eob_pt_*` symbol (`eobPt - 1`) for an eob in `1..=32`. The size-class
/// `eob_pt_*` symbol carries `eobPt - 1`, NOT `eob - 1`.
const fn eob_pt_symbol(eob: usize) -> u8 {
    (eob_pt_from_eob(eob) - 1) as u8
}

/// Derives the § 5.20.7.27 `(eob_extra, eob_extra_bits, width)` refinement for an eob
/// whose `eobPt >= 3`. With `base = eob_base_for_pt(eobPt)`, `offset = eob - base`,
/// `width = eobPt - 3`: `eob_extra` is the HIGH bit `(offset >> width) & 1` and
/// `eob_extra_bits` is the LOW `width` bits `offset & ((1 << width) - 1)`. The exact
/// inverse of the decoder `eob = base + (eob_extra << width) + eob_extra_bits`.
const fn eob_refinement(eob: usize, eob_pt: usize) -> (bool, u32, u32) {
    let base = eob_base_for_pt(eob_pt);
    let offset = eob - base;
    let width = (eob_pt - 3) as u32;
    let eob_extra = (offset >> width) & 1 != 0;
    let eob_extra_bits = (offset & ((1usize << width) - 1)) as u32;
    (eob_extra, eob_extra_bits, width)
}

/// Returns the constant § 8.3.2 `coeff_br` low-frequency luma context for the EOB
/// coefficient at raster position `pos`. The EOB coefficient is visited first in
/// reverse scan, so the running `Level[]` is empty: ctx 0 at the DC raster position 0,
/// ctx 7 at any non-DC low-frequency position.
const fn eob_coeff_br_ctx(pos: usize) -> usize {
    if pos == 0 {
        COEFF_BR_LF_CTX_EOB_DC
    } else {
        COEFF_BR_LF_CTX_EOB_AC
    }
}

/// Returns `scan[c]` as a raster position, validating the scan index. Shared with
/// [`super::general_walk_recover`].
pub(super) fn scan_pos(scan: &[u16], c: usize) -> Result<usize> {
    scan.get(c).map(|&raster| raster as usize).ok_or(
        Error::CoefficientTokenizationAllocationFailed {
            context: "general walk scan index out of range",
        },
    )
}

#[cfg(test)]
#[path = "general_walk_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "general_walk_golomb_tests.rs"]
mod golomb_tests;

#[cfg(test)]
#[path = "general_walk_eob_extra_tests.rs"]
mod eob_extra_tests;

#[cfg(test)]
#[path = "general_walk_hf_tests.rs"]
mod hf_tests;

#[cfg(test)]
#[path = "general_walk_hf_multi_tests.rs"]
mod hf_multi_tests;
