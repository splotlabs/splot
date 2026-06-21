// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.7.27 GENERAL coefficient-tokenization walk for the low-frequency
//! base tier (`ENC-COEFF-GENERAL-WALK-LF-BASE`).
//!
//! This walks an arbitrary quantized 4x4 DCT_DCT luma `Quant[16]` block whose
//! nonzero coefficients all sit in the low-frequency end-of-block window (scan
//! indices `0..=1`, eob `<= 2`) with base-tier magnitudes (`1..=4`), and emits the
//! ordered § 5.20.7.27 coefficient token stream the decoder coefficient loop reads:
//! the luma `txb_skip`, `eob_pt_16`, the reverse-scan `coeff_base_eob` /
//! `coeff_base` base pass (with the running-`Level[]` § 8.3.2 LF luma context from
//! [`super::coeff_base_lf_luma_context`]), the reverse-scan interleaved sign pass
//! (`dc_sign` CDF for the DC, `sign_bit` § 8.2.5 bypass for the AC), and the
//! all-zero chroma U/V `txb_skip` tail. It reuses the existing token constructors
//! and CDF routing; it never invents AV2 CDF values or contexts.
//!
//! Anything outside that window — a nonzero at scan index `>= 2`, or a magnitude
//! `> 4` (the base tier, no `coeff_br` / golomb) — is rejected with a typed error.
//!
//! HONESTY: the [`recover_quant_from_tokens`] proof is § 8.2 SELF-CONSISTENCY. The
//! same code authored the emission and its inverse, so it proves the encoder's
//! emitted (level, sign, position) triples are internally reversible — with
//! asymmetric values it catches a swapped sign order (AC-before-DC) or a
//! level/position transposition. It does NOT validate the § 8.3.2 CDF contexts
//! against a real decoder; context conformance is deferred to the splot-decode
//! cross-check brick.

use splot_recon::{PlaneId, PlaneRect, TransformClass, coefficient_scan_order};

use super::{
    CoefficientEntropyToken, EOB_CTX_LUMA_INTRA, LF_NUM_BASE_LEVELS, MAX_BASE_EOB_MAGNITUDE,
    chroma_u_all_zero_token, chroma_v_all_zero_token, coded_luma_all_zero_token,
    coeff_base_lf_eob_token, coeff_base_lf_luma_context, coeff_base_lf_token, eob_pt_16_token,
    luma_all_zero_token, luma_dc_sign_token,
};
use crate::block_symbol_trace::BlockSymbolToken;
use crate::error::{Error, Result};

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
/// The largest nonzero scan index this brick covers (eob `<= 2`).
const MAX_GENERAL_LF_SCAN_INDEX: usize = 1;
/// The neutral V-plane `txb_skip` context for an all-zero U/V tail (no `EobU`).
const V_TXB_SKIP_CTX_NEUTRAL: usize = 0;
/// AV2 § 8.3.2 `SIG_COEF_CONTEXTS_EOB` base used by `coeff_base_eob_ctx`; the
/// 4x4 `numCoeffs = TX_4X4_HEIGHT << TX_4X4_BWL = 16`, so the bands break at
/// `numCoeffs / 8 = 2` and `numCoeffs / 4 = 4`.
const NUM_COEFFS_4X4: usize = TX_4X4_HEIGHT << TX_4X4_BWL;

/// Tokenizes an arbitrary 4x4 DCT_DCT luma `Quant[16]` block in the general
/// low-frequency base tier (eob `<= 2`, magnitudes `1..=4`) into the ordered AV2
/// § 5.20.7.27 block-symbol trace (luma coefficients followed by the all-zero
/// chroma U/V tail).
///
/// `quant` is the row-major (raster) signed quantized block; `coeff_cdf_q_ctx` is
/// the caller-resolved coefficient-CDF q-context. An all-zero block emits exactly
/// one luma `all_zero == 1` token (no chroma tail, mirroring an all-zero residual
/// block in this brick's scope). A coded block emits the full luma residual then
/// the all-zero chroma U/V `txb_skip`. Errors:
///
/// - [`Error::CoefficientTokenizationUnsupportedEob`] when a nonzero coefficient
///   sits at a scan index `> MAX_GENERAL_LF_SCAN_INDEX`, and
/// - [`Error::CoefficientTokenizationUnsupportedMagnitude`] when a coefficient
///   magnitude exceeds the base tier (`MAX_BASE_EOB_MAGNITUDE`).
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

    // luma all_zero + eob_pt_16 + base pass + sign pass + chroma U + chroma V.
    let total = base_pass
        .len()
        .checked_add(sign_pass.len())
        .and_then(|n| n.checked_add(4))
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

