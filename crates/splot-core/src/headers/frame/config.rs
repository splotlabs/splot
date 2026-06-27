// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame-configuration helpers for the frame-header core parser
//! (AV2 v1.0.0 § 5.18.3, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-3`).
//!
//! Only `screen_content_params()` (§ 5.18.3.3) and `intrabc_params()` (§ 5.18.3.4)
//! are modeled — the two configuration structures the intra core path reaches.
//! `frame_opfl_refine_type()` (§ 5.18.3.2) and `get_relative_dist()` (§ 5.18.3.1)
//! belong to inter/TIP paths this phase does not parse.

use crate::bitio::BitReader;
use crate::error::Result;

/// `SELECT_SCREEN_CONTENT_TOOLS`: `seq_force_screen_content_tools` value meaning the
/// frame signals `allow_screen_content_tools` (AV2 v1.0.0 § 3 / § 6.4.7).
const SELECT_SCREEN_CONTENT_TOOLS: u8 = 2;

/// `SELECT_INTEGER_MV`: `seq_force_integer_mv` value meaning the frame signals
/// `force_integer_mv` (AV2 v1.0.0 § 3 / § 6.4.7).
const SELECT_INTEGER_MV: u8 = 2;

/// The two flags `screen_content_params()` derives (AV2 v1.0.0 § 5.18.3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScreenContentParams {
    /// `allow_screen_content_tools`.
    pub allow_screen_content_tools: bool,
    /// `force_integer_mv` (`0` when screen-content tools are off, else read or forced).
    /// The inter MV-precision block (§ 5.18.2 mirror :4885) gates on it.
    pub force_integer_mv: bool,
}

/// Parses `screen_content_params()` (AV2 v1.0.0 § 5.18.3.3) and returns both
/// `allow_screen_content_tools` and `force_integer_mv`.
///
/// `seq_force_screen_content_tools` and `seq_force_integer_mv` come from the active
/// sequence's `sequence_scc_config()` (§ 5.4.7). When a field is forced by the
/// sequence, no bit is read; only the `SELECT_*` sentinel reads a flag.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if a signaled
/// flag cannot be read.
pub(crate) fn parse_screen_content_params_full(
    reader: &mut BitReader<'_>,
    seq_force_screen_content_tools: u8,
    seq_force_integer_mv: u8,
) -> Result<ScreenContentParams> {
    // AV2 § 5.18.3.3.
    let allow_screen_content_tools =
        if seq_force_screen_content_tools == SELECT_SCREEN_CONTENT_TOOLS {
            reader.read_flag()?
        } else {
            seq_force_screen_content_tools != 0
        };

    // AV2 § 5.18.3.3: force_integer_mv is read only when screen-content tools are on and
    // the sequence selects it; otherwise it is forced (0, or the sequence value).
    let force_integer_mv = if allow_screen_content_tools {
        if seq_force_integer_mv == SELECT_INTEGER_MV {
            reader.read_flag()?
        } else {
            seq_force_integer_mv != 0
        }
    } else {
        false
    };

    Ok(ScreenContentParams {
        allow_screen_content_tools,
        force_integer_mv,
    })
}

/// Every field `intrabc_params()` reads (AV2 v1.0.0 § 5.18.3.4). Each conditionally-read
/// field is `Some` exactly when the bit was present in the bitstream, so the structure is a
/// faithful, byte-exact record of the syntax (consumed by the § 5.18.3.4 writer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntrabcParams {
    /// `allow_intrabc` `f(1)` (always read).
    pub allow_intrabc: bool,
    /// `allow_global_intrabc` `f(1)`, read when `allow_intrabc && frame_is_intra`.
    pub allow_global_intrabc: Option<bool>,
    /// `allow_local_intrabc` `f(1)`, read only when `allow_global_intrabc == 1`
    /// (otherwise inferred `1`, no bit).
    pub allow_local_intrabc: Option<bool>,
    /// `change_bvp_drl` `f(1)`, read when `allow_intrabc && allow_frame_max_bvp_drl_bits`.
    pub change_bvp_drl: Option<bool>,
    /// `max_bvp_drl_bits_minus_1` `ns(2)`, read when `change_bvp_drl == 1`.
    pub max_bvp_drl_bits_minus_1: Option<u32>,
}

