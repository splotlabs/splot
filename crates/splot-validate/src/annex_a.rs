// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 Annex A profile/level/tier static-constraint tables and helpers
//! (AV2 v1.0.0 Annex A,
//! `docs/spec/av2/1.0.0/annex-a-profiles-levels-and-tiers.md`).
//!
//! This module holds the table data transcribed **verbatim** from the committed
//! spec mirror — the AV2 profile definitions (Annex A.2 Table A.1) and the static
//! level limits (Annex A.4 Tables A.7/A.8/A.9) — plus pure helper functions over
//! them. Every value cell carries a mirror line citation; the rate columns
//! (`MaxDisplayRate`/`MaxDecodeRate`/`MaxHeaderRate`/`MainMbps`/`HighMbps`/`MainCR`/
//! `HighCR`) are deliberately **not** transcribed: they belong to the Annex E
//! decoder-model change (Feature ID `AV2-E-DECODER-MODEL`).
//!
//! The check wiring that consumes these tables lives in
//! [`crate::context`]; the rule semantics and the Table A.4 interoperability-point
//! presence requirements are documented there. Feature IDs: `AV2-A-PROFILES`,
//! `AV2-A-LEVELS-TIERS`.

use splot_core::headers::sequence::ChromaFormatIdc;

/// First reserved `seq_profile_idc` value (Annex A.2 Table A.1, mirror line 85:
/// "Reserved 5-30"). Values `5..=30` do not conform to any profile of this version.
pub(crate) const FIRST_RESERVED_PROFILE_IDC: u8 = 5;
/// Last reserved `seq_profile_idc` value (Annex A.2 Table A.1, mirror line 85:
/// "Reserved 5-30").
pub(crate) const LAST_RESERVED_PROFILE_IDC: u8 = 30;
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

/// The static level limits for one `LevelIdx` (Annex A.4 Tables A.8/A.9).
///
/// `max_h_size` and `max_v_size` share a single column in Table A.8 — its header is
/// "MaxHSize/MaxVSize" (mirror lines 330-331) — so one transcribed value bounds both
/// `FrameWidth <= MaxHSize` and `FrameHeight <= MaxVSize` (Annex A.4, mirror lines
/// 619-620). It is modeled as one field, [`LevelLimits::max_h_v_size`], to make the
/// shared column explicit and rule out a transposition.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LevelLimits {
    /// `MaxPicSize` in luma samples (Annex A.4 Table A.8, mirror lines 330-379).
    pub(crate) max_pic_size: u64,
    /// The shared `MaxHSize`/`MaxVSize` value in luma samples (Annex A.4 Table A.8
    /// column "MaxHSize/MaxVSize", mirror lines 330-379).
    pub(crate) max_h_v_size: u32,
    /// `MaxTiles` (Annex A.4 Table A.9, mirror lines 385-432).
    pub(crate) max_tiles: u32,
    /// `MaxTileCols` (Annex A.4 Table A.9, mirror lines 385-432).
    pub(crate) max_tile_cols: u32,
}

