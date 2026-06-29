// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 buffer removal timing OBU syntax model (AV2 v1.0.0 § 5.12).
//!
//! [`parse_buffer_removal_timing`] reads `buffer_removal_timing_obu()`. The OBU has
//! two forms selected by `br_ops_dependent_flag`:
//!
//! - `br_ops_dependent_flag == 0`: a single extended-layer removal time `br_time`.
//! - `br_ops_dependent_flag == 1`: timing for a specific operating point set,
//!   identified by `br_ops_id` with `br_ops_cnt` per-operating-point entries.
//!
//! `OBU_BUFFER_REMOVAL_TIMING` is **not** an extensible OBU, so its dispatcher uses
//! `trailing_bits()` directly (no `obu_extension_flag`). Full Annex E decoder
//! schedule / resource conformance is out of scope here; this module models the
//! syntax and preserves the values the validator needs for the § 6.11 reference
//! checks (`br_ops_id`, `br_ops_cnt`).

use crate::bitio::BitReader;
use crate::error::Result;

/// `br_ops_id` is a 4-bit field (`f(4)`).
const BR_OPS_ID_BITS: u32 = 4;
/// `br_ops_cnt` is a 3-bit field (`f(3)`).
const BR_OPS_COUNT_BITS: u32 = 3;
/// `br_time` / `br_time_op` use a `rg(4)` Rice-Golomb code.
const BR_TIME_RG_ORDER: u32 = 4;

/// Parsed `buffer_removal_timing_obu()` syntax (AV2 v1.0.0 § 5.12).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BufferRemovalTiming {
    /// `br_ops_dependent_flag == 0`: a single extended-layer removal time.
    ExtendedLayer {
        /// `br_time` (`rg(4)`): the frame removal time in `DecCT` clock ticks.
        br_time: u32,
    },
    /// `br_ops_dependent_flag == 1`: per-operating-point timing for the OPS
    /// `br_ops_id`.
    OperatingPointSet {
        /// `br_ops_id` (`f(4)`): the referenced operating point set id.
        br_ops_id: u8,
        /// `br_ops_cnt` (`f(3)`): the operating point count, which must equal the
        /// referenced OPS `ops_cnt` (§ 6.11).
        br_ops_cnt: u8,
        /// One entry per operating point, in index order.
        op_times: Vec<BufferRemovalOpTiming>,
    },
}

impl BufferRemovalTiming {
    /// Returns `true` for the OPS-dependent form (`br_ops_dependent_flag == 1`).
    #[must_use]
    pub fn is_ops_dependent(&self) -> bool {
        matches!(self, Self::OperatingPointSet { .. })
    }

    /// Returns `(br_ops_id, br_ops_cnt)` for the OPS-dependent form, else `None`.
    ///
    /// Lets callers in other crates resolve the referenced OPS without matching the
    /// `#[non_exhaustive]` enum.
    #[must_use]
    pub fn ops_reference(&self) -> Option<(u8, u8)> {
        match self {
            Self::OperatingPointSet {
                br_ops_id,
                br_ops_cnt,
                ..
            } => Some((*br_ops_id, *br_ops_cnt)),
            Self::ExtendedLayer { .. } => None,
        }
    }

    /// Returns `br_time` for the extended-layer form, else `None`.
    #[must_use]
    pub fn extended_layer_time(&self) -> Option<u32> {
        match self {
            Self::ExtendedLayer { br_time } => Some(*br_time),
            Self::OperatingPointSet { .. } => None,
        }
    }

    /// Returns the per-operating-point timing entries (empty for the extended-layer
    /// form).
    #[must_use]
    pub fn op_timings(&self) -> &[BufferRemovalOpTiming] {
        match self {
            Self::OperatingPointSet { op_times, .. } => op_times,
            Self::ExtendedLayer { .. } => &[],
        }
    }
}

/// One per-operating-point entry of an OPS-dependent buffer removal timing OBU
/// (AV2 v1.0.0 § 5.12).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BufferRemovalOpTiming {
    /// Operating point index `i` within the referenced OPS.
    pub index: u8,
    /// `br_decoder_model_present_op_flag` (`f(1)`): whether `br_time_op` is present.
    pub decoder_model_present: bool,
    /// `br_time_op` (`rg(4)`): present when `decoder_model_present` is set.
    pub br_time_op: Option<u32>,
}

