// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AV2 § 5.20.7.28 `read_quant` golomb tail for the general 4x4 luma walk's
//! SINGLE golomb coefficient (`ENC-COEFF-GENERAL-WALK-GOLOMB`, sub-brick 5e). Split
//! out of `general_walk.rs` to keep each file under the 1000-line source budget; the
//! emission half ([`super::general_walk`]) calls [`push_read_quant_golomb_tail`] /
//! [`read_quant_golomb_tail_len`], the recovery half
//! ([`super::general_walk_recover`]) calls [`recover_read_quant_golomb_tail`].
//!
//! GOLOMB PARAMETERS (the SINGLE golomb coefficient per block, this brick's scope):
//! with a single golomb coefficient the running `hrLevelAvg` is still `0` when its
//! `read_quant` fires (the §5.20.7.27 init value), so `predLevel = 0`,
//! `m = Clip3(1, 6, GetMsb(0)) = 1`, `k = m + 1 = 2`, `cMax = Min(m + 4, 6) = 5` —
//! the exact `m = 1` case the DC golomb composers
//! (`crates/splot-encode/src/block_symbol_trace/golomb.rs`) already implement, and
//! the order the decoder reads
//! (`crates/splot-decode/src/tile_payload/coeff_loop/read_quant.rs`). A block with
//! two-or-more golomb-range coefficients needs the `hrLevelAvg` predictor threaded
//! across coefficients (sub-brick 5e-ii) and is rejected upstream.

use crate::block_symbol_trace::BlockSymbolToken;
use crate::error::{Error, Result};

/// `m` for the single-golomb m=1 case.
const GOLOMB_M: u32 = 1;
/// `k = m + 1`.
const GOLOMB_K: u32 = GOLOMB_M + 1;
/// `cMax = Min(m + 4, 6) = 5`: the q-length unary cap. For the finite-q path
/// `q < cMax`; for the golomb-prefix path `q == cMax` (5 q-zeros, no terminator).
const GOLOMB_C_MAX: u32 = 5;
/// The golomb-prefix `xBase` bias `(cMax << m) - (1 << k) = 10 - 4 = 6`: for the
/// prefix path `xBase = bias + 2^length`, so `xm6 = x - 6 = 2^length + coeff_rem`.
const GOLOMB_PREFIX_XBASE_BIAS: u32 = (GOLOMB_C_MAX << GOLOMB_M) - (1 << GOLOMB_K);
/// The smallest golomb extension `x` that takes the golomb-prefix path
/// (`q == cMax`): `x = (cMax << m) = 10` (the finite-q path covers `x` in `0..=9`).
const GOLOMB_PREFIX_X_MIN: u32 = GOLOMB_C_MAX << GOLOMB_M;
/// The maximum golomb `length` (`coeff_rem` bit width) this brick emits/recovers in
/// the prefix path. The supported golomb cap (magnitude `525`) gives
/// `x = 525 - 8 = 517`, `xm6 = 511`, `length = GetMsb(511) = 8`. The recovery rejects
/// a longer prefix.
const GOLOMB_PREFIX_LENGTH_MAX: u32 = 8;

/// Emits the AV2 § 5.20.7.28 `read_quant` golomb tail for the extension
/// `x = magnitude - maxLevel` of the single golomb coefficient (the `m = 1` case;
/// `k = 2`, `cMax = 5`). Mirrors the decoder read order
/// (`crates/splot-decode/src/tile_payload/coeff_loop/read_quant.rs`: the q-length
/// unary capped at `cMax`, then for `q == cMax` the golomb-length unary, then the
/// `coeff_rem` `L(length)` literal) and the existing DC golomb composers
/// (`crates/splot-encode/src/block_symbol_trace/golomb.rs`). All bits are § 8.2.5
/// bypass literals (MSB-first via [`BlockSymbolToken::bypass`]).
///
/// - FINITE-Q (`x < 10`): `q = x >> 1`; emit `q` `q_length_bit` zeros, the
///   terminating `q_length_bit` one, then `coeff_rem` (`x & 1`) as one `L(m) = L(1)`
///   literal.
/// - GOLOMB-PREFIX (`x >= 10`, `q == cMax`): emit `cMax = 5` `q_length_bit` zeros
///   (NO terminator), then the golomb-length unary (`golomb_zeros = length - k`
///   zeros and a terminating one), then `coeff_rem` as one `L(length)` literal,
///   where `length = GetMsb(x - 6)`, `coeff_rem = (x - 6) - 2^length`.
///
/// The caller has validated `x = magnitude - maxLevel` with `magnitude` at or below
/// the supported golomb cap (`525`), so `length <= GOLOMB_PREFIX_LENGTH_MAX`.
pub(super) fn push_read_quant_golomb_tail(out: &mut Vec<BlockSymbolToken>, x: u32) -> Result<()> {
    if x < GOLOMB_PREFIX_X_MIN {
        // Finite-q (`q < cMax`): q zeros, a terminating 1, then `coeff_rem = x & 1`.
        let q = x >> GOLOMB_M;
        for _ in 0..q {
            out.push(BlockSymbolToken::bypass(1, 0));
        }
        out.push(BlockSymbolToken::bypass(1, 1));
        out.push(BlockSymbolToken::bypass(GOLOMB_M, x & 1));
    } else {
        // Golomb-prefix (`q == cMax`): `cMax` q-zeros (no terminator), the
        // golomb-length unary, then `coeff_rem` as one `L(length)` literal.
        let xm6 = x - GOLOMB_PREFIX_XBASE_BIAS;
        let length = xm6.ilog2();
        let golomb_zeros = length - GOLOMB_K;
        let coeff_rem = xm6 - (1 << length);
        for _ in 0..GOLOMB_C_MAX {
            out.push(BlockSymbolToken::bypass(1, 0));
        }
        for _ in 0..golomb_zeros {
            out.push(BlockSymbolToken::bypass(1, 0));
        }
        out.push(BlockSymbolToken::bypass(1, 1));
        out.push(BlockSymbolToken::bypass(length, coeff_rem));
    }
    Ok(())
}

