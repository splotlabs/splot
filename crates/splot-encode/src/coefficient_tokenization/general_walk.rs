// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.7.27 GENERAL coefficient-tokenization walk for the FULL 4x4 scan: the
//! low-frequency base tier plus the entire high-frequency tail
//! (`ENC-COEFF-GENERAL-WALK-LF-BASE`, extended by `ENC-COEFF-GENERAL-WALK-HF-EOB11`
//! and `ENC-COEFF-GENERAL-WALK-HF-MULTI`).
//!
//! This walks an arbitrary quantized 4x4 DCT_DCT luma `Quant[16]` block whose
//! nonzero coefficients sit anywhere in the 4x4 scan (scan indices `0..=15`, eob
//! `<= 16`) — the entire low-frequency region (scan `0..=9`) plus the entire
//! high-frequency tail (scan `10..=15`) — and emits the ordered § 5.20.7.27
//! coefficient token stream the decoder coefficient loop reads: the luma `txb_skip`,
//! `eob_pt_16`, an optional `eob_extra` CDF flag and `eob_extra_bit` bypass literals
//! (read only when eobPt `>= 3`, i.e. eob `>= 3`), the reverse-scan `coeff_base_eob`
//! / `coeff_base` base pass (with the running-`Level[]` § 8.3.2 LF luma context from
//! [`super::coeff_base_lf_luma_context`] for low-frequency coefficients and the HF
//! luma context from [`super::coeff_base_hf_luma_context`] for high-frequency ones),
//! the reverse-scan interleaved sign pass (`dc_sign` CDF for the DC, `sign_bit`
//! § 8.2.5 bypass for the AC), and the all-zero chroma U/V `txb_skip` tail. It reuses
//! the existing token constructors and CDF routing; it never invents AV2 CDF values
//! or contexts.
//!
//! LF REGION BOUNDARY: for luma `TX_CLASS_2D` the decoder
//! `get_lf_limits(row, col, txClass, plane)`
//! (`crates/splot-decode/src/tile_payload/coeff_loop/max_level.rs`) marks a
//! coefficient low-frequency iff `row + col < 4` — NOT by scan index. For the 4x4 2D
//! scan order `[0, 4, 1, 8, 5, 2, 12, 9, 6, 3, 13, 10, 7, 14, 11, 15]`, scan indices
//! `0..=9` map to raster positions whose `row + col` diagonals are all `<= 3` (the
//! first scan index that lands on diagonal 4 is scan index 10, raster 13 = row 3,
//! col 1). So eob `1..=10` are ENTIRELY low-frequency and use the LF `coeff_base` /
//! `coeff_br` contexts; eob `11..=16` add high-frequency coefficients (scan indices
//! `10..=15`, rasters 13, 10, 7, 14, 11, 15; diagonals 4..=6), which use DIFFERENT
//! § 8.3.2 CDF tables (see the HF notes below). eob `>= 17` is impossible for a 4x4
//! block (eob 16 is the max) and is rejected. Each coefficient's LF/HF predicate is
//! derived from its OWN raster `row + col < 4`, not a scan-index threshold (so the
//! per-position selection in [`compose_base_pass`] handles a mixed block: every
//! scan-`0..=9` coefficient is LF, every scan-`10..=15` coefficient is HF, regardless
//! of whether it is the EOB coefficient or a non-EOB one).
//!
//! HF EOB COEFFICIENT (eob 11, scan index 10, raster 13): the EOB coefficient at an
//! HF position uses DIFFERENT § 8.3.2 CDF tables than at an LF position — verified
//! against the decoder and the generated default tables:
//!
//! - `coeff_base_eob` reads the 4-symbol HF `DEFAULT_COEFF_BASE_EOB_CDF`
//!   (`[q][tx_size][ctx][row]`), NOT the 6-symbol LF `DEFAULT_COEFF_BASE_LF_EOB_CDF`.
//!   The `coeff_base_eob` *context* is shared (scan-band based,
//!   [`coeff_base_eob_ctx`]); for eob 11 the EOB coeff is at scan `c = 10` in a
//!   16-coeff block → `coeff_base_eob_ctx(10) == 3`. The HF EOB token level mapping
//!   uses the HF base-level cap (`eob_level = min(mag, NUM_BASE_LEVELS + 1) == min(mag,
//!   3)`, NOT the LF `LF_NUM_BASE_LEVELS + 1 == 5`).
//! - When the HF EOB coeff magnitude exceeds `NUM_BASE_LEVELS`, its `coeff_br`
//!   reads the HF `DEFAULT_COEFF_BR_CDF` (`[q][ctx][row]`, NO transform-size
//!   dimension), NOT the LF `DEFAULT_COEFF_BR_LF_CDF`. The HF `coeff_br` context for a
//!   non-DC luma coefficient is plain `mag` (range `0..=6`) with NO `+7` offset (the
//!   LF non-DC branch adds `+7`; the HF non-DC `else { mag }` branch of the decoder
//!   `CoeffBrContext::ctx` does not). For the EOB coefficient (visited first in
//!   reverse scan, empty `Level[]`) the neighbour sum is `0` → `mag == 0` → HF
//!   `coeff_br` ctx `== 0` (constant, [`HF_COEFF_BR_CTX_EOB`]).
//!
//! NON-EOB HF `coeff_base` (eob 12..=16, scan indices 10..eob-2 that are not the EOB
//! coefficient): a non-EOB high-frequency coefficient uses the 4-symbol HF
//! `DEFAULT_COEFF_BASE_CDF` (`[q][tx_size][ctx][tcq][row]`), DISTINCT from the LF
//! 6-symbol `DEFAULT_COEFF_BASE_LF_CDF`. Its § 8.3.2 context
//! ([`super::coeff_base_hf_luma_context`], the decoder `CoeffBaseContext::select`
//! `is_lf == false` branch) shares the neighbour magnitude-sum loop with the LF
//! context but with `magLimit = 3` for EVERY neighbour (NO near-DC `magLimit = 5`
//! carve-out) and NO `c == 0` / DC band; `ctx = (mag + 1) >> 1`, `ctx2 = min(ctx, 4)`,
//! 2D band `row+col < 6 -> ctx2`, `< 8 -> ctx2 + 5`, else `ctx2 + 10` (1-D: `ctx2 +
//! 15`). The non-EOB HF base level saturates at `NUM_BASE_LEVELS + 1 == 3` (the
//! 4-symbol table; symbol equals the level, capped at 3) and the HF `coeff_br` refines
//! when the magnitude exceeds `NUM_BASE_LEVELS`, up to `MAX_HF_BASE_BR_MAGNITUDE == 5`.
//!
//! EOB SIGNALING (mirrors the decoder `nonzero_coeff_eob` arithmetic and the
//! `read_nonzero_coeff_eob` read sequence in
//! `crates/splot-decode/src/tile_payload/coeff_loop.rs`, and the § 5.20.7.27 eob
//! refinement loop at `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`):
//! `eob_pt_16` carries `eobPt - 1`. eob 1 → eobPt 1, eob 2 → eobPt 2 (both eobPt
//! `< 3`, NO refinement). The eob→eobPt mapping (decoder base = `(1 << (eobPt-2)) + 1`):
//! eob 3..=4 → eobPt 3 (base 3), eob 5..=8 → eobPt 4 (base 5), eob 9..=10 → eobPt 5
//! (base 9). For eobPt `>= 3` the decoder reads `eob_extra` (a CDF flag = the HIGH
//! refinement bit) then `eobPt - 3` `eob_extra_bit` literals (the LOW refinement bits)
//! and computes `eob = base + (eob_extra << (eobPt - 3)) + eob_extra_bits`. From the
//! input eob this brick derives `offset = eob - base`,
//! `eob_extra = (offset >> (eobPt - 3)) & 1` (the high bit), and
//! `eob_extra_bits = offset & ((1 << (eobPt - 3)) - 1)` (the low bits). eobPt 3 has
//! `eobPt - 3 == 0` bypass bits (none); eobPt 4 has 1; eobPt 5 has 2.
//!
//! `eob_extra_bit` BIT ORDER (load-bearing, mirrored from decoder/spec — the § 8.2
//! roundtrip CANNOT catch a bit-order error, see HONESTY below): the spec loop
//! `for ( i = eobPt - 4; i >= 0; i-- ) { eob_extra_bit L(1); if (eob_extra_bit) eob
//! += 1 << i }` reads the bit for `i = eobPt - 4` (the MSB of `eob_extra_bits`,
//! position `eobPt - 4 == width - 1`) FIRST, down to `i = 0` (the LSB) LAST. The
//! decoder reads them as one `read_literal(width)` (`read_nonzero_coeff_eob` →
//! `read_eob_literal` → `SymbolDecoder::read_literal`), which is MSB-first
//! (`value = (value << 1) | read_bool()`). So this tokenizer emits the
//! `eob_extra_bit` bypass literals MSB-first: `bypass(1, (eob_extra_bits >> i) & 1)`
//! for `i` from `width - 1` down to `0`. [`recover_quant_from_tokens`] reads them
//! back in the SAME MSB-first order.
//!
//! Every coefficient may carry a base-range magnitude: a magnitude above the base
//! tier emits a `coeff_br` token right after its `coeff_base_eob` / `coeff_base`
//! (interleaved, before the `Level[]` write). The EOB coefficient's `coeff_br` context
//! is a constant because the running `Level[]` is empty when it is visited first in
//! reverse scan (LF: ctx 0 at the DC raster position, ctx 7 at a non-DC LF position —
//! see [`COEFF_BR_LF_CTX_EOB_DC`] / [`COEFF_BR_LF_CTX_EOB_AC`]; HF: the constant ctx 0,
//! [`HF_COEFF_BR_CTX_EOB`]). A NON-EOB coefficient's `coeff_br` context is
//! DATA-DEPENDENT — derived from the running `Level[]` (the already-written neighbours)
//! via the [`super::coeff_br_lf_luma_context`] mirror of the decoder
//! `CoeffBrContext::ctx`. The LF base tier saturates at `LF_NUM_BASE_LEVELS + 1` (max
//! magnitude `7`); the HF EOB coefficient saturates at the lower `NUM_BASE_LEVELS + 1`
//! (max magnitude `5`).
//!
//! A magnitude at-or-above its position `maxLevel` (LF `8`, HF `6`) is a § 5.20.7.28
//! `read_quant` GOLOMB coefficient: its base+`coeff_br` level saturates at `maxLevel`
//! and the extension `x = magnitude - maxLevel` is coded in the golomb tail (sign
//! pass). MULTIPLE golomb coefficients per block are supported — the running
//! `hrLevelAvg` predictor is threaded across them in reverse scan, so each golomb
//! coefficient's golomb parameter `m` (and thus its supported magnitude cap) varies
//! (`ENC-COEFF-GENERAL-WALK-GOLOMB-MULTI`). Only two things are rejected with a typed
//! error: a nonzero at scan index `>= 16` (eob `>= 17`, impossible for a 4x4 block),
//! and a golomb extension beyond the per-`m` cap (the golomb-prefix `length` would
//! exceed `8`).
//!
//! HONESTY: the [`recover_quant_from_tokens`] proof is § 8.2 SELF-CONSISTENCY. The
//! same code authored the emission and its inverse, so it proves the encoder's
//! emitted (level, sign, position) triples are internally reversible — with
//! asymmetric values it catches a swapped sign order (AC-before-DC) or a
//! level/position transposition. It does NOT validate the § 8.3.2 CDF contexts
//! against a real decoder; context conformance is deferred to the splot-decode
//! cross-check brick.

