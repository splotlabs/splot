// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Parse-result vocabulary for the AV2 § 5.18.2 frame-header core parser.
//!
//! The enums here are pure classification types shared between
//! [`parse_frame_header_core`](super::parse_frame_header_core) and its callers (the
//! validator, the inspector, the [`crate::write`] frame-header writer): the requested
//! [`FrameHeaderParseMode`], the stop-point [`FrameHeaderParseStatus`] (with its
//! truncation partition), the derived [`FrameType`], and the show-existing-frame
//! [`SefTrailingBits`] boundary classification. They carry no parser state and depend
//! on no other frame-header type.

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

/// How much of `frame_header_info()` a core parse consumed (AV2 v1.0.0 § 5.18.2).
///
/// A partial status means the parser intentionally stopped; callers must not infer
/// that the full payload or its trailing bits were validated.
///
/// # Truncation partition
///
/// The variants split cleanly into two disjoint classes that callers (the validator's
/// `frame_header_core_checks`) MUST keep separate:
///
/// - **EOF-in-a-fully-modeled region** — the OBU payload ended where the spec mandates
///   more syntax in a region this parser fully models, so the truncation is a decidable
///   bitstream defect that must surface as a validation error
///   ([`Self::is_truncated_in_modeled_region`] returns `true`):
///   [`Self::StoppedInsideFilterParams`], [`Self::StoppedInsideIntraTail`],
///   [`Self::StoppedInsideShowExistingFrame`], and [`Self::StoppedInsideInterControl`].
/// - **Bounded coverage stop / complete parse** — the parser stopped at a point whose
///   following syntax it does NOT fully model (unsupported coverage), or it completed,
///   so an early stop is *not* evidence of a truncated payload and must stay silent:
///   [`Self::ActivationFieldsOnly`], [`Self::CoreFieldsOnly`],
///   [`Self::ShowExistingFrameComplete`], [`Self::IntraHeaderComplete`],
///   [`Self::StoppedBeforeLoopRestorationParams`], [`Self::StoppedBeforeReadTxMode`],
///   [`Self::StoppedBeforeWienerNsFilter`], and [`Self::UnsupportedUntilFeature`].
///
/// The partition is exact: every status producer in this module either reaches a
/// modeled-region EOF (the four `StoppedInside*` statuses) or sets a coverage/complete
/// status. `CoreFieldsOnly` is deliberately on the silent side — it is reserved for an
/// ordinary bounded stop, never a truncation (the SEF film-grain EOF, which previously
/// reused it, now reports [`Self::StoppedInsideShowExistingFrame`]). The honest distinction
/// for the inter / bridge control region (§ 5.18.2): the region IS fully modeled up to its
/// coverage stops ([`InterStop`](crate::headers::frame::inter::InterStop)), so the parser
/// can only return `Ok` at one of those stops or `Err(UnexpectedEof)` while reading a
/// modeled field — the EOF case is the only truncation, recorded as
/// [`Self::StoppedInsideInterControl`], while a clean coverage stop stays on the silent side
/// (`UnsupportedUntilFeature`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrameHeaderParseStatus {
    /// Only the activation/reference fields were read — either the caller asked for
    /// [`FrameHeaderParseMode::ActivationPrefix`], or core mode lacked the sequence
    /// state (a fully parsed active sequence header) needed to continue.
    ActivationFieldsOnly,
    /// Core control fields were read and the parser stopped at a bounded point that is
    /// not the filtering/quantization/segmentation cluster.
    ///
    /// Reserved: superseded by [`Self::StoppedInsideShowExistingFrame`] for its last
    /// in-tree producer (the show-existing-frame `film_grain_config()` truncation);
    /// this variant is retained for completeness and out-of-tree compatibility but is
    /// no longer produced by the in-tree parser. It stays on the silent side of the
    /// truncation partition (an ordinary bounded stop, never a truncation).
    CoreFieldsOnly,
    /// The show-existing-frame path was consumed in full: `frame_to_show_map_idx`,
    /// `derive_sef_order_hint`, the optional `sef_order_hint`, and the terminal
    /// `film_grain_config()` (§ 5.18.10.1, mirror :4186) all parsed, after which the SEF
    /// path `return`s (mirror :4196). The frame header is complete.
    ShowExistingFrameComplete,
    /// An intra frame header was consumed in full through its § 5.18.2 tail
    /// (`read_tx_mode()` § 5.18.8.1, the no-bit `frame_reference_mode()` /
    /// `skip_mode_params()` / `allow_bawp` / `allow_warpmv_mode` intra inferences,
    /// `reduced_tx_set`, the no-bit intra arm of `global_motion_params()` § 5.18.9.1,
    /// and `film_grain_config()` § 5.18.10.1). The frame header is complete: every
    /// `frame_header_info()` field on the intra path has been read or derived. No
    /// full-payload trailing-bits conformance is implied — the frame header is followed
    /// by the rest of `tile_group_obu()` (§ 5.19), whose `trailing_bits()` reachability
    /// is tracked separately (AV2-5.19-TILE-GROUP / NumFrameHeaderBits).
    IntraHeaderComplete,
    /// A non-intra frame header was consumed in full through its § 5.18.2 shared tail and
    /// inter-specific arms. After the inter control region reached
    /// [`InterStop::ReachedSharedTail`](crate::headers::frame::inter::InterStop), the shared
    /// tail parsed `tile_info()` (§ 5.18.7.2), `quantization_params()` (§ 5.18.6.1),
    /// `segmentation_params()` (§ 5.18.7.1), `setup_qm_params()` (§ 5.18.6.2),
    /// `delta_q_params()` (§ 5.18.7.8), the § 5.18.2 lossless / `allow_tcq` /
    /// `allow_parity_hiding` derivation, the loop-filter cluster (`deblocking_filter_params()`
    /// with the inter `allow_df_sub_pu` arm, `gdf_params()`, `cdef_params()`, `lr_params()`,
    /// `ccso_params()`), and the inter tail (`read_tx_mode()` § 5.18.8.1,
    /// `frame_reference_mode()`'s `reference_select` § 5.18.8.3, `skip_mode_params()`'s
    /// `skip_mode_present` § 5.18.8.2, the gated `allow_bawp` / `allow_warpmv_mode`,
    /// `reduced_tx_set`, `global_motion_params()` § 5.18.9.1, and `film_grain_config()`
    /// § 5.18.10.1), reaching the terminal. The frame header is complete; the inter facts are
    /// on `core.inter` and the shared-tail facts on the shared `core` fields
    /// (`tile_info`/`quantization_params`/…). No full-payload trailing-bits conformance is
    /// implied (§ 5.19 tile data follows). Only reached for the minimal-tool single-reference
    /// inter subset the shared tail models exactly; anything outside it (segmentation on,
    /// `use_global_motion` warp models, the TIP / bridge return arms) stays an honest
    /// [`Self::UnsupportedUntilFeature`] coverage stop.
    InterHeaderComplete,
    /// An intra frame's control region was read through `disable_cdf_update`,
    /// `tile_info()` (§ 5.18.7.2), `quantization_params()` (§ 5.18.6.1),
    /// `segmentation_params()` (§ 5.18.7.1), `setup_qm_params()` (§ 5.18.6.2),
    /// `delta_q_params()` (§ 5.18.7.8), the § 5.18.2 lossless/`allow_tcq`/
    /// `allow_parity_hiding` tail, and the loop-filter cluster
    /// `deblocking_filter_params()` (§ 5.18.5.2), `gdf_params()` (§ 5.18.7.9), and
    /// `cdef_params()` (§ 5.18.7.10); the parser stopped before `lr_params()`
    /// (loop restoration, § 5.18.7.11).
    ///
    /// Reserved: superseded by [`Self::StoppedBeforeReadTxMode`] once `lr_params()` and
    /// `ccso_params()` parse on the intra path. The current intra path advances past both
    /// and reports the next stop; this variant is retained for completeness and
    /// out-of-tree compatibility but is no longer produced by the in-tree parser.
    StoppedBeforeLoopRestorationParams,
    /// An intra frame's control region was read in full through `cdef_params()`,
    /// then `lr_params()` (loop restoration, § 5.18.7.11) and `ccso_params()`
    /// (§ 5.18.7.12); the parser stopped before `read_tx_mode()` (§ 5.18.8.1), the next
    /// § 5.18.2 tail structure (mirror :5307).
    ///
    /// Reserved: superseded by [`Self::IntraHeaderComplete`] (and
    /// [`Self::StoppedInsideIntraTail`] on truncation) once the § 5.18.2 intra tail
    /// parses to completion. The current intra path advances past `read_tx_mode()`;
    /// this variant is retained for completeness and out-of-tree compatibility but is
    /// no longer produced by the in-tree parser.
    StoppedBeforeReadTxMode,
    /// Reserved for an unsupported loop-restoration branch discovered before `lr_params()`
    /// can be represented completely. The fixed-coded frame-level
    /// `read_wienerns_filter(plane, 0, 0, 1)` path (mirror :7377; § 5.20.10.6) is now
    /// modeled and stored on
    /// [`LrPlaneParams::frame_filter_bank`](crate::headers::frame::LrPlaneParams::frame_filter_bank),
    /// so this status is
    /// not produced for that path by the in-tree parser. It remains a non-truncation coverage
    /// stop for out-of-tree compatibility and future unsupported Wiener branches.
    /// `feature_id` is the implementation-matrix row for the missing decode.
    StoppedBeforeWienerNsFilter {
        /// Implementation-matrix Feature ID for the unsupported `read_wienerns_filter()` branch.
        feature_id: &'static str,
    },
    /// An intra frame's control region was read in full through the § 5.18.2
    /// lossless/`allow_tcq`/`allow_parity_hiding` tail, but the payload ran out
    /// **inside** the loop-filter cluster `deblocking_filter_params()` (§ 5.18.5.2),
    /// `gdf_params()` (§ 5.18.7.9), `cdef_params()` (§ 5.18.7.10), `lr_params()`
    /// (§ 5.18.7.11), or `ccso_params()` (§ 5.18.7.12). The already-parsed
    /// control-region facts (frame size, output flags, tile/quant/segmentation, and any
    /// cluster structure that completed before the truncation) are intact and exposed; the
    /// cluster fields that were not reached stay `None`. The truncation itself is a
    /// payload-bounds condition, not a structural violation, so it is reported through this
    /// status rather than as a hard parse error — earlier state-supported diagnostics still
    /// see the preserved facts (the pre-cluster behavior, which stopped here before any
    /// filter read, is preserved). No full-payload trailing-bits conformance is implied.
    StoppedInsideFilterParams,
    /// An intra frame parsed cleanly through `ccso_params()` (§ 5.18.7.12) but the
    /// payload ran out **inside** the § 5.18.2 tail (`read_tx_mode()` § 5.18.8.1,
    /// `reduced_tx_set`, or `film_grain_config()` § 5.18.10.1). The control-region and
    /// loop-filter-cluster facts and any tail field read before the EOF are preserved;
    /// the unreached tail (`intra_tail`) stays `None`. Like
    /// [`Self::StoppedInsideFilterParams`], the truncation is a payload-bounds condition,
    /// not a structural violation, so it is reported through this status rather than as a
    /// hard parse error. No full-payload trailing-bits conformance is implied.
    StoppedInsideIntraTail,
    /// The show-existing-frame path parsed `frame_to_show_map_idx`,
    /// `derive_sef_order_hint`, and the optional `sef_order_hint`, but the payload ran out
    /// **inside** the terminal `film_grain_config()` (§ 5.18.10.1, mirror :4186) — the SEF
    /// tail *is* `film_grain_config()`, so an EOF there is a truncation of a fully-modeled
    /// region, not an unsupported-coverage stop. The already-parsed SEF facts
    /// (`frame_to_show_map_idx`, the order hint, the output flags, `refresh_frame_flags`)
    /// are intact and exposed; `sef_film_grain` stays `None`. Like
    /// [`Self::StoppedInsideIntraTail`] / [`Self::StoppedInsideFilterParams`], the
    /// truncation is a payload-bounds condition, not a structural violation, so it is
    /// reported through this status rather than as a hard parse error — but it is on the
    /// truncated-in-modeled-region side of the partition, distinct from the ordinary
    /// bounded [`Self::CoreFieldsOnly`] stop it previously (incorrectly) shared.
    StoppedInsideShowExistingFrame,
    /// A non-intra frame's `frame_header_info()` reached the § 5.18.2 inter / switch / TIP /
    /// bridge control region (after `frame_size_override_flag` / `order_hint`, or after the
    /// bridge's `bridge_frame_ref_idx`), but the payload ran out **inside** one of the
    /// modeled control fields — the primary-reference signaling, `bridge_frame_overwrite_flag`,
    /// `refresh_frame_flags`, the explicit reference map (`num_total_refs` / `ref_frame_idx`),
    /// the reference-grounded frame size, or any field through `disable_cdf_update`. That
    /// region IS fully modeled up to its coverage stops
    /// ([`InterStop`](crate::headers::frame::inter::InterStop)), so the parser only returns
    /// `Ok` at a coverage stop or `Err(UnexpectedEof)` while reading a mandated field; this
    /// status records the EOF case. The fields parsed before the EOF are intact and exposed on
    /// `inter` (preserved via the caller-owned `control`), and earlier core facts survive.
    /// Like the other `StoppedInside*` statuses, the truncation is a payload-bounds condition
    /// reported through this status rather than a hard parse error (codex F2) — it is on the
    /// truncated-in-modeled-region side of the partition, distinct from the silent
    /// unsupported-coverage [`Self::UnsupportedUntilFeature`] a clean coverage stop sets.
    StoppedInsideInterControl,
    /// A branch needs decoder/reference state or syntax this phase does not model
    /// (e.g. the inter reference map, or the rest of a bridge frame). `feature_id` is
    /// the implementation-matrix row that tracks the missing coverage.
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
            Self::CoreFieldsOnly => "core_fields_only",
            Self::ShowExistingFrameComplete => "show_existing_frame_complete",
            Self::IntraHeaderComplete => "intra_header_complete",
            Self::InterHeaderComplete => "inter_header_complete",
            Self::StoppedBeforeLoopRestorationParams => "stopped_before_loop_restoration_params",
            Self::StoppedBeforeReadTxMode => "stopped_before_read_tx_mode",
            Self::StoppedBeforeWienerNsFilter { .. } => "stopped_before_wienerns_filter",
            Self::StoppedInsideFilterParams => "stopped_inside_filter_params",
            Self::StoppedInsideIntraTail => "stopped_inside_intra_tail",
            Self::StoppedInsideShowExistingFrame => "stopped_inside_show_existing_frame",
            Self::StoppedInsideInterControl => "stopped_inside_inter_control",
            Self::UnsupportedUntilFeature { .. } => "unsupported_until_feature",
        }
    }

    /// `true` when the status records an EOF **inside a region this parser fully
    /// models** — i.e. the OBU payload ended where the spec mandates more syntax, a
    /// decidable bitstream defect (the truncated-in-modeled-region side of the
    /// [enum partition](Self#truncation-partition)). Exactly
    /// [`Self::StoppedInsideFilterParams`], [`Self::StoppedInsideIntraTail`],
    /// [`Self::StoppedInsideShowExistingFrame`], and [`Self::StoppedInsideInterControl`].
    ///
    /// Coverage stops and complete parses return `false`: an early stop whose following
    /// syntax is unmodeled ([`Self::StoppedBeforeWienerNsFilter`],
    /// [`Self::UnsupportedUntilFeature`], [`Self::CoreFieldsOnly`], the reserved
    /// `StoppedBefore*` variants) is not evidence of a truncated payload, and a complete
    /// header ([`Self::IntraHeaderComplete`], [`Self::ShowExistingFrameComplete`]) was not
    /// truncated at all. The validator fires its truncated-frame-header diagnostic on
    /// exactly the `true` set.
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
