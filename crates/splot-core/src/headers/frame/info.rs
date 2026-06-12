// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! State-aware AV2 frame-header **core** parsing
//! (AV2 v1.0.0 § 5.18.2 `frame_header_info()`,
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`).
//!
//! This extends the activation-prefix parser ([`super::parse_frame_header_prefix`])
//! into the control region of `frame_header_info()` for the paths whose syntax is
//! fully determined by already-parsed state (the active sequence header). It is **not**
//! a full `frame_header()` parser: it stops with an explicit
//! [`FrameHeaderParseStatus`] at the first point that needs reference-frame buffer
//! state or the deep § 5.18.5–§ 5.18.10 structures, and it never guesses.
//!
//! Modeled paths and their stop points:
//! - **No sequence state / activation-prefix mode** → reads only the activation
//!   fields ([`FrameHeaderParseStatus::ActivationFieldsOnly`]).
//! - **Bridge frame** → reads `bridge_frame_ref_idx`, then parses the `IsBridge`
//!   reference-control region via [`super::inter::parse_inter_control`]
//!   (`bridge_frame_overwrite_flag`, the bridge `refresh_frame_flags` arms,
//!   `NumTotalRefs = 1`, `ref_frame_idx[0] = bridge_frame_ref_idx`, and
//!   `frame_size_with_bridge()` § 5.18.4.2). It then reaches the `IsBridge` early-return
//!   arm (§ 5.18.2 mirror :4971/:5045) and stops with
//!   [`super::inter::InterStop::BruInactiveOrBridgeReturn`] — the bridge tail
//!   (`film_grain_config()` / `tile_info()`) needs reference-frame dims this phase does not
//!   thread — recording the parsed bridge facts on `core.inter` and reporting
//!   [`FrameHeaderParseStatus::UnsupportedUntilFeature`].
//! - **Show-existing-frame (SEF)** → reads `frame_to_show_map_idx`,
//!   `derive_sef_order_hint`, `sef_order_hint`, and the terminal `film_grain_config()`
//!   (§ 5.18.10.1), completing the SEF frame header
//!   ([`FrameHeaderParseStatus::ShowExistingFrameComplete`]). A payload EOF inside the
//!   SEF `film_grain_config()` preserves the parsed SEF facts and reports
//!   [`FrameHeaderParseStatus::StoppedInsideShowExistingFrame`] — the SEF tail *is*
//!   `film_grain_config()`, so an EOF there is a truncation of a fully-modeled region,
//!   distinct from the ordinary bounded [`FrameHeaderParseStatus::CoreFieldsOnly`] stop.
//! - **Inter / switch / TIP / RAS frame** → reads the inter output-control flags and
//!   `order_hint`, then parses the § 5.18.2 reference-control region via
//!   [`super::inter::parse_inter_control`]: the primary-reference signaling, the explicit
//!   reference map (`frame_explicit_ref_frame_map`, `num_total_refs`, `ref_frame_idx[i]`),
//!   the reference-grounded frame size (`frame_size_with_refs()` § 5.18.4.3), the BRU
//!   triple, `use_ref_frame_mvs` / `tmvp_sample_step_minus_1`, the TIP block, the
//!   MV-precision / interpolation-filter / motion-mode reads, and `disable_cdf_update`
//!   (mirror :5041) — converging on the shared tail
//!   ([`super::inter::InterStop::ReachedSharedTail`]) or stopping at one of the honest
//!   [`super::inter::InterStop`] coverage stops (an unmodeled derivation such as the
//!   implicit reference map, a poisoned reference-state slot, or the TIP-as-output /
//!   bru-inactive / bridge early-return arms). The parsed inter facts are recorded on
//!   `core.inter`; the frame still stops with
//!   [`FrameHeaderParseStatus::UnsupportedUntilFeature`] because the shared tail past the
//!   control region is not yet threaded with the inter primary-reference / TIP inputs.
//! - **Intra frame (key / intra-only / single-picture)** → reads the full control
//!   region through `frame_size()`, `screen_content_params()`, `intrabc_params()`,
//!   `disable_cdf_update`, `tile_info()`, `quantization_params()`,
//!   `segmentation_params()`, `setup_qm_params()`, `delta_q_params()`, the
//!   § 5.18.2 lossless/`allow_tcq`/`allow_parity_hiding` tail, and the loop-filter
//!   cluster `deblocking_filter_params()` (§ 5.18.5.2), `gdf_params()` (§ 5.18.7.9),
//!   `cdef_params()` (§ 5.18.7.10), `lr_params()` (loop restoration, § 5.18.7.11),
//!   `ccso_params()` (§ 5.18.7.12), and the § 5.18.2 tail `read_tx_mode()` (§ 5.18.8.1),
//!   the no-bit `frame_reference_mode()` / `skip_mode_params()` / `allow_bawp` /
//!   `allow_warpmv_mode` intra inferences, `reduced_tx_set`, the no-bit intra arm of
//!   `global_motion_params()` (§ 5.18.9.1), and `film_grain_config()` (§ 5.18.10.1),
//!   completing the intra frame header
//!   ([`FrameHeaderParseStatus::IntraHeaderComplete`]). A payload EOF inside the tail
//!   preserves every earlier fact and reports
//!   [`FrameHeaderParseStatus::StoppedInsideIntraTail`]. When a plane in `lr_params()`
//!   signals `frame_filters_on`, the structure reaches the unmodeled
//!   `read_wienerns_filter()` frame-level Wiener bank decode and the parser stops with
//!   [`FrameHeaderParseStatus::StoppedBeforeWienerNsFilter`] (the control-region and
//!   pre-Wiener facts preserved). A payload that runs out **inside** the loop-filter
//!   cluster (deblocking through ccso) instead keeps the already-parsed control-region
//!   facts and reports the truncation as
//!   [`FrameHeaderParseStatus::StoppedInsideFilterParams`] rather than failing the whole
//!   parse (so earlier state-supported diagnostics still see the facts). On the
//!   `cur_mfh_id > 0` path the resolved in-band multi-frame header's § 5.7 state is
//!   threaded in (the [`MultiFrameHeaderRecord`] passed via
//!   [`FrameHeaderParseInput::mfh_record`]) so the § 5.18.4.1 default dimensions, the
//!   § 5.18.7.1 MFH-gated segmentation arm, and the § 5.18.5.2 MFH deblocking arm
//!   parse the same as the direct path; a `cur_mfh_id > 0` frame whose MFH is
//!   unresolvable still stops with
//!   [`FrameHeaderParseStatus::UnsupportedUntilFeature`] rather than guessing.

use crate::bitio::BitReader;
use crate::error::{Error, Result, TrailingBitsErrorKind};
use crate::headers::frame::config::{parse_intrabc_params, parse_screen_content_params};
use crate::headers::frame::filtering::{
    CdefParams, CoreSeqFilterView, DeblockingFilterParams, GdfGeometry, GdfParams,
    MfhDeblockingView, parse_cdef_params, parse_deblocking_filter_params, parse_gdf_params,
};
use crate::headers::frame::quant::{
    CoreSeqQuantView, DeltaQParams, LosslessInfo, QuantizationParams, SetupQmParams,
    parse_delta_q_params, parse_lossless_info, parse_quantization_params, parse_setup_qm_params,
};
use crate::headers::frame::restoration::{
    CcsoParams, CoreSeqCcsoView, CoreSeqRestorationView, LrGeometry, LrParams, LrParseOutcome,
    LrPartialParams, parse_ccso_params, parse_lr_params,
};
use crate::headers::frame::segmentation::{
    CoreSeqSegView, MfhSegView, SegmentationParams, parse_segmentation_params,
};
use crate::headers::frame::size::{FrameSize, ceil_log2, parse_frame_size};
use crate::headers::frame::tail::{
    FilmGrainConfig, FrameHeaderTail, FrameTailInput, parse_film_grain_config,
    parse_intra_tail as parse_intra_tail_grammar,
};
use crate::headers::frame::tiling::{CoreSeqTileView, TileInfo, parse_tile_info};
use crate::headers::sequence::{ChromaFormatIdc, SequenceHeader, SequenceHeaderId};
use crate::hls::{MfhId, MultiFrameHeaderRecord};
use crate::obu::parse_trailing_bits;
use crate::types::ObuType;

use super::{FrameHeaderPrefix, parse_frame_header_prefix};

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
///   [`Self::StoppedInsideFilterParams`], [`Self::StoppedInsideIntraTail`], and
///   [`Self::StoppedInsideShowExistingFrame`].
/// - **Bounded coverage stop / complete parse** — the parser stopped at a point whose
///   following syntax it does NOT fully model (unsupported coverage), or it completed,
///   so an early stop is *not* evidence of a truncated payload and must stay silent:
///   [`Self::ActivationFieldsOnly`], [`Self::CoreFieldsOnly`],
///   [`Self::ShowExistingFrameComplete`], [`Self::IntraHeaderComplete`],
///   [`Self::StoppedBeforeLoopRestorationParams`], [`Self::StoppedBeforeReadTxMode`],
///   [`Self::StoppedBeforeWienerNsFilter`], and [`Self::UnsupportedUntilFeature`].
///
/// The partition is exact: every status producer in this module either reaches a
/// modeled-region EOF (the three `StoppedInside*` statuses) or sets a coverage/complete
/// status. `CoreFieldsOnly` is deliberately on the silent side — it is reserved for an
/// ordinary bounded stop, never a truncation (the SEF film-grain EOF, which previously
/// reused it, now reports [`Self::StoppedInsideShowExistingFrame`]).
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
    /// An intra frame parsed through `cdef_params()` and into `lr_params()`
    /// (§ 5.18.7.11), but a plane signalled `frame_filters_on[plane]`, so the structure
    /// reached `read_wienerns_filter(plane, 0, 0, 1)` (mirror :7377) — a frame-level Wiener
    /// non-separable bank decode (`search_frame_filters()`, `predict_group()`,
    /// `decode_signed_subexp_with_ref()`) this phase does not model. The control-region and
    /// pre-Wiener `lr_params()` facts are intact and exposed; `read_tx_mode()` and beyond
    /// are unreached. `feature_id` is the implementation-matrix row for the missing decode.
    StoppedBeforeWienerNsFilter {
        /// Implementation-matrix Feature ID for the unmodeled `read_wienerns_filter()`.
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
            Self::StoppedBeforeLoopRestorationParams => "stopped_before_loop_restoration_params",
            Self::StoppedBeforeReadTxMode => "stopped_before_read_tx_mode",
            Self::StoppedBeforeWienerNsFilter { .. } => "stopped_before_wienerns_filter",
            Self::StoppedInsideFilterParams => "stopped_inside_filter_params",
            Self::StoppedInsideIntraTail => "stopped_inside_intra_tail",
            Self::StoppedInsideShowExistingFrame => "stopped_inside_show_existing_frame",
            Self::UnsupportedUntilFeature { .. } => "unsupported_until_feature",
        }
    }

    /// `true` when the status records an EOF **inside a region this parser fully
    /// models** — i.e. the OBU payload ended where the spec mandates more syntax, a
    /// decidable bitstream defect (the truncated-in-modeled-region side of the
    /// [enum partition](Self#truncation-partition)). Exactly
    /// [`Self::StoppedInsideFilterParams`], [`Self::StoppedInsideIntraTail`], and
    /// [`Self::StoppedInsideShowExistingFrame`].
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

/// A read-only view of reference-frame buffer state for frame-header decisions
/// (AV2 v1.0.0 § 7.23).
///
/// The validator models the § 7.23 reference-frame buffer state and threads it in via
/// [`FrameReferenceStateView::from_slots`]; callers with no modeled buffer (the
/// inspector, a direct/fuzz caller) pass [`FrameReferenceStateView::unknown`]. The core
/// parser does **not** yet branch on it: no § 5.18 intra parse path reads
/// `RefValid`/`RefOrderHint`/dims, so today the view is forward plumbing for the § 5.18
/// inter reference-state-dependent paths (explicit reference maps,
/// `frame_size_with_refs()`, `primary_ref_frame`). The validator already consumes the
/// modeled state directly for the § 6.17.2 show-existing-frame slot-validity check.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct FrameReferenceStateView<'a> {
    /// `RefValid[ i ]` per reference slot, when modeled.
    pub ref_valid: Option<&'a [bool]>,
    /// `RefOrderHint[ i ]` per reference slot, when modeled.
    pub ref_order_hint: Option<&'a [u32]>,
    /// `RefFrameWidth[ i ]` per reference slot, when modeled.
    pub ref_frame_width: Option<&'a [u32]>,
    /// `RefFrameHeight[ i ]` per reference slot, when modeled.
    pub ref_frame_height: Option<&'a [u32]>,
}

impl<'a> FrameReferenceStateView<'a> {
    /// A fully-unknown reference state (passed when the caller models no reference
    /// buffer for this layer).
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            ref_valid: None,
            ref_order_hint: None,
            ref_frame_width: None,
            ref_frame_height: None,
        }
    }

    /// Builds a reference state from the caller's modeled `RefValid[]` / `RefOrderHint[]`
    /// / `RefFrameWidth[]` / `RefFrameHeight[]` slices (AV2 § 7.23).
    ///
    /// The slices are parallel, one entry per reference slot. The caller (the validator's
    /// § 7.23 buffer model) owns the backing storage; the view borrows it for the parse.
    /// A `cur_mfh_id == 0` / intra parse does not read these today — the constructor is
    /// the forward-plumbing entry point for the § 5.18 inter reference paths.
    #[must_use]
    pub const fn from_slots(
        ref_valid: &'a [bool],
        ref_order_hint: &'a [u32],
        ref_frame_width: &'a [u32],
        ref_frame_height: &'a [u32],
    ) -> Self {
        Self {
            ref_valid: Some(ref_valid),
            ref_order_hint: Some(ref_order_hint),
            ref_frame_width: Some(ref_frame_width),
            ref_frame_height: Some(ref_frame_height),
        }
    }
}

/// Explicit inputs for [`parse_frame_header_core`] (AV2 v1.0.0 § 5.18.2).
///
/// Frame-header parsing depends on state the bitstream does not repeat: the active
/// sequence header, the resolving multi-frame header, the temporal-unit position, and
/// reference-frame buffers. Passing them explicitly keeps those dependencies visible
/// and lets a caller (or test) request a structured partial result by withholding
/// state rather than having the parser invent it.
#[derive(Debug, Clone, Copy)]
pub struct FrameHeaderParseInput<'a> {
    /// The OBU type carrying this frame header.
    pub obu_type: ObuType,
    /// `FirstPictureInTU` decoder state, used to derive `startCVS`.
    pub first_picture_in_tu: bool,
    /// The active sequence header this frame resolves to (`load_sequence_header()`),
    /// or `None` when it is unavailable. Core mode needs a fully parsed sequence
    /// header (with its inter and screen-content configs) to read beyond the
    /// activation fields; otherwise the result is [`FrameHeaderParseStatus::ActivationFieldsOnly`].
    pub active_sequence: Option<&'a SequenceHeader>,
    /// The multi-frame header resolving a `cur_mfh_id > 0` reference, when available.
    /// Its parsed § 5.7 state supplies the § 5.18.4.1 default frame dimensions (with the
    /// § 5.18.2 omitted-size inference) and the § 5.18.7.1 MFH-gated
    /// `segmentation_params()` arm. `None` for a `cur_mfh_id == 0` direct reference, or
    /// when the in-band MFH is unresolvable — the latter keeps the unsupported/Unknown
    /// routing rather than guessing field positions.
    pub mfh_record: Option<&'a MultiFrameHeaderRecord>,
    /// Reference-frame buffer state (see [`FrameReferenceStateView`]).
    pub reference_state: FrameReferenceStateView<'a>,
    /// Which parser path to take.
    pub mode: FrameHeaderParseMode,
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

/// Validates `trailing_bits( remainingPayloadBits )` over the rest of `reader`'s payload
/// for a show-existing-frame OBU (AV2 § 5.2.1 / § 5.2.3), classifying the outcome without
/// failing the parse so the already-parsed SEF facts survive.
fn classify_sef_trailing_bits(reader: &mut BitReader<'_>) -> SefTrailingBits {
    match parse_trailing_bits(reader, reader.remaining_bits()) {
        Ok(()) => SefTrailingBits::Valid,
        Err(Error::InvalidTrailingBits { kind, .. }) => match kind {
            TrailingBitsErrorKind::Empty => SefTrailingBits::Empty,
            TrailingBitsErrorKind::MissingOneBit => SefTrailingBits::MissingOneBit,
            TrailingBitsErrorKind::ZeroBitNotZero => SefTrailingBits::ZeroBitNotZero,
        },
        // `parse_trailing_bits` reads exactly `remaining_bits()` bits, so it cannot run
        // past the payload; an EOF here is unreachable, but treat it conservatively as a
        // missing marker rather than panicking.
        Err(_) => SefTrailingBits::MissingOneBit,
    }
}

/// A state-aware core parse of `frame_header_info()` (AV2 v1.0.0 § 5.18.2).
///
/// Fields beyond the activation prefix are `Option`, present only when the
/// corresponding syntax was reached and exactly determined by parsed state. The
/// [`status`](Self::status) records where parsing stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameHeaderCore {
    /// The OBU type carrying this frame header.
    pub obu_type: ObuType,
    /// Where the parse stopped.
    pub status: FrameHeaderParseStatus,
    /// `isFirst`: always `true` (the core path is the first-header path).
    pub is_first: bool,
    /// `keyFrame` derived from `obu_type`.
    pub is_key_frame: bool,
    /// `IsRegular` derived from `obu_type`.
    pub is_regular: bool,
    /// `IsBridge` derived from `obu_type`.
    pub is_bridge: bool,
    /// `startCVS`.
    pub starts_cvs: bool,
    /// `cur_mfh_id` (inferred `0` for bridge frames).
    pub cur_mfh_id: MfhId,
    /// `seq_header_id_in_frame_header` raw value, present when `cur_mfh_id == 0`.
    pub seq_header_id_in_frame_header: Option<u32>,
    /// The directly referenced sequence header id when in range and `cur_mfh_id == 0`.
    pub referenced_sequence_header_id: Option<SequenceHeaderId>,
    /// `ShowExistingFrame`, when the single-picture/SEF branch was evaluated.
    pub show_existing_frame: Option<bool>,
    /// `FrameType`, when derived.
    pub frame_type: Option<FrameType>,
    /// `FrameIsIntra`, when derived.
    pub frame_is_intra: Option<bool>,
    /// `immediate_output_frame`, when reached.
    pub immediate_output_frame: Option<bool>,
    /// `implicit_output_frame`, when reached.
    pub implicit_output_frame: Option<bool>,
    /// `OrderHintLsbs` (`order_hint` / `sef_order_hint`), when read.
    pub order_hint_lsb: Option<u32>,
    /// `refresh_frame_flags`, when derived or read.
    pub refresh_frame_flags: Option<u32>,
    /// `FrameWidth`/`FrameHeight` from `frame_size()`, when exactly known.
    pub frame_size: Option<FrameSize>,
    /// `frame_size_override_flag` (AV2 § 5.18.4 / § 5.18.2), when the intra tail read
    /// or inferred it. This records the *provenance* of [`Self::frame_size`]: on the
    /// `cur_mfh_id > 0` non-override path (`Some(false)`) `FrameWidth`/`FrameHeight`
    /// come from the resolved multi-frame header's stored default dimensions
    /// (`mfh_frame_width/height_minus_1 + 1`, § 5.18.4.1, mirror :5767), whereas on the
    /// override path (`Some(true)`) they come from this frame's explicit
    /// `frame_width_minus_1` / `frame_height_minus_1` fields. A single-picture key frame
    /// infers it `false` without reading a bit. `None` when the parse stopped before the
    /// intra tail.
    pub frame_size_override_flag: Option<bool>,
    /// `bridge_frame_ref_idx`, when read (bridge frames).
    pub bridge_frame_ref_idx: Option<u32>,
    /// `frame_to_show_map_idx`, when read (show-existing-frame).
    pub frame_to_show_map_idx: Option<u32>,
    /// `allow_screen_content_tools`, when `screen_content_params()` was reached.
    pub allow_screen_content_tools: Option<bool>,
    /// `allow_intrabc`, when `intrabc_params()` was reached.
    pub allow_intrabc: Option<bool>,
    /// `true` if any `ref_long_term_id[i]` equals the reserved value
    /// `(1 << long_term_frame_id_bits) - 1`, which AV2 § 6.17.2 forbids.
    pub forbidden_ref_long_term_id: bool,
    /// Parsed `tile_info()` (AV2 § 5.18.7.2), when reached on the intra path.
    pub tile_info: Option<TileInfo>,
    /// Parsed `quantization_params()` (AV2 § 5.18.6.1), when reached.
    pub quantization_params: Option<QuantizationParams>,
    /// Parsed `segmentation_params()` (AV2 § 5.18.7.1), when reached.
    pub segmentation_params: Option<SegmentationParams>,
    /// Parsed `setup_qm_params()` (AV2 § 5.18.6.2), when reached. Per § 5.18.2 call
    /// order it is parsed **after** `segmentation_params()`.
    pub setup_qm_params: Option<SetupQmParams>,
    /// Parsed `delta_q_params()` (AV2 § 5.18.7.8), when reached.
    pub delta_q_params: Option<DeltaQParams>,
    /// The § 5.18.2 per-segment lossless/QM derivation and the `allow_tcq` /
    /// `allow_parity_hiding` reads, when reached.
    pub lossless_info: Option<LosslessInfo>,
    /// Parsed `deblocking_filter_params()` (AV2 § 5.18.5.2), when reached on the intra
    /// tail (after the lossless derivation).
    pub deblocking_filter_params: Option<DeblockingFilterParams>,
    /// Parsed `gdf_params()` (AV2 § 5.18.7.9), when reached. Per § 5.18.2 call order it
    /// is parsed **after** `deblocking_filter_params()`.
    pub gdf_params: Option<GdfParams>,
    /// Parsed `cdef_params()` (AV2 § 5.18.7.10), when reached. Per § 5.18.2 call order
    /// it is parsed **after** `gdf_params()`.
    pub cdef_params: Option<CdefParams>,
    /// Parsed `lr_params()` (AV2 § 5.18.7.11), when reached on the intra tail (after
    /// `cdef_params()`) **and parsed to completion**. `None` when the parse stopped before
    /// it, or stopped inside the unmodeled frame-level Wiener bank decode — in the latter
    /// case the parsed prefix lives in [`Self::lr_params_partial`] instead (see
    /// [`FrameHeaderParseStatus::StoppedBeforeWienerNsFilter`]). This field always means a
    /// *complete* `lr_params()` parse, so consumers cannot mistake partial state for it.
    pub lr_params: Option<LrParams>,
    /// The partial `lr_params()` facts committed before the honest
    /// [`FrameHeaderParseStatus::StoppedBeforeWienerNsFilter`] stop (AV2 § 5.18.7.11): the
    /// per-plane restoration types, `frame_filters_on`, the luma `NumFilterClasses`, the
    /// derived `UsesLr`, and the `LoopRestorationSize` size flags. `Some` only on that stop
    /// (mutually exclusive with [`Self::lr_params`]); `None` otherwise. Kept separate from
    /// [`Self::lr_params`] so a partial parse is never mistaken for a complete one — the
    /// frame-level Wiener bank that follows was not parsed.
    pub lr_params_partial: Option<LrPartialParams>,
    /// Parsed `ccso_params()` (AV2 § 5.18.7.12), when reached. Per § 5.18.2 call order it
    /// is parsed **after** `lr_params()`.
    pub ccso_params: Option<CcsoParams>,
    /// The § 5.18.2 intra tail after `ccso_params()` — `read_tx_mode()` (§ 5.18.8.1),
    /// the no-bit `frame_reference_mode()` / `skip_mode_params()` / `allow_bawp` /
    /// `allow_warpmv_mode` intra inferences, `reduced_tx_set`, the no-bit intra arm of
    /// `global_motion_params()` (§ 5.18.9.1), and `film_grain_config()` (§ 5.18.10.1).
    /// `Some` only when the intra path parsed to completion
    /// ([`FrameHeaderParseStatus::IntraHeaderComplete`]).
    pub intra_tail: Option<FrameHeaderTail>,
    /// The show-existing-frame `film_grain_config()` (§ 5.18.10.1, mirror :4186). `Some`
    /// only on the SEF path once it parsed to completion
    /// ([`FrameHeaderParseStatus::ShowExistingFrameComplete`]); the SEF path reads only
    /// `film_grain_config()`, not the § 5.18.8 coding-mode tail.
    pub sef_film_grain: Option<FilmGrainConfig>,
    /// How a show-existing-frame OBU's `trailing_bits()` boundary resolved (AV2 § 5.2.1 /
    /// § 5.2.3). `Some` only on the [`FrameHeaderParseStatus::ShowExistingFrameComplete`]
    /// path: the SEF payload is exactly the SEF frame header plus
    /// `trailing_bits( remainingPayloadBits )` (no tile data), so the boundary is
    /// decidable from the payload alone. The validator surfaces a non-`Valid` outcome as a
    /// § 6.2.1 / § 5.2.3 diagnostic. `None` on every other path (no SEF boundary to check,
    /// or the SEF parse stopped before completing `film_grain_config()`).
    pub sef_trailing_bits: Option<SefTrailingBits>,
    /// The parsed non-intra control region (AV2 § 5.18.2, mirror :4351-5181), present on
    /// the inter / switch / TIP path (`frame_is_intra == Some(false)`, non-SEF) and on the
    /// bridge path (`parse_bridge_inter_path` records its control region here too). Carries
    /// the primary-reference signaling, the explicit reference map, the reference-grounded
    /// frame size, the BRU triple, `use_ref_frame_mvs` / TMVP, the TIP block, MV precision,
    /// the interpolation filter, and motion modes, plus the
    /// [`InterStop`](crate::headers::frame::inter::InterStop) recording where the inter
    /// region stopped. `None` on the intra / SEF paths.
    pub inter: Option<crate::headers::frame::inter::InterControl>,
    /// Bits consumed by this parse (not necessarily the whole frame header).
    pub consumed_bits: u64,
}

/// Matrix Feature ID for the frame-header-info coverage this phase does not model.
const FRAME_HEADER_INFO_FEATURE: &str = "AV2-5.18.2-FRAME-HEADER-INFO";

/// `MOTION_MODES` (AV2 v1.0.0 § 3): the motion-mode array length carried for the
/// § 5.18.2 inter motion-mode loop.
const MOTION_MODES: usize = 5;

/// The § 5.4.6 `sequence_inter_config()` flags the § 5.18.2 non-intra control region
/// consumes (AV2 v1.0.0 § 5.4.6), gathered alongside the rest of [`CoreSeqView`].
#[derive(Debug, Clone, Copy)]
struct CoreSeqInterView {
    enable_ref_frame_mvs: bool,
    explicit_ref_frame_map: bool,
    enable_bru: bool,
    enable_tip: bool,
    seq_max_drl_bits_minus_1: u32,
    allow_frame_max_drl_bits: bool,
    enable_flex_mvres: bool,
    seq_frame_motion_modes_present_flag: bool,
    seq_enabled_motion_modes: [bool; MOTION_MODES],
    enable_opfl_refine: u8,
}

/// Sequence-derived scalars the core parser needs, gathered from a fully parsed
/// [`SequenceHeader`]. `None` when any required child config (partition, segment,
/// inter, screen-content, transform/quant/entropy, or tile) is absent — the header
/// was not fully parsed — in which case core parsing degrades to the prefix.
///
/// The § 5.18.6 / § 5.18.7 inputs are grouped into per-structure sub-views
/// ([`CoreSeqQuantView`], [`CoreSeqSegView`], [`CoreSeqTileView`]) so each child
/// parser names exactly the state it consumes.
#[derive(Debug)]
struct CoreSeqView {
    num_ref_frames: u32,
    order_hint_bits: u32,
    long_term_frame_id_bits: u32,
    enable_short_refresh_frame_flags: bool,
    monotonic_output_order_flag: bool,
    single_picture_header_flag: bool,
    max_mlayer_id: u8,
    frame_width_bits: u32,
    frame_height_bits: u32,
    max_frame_width: u32,
    max_frame_height: u32,
    seq_force_screen_content_tools: u8,
    seq_force_integer_mv: u8,
    allow_frame_max_bvp_drl_bits: bool,
    /// § 5.4.6 inter-config inputs consumed by the § 5.18.2 non-intra control region
    /// ([`crate::headers::frame::inter`]).
    inter: CoreSeqInterView,
    /// § 5.18.6 / § 5.18.7.8 / § 5.18.2-lossless-tail inputs (AV2 § 5.4.8).
    quant: CoreSeqQuantView,
    /// § 5.18.7.1 segmentation inputs (AV2 § 5.4.4).
    seg: CoreSeqSegView,
    /// § 5.18.7.2 tile-info inputs (AV2 § 5.4.2 / § 5.4.3 / § 5.4.8).
    tile: CoreSeqTileView,
    /// § 5.18.5.2 / § 5.18.7.9 / § 5.18.7.10 loop-filter inputs (AV2 § 5.4.10).
    filter: CoreSeqFilterView,
    /// § 5.18.7.11 loop-restoration tool flags (AV2 § 5.4.10).
    restoration: CoreSeqRestorationView,
    /// § 5.18.7.12 CCSO inputs (AV2 § 5.4.10 / § 5.4.1).
    ccso: CoreSeqCcsoView,
    /// `chroma_format_idc` (AV2 § 5.4.1): the § 6.4.1 SubsamplingX/Y for `lr_params()`'s
    /// chroma `LoopRestorationSize` derivation.
    chroma_format_idc: ChromaFormatIdc,
    /// `film_grain_params_present` (AV2 § 5.4.1): gates the § 5.18.10.1
    /// `film_grain_config()` `apply_grain` derivation. `Some(false)` when the sequence
    /// header did not signal grain, `Some(true)` when it did. `None` when the active
    /// sequence header was recorded from a **bounded** stop that ended before
    /// `film_grain_params_present` (read last in § 5.4.1, after the child configs), e.g.
    /// the bounded `sequence_tile_config()` residual: the flag is then genuinely unknown.
    /// The control region (frame size, output flags, order hint, tile/quant/segmentation)
    /// does not consume this flag, so the parser still reaches and reports those facts; it
    /// stops honestly only when `film_grain_config()` itself needs the unknown flag.
    film_grain_params_present: Option<bool>,
}

impl CoreSeqView {
    fn from_sequence(seq: &SequenceHeader) -> Option<Self> {
        let partition = seq.partition.as_ref()?;
        let segment = seq.segment.as_ref()?;
        let inter = seq.inter.as_ref()?;
        let scc = seq.screen_content.as_ref()?;
        let tq = seq.transform_quant_entropy.as_ref()?;
        let tile = seq.tile.as_ref()?;
        // `sequence_filter_config()` (§ 5.4.10) gates the § 5.18.2 tail loop-filter
        // structures; without it the intra tail cannot reach deblocking/GDF/CDEF.
        let filter = seq.filter.as_ref()?;
        // `film_grain_params_present` (§ 5.4.1) is read last in the sequence header, AFTER
        // every child config above. A bounded `sequence_tile_config()` stop yields a header
        // with all those children present but this flag `None`. It is NOT required to read
        // the control region (frame size, output flags, order hint, tile/quant/segmentation,
        // the loop-filter cluster) — only `film_grain_config()` consumes it — so its absence
        // must NOT collapse the whole view (that would suppress every locally-decidable
        // frame-size / output / order-hint diagnostic). Carry it as `Option` and defer the
        // requirement to `film_grain_config()` consumption: an unknown flag there is an
        // honest stop with the parsed facts preserved, not a guess.
        let film_grain_params_present = seq.film_grain_params_present;
        let general = &seq.general;
        Some(Self {
            num_ref_frames: u32::from(inter.num_ref_frames),
            order_hint_bits: u32::from(inter.order_hint_bits),
            long_term_frame_id_bits: u32::from(inter.long_term_frame_id_bits),
            enable_short_refresh_frame_flags: inter.enable_short_refresh_frame_flags,
            monotonic_output_order_flag: general.monotonic_output_order_flag,
            single_picture_header_flag: general.single_picture_header_flag,
            max_mlayer_id: general.max_mlayer_id.get(),
            frame_width_bits: u32::from(general.frame_width_bits.get()),
            frame_height_bits: u32::from(general.frame_height_bits.get()),
            max_frame_width: general.max_frame_width.get(),
            max_frame_height: general.max_frame_height.get(),
            seq_force_screen_content_tools: scc.seq_force_screen_content_tools,
            seq_force_integer_mv: scc.seq_force_integer_mv,
            allow_frame_max_bvp_drl_bits: inter.allow_frame_max_bvp_drl_bits,
            // AV2 § 5.4.6: the inter-config flags consumed by the § 5.18.2 non-intra
            // control region.
            inter: CoreSeqInterView {
                enable_ref_frame_mvs: inter.enable_ref_frame_mvs,
                explicit_ref_frame_map: inter.explicit_ref_frame_map,
                enable_bru: inter.enable_bru,
                enable_tip: inter.enable_tip,
                seq_max_drl_bits_minus_1: inter.seq_max_drl_bits_minus_1,
                allow_frame_max_drl_bits: inter.allow_frame_max_drl_bits,
                enable_flex_mvres: inter.enable_flex_mvres,
                seq_frame_motion_modes_present_flag: inter.seq_frame_motion_modes_present_flag,
                seq_enabled_motion_modes: inter.seq_enabled_motion_modes,
                enable_opfl_refine: inter.enable_opfl_refine,
            },
            quant: CoreSeqQuantView::from_sequence_configs(general, tq),
            seg: CoreSeqSegView::from_sequence_config(segment),
            tile: CoreSeqTileView::from_sequence_configs(general, partition, tq, tile),
            // AV2 § 5.4.10: the loop-filter tool flags consumed by the § 5.18.2 tail.
            filter: CoreSeqFilterView {
                enable_cdef: filter.enable_cdef,
                enable_gdf: filter.enable_gdf,
                gdf_unit_matches_sb_size: filter.gdf_unit_matches_sb_size,
                disable_loopfilters_across_tiles: filter.disable_loopfilters_across_tiles,
                cdef_on_skip_txfm: filter.cdef_on_skip_txfm,
                df_par_bits_minus_2: filter.df_par_bits_minus_2,
                single_picture_header_flag: general.single_picture_header_flag,
            },
            // AV2 § 5.4.10: the loop-restoration tool flags consumed by lr_params().
            restoration: CoreSeqRestorationView {
                enable_restoration: filter.enable_restoration,
                lr_pc_wiener_disabled: filter.lr_pc_wiener_disabled,
                lr_wiener_nonsep_disabled: filter.lr_wiener_nonsep_disabled,
                lr_uv_pc_wiener_disabled: filter.lr_uv_pc_wiener_disabled,
                lr_uv_wiener_nonsep_disabled: filter.lr_uv_wiener_nonsep_disabled,
            },
            // AV2 § 5.4.10 / § 5.4.1: the CCSO inputs consumed by ccso_params().
            ccso: CoreSeqCcsoView {
                enable_ccso: filter.enable_ccso,
                single_picture_header_flag: general.single_picture_header_flag,
            },
            chroma_format_idc: general.chroma_format_idc,
            film_grain_params_present,
        })
    }
}

/// The resolved multi-frame header's § 5.7 state needed by the `cur_mfh_id > 0`
/// frame-header core path (AV2 v1.0.0 § 5.18.2), derived from a
/// [`MultiFrameHeaderRecord`] against the active sequence header's maxima.
///
/// Built only on the `cur_mfh_id > 0` path (with a resolved in-band record); on the
/// `cur_mfh_id == 0` direct path the parser keeps `None` and uses sequence state.
#[derive(Debug)]
struct MfhFrameView {
    /// `(FrameWidth, FrameHeight)` default dimensions for the § 5.18.4.1 non-override
    /// path: `mfh_frame_width/height_minus_1[ cur_mfh_id ] + 1`, with the § 5.18.2
    /// omitted-size inference (:4101) already applied — when the MFH carried no
    /// frame-size payload, these equal the sequence `max_frame_width/height`.
    default_dims: (u32, u32),
    /// The § 5.18.7.1 MFH-gated segmentation inputs, `Some` only when
    /// `mfh_seg_info_present_flag` is set (the gate selecting the MFH branch).
    seg: Option<MfhSegView>,
    /// The § 5.18.5.2 MFH deblocking-update inputs: `mfh_deblocking_filter_update`
    /// and `mfh_apply_deblocking_filter[0..4]` (AV2 § 5.7), consulted by the
    /// `cur_mfh_id > 0` deblocking arm (mirror :5949).
    deblocking: MfhDeblockingView,
}

impl MfhFrameView {
    /// Resolves a [`MultiFrameHeaderRecord`]'s § 5.7 state against the active
    /// sequence header's maxima for the `cur_mfh_id > 0` core path (AV2 § 5.18.2).
    fn from_record(record: &MultiFrameHeaderRecord, seq: &CoreSeqView) -> Self {
        // AV2 § 5.18.2 (:4101): `if ( cur_mfh_id == 0 || !mfh_frame_size_present_flag )`
        // infers `mfh_frame_width/height_minus_1[ cur_mfh_id ]` to `max_frame_*_minus_1`.
        // On this path `cur_mfh_id > 0`, so the inference applies exactly when the MFH
        // carried no frame-size payload; otherwise the explicit MFH dimensions are used
        // (§ 5.18.4.1, :5767). `width`/`height` here are the `*_minus_1 + 1` luma values.
        let default_dims = match record.mfh_frame_size {
            Some(size) => (
                size.width_minus_1.saturating_add(1),
                size.height_minus_1.saturating_add(1),
            ),
            None => (seq.max_frame_width, seq.max_frame_height),
        };
        // AV2 § 5.18.7.1: the MFH segmentation branch is gated on
        // `mfh_seg_info_present_flag`; build its view only then. The seg-info flags and
        // parsed feature data are present together with the flag (AV2 § 5.7).
        let seg = if record.mfh_seg_info_present_flag {
            match (
                record.mfh_ext_seg_flag,
                record.mfh_allow_seg_info_change,
                record.mfh_segment_info,
            ) {
                (Some(ext_seg), Some(allow_change), Some(segment_info)) => Some(MfhSegView {
                    mfh_ext_seg_flag: ext_seg,
                    mfh_allow_seg_info_change: allow_change,
                    mfh_segment_info: segment_info,
                }),
                // An inconsistent record (flag set without its payload) cannot select
                // the MFH branch soundly; fall back to the sequence/zero derivation
                // rather than guessing.
                _ => None,
            }
        } else {
            None
        };
        // AV2 § 5.18.5.2 (mirror :5949): the resolved MFH's deblocking-update state
        // for the `cur_mfh_id > 0` arm. `mfh_apply_deblocking_filter` is all-false
        // unless the record signalled an update (§ 5.7 parse), so copying it is safe
        // even when the update bit is clear (the arm is then not selected).
        let deblocking = MfhDeblockingView {
            mfh_deblocking_filter_update: record.mfh_deblocking_filter_update,
            mfh_apply_deblocking_filter: record.mfh_apply_deblocking_filter,
        };
        Self {
            default_dims,
            seg,
            deblocking,
        }
    }
}

/// Reads `f(n)`, treating `n == 0` as reading no bits (value `0`), matching the
/// AV2 convention that an `f(0)` field is absent.
fn read_f(reader: &mut BitReader<'_>, n: u32) -> Result<u32> {
    if n == 0 { Ok(0) } else { reader.read_bits(n) }
}

/// `allFrames = (1 << NumRefFrames) - 1` (AV2 § 5.18.2), saturating defensively.
fn all_frames_mask(num_ref_frames: u32) -> u32 {
    if num_ref_frames >= u32::BITS {
        u32::MAX
    } else {
        (1u32 << num_ref_frames).wrapping_sub(1)
    }
}

/// Parses the frame-header core (AV2 v1.0.0 § 5.18.2). See the module docs for the
/// modeled paths and stop points.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) or a typed
/// descriptor error if the payload ends or is malformed before a modeled field can be
/// read. A branch that needs unmodeled state returns `Ok` with a partial
/// [`FrameHeaderParseStatus`], never an error and never a guessed value.
pub fn parse_frame_header_core(
    reader: &mut BitReader<'_>,
    input: &FrameHeaderParseInput<'_>,
) -> Result<FrameHeaderCore> {
    let start_bits = reader.consumed_bits();

    // The activation/reference prefix is parsed exactly as the prefix parser does, so
    // existing behavior cannot regress (AV2 § 5.18.2 activation fields).
    let prefix = parse_frame_header_prefix(reader, input.obu_type, input.first_picture_in_tu)?;
    let mut core = init_core_from_prefix(&prefix, input.obu_type);

    // Activation-prefix mode, or core mode without a fully parsed active sequence
    // header, stops at the prefix: the next field (`order_hint`, `bridge_frame_ref_idx`,
    // …) needs OrderHintBits / NumRefFrames, which live in the sequence inter config.
    if input.mode == FrameHeaderParseMode::Core
        && let Some(seq) = input.active_sequence.and_then(CoreSeqView::from_sequence)
    {
        // Resolve the `cur_mfh_id > 0` multi-frame-header state once, against the active
        // sequence maxima. `None` when `cur_mfh_id == 0` (direct sequence reference) or
        // when the in-band MFH is unresolvable (the resolution guard upstream passes no
        // record), which keeps the unsupported/Unknown routing rather than guessing.
        let mfh_view = if core.cur_mfh_id.is_zero() {
            None
        } else {
            input
                .mfh_record
                .map(|record| MfhFrameView::from_record(record, &seq))
        };
        parse_core_body(
            reader,
            &mut core,
            &seq,
            mfh_view.as_ref(),
            &input.reference_state,
        )?;
    }

    core.consumed_bits = reader.consumed_bits().saturating_sub(start_bits);
    Ok(core)
}

/// Builds the initial core result from the activation prefix, with all post-prefix
/// fields unset and the conservative [`FrameHeaderParseStatus::ActivationFieldsOnly`]
/// status.
fn init_core_from_prefix(prefix: &FrameHeaderPrefix, obu_type: ObuType) -> FrameHeaderCore {
    FrameHeaderCore {
        obu_type,
        status: FrameHeaderParseStatus::ActivationFieldsOnly,
        is_first: prefix.is_first,
        is_key_frame: prefix.is_key_frame,
        is_regular: prefix.is_regular,
        is_bridge: prefix.is_bridge,
        starts_cvs: prefix.starts_cvs,
        cur_mfh_id: prefix.cur_mfh_id,
        seq_header_id_in_frame_header: prefix.seq_header_id_in_frame_header,
        referenced_sequence_header_id: prefix.referenced_sequence_header_id,
        show_existing_frame: None,
        frame_type: None,
        frame_is_intra: None,
        immediate_output_frame: None,
        implicit_output_frame: None,
        order_hint_lsb: None,
        refresh_frame_flags: None,
        frame_size: None,
        frame_size_override_flag: None,
        bridge_frame_ref_idx: None,
        frame_to_show_map_idx: None,
        allow_screen_content_tools: None,
        allow_intrabc: None,
        forbidden_ref_long_term_id: false,
        tile_info: None,
        quantization_params: None,
        segmentation_params: None,
        setup_qm_params: None,
        delta_q_params: None,
        lossless_info: None,
        deblocking_filter_params: None,
        gdf_params: None,
        cdef_params: None,
        lr_params: None,
        lr_params_partial: None,
        ccso_params: None,
        intra_tail: None,
        sef_film_grain: None,
        sef_trailing_bits: None,
        inter: None,
        consumed_bits: 0,
    }
}

/// Parses `frame_header_info()` beyond the activation prefix (AV2 § 5.18.2), setting
/// `core`'s fields and stop [`FrameHeaderParseStatus`]. The reader starts positioned
/// just after the activation/reference fields.
fn parse_core_body(
    reader: &mut BitReader<'_>,
    core: &mut FrameHeaderCore,
    seq: &CoreSeqView,
    mfh: Option<&MfhFrameView>,
    reference_state: &FrameReferenceStateView<'_>,
) -> Result<()> {
    let obu_type = core.obu_type;

    // AV2 § 5.18.2 (mirror :4117-4123): a bridge frame reads bridge_frame_ref_idx
    // f(CeilLog2(NumRefFrames)) immediately after load_sequence_header(), then takes the
    // IsBridge arm through the reference-control region. FrameType = INTER_FRAME (mirror
    // :4203-4205), immediate_output_frame / implicit_output_frame = 0 (mirror :4295-4313),
    // and frame_size_override_flag / order_hint are NOT read (the bridge skips the non-bridge
    // :4351+ block), so the bridge enters the inter control region directly.
    if core.is_bridge {
        let bridge_frame_ref_idx = read_f(reader, ceil_log2(seq.num_ref_frames))?;
        core.bridge_frame_ref_idx = Some(bridge_frame_ref_idx);
        core.frame_type = Some(FrameType::Inter);
        core.frame_is_intra = Some(false);
        core.immediate_output_frame = Some(false);
        core.implicit_output_frame = Some(false);
        return parse_bridge_inter_path(reader, core, seq, bridge_frame_ref_idx, reference_state);
    }

    if seq.single_picture_header_flag {
        // AV2 § 5.18.2: single_picture_header_flag forces a key frame and skips the
        // entire show-existing/frame-type/output-control block.
        core.show_existing_frame = Some(false);
        core.frame_type = Some(FrameType::Key);
        core.frame_is_intra = Some(true);
        core.immediate_output_frame = Some(true);
        core.implicit_output_frame = Some(false);
        return parse_intra_tail(reader, core, seq, mfh, FrameType::Key, true);
    }

    // AV2 § 5.18.2: ShowExistingFrame = is_sef().
    let show_existing_frame = obu_type.is_sef();
    core.show_existing_frame = Some(show_existing_frame);
    if show_existing_frame {
        return parse_show_existing_frame(reader, core, seq);
    }

    // AV2 § 5.18.2: frame-type determination (the non-SEF, non-bridge branch).
    let frame_type = if obu_type == ObuType::Switch || obu_type == ObuType::RasFrame {
        // restricted_prediction_switch f(1): a real bit. It affects only reference-state
        // derivations (OrderHint / RefOrderHint, mirror :4259-4277) the inter region does
        // not compute here, so its value does not change any modeled bit position.
        reader.read_bit()?;
        FrameType::Switch
    } else if obu_type.is_tip_frame() {
        FrameType::Inter
    } else if obu_type == ObuType::ClosedLoopKey || obu_type == ObuType::OpenLoopKey {
        FrameType::Key
    } else {
        let frame_is_inter = reader.read_bit()? != 0; // frame_is_inter f(1)
        if frame_is_inter {
            FrameType::Inter
        } else {
            FrameType::IntraOnly
        }
    };
    let frame_is_intra = matches!(frame_type, FrameType::Key | FrameType::IntraOnly);
    core.frame_type = Some(frame_type);
    core.frame_is_intra = Some(frame_is_intra);

    // AV2 § 5.18.2: long_term_id_plus_1 (KEY frames) and num_key_ref_frames +
    // ref_long_term_id[i] (RAS / OLK frames) are read after the frame-type field and
    // before the FrameIsIntra split. Both are fully determined by sequence state, so
    // they are read even on the non-intra paths the parser then stops on.
    if frame_type == FrameType::Key {
        read_f(reader, seq.long_term_frame_id_bits)?; // long_term_id_plus_1
    }
    if (obu_type == ObuType::RasFrame || obu_type == ObuType::OpenLoopKey)
        && seq.long_term_frame_id_bits != 0
    {
        // AV2 § 6.17.2: every ref_long_term_id[i] must differ from the reserved
        // (1 << long_term_frame_id_bits) - 1; record a violation for the validator.
        let reserved_long_term_id = (1u32 << seq.long_term_frame_id_bits).wrapping_sub(1);
        let num_key_ref_frames = reader.read_bits(3)?;
        for _ in 0..num_key_ref_frames {
            let ref_long_term_id = read_f(reader, seq.long_term_frame_id_bits)?;
            if ref_long_term_id == reserved_long_term_id {
                core.forbidden_ref_long_term_id = true;
            }
        }
    }

    // AV2 § 5.18.2 output control (mirror :4295-4313). This block is in the non-SEF,
    // non-single-picture branch and applies to BOTH intra and inter frames. A bridge
    // frame already returned above; here `obu_type` is never OBU_BRIDGE_FRAME, so the
    // gate reduces to the OLK / monotonic-output checks.
    let immediate_output_frame = if obu_type == ObuType::OpenLoopKey {
        false
    } else {
        reader.read_bit()? != 0
    };
    core.immediate_output_frame = Some(immediate_output_frame);
    let implicit_output_frame = if immediate_output_frame || seq.monotonic_output_order_flag {
        false
    } else {
        reader.read_bit()? != 0
    };
    core.implicit_output_frame = Some(implicit_output_frame);

    if frame_is_intra {
        return parse_intra_tail(reader, core, seq, mfh, frame_type, false);
    }

    // AV2 § 5.18.2 (mirror :4351-4403): the non-bridge inter / switch / TIP path reads
    // frame_size_override_flag (when not SWITCH / single-picture), order_hint, then the
    // reference control region. order_hint is bit-direct; OrderHint = get_disp_order_hint()
    // is a reference-state derivation that affects no bit position here, so it is not
    // computed.
    parse_inter_path(reader, core, seq, frame_type, reference_state)
}

/// Parses the non-intra `frame_header_info()` path (AV2 § 5.18.2, mirror :4351-5181):
/// `frame_size_override_flag`, `order_hint`, then the reference control region via
/// [`parse_inter_control`](crate::headers::frame::inter::parse_inter_control), converging
/// into the shared tail (`tile_info()` onward) where the parse reached it.
///
/// Whatever [`InterStop`](crate::headers::frame::inter::InterStop) the control region
/// reaches — including [`InterStop::ReachedSharedTail`](crate::headers::frame::inter::InterStop)
/// — the inter facts are recorded on `core.inter` and the parse stops here with the
/// unsupported-coverage [`FrameHeaderParseStatus`]; the distinct stop class is preserved on
/// `core.inter.stop`. Continuing into the shared structure cluster (the same
/// `tile_info()` → quant → segmentation → … path the intra tail uses, with inter inputs)
/// is the next phase.
fn parse_inter_path(
    reader: &mut BitReader<'_>,
    core: &mut FrameHeaderCore,
    seq: &CoreSeqView,
    frame_type: FrameType,
    reference_state: &FrameReferenceStateView<'_>,
) -> Result<()> {
    use crate::headers::frame::inter::{InterFrameContext, parse_inter_control};

    let obu_type = core.obu_type;

    // mirror :4353-4365: frame_size_override_flag. SWITCH_FRAME forces 1 (no bit);
    // single_picture_header_flag forces 0; otherwise f(1). The inter path is never a
    // single-picture frame (that path is intra-only above), so the gate is SWITCH vs read.
    let frame_size_override_flag = if frame_type == FrameType::Switch {
        true
    } else {
        reader.read_bit()? != 0
    };
    core.frame_size_override_flag = Some(frame_size_override_flag);

    // mirror :4367: order_hint f(OrderHintBits); OrderHintLsbs = order_hint.
    core.order_hint_lsb = Some(read_f(reader, seq.order_hint_bits)?);

    let inter_seq = build_inter_seq_view(seq);
    let ctx = InterFrameContext {
        obu_type,
        frame_type,
        is_bridge: false, // bridge frames take parse_bridge_inter_path, not this path
        bridge_frame_ref_idx: None,
        cur_mfh_id_is_zero: core.cur_mfh_id.is_zero(),
    };

    let control = parse_inter_control(
        reader,
        &inter_seq,
        &ctx,
        reference_state,
        frame_size_override_flag,
    )?;

    // Lift the inter reference-grounded frame size / refresh flags onto the core so
    // existing state-supported diagnostics and the inspector see them.
    if let Some(size) = control.frame_size {
        core.frame_size = Some(size);
    }
    if let Some(flags) = control.refresh_frame_flags {
        core.refresh_frame_flags = Some(flags);
    }

    // Whether the control region converged into the shared tail (tile_info() onward, see
    // InterStop::reaches_shared_tail) or stopped at a coverage / poisoned-state branch, the
    // parser stops honestly here: the shared structure cluster needs inter
    // primary-reference / TIP inputs the shared path does not yet accept, and the coverage /
    // poisoned-state stops are unmodeled by construction. In both cases the control-region
    // facts are preserved on `core.inter`. The distinct stop class lives in `control.stop`;
    // the core status is the unsupported-coverage class (never a truncation).
    core.inter = Some(control);
    core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
        feature_id: FRAME_HEADER_INFO_FEATURE,
    };
    Ok(())
}

/// Builds the [`InterSeqView`](crate::headers::frame::inter::InterSeqView) the inter
/// control region consumes from the parsed sequence configuration. Shared by the
/// non-bridge inter path and the bridge path.
fn build_inter_seq_view(seq: &CoreSeqView) -> crate::headers::frame::inter::InterSeqView {
    use crate::headers::frame::inter::InterSeqView;
    InterSeqView {
        num_ref_frames: seq.num_ref_frames,
        enable_short_refresh_frame_flags: seq.enable_short_refresh_frame_flags,
        explicit_ref_frame_map: seq.inter.explicit_ref_frame_map,
        enable_ref_frame_mvs: seq.inter.enable_ref_frame_mvs,
        enable_bru: seq.inter.enable_bru,
        enable_tip: seq.inter.enable_tip,
        seq_max_drl_bits_minus_1: seq.inter.seq_max_drl_bits_minus_1,
        allow_frame_max_drl_bits: seq.inter.allow_frame_max_drl_bits,
        enable_flex_mvres: seq.inter.enable_flex_mvres,
        seq_frame_motion_modes_present_flag: seq.inter.seq_frame_motion_modes_present_flag,
        seq_enabled_motion_modes: seq.inter.seq_enabled_motion_modes,
        enable_opfl_refine: seq.inter.enable_opfl_refine,
        max_mlayer_id: seq.max_mlayer_id,
        seq_force_screen_content_tools: seq.seq_force_screen_content_tools,
        seq_force_integer_mv: seq.seq_force_integer_mv,
        allow_frame_max_bvp_drl_bits: seq.allow_frame_max_bvp_drl_bits,
        frame_width_bits: seq.frame_width_bits,
        frame_height_bits: seq.frame_height_bits,
        max_frame_width: seq.max_frame_width,
        max_frame_height: seq.max_frame_height,
        sb_size: seq.tile.frame_sb_size(false),
    }
}

/// Parses a bridge frame's `frame_header_info()` reference-control region (AV2 § 5.18.2,
/// `IsBridge` arm). The reader is positioned just after `bridge_frame_ref_idx` (mirror
/// :4121); the bridge skips the non-bridge `frame_size_override_flag` / `order_hint` reads
/// (mirror :4353-4367) and enters the control region directly via
/// [`parse_inter_control`](crate::headers::frame::inter::parse_inter_control) with
/// `is_bridge == true`.
///
/// The bridge takes the `IsBridge` reference-control arms verbatim:
/// `primary_ref_frame = PRIMARY_REF_NONE` (mirror :4345, no bits),
/// `bridge_frame_overwrite_flag` f(1) (mirror :4425), the bridge `refresh_frame_flags`
/// arms (mirror :4489/:4533), `NumTotalRefs = 1` (mirror :4597),
/// `ref_frame_idx[0] = bridge_frame_ref_idx` (mirror :4615, no bits), and
/// `frame_size_with_bridge()` (mirror :4633, § 5.18.4.2). It then hits the
/// `IsBridge` early-return arm (mirror :4971/:5045), stopping with
/// [`InterStop::BruInactiveOrBridgeReturn`](crate::headers::frame::inter::InterStop::BruInactiveOrBridgeReturn)
/// — the bridge tail (`film_grain_config()` / `tile_info()`) needs reference-frame dims /
/// `MiRows`/`MiCols` this phase does not thread, an honest coverage stop. The parsed bridge
/// facts are preserved on `core.inter`.
fn parse_bridge_inter_path(
    reader: &mut BitReader<'_>,
    core: &mut FrameHeaderCore,
    seq: &CoreSeqView,
    bridge_frame_ref_idx: u32,
    reference_state: &FrameReferenceStateView<'_>,
) -> Result<()> {
    use crate::headers::frame::inter::{InterFrameContext, parse_inter_control};

    let inter_seq = build_inter_seq_view(seq);
    let ctx = InterFrameContext {
        obu_type: core.obu_type,
        frame_type: FrameType::Inter,
        is_bridge: true,
        bridge_frame_ref_idx: Some(bridge_frame_ref_idx),
        cur_mfh_id_is_zero: core.cur_mfh_id.is_zero(),
    };

    // frame_size_override_flag is never read on the bridge path; frame_size_with_bridge()
    // is selected unconditionally by the IsBridge arm (mirror :4627), so the flag is inert
    // for the bridge and passed as false.
    let control = parse_inter_control(reader, &inter_seq, &ctx, reference_state, false)?;

    // Lift the bridge reference-grounded frame size / refresh flags onto the core so the
    // state-supported diagnostics and the inspector see them.
    if let Some(size) = control.frame_size {
        core.frame_size = Some(size);
    }
    if let Some(flags) = control.refresh_frame_flags {
        core.refresh_frame_flags = Some(flags);
    }

    core.inter = Some(control);
    core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
        feature_id: FRAME_HEADER_INFO_FEATURE,
    };
    Ok(())
}

/// Parses the show-existing-frame sub-path (AV2 § 5.18.2), stopping before
/// `film_grain_config()`.
fn parse_show_existing_frame(
    reader: &mut BitReader<'_>,
    core: &mut FrameHeaderCore,
    seq: &CoreSeqView,
) -> Result<()> {
    core.frame_to_show_map_idx = Some(read_f(reader, ceil_log2(seq.num_ref_frames))?);
    let derive_sef_order_hint = reader.read_bit()? != 0;
    if !derive_sef_order_hint {
        core.order_hint_lsb = Some(read_f(reader, seq.order_hint_bits)?);
    }
    // AV2 § 5.18.2 (mirror :4180-4184): refresh_frame_flags = 0; immediate_output_frame = 1.
    // FrameType comes from the referenced slot (reference state), so it is left unknown.
    core.refresh_frame_flags = Some(0);
    core.immediate_output_frame = Some(true);

    // AV2 § 5.18.2 (mirror :4186): the SEF path calls film_grain_config() (§ 5.18.10.1),
    // then return()s (mirror :4196) — the frame header is complete. SEF only occurs when
    // single_picture_header_flag == 0 (the else arm of mirror :4131), so the
    // film_grain_config() single-picture inference is dead; with immediate_output_frame = 1
    // the (!immediate && !implicit) output gate is false, so apply_grain is f(1) when grain
    // is present. The save_grain_params() call at mirror :4190 reads no bits.
    //
    // film_grain_config() consumes film_grain_params_present (§ 5.4.1, the apply_grain
    // gate). If the active sequence header was a bounded stop that never read that flag, it
    // is genuinely unknown — the SEF facts above (frame_to_show_map_idx, order hint, output
    // flags) are preserved, but the parser cannot decide apply_grain without guessing, so it
    // stops honestly here rather than inventing the flag.
    let Some(film_grain_params_present) = seq.film_grain_params_present else {
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
    };
    let input = FrameTailInput {
        // SEF reads no read_tx_mode(): coded_lossless is irrelevant here but supplied for
        // the shared input shape (film_grain_config does not consult it).
        coded_lossless: false,
        film_grain_params_present,
        // SEF never runs under single_picture_header_flag (see above).
        single_picture_header_flag: false,
        immediate_output_frame: true,
        implicit_output_frame: false,
    };
    match parse_film_grain_config(reader, &input) {
        Ok(film_grain) => {
            core.sef_film_grain = Some(film_grain);
            // AV2 § 5.2.1 (:124-152) / § 5.2.3: a SEF OBU is not an is_tile_group() type,
            // so usedArith == 0 and the rest of the payload is exactly
            // trailing_bits( remainingPayloadBits ) — the SEF arm of § 5.18.2 (mirror :4145)
            // return()s right after film_grain_config() (:4186), and there is no tile data.
            // Classify that boundary so the validator can surface a non-conformant tail
            // (including the grain_seed-eats-the-marker case) as a § 6.2.1 / § 5.2.3
            // diagnostic, without failing the parse — the parsed SEF facts survive.
            core.sef_trailing_bits = Some(classify_sef_trailing_bits(reader));
            core.status = FrameHeaderParseStatus::ShowExistingFrameComplete;
            Ok(())
        }
        // A payload EOF inside the SEF film_grain_config() keeps the already-parsed SEF
        // facts (frame_to_show_map_idx, order hint, output flags) and reports the
        // truncation through the status rather than failing the whole parse. The SEF tail
        // IS film_grain_config() (a fully-modeled region), so this is a decidable
        // truncation (StoppedInsideShowExistingFrame), distinct from the ordinary bounded
        // CoreFieldsOnly stop — the validator surfaces it as a truncated-frame-header error.
        Err(Error::UnexpectedEof { .. }) => {
            core.status = FrameHeaderParseStatus::StoppedInsideShowExistingFrame;
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// Parses the intra-frame tail (AV2 § 5.18.2): `frame_size_override_flag`,
/// `order_hint`, `refresh_frame_flags`, then `frame_size()` /
/// `screen_content_params()` / `intrabc_params()`, `disable_cdf_update`, and the
/// § 5.18.2 structure cluster `tile_info()` → `quantization_params()` →
/// `segmentation_params()` → `setup_qm_params()` → `delta_q_params()` → the
/// per-segment lossless/QM derivation → `allow_tcq` / `allow_parity_hiding`,
/// stopping before `deblocking_filter_params()` (§ 5.18.5.2).
///
/// `single_picture` is `single_picture_header_flag` (forces `frame_size_override_flag
/// = 0`). For an intra frame `primary_ref_frame == PRIMARY_REF_NONE`, so no
/// primary-reference bits are read.
fn parse_intra_tail(
    reader: &mut BitReader<'_>,
    core: &mut FrameHeaderCore,
    seq: &CoreSeqView,
    mfh: Option<&MfhFrameView>,
    frame_type: FrameType,
    single_picture: bool,
) -> Result<()> {
    // frame_size_override_flag: 0 for a single-picture key frame, else f(1) (a key
    // frame is never SWITCH_FRAME, which would force it to 1).
    let frame_size_override_flag = if single_picture {
        false
    } else {
        reader.read_bit()? != 0
    };
    // Record the dims provenance for the §6.17.4.1 / §6.17.2 validator split: the
    // non-override path (`false`) derives FrameWidth/FrameHeight from the MFH default
    // dimensions (or the sequence maxima for cur_mfh_id == 0), the override path
    // (`true`) from this frame's explicit frame_width/height_minus_1 fields below.
    core.frame_size_override_flag = Some(frame_size_override_flag);

    // order_hint f(OrderHintBits); OrderHintLsbs = order_hint.
    core.order_hint_lsb = Some(read_f(reader, seq.order_hint_bits)?);
    // FrameIsIntra -> primary_ref_frame = PRIMARY_REF_NONE (no bits read).

    // refresh_frame_flags (AV2 § 5.18.2). For an intra frame this is the KEY_FRAME or
    // INTRA_ONLY_FRAME path; both are fully determined by sequence state.
    core.refresh_frame_flags = Some(read_refresh_frame_flags(
        reader,
        seq,
        core.obu_type,
        frame_type,
    )?);

    // FrameIsIntra branch: frame_size(); screen_content_params(); intrabc_params().
    // AV2 § 5.18.4.1 non-override default dimensions:
    //   - cur_mfh_id == 0: max_frame_width/height (the § 5.18.2 :4101 inference for the
    //     direct sequence reference).
    //   - cur_mfh_id > 0: mfh_frame_width/height_minus_1[ cur_mfh_id ] + 1, with the
    //     same omitted-size inference already folded into MfhFrameView::default_dims
    //     (:4101). `None` only when the in-band MFH was unresolvable, which keeps the
    //     parse from inventing a size (the structure cluster then stops).
    let default_dims = if core.cur_mfh_id.is_zero() {
        Some((seq.max_frame_width, seq.max_frame_height))
    } else {
        mfh.map(|view| view.default_dims)
    };
    core.frame_size = parse_frame_size(
        reader,
        frame_size_override_flag,
        seq.frame_width_bits,
        seq.frame_height_bits,
        default_dims,
    )?;
    core.allow_screen_content_tools = Some(parse_screen_content_params(
        reader,
        seq.seq_force_screen_content_tools,
        seq.seq_force_integer_mv,
    )?);
    core.allow_intrabc = Some(parse_intrabc_params(
        reader,
        true,
        seq.allow_frame_max_bvp_drl_bits,
    )?);

    // Not a TIP-as-output / bru-inactive / bridge frame -> disable_cdf_update f(1)
    // (AV2 § 5.18.2 else-branch of `if ( bru_inactive || IsBridge )`).
    reader.read_bit()?; // disable_cdf_update

    // AV2 § 5.18.2: on the intra path `bru_inactive == 0` and `!IsBridge` (handled
    // above), `use_ref_frame_mvs == 0` and `TipFrameMode == TIP_FRAME_DISABLED`
    // (FrameIsIntra branch), so none of the bru / motion-field / TIP blocks between
    // `disable_cdf_update` and `tile_info()` read bits or return, and parsing
    // continues directly at the structure cluster.
    parse_intra_structures(reader, core, seq, mfh)
}