/// `LevelLimits` indexed by `seq_level_idx` for the defined levels `0..=21`
/// (Annex A.4 Table A.7 maps these to levels 2.0..=8.3, mirror lines 269-319).
///
/// Each row's `max_pic_size` / `max_h_v_size` come from Table A.8 (mirror lines
/// 330-379, columns "MaxPicSize" and "MaxHSize/MaxVSize") and its `max_tiles` /
/// `max_tile_cols` from Table A.9 (mirror lines 385-432, columns "MaxTiles" and
/// "MaxTileCols"). Reserved indices `22..=30` (Table A.7, mirror line 321) and the
/// Maximum-parameters index `31` (mirror line 323) are not in this table; the lookup
/// returns `None` for any index outside `0..=21`. The rate columns are intentionally
/// omitted (owned by `AV2-E-DECODER-MODEL`).
const LEVEL_LIMITS: [LevelLimits; 22] = [
    LevelLimits {
        max_pic_size: 147_456,
        max_h_v_size: 640,
        max_tiles: 8,
        max_tile_cols: 4,
    },
    LevelLimits {
        max_pic_size: 278_784,
        max_h_v_size: 880,
        max_tiles: 8,
        max_tile_cols: 4,
    },
    LevelLimits {
        max_pic_size: 665_856,
        max_h_v_size: 1360,
        max_tiles: 16,
        max_tile_cols: 6,
    },
    LevelLimits {
        max_pic_size: 1_065_024,
        max_h_v_size: 1720,
        max_tiles: 16,
        max_tile_cols: 6,
    },
    LevelLimits {
        max_pic_size: 2_359_296,
        max_h_v_size: 2560,
        max_tiles: 32,
        max_tile_cols: 8,
    },
    LevelLimits {
        max_pic_size: 2_359_296,
        max_h_v_size: 2560,
        max_tiles: 32,
        max_tile_cols: 8,
    },
    LevelLimits {
        max_pic_size: 8_912_896,
        max_h_v_size: 4975,
        max_tiles: 64,
        max_tile_cols: 8,
    },
    LevelLimits {
        max_pic_size: 8_912_896,
        max_h_v_size: 4975,
        max_tiles: 64,
        max_tile_cols: 8,
    },
    LevelLimits {
        max_pic_size: 8_912_896,
        max_h_v_size: 4975,
        max_tiles: 64,
        max_tile_cols: 8,
    },
    LevelLimits {
        max_pic_size: 8_912_896,
        max_h_v_size: 4975,
        max_tiles: 64,
        max_tile_cols: 8,
    },
    LevelLimits {
        max_pic_size: 35_651_584,
        max_h_v_size: 9951,
        max_tiles: 128,
        max_tile_cols: 16,
    },
    LevelLimits {
        max_pic_size: 35_651_584,
        max_h_v_size: 9951,
        max_tiles: 128,
        max_tile_cols: 16,
    },
    LevelLimits {
        max_pic_size: 35_651_584,
        max_h_v_size: 9951,
        max_tiles: 128,
        max_tile_cols: 16,
    },
    LevelLimits {
        max_pic_size: 35_651_584,
        max_h_v_size: 9951,
        max_tiles: 128,
        max_tile_cols: 16,
    },
    LevelLimits {
        max_pic_size: 142_606_336,
        max_h_v_size: 19902,
        max_tiles: 256,
        max_tile_cols: 32,
    },
    LevelLimits {
        max_pic_size: 142_606_336,
        max_h_v_size: 19902,
        max_tiles: 256,
        max_tile_cols: 32,
    },
    LevelLimits {
        max_pic_size: 142_606_336,
        max_h_v_size: 19902,
        max_tiles: 256,
        max_tile_cols: 32,
    },
    LevelLimits {
        max_pic_size: 142_606_336,
        max_h_v_size: 19902,
        max_tiles: 256,
        max_tile_cols: 32,
    },
    LevelLimits {
        max_pic_size: 530_841_600,
        max_h_v_size: 38400,
        max_tiles: 512,
        max_tile_cols: 64,
    },
    LevelLimits {
        max_pic_size: 530_841_600,
        max_h_v_size: 38400,
        max_tiles: 512,
        max_tile_cols: 64,
    },
    LevelLimits {
        max_pic_size: 530_841_600,
        max_h_v_size: 38400,
        max_tiles: 512,
        max_tile_cols: 64,
    },
    LevelLimits {
        max_pic_size: 530_841_600,
        max_h_v_size: 38400,
        max_tiles: 512,
        max_tile_cols: 64,
    },
];

/// Returns the [`LevelLimits`] for `level_idx`, or `None` when `level_idx` is not a
/// table-mapped level (a reserved `22..=30`, the Maximum-parameters `31`, or any
/// `5`-bit value above `31`). Bounds-checked: no indexing panic for any `u8` input
/// (Annex A.4 Tables A.7/A.8/A.9).
#[must_use]
pub(crate) fn level_limits(level_idx: u8) -> Option<LevelLimits> {
    LEVEL_LIMITS.get(level_idx as usize).copied()
}

/// Returns `true` when `profile_idc` is a reserved profile (Annex A.2 Table A.1,
/// mirror line 85: "Reserved 5-30").
#[must_use]
pub(crate) fn is_reserved_profile(profile_idc: u8) -> bool {
    (FIRST_RESERVED_PROFILE_IDC..=LAST_RESERVED_PROFILE_IDC).contains(&profile_idc)
}

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

