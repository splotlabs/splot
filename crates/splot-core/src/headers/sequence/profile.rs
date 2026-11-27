// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 `seq_profile_idc` / `multistream_profile_idc` profile value model
//! (AV2 v1.0.0 § 5.4.1 / § 5.6, Annex A.2 Table A.1).

use super::ChromaFormatIdc;

/// The AV2 profile a `seq_profile_idc` (AV2 v1.0.0 § 5.4.1) or `multistream_profile_idc`
/// (§ 5.6) value names. Both 5-bit fields share this value space, defined by Annex A.2
/// Table A.1 (docs/spec/av2/1.0.0/annex-a-profiles-levels-and-tiers.md, mirror lines 59-90).
/// Feature: `AV2-A-PROFILES`.
///
/// The raw 5-bit value returned by [`Self::get`] is the canonical identity: equality,
/// ordering, hashing, and the [`Self::is_reserved`] / [`Self::is_configurable`] classifiers
/// are all defined in terms of it, not the enum variant. [`Self::from_bits`] (the only
/// constructor the parsers use) masks its argument to 5 bits and places `5..=30` in
/// `Reserved`, so every value the codebase produces is canonical (`Reserved` holds only
/// `5..=30`). Defining the traits on `get()` means even a hand-constructed `Reserved(31)`
/// behaves as the value 31 (== `Configurable`) rather than corrupting ordering or
/// classification — the public `Reserved(u8)` payload cannot break the value-space invariant.
/// Reserved values are parseable but conform to no profile of this version (the validator
/// flags them via `annex-a/profile-reserved`).
#[derive(Debug, Clone, Copy)]
pub enum ProfileIdc {
    /// `Main_420_10_IP0` (`seq_profile_idc == 0`, Table A.1 line 64).
    Main420Ip0,
    /// `Main_420_10_IP1` (`seq_profile_idc == 1`, Table A.1 line 67).
    Main420Ip1,
    /// `Main_420_10_IP2` (`seq_profile_idc == 2`, Table A.1 line 70).
    Main420Ip2,
    /// `Main_422_10_IP1` (`seq_profile_idc == 3`, Table A.1 line 73).
    Main422Ip1,
    /// `Main_444_10_IP1` (`seq_profile_idc == 4`, Table A.1 line 81).
    Main444Ip1,
    /// Reserved (`seq_profile_idc` in `5..=30`, Table A.1 line 85); conforms to no profile.
    /// `from_bits` only ever places `5..=30` here.
    Reserved(u8),
    /// `Configurable` (`seq_profile_idc == 31`, Table A.1 line 87): constraints come from
    /// `chroma_format_idc` / `bit_depth_idc` / `SeqMaxMlayerCnt` and the multi-sequence
    /// configuration, not the profile id.
    Configurable,
}

impl ProfileIdc {
    /// Creates a profile id from the 5-bit `seq_profile_idc` / `multistream_profile_idc`
    /// field. The argument is masked to 5 bits, so every input maps to a canonical value
    /// (reserved values `5..=30` keep their raw value in `Reserved`).
    #[must_use]
    pub const fn from_bits(value: u8) -> Self {
        match value & 0x1F {
            0 => Self::Main420Ip0,
            1 => Self::Main420Ip1,
            2 => Self::Main420Ip2,
            3 => Self::Main422Ip1,
            4 => Self::Main444Ip1,
            31 => Self::Configurable,
            other => Self::Reserved(other),
        }
    }

    /// Returns the raw `seq_profile_idc` value (the canonical identity).
    #[must_use]
    pub const fn get(self) -> u8 {
        match self {
            Self::Main420Ip0 => 0,
            Self::Main420Ip1 => 1,
            Self::Main420Ip2 => 2,
            Self::Main422Ip1 => 3,
            Self::Main444Ip1 => 4,
            Self::Reserved(value) => value,
            Self::Configurable => 31,
        }
    }

    /// Returns `true` for a reserved profile (`seq_profile_idc` in `5..=30`, Table A.1
    /// line 85), which conforms to no profile of this version. Defined on the raw value so a
    /// hand-constructed `Reserved(31)` is correctly not reserved (it is the value 31).
    #[must_use]
    pub const fn is_reserved(self) -> bool {
        matches!(self.get(), 5..=30)
    }

    /// Returns `true` for the Configurable profile (`seq_profile_idc == 31`, Table A.1
    /// line 87). Defined on the raw value.
    #[must_use]
    pub const fn is_configurable(self) -> bool {
        self.get() == 31
    }

    /// Returns `true` when `chroma` is in this profile's allowed `chroma_format_idc` set
    /// (Table A.1, mirror lines 61-90): 400 and 420 for profiles 0-2, plus 422 for
    /// profile 3, plus 444 for profile 4, and every format for Configurable. A reserved
    /// profile has no allowed set and returns `false`.
    #[must_use]
    pub const fn allows_chroma(self, chroma: ChromaFormatIdc) -> bool {
        match self.get() {
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
            31 => true,
            _ => false,
        }
    }

    /// Returns `BitrateProfileFactor` (Annex A.2 Table A.2) in thousandths:
    /// 1000 for profiles 0-2, 1667 for 3 and 2500 for 4. The Configurable
    /// and reserved profiles define none.
    #[must_use]
    pub const fn bitrate_factor_milli(self) -> Option<u32> {
        match self.get() {
            0..=2 => Some(1000),
            3 => Some(1667),
            4 => Some(2500),
            _ => None,
        }
    }
}

impl PartialEq for ProfileIdc {
    fn eq(&self, other: &Self) -> bool {
        self.get() == other.get()
    }
}

impl Eq for ProfileIdc {}

impl PartialOrd for ProfileIdc {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ProfileIdc {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.get().cmp(&other.get())
    }
}

impl core::hash::Hash for ProfileIdc {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.get().hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_CHROMA: [ChromaFormatIdc; 4] = [
        ChromaFormatIdc::Monochrome,
        ChromaFormatIdc::Yuv420,
        ChromaFormatIdc::Yuv422,
        ChromaFormatIdc::Yuv444,
    ];

    #[test]
    fn bitrate_factors_match_table_a2() {
        let factor = |value| ProfileIdc::from_bits(value).bitrate_factor_milli();
        assert_eq!(
            [0, 1, 2, 3, 4].map(factor),
            [1000, 1000, 1000, 1667, 2500].map(Some)
        );
        assert_eq!([5, 30, 31].map(factor), [None; 3]);
    }

    #[test]
    fn allowed_chroma_sets_match_table_a1() {
        for value in 0..=31 {
            let profile = ProfileIdc::from_bits(value);
            let allowed: Vec<ChromaFormatIdc> = ALL_CHROMA
                .into_iter()
                .filter(|&chroma| profile.allows_chroma(chroma))
                .collect();
            let expected: &[ChromaFormatIdc] = match value {
                0..=2 => &ALL_CHROMA[..2],
                3 => &ALL_CHROMA[..3],
                4 => &[
                    ChromaFormatIdc::Monochrome,
                    ChromaFormatIdc::Yuv420,
                    ChromaFormatIdc::Yuv444,
                ],
                31 => &ALL_CHROMA,
                _ => &[],
            };
            assert_eq!(allowed, expected, "seq_profile_idc {value}");
        }
    }
}
