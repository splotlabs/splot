// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AV2 § 8.2 SELF-CONSISTENCY recovery inverse of the size-generic general luma
//! coefficient-tokenization walk ([`super::general_walk`]). Split out of
//! `general_walk.rs` to keep each file under the 1000-line source budget; the EMISSION
//! half stays there. The two share the [`TxGeom`] descriptor and the scan-order / eob
//! helpers re-exported from [`super::general_walk`] (`build_scan`, `scan_pos`,
//! `eob_base_for_pt`, `MIN_EOB_WITH_EXTRA`, `MAX_GENERAL_EOB_PT`).
//!
//! [`recover_quant_from_tokens`] (the 4x4 form) and
//! [`recover_quant_from_tokens_geom`] (the size-generic form) re-read an emitted token
//! stream in the same reverse-scan order and rebuild the signed raster block, proving
//! the encoder's emitted (level, sign, position) triples — and, at every golomb-range
//! coefficient, its § 5.20.7.28 `read_quant` golomb tail with the running `hrLevelAvg`
//! predictor threaded across them — are internally reversible.
//!
//! HONESTY: this proof is § 8.2 SELF-CONSISTENCY, not decoder/AVM verification. The
//! same code authored the emission and this inverse. It does NOT validate the § 8.3.2
//! CDF contexts against a real decoder; context conformance is deferred to the
//! splot-decode cross-check brick.

use super::general_walk::{
    EOB_PT_256_EXTRA_SYMBOL, EOB_PT_WITH_EXTRA, MAX_GENERAL_EOB_PT, MIN_EOB_WITH_EXTRA, TxGeom,
    build_scan, eob_base_for_pt, general_walk_max_level_for_pos, scan_pos,
};
use super::general_walk_golomb::{
    golomb_params_from_hr_level_avg, next_hr_level_avg, recover_read_quant_golomb_tail,
};
use super::{CoefficientEntropyToken, CoefficientTokenSyntax, coded_luma_all_zero_token};
use crate::block_symbol_trace::BlockSymbolToken;
use crate::error::{Error, Result};

/// 4x4 DCT_DCT coefficient count (`Quant[16]`); the public 4x4 recovery's array length.
const TX_4X4_COEFF_COUNT: usize = 16;

/// Re-reads an emitted 4x4 token stream and rebuilds the signed `[i32; 16]` raster
/// block, proving the encoder's emitted (level, sign, position) triples are internally
/// reversible. Delegates to [`recover_quant_from_tokens_geom`] with [`TxGeom::TX_4X4`].
///
/// This is § 8.2 self-consistency, not decoder/AVM verification: the same code authored
/// the emission and this inverse. An all-zero trace (single `all_zero == 1`) recovers
/// the zero block.
pub(crate) fn recover_quant_from_tokens(
    tokens: &[BlockSymbolToken],
    coeff_cdf_q_ctx: usize,
) -> Result<[i32; TX_4X4_COEFF_COUNT]> {
    let quant = recover_quant_from_tokens_geom(tokens, TxGeom::TX_4X4, coeff_cdf_q_ctx)?;
    let mut out = [0i32; TX_4X4_COEFF_COUNT];
    if quant.len() != TX_4X4_COEFF_COUNT {
        return Err(Error::CoefficientTokenizationMalformedTokenTrace {
            context: "general walk 4x4 recovery length mismatch",
        });
    }
    out.copy_from_slice(&quant);
    Ok(out)
}

