// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 Annex A profile/level/tier static-constraint tables and helpers
//! (AV2 v1.0.0 Annex A,
//! `docs/spec/av2/1.0.0/annex-a-profiles-levels-and-tiers.md`).
//!
//! This module holds profile definitions and value-space checks. Frame limits
//! come from [`splot_core::level`] so decode and validation use the same levels.
//!
//! The check wiring that consumes these tables lives in
//! [`crate::context`]; the rule semantics and the Table A.4 interoperability-point
//! presence requirements are documented there. Feature IDs: `AV2-A-PROFILES`,
//! `AV2-A-LEVELS-TIERS`.

/// The Configurable profile `seq_profile_idc` (Annex A.2 Table A.1, mirror line 87:
/// "Configurable 31"). Its chroma/bit-depth constraints are unconstrained by the
/// profile (Table A.1 dashes), so the profile-mismatch checks skip it.
pub(crate) const CONFIGURABLE_PROFILE_IDC: u8 = 31;

/// First reserved `seq_level_idx` value (Annex A.4 Table A.7, mirror line 321:
/// "22-30 Reserved").
pub(crate) const FIRST_RESERVED_LEVEL_IDX: u8 = 22;
/// Last reserved `seq_level_idx` value (Annex A.4 Table A.7, mirror line 321:
/// "22-30 Reserved").
pub(crate) const LAST_RESERVED_LEVEL_IDX: u8 = 30;
/// The minimum conformant `FrameWidth` / `FrameHeight` (Annex A.4 static
/// conformance block, mirror lines 628-629: "FrameWidth is greater than or equal to
/// 16" and "FrameHeight is greater than or equal to 16").
pub(crate) const MIN_FRAME_DIMENSION: u32 = 16;

pub(crate) use splot_core::level::level_limits;

/// The interoperability point a profile signals (Annex A.2 Table A.1 column
/// "Interoperability point", mirror lines 61-91).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InteroperabilityPoint {
    /// IOP 0 (profile 0, Table A.1 line 64). Single extended layer, single embedded
    /// layer.
    Iop0,
    /// IOP 1 (profiles 1, 3, 4, Table A.1 lines 67/73/81).
    Iop1,
    /// IOP 2 (profile 2, Table A.1 line 70).
    Iop2,
}

impl InteroperabilityPoint {
    /// The numeric interoperability-point value (`0`, `1`, or `2`), the value
    /// `lcr_max_interop` carries (AV2 § 6.8.2, mirror lines 1661-1662) and the value
    /// used in diagnostic messages.
    #[must_use]
    pub(crate) fn value(self) -> u8 {
        match self {
            Self::Iop0 => 0,
            Self::Iop1 => 1,
            Self::Iop2 => 2,
        }
    }
}

/// Returns the interoperability point a non-Configurable profile signals
/// (Annex A.2 Table A.1, mirror lines 64-81: profile 0 -> IOP0; 1, 3, 4 -> IOP1;
/// 2 -> IOP2), or `None` for a reserved (`5..=30`) or the Configurable (`31`)
/// profile, whose interoperability point is not directly determined by the profile id
/// (the Configurable profile's IOP is conveyed through the Annex A.3 multi-sequence
/// configuration / aggregate signaling, mirror lines 95-101, not the profile id).
#[must_use]
pub(crate) fn interoperability_point(profile_idc: u8) -> Option<InteroperabilityPoint> {
    match profile_idc {
        0 => Some(InteroperabilityPoint::Iop0),
        1 | 3 | 4 => Some(InteroperabilityPoint::Iop1),
        2 => Some(InteroperabilityPoint::Iop2),
        _ => None,
    }
}