/// Parses `intrabc_params()` (AV2 v1.0.0 § 5.18.3.4) and returns every field it reads.
///
/// `frame_is_intra` is `FrameIsIntra`; `allow_frame_max_bvp_drl_bits` comes from the
/// active sequence's `sequence_inter_config()` (§ 5.4.6). The conditionally-read
/// `allow_global_intrabc` / `allow_local_intrabc` / `change_bvp_drl` /
/// `max_bvp_drl_bits_minus_1` are `Some` exactly when their bit was present, so the result
/// records the syntax byte-for-byte even though the decode process derives nothing from them
/// on the modeled path.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) or
/// [`Error::InvalidNs`](crate::error::Error::InvalidNs) if a signaled field is
/// truncated.
pub(crate) fn parse_intrabc_params_full(
    reader: &mut BitReader<'_>,
    frame_is_intra: bool,
    allow_frame_max_bvp_drl_bits: bool,
) -> Result<IntrabcParams> {
    // AV2 § 5.18.3.4.
    let allow_intrabc = reader.read_flag()?;
    let mut params = IntrabcParams {
        allow_intrabc,
        allow_global_intrabc: None,
        allow_local_intrabc: None,
        change_bvp_drl: None,
        max_bvp_drl_bits_minus_1: None,
    };
    if allow_intrabc {
        if frame_is_intra {
            let allow_global_intrabc = reader.read_flag()?;
            params.allow_global_intrabc = Some(allow_global_intrabc);
            if allow_global_intrabc {
                params.allow_local_intrabc = Some(reader.read_flag()?);
            }
            // else: allow_local_intrabc = 1 (inferred, no bit).
        }

        if allow_frame_max_bvp_drl_bits {
            let change_bvp_drl = reader.read_flag()?;
            params.change_bvp_drl = Some(change_bvp_drl);
            if change_bvp_drl {
                params.max_bvp_drl_bits_minus_1 = Some(reader.read_ns(2)?);
            }
        }
    }

    Ok(params)
}

