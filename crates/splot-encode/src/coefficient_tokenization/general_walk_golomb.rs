// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AV2 § 5.20.7.28 `read_quant` golomb tail for the general 4x4 luma walk's
//! golomb-range coefficients (`ENC-COEFF-GENERAL-WALK-GOLOMB`, sub-brick 5e, extended
//! to MULTIPLE golomb coefficients per block by `ENC-COEFF-GENERAL-WALK-GOLOMB-MULTI`,
//! sub-brick 5e-ii). Split out of `general_walk.rs` to keep each file under the
//! 1000-line source budget; the emission half ([`super::general_walk`]) calls
//! [`push_read_quant_golomb_tail`] / [`read_quant_golomb_tail_len`], the recovery half
//! ([`super::general_walk_recover`]) calls [`recover_read_quant_golomb_tail`].
//!
//! GENERAL GOLOMB PARAMETER `m` (mirrored from the decoder
//! `crates/splot-decode/src/tile_payload/coeff_loop/read_quant.rs`, the `m`/`k`/`cMax`
//! derivation ~lines 244-249 and the `coeff_rem` decode ~250-313): for a golomb
//! coefficient (magnitude `>= maxLevel`) with `lvlShift == 0` (TCQ off, not
//! parity-hidden in this path) the predictor is `predLevel = hrLevelAvg >> lvlShift =
//! hrLevelAvg`, and
//!
//! - `m = Clip3(1, 6, GetMsb(predLevel))` (`GetMsb(0) == 0` → clamp → `1`),
//! - `k = m + 1`,
//! - `cMax = Min(m + 4, 6)`,
//! - `x = magnitude - maxLevel`.
//!
//! The SINGLE-golomb case (a block with one golomb coefficient) has `hrLevelAvg == 0`
//! when its `read_quant` fires (the § 5.20.7.27 init value), so `predLevel == 0`,
//! `m == 1`, `k == 2`, `cMax == 5` — the original sub-brick 5e case. A SECOND (or
//! later) golomb coefficient is reached with a non-zero `hrLevelAvg` threaded from the
//! earlier golomb coefficient(s) (in reverse scan `c = eob-1 .. 0`), so its `m`
//! varies. The caller ([`super::general_walk::compose_sign_pass`]) derives `m` per
//! coefficient from the running `hrLevelAvg` via [`golomb_m_from_hr_level_avg`] and
//! passes the resulting [`GolombParams`] here; the recovery side
//! ([`super::general_walk_recover`]) threads the same `hrLevelAvg` and derives the
//! identical `m`.
//!
//! `hrLevelAvg` UPDATE (decoder ~lines 315-343, `lvlShift == 0`):
//! `hrLevelAvg = ((x << lvlShift) + hrLevelAvg) >> 1 = (x + hrLevelAvg) >> 1`, carried
//! to the NEXT golomb coefficient in reverse-scan order. Computed by
//! [`next_hr_level_avg`].

use crate::block_symbol_trace::BlockSymbolToken;
use crate::error::{Error, Result};

/// `m` lower clamp `Clip3(1, 6, ...)` (`MIN_M`), mirroring the decoder.
const MIN_M: u32 = 1;
/// `m` upper clamp `Clip3(1, 6, ...)` (`MAX_M`), mirroring the decoder.
const MAX_M: u32 = 6;
/// `cMax = Min(m + 4, 6)` upper bound (the decoder `Min(m + 4, 6)`).
const C_MAX_CAP: u32 = 6;
/// The maximum golomb-prefix `length` (`coeff_rem` bit width) this brick emits and
/// recovers. The decoder supports up to `MAX_COEFF_REM_BITS == 32`
/// (`crates/splot-decode/src/tile_payload/coeff_loop/read_quant.rs`); this encoder
/// caps at `8` (matching the original single-golomb `m = 1` bound) so a `coeff_rem`
/// stays at most `255` (exact in the decoded `u8` symbol view) and the § 8.2
/// self-consistency recovery can read every accepted magnitude back. This single
/// length budget drives the per-`m` magnitude cap
/// [`super::general_walk::golomb_x_max_for_m`]; a higher golomb extension is rejected
/// upstream by [`super::general_walk::validate_general_lf_scope`].
pub(super) const GOLOMB_PREFIX_LENGTH_MAX: u32 = 8;

/// The golomb parameters for ONE § 5.20.7.28 `read_quant` coefficient, derived from
/// the running `hrLevelAvg` (with `lvlShift == 0`). `bias = (cMax << m) - (1 << k)` is
/// the golomb-prefix `xBase` offset so that for the prefix path
/// `xBase = bias + (1 << length)` (see [`push_read_quant_golomb_tail`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct GolombParams {
    /// `m = Clip3(1, 6, GetMsb(hrLevelAvg))`.
    m: u32,
    /// `k = m + 1`.
    k: u32,
    /// `cMax = Min(m + 4, 6)`.
    c_max: u32,
    /// `bias = (cMax << m) - (1 << k)` (the golomb-prefix `xBase` offset).
    bias: u32,
    /// `prefix_x_min = cMax << m`: the smallest extension `x` that takes the
    /// golomb-prefix path (`q == cMax`); below it the finite-q path applies.
    prefix_x_min: u32,
}

