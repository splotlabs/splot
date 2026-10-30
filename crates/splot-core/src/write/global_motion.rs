// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Writer of `read_global_param()` (AV2 § 5.18.9.2 to § 5.18.9.6), the
//! inverse of [`crate::headers::frame::read_global_param`]: a warp coefficient
//! coded as a sub-exponential difference from its predicted value.

use crate::headers::frame::global_motion::{
    GLOBAL_PARAM_K, GM_ALPHA_MAX, GM_ALPHA_PREC_BITS, GM_TRANS_MAX, GM_TRANS_PREC_BITS,
    WARPEDMODEL_PREC_BITS,
};
use crate::write::{BitWriter, WriteError, WriteResult};

/// Writes `gm_params[ ref ][ idx ] = value` predicted from `prev_gm_param`
/// (`PrevGmParams[ ref ][ idx ]`), the inverse of `read_global_param`.
///
/// # Errors
/// [`WriteError::NonCanonicalFrameHeader`] for a value off the coefficient's
/// precision lattice or outside its range.
pub fn write_global_param(
    writer: &mut BitWriter,
    idx: usize,
    prev_gm_param: i32,
    value: i32,
) -> WriteResult<()> {
    let (prec_bits, mx) = if idx < 2 {
        (GM_TRANS_PREC_BITS, GM_TRANS_MAX)
    } else {
        (GM_ALPHA_PREC_BITS, GM_ALPHA_MAX)
    };
    let prec_diff = WARPEDMODEL_PREC_BITS - prec_bits;
    let scale_term = idx % 3 == 2;
    let round = if scale_term {
        1i32 << WARPEDMODEL_PREC_BITS
    } else {
        0
    };
    let sub = if scale_term { 1i32 << prec_bits } else { 0 };
    let off_lattice = WriteError::NonCanonicalFrameHeader { what: "gm_params" };
    let shifted = value.checked_sub(round).ok_or(off_lattice.clone())?;
    let decoded = shifted >> prec_diff;
    if decoded << prec_diff != shifted || !(-mx..=mx).contains(&decoded) {
        return Err(off_lattice);
    }
    let r = (prev_gm_param >> prec_diff) - sub;
    let (low, high) = (-mx, mx + 1);
    let num_syms = high - low;
    let x = decoded - low;
    let r = (r - low).clamp(0, num_syms - 1);
    let v = if r <= num_syms / 2 {
        recenter(r, x)
    } else {
        recenter(num_syms - 1 - r, num_syms - 1 - x)
    };
    write_subexp(writer, num_syms, GLOBAL_PARAM_K, v)
}

/// The inverse of `inverse_recenter( r, v )` (§ 5.18.9.6).
const fn recenter(r: i32, v: i32) -> i32 {
    if v > 2 * r {
        v
    } else if v >= r {
        (v - r) << 1
    } else {
        ((r - v) << 1) - 1
    }
}

/// Writes `v` as `decode_subexp( num_syms, k )` reads it (§ 5.18.9.5).
fn write_subexp(writer: &mut BitWriter, num_syms: i32, k: u32, v: i32) -> WriteResult<()> {
    let (num_syms, v) = (num_syms.unsigned_abs(), v.unsigned_abs());
    let (mut i, mut mk) = (0u32, 0u32);
    loop {
        let b2 = if i == 0 { k } else { k + i - 1 };
        let a = 1u32 << b2;
        if num_syms <= mk + 3 * a {
            return writer.write_ns(v - mk, num_syms - mk);
        }
        let more = v >= mk + a;
        writer.write_flag(more)?;
        if !more {
            return writer.write_bits(v - mk, b2);
        }
        i += 1;
        mk += a;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::frame::read_global_param;
    use crate::span::ByteOffset;

    /// Every lattice value of every coefficient kind, predicted from values
    /// across, below, above and at the ends of the range, reads back.
    #[test]
    fn global_params_round_trip_through_the_reader() {
        for idx in 0..6 {
            let (prec_bits, mx) = if idx < 2 {
                (GM_TRANS_PREC_BITS, GM_TRANS_MAX)
            } else {
                (GM_ALPHA_PREC_BITS, GM_ALPHA_MAX)
            };
            let prec_diff = WARPEDMODEL_PREC_BITS - prec_bits;
            let round = if idx % 3 == 2 { 1 << 16 } else { 0 };
            let at = |d: i32| (d << prec_diff) + round;
            for prev in [at(0), at(-mx), at(mx), at(mx / 3), at(-mx / 2 + 1), at(7)] {
                for decoded in (-mx..=mx).step_by(13).chain([-mx, mx, 0, 1, -1]) {
                    let mut writer = BitWriter::new();
                    write_global_param(&mut writer, idx, prev, at(decoded)).unwrap();
                    let bits = writer.bit_len();
                    writer.align_to_byte();
                    let bytes = writer.into_bytes();
                    let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
                    let read = read_global_param(&mut reader, idx, prev).unwrap();
                    assert_eq!(read, at(decoded), "idx {idx} prev {prev}");
                    assert_eq!(reader.consumed_bits(), bits, "idx {idx} prev {prev}");
                }
            }
        }
    }

    /// Values off the coefficient lattice or past its range are refused.
    #[test]
    fn rejects_values_the_reader_cannot_produce() {
        let mut writer = BitWriter::new();
        assert!(write_global_param(&mut writer, 0, 0, 1).is_err());
        assert!(write_global_param(&mut writer, 2, 1 << 16, (1 << 16) + (1 << 20)).is_err());
    }
}