/// Returns `true` when `seq_profile_idc` / `multistream_profile_idc` is permitted under
/// the multi-sequence configuration `lcr_config_idc` (Annex A.3 Table A.6 column
/// "seq_profile_idc", mirror lines 242-254):
///
/// - `C_Main_420_10` (`lcr_config_idc == 0`): profiles `0..=2`, `31` (line 247);
/// - `C_Main_422_10` (`lcr_config_idc == 1`): profiles `0..=3`, `31` (line 249);
/// - `C_Main_444_10` (`lcr_config_idc == 2`): profiles `0..=2`, `4`, `31` (line 252).
///
/// Any other `config_idc` (`3..=63` "Reserved", Table A.5 line 238) has no defined
/// value space, so the caller must not consult this helper for a reserved configuration;
/// it returns `false` for every profile there (no consistency claim can be made).
#[must_use]
pub(crate) fn config_idc_allows_profile(config_idc: u8, profile_idc: u8) -> bool {
    match config_idc {
        0 => matches!(profile_idc, 0..=2 | CONFIGURABLE_PROFILE_IDC),
        1 => matches!(profile_idc, 0..=3 | CONFIGURABLE_PROFILE_IDC),
        2 => matches!(profile_idc, 0..=2 | 4 | CONFIGURABLE_PROFILE_IDC),
        _ => false,
    }
}

/// Returns `true` when `config_idc` names a defined Annex A.3 multi-sequence
/// configuration (`0`, `1`, or `2`; `3..=63` are "Reserved", Table A.5 line 238). The
/// § 6.8.2 aggregate-consistency check only runs the [`config_idc_allows_profile`]
/// comparison for a defined configuration; a reserved value is owned by the §6.8.4
/// Annex-A range residual, not this agreement check.
#[must_use]
pub(crate) fn is_defined_config_idc(config_idc: u8) -> bool {
    matches!(config_idc, 0..=2)
}

/// Returns `true` when `level_idx` is a reserved level index (Annex A.4 Table A.7,
/// mirror line 321: "22-30 Reserved").
#[must_use]
pub(crate) fn is_reserved_level(level_idx: u8) -> bool {
    (FIRST_RESERVED_LEVEL_IDX..=LAST_RESERVED_LEVEL_IDX).contains(&level_idx)
}