/// Re-reads the emitted token stream in the same reverse-scan order and rebuilds
/// the signed `[i32; 16]` raster block, proving the encoder's emitted
/// (level, sign, position) triples are internally reversible.
///
/// This is § 8.2 self-consistency, not decoder/AVM verification: the same code
/// authored the emission and this inverse. It walks the trace's base-pass and
/// sign-pass tokens (skipping the `all_zero` / `eob_pt_16` / chroma tail), pairs
/// each base level with its reverse-scan sign, and writes the signed value at the
/// scan-derived raster position. An all-zero trace (single `all_zero == 1`)
/// recovers the zero block.
pub(crate) fn recover_quant_from_tokens(
    tokens: &[BlockSymbolToken],
    coeff_cdf_q_ctx: usize,
) -> Result<[i32; TX_4X4_COEFF_COUNT]> {
    let scan = scan_2d_4x4()?;
    let mut quant = [0i32; TX_4X4_COEFF_COUNT];

    // An all-zero block trace is the single luma `all_zero == 1` token.
    if tokens.len() == 1 {
        return Ok(quant);
    }

    // Locate the `eob_pt_16` token to recover eob, then walk the base pass
    // (`eob` coefficient tokens) and the interleaved sign pass that follows it.
    let mut index = 0usize;
    skip_expected_all_zero(tokens, &mut index, coeff_cdf_q_ctx)?;
    let eob = read_eob_from_tokens(tokens, &mut index)?;

    // Base pass: `eob` reverse-scan coefficients, levels only.
    let mut levels = [0u32; TX_4X4_COEFF_COUNT];
    for offset in 0..eob {
        let c = eob - 1 - offset;
        let pos = scan_pos(&scan, c)?;
        let token = coeff_token_at(tokens, &mut index)?;
        let level = recover_base_level(token, offset);
        levels[pos] = level;
    }

    // Sign pass: reverse-scan, interleaved per nonzero coefficient.
    for offset in 0..eob {
        let c = eob - 1 - offset;
        let pos = scan_pos(&scan, c)?;
        let level = levels[pos];
        if level == 0 {
            continue;
        }
        let negative = read_sign_from_tokens(tokens, &mut index)?;
        let signed = if negative {
            -(level as i32)
        } else {
            level as i32
        };
        quant[pos] = signed;
    }

    Ok(quant)
}

/// Builds the AV2 2D scan order for the 4x4 DCT_DCT block
/// (`[0, 4, 1, 8, 5, 2, 12, 9, 6, 3, 13, 10, 7, 14, 11, 15]`).
fn scan_2d_4x4() -> Result<[u16; TX_4X4_COEFF_COUNT]> {
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

/// Rejects any nonzero outside the supported low-frequency window or base tier.
fn validate_general_lf_scope(
    quant: &[i32; TX_4X4_COEFF_COUNT],
    scan: &[u16; TX_4X4_COEFF_COUNT],
    eob: usize,
) -> Result<()> {
    for (c, &raster) in scan.iter().enumerate() {
        let value = quant[raster as usize];
        if value == 0 {
            continue;
        }
        if c > MAX_GENERAL_LF_SCAN_INDEX {
            return Err(Error::CoefficientTokenizationUnsupportedEob {
                scan_index: c,
                position: raster as usize,
                value,
                max_scan_index: MAX_GENERAL_LF_SCAN_INDEX,
            });
        }
        let magnitude = value.unsigned_abs();
        if magnitude > MAX_BASE_EOB_MAGNITUDE {
            return Err(Error::CoefficientTokenizationUnsupportedMagnitude {
                plane: PLANE_Y,
                block: lf_block_rect()?,
                coefficient_index: raster as usize,
                magnitude,
                max_magnitude: MAX_BASE_EOB_MAGNITUDE,
            });
        }
    }
    debug_assert!((1..=MAX_GENERAL_LF_SCAN_INDEX + 1).contains(&eob));
    Ok(())
}

/// Composes the reverse-scan base pass over `c = eob - 1 .. 0` using a running
/// `Level[]` for the § 8.3.2 LF luma `coeff_base` context of the non-EOB
/// coefficients.
fn compose_base_pass(
    quant: &[i32; TX_4X4_COEFF_COUNT],
    scan: &[u16; TX_4X4_COEFF_COUNT],
    eob: usize,
    coeff_cdf_q_ctx: usize,
) -> Result<Vec<CoefficientEntropyToken>> {
    let mut tokens = Vec::new();
    tokens
        .try_reserve_exact(eob)
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
            context: "general LF base pass tokens",
        })?;
    let mut level = [0u32; TX_4X4_COEFF_COUNT];

    for offset in 0..eob {
        let c = eob - 1 - offset;
        let pos = scan_pos(scan, c)?;
        let magnitude = quant[pos].unsigned_abs();
        if offset == 0 {
            // EOB coefficient: `coeff_base_eob`; `level = coeff_base_eob + 1`, so the
            // token level is `min(mag, LF_NUM_BASE_LEVELS + 1)` (== mag for mag <= 4).
            let eob_level = magnitude.min(LF_NUM_BASE_LEVELS + 1) as u8;
            tokens.push(coeff_base_lf_eob_token(
                coeff_cdf_q_ctx,
                coeff_base_eob_ctx(c),
                eob_level,
            ));
        } else {
            // Non-EOB coefficient: the § 8.3.2 LF luma `coeff_base` context derived
            // from the partially-built `Level[]` (the AC neighbour is already
            // written). A non-EOB `coeff_base` symbol is `min(mag, LF_NUM_BASE_LEVELS
            // + 1)` (NOT minus one); a zero coefficient emits symbol 0 and no sign.
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
        }
        // Write `Level[pos] = mag` before deriving the next (lower-c) context.
        level[pos] = magnitude;
    }
    Ok(tokens)
}