/// Parses `intrabc_params()` (AV2 v1.0.0 § 5.18.3.4) and returns `allow_intrabc` only. A
/// thin wrapper over [`parse_intrabc_params_full`] for callers that do not surface the
/// remaining fields (the inter control region).
///
/// `frame_is_intra` is `FrameIsIntra`; `allow_frame_max_bvp_drl_bits` comes from the
/// active sequence's `sequence_inter_config()` (§ 5.4.6).
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) or
/// [`Error::InvalidNs`](crate::error::Error::InvalidNs) if a signaled field is
/// truncated.
pub(crate) fn parse_intrabc_params(
    reader: &mut BitReader<'_>,
    frame_is_intra: bool,
    allow_frame_max_bvp_drl_bits: bool,
) -> Result<bool> {
    Ok(
        parse_intrabc_params_full(reader, frame_is_intra, allow_frame_max_bvp_drl_bits)?
            .allow_intrabc,
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::span::ByteOffset;

    use crate::test_bits::Bits;

    #[test]
    fn screen_content_forced_reads_no_bits() {
        // seq_force_screen_content_tools = 0 (forced off) -> no flag bit.
        let data = [0u8; 0];
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let allow = parse_screen_content_params_full(&mut reader, 0, 0)
            .unwrap()
            .allow_screen_content_tools;
        assert!(!allow);
        assert_eq!(reader.consumed_bits(), 0);
    }

    #[test]
    fn screen_content_select_reads_flag_and_force_integer_mv() {
        // SELECT for both: allow_screen_content_tools=1 then force_integer_mv=1.
        let mut bits = Bits::default();
        bits.bit(1); // allow_screen_content_tools
        bits.bit(1); // force_integer_mv
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let allow = parse_screen_content_params_full(
            &mut reader,
            SELECT_SCREEN_CONTENT_TOOLS,
            SELECT_INTEGER_MV,
        )
        .unwrap()
        .allow_screen_content_tools;
        assert!(allow);
        assert_eq!(reader.consumed_bits(), 2);
    }

    #[test]
    fn screen_content_select_tools_off_skips_force_integer_mv() {
        // allow_screen_content_tools=0 -> force_integer_mv not read.
        let mut bits = Bits::default();
        bits.bit(0); // allow_screen_content_tools
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let allow = parse_screen_content_params_full(
            &mut reader,
            SELECT_SCREEN_CONTENT_TOOLS,
            SELECT_INTEGER_MV,
        )
        .unwrap()
        .allow_screen_content_tools;
        assert!(!allow);
        assert_eq!(reader.consumed_bits(), 1);
    }

    #[test]
    fn intrabc_disallowed_reads_one_bit() {
        let mut bits = Bits::default();
        bits.bit(0); // allow_intrabc = 0
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let allow = parse_intrabc_params(&mut reader, true, false).unwrap();
        assert!(!allow);
        assert_eq!(reader.consumed_bits(), 1);
    }

    #[test]
    fn intrabc_intra_global_reads_local_flag() {
        // allow_intrabc=1, FrameIsIntra: allow_global_intrabc=1 -> allow_local_intrabc f(1).
        let mut bits = Bits::default();
        bits.bit(1); // allow_intrabc
        bits.bit(1); // allow_global_intrabc
        bits.bit(0); // allow_local_intrabc
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let allow = parse_intrabc_params(&mut reader, true, false).unwrap();
        assert!(allow);
        assert_eq!(reader.consumed_bits(), 3);
    }

    #[test]
    fn intrabc_change_bvp_drl_reads_ns() {
        // allow_intrabc=1, intra, allow_global=0; allow_frame_max_bvp_drl_bits=1,
        // change_bvp_drl=1 -> max_bvp_drl_bits_minus_1 ns(2).
        let mut bits = Bits::default();
        bits.bit(1); // allow_intrabc
        bits.bit(0); // allow_global_intrabc (-> allow_local_intrabc inferred 1, no bit)
        bits.bit(1); // change_bvp_drl
        bits.bit(0); // ns(2): first bit 0 -> value 0 (w=2, m=4-2=2; v read as 1 bit = 0 < 2)
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let allow = parse_intrabc_params(&mut reader, true, true).unwrap();
        assert!(allow);
        assert_eq!(reader.consumed_bits(), 4);
    }

    #[test]
    fn intrabc_full_surfaces_every_read_field() {
        // allow_intrabc=1, intra, allow_global_intrabc=1 -> allow_local_intrabc read;
        // allow_frame_max_bvp_drl_bits=1, change_bvp_drl=1 -> max_bvp_drl_bits_minus_1 ns(2).
        let mut bits = Bits::default();
        bits.bit(1); // allow_intrabc
        bits.bit(1); // allow_global_intrabc
        bits.bit(0); // allow_local_intrabc
        bits.bit(1); // change_bvp_drl
        bits.bit(0); // max_bvp_drl_bits_minus_1 ns(2) -> 0
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let p = parse_intrabc_params_full(&mut reader, true, true).unwrap();
        assert_eq!(
            p,
            IntrabcParams {
                allow_intrabc: true,
                allow_global_intrabc: Some(true),
                allow_local_intrabc: Some(false),
                change_bvp_drl: Some(true),
                max_bvp_drl_bits_minus_1: Some(0),
            }
        );
        assert_eq!(reader.consumed_bits(), 5);
    }

    #[test]
    fn intrabc_full_global_off_infers_local_no_bit() {
        // allow_global_intrabc=0 -> allow_local_intrabc inferred (None, no bit); no DRL.
        let mut bits = Bits::default();
        bits.bit(1); // allow_intrabc
        bits.bit(0); // allow_global_intrabc -> allow_local_intrabc inferred 1, no bit
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let p = parse_intrabc_params_full(&mut reader, true, false).unwrap();
        assert_eq!(
            p,
            IntrabcParams {
                allow_intrabc: true,
                allow_global_intrabc: Some(false),
                allow_local_intrabc: None,
                change_bvp_drl: None,
                max_bvp_drl_bits_minus_1: None,
            }
        );
        assert_eq!(reader.consumed_bits(), 2);
    }

    #[test]
    fn intrabc_eof_is_structured_error() {
        let data = [0u8; 0];
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            parse_intrabc_params(&mut reader, true, false),
            Err(Error::UnexpectedEof { .. })
        ));
    }
}