/// Re-reads an emitted token stream for the block described by `geom` in the same
/// reverse-scan order and rebuilds the signed raster block (`Vec<i32>` of length
/// `geom.coeff_count`). The size-generic inverse shared by the 4x4 and 16x16 walks: it
/// walks the trace's base-pass and sign-pass tokens (skipping the `all_zero` /
/// `eob_pt_*` / `eob_extra` / chroma tail), pairs each base level with its
/// reverse-scan sign, and writes the signed value at the scan-derived raster position.
/// A coefficient whose recovered base+`coeff_br` level reaches its position `maxLevel`
/// additionally carries a § 5.20.7.28 `read_quant` golomb tail; the tail is read back
/// and `x = magnitude - maxLevel` is added. MULTIPLE golomb coefficients are supported
/// via the running `hrLevelAvg` predictor. The `eob_extra` flag (present when
/// eobPt `>= 3`) is consumed by [`read_eob_from_tokens`] to recover the eob.
pub(crate) fn recover_quant_from_tokens_geom(
    tokens: &[BlockSymbolToken],
    geom: TxGeom,
    coeff_cdf_q_ctx: usize,
) -> Result<Vec<i32>> {
    let scan = build_scan(geom)?;
    let mut quant = vec_of_zero_i32(geom.coeff_count)?;

    if tokens.len() == 1 {
        return Ok(quant);
    }

    let mut index = 0usize;
    skip_expected_all_zero(tokens, &mut index, coeff_cdf_q_ctx)?;
    let eob = read_eob_from_tokens(tokens, &mut index)?;

    let mut levels = vec_of_zero_u32(geom.coeff_count)?;
    for offset in 0..eob {
        let c = eob - 1 - offset;
        let pos = scan_pos(&scan, c)?;
        let token = coeff_token_at(tokens, &mut index)?;
        let mut level = recover_base_level(token, offset);
        level += recover_interleaved_coeff_br(tokens, &mut index);
        if let Some(slot) = levels.get_mut(pos) {
            *slot = level;
        }
    }

    let mut hr_level_avg = 0u32;
    for offset in 0..eob {
        let c = eob - 1 - offset;
        let pos = scan_pos(&scan, c)?;
        let level = levels.get(pos).copied().unwrap_or(0);
        if level == 0 {
            continue;
        }
        let negative = read_sign_from_tokens(tokens, &mut index)?;
        let max_level = general_walk_max_level_for_pos(pos, geom);
        let magnitude = if level >= max_level {
            let params = golomb_params_from_hr_level_avg(hr_level_avg);
            let x = recover_read_quant_golomb_tail(tokens, &mut index, params)?;
            hr_level_avg = next_hr_level_avg(x, hr_level_avg);
            max_level
                .checked_add(x)
                .ok_or(Error::CoefficientTokenizationMalformedTokenTrace {
                    context: "general walk recovery golomb magnitude overflow",
                })?
        } else {
            level
        };
        let signed = if negative {
            -(magnitude as i32)
        } else {
            magnitude as i32
        };
        if let Some(slot) = quant.get_mut(pos) {
            *slot = signed;
        }
    }

    Ok(quant)
}

/// Allocates a zeroed `Vec<i32>` of `len` with a checked reservation.
fn vec_of_zero_i32(len: usize) -> Result<Vec<i32>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
            context: "general walk recovery quant allocation",
        })?;
    v.resize(len, 0i32);
    Ok(v)
}

/// Allocates a zeroed `Vec<u32>` of `len` with a checked reservation.
fn vec_of_zero_u32(len: usize) -> Result<Vec<u32>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
            context: "general walk recovery level allocation",
        })?;
    v.resize(len, 0u32);
    Ok(v)
}

/// Skips the leading coded luma `all_zero == 0` token during recovery, asserting it is
/// present. The 4x4 and 16x16 walks both emit a coded `all_zero == 0` at their own
/// `txSzCtx`; recovery only asserts the symbol is the coded-block flag (the leading
/// token's selector varies by size, so it matches on syntax/symbol rather than the
/// exact 4x4 token when the trace is not 4x4).
fn skip_expected_all_zero(
    tokens: &[BlockSymbolToken],
    index: &mut usize,
    coeff_cdf_q_ctx: usize,
) -> Result<()> {
    let token = coeff_token_at(tokens, index)?;
    let expected_4x4 = coded_luma_all_zero_token(coeff_cdf_q_ctx);
    let is_coded_all_zero =
        matches!(token.syntax(), CoefficientTokenSyntax::AllZero) && token.symbol() == 0;
    if token != expected_4x4 && !is_coded_all_zero {
        return Err(Error::CoefficientTokenizationMalformedTokenTrace {
            context: "general walk recovery expected coded all_zero token",
        });
    }
    Ok(())
}