/// Parses the § 5.18.2 intra-path structure cluster after `disable_cdf_update`:
/// `tile_info()` (§ 5.18.7.2), `quantization_params()` (§ 5.18.6.1),
/// `set_primary_ref_frame_and_ctx( 1 )` (no bits), `segmentation_params()`
/// (§ 5.18.7.1), `setup_qm_params()` (§ 5.18.6.2), `delta_q_params()` (§ 5.18.7.8),
/// the per-segment lossless/QM derivation, `allow_tcq` / `allow_parity_hiding`, and the
/// loop-filter cluster `deblocking_filter_params()` (§ 5.18.5.2), `gdf_params()`
/// (§ 5.18.7.9), and `cdef_params()` (§ 5.18.7.10), in exactly that order
/// (AV2 v1.0.0 § 5.18.2, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`). The
/// parser then stops before `lr_params()` (§ 5.18.7.11).
///
/// The intra path always has `TipFrameMode == TIP_FRAME_DISABLED` and `!IsBridge`.
/// On the `cur_mfh_id > 0` path the resolved multi-frame-header state is supplied via
/// `mfh` (the § 5.18.4.1 default dimensions, the § 5.18.7.1 MFH segmentation arm, and
/// the § 5.18.5.2 MFH deblocking arm); a `cur_mfh_id > 0` frame whose in-band MFH is
/// unresolvable never reaches here with a known `frame_size`, so it still stops with
/// [`FrameHeaderParseStatus::UnsupportedUntilFeature`] rather than guessing.
fn parse_intra_structures(
    reader: &mut BitReader<'_>,
    core: &mut FrameHeaderCore,
    seq: &CoreSeqView,
    mfh: Option<&MfhFrameView>,
) -> Result<()> {
    // AV2 § 5.18.7.2: tile_info() derives sbCols/sbRows from MiCols/MiRows, i.e. from
    // the exact FrameWidth/FrameHeight (§ 5.18.4.4). On the cur_mfh_id > 0 path those
    // come from MfhFrameView::default_dims; `frame_size` is `None` only when the in-band
    // MFH was unresolvable (no record), which keeps the unsupported/Unknown routing.
    let Some(frame_size) = core.frame_size else {
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
    };

    // AV2 § 5.18.2: tile_info() (§ 5.18.7.2). FrameIsIntra here, and the intra path
    // has IsBridge == 0 and TipFrameMode == TIP_FRAME_DISABLED.
    core.tile_info = match parse_tile_info(reader, &seq.tile, frame_size, true, false, false) {
        Ok(tile_info) => Some(tile_info),
        // The tile layout depends on unmodeled sequence state (reserved
        // seq_level_idx, or the unrecorded non-uniform sequence start arrays); stop
        // with the blocking Feature ID rather than guessing bit positions.
        Err(Error::Unimplemented { feature }) => {
            core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
                feature_id: feature,
            };
            return Ok(());
        }
        Err(error) => return Err(error),
    };

    // AV2 § 5.18.2: quantization_params() (§ 5.18.6.1); TIP_FRAME_AS_OUTPUT is
    // impossible on the intra path.
    let quantization = parse_quantization_params(reader, &seq.quant, false)?;
    core.quantization_params = Some(quantization);

    // AV2 § 5.18.2: set_primary_ref_frame_and_ctx( 1 ) reads no bits.

    // AV2 § 5.18.7.1: segmentation_params() consults mfh_seg_info_present_flag /
    // mfh_ext_seg_flag / mfh_allow_seg_info_change when cur_mfh_id > 0. On the
    // cur_mfh_id > 0 path the resolved MFH state must be known to derive the
    // haveSegParams / allowChange / mfhId arm; if the in-band MFH is unresolvable
    // (`mfh` is None) the derivation is undecidable, so stop here rather than guess.
    // (Reachable only when frame_size_override_flag == 1 supplied an explicit size;
    // otherwise the frame_size guard above already stopped.)
    if !core.cur_mfh_id.is_zero() && mfh.is_none() {
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
    }
    // The resolved MFH segmentation arm (`MfhSegView`) is passed through; it is `Some`
    // only when cur_mfh_id > 0 with mfh_seg_info_present_flag set, otherwise the
    // sequence/zero derivation applies (cur_mfh_id == 0, or the MFH did not signal
    // segment info).
    let mfh_seg = mfh.and_then(|view| view.seg.as_ref());
    let segmentation = parse_segmentation_params(reader, &seq.seg, mfh_seg)?;

    // AV2 § 5.18.2: setup_qm_params() (§ 5.18.6.2) runs after segmentation_params()
    // and is gated on the frame's parsed segmentation_enabled.
    let qm = parse_setup_qm_params(reader, &seq.quant, segmentation.segmentation_enabled)?;

    // AV2 § 5.18.2: delta_q_params() (§ 5.18.7.8), gated on base_q_idx.
    let delta_q = parse_delta_q_params(reader, quantization.base_q_idx)?;

    // AV2 § 5.18.2: init_coeff_cdfs() / load_previous_segment_ids() read no bits
    // (and the intra path has DerivedPrimaryRefFrame == PRIMARY_REF_NONE).

    // AV2 § 5.18.2: the per-segment lossless/QM derivation loop (qm_index reads),
    // then allow_tcq and allow_parity_hiding.
    core.lossless_info = Some(parse_lossless_info(
        reader,
        &seq.quant,
        &quantization,
        &qm,
        &delta_q,
        &segmentation,
        seq.seg.max_segments,
    )?);
    let coded_lossless = core
        .lossless_info
        .as_ref()
        .is_some_and(|info| info.coded_lossless);
    // These were parsed earlier but are stored only after `parse_lossless_info`
    // releases its borrows; on error the core is never returned, so the deferred
    // assignment is unobservable.
    core.segmentation_params = Some(segmentation);
    core.setup_qm_params = Some(qm);
    core.delta_q_params = Some(delta_q);

    // AV2 § 5.18.2 tail (mirror :5297-5307): the loop-filter cluster
    // deblocking_filter_params() / gdf_params() / cdef_params(), then lr_params()
    // (§ 5.18.7.11) and ccso_params() (§ 5.18.7.12). A truncation INSIDE the cluster must
    // not discard the control-region facts already parsed above (frame size, output flags,
    // tile/quant/segmentation): the validator/inspect call sites .ok() the result, so an Err
    // would silently drop every earlier state-supported diagnostic. parse_filter_cluster()
    // therefore converts a payload-EOF into the StoppedInsideFilterParams status (facts
    // preserved, unreached cluster fields left None) and only propagates a genuine structural
    // error.
    match parse_filter_cluster(reader, core, seq, mfh, coded_lossless) {
        // parse_filter_cluster sets the terminal status itself (the cluster-complete stop,
        // or the unreachable missing-tile_info guard), so the Ok arm leaves it untouched.
        Ok(()) => Ok(()),
        // The payload ran out mid-cluster: keep the preserved control-region facts and
        // record the truncation through the status rather than failing the whole parse.
        Err(Error::UnexpectedEof { .. }) => {
            core.status = FrameHeaderParseStatus::StoppedInsideFilterParams;
            Ok(())
        }
        // A structural error (e.g. an impossible read width) is a real parse failure.
        Err(error) => Err(error),
    }
}

