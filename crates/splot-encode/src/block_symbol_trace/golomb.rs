// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The §5.20.7.28 golomb-tail block-trace composers (finite-q and
//! golomb-prefix) for a larger coded luma DC coefficient. Split out of
//! `block_symbol_trace` to keep the parent file under the 1000-line budget.

use super::*;

/// Composes the minimal ordered intra DC coded *golomb-tail* block trace: the AV2
/// § 5.20.5.3 mode-info prefix, then the luma `residual()` for a single DC
/// coefficient whose level reaches `maxLevel` (the fixed `txb_skip=0` /
/// `eob_pt_16` / `coeff_base_eob=LF_NUM_BASE_LEVELS` / `coeff_br=COEFF_BASE_RANGE`
/// level tokens), then the luma `dc_sign` CDF token, then the § 5.20.7.28
/// `read_quant` finite-q golomb `coeff_rem` bypass bits encoding
/// `x = magnitude - maxLevel` (§ 5.20.7.27's sign+quant pass reads the sign before
/// calling `read_quant`), then the all-zero U and V `txb_skip` (the U plane is
/// all-zero, so V uses the neutral context 0).
///
/// For the value `+10` the golomb extension is `x = 2`: with `m = 1` (the first DC
/// coefficient, `hrLevelAvg = 0`) the finite-q path is `q = x >> 1 = 1`,
/// `coeff_rem = x & 1 = 0`, i.e. the bypass bits `0` (one `q_length_bit` zero),
/// `1` (the terminating `q_length_bit`), `0` (`coeff_rem`). This covers the
/// finite-q magnitude range `maxLevel..=maxLevel + 9` (8..=17); the golomb-prefix
/// path (magnitude 18+) is a later brick.
pub(crate) fn compose_minimal_intra_dc_golomb_block_trace() -> Result<Vec<BlockSymbolToken>> {
    compose_intra_dc_golomb_block_trace(MINIMAL_GOLOMB_DC_MAGNITUDE, MINIMAL_GOLOMB_DC_NEGATIVE)
}

/// Composes the intra DC coded golomb-tail block trace for any finite-q luma DC
/// `magnitude` in `GOLOMB_MAXLEVEL..=GOLOMB_FINITE_Q_MAGNITUDE_MAX` (8..=17). The
/// level tokens are identical across the tier (the level always saturates to
/// `maxLevel`); only the § 5.20.7.28 golomb `q_length`/`coeff_rem` bypass bits
/// vary with `x = magnitude - maxLevel` (`q = x >> 1`, `coeff_rem = x & 1`, `m = 1`
/// for the first DC coefficient). The `dc_sign` CDF token precedes the golomb bits
/// (§ 5.20.7.27's sign+quant pass reads the sign before calling `read_quant`).
pub(crate) fn compose_intra_dc_golomb_block_trace(
    magnitude: u32,
    negative: bool,
) -> Result<Vec<BlockSymbolToken>> {
    if !(GOLOMB_MAXLEVEL..=GOLOMB_FINITE_Q_MAGNITUDE_MAX).contains(&magnitude) {
        return Err(Error::BlockSymbolTraceGolombMagnitudeOutOfRange {
            magnitude,
            min: GOLOMB_MAXLEVEL,
            max: GOLOMB_FINITE_Q_MAGNITUDE_MAX,
        });
    }
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let level = luma_dc_golomb_level_tokens(MINIMAL_COEFF_CDF_Q_CTX)?;
    let x = magnitude - GOLOMB_MAXLEVEL;
    let q = x >> GOLOMB_DC_M;
    let coeff_rem = x & 1;
    let golomb_bits =
        (q as usize)
            .checked_add(2)
            .ok_or(Error::BlockSymbolTraceAllocationFailed {
                context: "golomb block trace length",
            })?;
    let total = modes
        .len()
        .checked_add(level.len())
        .and_then(|n| n.checked_add(golomb_bits))
        .and_then(|n| n.checked_add(3)) // dc_sign + U + V all-zero
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "golomb block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "golomb block trace",
        })?;
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.extend(level.into_iter().map(BlockSymbolToken::Coeff));
    trace.push(BlockSymbolToken::Coeff(luma_dc_sign_token(
        MINIMAL_COEFF_CDF_Q_CTX,
        negative,
    )));
    for _ in 0..q {
        trace.push(BlockSymbolToken::bypass(1, 0));
    }
    trace.push(BlockSymbolToken::bypass(1, 1));
    trace.push(BlockSymbolToken::bypass(1, coeff_rem));
    trace.push(BlockSymbolToken::Coeff(chroma_u_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
    )));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
        V_TXB_SKIP_CTX_NEUTRAL,
    )));
    Ok(trace)
}