/// `GetMsb(value)`: the position of the most-significant set bit, `0` for `0`
/// (mirroring the decoder `get_msb`). `GetMsb(0) == 0` clamps `m` to `MIN_M == 1`.
const fn get_msb(value: u32) -> u32 {
    if value == 0 {
        0
    } else {
        u32::BITS - 1 - value.leading_zeros()
    }
}

/// Derives `m = Clip3(1, 6, GetMsb(hrLevelAvg))` for a golomb coefficient with
/// `lvlShift == 0` (`predLevel == hrLevelAvg`), mirroring the decoder's `m`
/// derivation. Used by [`golomb_params_from_hr_level_avg`].
const fn golomb_m_from_hr_level_avg(hr_level_avg: u32) -> u32 {
    let msb = get_msb(hr_level_avg);
    if msb < MIN_M {
        MIN_M
    } else if msb > MAX_M {
        MAX_M
    } else {
        msb
    }
}

/// Derives the [`GolombParams`] for the golomb coefficient whose `read_quant` fires
/// at the given running `hrLevelAvg` (with `lvlShift == 0`). `m`, `k`, `cMax`, `bias`,
/// and `prefix_x_min` follow the decoder formulas.
pub(super) const fn golomb_params_from_hr_level_avg(hr_level_avg: u32) -> GolombParams {
    let m = golomb_m_from_hr_level_avg(hr_level_avg);
    let k = m + 1;
    let c_max = if m + 4 < C_MAX_CAP { m + 4 } else { C_MAX_CAP };
    let prefix_x_min = c_max << m;
    let bias = prefix_x_min - (1 << k);
    GolombParams {
        m,
        k,
        c_max,
        bias,
        prefix_x_min,
    }
}

/// Returns the largest golomb extension `x = magnitude - maxLevel` this brick can
/// emit and recover for a golomb coefficient with the given parameters, keeping the
/// golomb-prefix `length` within [`GOLOMB_PREFIX_LENGTH_MAX`] (`8`). The golomb-prefix
/// `length = GetMsb(x - bias)` is `<= GOLOMB_PREFIX_LENGTH_MAX` iff
/// `x - bias < (1 << (GOLOMB_PREFIX_LENGTH_MAX + 1))`, so the maximum extension is
/// `bias + (1 << (GOLOMB_PREFIX_LENGTH_MAX + 1)) - 1`. For `m == 1` (`bias == 6`) this
/// is `6 + 511 = 517`, matching the original single-golomb cap; for the largest
/// `m == 6` (`bias == 256`) it is `256 + 511 = 767` — all small enough that
/// `maxLevel + x` never overflows the `u32`/`i32` magnitude.
pub(super) const fn golomb_x_max(params: GolombParams) -> u32 {
    params.bias + (1 << (GOLOMB_PREFIX_LENGTH_MAX + 1)) - 1
}

/// Updates the running `hrLevelAvg` after a golomb coefficient with extension `x`
/// (`lvlShift == 0`): `hrLevelAvg = (x + hrLevelAvg) >> 1` (the decoder
/// `((x << lvlShift) + hrLevelAvg) >> 1`). Carried to the NEXT golomb coefficient in
/// reverse-scan order. Shared with the recovery side so both thread identically.
pub(super) const fn next_hr_level_avg(x: u32, hr_level_avg: u32) -> u32 {
    // `x` and `hr_level_avg` are both bounded by the accepted golomb magnitude cap, so
    // the sum stays well within `u32`; `>> 1` cannot overflow.
    (x + hr_level_avg) >> 1
}