/// Parses the § 5.18.2 tail loop-filter cluster on the intra path:
/// `deblocking_filter_params()` (§ 5.18.5.2), `gdf_params()` (§ 5.18.7.9),
/// `cdef_params()` (§ 5.18.7.10), `lr_params()` (§ 5.18.7.11), and `ccso_params()`
/// (§ 5.18.7.12), in that order (AV2 v1.0.0 § 5.18.2, mirror :5297-5307). All are
/// determined by the parsed sequence filter config (§ 5.4.10), the frame state
/// (`CodedLossless`, `NumPlanes`, `SbSize`, chroma subsampling, `base_q_idx`), the parsed
/// `tile_info()` geometry, and — on the `cur_mfh_id > 0` path — the resolved MFH's
/// deblocking-update state.
///
/// On a clean parse the `core` filter / lr / ccso fields are populated, the § 5.18.2
/// intra tail is parsed (see [`parse_intra_tail_structures`]), and the terminal
/// [`FrameHeaderParseStatus::IntraHeaderComplete`] is set (or
/// [`FrameHeaderParseStatus::StoppedInsideIntraTail`] when the payload runs out mid-tail).
/// When a plane signals a frame-level Wiener filter, `lr_params()` reaches the unmodeled
/// `read_wienerns_filter()` decode: this function sets
/// [`FrameHeaderParseStatus::StoppedBeforeWienerNsFilter`] and returns `Ok(())` (the
/// control-region facts are preserved and the partial `lr_params()` prefix is stored on
/// `core.lr_params_partial`, leaving `core.lr_params` `None`). On error the partially-read
/// fields stay `None`; the caller decides whether a payload EOF in the loop-filter cluster
/// (deblocking through ccso) is a truncation (`StoppedInsideFilterParams`) or a hard
/// failure (a tail EOF is handled here as `StoppedInsideIntraTail`).
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the payload
/// ends mid-cluster, or another typed error if a sub-parser rejects its inputs (for
/// example an out-of-range `df_par_bits_minus_2` read width).
fn parse_filter_cluster(
    reader: &mut BitReader<'_>,
    core: &mut FrameHeaderCore,
    seq: &CoreSeqView,
    mfh: Option<&MfhFrameView>,
    coded_lossless: bool,
) -> Result<()> {
    // AV2 § 5.18.5.2: the cur_mfh_id > 0 arm copies apply_deblocking_filter from the
    // resolved MFH; on the cur_mfh_id == 0 direct path no MFH view is supplied.
    let mfh_deblocking = mfh.map(|view| &view.deblocking);
    core.deblocking_filter_params = Some(parse_deblocking_filter_params(
        reader,
        coded_lossless,
        seq.quant.num_planes,
        seq.filter.df_par_bits_minus_2,
        mfh_deblocking,
    )?);

    // AV2 § 5.18.7.9: gdf_params() needs the frame SbSize and the parsed tile_info()
    // geometry (MiCols/MiRows via the start-array sentinels, TileCols/TileRows, and the
    // per-tile MiColStarts/MiRowStarts for the SB-64x64 alignment scan). The intra path
    // SbSize is frame_sb_size(frame_is_intra == true). The geometry borrow of
    // `core.tile_info` is scoped so the later `core.gdf_params` write is unambiguous.
    let gdf = {
        // `tile_info` was set to `Some` earlier in parse_intra_structures (every other
        // path returns before the cluster), so this binding never falls through; the
        // explicit guard keeps the parser panic-free even under direct API misuse rather
        // than unwrapping. The borrow is scoped to this block so the later
        // `core.gdf_params` write is unambiguous.
        let Some(tile_info) = core.tile_info.as_ref() else {
            core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
                feature_id: FRAME_HEADER_INFO_FEATURE,
            };
            return Ok(());
        };
        let geometry = GdfGeometry {
            sb_size: seq.tile.frame_sb_size(true),
            // MiColStarts[TileCols] / MiRowStarts[TileRows] are the MiCols / MiRows
            // sentinels appended by parse_tile_info(); fall back to 0 when absent.
            mi_cols: tile_info.mi_col_starts.last().copied().unwrap_or(0),
            mi_rows: tile_info.mi_row_starts.last().copied().unwrap_or(0),
            tile_cols: tile_info.tile_cols,
            tile_rows: tile_info.tile_rows,
            mi_col_starts: &tile_info.mi_col_starts,
            mi_row_starts: &tile_info.mi_row_starts,
        };
        parse_gdf_params(reader, coded_lossless, &seq.filter, geometry)?
    };
    core.gdf_params = Some(gdf);

    // AV2 § 5.18.7.10: cdef_params().
    core.cdef_params = Some(parse_cdef_params(
        reader,
        coded_lossless,
        seq.quant.num_planes,
        &seq.filter,
    )?);

    // AV2 § 5.18.7.11: lr_params() (loop restoration). On the intra path FrameIsIntra, so
    // numRefFrames == 0 and the temporal-prediction arm is dead; the SbSize and chroma
    // subsampling drive the size signaling. A plane signalling frame_filters_on enters the
    // unmodeled read_wienerns_filter() decode, which stops the parse honestly: the partial
    // lr facts parsed up to that loop are surfaced on core.lr_params_partial (not
    // core.lr_params, which means a complete parse) so consumers see the parsed prefix.
    let lr_geometry = LrGeometry::new(seq.tile.frame_sb_size(true), seq.chroma_format_idc);
    // base_q_idx feeds the spec's get_filter_set_index derivation only (SubclassLookup); it
    // signals no bits. It is `Some` here because quantization_params() always parses before
    // the cluster on the reached intra path.
    let base_q_idx = core
        .quantization_params
        .as_ref()
        .map_or(0, |quant| quant.base_q_idx);
    match parse_lr_params(
        reader,
        coded_lossless,
        seq.quant.num_planes,
        &seq.restoration,
        lr_geometry,
        base_q_idx,
    )? {
        LrParseOutcome::Parsed(lr) => {
            core.lr_params = Some(lr);
        }
        LrParseOutcome::StoppedBeforeWienerNsFilter {
            feature_id,
            partial,
        } => {
            // The frame-level Wiener bank decode is unmodeled; stop honestly. The
            // control-region facts and the cdef/lr-pre-Wiener reads above are preserved.
            // The partial lr_params facts parsed up to the loop (per-plane types,
            // frame_filters_on, NumFilterClasses, UsesLr, size flags) were really consumed,
            // so surface them on the dedicated partial field rather than discarding them.
            // `lr_params` stays None (no complete parse) so the two can never be confused.
            core.lr_params_partial = Some(partial);
            core.status = FrameHeaderParseStatus::StoppedBeforeWienerNsFilter { feature_id };
            return Ok(());
        }
    }

    // AV2 § 5.18.7.12: ccso_params(). The intra path's reuse arm (reuse_ccso / ccso_ref_idx)
    // is dead (FrameIsIntra), so it parses fully on plain (f/tu) reads.
    core.ccso_params = Some(parse_ccso_params(
        reader,
        coded_lossless,
        seq.quant.num_planes,
        &seq.ccso,
    )?);

    // AV2 § 5.18.2 tail (mirror :5307-5341): read_tx_mode() (§ 5.18.8.1), the no-bit
    // frame_reference_mode() / skip_mode_params() / allow_bawp / allow_warpmv_mode intra
    // inferences, reduced_tx_set, the no-bit intra arm of global_motion_params()
    // (§ 5.18.9.1), and film_grain_config() (§ 5.18.10.1). This completes the intra
    // frame header.
    parse_intra_tail_structures(reader, core, seq, coded_lossless)
}