/// Parses a `buffer_removal_timing_obu()` (AV2 v1.0.0 § 5.12).
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) or
/// [`Error::InvalidRg`](crate::error::Error::InvalidRg) from [`BitReader`] when the
/// input is truncated or a `rg(4)` code is malformed.
pub fn parse_buffer_removal_timing(reader: &mut BitReader<'_>) -> Result<BufferRemovalTiming> {
    let ops_dependent = reader.read_flag()?;
    if ops_dependent {
        let br_ops_id = reader.read_bits_u8(BR_OPS_ID_BITS)?;
        let br_ops_cnt = reader.read_bits_u8(BR_OPS_COUNT_BITS)?;
        let mut op_times = Vec::new();
        for index in 0..br_ops_cnt {
            let decoder_model_present = reader.read_flag()?;
            let br_time_op = if decoder_model_present {
                Some(reader.read_rg(BR_TIME_RG_ORDER)?)
            } else {
                None
            };
            op_times.push(BufferRemovalOpTiming {
                index,
                decoder_model_present,
                br_time_op,
            });
        }
        Ok(BufferRemovalTiming::OperatingPointSet {
            br_ops_id,
            br_ops_cnt,
            op_times,
        })
    } else {
        let br_time = reader.read_rg(BR_TIME_RG_ORDER)?;
        Ok(BufferRemovalTiming::ExtendedLayer { br_time })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::span::ByteOffset;

    use crate::test_bits::Bits;

    fn parse(bytes: &[u8]) -> Result<BufferRemovalTiming> {
        let mut reader = BitReader::new(bytes, ByteOffset::new(0));
        parse_buffer_removal_timing(&mut reader)
    }

    #[test]
    fn brt_extended_layer_time_parses() {
        let mut bits = Bits::default();
        bits.bit(0); // br_ops_dependent_flag = 0
        bits.rg(42, 4); // br_time
        let data = bits.into_bytes();
        let brt = parse(&data).unwrap();
        assert_eq!(brt, BufferRemovalTiming::ExtendedLayer { br_time: 42 });
        assert!(!brt.is_ops_dependent());
    }

    #[test]
    fn brt_ops_dependent_times_parse() {
        let mut bits = Bits::default();
        bits.bit(1); // br_ops_dependent_flag = 1
        bits.f(3, 4); // br_ops_id
        bits.f(2, 3); // br_ops_cnt = 2
        bits.bit(1); // op 0: decoder model present
        bits.rg(7, 4); // op 0: br_time_op
        bits.bit(0); // op 1: decoder model absent
        let data = bits.into_bytes();
        let brt = parse(&data).unwrap();
        match brt {
            BufferRemovalTiming::OperatingPointSet {
                br_ops_id,
                br_ops_cnt,
                op_times,
            } => {
                assert_eq!(br_ops_id, 3);
                assert_eq!(br_ops_cnt, 2);
                assert_eq!(op_times.len(), 2);
                assert!(op_times[0].decoder_model_present);
                assert_eq!(op_times[0].br_time_op, Some(7));
                assert!(!op_times[1].decoder_model_present);
                assert_eq!(op_times[1].br_time_op, None);
            }
            BufferRemovalTiming::ExtendedLayer { .. } => panic!("expected ops-dependent BRT"),
        }
    }

    #[test]
    fn brt_truncated_is_error_not_panic() {
        let mut bits = Bits::default();
        bits.bit(1); // br_ops_dependent_flag
        bits.f(0, 4); // br_ops_id
        bits.f(1, 3); // br_ops_cnt = 1
        let data = bits.into_bytes();
        assert_eq!(data.len(), 1);
        assert!(parse(&data).is_err());
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    proptest! {
        /// The BRT parser must never panic on arbitrary input.
        #[test]
        fn buffer_removal_timing_parser_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..128),
        ) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_buffer_removal_timing(&mut reader);
        }
    }
}