use splot_recon::{PlaneId, PlaneRect, TransformClass, coefficient_scan_order};

use super::general_walk_golomb::{
    golomb_params_from_hr_level_avg, golomb_x_max, next_hr_level_avg, push_read_quant_golomb_tail,
    read_quant_golomb_tail_len,
};
use super::{
    COEFF_BASE_RANGE, CoefficientEntropyToken, EOB_CTX_LUMA_INTRA, LF_NUM_BASE_LEVELS,
    MAX_BASE_BR_MAGNITUDE, NUM_BASE_LEVELS, chroma_u_all_zero_token, chroma_v_all_zero_token,
    coded_luma_all_zero_token, coeff_base_hf_eob_token, coeff_base_hf_luma_context,
    coeff_base_hf_token, coeff_base_lf_eob_token, coeff_base_lf_luma_context, coeff_base_lf_token,
    coeff_br_hf_token, coeff_br_lf_luma_context, coeff_br_lf_token, eob_extra_token,
    eob_pt_16_token, luma_all_zero_token, luma_dc_sign_token,
};
use crate::block_symbol_trace::BlockSymbolToken;
use crate::error::{Error, Result};

// The § 8.2 self-consistency recovery inverse moved to `general_walk_recover`; it is
// re-exported here so the sibling test modules resolve it through `super::*`. Only the
// test modules reference it, so the lib-only build sees it as unused.
#[allow(unused_imports)]
pub(super) use super::general_walk_recover::recover_quant_from_tokens;

/// The luma plane identity for general-walk diagnostics.
const PLANE_Y: PlaneId = PlaneId::Y;
/// `TX_CLASS_2D` index for [`coeff_base_lf_luma_context`].
const TRANSFORM_CLASS_2D: usize = 0;