/// Reads the eob from the `eob_pt_*` token at the cursor and, when its symbol selects
/// eobPt `>= 3`, the interleaved `eob_extra` CDF flag and the `eobPt - 3`
/// `eob_extra_bit` bypass literals that follow it. The `eob_pt_*` symbol is `eobPt - 1`
/// for eobPt `1..=7`; for the `eob_pt_256` symbol 7 the eobPt is instead
/// `8 + eob_pt_extra`, read from a 1-bit `eob_pt_extra` bypass literal that immediately
/// follows the symbol ([`read_eob_pt`]). For eobPt `< 3` `eob == eobPt` so
/// `eob = symbol + 1`. For eobPt `>= 3`,
/// `eob = base + (eob_extra << (eobPt - 3)) + eob_extra_bits`. The `eob_pt_extra` bit
/// (when present) and the `eob_extra_bit` literals are read back MSB-first (the same
/// order they were emitted). The exact inverse of the decoder `read_nonzero_coeff_eob`
/// / `resolved_eob_pt` (`crates/splot-decode/src/tile_payload/coeff_loop.rs`).
fn read_eob_from_tokens(tokens: &[BlockSymbolToken], index: &mut usize) -> Result<usize> {
    let eob_pt = read_eob_pt(tokens, index)?;
    if eob_pt < MIN_EOB_WITH_EXTRA {
        return Ok(eob_pt);
    }
    if eob_pt > MAX_GENERAL_EOB_PT {
        return Err(Error::CoefficientTokenizationMalformedTokenTrace {
            context: "general walk recovery eob_pt symbol out of range",
        });
    }
    let extra_token = coeff_token_at(tokens, index)?;
    if !matches!(extra_token.syntax(), CoefficientTokenSyntax::EobExtra) {
        return Err(Error::CoefficientTokenizationMalformedTokenTrace {
            context: "general walk recovery expected an eob_extra token",
        });
    }
    let eob_extra = extra_token.symbol() != 0;
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

/// Reads the eobPt from the `eob_pt_*` symbol token at the cursor (and, for the
/// `eob_pt_256` symbol 7, the 1-bit `eob_pt_extra` bypass literal that immediately
/// follows it). For symbol `0..=6` the eobPt is `symbol + 1`. For symbol 7
/// ([`EOB_PT_256_EXTRA_SYMBOL`]) the eobPt is `8 + eob_pt_extra` (eobPt 8 → bit 0,
/// eobPt 9 → bit 1), the EXACT inverse of the decoder `resolved_eob_pt`'s
/// `8 + eob_pt_extra`. A symbol `> 7` is rejected (no modeled `eob_pt_*` class has a
/// larger symbol).
fn read_eob_pt(tokens: &[BlockSymbolToken], index: &mut usize) -> Result<usize> {
    let symbol = coeff_token_at(tokens, index)?.symbol();
    if symbol < EOB_PT_256_EXTRA_SYMBOL {
        return Ok(usize::from(symbol) + 1);
    }
    if symbol > EOB_PT_256_EXTRA_SYMBOL {
        return Err(Error::CoefficientTokenizationMalformedTokenTrace {
            context: "general walk recovery eob_pt symbol out of range",
        });
    }
    let eob_pt_extra = read_eob_extra_bit(tokens, index)?;
    Ok(EOB_PT_WITH_EXTRA + eob_pt_extra)
}

/// Reads one `eob_extra_bit` (or `eob_pt_extra`) bypass literal (`bypass(1, bit)`) at
/// the cursor and advances it, returning its `0`/`1` value. Errors if the token is not
/// a width-1 bypass literal.
fn read_eob_extra_bit(tokens: &[BlockSymbolToken], index: &mut usize) -> Result<usize> {
    match next_token(tokens, index)? {
        BlockSymbolToken::Bypass { width: 1, value } => Ok(value as usize),
        _ => Err(Error::CoefficientTokenizationMalformedTokenTrace {
            context: "general walk recovery expected an eob_extra_bit bypass literal",
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
/// coefficient's `coeff_base_eob` / `coeff_base`: when the next token is a `coeff_br`,
/// it is consumed and its symbol is returned (the level increment); otherwise the
/// cursor is left untouched and `0` is returned.
fn recover_interleaved_coeff_br(tokens: &[BlockSymbolToken], index: &mut usize) -> u32 {
    match tokens.get(*index) {
        Some(BlockSymbolToken::Coeff(coeff))
            if matches!(coeff.syntax(), CoefficientTokenSyntax::CoeffBr) =>
        {
            *index += 1;
            u32::from(coeff.symbol())
        }
        _ => 0,
    }
}

/// Reads one sign from the cursor: a `dc_sign` CDF token (`symbol == 1` negative) or a
/// `sign_bit` bypass literal (`value == 1` negative).
fn read_sign_from_tokens(tokens: &[BlockSymbolToken], index: &mut usize) -> Result<bool> {
    let token = next_token(tokens, index)?;
    match token {
        BlockSymbolToken::Coeff(coeff) => Ok(coeff.symbol() != 0),
        BlockSymbolToken::Bypass { value, .. } => Ok(value != 0),
        _ => Err(Error::CoefficientTokenizationMalformedTokenTrace {
            context: "general walk recovery expected a sign token",
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
        _ => Err(Error::CoefficientTokenizationMalformedTokenTrace {
            context: "general walk recovery expected a coefficient token",
        }),
    }
}

/// Returns the token at the cursor and advances it.
fn next_token(tokens: &[BlockSymbolToken], index: &mut usize) -> Result<BlockSymbolToken> {
    let token =
        tokens
            .get(*index)
            .copied()
            .ok_or(Error::CoefficientTokenizationMalformedTokenTrace {
                context: "general walk recovery token cursor out of range",
            })?;
    *index += 1;
    Ok(token)
}
