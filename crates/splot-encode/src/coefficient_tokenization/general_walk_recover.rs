// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AV2 § 8.2 SELF-CONSISTENCY recovery inverse of the general 4x4 luma
//! coefficient-tokenization walk ([`super::general_walk`]). Split out of
//! `general_walk.rs` to keep each file under the 1000-line source budget; the
//! EMISSION half stays there. The two share the scan-order and eob helpers
//! re-exported from [`super::general_walk`] (`scan_2d_4x4`, `scan_pos`,
//! `eob_base_for_pt`, `MIN_EOB_WITH_EXTRA`, `MAX_GENERAL_EOB_PT`).
//!
//! [`recover_quant_from_tokens`] re-reads an emitted token stream in the same
//! reverse-scan order and rebuilds the signed `[i32; 16]` raster block, proving the
//! encoder's emitted (level, sign, position) triples — and, at every golomb-range
//! coefficient, its § 5.20.7.28 `read_quant` golomb tail with the running `hrLevelAvg`
//! predictor threaded across them — are internally reversible.
//!
//! HONESTY: this proof is § 8.2 SELF-CONSISTENCY, not decoder/AVM verification. The
//! same code authored the emission and this inverse, so it proves the round trip is
//! internally consistent — with asymmetric values it catches a swapped sign order
//! (AC-before-DC) or a level/position transposition. It does NOT validate the
//! § 8.3.2 CDF contexts against a real decoder; context conformance is deferred to
//! the splot-decode cross-check brick. The golomb tail is mirrored from the decoder
//! `read_quant` (`crates/splot-decode/src/tile_payload/coeff_loop/read_quant.rs`)
//! and the existing DC golomb composers
//! (`crates/splot-encode/src/block_symbol_trace/golomb.rs`).

use super::general_walk::{
    MAX_GENERAL_EOB_PT, MIN_EOB_WITH_EXTRA, eob_base_for_pt, general_walk_max_level_for_pos,
    scan_2d_4x4, scan_pos,
};
use super::general_walk_golomb::{
    golomb_params_from_hr_level_avg, next_hr_level_avg, recover_read_quant_golomb_tail,
};
use super::{CoefficientEntropyToken, CoefficientTokenSyntax, coded_luma_all_zero_token};
use crate::block_symbol_trace::BlockSymbolToken;
use crate::error::{Error, Result};

/// 4x4 DCT_DCT coefficient count (`Quant[16]`).
const TX_4X4_COEFF_COUNT: usize = 16;

/// Re-reads the emitted token stream in the same reverse-scan order and rebuilds
/// the signed `[i32; 16]` raster block, proving the encoder's emitted
/// (level, sign, position) triples are internally reversible.
///
/// This is § 8.2 self-consistency, not decoder/AVM verification: the same code
/// authored the emission and this inverse. It walks the trace's base-pass and
/// sign-pass tokens (skipping the `all_zero` / `eob_pt_16` / `eob_extra` / chroma
/// tail), pairs each base level with its reverse-scan sign, and writes the signed
/// value at the scan-derived raster position. A coefficient whose recovered
/// base+`coeff_br` level reaches its position `maxLevel` additionally carries a
/// § 5.20.7.28 `read_quant` golomb tail right after its sign token; the tail is read
/// back and `x = magnitude - maxLevel` is added to the recovered magnitude. MULTIPLE
/// golomb coefficients are supported: the running `hrLevelAvg` predictor is threaded
/// across them in reverse scan so each tail's golomb parameter `m` matches the
/// emission (see [`super::general_walk_golomb::recover_read_quant_golomb_tail`]). The
/// `eob_extra` flag
/// (present when eobPt `>= 3`, i.e. eob `>= 3`) is consumed by
/// [`read_eob_from_tokens`] to recover the eob. An all-zero trace (single
/// `all_zero == 1`) recovers the zero block.
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

    // Base pass: `eob` reverse-scan coefficients, levels only. ANY coefficient (the
    // EOB coefficient at offset 0, or a non-EOB coefficient) may be followed by an
    // interleaved `coeff_br` token that refines its level (mirroring the emission in
    // `compose_base_pass`); a zero non-EOB coefficient has level 0 and no `coeff_br`.
    // Recovery is keyed on token SYNTAX (`CoeffBaseEob` / `CoeffBase` / `CoeffBr`), so
    // the HF tokens — which share those syntaxes but route different CDF tables —
    // recover identically to the LF tokens (the non-EOB level mapping is the same).
    let mut levels = [0u32; TX_4X4_COEFF_COUNT];
    for offset in 0..eob {
        let c = eob - 1 - offset;
        let pos = scan_pos(&scan, c)?;
        let token = coeff_token_at(tokens, &mut index)?;
        let mut level = recover_base_level(token, offset);
        level += recover_interleaved_coeff_br(tokens, &mut index)?;
        levels[pos] = level;
    }

    // Sign pass: reverse-scan, interleaved per nonzero coefficient. A coefficient
    // whose recovered level reached its position `maxLevel` carries a § 5.20.7.28
    // `read_quant` golomb tail right after its sign token (the sign+quant pass reads
    // the sign first, then `read_quant`); the tail recovers `x = magnitude - maxLevel`.
    // MULTIPLE golomb coefficients are supported (sub-brick 5e-ii): the running
    // `hrLevelAvg` predictor (init `0`) is threaded across them in this reverse-scan
    // order, exactly as the emission side does, so each golomb coefficient's `m` (and
    // therefore the parameters used to read its tail back) matches.
    let mut hr_level_avg = 0u32;
    for offset in 0..eob {
        let c = eob - 1 - offset;
        let pos = scan_pos(&scan, c)?;
        let level = levels[pos];
        if level == 0 {
            continue;
        }
        let negative = read_sign_from_tokens(tokens, &mut index)?;
        // A coefficient whose recovered base+`coeff_br` level reached its position
        // `maxLevel` carries a § 5.20.7.28 `read_quant` golomb tail right after its
        // sign token (the sign+quant pass reads the sign first, then `read_quant`);
        // the tail recovers `x = magnitude - maxLevel`. The golomb parameters come from
        // the running `hrLevelAvg`, which then updates for the next golomb coefficient.
        let max_level = general_walk_max_level_for_pos(pos);
        let magnitude = if level >= max_level {
            let params = golomb_params_from_hr_level_avg(hr_level_avg);
            let x = recover_read_quant_golomb_tail(tokens, &mut index, params)?;
            hr_level_avg = next_hr_level_avg(x, hr_level_avg);
            max_level
                .checked_add(x)
                .ok_or(Error::CoefficientTokenizationMalformedTokenTrace {
                    context: "general LF recovery golomb magnitude overflow",
                })?
        } else {
            level
        };
        let signed = if negative {
            -(magnitude as i32)
        } else {
            magnitude as i32
        };
        quant[pos] = signed;
    }

    Ok(quant)
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