/// 4x4 DCT_DCT transform geometry for the general low-frequency walk.
const TX_4X4_WIDTH: usize = 4;
const TX_4X4_HEIGHT: usize = 4;
const TX_4X4_COEFF_COUNT: usize = TX_4X4_WIDTH * TX_4X4_HEIGHT;
/// `bwl = Tx_Width_Log2[TX_4X4] = 2`.
const TX_4X4_BWL: u32 = 2;
/// `tcq_ctx = (tcqState >> 1) & 1` is 0 when TCQ is off.
const COEFF_BASE_LF_TCQ_CTX_NEUTRAL: usize = 0;
/// The largest nonzero scan index this brick covers (eob `<= 16`, eobPt `<= 6`).
/// The whole low-frequency region of a 4x4 luma 2D block is scan indices `0..=9`
/// (every one has `row + col < 4`; see the module LF REGION BOUNDARY note), plus the
/// entire high-frequency tail at scan indices `10..=15` (rasters 13, 10, 7, 14, 11,
/// 15; diagonals 4..=6). A nonzero at scan index `>= 16` is impossible for a 4x4
/// block (eob 16 is the max) and is rejected. The name keeps its historical `LF`
/// spelling, but the window is now the full 4x4 scan (LF + HF).
const MAX_GENERAL_SCAN_INDEX: usize = 15;
/// The smallest eob (eobPt `>= 3`) that carries the § 5.20.7.27 `eob_extra` CDF
/// flag. The decoder base for eobPt 3 is `(1 << (3 - 2)) + 1 == 3`, so eob 3 is the
/// smallest refined eob. Shared with [`super::general_walk_recover`].
pub(super) const MIN_EOB_WITH_EXTRA: usize = 3;
/// The largest eobPt this brick reaches: eob 9..=16 → eobPt 5 (`eob_pt_16` symbol
/// 4). eobPt 5 carries `eobPt - 3 == 2` `eob_extra_bit` literals and (base 9) spans
/// the full eob 9..=16 (eob_extra 0 → 9..=12, eob_extra 1 → 13..=16). A 4x4 block
/// cannot reach eobPt 6 (its base is 17 > 16). Shared with
/// [`super::general_walk_recover`].
pub(super) const MAX_GENERAL_EOB_PT: usize = 5;
/// The neutral V-plane `txb_skip` context for an all-zero U/V tail (no `EobU`).
const V_TXB_SKIP_CTX_NEUTRAL: usize = 0;
/// AV2 § 8.3.2 `SIG_COEF_CONTEXTS_EOB` base used by `coeff_base_eob_ctx`; the
/// 4x4 `numCoeffs = TX_4X4_HEIGHT << TX_4X4_BWL = 16`, so the bands break at
/// `numCoeffs / 8 = 2` and `numCoeffs / 4 = 4`.
const NUM_COEFFS_4X4: usize = TX_4X4_HEIGHT << TX_4X4_BWL;
/// The § 8.3.2 `coeff_br` low-frequency luma context for the EOB coefficient at the
/// DC raster position 0 (eob == 1). In the reverse-scan base pass the EOB
/// coefficient is visited FIRST, so the running `Level[]` is empty when its
/// `coeff_br` context is derived; mirroring the decoder `CoeffBrContext::ctx`
/// (`crates/splot-decode/src/tile_payload/cdf/coeff_context.rs`, the `ctx` method)
/// with an all-zero `Level[]`, the neighbour sum `mag` is 0, `Min((0 + 1) >> 1, 6) =
/// 0`, and for `self.pos == 0` (the DC) the result is `mag == 0`.
const COEFF_BR_LF_CTX_EOB_DC: usize = 0;
/// The § 8.3.2 `coeff_br` low-frequency luma context for the EOB coefficient at a
/// non-zero raster position (any AC, eob `>= 2`). With the same empty `Level[]`
/// (`mag == 0`), the `self.is_lf` branch of `CoeffBrContext::ctx` yields
/// `mag + 7 == 7`.
const COEFF_BR_LF_CTX_EOB_AC: usize = 7;
/// The § 8.3.2 `coeff_br` HIGH-frequency luma context for the EOB coefficient at an
/// HF raster position (eob `== 11`, scan index 10, raster 13). The EOB coefficient is
/// visited FIRST in reverse scan, so the running `Level[]` is empty when its
/// `coeff_br` context is derived → neighbour sum `mag == 0`. The non-DC HF luma
/// `else { mag }` branch of the decoder `CoeffBrContext::ctx` (`is_lf == false`)
/// yields `mag == 0` — with NO `+7` offset (contrast [`COEFF_BR_LF_CTX_EOB_AC`]).
const HF_COEFF_BR_CTX_EOB: usize = 0;
/// The LF/HF boundary diagonal for a 4x4 luma 2D block: a coefficient at raster
/// `(row, col)` is low-frequency iff `row + col < LF_DIAGONAL_LIMIT_4X4` (`4`),
/// mirroring the decoder `get_lf_limits` for `TX_CLASS_2D` luma
/// (`crates/splot-decode/src/tile_payload/coeff_loop/max_level.rs`).
const LF_DIAGONAL_LIMIT_4X4: usize = 4;
/// The largest magnitude a HIGH-frequency luma coefficient (EOB or non-EOB) codes
/// with one HF `coeff_base`/`coeff_base_eob` and one HF `coeff_br` before the
/// § 5.20.7.28 `read_quant` golomb tail. The HF base-level threshold is the
/// decode-local `NUM_BASE_LEVELS` (`2`), NOT the low-frequency `LF_NUM_BASE_LEVELS`
/// (`4`): the HF base saturates at `NUM_BASE_LEVELS + 1 = 3` (a 4-entry HF CDF row, vs
/// the 6-entry LF row) and `coeff_br` (read when the level exceeds `NUM_BASE_LEVELS`)
/// adds `0..COEFF_BASE_RANGE`, so HF `maxLevel = NUM_BASE_LEVELS + COEFF_BASE_RANGE +
/// 1 = 6` and the largest no-golomb magnitude is `NUM_BASE_LEVELS + COEFF_BASE_RANGE =
/// 5` (mirroring the decoder `derive_coeff_max_level` HF branch). A higher-magnitude
/// HF coefficient (the golomb tail) is a later sub-brick and is rejected.
const MAX_HF_BASE_BR_MAGNITUDE: u32 = NUM_BASE_LEVELS + COEFF_BASE_RANGE;
/// The LF luma `maxLevel` (`LF_NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1 = 8`): the
/// level at which the § 5.20.7.28 `read_quant` golomb tail fires for a low-frequency
/// coefficient (with TCQ off; see [`general_walk_max_level_for_pos`]).
const LF_GOLOMB_MAX_LEVEL: u32 = MAX_BASE_BR_MAGNITUDE + 1;
/// The HF luma `maxLevel` (`NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1 = 6`): the level
/// at which the § 5.20.7.28 `read_quant` golomb tail fires for a high-frequency
/// coefficient.
const HF_GOLOMB_MAX_LEVEL: u32 = MAX_HF_BASE_BR_MAGNITUDE + 1;