/// Returns the number of § 8.2.5 bypass literals [`push_read_quant_golomb_tail`]
/// emits for the extension `x`, so the trace allocator can reserve exactly. Finite-q
/// (`x < 10`): `q + 2` (q zeros + terminator + `coeff_rem`). Golomb-prefix
/// (`x >= 10`): `cMax + golomb_zeros + 1 (terminator) + 1 (coeff_rem literal)`.
pub(super) fn read_quant_golomb_tail_len(x: u32) -> usize {
    if x < GOLOMB_PREFIX_X_MIN {
        (x >> GOLOMB_M) as usize + 2
    } else {
        let xm6 = x - GOLOMB_PREFIX_XBASE_BIAS;
        let golomb_zeros = xm6.ilog2() - GOLOMB_K;
        GOLOMB_C_MAX as usize + golomb_zeros as usize + 2
    }
}

/// Reads back the AV2 § 5.20.7.28 `read_quant` golomb tail
/// [`push_read_quant_golomb_tail`] emitted for the single golomb coefficient,
/// returning the extension `x = magnitude - maxLevel`. Mirrors the decoder
/// (`crates/splot-decode/src/tile_payload/coeff_loop/read_quant.rs`): the q-length
/// unary capped at `cMax` (finite-q if it terminates before `cMax`, else
/// golomb-prefix), then the `coeff_rem` `L(length)` literal.
pub(super) fn recover_read_quant_golomb_tail(
    tokens: &[BlockSymbolToken],
    index: &mut usize,
) -> Result<u32> {
    // q-length unary capped at `cMax`: count zeros until a terminating one or `cMax`.
    let mut q = 0u32;
    let mut terminated = false;
    while q < GOLOMB_C_MAX {
        if read_golomb_bit(tokens, index)? {
            terminated = true;
            break;
        }
        q += 1;
    }

    if terminated {
        // Finite-q: `length = m`, `coeff_rem = L(m)`, `x = (q << m) + coeff_rem`.
        let coeff_rem = read_golomb_literal(tokens, index, GOLOMB_M)?;
        return Ok((q << GOLOMB_M) + coeff_rem);
    }

    // Golomb-prefix (`q == cMax`): read the golomb-length unary (zeros + a 1) to get
    // `length = golomb_zeros + k`, then `coeff_rem = L(length)`.
    let mut golomb_zeros = 0u32;
    while !read_golomb_bit(tokens, index)? {
        golomb_zeros += 1;
        if golomb_zeros > GOLOMB_PREFIX_LENGTH_MAX - GOLOMB_K {
            return Err(Error::CoefficientTokenizationMalformedTokenTrace {
                context: "general LF recovery golomb-prefix length out of range",
            });
        }
    }
    let length = golomb_zeros + GOLOMB_K;
    let coeff_rem = read_golomb_literal(tokens, index, length)?;
    // `x = (cMax << m) + (2^length - 2^k) + coeff_rem = bias + 2^length + coeff_rem`.
    let x = GOLOMB_PREFIX_XBASE_BIAS + (1 << length) + coeff_rem;
    Ok(x)
}

/// Reads one width-1 § 8.2.5 bypass literal from the golomb tail at the cursor,
/// returning whether it is set. Errors if the token is not a width-1 bypass literal.
fn read_golomb_bit(tokens: &[BlockSymbolToken], index: &mut usize) -> Result<bool> {
    match tokens.get(*index).copied() {
        Some(BlockSymbolToken::Bypass { width: 1, value }) => {
            *index += 1;
            Ok(value != 0)
        }
        _ => Err(Error::CoefficientTokenizationMalformedTokenTrace {
            context: "general LF recovery expected a golomb q_length/length bypass bit",
        }),
    }
}

/// Reads one `width`-bit § 8.2.5 bypass literal from the golomb tail at the cursor.
/// Errors if the token is not a bypass literal of the expected width.
fn read_golomb_literal(tokens: &[BlockSymbolToken], index: &mut usize, width: u32) -> Result<u32> {
    match tokens.get(*index).copied() {
        Some(BlockSymbolToken::Bypass {
            width: token_width,
            value,
        }) if token_width == width => {
            *index += 1;
            Ok(value)
        }
        _ => Err(Error::CoefficientTokenizationMalformedTokenTrace {
            context: "general LF recovery expected a golomb coeff_rem bypass literal",
        }),
    }
}