/// Reads the eob from the `eob_pt_16` token at the cursor and, when its symbol
/// selects eobPt `>= 3`, the interleaved `eob_extra` CDF flag and the `eobPt - 3`
/// `eob_extra_bit` bypass literals that follow it. The `eob_pt_16` symbol is
/// `eobPt - 1`; for eobPt `< 3` `eob == eobPt` so `eob = symbol + 1`. For
/// eobPt `>= 3`, `eob = base + (eob_extra << (eobPt - 3)) + eob_extra_bits` where
/// `base = eob_base_for_pt(eobPt)`, mirroring the emission in
/// [`super::general_walk::tokenize_general_lf_luma_block`] (and the decoder
/// `nonzero_coeff_eob`).
///
/// The `eob_extra_bit` literals are read back MSB-first (bit `eobPt - 4` down to bit
/// 0), the SAME order they were emitted (see the `general_walk` `eob_extra_bit` BIT
/// ORDER note); `eob_extra_bits` accumulates `(value << 1) | bit`, matching the
/// decoder's `read_literal`.
fn read_eob_from_tokens(tokens: &[BlockSymbolToken], index: &mut usize) -> Result<usize> {
    let eob_pt = usize::from(coeff_token_at(tokens, index)?.symbol()) + 1;
    if eob_pt < MIN_EOB_WITH_EXTRA {
        return Ok(eob_pt);
    }
    // Reject an out-of-range eobPt from a malformed trace BEFORE the `1 << width`
    // shift below: a large `eob_pt_16` symbol (the symbol is a `u8`, so eobPt can be
    // up to 256) would otherwise shift by `>= usize::BITS` and panic, violating the
    // library no-panic error model. A well-formed trace from this tokenizer never
    // exceeds `MAX_GENERAL_EOB_PT` (5).
    if eob_pt > MAX_GENERAL_EOB_PT {
        return Err(Error::CoefficientTokenizationMalformedTokenTrace {
            context: "general LF recovery eob_pt_16 symbol out of range",
        });
    }
    // eobPt >= 3: the next token is the `eob_extra` CDF flag (the HIGH refinement bit).
    let extra_token = coeff_token_at(tokens, index)?;
    if !matches!(extra_token.syntax(), CoefficientTokenSyntax::EobExtra) {
        return Err(Error::CoefficientTokenizationMalformedTokenTrace {
            context: "general LF recovery expected an eob_extra token",
        });
    }
    let eob_extra = extra_token.symbol() != 0;
    // Then `eobPt - 3` `eob_extra_bit` bypass literals (the LOW refinement bits),
    // MSB-first. Reassemble `eob_extra_bits` the way the decoder `read_literal` does.
    let width = eob_pt - MIN_EOB_WITH_EXTRA;
    let mut eob_extra_bits = 0usize;
    for _ in 0..width {
        let bit = read_eob_extra_bit(tokens, index)?;
        eob_extra_bits = (eob_extra_bits << 1) | bit;
    }
    let base = eob_base_for_pt(eob_pt);
    let extra = if eob_extra { 1usize << width } else { 0 };
    Ok(base + extra + eob_extra_bits)
}

/// Reads one `eob_extra_bit` bypass literal (`bypass(1, bit)`) at the cursor and
/// advances it, returning its `0`/`1` value. Errors if the token is not a width-1
/// bypass literal.
fn read_eob_extra_bit(tokens: &[BlockSymbolToken], index: &mut usize) -> Result<usize> {
    match next_token(tokens, index)? {
        BlockSymbolToken::Bypass { width: 1, value } => Ok(value as usize),
        _ => Err(Error::CoefficientTokenizationMalformedTokenTrace {
            context: "general LF recovery expected an eob_extra_bit bypass literal",
        }),
    }
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

/// Reads the optional interleaved `coeff_br` refinement that follows a base-pass
/// coefficient's `coeff_base_eob` / `coeff_base`: when the next token is a
/// `coeff_br`, it is consumed and its symbol is returned (the level increment);
/// otherwise the cursor is left untouched and `0` is returned. Both the EOB
/// coefficient and the non-EOB coefficient may carry one, so every base-pass
/// coefficient peeks for it.
fn recover_interleaved_coeff_br(tokens: &[BlockSymbolToken], index: &mut usize) -> Result<u32> {
    match tokens.get(*index) {
        Some(BlockSymbolToken::Coeff(coeff))
            if matches!(coeff.syntax(), CoefficientTokenSyntax::CoeffBr) =>
        {
            *index += 1;
            Ok(u32::from(coeff.symbol()))
        }
        _ => Ok(0),
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