/// Returns `true` when `chroma_format_idc` is permitted under `profile_idc`
/// (Annex A.2 Table A.1 column "chroma_format_idc", mirror lines 61-90):
///
/// - profiles 0, 1, 2: `CHROMA_FORMAT_400`, `CHROMA_FORMAT_420` (lines 64-71);
/// - profile 3: adds `CHROMA_FORMAT_422` (lines 73-75);
/// - profile 4: adds `CHROMA_FORMAT_444` (lines 81-83);
/// - profile 31 (Configurable): all four formats are listed (lines 87-90).
///
/// Reserved profiles (`5..=30`, line 85, no chroma column) and any value above 31
/// return `false`: there is no defined allowed set, so any format mismatches. The
/// caller skips this check for reserved and Configurable profiles (the
/// reserved-profile error and the Table A.1 dashes for profile 31 cover those).
#[must_use]
pub(crate) fn profile_allows_chroma(profile_idc: u8, chroma: ChromaFormatIdc) -> bool {
    match profile_idc {
        0..=2 => matches!(
            chroma,
            ChromaFormatIdc::Monochrome | ChromaFormatIdc::Yuv420
        ),
        3 => matches!(
            chroma,
            ChromaFormatIdc::Monochrome | ChromaFormatIdc::Yuv420 | ChromaFormatIdc::Yuv422
        ),
        4 => matches!(
            chroma,
            ChromaFormatIdc::Monochrome | ChromaFormatIdc::Yuv420 | ChromaFormatIdc::Yuv444
        ),
        CONFIGURABLE_PROFILE_IDC => true,
        _ => false,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn level_limits_match_mirror_spot_values() {
        let level_2_0 = level_limits(0).unwrap();
        assert_eq!(level_2_0.max_pic_size, 147_456);
        assert_eq!(level_2_0.max_h_v_size, 640);
        assert_eq!(level_2_0.max_tiles, 8);
        assert_eq!(level_2_0.max_tile_cols, 4);

        let level_5_3 = level_limits(9).unwrap();
        assert_eq!(level_5_3.max_pic_size, 8_912_896);
        assert_eq!(level_5_3.max_h_v_size, 4975);
        assert_eq!(level_5_3.max_tiles, 64);
        assert_eq!(level_5_3.max_tile_cols, 8);

        let level_8_3 = level_limits(21).unwrap();
        assert_eq!(level_8_3.max_pic_size, 530_841_600);
        assert_eq!(level_8_3.max_h_v_size, 38400);
        assert_eq!(level_8_3.max_tiles, 512);
        assert_eq!(level_8_3.max_tile_cols, 64);

        let level_4_0 = level_limits(4).unwrap();
        assert_eq!(level_4_0.max_pic_size, 2_359_296);
        assert_eq!(level_4_0.max_h_v_size, 2560);
        assert_eq!(level_4_0.max_tiles, 32);
        assert_eq!(level_4_0.max_tile_cols, 8);

        let level_6_0 = level_limits(10).unwrap();
        assert_eq!(level_6_0.max_pic_size, 35_651_584);
        assert_eq!(level_6_0.max_h_v_size, 9951);
        assert_eq!(level_6_0.max_tiles, 128);
        assert_eq!(level_6_0.max_tile_cols, 16);

        let level_7_0 = level_limits(14).unwrap();
        assert_eq!(level_7_0.max_pic_size, 142_606_336);
        assert_eq!(level_7_0.max_h_v_size, 19902);
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
    fn reserved_profile_range_is_5_through_30() {
        assert!(!is_reserved_profile(4));
        assert!(is_reserved_profile(5));
        assert!(is_reserved_profile(30));
        assert!(!is_reserved_profile(CONFIGURABLE_PROFILE_IDC));
    }

    #[test]
    fn profile_chroma_allowed_sets_match_table_a1() {
        for profile in [0u8, 1, 2] {
            assert!(profile_allows_chroma(profile, ChromaFormatIdc::Monochrome));
            assert!(profile_allows_chroma(profile, ChromaFormatIdc::Yuv420));
            assert!(!profile_allows_chroma(profile, ChromaFormatIdc::Yuv422));
            assert!(!profile_allows_chroma(profile, ChromaFormatIdc::Yuv444));
        }
        assert!(profile_allows_chroma(3, ChromaFormatIdc::Yuv422));
        assert!(!profile_allows_chroma(3, ChromaFormatIdc::Yuv444));
        assert!(profile_allows_chroma(4, ChromaFormatIdc::Yuv444));
        assert!(!profile_allows_chroma(4, ChromaFormatIdc::Yuv422));
        for chroma in [
            ChromaFormatIdc::Monochrome,
            ChromaFormatIdc::Yuv420,
            ChromaFormatIdc::Yuv422,
            ChromaFormatIdc::Yuv444,
        ] {
            assert!(profile_allows_chroma(CONFIGURABLE_PROFILE_IDC, chroma));
        }
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
