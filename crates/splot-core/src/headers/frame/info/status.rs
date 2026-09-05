// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Parse-result vocabulary for the AV2 § 5.18.2 frame-header core parser.

/// Which parser path a caller selects for a frame header (AV2 v1.0.0 § 5.18).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrameHeaderParseMode {
    /// Read only the activation/reference prefix of `frame_header_info()` — exactly
    /// the fields [`super::parse_frame_header_prefix`] consumes.
    ActivationPrefix,
    /// Read the frame-header core control region for state-supported paths, stopping
    /// with an explicit status before unmodeled syntax.
    Core,
}

/// How much of `frame_header_info()` was consumed (AV2 § 5.18.2).
///
/// The four `StoppedInside*` variants record EOF in modeled syntax; earlier parsed
/// facts remain available. Coverage stops indicate missing state or unsupported syntax,
/// not a truncated payload. Header completion does not validate subsequent tile data
/// or its trailing bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrameHeaderParseStatus {
    /// Only activation fields were read: prefix mode was selected or sequence state was unavailable.
    ActivationFieldsOnly,
    /// The show-existing header, including its film-grain fields, is complete.
    ShowExistingFrameComplete,
    /// The intra header is complete through `film_grain_config()` (§ 5.18.10.1).
    IntraHeaderComplete,
    /// The ordinary inter, TIP-output, or bridge header is complete.
    InterHeaderComplete,
    /// EOF inside the intra deblocking/GDF/CDEF/LR/CCSO cluster. Completed structures are retained.
    StoppedInsideFilterParams,
    /// EOF after CCSO inside the intra coding-mode or film-grain tail; `intra_tail` is absent.
    StoppedInsideIntraTail,
    /// EOF inside the show-existing film-grain fields; earlier SEF facts are retained.
    StoppedInsideShowExistingFrame,
    /// EOF inside inter/bridge control or its modeled tail; earlier core and inter facts are retained.
    StoppedInsideInterControl,
    /// Required decoder/reference state or syntax coverage is unavailable.
    UnsupportedUntilFeature {
        /// Implementation-matrix Feature ID for the unmodeled coverage.
        feature_id: &'static str,
    },
}

impl FrameHeaderParseStatus {
    /// Returns a stable snake-case label for tools and JSON output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ActivationFieldsOnly => "activation_fields_only",
            Self::ShowExistingFrameComplete => "show_existing_frame_complete",
            Self::IntraHeaderComplete => "intra_header_complete",
            Self::InterHeaderComplete => "inter_header_complete",
            Self::StoppedInsideFilterParams => "stopped_inside_filter_params",
            Self::StoppedInsideIntraTail => "stopped_inside_intra_tail",
            Self::StoppedInsideShowExistingFrame => "stopped_inside_show_existing_frame",
            Self::StoppedInsideInterControl => "stopped_inside_inter_control",
            Self::UnsupportedUntilFeature { .. } => "unsupported_until_feature",
        }
    }

    /// Whether the status records EOF in modeled syntax rather than a coverage stop.
    /// The validator uses this distinction for its truncated-frame-header diagnostic.
    #[must_use]
    pub const fn is_truncated_in_modeled_region(self) -> bool {
        matches!(
            self,
            Self::StoppedInsideFilterParams
                | Self::StoppedInsideIntraTail
                | Self::StoppedInsideShowExistingFrame
                | Self::StoppedInsideInterControl
        )
    }
}

/// `FrameType` for the paths the core parser derives (AV2 v1.0.0 § 5.18.2).
///
/// A bridge frame's `INTER_FRAME` and a switch/RAS frame's `SWITCH_FRAME` are derived
/// before the parser stops; show-existing-frame leaves `FrameType` unknown because it
/// comes from reference-frame state this phase does not model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrameType {
    /// `KEY_FRAME`.
    Key,
    /// `INTER_FRAME`.
    Inter,
    /// `INTRA_ONLY_FRAME`.
    IntraOnly,
    /// `SWITCH_FRAME`.
    Switch,
}

impl FrameType {
    /// Returns a stable snake-case label for tools and JSON output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Inter => "inter",
            Self::IntraOnly => "intra_only",
            Self::Switch => "switch",
        }
    }
}

/// How a show-existing-frame OBU's `trailing_bits()` boundary resolved (AV2 v1.0.0
/// § 5.2.1 / § 5.2.3).
///
/// A show-existing-frame OBU's payload is **exactly** the SEF `frame_header()` followed
/// by `trailing_bits( remainingPayloadBits )`: the SEF arm of § 5.18.2 (mirror :4145)
/// `return`s immediately after `film_grain_config()` (mirror :4186), and a SEF OBU
/// (`OBU_LEADING_SEF` / `OBU_REGULAR_SEF`) is not an `is_tile_group()` type, so
/// `usedArith == 0` and § 5.2.1 (:132-152) reads `trailing_bits( remainingPayloadBits )`
/// over the rest of the payload (the type is not extensible, so the `else` arm applies).
/// There is no tile data after a SEF frame header, so the boundary is decidable from the
/// payload alone. Recorded only on the [`FrameHeaderParseStatus::ShowExistingFrameComplete`]
/// path; the validator surfaces a non-`Valid` outcome as a § 6.2.1 / § 5.2.3 diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SefTrailingBits {
    /// `trailing_bits( remainingPayloadBits )` was exactly one `trailing_one_bit == 1`
    /// followed by zero bits to the OBU boundary (AV2 § 5.2.3 / § 6.2.3).
    Valid,
    /// The payload ended with no bits left for `trailing_bits()` — there was no
    /// `trailing_one_bit`. This catches the `grain_seed`-eats-the-marker case: a
    /// `grain_seed` short by its final bit consumes what should have been the
    /// `trailing_one_bit`, leaving nothing for the trailing-bits boundary (AV2 § 6.2.1).
    Empty,
    /// The first remaining bit was not the required `trailing_one_bit == 1`
    /// (AV2 § 6.2.3).
    MissingOneBit,
    /// A bit after the `trailing_one_bit` was not `0` (AV2 § 6.2.3).
    ZeroBitNotZero,
}

impl SefTrailingBits {
    /// A stable snake-case label for tools and diagnostics.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Empty => "empty",
            Self::MissingOneBit => "missing_one_bit",
            Self::ZeroBitNotZero => "zero_bit_not_zero",
        }
    }

    /// A human-readable description of the specific § 5.2.3 / § 6.2.3 violation, or
    /// `None` for [`Self::Valid`].
    #[must_use]
    pub const fn violation_message(self) -> Option<&'static str> {
        match self {
            Self::Valid => None,
            Self::Empty => Some(
                "the OBU payload ended with no trailing_bits() — there was no trailing_one_bit \
                 after the show-existing-frame film_grain_config() (a grain_seed short by one bit \
                 consumes the marker)",
            ),
            Self::MissingOneBit => Some(
                "the first bit after the show-existing-frame frame header was not the \
                 required trailing_one_bit == 1",
            ),
            Self::ZeroBitNotZero => {
                Some("a trailing_zero_bit after the show-existing-frame trailing_one_bit was not 0")
            }
        }
    }
}