/// Emits the AV2 § 5.20.7.28 `read_quant` golomb tail for the extension
/// `x = magnitude - maxLevel` of a golomb coefficient with the given golomb
/// parameters (general `m`; `k = m + 1`, `cMax = Min(m + 4, 6)`). Mirrors the decoder
/// read order (`crates/splot-decode/src/tile_payload/coeff_loop/read_quant.rs`: the
/// q-length unary capped at `cMax`, then for `q == cMax` the golomb-length unary, then
/// the `coeff_rem` `L(length)` literal) and the existing DC golomb composers
/// (`crates/splot-encode/src/block_symbol_trace/golomb.rs`). All bits are § 8.2.5
/// bypass literals (MSB-first via [`BlockSymbolToken::bypass`]).
///
/// - FINITE-Q (`x < cMax << m`, so `q = x >> m < cMax`): emit `q` `q_length_bit`
///   zeros, the terminating `q_length_bit` one, then `coeff_rem = x & ((1 << m) - 1)`
///   as one `L(m)` literal (NOT `L(1)`: the remainder is `m` bits wide for general
///   `m`).
/// - GOLOMB-PREFIX (`x >= cMax << m`, so `q == cMax`): emit `cMax` `q_length_bit`
///   zeros (NO terminator), then the golomb-length unary (`golomb_zeros = length - k`
///   zeros and a terminating one), then `coeff_rem` as one `L(length)` literal, where
///   `length = GetMsb(x - bias)` (`bias = (cMax << m) - (1 << k)`) and
///   `coeff_rem = (x - bias) - (1 << length)`.
///
/// The caller has validated `x` is within the per-`m` golomb cap
/// ([`super::general_walk::golomb_x_max_for_m`]), so `length <=
/// GOLOMB_PREFIX_LENGTH_MAX`.
pub(super) fn push_read_quant_golomb_tail(
    out: &mut Vec<BlockSymbolToken>,
    x: u32,
    params: GolombParams,
) -> Result<()> {
    if x < params.prefix_x_min {
        // Finite-q (`q < cMax`): q zeros, a terminating 1, then `coeff_rem = x mod 2^m`
        // as one `L(m)` literal.
        let q = x >> params.m;
        for _ in 0..q {
            out.push(BlockSymbolToken::bypass(1, 0));
        }
        out.push(BlockSymbolToken::bypass(1, 1));
        let coeff_rem = x & ((1 << params.m) - 1);
        out.push(BlockSymbolToken::bypass(params.m, coeff_rem));
    } else {
        // Golomb-prefix (`q == cMax`): `cMax` q-zeros (no terminator), the
        // golomb-length unary, then `coeff_rem` as one `L(length)` literal.
        let xmbias = x - params.bias;
        let length = xmbias.ilog2();
        let golomb_zeros = length - params.k;
        let coeff_rem = xmbias - (1 << length);
        for _ in 0..params.c_max {
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
/// emits for the extension `x` with the given golomb parameters, so the trace
/// allocator can reserve exactly. Finite-q (`x < cMax << m`): `q + 2` (q zeros +
/// terminator + the `L(m)` `coeff_rem` literal). Golomb-prefix (`x >= cMax << m`):
/// `cMax + golomb_zeros + 1 (terminator) + 1 (coeff_rem literal)`.
pub(super) fn read_quant_golomb_tail_len(x: u32, params: GolombParams) -> usize {
    if x < params.prefix_x_min {
        (x >> params.m) as usize + 2
    } else {
        let xmbias = x - params.bias;
        let golomb_zeros = xmbias.ilog2() - params.k;
        params.c_max as usize + golomb_zeros as usize + 2
    }
}

/// Reads back the AV2 § 5.20.7.28 `read_quant` golomb tail
/// [`push_read_quant_golomb_tail`] emitted for a golomb coefficient with the given
/// golomb parameters, returning the extension `x = magnitude - maxLevel`. Mirrors the
/// decoder (`crates/splot-decode/src/tile_payload/coeff_loop/read_quant.rs`): the
/// q-length unary capped at `cMax` (finite-q if it terminates before `cMax`, else
/// golomb-prefix), then the `coeff_rem` `L(m)` (finite-q) or `L(length)`
/// (golomb-prefix) literal.
pub(super) fn recover_read_quant_golomb_tail(
    tokens: &[BlockSymbolToken],
    index: &mut usize,
    params: GolombParams,
) -> Result<u32> {
    // q-length unary capped at `cMax`: count zeros until a terminating one or `cMax`.
    let mut q = 0u32;
    let mut terminated = false;
    while q < params.c_max {
        if read_golomb_bit(tokens, index)? {
            terminated = true;
            break;
        }
        q += 1;
    }

    if terminated {
        // Finite-q: `coeff_rem = L(m)`, `x = (q << m) + coeff_rem`.
        let coeff_rem = read_golomb_literal(tokens, index, params.m)?;
        return Ok((q << params.m) + coeff_rem);
    }

    // Golomb-prefix (`q == cMax`): read the golomb-length unary (zeros + a 1) to get
    // `length = golomb_zeros + k`, then `coeff_rem = L(length)`.
    let mut golomb_zeros = 0u32;
    while !read_golomb_bit(tokens, index)? {
        golomb_zeros += 1;
        if golomb_zeros > GOLOMB_PREFIX_LENGTH_MAX - params.k {
            return Err(Error::CoefficientTokenizationMalformedTokenTrace {
                context: "general LF recovery golomb-prefix length out of range",
            });
        }
    }
    let length = golomb_zeros + params.k;
    let coeff_rem = read_golomb_literal(tokens, index, length)?;
    // `x = bias + 2^length + coeff_rem` (`bias = (cMax << m) - (1 << k)`).
    let x = params.bias + (1 << length) + coeff_rem;
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
