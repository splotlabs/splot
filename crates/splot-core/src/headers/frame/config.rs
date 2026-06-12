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
            reader.read_bit()? != 0
        } else {
            seq_force_screen_content_tools != 0
        };

    // AV2 § 5.18.3.3: force_integer_mv is read only when screen-content tools are on and
    // the sequence selects it; otherwise it is forced (0, or the sequence value).
    let force_integer_mv = if allow_screen_content_tools {
        if seq_force_integer_mv == SELECT_INTEGER_MV {
            reader.read_bit()? != 0
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

/// Parses `screen_content_params()` (AV2 v1.0.0 § 5.18.3.3) and returns
/// `allow_screen_content_tools` only (the intra path does not consume `force_integer_mv`,
/// since `FrameIsIntra` skips the MV-precision block). A thin wrapper over
/// [`parse_screen_content_params_full`].
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if a signaled
/// flag cannot be read.
pub(crate) fn parse_screen_content_params(
    reader: &mut BitReader<'_>,
    seq_force_screen_content_tools: u8,
    seq_force_integer_mv: u8,
) -> Result<bool> {
    Ok(parse_screen_content_params_full(
        reader,
        seq_force_screen_content_tools,
        seq_force_integer_mv,
    )?
    .allow_screen_content_tools)
}

/// Parses `intrabc_params()` (AV2 v1.0.0 § 5.18.3.4) and returns `allow_intrabc`.
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
    // AV2 § 5.18.3.4.
    let allow_intrabc = reader.read_bit()? != 0;
    if allow_intrabc {
        if frame_is_intra {
            let allow_global_intrabc = reader.read_bit()? != 0;
            if allow_global_intrabc {
                // allow_local_intrabc f(1): read to stay bit-aligned (value not surfaced).
                reader.read_bit()?;
            }
        }
        // else: allow_global_intrabc = 0, allow_local_intrabc = 1 (no bits read).

        if allow_frame_max_bvp_drl_bits {
            let change_bvp_drl = reader.read_bit()? != 0;
            if change_bvp_drl {
                // max_bvp_drl_bits_minus_1 ns(2): read for alignment only; its value
                // (and the +1 adjustment against the sequence default) gates DRL syntax
                // this phase stops before, so it is not surfaced.
                let _raw = reader.read_ns(2)?;
            }
        }
    }

    Ok(allow_intrabc)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::span::ByteOffset;

    #[derive(Default)]
    struct Bits {
        bits: Vec<u8>,
    }

    impl Bits {
        fn bit(&mut self, bit: u8) {
            self.bits.push(bit & 1);
        }

        fn into_bytes(self) -> Vec<u8> {
            let mut bytes = Vec::new();
            for chunk in self.bits.chunks(8) {
                let mut byte = 0u8;
                for (i, bit) in chunk.iter().enumerate() {
                    byte |= *bit << (7 - i);
                }
                bytes.push(byte);
            }
            bytes
        }
    }

    #[test]
    fn screen_content_forced_reads_no_bits() {
        // seq_force_screen_content_tools = 0 (forced off) -> no flag bit.
        let data = [0u8; 0];
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let allow = parse_screen_content_params(&mut reader, 0, 0).unwrap();
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
        let allow = parse_screen_content_params(
            &mut reader,
            SELECT_SCREEN_CONTENT_TOOLS,
            SELECT_INTEGER_MV,
        )
        .unwrap();
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
        let allow = parse_screen_content_params(
            &mut reader,
            SELECT_SCREEN_CONTENT_TOOLS,
            SELECT_INTEGER_MV,
        )
        .unwrap();
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
    fn intrabc_eof_is_structured_error() {
        let data = [0u8; 0];
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            parse_intrabc_params(&mut reader, true, false),
            Err(Error::UnexpectedEof { .. })
        ));
    }
}