/// Composes the reverse-scan, interleaved sign pass over `c = eob - 1 .. 0`: a
/// `dc_sign` CDF token for the DC at raster position 0, a `sign_bit` bypass for
/// every other coefficient, and no sign for a zero coefficient.
fn compose_sign_pass(
    quant: &[i32; TX_4X4_COEFF_COUNT],
    scan: &[u16; TX_4X4_COEFF_COUNT],
    eob: usize,
    coeff_cdf_q_ctx: usize,
) -> Result<Vec<BlockSymbolToken>> {
    let mut tokens = Vec::new();
    tokens
        .try_reserve_exact(eob)
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
            context: "general LF sign pass tokens",
        })?;

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

/// Returns the `eob_pt_16` symbol for a low-frequency eob (`eob <= 2`): symbol
/// `eob - 1` (eobPt `eob`, no extra bits for eobPt `< 3`).
const fn eob_pt_16_symbol(eob: usize) -> u8 {
    (eob - 1) as u8
}

/// Returns `scan[c]` as a raster position, validating the scan index.
fn scan_pos(scan: &[u16; TX_4X4_COEFF_COUNT], c: usize) -> Result<usize> {
    scan.get(c).map(|&raster| raster as usize).ok_or(
        Error::CoefficientTokenizationAllocationFailed {
            context: "general LF scan index out of range",
        },
    )
}

/// Skips the leading coded luma `all_zero == 0` token during recovery, asserting
/// it is present.
fn skip_expected_all_zero(
    tokens: &[BlockSymbolToken],
    index: &mut usize,
    coeff_cdf_q_ctx: usize,
) -> Result<()> {
    let token = coeff_token_at(tokens, index)?;
    let expected = coded_luma_all_zero_token(coeff_cdf_q_ctx);
    if token != expected {
        return Err(Error::CoefficientTokenizationAllocationFailed {
            context: "general LF recovery expected coded all_zero token",
        });
    }
    Ok(())
}

/// Reads the eob from the `eob_pt_16` token at the cursor (`eob = symbol + 1`).
fn read_eob_from_tokens(tokens: &[BlockSymbolToken], index: &mut usize) -> Result<usize> {
    let token = coeff_token_at(tokens, index)?;
    Ok(usize::from(token.symbol()) + 1)
}

/// Recovers the base level of one base-pass coefficient: the EOB coefficient
/// (`offset == 0`) carries `coeff_base_eob` with `level = symbol + 1`; a non-EOB
/// `coeff_base` carries `level = symbol`.
fn recover_base_level(token: CoefficientEntropyToken, offset: usize) -> u32 {
    if offset == 0 {
        u32::from(token.symbol()) + 1
    } else {
        u32::from(token.symbol())
    }
}

/// Reads one sign from the cursor: a `dc_sign` CDF token (`symbol == 1` negative)
/// or a `sign_bit` bypass literal (`value == 1` negative).
fn read_sign_from_tokens(tokens: &[BlockSymbolToken], index: &mut usize) -> Result<bool> {
    let token = next_token(tokens, index)?;
    match token {
        BlockSymbolToken::Coeff(coeff) => Ok(coeff.symbol() != 0),
        BlockSymbolToken::Bypass { value, .. } => Ok(value != 0),
        _ => Err(Error::CoefficientTokenizationAllocationFailed {
            context: "general LF recovery expected a sign token",
        }),
    }
}

/// Returns the coefficient token at the cursor and advances it, or an error if the
/// token is not a coefficient token.
fn coeff_token_at(
    tokens: &[BlockSymbolToken],
    index: &mut usize,
) -> Result<CoefficientEntropyToken> {
    match next_token(tokens, index)? {
        BlockSymbolToken::Coeff(coeff) => Ok(coeff),
        _ => Err(Error::CoefficientTokenizationAllocationFailed {
            context: "general LF recovery expected a coefficient token",
        }),
    }
}

/// Returns the token at the cursor and advances it.
fn next_token(tokens: &[BlockSymbolToken], index: &mut usize) -> Result<BlockSymbolToken> {
    let token =
        tokens
            .get(*index)
            .copied()
            .ok_or(Error::CoefficientTokenizationAllocationFailed {
                context: "general LF recovery token cursor out of range",
            })?;
    *index += 1;
    Ok(token)
}

#[cfg(test)]
#[path = "general_walk_tests.rs"]
mod tests;