/// Returns `true` when `interop` names a defined Annex A.3 interoperability point.
///
/// Table A.3 (mirror lines 125-138) defines interoperability points `0`, `1`, `2`, and
/// `15` ("15 (max)"); values `3..=14` are "Reserved" (line 136). `lcr_max_interop` is a
/// 4-bit field, so its whole `0..=15` value space is covered by this table. Used by the
/// § 6.8.4 value-space check; the `15` "max" point is *not* a per-profile interoperability
/// point ([`interoperability_point`] only yields `0`/`1`/`2`), so it is enumerated here.
#[must_use]
pub(crate) fn is_defined_max_interop(interop: u8) -> bool {
    matches!(interop, 0..=2 | 15)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_limits_match_mirror_spot_values() {
        let level_2_0 = level_limits(0).unwrap();
        assert_eq!(level_2_0.max_pic_size, 147_456);
        assert_eq!(level_2_0.max_width, 640);
        assert_eq!(level_2_0.max_tiles, 8);
        assert_eq!(level_2_0.max_tile_cols, 4);

        let level_5_3 = level_limits(9).unwrap();
        assert_eq!(level_5_3.max_pic_size, 8_912_896);
        assert_eq!(level_5_3.max_width, 4975);
        assert_eq!(level_5_3.max_tiles, 64);
        assert_eq!(level_5_3.max_tile_cols, 8);

        let level_8_3 = level_limits(21).unwrap();
        assert_eq!(level_8_3.max_pic_size, 530_841_600);
        assert_eq!(level_8_3.max_width, 38400);
        assert_eq!(level_8_3.max_tiles, 512);
        assert_eq!(level_8_3.max_tile_cols, 64);

        let level_4_0 = level_limits(4).unwrap();
        assert_eq!(level_4_0.max_pic_size, 2_359_296);
        assert_eq!(level_4_0.max_width, 2560);
        assert_eq!(level_4_0.max_tiles, 32);
        assert_eq!(level_4_0.max_tile_cols, 8);

        let level_6_0 = level_limits(10).unwrap();
        assert_eq!(level_6_0.max_pic_size, 35_651_584);
        assert_eq!(level_6_0.max_width, 9951);
        assert_eq!(level_6_0.max_tiles, 128);
        assert_eq!(level_6_0.max_tile_cols, 16);

        let level_7_0 = level_limits(14).unwrap();
        assert_eq!(level_7_0.max_pic_size, 142_606_336);
        assert_eq!(level_7_0.max_width, 19902);
        assert_eq!(level_7_0.max_tiles, 256);
        assert_eq!(level_7_0.max_tile_cols, 32);
    }

    #[test]
    fn reserved_and_max_parameters_levels_have_no_limits() {
        for idx in FIRST_RESERVED_LEVEL_IDX..=LAST_RESERVED_LEVEL_IDX {
            assert_eq!(
                level_limits(idx),
                None,
                "reserved level {idx} must map to None"
            );
        }
        assert_eq!(
            level_limits(LAST_RESERVED_LEVEL_IDX + 1),
            None,
            "maximum-parameters level must map to None"
        );
        assert_eq!(level_limits(u8::MAX), None);
    }

    #[test]
    fn reserved_level_range_is_22_through_30() {
        assert!(!is_reserved_level(21));
        assert!(is_reserved_level(22));
        assert!(is_reserved_level(30));
        assert!(!is_reserved_level(31));
    }

    #[test]
    fn defined_max_interop_values_match_table_a3() {
        assert!(is_defined_max_interop(0));
        assert!(is_defined_max_interop(1));
        assert!(is_defined_max_interop(2));
        assert!(!is_defined_max_interop(3));
        assert!(!is_defined_max_interop(14));
        assert!(is_defined_max_interop(15));
    }

    #[test]
    fn interoperability_points_match_table_a1() {
        assert_eq!(interoperability_point(0), Some(InteroperabilityPoint::Iop0));
        assert_eq!(interoperability_point(1), Some(InteroperabilityPoint::Iop1));
        assert_eq!(interoperability_point(2), Some(InteroperabilityPoint::Iop2));
        assert_eq!(interoperability_point(3), Some(InteroperabilityPoint::Iop1));
        assert_eq!(interoperability_point(4), Some(InteroperabilityPoint::Iop1));
        assert_eq!(interoperability_point(5), None);
        assert_eq!(interoperability_point(CONFIGURABLE_PROFILE_IDC), None);
        assert_eq!(InteroperabilityPoint::Iop0.value(), 0);
        assert_eq!(InteroperabilityPoint::Iop1.value(), 1);
        assert_eq!(InteroperabilityPoint::Iop2.value(), 2);
    }

    #[test]
    fn config_idc_profile_sets_match_table_a6() {
        for p in [0u8, 1, 2, CONFIGURABLE_PROFILE_IDC] {
            assert!(
                config_idc_allows_profile(0, p),
                "config 0 allows profile {p}"
            );
        }
        for p in [3u8, 4, 5, 30] {
            assert!(
                !config_idc_allows_profile(0, p),
                "config 0 disallows profile {p}"
            );
        }
        for p in [0u8, 1, 2, 3, CONFIGURABLE_PROFILE_IDC] {
            assert!(
                config_idc_allows_profile(1, p),
                "config 1 allows profile {p}"
            );
        }
        for p in [4u8, 5] {
            assert!(
                !config_idc_allows_profile(1, p),
                "config 1 disallows profile {p}"
            );
        }
        for p in [0u8, 1, 2, 4, CONFIGURABLE_PROFILE_IDC] {
            assert!(
                config_idc_allows_profile(2, p),
                "config 2 allows profile {p}"
            );
        }
        for p in [3u8, 5] {
            assert!(
                !config_idc_allows_profile(2, p),
                "config 2 disallows profile {p}"
            );
        }
        assert!(!config_idc_allows_profile(3, 0));
        assert!(!config_idc_allows_profile(63, CONFIGURABLE_PROFILE_IDC));
        assert!(is_defined_config_idc(0));
        assert!(is_defined_config_idc(2));
        assert!(!is_defined_config_idc(3));
        assert!(!is_defined_config_idc(63));
    }
}