/// Tokenizes an arbitrary 4x4 DCT_DCT luma `Quant[16]` block in the general walk
/// window (eob `<= 16`: the full 4x4 scan — the low-frequency region scan `0..=9`
/// plus the entire high-frequency tail scan `10..=15`) into the ordered AV2
/// § 5.20.7.27 block-symbol trace (luma coefficients followed by the all-zero chroma
/// U/V tail). EVERY low-frequency coefficient may have a base-range magnitude
/// `1..=MAX_BASE_BR_MAGNITUDE` (`7`, adding `coeff_br`); a high-frequency coefficient
/// (EOB or non-EOB) caps at `MAX_HF_BASE_BR_MAGNITUDE` (`5`). The HF EOB coefficient
/// uses the 4-symbol HF `coeff_base_eob` table; a non-EOB HF coefficient uses the
/// 4-symbol HF `coeff_base` table (`DEFAULT_COEFF_BASE_CDF`); both, if refined, use
/// the HF `coeff_br` table (see the module HF notes).
///
/// `quant` is the row-major (raster) signed quantized block; `coeff_cdf_q_ctx` is
/// the caller-resolved coefficient-CDF q-context. An all-zero block emits exactly
/// one luma `all_zero == 1` token (no chroma tail, mirroring an all-zero residual
/// block in this brick's scope). A coded block emits the full luma residual then
/// the all-zero chroma U/V `txb_skip`. Errors:
///
/// - [`Error::CoefficientTokenizationUnsupportedEob`] when a nonzero coefficient
///   sits at a scan index `> MAX_GENERAL_SCAN_INDEX` (`15`, i.e. eob `>= 17`, which is
///   impossible for a 4x4 block), and
/// - [`Error::CoefficientTokenizationUnsupportedMagnitude`] when a golomb-range
///   coefficient's § 5.20.7.28 extension `x = magnitude - maxLevel` exceeds the per-`m`
///   golomb cap [`golomb_x_max`] (so the golomb-prefix `length` would exceed `8`).
///   MULTIPLE golomb coefficients per block are supported: their golomb parameter `m`
///   varies as the running `hrLevelAvg` is threaded across them in reverse scan, so
///   the cap is per-coefficient.
///
/// # Preconditions
/// Assumes **TCQ is off** (`allow_tcq == 0`), as the minimal intra encoder path is
/// (`enable_tcq == false` in the sequence header). The § 5.20.7.28 `read_quant`
/// threshold is `level >= maxLevel - allow_tcq`; for low-frequency luma
/// `maxLevel == 8`, so with `allow_tcq == 0` a magnitude up to `7` (the EOB
/// `coeff_br` cap) carries **no** `read_quant` bypass tail. Under TCQ the threshold
/// drops to `7`, so a level-7 coefficient would need a `read_quant` tail and this
/// tokenizer's stream would desynchronize a TCQ-enabled decoder — that TCQ
/// interaction (and the general golomb tail for `maxLevel`+) is a deferred
/// sub-brick. Do not reuse this tokenizer on a TCQ-enabled block.
pub(crate) fn tokenize_general_lf_luma_block(
    quant: &[i32; TX_4X4_COEFF_COUNT],
    coeff_cdf_q_ctx: usize,
) -> Result<Vec<BlockSymbolToken>> {
    let scan = scan_2d_4x4()?;
    let eob = end_of_block(quant, &scan);

    if eob == 0 {
        let mut trace = Vec::new();
        trace
            .try_reserve_exact(1)
            .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
                context: "general LF all-zero block trace",
            })?;
        trace.push(BlockSymbolToken::Coeff(luma_all_zero_token(
            coeff_cdf_q_ctx,
        )));
        return Ok(trace);
    }

    validate_general_lf_scope(quant, &scan, eob)?;

    let base_pass = compose_base_pass(quant, &scan, eob, coeff_cdf_q_ctx)?;
    let sign_pass = compose_sign_pass(quant, &scan, eob, coeff_cdf_q_ctx)?;

    // An eobPt-`>=3` block (eob `>= 3`) carries an `eob_extra` CDF token after
    // `eob_pt_16`, followed by `eobPt - 3` `eob_extra_bit` bypass literals (0 for
    // eobPt 3, 1 for eobPt 4, 2 for eobPt 5). eobPt `< 3` (eob 1 or 2) carries none.
    let eob_pt = eob_pt_from_eob(eob);
    let has_eob_extra = eob >= MIN_EOB_WITH_EXTRA;
    let (eob_extra_flag, eob_extra_bits, eob_extra_width) = if has_eob_extra {
        eob_refinement(eob, eob_pt)
    } else {
        (false, 0, 0)
    };
    // Header: luma all_zero + eob_pt_16, plus (when refined) the `eob_extra` flag and
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

    // luma all_zero + eob_pt_16 [+ eob_extra + eob_extra_bits] + base pass + sign
    // pass + chroma U/V. The chroma tail MUST be in the reserved total so its `push`
    // calls stay inside the `try_reserve_exact` checked-allocation path.
    let total = base_pass
        .len()
        .checked_add(sign_pass.len())
        .and_then(|n| n.checked_add(header_len))
        .and_then(|n| n.checked_add(CHROMA_ALL_ZERO_TAIL_LEN))
        .ok_or(Error::CoefficientTokenizationAllocationFailed {
            context: "general LF coded block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
            context: "general LF coded block trace",
        })?;

    trace.push(BlockSymbolToken::Coeff(coded_luma_all_zero_token(
        coeff_cdf_q_ctx,
    )));
    trace.push(BlockSymbolToken::Coeff(eob_pt_16_token(
        coeff_cdf_q_ctx,
        EOB_CTX_LUMA_INTRA,
        eob_pt_16_symbol(eob),
    )));
    if has_eob_extra {
        // Emit the `eob_extra` CDF flag (the HIGH refinement bit), then the
        // `eob_extra_width` `eob_extra_bit` bypass literals (the LOW refinement bits)
        // MSB-first — the order the decoder reads them into `eob_extra_bits` via one
        // `read_literal(width)` (see the module `eob_extra_bit` BIT ORDER note and the
        // § 5.20.7.27 loop `for i = eobPt - 4; i >= 0; i--`). For eobPt 3 the width is
        // 0, so no bypass literal follows.
        trace.push(BlockSymbolToken::Coeff(eob_extra_token(
            coeff_cdf_q_ctx,
            eob_extra_flag,
        )));
        // MSB-first: bit `i = eob_extra_width - 1` (the spec's `i = eobPt - 4`) down
        // to bit 0. The first literal pushed is the most-significant `eob_extra_bit`.
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

/// Builds the AV2 2D scan order for the 4x4 DCT_DCT block
/// (`[0, 4, 1, 8, 5, 2, 12, 9, 6, 3, 13, 10, 7, 14, 11, 15]`). Shared with
/// [`super::general_walk_recover`].
pub(super) fn scan_2d_4x4() -> Result<[u16; TX_4X4_COEFF_COUNT]> {
    let mut scan = [0u16; TX_4X4_COEFF_COUNT];
    coefficient_scan_order(TX_4X4_WIDTH, TX_4X4_HEIGHT, TransformClass::TwoD, &mut scan).map_err(
        |_| Error::CoefficientTokenizationAllocationFailed {
            context: "general LF 4x4 2D scan order",
        },
    )?;
    Ok(scan)
}

/// Returns the fixed 4x4 visible block rectangle used for general-walk
/// diagnostics. The dimensions are nonzero, so `PlaneRect::new` cannot fail; the
/// error is mapped to a typed tokenization error rather than panicking.
fn lf_block_rect() -> Result<PlaneRect> {
    PlaneRect::new(0, 0, TX_4X4_WIDTH, TX_4X4_HEIGHT).map_err(|_| {
        Error::CoefficientTokenizationAllocationFailed {
            context: "general LF 4x4 block rectangle",
        }
    })
}

/// Returns `eob = (max scan index c with Quant[scan[c]] != 0) + 1`, or 0 for an
/// all-zero block.
fn end_of_block(quant: &[i32; TX_4X4_COEFF_COUNT], scan: &[u16; TX_4X4_COEFF_COUNT]) -> usize {
    let mut eob = 0;
    for (c, &raster) in scan.iter().enumerate() {
        if quant[raster as usize] != 0 {
            eob = c + 1;
        }
    }
    eob
}

/// Rejects any nonzero outside the supported window (scan indices
/// `0..=MAX_GENERAL_SCAN_INDEX`, eob `<= 16`) or magnitude tier. The window is the
/// FULL 4x4 scan: the whole low-frequency region (scan `0..=9`) plus the entire
/// high-frequency tail (scan `10..=15`, rasters 13, 10, 7, 14, 11, 15). BOTH the
/// end-of-block coefficient (scan index `eob - 1`, coded with `coeff_base_eob` +
/// optional `coeff_br`) and every non-EOB coefficient (coded with `coeff_base` +
/// optional `coeff_br`) may have any magnitude up to its position golomb cap: a
/// magnitude below its `maxLevel` (LF `8`, HF `6`) is the plain base+`coeff_br` tier;
/// a magnitude at-or-above its `maxLevel` is a § 5.20.7.28 `read_quant` GOLOMB
/// coefficient (`ENC-COEFF-GENERAL-WALK-GOLOMB`/`ENC-COEFF-GENERAL-WALK-GOLOMB-MULTI`).
///
/// MULTIPLE golomb coefficients per block are now supported (sub-brick 5e-ii): the
/// running `hrLevelAvg` predictor is threaded across them in reverse scan
/// (`c = eob-1 .. 0`), so each golomb coefficient's `m = Clip3(1, 6,
/// GetMsb(hrLevelAvg))` (and therefore its golomb cap) varies. This validation walks
/// the SAME reverse-scan order, deriving each golomb coefficient's `m` from the
/// running `hrLevelAvg`, then capping its golomb extension `x = magnitude - maxLevel`
/// at the per-`m` [`golomb_x_max`] (so the golomb-prefix `length` stays `<= 8` and the
/// § 8.2 self-consistency recovery can read it back) and updating `hrLevelAvg` with
/// the same `(x + hrLevelAvg) >> 1` formula the emission and recovery use. A larger
/// extension is rejected with
/// [`Error::CoefficientTokenizationUnsupportedMagnitude`] (reporting the per-position
/// per-`m` cap `max_level + golomb_x_max(m)`, which is `>= max_level`, so the rejected
/// magnitude always strictly exceeds the reported `max_magnitude`).
///
/// A nonzero at scan index `>= 16` (eob `>= 17`, impossible for a 4x4 block) is
/// rejected as before.
fn validate_general_lf_scope(
    quant: &[i32; TX_4X4_COEFF_COUNT],
    scan: &[u16; TX_4X4_COEFF_COUNT],
    eob: usize,
) -> Result<()> {
    // First reject any nonzero outside the supported scan window (forward scan, so the
    // smallest offending scan index is reported).
    for (c, &raster) in scan.iter().enumerate() {
        let value = quant[raster as usize];
        if value == 0 {
            continue;
        }
        if c > MAX_GENERAL_SCAN_INDEX {
            return Err(Error::CoefficientTokenizationUnsupportedEob {
                scan_index: c,
                position: raster as usize,
                value,
                max_scan_index: MAX_GENERAL_SCAN_INDEX,
            });
        }
    }

    // Thread the running `hrLevelAvg` across the golomb coefficients in reverse scan
    // (`c = eob-1 .. 0`), the SAME order `compose_sign_pass` and `recover_quant_from_tokens`
    // walk. Each golomb coefficient's `m` (and thus its extension cap) is derived from
    // the `hrLevelAvg` entering it; `hrLevelAvg` updates by `(x + hrLevelAvg) >> 1`.
    let mut hr_level_avg = 0u32;
    for offset in 0..eob {
        let c = eob - 1 - offset;
        let pos = scan_pos(scan, c)?;
        let value = quant[pos];
        if value == 0 {
            continue;
        }
        let magnitude = value.unsigned_abs();
        let max_level = general_walk_max_level_for_pos(pos);
        if magnitude < max_level {
            continue;
        }
        // A § 5.20.7.28 `read_quant` golomb coefficient. Derive its `m` from the
        // running `hrLevelAvg`, cap its golomb extension `x = magnitude - maxLevel` at
        // the per-`m` cap, then thread `hrLevelAvg` to the next golomb coefficient.
        let params = golomb_params_from_hr_level_avg(hr_level_avg);
        let x = magnitude - max_level;
        let x_max = golomb_x_max(params);
        if x > x_max {
            return Err(Error::CoefficientTokenizationUnsupportedMagnitude {
                plane: PLANE_Y,
                block: lf_block_rect()?,
                coefficient_index: pos,
                magnitude,
                max_magnitude: max_level + x_max,
            });
        }
        hr_level_avg = next_hr_level_avg(x, hr_level_avg);
    }
    debug_assert!((1..=MAX_GENERAL_SCAN_INDEX + 1).contains(&eob));
    Ok(())
}

/// Returns whether the 4x4 luma 2D coefficient at raster position `pos` is in the
/// low-frequency region (`row + col < LF_DIAGONAL_LIMIT_4X4`, i.e. `< 4`), mirroring
/// the decoder `get_lf_limits` for `TX_CLASS_2D` luma. For the 4x4 2D scan order,
/// scan indices `0..=9` are LF and scan index `10` (raster 13 = row 3, col 1) is the
/// first HF coefficient.
const fn is_lf_position(pos: usize) -> bool {
    let row = pos >> TX_4X4_BWL;
    let col = pos - (row << TX_4X4_BWL);
    row + col < LF_DIAGONAL_LIMIT_4X4
}

/// Returns the AV2 § 5.20.7.27/§ 5.20.7.28 `maxLevel` for the 4x4 luma 2D coefficient
/// at raster position `pos` — the level at which `read_quant` fires (with TCQ off, so
/// `read_quant` is invoked when `level >= maxLevel`). A low-frequency coefficient
/// saturates its base+`coeff_br` at `MAX_BASE_BR_MAGNITUDE` (`7`), so its `maxLevel`
/// is `8` ([`LF_GOLOMB_MAX_LEVEL`]); a high-frequency coefficient saturates at
/// `MAX_HF_BASE_BR_MAGNITUDE` (`5`), so its `maxLevel` is `6` ([`HF_GOLOMB_MAX_LEVEL`]).
/// Mirrors the decoder `derive_coeff_max_level` per region. Shared with
/// [`super::general_walk_recover`].
pub(super) const fn general_walk_max_level_for_pos(pos: usize) -> u32 {
    if is_lf_position(pos) {
        LF_GOLOMB_MAX_LEVEL
    } else {
        HF_GOLOMB_MAX_LEVEL
    }
}

/// Composes the reverse-scan base pass over `c = eob - 1 .. 0` using a running
/// `Level[]` for the § 8.3.2 luma `coeff_base` / `coeff_br` contexts of the non-EOB
/// coefficients. The EOB coefficient (visited first) emits its `coeff_base_eob` and,
/// when its magnitude exceeds its position base-level threshold (`LF_NUM_BASE_LEVELS`
/// low-frequency, `NUM_BASE_LEVELS` high-frequency), an interleaved `coeff_br` at the
/// constant empty-`Level[]` context. Each non-EOB coefficient emits its `coeff_base`
/// and, when its magnitude exceeds that same per-region threshold, an interleaved
/// `coeff_br` whose context is derived from the running `Level[]` (the
/// already-written neighbours).
///
/// LF/HF SELECTION: each coefficient's low-frequency predicate is derived from its
/// OWN raster `row + col < LF_DIAGONAL_LIMIT_4X4` (`4`), per the decoder
/// `get_lf_limits` for `TX_CLASS_2D` luma — NOT a scan-index threshold. A
/// low-frequency coefficient (scan `0..=9`) emits the 6-symbol LF
/// `coeff_base`/`coeff_base_eob` (LF context, cap `LF_NUM_BASE_LEVELS`) and, if
/// refined, the LF `coeff_br` (`is_lf = true`). A high-frequency coefficient (scan
/// `10..=15`) emits the 4-symbol HF table — `coeff_base_hf_eob_token` for the EOB
/// coefficient (constant ctx `0` `coeff_br`), `coeff_base_hf_token` for a non-EOB one
/// (the `coeff_base_hf_luma_context` band, cap `NUM_BASE_LEVELS`) — and, if refined,
/// the HF `coeff_br` (`is_lf = false`, the no-`+7` branch). The per-position
/// selection therefore handles a mixed eob-12..=16 block where the EOB coefficient
/// and one or more non-EOB coefficients are all high-frequency.
fn compose_base_pass(
    quant: &[i32; TX_4X4_COEFF_COUNT],
    scan: &[u16; TX_4X4_COEFF_COUNT],
    eob: usize,
    coeff_cdf_q_ctx: usize,
) -> Result<Vec<CoefficientEntropyToken>> {
    let mut tokens = Vec::new();
    // One `coeff_base`/`coeff_base_eob` token per coefficient, plus at most one
    // `coeff_br` per coefficient (both coefficients may now reach `coeff_br`).
    let capacity = eob
        .checked_mul(2)
        .ok_or(Error::CoefficientTokenizationAllocationFailed {
            context: "general LF base pass token capacity",
        })?;
    tokens.try_reserve_exact(capacity).map_err(|_| {
        Error::CoefficientTokenizationAllocationFailed {
            context: "general LF base pass tokens",
        }
    })?;
    let mut level = [0u32; TX_4X4_COEFF_COUNT];

    for offset in 0..eob {
        let c = eob - 1 - offset;
        let pos = scan_pos(scan, c)?;
        let magnitude = quant[pos].unsigned_abs();
        if offset == 0 {
            // EOB coefficient: `coeff_base_eob`; `level = coeff_base_eob + 1`. The base
            // level saturates at `base_levels + 1` and `coeff_br` is read when the level
            // exceeds `base_levels`. The base-level threshold differs by region: LF uses
            // `LF_NUM_BASE_LEVELS` (4; saturate at 5, a 5-symbol CDF), HF uses the
            // decode-local `NUM_BASE_LEVELS` (2; saturate at 3, a 3-symbol CDF) —
            // mirroring the decoder `derive_base_symbol_input` `base_levels` selection.
            if is_lf_position(pos) {
                // LF EOB coefficient: 6-entry (5-symbol) `coeff_base_eob` + LF `coeff_br`
                // (ctx 0 at the DC raster position, else ctx 7 — the empty-`Level[]` LF
                // band).
                let eob_level = magnitude.min(LF_NUM_BASE_LEVELS + 1) as u8;
                tokens.push(coeff_base_lf_eob_token(
                    coeff_cdf_q_ctx,
                    coeff_base_eob_ctx(c),
                    eob_level,
                ));
                if magnitude > LF_NUM_BASE_LEVELS {
                    // Saturate the base+`coeff_br` level at the LF `maxLevel` 8: a golomb
                    // coefficient (magnitude `>= maxLevel`) pins `coeff_br` to its max
                    // symbol `COEFF_BASE_RANGE` and codes the remainder in the
                    // § 5.20.7.28 golomb tail (appended in the sign pass).
                    let br_symbol =
                        (magnitude - (LF_NUM_BASE_LEVELS + 1)).min(COEFF_BASE_RANGE) as u8;
                    tokens.push(coeff_br_lf_token(
                        coeff_cdf_q_ctx,
                        eob_coeff_br_ctx(pos),
                        br_symbol,
                    ));
                }
            } else {
                // HF EOB coefficient (eob 11, scan index 10, raster 13): the 4-entry
                // (3-symbol) HF `coeff_base_eob` (`DEFAULT_COEFF_BASE_EOB_CDF`) at the
                // SHARED scan-band context, and — when `coeff_br` applies — the HF
                // `coeff_br` (`DEFAULT_COEFF_BR_CDF`) at the constant empty-`Level[]`
                // context 0 (the non-DC HF `else { mag }` branch with `mag == 0`, NO
                // `+7`). The level saturates at `NUM_BASE_LEVELS + 1 = 3`.
                let eob_level = magnitude.min(NUM_BASE_LEVELS + 1) as u8;
                tokens.push(coeff_base_hf_eob_token(
                    coeff_cdf_q_ctx,
                    coeff_base_eob_ctx(c),
                    eob_level,
                ));
                if magnitude > NUM_BASE_LEVELS {
                    // Saturate the base+`coeff_br` level at the HF `maxLevel` 6: a golomb
                    // coefficient pins `coeff_br` to its max symbol `COEFF_BASE_RANGE`
                    // and codes the remainder in the golomb tail (sign pass).
                    let br_symbol = (magnitude - (NUM_BASE_LEVELS + 1)).min(COEFF_BASE_RANGE) as u8;
                    tokens.push(coeff_br_hf_token(
                        coeff_cdf_q_ctx,
                        HF_COEFF_BR_CTX_EOB,
                        br_symbol,
                    ));
                }
            }
        } else if is_lf_position(pos) {
            // Non-EOB LOW-frequency coefficient: the § 8.3.2 LF luma `coeff_base`
            // context derived from the partially-built `Level[]` (the AC neighbour is
            // already written). A non-EOB `coeff_base` symbol is `min(mag,
            // LF_NUM_BASE_LEVELS + 1)` (NOT minus one); a zero coefficient emits symbol
            // 0 and no sign. The base level saturates at `LF_NUM_BASE_LEVELS + 1` (a
            // 6-symbol CDF) and `coeff_br` refines when the magnitude exceeds
            // `LF_NUM_BASE_LEVELS`.
            let ctx = coeff_base_lf_luma_context(
                pos,
                TX_4X4_BWL,
                TX_4X4_WIDTH,
                TX_4X4_HEIGHT,
                TRANSFORM_CLASS_2D,
                c,
                &level,
            );
            let base_symbol = magnitude.min(LF_NUM_BASE_LEVELS + 1) as u8;
            tokens.push(coeff_base_lf_token(
                coeff_cdf_q_ctx,
                ctx,
                COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
                base_symbol,
            ));
            if magnitude > LF_NUM_BASE_LEVELS {
                // `coeff_br` refines the non-EOB LF level: symbol = mag -
                // (LF_NUM_BASE_LEVELS + 1). Its context is data-dependent — derived
                // from the running `Level[]` (the already-written neighbour) via
                // `coeff_br_lf_luma_context`, mirroring the decoder `CoeffBrContext`.
                // Emitted BEFORE the `Level[pos]` write below, exactly like the EOB.
                // The `is_lf` predicate (derived from this coefficient's own raster) is
                // `true` here, so the LF token constructor routes the LF `coeff_br`
                // table.
                let br_ctx = coeff_br_lf_luma_context(
                    pos,
                    TX_4X4_BWL,
                    TX_4X4_WIDTH,
                    TX_4X4_HEIGHT,
                    TRANSFORM_CLASS_2D,
                    true,
                    &level,
                );
                // Saturate at the LF `maxLevel` 8: a golomb coefficient pins the
                // `coeff_br` symbol to `COEFF_BASE_RANGE` and codes the rest in the
                // golomb tail (sign pass).
                let br_symbol = (magnitude - (LF_NUM_BASE_LEVELS + 1)).min(COEFF_BASE_RANGE) as u8;
                tokens.push(coeff_br_lf_token(coeff_cdf_q_ctx, br_ctx, br_symbol));
            }
        } else {
            // Non-EOB HIGH-frequency coefficient (eob 12..=16, scan index 11..=15): the
            // § 8.3.2 HF luma `coeff_base` context derived from the partially-built
            // `Level[]` via `coeff_base_hf_luma_context` (NO near-DC `magLimit = 5`
            // carve-out, NO DC band; see the decoder `CoeffBaseContext::select`
            // `is_lf == false` branch). The HF base level saturates at `NUM_BASE_LEVELS
            // + 1 == 3` (a 4-symbol CDF) — NOT the LF `LF_NUM_BASE_LEVELS + 1 == 5` —
            // and `coeff_br` (the HF `coeff_br`, no `+7`) refines when the magnitude
            // exceeds `NUM_BASE_LEVELS`. A non-EOB `coeff_base` symbol equals the level
            // (NOT minus one); a zero coefficient emits symbol 0 and no sign.
            let ctx = coeff_base_hf_luma_context(
                pos,
                TX_4X4_BWL,
                TX_4X4_WIDTH,
                TX_4X4_HEIGHT,
                TRANSFORM_CLASS_2D,
                &level,
            );
            let base_symbol = magnitude.min(NUM_BASE_LEVELS + 1) as u8;
            tokens.push(coeff_base_hf_token(
                coeff_cdf_q_ctx,
                ctx,
                COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
                base_symbol,
            ));
            if magnitude > NUM_BASE_LEVELS {
                // The HF `coeff_br` refines the non-EOB HF level: symbol = mag -
                // (NUM_BASE_LEVELS + 1). Its context is data-dependent — derived from
                // the running `Level[]` via `coeff_br_lf_luma_context` with `is_lf =
                // false`, which takes the decoder final `else { mag }` branch (plain
                // `mag`, NO `+7`). Emitted BEFORE the `Level[pos]` write below.
                let br_ctx = coeff_br_lf_luma_context(
                    pos,
                    TX_4X4_BWL,
                    TX_4X4_WIDTH,
                    TX_4X4_HEIGHT,
                    TRANSFORM_CLASS_2D,
                    false,
                    &level,
                );
                // Saturate at the HF `maxLevel` 6: a golomb coefficient pins the
                // `coeff_br` symbol to `COEFF_BASE_RANGE` and codes the rest in the
                // golomb tail (sign pass).
                let br_symbol = (magnitude - (NUM_BASE_LEVELS + 1)).min(COEFF_BASE_RANGE) as u8;
                tokens.push(coeff_br_hf_token(coeff_cdf_q_ctx, br_ctx, br_symbol));
            }
        }
        // Write `Level[pos] = mag` before deriving the next (lower-c) context.
        level[pos] = magnitude;
    }
    Ok(tokens)
}

/// Composes the reverse-scan, interleaved sign+quant pass over `c = eob - 1 .. 0`: a
/// `dc_sign` CDF token for the DC at raster position 0, a `sign_bit` bypass for
/// every other coefficient, no sign for a zero coefficient, and — for a coefficient
/// whose magnitude reaches its position `maxLevel` — the § 5.20.7.28 `read_quant`
/// golomb tail emitted RIGHT AFTER its sign token (§ 5.20.7.27 reads sign then
/// `read_quant` per coefficient). The base pass already saturated the
/// base+`coeff_br` level to `maxLevel`; the golomb tail codes the extension
/// `x = magnitude - maxLevel`.
///
/// MULTIPLE golomb coefficients per block are supported (sub-brick 5e-ii): the running
/// `hrLevelAvg` predictor (init `0`) is threaded across the golomb coefficients in
/// this reverse-scan order. Each golomb coefficient derives its golomb parameters
/// (`m = Clip3(1, 6, GetMsb(hrLevelAvg))`, `k`, `cMax`) from the `hrLevelAvg` entering
/// it, emits its tail with those parameters, then updates
/// `hrLevelAvg = (x + hrLevelAvg) >> 1` (the decoder's `lvlShift == 0` formula). The
/// FIRST golomb coefficient sees `hrLevelAvg == 0` → `m == 1` (byte-identical to the
/// original single-golomb sub-brick 5e tail).
fn compose_sign_pass(
    quant: &[i32; TX_4X4_COEFF_COUNT],
    scan: &[u16; TX_4X4_COEFF_COUNT],
    eob: usize,
    coeff_cdf_q_ctx: usize,
) -> Result<Vec<BlockSymbolToken>> {
    // Reserve one sign token per nonzero coefficient plus, for every golomb
    // coefficient, its golomb-tail bypass literals. The reservation threads the SAME
    // running `hrLevelAvg` as the emission loop so each tail length is computed with
    // the same per-coefficient golomb parameters — an exact upper bound.
    let mut reserve = eob;
    let mut hr_level_avg = 0u32;
    for offset in 0..eob {
        let c = eob - 1 - offset;
        let pos = scan_pos(scan, c)?;
        let magnitude = quant[pos].unsigned_abs();
        let max_level = general_walk_max_level_for_pos(pos);
        if magnitude >= max_level {
            let x = magnitude - max_level;
            let params = golomb_params_from_hr_level_avg(hr_level_avg);
            reserve = reserve
                .checked_add(read_quant_golomb_tail_len(x, params))
                .ok_or(Error::CoefficientTokenizationAllocationFailed {
                    context: "general LF sign pass golomb reservation",
                })?;
            hr_level_avg = next_hr_level_avg(x, hr_level_avg);
        }
    }
    let mut tokens = Vec::new();
    tokens.try_reserve_exact(reserve).map_err(|_| {
        Error::CoefficientTokenizationAllocationFailed {
            context: "general LF sign pass tokens",
        }
    })?;

    // Reset the predictor for the emission loop (it walks the same reverse-scan order).
    let mut hr_level_avg = 0u32;
    for offset in 0..eob {
        let c = eob - 1 - offset;
        let pos = scan_pos(scan, c)?;
        let value = quant[pos];
        if value == 0 {
            continue;
        }
        let negative = value < 0;
        let row = pos >> TX_4X4_BWL;
        let col = pos - (row << TX_4X4_BWL);
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
        // § 5.20.7.27: the sign+quant pass reads the sign then `read_quant`. A
        // coefficient whose magnitude reached its position `maxLevel` carries the
        // § 5.20.7.28 golomb tail (the extension `x = magnitude - maxLevel`) right
        // after its sign token. The golomb parameters come from the running
        // `hrLevelAvg`, which then updates for the next golomb coefficient.
        let magnitude = value.unsigned_abs();
        let max_level = general_walk_max_level_for_pos(pos);
        if magnitude >= max_level {
            let x = magnitude - max_level;
            let params = golomb_params_from_hr_level_avg(hr_level_avg);
            push_read_quant_golomb_tail(&mut tokens, x, params)?;
            hr_level_avg = next_hr_level_avg(x, hr_level_avg);
        }
    }
    Ok(tokens)
}

/// Returns the AV2 § 8.3.2 `coeff_base_eob` context for a 4x4 luma EOB coefficient
/// at scan index `c`: `c == 0 -> 0`, `c <= numCoeffs/8 (2) -> 1`,
/// `c <= numCoeffs/4 (4) -> 2`, otherwise `3`.
const fn coeff_base_eob_ctx(c: usize) -> usize {
    if c == 0 {
        0
    } else if c <= NUM_COEFFS_4X4 / 8 {
        1
    } else if c <= NUM_COEFFS_4X4 / 4 {
        2
    } else {
        3
    }
}

/// Returns the AV2 § 5.20.7.27 `eobPt` for an eob in the brick window (`1..=16`).
/// `eobPt` is the inverse of the decoder base
/// `(eobPt < 2) ? eobPt : (1 << (eobPt - 2)) + 1`: eob 1 → 1, eob 2 → 2,
/// eob 3..=4 → 3 (base 3), eob 5..=8 → 4 (base 5), eob 9..=16 → 5 (base 9; eobPt 5
/// spans eob 9..=16). It is `const` and total over the brick's `1..=16` window; an
/// eob outside it is rejected upstream by [`validate_general_lf_scope`].
const fn eob_pt_from_eob(eob: usize) -> usize {
    if eob < 2 {
        1
    } else if eob <= 2 {
        2
    } else if eob <= 4 {
        3
    } else if eob <= 8 {
        4
    } else {
        MAX_GENERAL_EOB_PT
    }
}

/// Returns the decoder eob base for `eobPt`: `(eobPt < 2) ? eobPt :
/// (1 << (eobPt - 2)) + 1` (eobPt 1 → 1, 2 → 2, 3 → 3, 4 → 5, 5 → 9). Mirrors
/// `nonzero_coeff_eob` (`crates/splot-decode/src/tile_payload/coeff_loop.rs`).
/// Shared with [`super::general_walk_recover`].
pub(super) const fn eob_base_for_pt(eob_pt: usize) -> usize {
    if eob_pt < 2 {
        eob_pt
    } else {
        (1 << (eob_pt - 2)) + 1
    }
}

/// Returns the `eob_pt_16` symbol (`eobPt - 1`) for an eob in the brick window
/// (`eob <= 16`). The `eob_pt_16` symbol carries `eobPt - 1`, NOT `eob - 1`.
const fn eob_pt_16_symbol(eob: usize) -> u8 {
    (eob_pt_from_eob(eob) - 1) as u8
}

/// Derives the § 5.20.7.27 `(eob_extra, eob_extra_bits)` refinement for an eob whose
/// `eobPt >= 3`. With `base = eob_base_for_pt(eobPt)`, `offset = eob - base`,
/// `width = eobPt - 3`: `eob_extra` is the HIGH bit `(offset >> width) & 1` and
/// `eob_extra_bits` is the LOW `width` bits `offset & ((1 << width) - 1)`. This is
/// the exact inverse of the decoder `eob = base + (eob_extra << width) +
/// eob_extra_bits`. Returns `(eob_extra_flag, eob_extra_bits, width)`.
const fn eob_refinement(eob: usize, eob_pt: usize) -> (bool, u32, u32) {
    let base = eob_base_for_pt(eob_pt);
    let offset = eob - base;
    let width = (eob_pt - 3) as u32;
    let eob_extra = (offset >> width) & 1 != 0;
    let eob_extra_bits = (offset & ((1usize << width) - 1)) as u32;
    (eob_extra, eob_extra_bits, width)
}

/// Returns the constant § 8.3.2 `coeff_br` low-frequency luma context for the EOB
/// coefficient at raster position `pos`. Because the EOB coefficient is visited
/// first in reverse scan, the running `Level[]` is empty when this context is
/// derived, so the decoder `CoeffBrContext::ctx` reduces to a constant: ctx 0 at the
/// DC raster position 0, ctx 7 at any non-DC low-frequency position (see
/// [`COEFF_BR_LF_CTX_EOB_DC`] / [`COEFF_BR_LF_CTX_EOB_AC`]).
const fn eob_coeff_br_ctx(pos: usize) -> usize {
    if pos == 0 {
        COEFF_BR_LF_CTX_EOB_DC
    } else {
        COEFF_BR_LF_CTX_EOB_AC
    }
}

/// Returns `scan[c]` as a raster position, validating the scan index. Shared with
/// [`super::general_walk_recover`].
pub(super) fn scan_pos(scan: &[u16; TX_4X4_COEFF_COUNT], c: usize) -> Result<usize> {
    scan.get(c).map(|&raster| raster as usize).ok_or(
        Error::CoefficientTokenizationAllocationFailed {
            context: "general LF scan index out of range",
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