/// Parses the § 5.18.2 intra tail after `ccso_params()` (AV2 v1.0.0 § 5.18.2, mirror
/// :5307-5341) and sets the terminal status. On a clean parse the tail is stored on
/// `core.intra_tail` and the status is [`FrameHeaderParseStatus::IntraHeaderComplete`].
/// A payload EOF inside the tail keeps every already-parsed fact (the tail field stays
/// `None`) and reports [`FrameHeaderParseStatus::StoppedInsideIntraTail`] rather than a
/// hard error, mirroring the loop-filter-cluster truncation handling so earlier
/// state-supported diagnostics still see the facts.
///
/// `coded_lossless` is the derived `CodedLossless` from `parse_lossless_info`, gating
/// `read_tx_mode()`. The output flags (`immediate_output_frame` / `implicit_output_frame`)
/// and `single_picture_header_flag` come from the already-set `core` / `seq` state.
///
/// # Errors
/// Returns a typed descriptor error if a sub-read rejects its inputs (not a payload EOF,
/// which is converted to the truncation status).
fn parse_intra_tail_structures(
    reader: &mut BitReader<'_>,
    core: &mut FrameHeaderCore,
    seq: &CoreSeqView,
    coded_lossless: bool,
) -> Result<()> {
    // film_grain_config() (the last § 5.18.2 tail structure) consumes
    // film_grain_params_present (§ 5.4.1, the apply_grain gate). If the active sequence
    // header was a bounded stop that never read that flag, it is genuinely unknown: the
    // control region and loop-filter cluster already parsed and their facts are preserved,
    // but the parser cannot decide apply_grain without guessing. Stop honestly before the
    // tail rather than inventing the flag — this is the deferred half of the § 5.4.1
    // film_grain_params_present requirement (the view no longer gates the whole parse on it).
    let Some(film_grain_params_present) = seq.film_grain_params_present else {
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
    };
    // The output flags are always Some by the time the intra tail is reached: the
    // single-picture path sets them in parse_core_body, and the non-single-picture intra
    // path sets them before parse_intra_tail. Defaulting to the spec's intra inference
    // (immediate_output_frame = whatever was parsed) keeps the parser panic-free under
    // direct API misuse without inventing bits.
    let input = FrameTailInput {
        coded_lossless,
        film_grain_params_present,
        single_picture_header_flag: seq.single_picture_header_flag,
        immediate_output_frame: core.immediate_output_frame.unwrap_or(false),
        implicit_output_frame: core.implicit_output_frame.unwrap_or(false),
    };
    match parse_intra_tail_grammar(reader, &input) {
        Ok(tail) => {
            core.intra_tail = Some(tail);
            core.status = FrameHeaderParseStatus::IntraHeaderComplete;
            Ok(())
        }
        // The payload ran out mid-tail: keep the preserved control-region / cluster facts
        // and record the truncation through the status rather than failing the whole parse.
        Err(Error::UnexpectedEof { .. }) => {
            core.status = FrameHeaderParseStatus::StoppedInsideIntraTail;
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// Reads `refresh_frame_flags` for an intra frame (AV2 § 5.18.2): the KEY_FRAME branch
/// for a key frame, or the INTRA_ONLY_FRAME branch otherwise.
fn read_refresh_frame_flags(
    reader: &mut BitReader<'_>,
    seq: &CoreSeqView,
    obu_type: ObuType,
    frame_type: FrameType,
) -> Result<u32> {
    if frame_type == FrameType::Key {
        if obu_type == ObuType::ClosedLoopKey && seq.max_mlayer_id == 0 {
            Ok(all_frames_mask(seq.num_ref_frames))
        } else if seq.enable_short_refresh_frame_flags {
            let frame_to_refresh = read_f(reader, ceil_log2(seq.num_ref_frames))?;
            Ok(1u32.wrapping_shl(frame_to_refresh))
        } else {
            read_f(reader, seq.num_ref_frames)
        }
    } else if seq.enable_short_refresh_frame_flags {
        // INTRA_ONLY_FRAME with the compact signaling mode.
        let has_refresh_frame_flags = reader.read_bit()? != 0;
        if has_refresh_frame_flags {
            let frame_to_refresh = read_f(reader, ceil_log2(seq.num_ref_frames))?;
            Ok(1u32.wrapping_shl(frame_to_refresh))
        } else {
            Ok(0)
        }
    } else {
        read_f(reader, seq.num_ref_frames)
    }
}

/// Arbitrary-but-fixed § 5.18.6 / § 5.18.7 sub-views for tests: an 8-bit, 4:2:0,
/// no-sequence-tile, no-sequence-segmentation stream with 128×128 superblocks and
/// every optional quantizer/segmentation/tile read disabled. With these views the
/// intra tail reads `uniform_tile_spacing_flag` (plus any increment bits),
/// `base_q_idx` `f(8)`, `segmentation_enabled`, `using_qmatrix`, and
/// `delta_q_present` (when `base_q_idx > 0`) after `disable_cdf_update`.
#[cfg(test)]
fn test_sub_views() -> (CoreSeqQuantView, CoreSeqSegView, CoreSeqTileView) {
    use crate::headers::sequence::{LevelIdx, SuperblockSize, Tier};
    (
        CoreSeqQuantView {
            bit_depth: 8,
            num_planes: 3,
            separate_uv_delta_q: false,
            equal_ac_dc_q: false,
            y_dc_delta_q_enabled: false,
            uv_dc_delta_q_enabled: false,
            uv_ac_delta_q_enabled: false,
            base_y_dc_delta_q: 0,
            base_uv_dc_delta_q: 0,
            base_uv_ac_delta_q: 0,
            enable_tcq: false,
            choose_tcq_per_frame: false,
            enable_parity_hiding: false,
        },
        CoreSeqSegView {
            seq_seg_info_present_flag: false,
            seq_allow_seg_info_change: false,
            enable_ext_seg: false,
            max_segments: 8,
            seq_segment_info: None,
        },
        CoreSeqTileView {
            seq_tile_info_present_flag: false,
            allow_tile_info_change: false,
            seq_tile_params: None,
            seq_sb_col_starts: Vec::new(),
            seq_sb_row_starts: Vec::new(),
            seq_sb_size: SuperblockSize::Block128x128,
            use_256x256_superblock: false,
            use_128x128_superblock: true,
            enable_avg_cdf: false,
            avg_cdf_type: 0,
            seq_tier: Tier::Main,
            seq_level_idx: LevelIdx::from_bits(0),
        },
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::headers::frame::filtering::InterpolationFilter;
    use crate::headers::frame::inter::{InterStop, MvPrecision, NUM_REF_FRAMES};
    use crate::headers::frame::restoration::FrameRestorationType;
    use crate::headers::frame::tail::TxMode;
    use crate::segment::{MAX_SEGMENTS, SEG_LVL_MAX, SegmentFeature, SegmentInfo};
    use crate::span::ByteOffset;

    #[derive(Default)]
    struct Bits {
        bits: Vec<u8>,
    }

    impl Bits {
        fn bit(&mut self, bit: u8) {
            self.bits.push(bit & 1);
        }

        fn f(&mut self, value: u32, width: u32) {
            for shift in (0..width).rev() {
                self.bit(((value >> shift) & 1) as u8);
            }
        }

        fn uvlc(&mut self, value: u32) {
            let code_num = value + 1;
            let leading_zeros = u32::BITS - 1 - code_num.leading_zeros();
            for _ in 0..leading_zeros {
                self.bit(0);
            }
            self.bit(1);
            if leading_zeros > 0 {
                self.f(code_num - (1 << leading_zeros), leading_zeros);
            }
        }

        /// `ns(n)` encoding of `value` (0..n-1), the inverse of [`BitReader::read_ns`].
        fn ns(&mut self, value: u32, n: u32) {
            let w = u32::BITS - n.leading_zeros();
            let m = (1u32 << w) - n;
            if value < m {
                self.f(value, w - 1);
            } else {
                self.f(value + m, w);
            }
        }

        /// Number of bits accumulated so far (for byte-exact test truncation).
        fn bit_len(&self) -> usize {
            self.bits.len()
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

    /// A representative non-single-picture sequence view: OrderHintBits = 4,
    /// NumRefFrames = 8, no long-term ids, full refresh signaling, screen-content
    /// forced off, 12-bit frame dimensions, 4096x2304 maximum.
    /// A test sequence filter view (§ 5.4.10) with CDEF and GDF disabled, so the
    /// § 5.18.2 tail loop-filter cluster reads only the `deblocking_filter_params()`
    /// `apply_deblocking_filter` bits (GDF / CDEF return without reading). Override the
    /// individual flags in a test that needs the enabled arms.
    fn base_filter() -> CoreSeqFilterView {
        CoreSeqFilterView {
            enable_cdef: false,
            enable_gdf: false,
            gdf_unit_matches_sb_size: false,
            disable_loopfilters_across_tiles: false,
            cdef_on_skip_txfm: crate::headers::sequence::CdefOnSkipTxfm::Adaptive,
            df_par_bits_minus_2: 0,
            single_picture_header_flag: false,
        }
    }

    /// A test restoration view (§ 5.4.10) with `enable_restoration == false`, so
    /// `lr_params()` returns without reading any bits. Override it in a test that needs the
    /// enabled per-plane arm.
    fn base_restoration() -> CoreSeqRestorationView {
        CoreSeqRestorationView {
            enable_restoration: false,
            lr_pc_wiener_disabled: false,
            lr_wiener_nonsep_disabled: false,
            lr_uv_pc_wiener_disabled: false,
            lr_uv_wiener_nonsep_disabled: false,
        }
    }

    /// A test CCSO view (§ 5.4.10) with `enable_ccso == false`, so `ccso_params()` returns
    /// without reading any bits. Override it in a test that needs the enabled arm.
    fn base_ccso() -> CoreSeqCcsoView {
        CoreSeqCcsoView {
            enable_ccso: false,
            single_picture_header_flag: false,
        }
    }

    /// A § 5.4.6 inter sub-view for tests with every inter tool disabled: the inter path
    /// reads the explicit reference map only when `explicit_ref_frame_map` is overridden.
    fn base_inter() -> CoreSeqInterView {
        CoreSeqInterView {
            enable_ref_frame_mvs: false,
            explicit_ref_frame_map: false,
            enable_bru: false,
            enable_tip: false,
            seq_max_drl_bits_minus_1: 0,
            allow_frame_max_drl_bits: false,
            enable_flex_mvres: false,
            seq_frame_motion_modes_present_flag: false,
            seq_enabled_motion_modes: [false; MOTION_MODES],
            enable_opfl_refine: 0,
        }
    }

    fn base_seq() -> CoreSeqView {
        let (quant, seg, tile) = test_sub_views();
        CoreSeqView {
            num_ref_frames: 8,
            order_hint_bits: 4,
            long_term_frame_id_bits: 0,
            enable_short_refresh_frame_flags: false,
            monotonic_output_order_flag: false,
            single_picture_header_flag: false,
            max_mlayer_id: 0,
            frame_width_bits: 12,
            frame_height_bits: 12,
            max_frame_width: 4096,
            max_frame_height: 2304,
            seq_force_screen_content_tools: 0,
            seq_force_integer_mv: 0,
            allow_frame_max_bvp_drl_bits: false,
            inter: base_inter(),
            quant,
            seg,
            tile,
            filter: base_filter(),
            restoration: base_restoration(),
            ccso: base_ccso(),
            chroma_format_idc: ChromaFormatIdc::Yuv420,
            film_grain_params_present: Some(false),
        }
    }

    /// Parses the activation prefix then the core body, returning the result and the
    /// total bits consumed (prefix + body). `cur_mfh_id == 0` paths pass no MFH state.
    fn parse_body(
        data: &[u8],
        obu_type: ObuType,
        first_picture_in_tu: bool,
        seq: &CoreSeqView,
    ) -> Result<(FrameHeaderCore, u64)> {
        parse_body_with_mfh(data, obu_type, first_picture_in_tu, seq, None)
    }

    /// Like [`parse_body`] but resolves a `cur_mfh_id > 0` reference against `mfh_view`
    /// (the in-band multi-frame-header state) when present.
    fn parse_body_with_mfh(
        data: &[u8],
        obu_type: ObuType,
        first_picture_in_tu: bool,
        seq: &CoreSeqView,
        mfh_view: Option<&MfhFrameView>,
    ) -> Result<(FrameHeaderCore, u64)> {
        parse_body_with_ref(
            data,
            obu_type,
            first_picture_in_tu,
            seq,
            mfh_view,
            &FrameReferenceStateView::unknown(),
        )
    }

    /// Like [`parse_body_with_mfh`] but threads a modeled reference state into the core
    /// body (the inter reference paths consume it).
    fn parse_body_with_ref(
        data: &[u8],
        obu_type: ObuType,
        first_picture_in_tu: bool,
        seq: &CoreSeqView,
        mfh_view: Option<&MfhFrameView>,
        reference_state: &FrameReferenceStateView<'_>,
    ) -> Result<(FrameHeaderCore, u64)> {
        let mut reader = BitReader::new(data, ByteOffset::new(0));
        let prefix = parse_frame_header_prefix(&mut reader, obu_type, first_picture_in_tu)?;
        let mut core = init_core_from_prefix(&prefix, obu_type);
        parse_core_body(&mut reader, &mut core, seq, mfh_view, reference_state)?;
        let consumed = reader.consumed_bits();
        Ok((core, consumed))
    }

    #[test]
    fn frame_header_core_reads_direct_sequence_reference() {
        // CLK, cur_mfh_id == 0, seq_header_id_in_frame_header == 1, full intra path.
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(1); // seq_header_id_in_frame_header
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag
        bits.f(5, 4); // order_hint
        // refresh_frame_flags: CLK + max_mlayer_id == 0 -> allFrames (no bits)
        bits.f(1920 - 1, 12); // frame_width_minus_1
        bits.f(1080 - 1, 12); // frame_height_minus_1
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        // tile_info() (§ 5.18.7.2): no sequence tile info -> tile_params(). 1920x1080
        // with 128x128 superblocks: sbCols = 15, sbRows = 9, single uniform tile.
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2 = 0
        bits.bit(0); // increment_tile_rows_log2 = 0
        // quantization_params() (§ 5.18.6.1): 8-bit -> base_q_idx f(8); all delta
        // reads disabled in the test view.
        bits.f(90, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled (§ 5.18.7.1)
        bits.bit(0); // using_qmatrix (§ 5.18.6.2)
        bits.bit(0); // delta_q_present (§ 5.18.7.8, base_q_idx > 0)
        // § 5.18.2 lossless tail: base_q_idx 90 -> CodedLossless = 0; allow_tcq is
        // inferred enable_tcq (0) and allow_parity_hiding is forced 0 (no bits).
        // deblocking_filter_params() (§ 5.18.5.2): not lossless -> apply[0]/[1] read.
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1] (both 0 -> no chroma pair, no delta-Q)
        // gdf_params() / cdef_params(): enable_gdf == enable_cdef == 0 -> no bits.
        // lr_params()/ccso_params(): restoration and CCSO disabled -> no bits.
        // § 5.18.2 tail: read_tx_mode() not lossless -> tx_mode_select f(1); reduced_tx_set
        // f(2); film_grain_config() grain absent -> apply_grain inferred 0, no bits.
        bits.bit(0); // tx_mode_select = 0 -> TX_MODE_LARGEST
        bits.f(0, 2); // reduced_tx_set = 0
        let data = bits.into_bytes();
        let (core, consumed) =
            parse_body(&data, ObuType::ClosedLoopKey, true, &base_seq()).unwrap();

        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        assert!(core.cur_mfh_id.is_zero());
        assert_eq!(core.seq_header_id_in_frame_header, Some(1));
        assert_eq!(core.frame_type, Some(FrameType::Key));
        assert_eq!(core.frame_is_intra, Some(true));
        assert_eq!(core.immediate_output_frame, Some(false));
        assert_eq!(core.implicit_output_frame, Some(false));
        assert_eq!(core.order_hint_lsb, Some(5));
        assert_eq!(core.refresh_frame_flags, Some((1 << 8) - 1));
        assert_eq!(core.frame_size, Some(FrameSize::new(1920, 1080)));
        assert_eq!(core.allow_screen_content_tools, Some(false));
        assert_eq!(core.allow_intrabc, Some(false));
        let tile_info = core.tile_info.as_ref().unwrap();
        assert_eq!(tile_info.tile_cols, 1);
        assert_eq!(tile_info.tile_rows, 1);
        assert_eq!(tile_info.context_update_tile_id, 0);
        assert_eq!(tile_info.tile_size_bytes, None);
        assert_eq!(core.quantization_params.unwrap().base_q_idx, 90);
        assert!(!core.segmentation_params.unwrap().segmentation_enabled);
        assert!(!core.setup_qm_params.unwrap().using_qmatrix);
        assert!(!core.delta_q_params.unwrap().delta_q_present);
        let lossless = core.lossless_info.unwrap();
        assert!(!lossless.coded_lossless);
        assert!(!lossless.has_lossless_segment);
        assert!(!lossless.allow_tcq);
        assert!(!lossless.allow_parity_hiding);
        let deblocking = core.deblocking_filter_params.unwrap();
        assert_eq!(deblocking.apply_deblocking_filter, [false; 4]);
        assert!(!core.gdf_params.unwrap().gdf_frame_enable);
        assert!(!core.cdef_params.unwrap().cdef_frame_enable);
        let tail = core.intra_tail.as_ref().unwrap();
        assert_eq!(tail.tx_mode, TxMode::Largest);
        assert_eq!(tail.reduced_tx_set, 0);
        assert!(!tail.film_grain.apply_grain);
        // uvlc(0)=1 + uvlc(1)=3 prefix bits, then 33 core bits (1+1+1+4 control/output,
        // 24 frame_size, 1 allow_intrabc, 1 disable_cdf_update), then 14 structure
        // bits (3 tile_info, 8 base_q_idx, 1 segmentation_enabled, 1 using_qmatrix,
        // 1 delta_q_present), then 2 deblocking apply bits (GDF/CDEF disabled -> 0 bits),
        // then 3 tail bits (tx_mode_select + reduced_tx_set; grain absent).
        assert_eq!(consumed, 4 + 33 + 14 + 2 + 3);
    }

    /// A fixed in-band multi-frame-header record resolving `cur_mfh_id` for the
    /// `cur_mfh_id > 0` core path. `mfh_frame_size` / `mfh_seg_info_present_flag`
    /// control which § 5.18.4.1 / § 5.18.7.1 arm is exercised.
    fn mfh_record(
        mfh_frame_size: Option<crate::hls::MfhFrameSize>,
        seg: Option<(bool, bool, SegmentInfo)>,
    ) -> MultiFrameHeaderRecord {
        let (present, ext, allow, info) = match seg {
            Some((ext, allow, info)) => (true, Some(ext), Some(allow), Some(info)),
            None => (false, None, None, None),
        };
        MultiFrameHeaderRecord {
            mfh_id: MfhId::from_raw(1),
            mfh_seq_header_id: SequenceHeaderId::try_new(0).unwrap(),
            mfh_tlayer_id: crate::types::TemporalLayerId::from_bits(0),
            mfh_mlayer_id: crate::types::EmbeddedLayerId::from_bits(0),
            mfh_frame_size,
            mfh_seg_info_present_flag: present,
            mfh_ext_seg_flag: ext,
            mfh_allow_seg_info_change: allow,
            mfh_segment_info: info,
            mfh_deblocking_filter_update: false,
            mfh_apply_deblocking_filter: [false; 4],
            offset: ByteOffset::new(0),
        }
    }

    /// Like [`mfh_record`] but sets the § 5.18.5.2 deblocking-update arm inputs.
    fn mfh_record_with_deblocking(
        mfh_frame_size: Option<crate::hls::MfhFrameSize>,
        update: bool,
        apply: [bool; 4],
    ) -> MultiFrameHeaderRecord {
        let mut record = mfh_record(mfh_frame_size, None);
        record.mfh_deblocking_filter_update = update;
        record.mfh_apply_deblocking_filter = apply;
        record
    }

    #[test]
    fn frame_header_core_mfh_deblocking_update_copies_apply_no_apply_bits() {
        // cur_mfh_id == 1, resolved MFH with mfh_deblocking_filter_update == 1 and
        // mfh_apply_deblocking_filter == [1, 0, 1, 1]: § 5.18.5.2 copies apply from the
        // MFH (no apply bits read), and NumPlanes == 3 with apply[0] set copies the
        // chroma pair. Only the per-i df_delta_q_present bits are read.
        let mfh_size = Some(crate::hls::MfhFrameSize {
            width_bits: 12,
            height_bits: 12,
            width_minus_1: 1920 - 1,
            height_minus_1: 1080 - 1,
        });
        let record = mfh_record_with_deblocking(mfh_size, true, [true, false, true, true]);
        let view = MfhFrameView::from_record(&record, &base_seq());

        let mut bits = Bits::default();
        bits.uvlc(1); // cur_mfh_id == 1
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(0); // frame_size_override_flag == 0 (MFH default dims, no bits)
        bits.f(7, 4); // order_hint
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag (single tile)
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(70, 8); // base_q_idx (non-lossless)
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        // deblocking_filter_params(): MFH arm -> apply copied [1,0,1,1], no apply bits.
        // df_delta_q_present read for i in {0, 2, 3} (apply set); i == 1 skipped.
        bits.bit(0); // df_delta_q_present[0]
        bits.bit(0); // df_delta_q_present[2]
        bits.bit(0); // df_delta_q_present[3]
        // lr_params()/ccso_params(): restoration and CCSO disabled -> no bits.
        // § 5.18.2 tail: tx_mode_select f(1) + reduced_tx_set f(2); grain absent.
        bits.bit(0); // tx_mode_select = 0
        bits.f(0, 2); // reduced_tx_set = 0
        let data = bits.into_bytes();
        let (core, _) = parse_body_with_mfh(
            &data,
            ObuType::ClosedLoopKey,
            true,
            &base_seq(),
            Some(&view),
        )
        .unwrap();

        let deblocking = core.deblocking_filter_params.unwrap();
        assert_eq!(
            deblocking.apply_deblocking_filter,
            [true, false, true, true],
            "the MFH update arm copies apply_deblocking_filter from the record"
        );
        assert_eq!(deblocking.df_delta_q, [0; 4]);
        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        assert!(core.intra_tail.is_some());
    }

    /// A `base_seq()` whose `order_hint_bits` is widened to 5 so the intra body built by
    /// [`intra_body_up_to_filter_cluster`] ends on a byte boundary, putting the start of
    /// the loop-filter cluster exactly at bit 48 (byte 6). This lets the truncation tests
    /// land an EOF at a precise byte without disturbing the preceding structures.
    fn byte_aligned_filter_seq() -> CoreSeqView {
        let mut seq = base_seq();
        seq.order_hint_bits = 5;
        seq
    }

    /// Builds an intra CLK frame-header body parsed cleanly through the § 5.18.2 structure
    /// cluster (frame_size 16x16, both output flags 0) up to and including
    /// `delta_q_present`, i.e. positioned exactly at the start of the loop-filter cluster.
    /// The caller appends the loop-filter bits and applies the truncation. Paired with
    /// [`byte_aligned_filter_seq`] (`order_hint_bits == 5`) the cluster starts at bit 48.
    fn intra_body_up_to_filter_cluster() -> Bits {
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.bit(0); // immediate_output_frame == 0
        bits.bit(0); // implicit_output_frame == 0
        bits.bit(1); // frame_size_override_flag
        bits.f(5, 5); // order_hint f(order_hint_bits == 5)
        bits.f(16 - 1, 12); // frame_width_minus_1 -> FrameWidth 16
        bits.f(16 - 1, 12); // frame_height_minus_1 -> FrameHeight 16
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        // tile_info() for a 16x16 frame with 128x128 superblocks is a single superblock
        // (MiCols == MiRows == 4, sbCols == sbRows == 1), so tile_params() reads only
        // uniform_tile_spacing_flag and skips the increment / context fields.
        bits.bit(1); // uniform_tile_spacing_flag (single tile)
        bits.f(90, 8); // base_q_idx (non-lossless -> deblocking reads apply bits)
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits
    }

    /// Asserts the control-region facts parsed before the loop-filter cluster survived a
    /// mid-cluster truncation: the parse returned Ok, the frame size and output flags are
    /// intact, and the status records the truncation.
    fn assert_truncated_filter_cluster_preserves_facts(core: &FrameHeaderCore) {
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::StoppedInsideFilterParams,
            "a mid-cluster truncation reports StoppedInsideFilterParams"
        );
        assert_eq!(
            core.frame_size,
            Some(FrameSize::new(16, 16)),
            "frame_size parsed before the cluster must survive the truncation"
        );
        assert_eq!(core.immediate_output_frame, Some(false));
        assert_eq!(core.implicit_output_frame, Some(false));
        assert!(
            core.quantization_params.is_some(),
            "quantization_params parsed before the cluster must survive"
        );
        assert!(
            core.tile_info.is_some(),
            "tile_info parsed before the cluster must survive"
        );
    }

    #[test]
    fn frame_header_core_eof_inside_deblocking_filter_params_preserves_facts() {
        // REGRESSION (codex F2): a payload truncated mid-deblocking_filter_params() must
        // NOT fail the whole core parse. Before the loop-filter cluster existed the parser
        // stopped here and returned Ok with the control-region facts; the validator/inspect
        // .ok() the result, so an Err would silently drop every earlier state-supported
        // diagnostic. The parser now keeps the facts and reports StoppedInsideFilterParams.
        //
        // byte_aligned_filter_seq() puts the loop-filter cluster on byte 6 (bit 48), so
        // deblocking's first read (apply_deblocking_filter[0]) sits in byte 6. Truncating
        // the payload to 6 bytes makes that read overrun, landing the EOF at the very start
        // of the cluster with deblocking_filter_params still None.
        let mut bits = intra_body_up_to_filter_cluster();
        let cluster_start = bits.bit_len();
        assert_eq!(
            cluster_start, 48,
            "with order_hint_bits == 5 the loop-filter cluster starts on byte 6"
        );
        // deblocking apply[0] is the cluster's first read (bit 48 = byte 6). Truncating to
        // 6 bytes makes that read overrun, landing the EOF at the very start of the cluster.
        bits.bit(0); // apply_deblocking_filter[0] (in the dropped byte 6)
        let mut data = bits.into_bytes();
        data.truncate(6); // 48 bits: the deblocking apply reads overrun
        let (core, _) = parse_body(
            &data,
            ObuType::ClosedLoopKey,
            true,
            &byte_aligned_filter_seq(),
        )
        .unwrap();
        assert_truncated_filter_cluster_preserves_facts(&core);
        assert_eq!(
            core.deblocking_filter_params, None,
            "the truncated deblocking structure leaves its field None"
        );
        assert_eq!(core.gdf_params, None);
        assert_eq!(core.cdef_params, None);
    }

    #[test]
    fn frame_header_core_eof_inside_gdf_params_preserves_facts() {
        // The payload parses cleanly through deblocking_filter_params() and into
        // gdf_params(), then runs out: deblocking is preserved, gdf/cdef stay None, and the
        // status is the truncation marker. deblocking is built to consume exactly the full
        // byte 6 (apply[0..4] = 1 + df_delta_q_present[0..4] = 0, 8 bits), so gdf begins at
        // the byte-7 boundary (bit 56) and truncating to 7 bytes drops the byte gdf needs.
        let mut seq = byte_aligned_filter_seq();
        seq.filter.enable_gdf = true; // gdf_params() reads bits instead of short-circuiting
        let mut bits = intra_body_up_to_filter_cluster();
        bits.bit(1); // apply_deblocking_filter[0]
        bits.bit(1); // apply_deblocking_filter[1]
        bits.bit(1); // apply_deblocking_filter[2] (NumPlanes 3, luma set)
        bits.bit(1); // apply_deblocking_filter[3]
        bits.bit(0); // df_delta_q_present[0]
        bits.bit(0); // df_delta_q_present[1]
        bits.bit(0); // df_delta_q_present[2]
        bits.bit(0); // df_delta_q_present[3] -> deblocking ends at bit 56 (byte boundary)
        assert_eq!(
            bits.bit_len(),
            56,
            "deblocking consumes exactly byte 6 so gdf starts on byte 7"
        );
        bits.bit(1); // gdf_frame_enable (byte 7) -> dropped
        let mut data = bits.into_bytes();
        data.truncate(7); // 56 bits: the gdf_frame_enable read overruns
        let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();
        assert_truncated_filter_cluster_preserves_facts(&core);
        assert!(
            core.deblocking_filter_params.is_some(),
            "deblocking parsed before the gdf truncation must survive"
        );
        assert_eq!(
            core.gdf_params, None,
            "the truncated gdf structure stays None"
        );
        assert_eq!(core.cdef_params, None);
    }

    #[test]
    fn frame_header_core_eof_inside_cdef_params_preserves_facts() {
        // The payload parses cleanly through deblocking and gdf (gdf disabled-by-flag so it
        // short-circuits with no reads) and into cdef_params(), then runs out: deblocking
        // and gdf are preserved, cdef stays None, status is the marker. deblocking again
        // consumes exactly byte 6 (8 bits) so cdef begins at the byte-7 boundary (bit 56).
        let mut seq = byte_aligned_filter_seq();
        seq.filter.enable_cdef = true; // cdef_params() reads bits instead of short-circuiting
        // enable_gdf stays false so gdf_params() short-circuits with no reads.
        let mut bits = intra_body_up_to_filter_cluster();
        bits.bit(1); // apply_deblocking_filter[0]
        bits.bit(1); // apply_deblocking_filter[1]
        bits.bit(1); // apply_deblocking_filter[2]
        bits.bit(1); // apply_deblocking_filter[3]
        bits.bit(0); // df_delta_q_present[0]
        bits.bit(0); // df_delta_q_present[1]
        bits.bit(0); // df_delta_q_present[2]
        bits.bit(0); // df_delta_q_present[3] -> deblocking ends at bit 56 (byte boundary)
        // gdf_params(): enable_gdf == false -> no bits.
        bits.bit(1); // cdef_frame_enable (byte 7) -> dropped
        let mut data = bits.into_bytes();
        data.truncate(7); // 56 bits: the cdef_frame_enable read overruns
        let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();
        assert_truncated_filter_cluster_preserves_facts(&core);
        assert!(
            core.deblocking_filter_params.is_some(),
            "deblocking parsed before the cdef truncation must survive"
        );
        // gdf was frame-disabled, so its field is Some with gdf_frame_enable == false.
        assert_eq!(
            core.gdf_params.as_ref().map(|g| g.gdf_frame_enable),
            Some(false),
            "the frame-disabled gdf structure parsed (no bits) before the cdef truncation"
        );
        assert_eq!(
            core.cdef_params, None,
            "the truncated cdef structure stays None"
        );
    }

    #[test]
    fn frame_header_core_intra_tail_parses_lr_ccso_and_tail_to_completion() {
        // Restoration AND CCSO enabled: the intra tail parses cdef, then lr_params()
        // (no plane signals frame_filters_on, so no read_wienerns_filter) and ccso_params(),
        // then the §5.18.2 tail (read_tx_mode + reduced_tx_set, grain absent) to the
        // IntraHeaderComplete terminal. CDEF/GDF stay disabled so the cluster's only pre-lr
        // reads are the 2 deblocking apply bits.
        let mut seq = byte_aligned_filter_seq();
        // lr_tools both luma tools enabled; chroma PC-Wiener inferred disabled.
        seq.restoration.enable_restoration = true;
        seq.restoration.lr_uv_pc_wiener_disabled = true;
        seq.ccso.enable_ccso = true;
        let mut bits = intra_body_up_to_filter_cluster();
        // deblocking_filter_params(): not lossless -> apply[0]/[1] read, both 0.
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        // gdf_params() / cdef_params(): disabled -> no bits.
        // lr_params(): luma tool_index ns(4) == 0 -> RESTORE_NONE; chroma planes ns(2) == 0
        // -> RESTORE_NONE. No frame_filters_on, no size flags.
        bits.ns(0, 4); // plane 0 tool_index -> RESTORE_NONE
        bits.ns(0, 2); // plane 1 tool_index -> RESTORE_NONE
        bits.ns(0, 2); // plane 2 tool_index -> RESTORE_NONE
        // ccso_params(): not single picture -> ccso_frame_flag f(1) == 1, then all planes
        // ccso_planes == 0.
        bits.bit(1); // ccso_frame_flag
        bits.bit(0); // ccso_planes[0]
        bits.bit(0); // ccso_planes[1]
        bits.bit(0); // ccso_planes[2]
        // §5.18.2 tail: read_tx_mode() not lossless -> tx_mode_select f(1); reduced_tx_set
        // f(2); film_grain_config() grain absent (test_seq has film_grain_params_present ==
        // false) -> apply_grain inferred 0, no bits.
        bits.bit(1); // tx_mode_select = 1 -> TX_MODE_SELECT
        bits.f(3, 2); // reduced_tx_set = 3
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();
        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        let lr = core.lr_params.as_ref().unwrap();
        assert!(!lr.uses_lr);
        assert_eq!(lr.planes.len(), 3);
        assert!(
            lr.planes
                .iter()
                .all(|p| p.restoration_type == FrameRestorationType::None)
        );
        let ccso = core.ccso_params.as_ref().unwrap();
        assert_eq!(ccso.ccso_frame_flag, Some(true));
        assert_eq!(ccso.planes.len(), 3);
        assert!(ccso.planes.iter().all(|p| !p.ccso_planes));
        let tail = core.intra_tail.as_ref().expect("intra tail parsed");
        assert_eq!(tail.tx_mode, TxMode::Select);
        assert_eq!(tail.reduced_tx_set, 3);
        assert!(!tail.reference_select);
        assert!(!tail.skip_mode_present);
        assert!(!tail.allow_bawp);
        assert!(!tail.use_global_motion);
        assert!(!tail.film_grain.apply_grain);
    }

    #[test]
    fn frame_header_core_frame_filters_on_stops_before_wienerns() {
        // A luma plane selects RESTORE_WIENER_NONSEP and signals frame_filters_on -> the
        // structure reaches the unmodeled read_wienerns_filter() decode, so the parse stops
        // honestly with StoppedBeforeWienerNsFilter and the pre-Wiener facts are preserved.
        let mut seq = byte_aligned_filter_seq();
        seq.restoration.enable_restoration = true;
        seq.restoration.lr_uv_pc_wiener_disabled = true;
        seq.ccso.enable_ccso = true; // ccso never reached (lr stops first)
        let mut bits = intra_body_up_to_filter_cluster();
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        // lr_params(): plane 0 tool_index ns(4) == 2 -> RESTORE_WIENER_NONSEP.
        bits.ns(2, 4); // plane 0 -> RESTORE_WIENER_NONSEP
        bits.bit(1); // frame_filters_on[0] == 1
        bits.f(2, 3); // num_filter_classes_idx == 2 -> Decode_Num_Filter_Classes[2] == 3
        bits.ns(0, 2); // plane 1 -> RESTORE_NONE
        bits.ns(0, 2); // plane 2 -> RESTORE_NONE
        bits.bit(1); // lr_luma_use_half_size (size signaling still runs before the stop)
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::StoppedBeforeWienerNsFilter {
                feature_id: "AV2-5.18.7-SEGMENTATION-TILING"
            }
        );
        // The pre-cluster facts and the deblocking/cdef reads before the stop survive; the
        // lr_params field stays None (the structure did not complete) and ccso is unreached.
        assert_eq!(core.frame_size, Some(FrameSize::new(16, 16)));
        assert!(core.deblocking_filter_params.is_some());
        assert_eq!(core.lr_params, None);
        assert_eq!(core.ccso_params, None);
        // The lr_params() prefix parsed before the Wiener stop is preserved on the dedicated
        // partial field (facts-preservation invariant): the per-plane types,
        // frame_filters_on, the luma NumFilterClasses, UsesLr, and the size flags are real
        // consumed facts and must not be discarded.
        let partial = core
            .lr_params_partial
            .as_ref()
            .expect("the partial lr_params facts parsed before the Wiener stop are preserved");
        assert!(partial.uses_lr, "luma RESTORE_WIENER_NONSEP uses LR");
        assert_eq!(partial.planes.len(), 3);
        assert_eq!(
            partial.planes[0].restoration_type,
            FrameRestorationType::WienerNonsep
        );
        assert!(partial.planes[0].frame_filters_on);
        // num_filter_classes_idx == 2 -> Decode_Num_Filter_Classes[2] == 3.
        assert_eq!(partial.planes[0].num_filter_classes, Some(3));
        assert_eq!(
            partial.planes[1].restoration_type,
            FrameRestorationType::None
        );
        assert!(!partial.planes[1].frame_filters_on);
        // lr_luma_use_half_size -> 512 >> 1 == 256 (size flags read before the stop).
        assert_eq!(partial.loop_restoration_size[0], 256);
    }

    #[test]
    fn frame_header_core_eof_inside_ccso_params_preserves_facts() {
        // The payload parses through deblocking and lr_params() (restoration disabled so it
        // reads nothing) and into ccso_params(), then runs out at the ccso_frame_flag read:
        // the earlier facts survive, ccso stays None, status is the truncation marker. The
        // deblocking reads consume exactly byte 6 (bit 56) so ccso begins at the byte-7
        // boundary.
        let mut seq = byte_aligned_filter_seq();
        // restoration disabled (lr reads nothing); ccso enabled (reads the frame flag).
        seq.ccso.enable_ccso = true;
        let mut bits = intra_body_up_to_filter_cluster();
        bits.bit(1); // apply_deblocking_filter[0]
        bits.bit(1); // apply_deblocking_filter[1]
        bits.bit(1); // apply_deblocking_filter[2]
        bits.bit(1); // apply_deblocking_filter[3]
        bits.bit(0); // df_delta_q_present[0]
        bits.bit(0); // df_delta_q_present[1]
        bits.bit(0); // df_delta_q_present[2]
        bits.bit(0); // df_delta_q_present[3] -> deblocking ends at bit 56 (byte boundary)
        // gdf/cdef disabled -> no bits. lr disabled -> no bits.
        bits.bit(1); // ccso_frame_flag (byte 7) -> dropped by truncation
        let mut data = bits.into_bytes();
        data.truncate(7); // 56 bits: the ccso_frame_flag read overruns
        let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();
        assert_truncated_filter_cluster_preserves_facts(&core);
        assert!(
            core.deblocking_filter_params.is_some(),
            "deblocking parsed before the ccso truncation must survive"
        );
        assert!(
            core.lr_params.is_some(),
            "the restoration-disabled lr structure parsed (no bits) before the ccso truncation"
        );
        assert_eq!(
            core.ccso_params, None,
            "the truncated ccso structure stays None"
        );
    }

    #[test]
    fn frame_header_core_eof_inside_intra_tail_preserves_cluster_facts() {
        // The payload parses cleanly through ccso_params() but ends inside the § 5.18.2
        // tail (the reduced_tx_set f(2) read overruns): the control-region and loop-filter
        // facts survive, intra_tail stays None, and the status is StoppedInsideIntraTail.
        let mut seq = byte_aligned_filter_seq();
        seq.ccso.enable_ccso = true;
        let mut bits = intra_body_up_to_filter_cluster();
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        // gdf/cdef/lr disabled -> no bits. ccso enabled -> ccso_frame_flag f(1) + 3 planes.
        bits.bit(1); // ccso_frame_flag
        bits.bit(0); // ccso_planes[0]
        bits.bit(0); // ccso_planes[1]
        bits.bit(0); // ccso_planes[2]
        // § 5.18.2 tail: tx_mode_select f(1), then reduced_tx_set f(2) — supply only the
        // tx bit and ONE of the two reduced_tx_set bits, then truncate so the second
        // reduced_tx_set bit overruns.
        bits.bit(0); // tx_mode_select = 0 -> Largest
        bits.bit(0); // 1 of 2 reduced_tx_set bits; the next bit is missing
        let total_bits = bits.bit_len();
        let mut data = bits.into_bytes();
        // Truncate to the last whole byte that still contains the tx + partial reduced bits
        // but not a full second reduced_tx_set bit. total_bits here is mid-byte, so keeping
        // ceil(total_bits/8) - 0 bytes and not padding more leaves the read short.
        let keep_bytes = total_bits / 8; // drop the partial trailing byte -> reduced_tx_set overruns
        data.truncate(keep_bytes);
        let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();
        assert_eq!(core.status, FrameHeaderParseStatus::StoppedInsideIntraTail);
        // Control-region and cluster facts survive; the tail itself was not committed.
        assert_eq!(core.frame_size, Some(FrameSize::new(16, 16)));
        assert!(core.deblocking_filter_params.is_some());
        assert!(core.lr_params.is_some());
        assert!(core.ccso_params.is_some());
        assert_eq!(core.intra_tail, None, "the truncated tail stays None");
    }

    #[test]
    fn frame_header_core_intra_tail_with_grain_present_reads_id_and_seed() {
        // film_grain_params_present == true on an OUTPUT key frame (immediate_output_frame
        // == 1): film_grain_config()'s output gate is false, so apply_grain is read f(1);
        // when set, fgm_id f(3) + grain_seed f(16). Build the body with the output flag set
        // (the byte-aligned helper hardcodes both output flags to 0, which would force
        // apply_grain = 0).
        let mut seq = base_seq();
        seq.film_grain_params_present = Some(true);
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.bit(1); // immediate_output_frame == 1 (output frame -> apply_grain readable)
        // implicit_output_frame inferred 0 (immediate_output_frame == 1), no bit.
        bits.bit(0); // frame_size_override_flag == 0 (cur_mfh_id == 0 -> max dims)
        bits.f(3, 4); // order_hint f(4)
        // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag (4096x2304 single uniform tile)
        bits.bit(0); // increment_tile_cols_log2 = 0
        bits.bit(0); // increment_tile_rows_log2 = 0
        bits.f(90, 8); // base_q_idx (non-lossless)
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        // gdf/cdef/lr/ccso all disabled in base_seq -> no bits.
        // § 5.18.2 tail: tx_mode_select f(1); reduced_tx_set f(2); film_grain_config()
        // grain present + immediate_output -> apply_grain f(1) + fgm_id f(3) + grain_seed f(16).
        bits.bit(1); // tx_mode_select = 1 -> Select
        bits.f(1, 2); // reduced_tx_set = 1
        bits.bit(1); // apply_grain = 1
        bits.f(4, 3); // fgm_id = 4
        bits.f(0xC0DE, 16); // grain_seed
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();
        assert_eq!(core.immediate_output_frame, Some(true));
        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        let tail = core.intra_tail.as_ref().unwrap();
        assert_eq!(tail.tx_mode, TxMode::Select);
        assert_eq!(tail.reduced_tx_set, 1);
        assert!(tail.film_grain.apply_grain);
        assert_eq!(tail.film_grain.fgm_id, Some(4));
        assert_eq!(tail.film_grain.grain_seed, Some(0xC0DE));
    }

    #[test]
    fn frame_header_core_intra_unknown_grain_flag_parses_control_region_then_stops() {
        // film_grain_params_present == None models an active sequence header recorded from a
        // bounded sequence_tile_config() stop (the flag is read last in § 5.4.1, after every
        // child config). Pre-fix CoreSeqView::from_sequence's `?` on the flag collapsed the
        // whole view, so the frame parse stopped at ActivationFieldsOnly and every
        // frame-size / output / order-hint diagnostic was suppressed. Now the control region
        // (which never consumes the flag) parses to completion and the parser stops honestly
        // at the film_grain_config() boundary — facts preserved, NOT a guessed apply_grain.
        let mut seq = base_seq();
        seq.film_grain_params_present = None;
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.bit(1); // immediate_output_frame == 1
        bits.bit(0); // frame_size_override_flag == 0 (cur_mfh_id == 0 -> max dims 4096x2304)
        bits.f(3, 4); // order_hint f(4)
        // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2 = 0
        bits.bit(0); // increment_tile_rows_log2 = 0
        bits.f(90, 8); // base_q_idx (non-lossless)
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        // gdf/cdef/lr/ccso disabled in base_seq -> no bits. The next structure is the
        // § 5.18.2 tail, whose film_grain_config() needs the (unknown) grain flag.
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();
        // The control region parsed: these facts feed the validator's §6.17 diagnostics.
        assert_eq!(core.immediate_output_frame, Some(true));
        assert_eq!(core.order_hint_lsb, Some(3));
        assert_eq!(core.frame_size, Some(FrameSize::new(4096, 2304)));
        assert!(core.quantization_params.is_some());
        assert!(core.deblocking_filter_params.is_some());
        // The parser stopped honestly at the film_grain_config() boundary, not at the prefix
        // (ActivationFieldsOnly) and never guessing apply_grain.
        assert!(matches!(
            core.status,
            FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
        ));
        assert_ne!(core.status, FrameHeaderParseStatus::ActivationFieldsOnly);
        assert_eq!(
            core.intra_tail, None,
            "the grain-gated tail was not reached"
        );
        assert!(
            !core.status.is_truncated_in_modeled_region(),
            "an unknown-flag stop is a coverage stop, not a truncation defect"
        );
    }

    #[test]
    fn frame_header_core_sef_unknown_grain_flag_preserves_facts_then_stops() {
        // SEF whose active sequence header is a bounded stop (film_grain_params_present ==
        // None). The SEF fields (frame_to_show_map_idx, order hint, output flags) are parsed
        // and preserved, but film_grain_config() needs the unknown grain flag, so the parser
        // stops honestly rather than guessing apply_grain. Pre-fix the whole parse collapsed
        // to ActivationFieldsOnly.
        let mut seq = base_seq();
        seq.film_grain_params_present = None;
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.f(6, 3); // frame_to_show_map_idx
        bits.bit(0); // derive_sef_order_hint == 0
        bits.f(11, 4); // sef_order_hint f(4)
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::RegularSef, true, &seq).unwrap();
        assert_eq!(core.show_existing_frame, Some(true));
        assert_eq!(core.frame_to_show_map_idx, Some(6));
        assert_eq!(core.order_hint_lsb, Some(11));
        assert_eq!(core.refresh_frame_flags, Some(0));
        assert!(matches!(
            core.status,
            FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
        ));
        assert_ne!(core.status, FrameHeaderParseStatus::ActivationFieldsOnly);
        assert_eq!(
            core.sef_film_grain, None,
            "grain not decided without the flag"
        );
        assert_eq!(
            core.sef_trailing_bits, None,
            "no completed SEF tail to classify"
        );
    }

    #[test]
    fn frame_header_core_sef_with_grain_reads_apply_grain_then_completes() {
        // SEF with film_grain_params_present == true: immediate_output_frame == 1 makes the
        // output gate false, so apply_grain is read f(1); when set, fgm_id + grain_seed.
        let mut seq = base_seq();
        seq.film_grain_params_present = Some(true);
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.f(6, 3); // frame_to_show_map_idx
        bits.bit(1); // derive_sef_order_hint == 1 -> no sef_order_hint
        // film_grain_config(): grain present, immediate_output -> apply_grain f(1).
        bits.bit(1); // apply_grain = 1
        bits.f(2, 3); // fgm_id = 2
        bits.f(0x1357, 16); // grain_seed
        // § 5.2.3 trailing_bits(): trailing_one_bit == 1, then into_bytes() zero-pads the
        // rest of the byte (the trailing_zero_bits) — a conformant SEF tail.
        bits.bit(1); // trailing_one_bit
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::RegularSef, true, &seq).unwrap();
        assert_eq!(core.show_existing_frame, Some(true));
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::ShowExistingFrameComplete
        );
        let fg = core.sef_film_grain.expect("SEF film_grain_config parsed");
        assert!(fg.apply_grain);
        assert_eq!(fg.fgm_id, Some(2));
        assert_eq!(fg.grain_seed, Some(0x1357));
        // A conformant SEF tail classifies as Valid (no diagnostic).
        assert_eq!(core.sef_trailing_bits, Some(SefTrailingBits::Valid));
    }

    #[test]
    fn frame_header_core_sef_eof_inside_film_grain_preserves_facts() {
        // SEF with grain present but the payload ends inside film_grain_config(): the SEF
        // facts survive and the status reports StoppedInsideShowExistingFrame — the SEF
        // tail IS film_grain_config(), a fully-modeled region, so an EOF there is a
        // decidable truncation (distinct from the ordinary bounded CoreFieldsOnly stop),
        // surfaced as truncated-in-modeled-region. Not a hard error.
        let mut seq = base_seq();
        seq.film_grain_params_present = Some(true);
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.f(6, 3); // frame_to_show_map_idx
        bits.bit(1); // derive_sef_order_hint == 1
        bits.bit(1); // apply_grain = 1
        bits.f(2, 3); // fgm_id = 2
        bits.f(0, 8); // only 8 of 16 grain_seed bits, then EOF
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::RegularSef, true, &seq).unwrap();
        assert_eq!(core.show_existing_frame, Some(true));
        assert_eq!(core.frame_to_show_map_idx, Some(6));
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::StoppedInsideShowExistingFrame
        );
        assert!(
            core.status.is_truncated_in_modeled_region(),
            "an EOF in the SEF film_grain_config() tail is a truncation in a modeled region"
        );
        assert_eq!(
            core.sef_film_grain, None,
            "the truncated SEF grain stays None"
        );
        // No trailing-bits boundary is recorded on a truncated SEF: the payload ended
        // inside film_grain_config(), so there is no completed tail to classify.
        assert_eq!(core.sef_trailing_bits, None);
    }

    #[test]
    fn frame_header_core_sef_nonzero_bits_after_fields_flag_trailing_bits() {
        // A grain-free SEF whose payload carries arbitrary nonzero bits after the parsed
        // fields where § 5.2.3 trailing_bits() must be. Pre-fix this completed silently
        // (ShowExistingFrameComplete with no trailing-bits boundary); now the SEF tail is
        // classified and a non-conformant tail is recorded for the validator.
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.f(6, 3); // frame_to_show_map_idx
        bits.bit(1); // derive_sef_order_hint == 1 -> no sef_order_hint
        // No grain (base_seq has film_grain_params_present == false) -> apply_grain = 0.
        // The next bit must be the trailing_one_bit == 1; instead a 0 then arbitrary bits.
        bits.bit(0); // would-be trailing_one_bit, but it is 0
        bits.f(0b1011, 4); // arbitrary nonzero bits after the SEF fields
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::RegularSef, true, &base_seq()).unwrap();
        assert_eq!(core.show_existing_frame, Some(true));
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::ShowExistingFrameComplete,
            "the SEF fields still parse to completion; the defect is in the tail"
        );
        assert_eq!(
            core.sef_trailing_bits,
            Some(SefTrailingBits::MissingOneBit),
            "the first post-field bit was not the required trailing_one_bit"
        );
    }

    #[test]
    fn frame_header_core_sef_grain_seed_short_one_bit_eats_trailing_marker() {
        // A SEF with grain where grain_seed is short by its final bit: the f(16) read
        // consumes what should have been the § 5.2.3 trailing_one_bit, leaving no marker.
        // Pre-fix this completed clean with a corrupted seed; now the trailing-bits check
        // fails (the marker was eaten), so the SEF no longer completes silently.
        let mut seq = base_seq();
        seq.film_grain_params_present = Some(true);
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.f(6, 3); // frame_to_show_map_idx
        bits.bit(1); // derive_sef_order_hint == 1
        bits.bit(1); // apply_grain = 1
        bits.f(2, 3); // fgm_id = 2
        // A conformant frame would code grain_seed f(16) then a trailing_one_bit. Here the
        // encoder coded only 15 distinct seed bits plus the marker bit, so the f(16) read
        // swallows the marker: 15 seed bits then the would-be trailing_one_bit as bit 16,
        // and into_bytes() zero-fills the rest — no trailing_one_bit remains.
        bits.f(0x0000, 15); // 15 seed bits
        bits.bit(1); // the marker bit, consumed as the 16th grain_seed bit
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::RegularSef, true, &seq).unwrap();
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::ShowExistingFrameComplete
        );
        let fg = core.sef_film_grain.expect("SEF film_grain_config parsed");
        // The seed is parsed (with the marker bit folded into it), but the trailing-bits
        // boundary is now non-conformant: the bytes after grain_seed are all zero, so the
        // first remaining bit is 0 (MissingOneBit) — or, if grain_seed ended exactly at a
        // byte boundary, no bits remain (Empty). Either is a recorded violation.
        assert_eq!(fg.grain_seed, Some(1));
        assert_ne!(
            core.sef_trailing_bits,
            Some(SefTrailingBits::Valid),
            "the eaten trailing_one_bit makes the SEF tail non-conformant"
        );
        assert!(matches!(
            core.sef_trailing_bits,
            Some(SefTrailingBits::MissingOneBit | SefTrailingBits::Empty)
        ));
    }

    #[test]
    fn frame_header_core_unresolvable_mfh_default_size_stays_unsupported() {
        // CLK, cur_mfh_id == 2 with NO resolved MFH record and
        // frame_size_override_flag == 0: the default dims come from the (unresolvable)
        // MFH, so the size is unknown and the parser stops before tile_info() without
        // guessing — the Unknown-routing case.
        let mut bits = Bits::default();
        bits.uvlc(2); // cur_mfh_id == 2 -> no seq_header_id_in_frame_header
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(0); // frame_size_override_flag == 0 (default dims)
        bits.f(7, 4); // order_hint
        // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
        // frame_size(): default path, no bits
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        let data = bits.into_bytes();
        // No MFH record -> unresolvable.
        let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &base_seq()).unwrap();

        assert_eq!(core.cur_mfh_id.get(), 2);
        assert_eq!(core.seq_header_id_in_frame_header, None);
        assert_eq!(core.order_hint_lsb, Some(7));
        assert_eq!(
            core.frame_size, None,
            "unresolvable cur_mfh_id > 0 default dims stay unknown"
        );
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::UnsupportedUntilFeature {
                feature_id: "AV2-5.18.2-FRAME-HEADER-INFO"
            }
        );
        assert_eq!(core.tile_info, None);
        assert_eq!(core.quantization_params, None);
    }

    #[test]
    fn frame_header_core_unresolvable_mfh_with_explicit_size_stops_before_segmentation() {
        // CLK, cur_mfh_id == 1, frame_size_override_flag == 1 (explicit dims), but NO
        // resolved MFH record: tile_info() / quantization_params() parse from the
        // explicit size, but segmentation_params() needs mfh_seg_info_present_flag
        // (§ 5.18.7.1), which is undecidable without the record — so the parser stops
        // there rather than guessing the sequence/zero arm.
        let mut bits = Bits::default();
        bits.uvlc(1); // cur_mfh_id == 1 -> no seq_header_id_in_frame_header
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag == 1 (explicit dims)
        bits.f(7, 4); // order_hint
        // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
        bits.f(1920 - 1, 12); // frame_width_minus_1
        bits.f(1080 - 1, 12); // frame_height_minus_1
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag (tile_info, single tile)
        bits.bit(0); // increment_tile_cols_log2 = 0
        bits.bit(0); // increment_tile_rows_log2 = 0
        bits.f(70, 8); // base_q_idx (quantization_params)
        let data = bits.into_bytes();
        let (core, consumed) =
            parse_body(&data, ObuType::ClosedLoopKey, true, &base_seq()).unwrap();

        assert_eq!(core.cur_mfh_id.get(), 1);
        assert_eq!(core.frame_size, Some(FrameSize::new(1920, 1080)));
        assert_eq!(
            core.frame_size_override_flag,
            Some(true),
            "the override path records frame_size_override_flag == 1 (explicit dims provenance)"
        );
        assert_eq!(core.tile_info.as_ref().unwrap().tile_cols, 1);
        assert_eq!(core.quantization_params.unwrap().base_q_idx, 70);
        assert_eq!(core.segmentation_params, None);
        assert_eq!(core.setup_qm_params, None);
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::UnsupportedUntilFeature {
                feature_id: "AV2-5.18.2-FRAME-HEADER-INFO"
            }
        );
        // uvlc(1)=3 prefix bits, then 33 core bits, then 3 tile_info bits and the
        // 8-bit base_q_idx; segmentation_params() is not reached.
        assert_eq!(consumed, 3 + 33 + 3 + 8);
    }

    #[test]
    fn frame_header_core_mfh_default_dims_parse_through_tile_info() {
        // CLK, cur_mfh_id == 1, frame_size_override_flag == 0, resolved MFH carrying
        // explicit 1920x1080 dims: the § 5.18.4.1 default path uses the MFH dims (no
        // frame-size bits), and tile_info()/quantization_params()/segmentation_params()
        // parse through to the deblocking stop.
        let mfh_size = Some(crate::hls::MfhFrameSize {
            width_bits: 12,
            height_bits: 12,
            width_minus_1: 1920 - 1,
            height_minus_1: 1080 - 1,
        });
        let record = mfh_record(mfh_size, None); // mfh_seg_info_present_flag == 0
        let view = MfhFrameView::from_record(&record, &base_seq());

        let mut bits = Bits::default();
        bits.uvlc(1); // cur_mfh_id == 1
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(0); // frame_size_override_flag == 0 (MFH default dims, no bits)
        bits.f(7, 4); // order_hint
        // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
        // frame_size(): MFH default path, no bits
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag (single tile)
        bits.bit(0); // increment_tile_cols_log2 = 0
        bits.bit(0); // increment_tile_rows_log2 = 0
        bits.f(70, 8); // base_q_idx
        // segmentation_params(): mfh_seg_info_present_flag == 0, seq has no info ->
        // sequence/zero arm. segmentation_enabled == 0 (no further bits).
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix (setup_qm_params)
        bits.bit(0); // delta_q_present (base_q_idx 70 > 0; 0 -> no further delta_q bits)
        // lossless tail: base_q_idx 70 non-lossless, no QM -> no qm_index bits; base_seq
        // has choose_tcq_per_frame / enable_parity_hiding off -> no allow_* bits.
        // deblocking_filter_params(): the resolved MFH did not signal an update
        // (mfh_deblocking_filter_update == 0), so apply[0]/[1] are read from the
        // bitstream. GDF/CDEF disabled in base_filter -> no bits.
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        // lr_params()/ccso_params(): restoration and CCSO disabled -> no bits.
        // § 5.18.2 tail: tx_mode_select f(1) + reduced_tx_set f(2); grain absent.
        bits.bit(0); // tx_mode_select = 0
        bits.f(0, 2); // reduced_tx_set = 0
        let data = bits.into_bytes();
        let (core, _) = parse_body_with_mfh(
            &data,
            ObuType::ClosedLoopKey,
            true,
            &base_seq(),
            Some(&view),
        )
        .unwrap();

        assert_eq!(core.cur_mfh_id.get(), 1);
        assert_eq!(
            core.frame_size,
            Some(FrameSize::new(1920, 1080)),
            "MFH default dims drive frame_size on the non-override path"
        );
        assert_eq!(
            core.frame_size_override_flag,
            Some(false),
            "the non-override default path records frame_size_override_flag == 0 (MFH-default provenance)"
        );
        assert_eq!(core.tile_info.as_ref().unwrap().tile_cols, 1);
        assert_eq!(core.quantization_params.unwrap().base_q_idx, 70);
        assert!(!core.segmentation_params.unwrap().segmentation_enabled);
        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
    }

    #[test]
    fn frame_header_core_mfh_omitted_size_infers_sequence_maxima() {
        // cur_mfh_id == 1, resolved MFH with NO frame-size payload: § 5.18.2 (:4101)
        // infers the default dims to the sequence maxima (base_seq: 4096x2304).
        let record = mfh_record(None, None); // no mfh_frame_size, no seg info
        let view = MfhFrameView::from_record(&record, &base_seq());
        assert_eq!(view.default_dims, (4096, 2304));

        let mut bits = Bits::default();
        bits.uvlc(1); // cur_mfh_id == 1
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(0); // frame_size_override_flag == 0 (MFH default = inferred maxima)
        bits.f(7, 4); // order_hint
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(0, 8); // base_q_idx == 0 (no delta_q bits)
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        // base_q_idx == 0 -> delta_q_present inferred 0 (no bit). Lossless tail: every
        // segment lossless, so no qm_index bits; then allow_tcq / allow_parity_hiding
        // gated off in base_seq -> no bits.
        // lr_params()/ccso_params(): restoration and CCSO disabled -> no bits.
        // § 5.18.2 tail: CodedLossless == 1 -> read_tx_mode() reads NO bit (TxMode =
        // ONLY_4X4); reduced_tx_set f(2) is still read; grain absent.
        bits.f(0, 2); // reduced_tx_set = 0 (no tx_mode_select bit on the lossless gate)
        let data = bits.into_bytes();
        let (core, _) = parse_body_with_mfh(
            &data,
            ObuType::ClosedLoopKey,
            true,
            &base_seq(),
            Some(&view),
        )
        .unwrap();

        assert_eq!(
            core.frame_size,
            Some(FrameSize::new(4096, 2304)),
            "omitted MFH size infers the sequence maxima (:4101)"
        );
        // CodedLossless == 1 here, so deblocking_filter_params() returns with all
        // apply flags 0 and GDF/CDEF stay disabled, all without reading bits.
        assert!(core.lossless_info.as_ref().unwrap().coded_lossless);
        assert_eq!(
            core.deblocking_filter_params
                .unwrap()
                .apply_deblocking_filter,
            [false; 4]
        );
        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        // The CodedLossless gate skipped tx_mode_select: TxMode is ONLY_4X4.
        let tail = core.intra_tail.as_ref().unwrap();
        assert_eq!(tail.tx_mode, TxMode::Only4x4);
        assert_eq!(tail.reduced_tx_set, 0);
    }

    #[test]
    fn frame_header_core_mfh_segmentation_arm_reuses_mfh_feature_data() {
        // cur_mfh_id == 1, frame_size_override_flag == 1, resolved MFH with
        // mfh_seg_info_present_flag == 1, mfh_ext_seg_flag == enable_ext_seg (false),
        // mfh_allow_seg_info_change == 0: § 5.18.7.1 selects the MFH arm with
        // haveSegParams == 1, allowChange == 0, so reuse_seg_info is inferred 1 (no bit)
        // and FeatureData copies MfhFeatureData[cur_mfh_id].
        let mut mfh_features = [[SegmentFeature::DISABLED; SEG_LVL_MAX]; MAX_SEGMENTS];
        mfh_features[3][0] = SegmentFeature {
            enabled: true,
            data: 7,
        };
        let record = mfh_record(
            None,
            Some((
                false, // mfh_ext_seg_flag == enable_ext_seg (base_seq enable_ext_seg = false)
                false, // mfh_allow_seg_info_change
                SegmentInfo {
                    num_segments: 8,
                    features: mfh_features,
                },
            )),
        );
        let view = MfhFrameView::from_record(&record, &base_seq());

        let mut bits = Bits::default();
        bits.uvlc(1); // cur_mfh_id == 1
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag == 1
        bits.f(7, 4); // order_hint
        bits.f(1920 - 1, 12); // frame_width_minus_1
        bits.f(1080 - 1, 12); // frame_height_minus_1
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2
        bits.bit(0); // increment_tile_rows_log2
        bits.f(70, 8); // base_q_idx
        // segmentation_params(): MFH arm, haveSegParams==1, allowChange==0 ->
        // reuse_seg_info inferred 1, no reuse bit, copy MFH features.
        bits.bit(1); // segmentation_enabled
        // setup_qm_params(): using_qmatrix off.
        bits.bit(0); // using_qmatrix
        // delta_q_params(): base_q_idx 70 > 0.
        bits.bit(0); // delta_q_present
        // lossless tail: segment 3 has alt-q feature data 7 -> non-lossless; others
        // disabled (qindex == base_q_idx 70, non-lossless). No QM -> no qm_index bits.
        // deblocking_filter_params(): not lossless, MFH did not signal an update ->
        // apply[0]/[1] read. GDF/CDEF disabled in base_filter.
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        // lr_params()/ccso_params(): restoration and CCSO disabled -> no bits.
        // § 5.18.2 tail: tx_mode_select f(1) + reduced_tx_set f(2); grain absent.
        bits.bit(0); // tx_mode_select = 0
        bits.f(0, 2); // reduced_tx_set = 0
        let data = bits.into_bytes();
        let (core, _) = parse_body_with_mfh(
            &data,
            ObuType::ClosedLoopKey,
            true,
            &base_seq(),
            Some(&view),
        )
        .unwrap();

        let seg = core
            .segmentation_params
            .expect("segmentation parsed on MFH arm");
        assert!(seg.segmentation_enabled);
        assert!(
            seg.reuse_seg_info,
            "MFH arm with allowChange==0 infers reuse"
        );
        assert!(
            seg.features[3][0].enabled,
            "reuse copies MfhFeatureEnabled/MfhFeatureData, not sequence data"
        );
        assert_eq!(seg.features[3][0].data, 7);
        assert_eq!(seg.last_active_seg_id, 3);
        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        assert!(core.intra_tail.is_some());
    }

    #[test]
    fn frame_header_core_intra_tail_parses_full_structure_cluster() {
        // The full § 5.18.2 intra tail in spec order: a 2x1-tile tile_info() with
        // context fields, quantization_params(), segmentation_params() (enabled,
        // fresh all-disabled seg_info), setup_qm_params() with two QM sets,
        // delta_q_params(), per-segment qm_index reads, allow_tcq,
        // allow_parity_hiding, and the loop-filter cluster
        // deblocking_filter_params() / gdf_params() / cdef_params() with both GDF and
        // CDEF enabled.
        let mut seq = base_seq();
        seq.quant.choose_tcq_per_frame = true;
        seq.quant.enable_parity_hiding = true;
        seq.filter.enable_gdf = true;
        seq.filter.enable_cdef = true;
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag
        bits.f(5, 4); // order_hint
        // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
        bits.f(1920 - 1, 12); // frame_width_minus_1
        bits.f(1080 - 1, 12); // frame_height_minus_1
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        // tile_info() (§ 5.18.7.2): 1920x1080 with 128x128 superblocks (sbCols = 15,
        // sbRows = 9), one column increment -> TileCols = 2 (starts 0, 8), 1 row.
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(1); // increment_tile_cols_log2 = 1
        bits.bit(0); // increment_tile_cols_log2 = 0
        bits.bit(0); // increment_tile_rows_log2 = 0
        bits.f(1, 1); // context_update_tile_id f(TileRowsLog2 + TileColsLog2 == 1)
        bits.f(3, 2); // tile_size_bytes_minus_1 -> TileSizeBytes = 4
        // quantization_params() (§ 5.18.6.1).
        bits.f(40, 8); // base_q_idx
        // segmentation_params() (§ 5.18.7.1): enabled, no sequence info ->
        // reuse_seg_info inferred 0, fresh seg_info(8) with all features disabled.
        bits.bit(1); // segmentation_enabled
        for _ in 0..8 {
            bits.f(0, 3); // seg_info: feature_enabled[i][0..3] = 0
        }
        // setup_qm_params() (§ 5.18.6.2): segmentation_enabled gates pic_qm_num.
        bits.bit(1); // using_qmatrix
        bits.f(1, 2); // pic_qm_num_minus_1 -> qmNum = 2
        bits.f(3, 4); // qm_y[0]
        bits.bit(1); // qm_uv_same_as_y[0]
        bits.f(5, 4); // qm_y[1]
        bits.bit(1); // qm_uv_same_as_y[1]
        // delta_q_params() (§ 5.18.7.8).
        bits.bit(0); // delta_q_present
        // § 5.18.2 lossless tail: every segment has qindex 40 (non-lossless), so each
        // of the 8 segments reads qm_index f(CeilLog2(2) == 1) == 1.
        for _ in 0..8 {
            bits.bit(1); // qm_index
        }
        bits.bit(0); // allow_tcq (choose_tcq_per_frame)
        bits.bit(1); // allow_parity_hiding
        // deblocking_filter_params() (§ 5.18.5.2): not lossless, df_par_bits_minus_2 == 0
        // -> dfParBits = 2. apply[0]=1, apply[1]=0, NumPlanes 3 + luma set -> apply[2]/[3]
        // read.
        bits.bit(1); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // apply_deblocking_filter[2]
        bits.bit(0); // apply_deblocking_filter[3]
        // i == 0 applies: df_delta_q_present[0]=1, df_delta_q[0] f(2)==3 -> 3-2==1.
        bits.bit(1); // df_delta_q_present[0]
        bits.f(3, 2); // df_delta_q[0]
        // i == 1: apply==0 -> DfDeltaQ[1] = DfDeltaQ[0] == 1 (no bits).
        // i == 2/3: apply==0 -> DfDeltaQ == 0 (no bits).
        // gdf_params() (§ 5.18.7.9): not single picture -> gdf_frame_enable f(1)==1.
        // SbSize 128x128, MiCols(480)*4 == 1920 > gdfBlkSize(128) -> gdf_per_block f(1).
        bits.bit(1); // gdf_frame_enable
        bits.bit(0); // gdf_per_block
        bits.f(2, 2); // gdf_pic_qc_idx
        bits.f(3, 2); // gdf_pic_scale_idx -> GdfPixScale = 4
        // cdef_params() (§ 5.18.7.10): not single picture -> cdef_frame_enable f(1)==1.
        bits.bit(1); // cdef_frame_enable
        bits.f(1, 2); // cdef_damping_minus_3 -> CdefDamping = 4
        bits.f(0, 3); // cdef_strengths_minus_1 -> CdefStrengths = 1
        bits.bit(1); // cdef_on_skip_txfm_frame_enable (adaptive -> read)
        bits.bit(0); // cdef_y_pri_zero -> read f(4)
        bits.f(9, 4); // cdef_y_pri_strength[0]
        bits.f(1, 2); // cdef_y_sec_strength[0]
        bits.bit(1); // cdef_uv_pri_zero -> 0
        bits.f(3, 2); // cdef_uv_sec_strength[0] == 3 -> 4
        // lr_params()/ccso_params(): restoration and CCSO disabled (base_seq) -> no bits.
        // § 5.18.2 tail: read_tx_mode() not lossless -> tx_mode_select f(1); reduced_tx_set
        // f(2); film_grain_config() grain absent -> apply_grain inferred 0, no bits.
        bits.bit(1); // tx_mode_select = 1 -> TX_MODE_SELECT
        bits.f(2, 2); // reduced_tx_set = 2
        let data = bits.into_bytes();
        let (core, consumed) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();

        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        let tile_info = core.tile_info.as_ref().unwrap();
        assert_eq!(tile_info.tile_cols, 2);
        assert_eq!(tile_info.tile_rows, 1);
        assert_eq!(tile_info.tile_cols_log2, 1);
        assert_eq!(tile_info.mi_col_starts, vec![0, 256, 480]);
        assert_eq!(tile_info.mi_row_starts, vec![0, 270]);
        assert_eq!(tile_info.context_update_tile_id, 1);
        assert_eq!(tile_info.tile_size_bytes, Some(4));
        assert_eq!(core.quantization_params.unwrap().base_q_idx, 40);
        let segmentation = core.segmentation_params.unwrap();
        assert!(segmentation.segmentation_enabled);
        assert!(segmentation.segmentation_update_map);
        assert!(!segmentation.segmentation_temporal_update);
        let qm = core.setup_qm_params.unwrap();
        assert!(qm.using_qmatrix);
        assert_eq!(qm.pic_qm_num_minus_1, 1);
        assert_eq!(qm.levels[0].qm_y, 3);
        assert_eq!(qm.levels[1].qm_y, 5);
        assert!(!core.delta_q_params.unwrap().delta_q_present);
        let lossless = core.lossless_info.unwrap();
        assert!(!lossless.coded_lossless);
        assert!(!lossless.has_lossless_segment);
        // Every segment selected QM set 1 (qm_uv_same_as_y -> [5, 5, 5]).
        assert!(lossless.seg_qm_levels[..8].iter().all(|l| *l == [5, 5, 5]));
        assert!(!lossless.allow_tcq);
        assert!(lossless.allow_parity_hiding);
        // deblocking_filter_params(): apply[0] set, df_delta_q[0] == 1; apply[1..4] == 0
        // so DfDeltaQ[1..4] take the outer-else 0.
        let deblocking = core.deblocking_filter_params.unwrap();
        assert_eq!(
            deblocking.apply_deblocking_filter,
            [true, false, false, false]
        );
        assert_eq!(deblocking.df_delta_q_present, [true, false, false, false]);
        assert_eq!(deblocking.df_delta_q, [1, 0, 0, 0]);
        // gdf_params(): frame-enabled, per-block 0, qc 2, scale 3.
        let gdf = core.gdf_params.unwrap();
        assert!(gdf.gdf_frame_enable);
        assert_eq!(gdf.gdf_per_block, Some(false));
        assert_eq!(gdf.gdf_pic_qc_idx, Some(2));
        assert_eq!(gdf.gdf_pic_scale_idx, Some(3));
        // cdef_params(): one strength set, CdefDamping 4, y_sec remap 1, uv_sec 3->4.
        let cdef = core.cdef_params.unwrap();
        assert!(cdef.cdef_frame_enable);
        assert_eq!(cdef.cdef_damping, Some(4));
        assert_eq!(cdef.cdef_strengths, Some(1));
        assert_eq!(cdef.cdef_on_skip_txfm_frame_enable, Some(true));
        assert_eq!(cdef.strengths.len(), 1);
        assert_eq!(cdef.strengths[0].y_pri_strength, 9);
        assert_eq!(cdef.strengths[0].y_sec_strength, 1);
        assert_eq!(cdef.strengths[0].uv_pri_strength, 0);
        assert_eq!(cdef.strengths[0].uv_sec_strength, 4);
        // lr_params(): restoration disabled (base_seq) -> Parsed with uses_lr == false and
        // no per-plane reads. ccso_params(): CCSO disabled -> ccso_frame_flag None, no reads.
        let lr = core.lr_params.as_ref().unwrap();
        assert!(!lr.uses_lr);
        assert!(lr.planes.is_empty());
        let ccso = core.ccso_params.as_ref().unwrap();
        assert_eq!(ccso.ccso_frame_flag, None);
        assert!(ccso.planes.is_empty());
        let tail = core.intra_tail.as_ref().unwrap();
        assert_eq!(tail.tx_mode, TxMode::Select);
        assert_eq!(tail.reduced_tx_set, 2);
        assert!(!tail.film_grain.apply_grain);
        // 2 prefix bits + 33 control/size bits + 64 pre-filter structure bits (7 tile_info,
        // 8 base_q_idx, 25 segmentation, 13 setup_qm, 1 delta_q_present, 8 qm_index,
        // 1 allow_tcq, 1 allow_parity_hiding) + 30 loop-filter bits (7 deblocking,
        // 6 gdf, 17 cdef) + 0 lr/ccso bits (both disabled) + 3 tail bits (tx_mode_select +
        // reduced_tx_set; grain absent).
        assert_eq!(consumed, 2 + 33 + 64 + 30 + 3);
    }

    #[test]
    fn frame_header_core_eof_inside_intra_structures() {
        // The payload ends right after disable_cdf_update: the § 5.18.2 structure
        // cluster needs at least 14 more bits, so the parse reports a typed EOF.
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag
        bits.f(5, 4); // order_hint
        bits.f(1920 - 1, 12); // frame_width_minus_1
        bits.f(1080 - 1, 12); // frame_height_minus_1
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        let data = bits.into_bytes();
        let err = parse_body(&data, ObuType::ClosedLoopKey, true, &base_seq()).unwrap_err();
        assert!(matches!(err, Error::UnexpectedEof { .. }));
    }

    #[test]
    fn frame_header_core_intra_only_reads_refresh_frame_flags() {
        // Regular tile group, frame_is_inter == 0 -> INTRA_ONLY_FRAME; refresh_frame_flags
        // is read as f(NumRefFrames) (no short-refresh mode).
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.bit(0); // frame_is_inter == 0 -> INTRA_ONLY
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(0); // frame_size_override_flag == 0 (cur_mfh_id == 0 -> max dims)
        bits.f(3, 4); // order_hint
        bits.f(0b0000_0101, 8); // refresh_frame_flags f(NumRefFrames == 8)
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        // Intra structure cluster (4096x2304, 128x128 superblocks: sbCols = 32,
        // sbRows = 18, single uniform tile).
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2 = 0
        bits.bit(0); // increment_tile_rows_log2 = 0
        bits.f(45, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        // deblocking_filter_params(): not lossless -> apply[0]/[1] read (GDF/CDEF off).
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        // lr_params()/ccso_params(): restoration and CCSO disabled -> no bits.
        // § 5.18.2 tail: tx_mode_select f(1) + reduced_tx_set f(2); grain absent.
        bits.bit(0); // tx_mode_select = 0
        bits.f(0, 2); // reduced_tx_set = 0
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::RegularTileGroup, true, &base_seq()).unwrap();

        assert_eq!(core.frame_type, Some(FrameType::IntraOnly));
        assert_eq!(core.frame_is_intra, Some(true));
        assert_eq!(core.refresh_frame_flags, Some(0b0000_0101));
        assert_eq!(core.frame_size, Some(FrameSize::new(4096, 2304)));
        assert_eq!(core.quantization_params.unwrap().base_q_idx, 45);
        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        assert!(core.intra_tail.is_some());
    }

    #[test]
    fn frame_header_core_single_picture_path() {
        // single_picture_header_flag skips the frame-type/output block; frame_size uses
        // the default (max) dimensions.
        let mut seq = base_seq();
        seq.single_picture_header_flag = true;
        seq.filter.single_picture_header_flag = true;
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        // single_picture: no type/output bits; frame_size_override_flag inferred 0
        bits.f(9, 4); // order_hint
        // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        // Intra structure cluster (4096x2304 single uniform tile, see above).
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2 = 0
        bits.bit(0); // increment_tile_rows_log2 = 0
        bits.f(45, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        // deblocking_filter_params(): not lossless -> apply[0]/[1] read. GDF/CDEF are
        // disabled in base_filter, so the single-picture enable inference is not reached.
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        // lr_params()/ccso_params(): restoration and CCSO disabled -> no bits.
        // §5.18.2 tail: read_tx_mode() not lossless -> tx_mode_select f(1); reduced_tx_set
        // f(2); film_grain_config() grain absent (film_grain_params_present == false) ->
        // apply_grain inferred 0 even though single_picture_header_flag is set, since the
        // first gate (!film_grain_params_present) wins.
        bits.bit(0); // tx_mode_select = 0 -> TX_MODE_LARGEST
        bits.f(1, 2); // reduced_tx_set = 1
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();

        assert_eq!(core.show_existing_frame, Some(false));
        assert_eq!(core.frame_type, Some(FrameType::Key));
        assert_eq!(core.immediate_output_frame, Some(true));
        assert_eq!(core.implicit_output_frame, Some(false));
        assert_eq!(core.order_hint_lsb, Some(9));
        assert_eq!(core.frame_size, Some(FrameSize::new(4096, 2304)));
        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        let tail = core.intra_tail.as_ref().expect("intra tail parsed");
        assert_eq!(tail.tx_mode, TxMode::Largest);
        assert_eq!(tail.reduced_tx_set, 1);
        assert!(!tail.film_grain.apply_grain);
    }

    #[test]
    fn frame_header_core_bridge_parses_overwrite_refresh_and_size_arms() {
        // Bridge frame: cur_mfh_id inferred 0, reads seq_header_id, bridge_frame_ref_idx
        // f(CeilLog2(8) == 3), then the IsBridge reference-control arms (AV2 § 5.18.2,
        // mirror :4425-4633): bridge_frame_overwrite_flag f(1) == 0 -> refresh = 1 <<
        // bridge_frame_ref_idx (no bits), NumTotalRefs = 1 / ref_frame_idx = bridge (no
        // bits), then frame_size_with_bridge() Min(ref dims, explicit dims). The IsBridge
        // early-return arm (mirror :4971/:5045) then stops.
        let mut bits = Bits::default();
        bits.uvlc(4); // seq_header_id_in_frame_header (bridge infers cur_mfh_id == 0)
        bits.f(5, 3); // bridge_frame_ref_idx = 5
        bits.bit(0); // bridge_frame_overwrite_flag = 0 -> refresh = 1 << 5 (no bits)
        bits.f(1920 - 1, 12); // bridge_frame_width_minus_1
        bits.f(1080 - 1, 12); // bridge_frame_height_minus_1
        let data = bits.into_bytes();

        // RefFrameWidth/Height[5] modeled so frame_size_with_bridge() Min resolves.
        let mut ref_valid = [false; NUM_REF_FRAMES];
        ref_valid[5] = true;
        let ref_oh = [0u32; NUM_REF_FRAMES];
        let mut ref_w = [0u32; NUM_REF_FRAMES];
        let mut ref_h = [0u32; NUM_REF_FRAMES];
        ref_w[5] = 1280;
        ref_h[5] = 720;
        let rs = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);
        let (core, _) =
            parse_body_with_ref(&data, ObuType::BridgeFrame, true, &base_seq(), None, &rs).unwrap();

        assert!(core.is_bridge);
        assert_eq!(core.bridge_frame_ref_idx, Some(5));
        assert_eq!(core.frame_type, Some(FrameType::Inter));
        assert_eq!(core.frame_is_intra, Some(false));
        assert_eq!(core.immediate_output_frame, Some(false));
        assert_eq!(core.implicit_output_frame, Some(false));
        let inter = core.inter.as_ref().expect("bridge inter control parsed");
        assert_eq!(inter.bridge_frame_overwrite_flag, Some(false));
        assert_eq!(inter.refresh_frame_flags, Some(1 << 5));
        assert_eq!(inter.primary_ref_frame, Some(7)); // PRIMARY_REF_NONE
        assert_eq!(inter.explicit_ref_frame_map, Some(true));
        assert_eq!(inter.num_total_refs, Some(1));
        assert_eq!(inter.ref_frame_idx, vec![5]);
        // frame_size_with_bridge() Min(1280, 1920) x Min(720, 1080).
        assert_eq!(core.frame_size, Some(FrameSize::new(1280, 720)));
        // The bridge takes the IsBridge early-return arm.
        assert_eq!(inter.stop, Some(InterStop::BruInactiveOrBridgeReturn));
        assert!(matches!(
            core.status,
            FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
        ));
    }

    #[test]
    fn frame_header_core_bridge_overwrite_reads_refresh_frame_flags() {
        // Bridge frame with bridge_frame_overwrite_flag == 1 takes the else refresh arm
        // (AV2 § 5.18.2 mirror :4533): refresh_frame_flags f(NumRefFrames == 8).
        let mut bits = Bits::default();
        bits.uvlc(4); // seq_header_id_in_frame_header
        bits.f(5, 3); // bridge_frame_ref_idx = 5
        bits.bit(1); // bridge_frame_overwrite_flag = 1 -> refresh f(NumRefFrames)
        bits.f(0b1010_0101, 8); // refresh_frame_flags f(8)
        bits.f(1920 - 1, 12); // bridge_frame_width_minus_1
        bits.f(1080 - 1, 12); // bridge_frame_height_minus_1
        let data = bits.into_bytes();

        let mut ref_valid = [false; NUM_REF_FRAMES];
        ref_valid[5] = true;
        let ref_oh = [0u32; NUM_REF_FRAMES];
        let mut ref_w = [0u32; NUM_REF_FRAMES];
        let mut ref_h = [0u32; NUM_REF_FRAMES];
        ref_w[5] = 1280;
        ref_h[5] = 720;
        let rs = FrameReferenceStateView::from_slots(&ref_valid, &ref_oh, &ref_w, &ref_h);
        let (core, _) =
            parse_body_with_ref(&data, ObuType::BridgeFrame, true, &base_seq(), None, &rs).unwrap();

        let inter = core.inter.as_ref().expect("bridge inter control parsed");
        assert_eq!(inter.bridge_frame_overwrite_flag, Some(true));
        assert_eq!(inter.refresh_frame_flags, Some(0b1010_0101));
        assert_eq!(core.frame_size, Some(FrameSize::new(1280, 720)));
        assert_eq!(inter.stop, Some(InterStop::BruInactiveOrBridgeReturn));
    }

    #[test]
    fn frame_header_core_show_existing_frame_reads_map_idx_and_order_hint() {
        // Regular SEF: ShowExistingFrame == 1; reads frame_to_show_map_idx f(3),
        // derive_sef_order_hint f(1) == 0, then sef_order_hint f(OrderHintBits == 4).
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.f(6, 3); // frame_to_show_map_idx
        bits.bit(0); // derive_sef_order_hint == 0
        bits.f(11, 4); // sef_order_hint
        bits.bit(1); // § 5.2.3 trailing_one_bit; into_bytes() zero-pads the rest.
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::RegularSef, true, &base_seq()).unwrap();

        assert_eq!(core.show_existing_frame, Some(true));
        assert_eq!(core.frame_to_show_map_idx, Some(6));
        assert_eq!(core.order_hint_lsb, Some(11));
        assert_eq!(core.refresh_frame_flags, Some(0));
        assert_eq!(
            core.frame_type, None,
            "FrameType comes from reference state"
        );
        // base_seq() has film_grain_params_present == false, so film_grain_config()
        // infers apply_grain = 0 (no bit) and the SEF header completes.
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::ShowExistingFrameComplete
        );
        let fg = core.sef_film_grain.expect("SEF film_grain_config parsed");
        assert!(!fg.apply_grain);
        assert_eq!(fg.fgm_id, None);
        // A conformant grain-free SEF tail classifies as Valid (no diagnostic).
        assert_eq!(core.sef_trailing_bits, Some(SefTrailingBits::Valid));
    }

    #[test]
    fn frame_header_core_show_existing_frame_derives_order_hint() {
        // derive_sef_order_hint == 1: sef_order_hint is not read; OrderHintLsbs is
        // derived from the referenced slot (reference state), so it is left unknown.
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.f(2, 3); // frame_to_show_map_idx
        bits.bit(1); // derive_sef_order_hint == 1 -> no sef_order_hint bits
        bits.bit(1); // § 5.2.3 trailing_one_bit; into_bytes() zero-pads the rest.
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::RegularSef, true, &base_seq()).unwrap();

        assert_eq!(core.show_existing_frame, Some(true));
        assert_eq!(core.frame_to_show_map_idx, Some(2));
        assert_eq!(
            core.order_hint_lsb, None,
            "order hint is derived from the slot, not signaled"
        );
        // Grain not present -> apply_grain inferred 0, SEF header completes.
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::ShowExistingFrameComplete
        );
        assert!(core.sef_film_grain.is_some());
        assert_eq!(core.sef_trailing_bits, Some(SefTrailingBits::Valid));
    }

    #[test]
    fn frame_header_core_inter_implicit_map_stops_unmodeled() {
        // Regular tile group, frame_is_inter == 1 -> INTER_FRAME. With the sequence's
        // explicit_ref_frame_map off, explicitRefFrameMap derives 0 and get_ref_frames(0)
        // is unmodeled, so the inter region stops honestly right after the refresh flags.
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.bit(1); // frame_is_inter == 1
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(0); // frame_size_override_flag
        bits.f(5, 4); // order_hint f(OrderHintBits == 4)
        bits.bit(0); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init (not TIP)
        bits.f(0, 8); // refresh_frame_flags f(NumRefFrames == 8)
        // explicit_ref_frame_map seq flag off -> explicitRefFrameMap 0 -> get_ref_frames(0).
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::RegularTileGroup, true, &base_seq()).unwrap();

        assert_eq!(core.frame_type, Some(FrameType::Inter));
        assert_eq!(core.frame_is_intra, Some(false));
        assert_eq!(core.immediate_output_frame, Some(false));
        assert_eq!(core.order_hint_lsb, Some(5));
        assert!(matches!(
            core.status,
            FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
        ));
        let inter = core.inter.as_ref().unwrap();
        assert_eq!(inter.explicit_ref_frame_map, Some(false));
        assert_eq!(
            inter.stop,
            Some(crate::headers::frame::inter::InterStop::UnmodeledDerivation)
        );
    }

    #[test]
    fn frame_header_core_inter_explicit_map_reaches_shared_tail() {
        // Regular tile group, INTER, with the sequence explicit_ref_frame_map on: the
        // inter control region parses the explicit map, frame size, MV precision, the
        // interpolation filter, and motion modes, converging into the shared tail (the
        // core status is the unsupported-coverage class; the shared tail needs inter inputs
        // the shared cluster does not yet accept).
        let mut seq = base_seq();
        seq.inter.explicit_ref_frame_map = true;
        seq.inter.enable_ref_frame_mvs = true;
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.bit(1); // frame_is_inter == 1
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(0); // frame_size_override_flag
        bits.f(7, 4); // order_hint
        bits.bit(0); // signal_primary_ref_frame
        bits.bit(0); // disable_cross_frame_cdf_init
        bits.f(0, 8); // refresh_frame_flags
        bits.bit(1); // frame_explicit_ref_frame_map
        bits.f(1, 3); // num_total_refs = 1
        bits.f(2, 3); // ref_frame_idx[0]
        // non-override, cur_mfh_id == 0 -> frame_size() default dims (no bits).
        bits.bit(0); // use_ref_frame_mvs (num_total_refs == 1 -> no tmvp)
        bits.bit(0); // allow_intrabc
        bits.bit(0); // use_qtr_precision_mv
        bits.bit(0); // allow_high_precision_mv
        bits.bit(1); // is_filter_switchable
        // motion modes: seq_frame_motion_modes_present_flag false -> no bits.
        bits.bit(0); // disable_cdf_update f(1) (mirror :5041), just before the shared tail.
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::RegularTileGroup, true, &seq).unwrap();

        assert_eq!(core.frame_type, Some(FrameType::Inter));
        let inter = core.inter.as_ref().unwrap();
        assert_eq!(inter.explicit_ref_frame_map, Some(true));
        assert_eq!(inter.num_total_refs, Some(1));
        assert_eq!(inter.ref_frame_idx, vec![2]);
        assert_eq!(inter.frame_size, Some(FrameSize::new(4096, 2304)));
        assert_eq!(inter.mv_precision, Some(MvPrecision::HalfPel));
        assert_eq!(
            inter.interpolation_filter,
            Some(InterpolationFilter::Switchable)
        );
        assert_eq!(inter.disable_cdf_update, Some(false));
        assert_eq!(
            inter.stop,
            Some(crate::headers::frame::inter::InterStop::ReachedSharedTail)
        );
        // The core status is the unsupported-coverage class (the shared tail needs inter
        // inputs not yet threaded), never a truncation.
        assert!(!core.status.is_truncated_in_modeled_region());
    }

    #[test]
    fn frame_header_core_ras_reads_num_key_ref_frames_then_stops() {
        // RAS frame: restricted_prediction_switch f(1), then (long_term_frame_id_bits
        // != 0) num_key_ref_frames f(3) and the ref_long_term_id loop, then the inter
        // output-control flags and order_hint, before the RAS refresh derivation
        // (max_mlayer_id == 0) stops honestly (it reads RefValid / RefLongTermId).
        let mut seq = base_seq();
        seq.long_term_frame_id_bits = 4;
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.bit(0); // restricted_prediction_switch
        bits.f(2, 3); // num_key_ref_frames == 2
        bits.f(5, 4); // ref_long_term_id[0]
        bits.f(9, 4); // ref_long_term_id[1]
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        // frame_size_override_flag forced 1 for SWITCH (no bit).
        bits.f(3, 4); // order_hint f(OrderHintBits == 4)
        // RAS + max_mlayer_id == 0 -> refresh_frame_flags derivation reads RefValid (no
        // bits), stop honestly.
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::RasFrame, true, &seq).unwrap();

        assert_eq!(core.frame_type, Some(FrameType::Switch));
        assert_eq!(core.frame_is_intra, Some(false));
        assert_eq!(core.order_hint_lsb, Some(3));
        assert_eq!(core.frame_size_override_flag, Some(true));
        assert!(matches!(
            core.status,
            FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
        ));
        let inter = core.inter.as_ref().unwrap();
        assert_eq!(
            inter.stop,
            Some(crate::headers::frame::inter::InterStop::UnmodeledDerivation)
        );
        // ref_long_term_id values 5 and 9 are not the reserved (1 << 4) - 1 == 15.
        assert!(!core.forbidden_ref_long_term_id);
    }

    #[test]
    fn frame_header_core_flags_reserved_ref_long_term_id() {
        // A ref_long_term_id equal to (1 << long_term_frame_id_bits) - 1 is reserved
        // (AV2 § 6.17.2); the parser records the violation for the validator.
        let mut seq = base_seq();
        seq.long_term_frame_id_bits = 4;
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.bit(0); // restricted_prediction_switch
        bits.f(1, 3); // num_key_ref_frames == 1
        bits.f(15, 4); // ref_long_term_id[0] == (1 << 4) - 1 (reserved)
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::RasFrame, true, &seq).unwrap();
        assert!(core.forbidden_ref_long_term_id);
    }

    #[test]
    fn frame_header_core_eof_in_ref_long_term_id_loop() {
        // num_key_ref_frames == 7 (7 * 4 = 28 bits) overruns the payload, which ends
        // right after num_key_ref_frames.
        let mut seq = base_seq();
        seq.long_term_frame_id_bits = 4;
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.bit(0); // restricted_prediction_switch
        bits.f(7, 3); // num_key_ref_frames == 7; the ref_long_term_id loop overruns
        let data = bits.into_bytes();
        let err = parse_body(&data, ObuType::RasFrame, true, &seq).unwrap_err();
        assert!(matches!(err, Error::UnexpectedEof { .. }));
    }

    #[test]
    fn frame_header_core_olk_reads_long_term_ids_then_intra_tail() {
        // OLK: FrameType::Key reads long_term_id_plus_1 f(4), then (long_term_frame_id_bits
        // != 0) num_key_ref_frames f(3) + the ref_long_term_id loop, then continues into
        // the intra tail. Unlike CLK, OLK is not the `obu_type == OBU_CLOSED_LOOP_KEY`
        // allFrames case, so refresh_frame_flags is read as f(NumRefFrames) (AV2 § 5.18.2).
        let mut seq = base_seq();
        seq.long_term_frame_id_bits = 4;
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.f(1, 4); // long_term_id_plus_1
        bits.f(1, 3); // num_key_ref_frames == 1
        bits.f(3, 4); // ref_long_term_id[0]
        // immediate_output_frame: OLK forces false (no bit)
        bits.bit(0); // implicit_output_frame
        bits.bit(0); // frame_size_override_flag (cur_mfh_id == 0 -> max dims)
        bits.f(2, 4); // order_hint
        bits.f(0b0000_0101, 8); // refresh_frame_flags f(NumRefFrames == 8)
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
        // Intra structure cluster (4096x2304 single uniform tile, see above).
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(0); // increment_tile_cols_log2 = 0
        bits.bit(0); // increment_tile_rows_log2 = 0
        bits.f(45, 8); // base_q_idx
        bits.bit(0); // segmentation_enabled
        bits.bit(0); // using_qmatrix
        bits.bit(0); // delta_q_present
        // deblocking_filter_params(): not lossless -> apply[0]/[1] read (GDF/CDEF off).
        bits.bit(0); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        // lr_params()/ccso_params(): restoration and CCSO disabled -> no bits.
        // § 5.18.2 tail: tx_mode_select f(1) + reduced_tx_set f(2); grain absent.
        bits.bit(0); // tx_mode_select = 0
        bits.f(0, 2); // reduced_tx_set = 0
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::OpenLoopKey, true, &seq).unwrap();

        assert_eq!(core.frame_type, Some(FrameType::Key));
        assert_eq!(core.frame_is_intra, Some(true));
        assert_eq!(core.immediate_output_frame, Some(false));
        assert_eq!(core.implicit_output_frame, Some(false));
        assert_eq!(core.order_hint_lsb, Some(2));
        assert_eq!(core.refresh_frame_flags, Some(0b0000_0101));
        assert_eq!(core.frame_size, Some(FrameSize::new(4096, 2304)));
        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        assert!(core.intra_tail.is_some());
    }

    #[test]
    fn frame_header_core_eof_at_order_hint() {
        // Enough bits for the prefix and output flags, but order_hint f(4) overruns.
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag
        // order_hint f(4) starts here but only padding bits remain.
        let data = bits.into_bytes();
        let err = parse_body(&data, ObuType::ClosedLoopKey, true, &base_seq()).unwrap_err();
        assert!(matches!(err, Error::UnexpectedEof { .. }));
    }

    #[test]
    fn frame_header_core_eof_at_frame_size() {
        // Reaches frame_size() but the explicit width/height overruns the payload.
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.bit(0); // immediate_output_frame
        bits.bit(0); // implicit_output_frame
        bits.bit(1); // frame_size_override_flag
        bits.f(0, 4); // order_hint
        // frame_width_minus_1 f(12) starts here but the payload ends early.
        let data = bits.into_bytes();
        let err = parse_body(&data, ObuType::ClosedLoopKey, true, &base_seq()).unwrap_err();
        assert!(matches!(err, Error::UnexpectedEof { .. }));
    }

    #[test]
    fn frame_header_core_activation_prefix_mode_stops_at_prefix() {
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(1); // seq_header_id_in_frame_header
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let input = FrameHeaderParseInput {
            obu_type: ObuType::ClosedLoopKey,
            first_picture_in_tu: true,
            active_sequence: None,
            mfh_record: None,
            reference_state: FrameReferenceStateView::unknown(),
            mode: FrameHeaderParseMode::ActivationPrefix,
        };
        let core = parse_frame_header_core(&mut reader, &input).unwrap();
        assert_eq!(core.status, FrameHeaderParseStatus::ActivationFieldsOnly);
        assert_eq!(core.seq_header_id_in_frame_header, Some(1));
        assert_eq!(core.frame_type, None);
        assert_eq!(core.frame_size, None);
    }

    #[test]
    fn frame_header_core_without_sequence_is_activation_fields_only() {
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(1); // seq_header_id_in_frame_header
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let input = FrameHeaderParseInput {
            obu_type: ObuType::ClosedLoopKey,
            first_picture_in_tu: true,
            active_sequence: None,
            mfh_record: None,
            reference_state: FrameReferenceStateView::unknown(),
            mode: FrameHeaderParseMode::Core,
        };
        let core = parse_frame_header_core(&mut reader, &input).unwrap();
        assert_eq!(core.status, FrameHeaderParseStatus::ActivationFieldsOnly);
        assert_eq!(
            core.referenced_sequence_header_id,
            SequenceHeaderId::try_new(1)
        );
        assert_eq!(core.frame_type, None);
    }

    #[test]
    fn frame_header_core_eof_at_cur_mfh_id() {
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        let input = FrameHeaderParseInput {
            obu_type: ObuType::ClosedLoopKey,
            first_picture_in_tu: true,
            active_sequence: None,
            mfh_record: None,
            reference_state: FrameReferenceStateView::unknown(),
            mode: FrameHeaderParseMode::Core,
        };
        assert!(matches!(
            parse_frame_header_core(&mut reader, &input),
            Err(Error::UnexpectedEof { .. })
        ));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::headers::sequence::{LevelIdx, SuperblockSize, Tier};
    use crate::segment::{MAX_SEGMENTS, SEG_LVL_MAX, SegmentFeature, SegmentInfo};
    use crate::span::ByteOffset;
    use crate::tile::TileParams;
    use proptest::prelude::*;

    /// Arbitrary [`CoreSeqQuantView`] values across the type ranges (bit depth and
    /// plane count restricted to the AV2-legal domain, offsets unrestricted).
    fn arbitrary_quant_view() -> impl Strategy<Value = CoreSeqQuantView> {
        (
            prop_oneof![Just(8u8), Just(10u8)],
            prop_oneof![Just(1u8), Just(3u8)],
            any::<[bool; 5]>(),
            any::<[i32; 3]>(),
            any::<[bool; 3]>(),
        )
            .prop_map(
                |(bit_depth, num_planes, flags, bases, tcq)| CoreSeqQuantView {
                    bit_depth,
                    num_planes,
                    separate_uv_delta_q: flags[0],
                    equal_ac_dc_q: flags[1],
                    y_dc_delta_q_enabled: flags[2],
                    uv_dc_delta_q_enabled: flags[3],
                    uv_ac_delta_q_enabled: flags[4],
                    base_y_dc_delta_q: bases[0],
                    base_uv_dc_delta_q: bases[1],
                    base_uv_ac_delta_q: bases[2],
                    enable_tcq: tcq[0],
                    choose_tcq_per_frame: tcq[1],
                    enable_parity_hiding: tcq[2],
                },
            )
    }

    /// Arbitrary [`CoreSeqSegView`] values, including internally inconsistent ones
    /// (hostile `max_segments`, stored info without the present flag).
    fn arbitrary_seg_view() -> impl Strategy<Value = CoreSeqSegView> {
        (
            any::<[bool; 4]>(),
            any::<u8>(),
            0..MAX_SEGMENTS,
            0..SEG_LVL_MAX,
            any::<i32>(),
        )
            .prop_map(|(flags, max_segments, seg_idx, feature_idx, data)| {
                let mut features = [[SegmentFeature::DISABLED; SEG_LVL_MAX]; MAX_SEGMENTS];
                features[seg_idx][feature_idx] = SegmentFeature {
                    enabled: true,
                    data,
                };
                CoreSeqSegView {
                    seq_seg_info_present_flag: flags[0],
                    seq_allow_seg_info_change: flags[1],
                    enable_ext_seg: flags[2],
                    max_segments,
                    seq_segment_info: flags[3].then_some(SegmentInfo {
                        num_segments: max_segments.min(MAX_SEGMENTS as u8),
                        features,
                    }),
                }
            })
    }

    fn sb_size(idx: u8) -> SuperblockSize {
        match idx % 3 {
            0 => SuperblockSize::Block64x64,
            1 => SuperblockSize::Block128x128,
            _ => SuperblockSize::Block256x256,
        }
    }

    /// Arbitrary [`CoreSeqTileView`] values, including stored layouts that are
    /// ineligible, non-uniform, or absent despite the present flag.
    fn arbitrary_tile_view() -> impl Strategy<Value = CoreSeqTileView> {
        (
            any::<[bool; 4]>(),
            (0u32..=64, 0u32..=64, 0u8..=8, 0u8..=8),
            (0u32..=2048, 0u32..=2048),
            any::<[u8; 2]>(),
            (any::<bool>(), 0u8..=3, any::<bool>(), 0u8..=31),
            (
                proptest::collection::vec(0u32..=4096, 0..=64),
                proptest::collection::vec(0u32..=4096, 0..=64),
            ),
        )
            .prop_map(|(flags, counts, grid, sbs, misc, starts)| {
                let (use_256, use_128) = match sbs[0] % 3 {
                    0 => (false, false),
                    1 => (false, true),
                    _ => (true, false),
                };
                CoreSeqTileView {
                    seq_tile_info_present_flag: flags[0],
                    allow_tile_info_change: flags[1],
                    seq_tile_params: flags[2].then_some(TileParams {
                        tile_cols: counts.0,
                        tile_rows: counts.1,
                        tile_cols_log2: counts.2,
                        tile_rows_log2: counts.3,
                        sb_cols: grid.0,
                        sb_rows: grid.1,
                        uniform_spacing: flags[3],
                        covers_cols: true,
                        covers_rows: true,
                    }),
                    seq_sb_col_starts: starts.0,
                    seq_sb_row_starts: starts.1,
                    seq_sb_size: sb_size(sbs[1]),
                    use_256x256_superblock: use_256,
                    use_128x128_superblock: use_128,
                    enable_avg_cdf: misc.0,
                    avg_cdf_type: misc.1,
                    seq_tier: if misc.2 { Tier::High } else { Tier::Main },
                    seq_level_idx: LevelIdx::from_bits(misc.3),
                }
            })
    }

    /// Arbitrary `sequence_filter_config()` (§ 5.4.10) inputs consumed by the
    /// § 5.18.2 tail loop-filter cluster.
    fn arbitrary_filter_view() -> impl Strategy<Value = CoreSeqFilterView> {
        use crate::headers::sequence::CdefOnSkipTxfm;
        (
            any::<[bool; 4]>(),
            prop_oneof![
                Just(CdefOnSkipTxfm::Adaptive),
                Just(CdefOnSkipTxfm::AlwaysOn),
                Just(CdefOnSkipTxfm::Disabled),
            ],
            0u8..=3,
        )
            .prop_map(
                |(flags, skip_txfm, df_par_bits_minus_2)| CoreSeqFilterView {
                    enable_cdef: flags[0],
                    enable_gdf: flags[1],
                    gdf_unit_matches_sb_size: flags[2],
                    disable_loopfilters_across_tiles: flags[3],
                    cdef_on_skip_txfm: skip_txfm,
                    df_par_bits_minus_2,
                    single_picture_header_flag: false,
                },
            )
    }

    /// Arbitrary [`CoreSeqRestorationView`] values, with `lr_uv_pc_wiener_disabled` tied
    /// to `enable_restoration` per the § 5.4.10 inference (mirror :1382).
    fn arbitrary_restoration_view() -> impl Strategy<Value = CoreSeqRestorationView> {
        any::<[bool; 4]>().prop_map(|flags| CoreSeqRestorationView {
            enable_restoration: flags[0],
            lr_pc_wiener_disabled: flags[1],
            lr_wiener_nonsep_disabled: flags[2],
            lr_uv_pc_wiener_disabled: flags[0],
            lr_uv_wiener_nonsep_disabled: flags[3],
        })
    }

    /// Arbitrary [`CoreSeqCcsoView`] values.
    fn arbitrary_ccso_view() -> impl Strategy<Value = CoreSeqCcsoView> {
        any::<bool>().prop_map(|enable_ccso| CoreSeqCcsoView {
            enable_ccso,
            single_picture_header_flag: false,
        })
    }

    /// Arbitrary `chroma_format_idc` values (§ 5.4.1).
    fn arbitrary_chroma_format() -> impl Strategy<Value = ChromaFormatIdc> {
        prop_oneof![
            Just(ChromaFormatIdc::Yuv420),
            Just(ChromaFormatIdc::Monochrome),
            Just(ChromaFormatIdc::Yuv444),
            Just(ChromaFormatIdc::Yuv422),
        ]
    }

    /// Arbitrary [`CoreSeqView`] values within their type ranges, including the
    /// § 5.18.6 / § 5.18.7 / § 5.4.10 sub-views consumed by the new intra structure
    /// cluster.
    fn arbitrary_seq_view() -> impl Strategy<Value = CoreSeqView> {
        (
            (
                1u32..=8,
                0u32..=8,
                0u32..=5,
                any::<[bool; 3]>(),
                0u8..=2,
                (1u32..=16, 1u32..=16),
                (1u32..=65536, 1u32..=65536),
                (0u8..=2, 0u8..=2, any::<bool>()),
            ),
            arbitrary_quant_view(),
            arbitrary_seg_view(),
            arbitrary_tile_view(),
            arbitrary_filter_view(),
            arbitrary_restoration_view(),
            arbitrary_ccso_view(),
            arbitrary_chroma_format(),
        )
            .prop_map(
                |(general, quant, seg, tile, filter, restoration, ccso, chroma_format_idc)| {
                    let (
                        num_ref_frames,
                        order_hint_bits,
                        long_term_frame_id_bits,
                        flags,
                        max_mlayer_id,
                        dim_bits,
                        max_dims,
                        scc,
                    ) = general;
                    CoreSeqView {
                        num_ref_frames,
                        order_hint_bits,
                        long_term_frame_id_bits,
                        enable_short_refresh_frame_flags: flags[0],
                        monotonic_output_order_flag: flags[1],
                        single_picture_header_flag: flags[2],
                        max_mlayer_id,
                        frame_width_bits: dim_bits.0,
                        frame_height_bits: dim_bits.1,
                        max_frame_width: max_dims.0,
                        max_frame_height: max_dims.1,
                        seq_force_screen_content_tools: scc.0,
                        seq_force_integer_mv: scc.1,
                        allow_frame_max_bvp_drl_bits: scc.2,
                        inter: CoreSeqInterView {
                            enable_ref_frame_mvs: flags[0],
                            explicit_ref_frame_map: flags[1],
                            enable_bru: flags[2],
                            enable_tip: flags[0],
                            seq_max_drl_bits_minus_1: u32::from(scc.0),
                            allow_frame_max_drl_bits: scc.2,
                            enable_flex_mvres: flags[1],
                            seq_frame_motion_modes_present_flag: flags[2],
                            seq_enabled_motion_modes: [false, flags[0], flags[1], flags[2], false],
                            enable_opfl_refine: scc.1,
                        },
                        quant,
                        seg,
                        tile,
                        filter,
                        restoration,
                        ccso,
                        chroma_format_idc,
                        film_grain_params_present: Some(false),
                    }
                },
            )
    }

    /// A fixed in-band multi-frame-header record for the `cur_mfh_id > 0` never-panic
    /// property: it signals both a frame-size payload and a segment-info arm so the
    /// resolved-MFH paths are exercised. Dimensions are bounded to `seq`'s bit widths.
    /// `seq_id` is a valid `SequenceHeaderId` provided by the caller (always `Some` for
    /// `0`), so this helper itself never constructs an id.
    fn arbitrary_mfh_record(seq: &CoreSeqView, seq_id: SequenceHeaderId) -> MultiFrameHeaderRecord {
        use crate::hls::MfhFrameSize;
        let width_bits = seq.frame_width_bits.clamp(1, 16);
        let height_bits = seq.frame_height_bits.clamp(1, 16);
        let mut features = [[SegmentFeature::DISABLED; SEG_LVL_MAX]; MAX_SEGMENTS];
        features[0][0] = SegmentFeature {
            enabled: true,
            data: 1,
        };
        MultiFrameHeaderRecord {
            mfh_id: MfhId::from_raw(1),
            mfh_seq_header_id: seq_id,
            mfh_tlayer_id: crate::types::TemporalLayerId::from_bits(0),
            mfh_mlayer_id: crate::types::EmbeddedLayerId::from_bits(0),
            mfh_frame_size: Some(MfhFrameSize {
                width_bits: width_bits as u8,
                height_bits: height_bits as u8,
                width_minus_1: 0,
                height_minus_1: 0,
            }),
            mfh_seg_info_present_flag: true,
            mfh_ext_seg_flag: Some(false),
            mfh_allow_seg_info_change: Some(false),
            mfh_segment_info: Some(SegmentInfo {
                num_segments: 8,
                features,
            }),
            mfh_deblocking_filter_update: false,
            mfh_apply_deblocking_filter: [false; 4],
            offset: ByteOffset::new(0),
        }
    }

    proptest! {
        /// The frame-header core parser must never panic on arbitrary input, in either
        /// mode, with no modeled sequence state.
        #[test]
        fn parse_frame_header_core_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..64),
            raw_type in 0u8..=31,
            first_picture in any::<bool>(),
            core_mode in any::<bool>(),
        ) {
            let obu_type = ObuType::from_raw(raw_type);
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let input = FrameHeaderParseInput {
                obu_type,
                first_picture_in_tu: first_picture,
                active_sequence: None,
                mfh_record: None,
                reference_state: FrameReferenceStateView::unknown(),
                mode: if core_mode {
                    FrameHeaderParseMode::Core
                } else {
                    FrameHeaderParseMode::ActivationPrefix
                },
            };
            let _ = parse_frame_header_core(&mut reader, &input);
        }

        /// The core body — including the full § 5.18.2 intra structure cluster
        /// (tile_info, quantization, segmentation, QM setup, delta-q, lossless tail)
        /// — must never panic and never over-read for arbitrary payload bytes and
        /// arbitrary [`CoreSeqView`] values within their type ranges.
        #[test]
        fn parse_core_body_with_sequence_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..96),
            raw_type in 0u8..=31,
            first_picture in any::<bool>(),
            seq in arbitrary_seq_view(),
        ) {
            let obu_type = ObuType::from_raw(raw_type);
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            if let Ok(prefix) = parse_frame_header_prefix(&mut reader, obu_type, first_picture) {
                let mut core = init_core_from_prefix(&prefix, obu_type);
                // On a cur_mfh_id > 0 prefix, resolve against a fixed in-band MFH record
                // so the resolved-MFH paths are exercised; `SequenceHeaderId::try_new(0)`
                // is always Some (0 < MAX_SEQ_NUM).
                let mfh_view = match (core.cur_mfh_id.is_zero(), SequenceHeaderId::try_new(0)) {
                    (false, Some(seq_id)) => {
                        Some(MfhFrameView::from_record(&arbitrary_mfh_record(&seq, seq_id), &seq))
                    }
                    _ => None,
                };
                let _ = parse_core_body(
                    &mut reader,
                    &mut core,
                    &seq,
                    mfh_view.as_ref(),
                    &FrameReferenceStateView::unknown(),
                );
                prop_assert!(reader.consumed_bits() <= (data.len() as u64) * 8);
            }
        }
    }
}