/// Composes the canonical minimal intra DC coded golomb-*prefix* block trace
/// (magnitude +18, the smallest golomb-prefix coefficient).
pub(crate) fn compose_minimal_intra_dc_golomb_prefix_block_trace() -> Result<Vec<BlockSymbolToken>>
{
    compose_intra_dc_golomb_prefix_block_trace(
        MINIMAL_GOLOMB_PREFIX_DC_MAGNITUDE,
        MINIMAL_GOLOMB_PREFIX_DC_NEGATIVE,
    )
}

/// Composes the intra DC coded golomb-*prefix* block trace for any luma DC
/// `magnitude` in `GOLOMB_PREFIX_MAGNITUDE_MIN..=GOLOMB_PREFIX_MAGNITUDE_MAX`
/// (18..=525). This is the AV2 § 5.20.7.28 `read_quant` golomb-prefix path
/// (`q == cMax`): the mode prefix, the fixed golomb level tokens, the luma
/// `dc_sign` CDF token (the sign precedes `read_quant`), then the golomb-prefix
/// bypass bits — `cMax` (5) `q_length` zeros, the `golomb_length` unary
/// (`golomb_zeros` zeros and a terminating 1, `length = golomb_zeros + k`), and
/// `coeff_rem` as one `L(length)` literal — then all-zero U/V `txb_skip`.
///
/// Encoding `x = magnitude - maxLevel` (`x >= 10`): `length = GetMsb(x - 6)`,
/// `golomb_zeros = length - k`, `coeff_rem = (x - 6) - 2^length`,
/// `xBase = 6 + 2^length`. For magnitude 18: `x = 10`, `length = 2`,
/// `golomb_zeros = 0`, `coeff_rem = 0` — the 17-token trace
/// `[0,0,0, 0,0,4,3, 0, 0,0,0,0,0, 1, 0, 1,1]`.
pub(crate) fn compose_intra_dc_golomb_prefix_block_trace(
    magnitude: u32,
    negative: bool,
) -> Result<Vec<BlockSymbolToken>> {
    if !(GOLOMB_PREFIX_MAGNITUDE_MIN..=GOLOMB_PREFIX_MAGNITUDE_MAX).contains(&magnitude) {
        return Err(Error::BlockSymbolTraceGolombMagnitudeOutOfRange {
            magnitude,
            min: GOLOMB_PREFIX_MAGNITUDE_MIN,
            max: GOLOMB_PREFIX_MAGNITUDE_MAX,
        });
    }
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let level = luma_dc_golomb_level_tokens(MINIMAL_COEFF_CDF_Q_CTX)?;
    let x = magnitude - GOLOMB_MAXLEVEL;
    let xm6 = x - GOLOMB_PREFIX_XBASE_BIAS;
    let length = xm6.ilog2();
    let golomb_zeros = length - GOLOMB_DC_K;
    let coeff_rem = xm6 - (1 << length);
    let golomb_bits = (GOLOMB_PREFIX_Q_ZEROS as usize)
        .checked_add(golomb_zeros as usize)
        .and_then(|n| n.checked_add(2))
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "golomb-prefix block trace length",
        })?;
    let total = modes
        .len()
        .checked_add(level.len())
        .and_then(|n| n.checked_add(golomb_bits))
        .and_then(|n| n.checked_add(3)) // dc_sign + U + V all-zero
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "golomb-prefix block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "golomb-prefix block trace",
        })?;
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.extend(level.into_iter().map(BlockSymbolToken::Coeff));
    trace.push(BlockSymbolToken::Coeff(luma_dc_sign_token(
        MINIMAL_COEFF_CDF_Q_CTX,
        negative,
    )));
    for _ in 0..GOLOMB_PREFIX_Q_ZEROS {
        trace.push(BlockSymbolToken::bypass(1, 0));
    }
    for _ in 0..golomb_zeros {
        trace.push(BlockSymbolToken::bypass(1, 0));
    }
    trace.push(BlockSymbolToken::bypass(1, 1));
    trace.push(BlockSymbolToken::bypass(length, coeff_rem));
    trace.push(BlockSymbolToken::Coeff(chroma_u_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
    )));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
        V_TXB_SKIP_CTX_NEUTRAL,
    )));
    Ok(trace)
}
