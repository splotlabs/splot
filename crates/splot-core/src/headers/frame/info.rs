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
//! - **Bridge frame** (non-single-picture) → reads `bridge_frame_ref_idx`, then parses the
//!   `IsBridge` reference-control region via [`super::inter::parse_inter_control_into`]
//!   (`bridge_frame_overwrite_flag`, the bridge `refresh_frame_flags` arms,
//!   `NumTotalRefs = 1`, `ref_frame_idx[0] = bridge_frame_ref_idx`, and
//!   `frame_size_with_bridge()` § 5.18.4.2). It then reaches the `IsBridge` early-return
//!   arm (§ 5.18.2 mirror :4971/:5045) and stops with
//!   [`super::inter::InterStop::BruInactiveOrBridgeReturn`] — the arm's `base_q_idx =
//!   RefBaseQIdx[refIdx]` / `DeltaQ` are reference-derived (no-bit) values this phase does not
//!   thread (`tile_info()` reads zero bits for a bridge, and a non-single bridge's
//!   `film_grain_config()` reads zero bits since `apply_grain == 0` when `immediate_output_frame
//!   == 0`) — recording the parsed bridge facts on `core.inter` and reporting
//!   [`FrameHeaderParseStatus::UnsupportedUntilFeature`].
//! - **Single-picture bridge** (`single_picture_header_flag == 1` + `IsBridge`) → a hybrid:
//!   the single-picture branch forces `KEY_FRAME` / `FrameIsIntra = 1` *before* the
//!   `IsBridge` `INTER_FRAME` assignment, so it travels the intra (`FrameIsIntra`) reads
//!   (`bridge_frame_overwrite_flag`, the overwrite-gated `refresh_frame_flags` per § 6.17.2 +
//!   AVM, the non-override `frame_size()`, `screen_content_params()`, `intrabc_params()`) plus
//!   the arm's decidable `film_grain_config()` (`apply_grain` inferred 1 here because
//!   `immediate_output_frame == 1`, reading `fgm_id` + `grain_seed`), then stops at the
//!   reference-derived `base_q_idx` with `BruInactiveOrBridgeReturn` — *not* the full intra
//!   structure cluster. See [`parse_single_picture_bridge_tail`].
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
//!   [`super::inter::parse_inter_control_into`]: the primary-reference signaling, the explicit
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
//!   signals `frame_filters_on`, the fixed-coded frame-level
//!   `read_wienerns_filter()` bank is parsed into `lr_params()`. A payload that runs out
//!   **inside** the loop-filter cluster (deblocking through ccso, including the bank)
//!   keeps the already-parsed control-region facts and reports the truncation as
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
use crate::headers::frame::config::{
    IntrabcParams, parse_intrabc_params_full, parse_screen_content_params_full,
};
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
    /// modeled and stored on [`super::LrPlaneParams::frame_filter_bank`], so this status is
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
    /// `RefBaseQIdx[ i ]` per reference slot (AV2 § 7.23), when modeled. This is a
    /// § 7.7 `get_ref_frames()` scoring input: with **two or more** valid reference
    /// slots the implicit reference-map ranking depends on it, so the inter parser
    /// can only lift its at-most-one-valid-slot gate once the caller supplies it.
    /// `None` (the [`Self::from_slots`] constructor) keeps the historical
    /// at-most-one-valid-slot behavior — the unmodeled `RefBaseQIdx` makes a
    /// multi-valid-slot derivation an honest `UnmodeledDerivation` stop.
    pub ref_base_q_idx: Option<&'a [u32]>,
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
            ref_base_q_idx: None,
        }
    }

    /// Builds a reference state from the caller's modeled `RefValid[]` / `RefOrderHint[]`
    /// / `RefFrameWidth[]` / `RefFrameHeight[]` slices (AV2 § 7.23), with `RefBaseQIdx`
    /// unmodeled.
    ///
    /// The slices are parallel, one entry per reference slot. The caller (the validator's
    /// § 7.23 buffer model) owns the backing storage; the view borrows it for the parse.
    /// A `cur_mfh_id == 0` / intra parse does not read these today — the constructor is
    /// the forward-plumbing entry point for the § 5.18 inter reference paths.
    ///
    /// Because `RefBaseQIdx` is left unmodeled, a § 7.7 derivation that finds **two or
    /// more** valid slots stays an honest `UnmodeledDerivation` stop (the ranking needs
    /// the per-slot quantizer). Use [`Self::from_slots_with_base_q_idx`] when the caller
    /// models `RefBaseQIdx` and needs the multi-valid-slot ranking.
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
            ref_base_q_idx: None,
        }
    }

    /// Builds a reference state that additionally models `RefBaseQIdx[]` (AV2 § 7.23),
    /// the per-slot quantizer the § 7.7 `get_ref_frames()` ranking scores.
    ///
    /// The five slices are parallel, one entry per reference slot. Supplying
    /// `RefBaseQIdx` lets the inter parser run the § 7.7 ranking over **two or more**
    /// valid reference slots (the multi-reference case): every other § 7.7 scoring input
    /// is deterministic for the single-spatial-layer minimal frame (`AllowedFrames == -1`,
    /// all layers depend). The committed fixtures refresh one slot per frame, so the
    /// per-slot `RefCounter`s are naturally distinct; and even were two slots to hold one
    /// frame (a shared `RefCounter`, e.g. `refresh_frame_flags` with multiple bits), the
    /// § 7.7 `new_score_or_dist` step still drops the duplicate by identical
    /// `(orderHint, score, mLayer)`. So with `RefBaseQIdx` modeled the derivation is exact
    /// rather than an `UnmodeledDerivation` stop.
    #[must_use]
    pub const fn from_slots_with_base_q_idx(
        ref_valid: &'a [bool],
        ref_order_hint: &'a [u32],
        ref_frame_width: &'a [u32],
        ref_frame_height: &'a [u32],
        ref_base_q_idx: &'a [u32],
    ) -> Self {
        Self {
            ref_valid: Some(ref_valid),
            ref_order_hint: Some(ref_order_hint),
            ref_frame_width: Some(ref_frame_width),
            ref_frame_height: Some(ref_frame_height),
            ref_base_q_idx: Some(ref_base_q_idx),
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
    /// `disable_cdf_update`, when reached on the intra or non-intra control path.
    pub disable_cdf_update: Option<bool>,
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
    /// `force_integer_mv` from `screen_content_params()` (§ 5.18.3.3), when reached. The
    /// modeled intra path derives nothing from it (`FrameIsIntra` skips the MV-precision
    /// block), but it is surfaced so the § 5.18.3.3 writer can reproduce the bit exactly.
    pub force_integer_mv: Option<bool>,
    /// `allow_intrabc`, when `intrabc_params()` was reached.
    pub allow_intrabc: Option<bool>,
    /// The full `intrabc_params()` (§ 5.18.3.4) record, when reached. Surfaces the
    /// conditionally-read `allow_global_intrabc` / `allow_local_intrabc` / `change_bvp_drl` /
    /// `max_bvp_drl_bits_minus_1` bits (which the modeled path derives nothing from) so the
    /// § 5.18.3.4 writer can reproduce them byte-for-byte. `allow_intrabc` is mirrored on the
    /// flatter [`Self::allow_intrabc`] field above for existing consumers.
    pub intrabc: Option<IntrabcParams>,
    /// `true` if any `ref_long_term_id[i]` equals the reserved value
    /// `(1 << long_term_frame_id_bits) - 1`, which AV2 § 6.17.2 forbids.
    pub forbidden_ref_long_term_id: bool,
    /// `restricted_prediction_switch` (AV2 § 5.18.2, mirror :4256), the `f(1)` read on
    /// the SWITCH / RAS frame-type arm. `Some(true)`/`Some(false)` when the bit was read
    /// (a SWITCH or RAS frame); `None` on every other frame type (the bit is not present).
    /// AV2 § 7.3.8.9: the OBU_SWITCH quantizer-matrix-level reset only applies when this is
    /// `1`. AV2 § 7.4.5: the RAS OrderHint bound applies only when this is `0` (a residual —
    /// the unwrapped OrderHint is not header-decidable).
    pub restricted_prediction_switch: Option<bool>,
    /// `LongTermId` for this frame (AV2 § 5.18.2, mirror :4231-4239): for a KEY frame when
    /// `long_term_frame_id_bits > 0` it is `long_term_id_plus_1` minus one, else `-1`. `None`
    /// when the frame type was not derived or the read did not reach this point. The § 7.23
    /// reference frame update process stores this as `RefLongTermId[i]` for the refreshed
    /// slots (mirror :14113), so a KEY frame's value becomes the long-term id of those slots.
    pub long_term_id: Option<i64>,
    /// `ref_long_term_id[0..num_key_ref_frames]` (AV2 § 5.18.2, mirror :4243-4253), the
    /// long-term ids a RAS / OLK frame lists. Empty when not a RAS / OLK frame, when
    /// `long_term_frame_id_bits == 0`, or when `num_key_ref_frames == 0`. AV2 § 6.17.2
    /// (mirror :4615-4616): a RAS frame's `ref_frame_idx[i]` must select a slot whose
    /// `RefLongTermId` is `long_term_id_in_use` in this list.
    pub ref_long_term_ids: Vec<u32>,
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
    /// `cdef_params()`) **and parsed to completion**. If a plane signals the fixed-coded
    /// frame-level Wiener NS bank, that bank is part of this complete value via
    /// [`super::LrPlaneParams::frame_filter_bank`]. `None` when the parse stopped before
    /// `lr_params()` or in a reserved unsupported branch.
    pub lr_params: Option<LrParams>,
    /// Partial `lr_params()` facts committed before a reserved
    /// [`FrameHeaderParseStatus::StoppedBeforeWienerNsFilter`] coverage stop (AV2
    /// § 5.18.7.11). The fixed-coded frame-level Wiener NS bank is now modeled, so this is
    /// retained for out-of-tree compatibility and future unsupported branches. When set it
    /// is mutually exclusive with [`Self::lr_params`], preserving the distinction between a
    /// complete and partial `lr_params()` parse.
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
    /// The parsed § 5.18.2 inter-tail coding-mode arms (mirror :5307-5341): `read_tx_mode()`,
    /// `frame_reference_mode()`'s `reference_select`, `skip_mode_params()`'s
    /// `skip_mode_present`, the gated `allow_bawp` / `allow_warpmv_mode`, `reduced_tx_set`,
    /// `global_motion_params()`'s `use_global_motion`, and `film_grain_config()`'s
    /// `apply_grain`. `Some` only when the inter path parsed to completion
    /// ([`FrameHeaderParseStatus::InterHeaderComplete`]).
    pub inter_tail: Option<crate::headers::frame::inter_shared_tail::InterTail>,
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
    /// `true` once the parse passes the § 5.18.2 `reset_qm()` call site (AV2 mirror
    /// `05-syntax-structures.md` :4279-4283) with its trigger condition satisfied — i.e. the
    /// parse reached the point AFTER `restricted_prediction_switch`, the
    /// `num_key_ref_frames` / `ref_long_term_id[i]` list, and the SWITCH-restricted output
    /// flush, with `obu_type == OBU_RAS_FRAME || (obu_type == OBU_SWITCH &&
    /// restricted_prediction_switch)`. This is an explicit "reached reset_qm" fact: it stays
    /// `true` even when the parse later truncates inside the inter control region (the
    /// facts-preserving [`FrameHeaderParseStatus::StoppedInsideInterControl`] keeps the core),
    /// so a consumer can confirm the § 7.3.8.9 quantizer-matrix availability reset from the
    /// parsed bits alone rather than requiring the whole core parse to complete (codex F2).
    /// `false` for every frame type whose `reset_qm()` trigger is not met, and for a RAS /
    /// restricted SWITCH whose parse stops BEFORE the call site (truncated mid-prefix or
    /// mid-`ref_long_term_id` — the reset is then unconfirmed).
    pub reached_qm_reset: bool,
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
///
/// Public so a [`CoreSeqView`] (a writer input) is constructible outside `info`; the intra
/// frame-header writer does not consume these inter flags but they are part of the view.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct CoreSeqInterView {
    pub(crate) enable_ref_frame_mvs: bool,
    pub(crate) explicit_ref_frame_map: bool,
    pub(crate) enable_bru: bool,
    pub(crate) enable_tip: bool,
    pub(crate) seq_max_drl_bits_minus_1: u32,
    pub(crate) allow_frame_max_drl_bits: bool,
    pub(crate) enable_flex_mvres: bool,
    pub(crate) seq_frame_motion_modes_present_flag: bool,
    pub(crate) seq_enabled_motion_modes: [bool; MOTION_MODES],
    pub(crate) enable_opfl_refine: u8,
    /// `enable_bawp` (AV2 § 5.4.6): gates the § 5.18.2 inter-tail `allow_bawp` `f(1)`
    /// read (`!FrameIsIntra && enable_bawp`, mirror :5313).
    pub(crate) enable_bawp: bool,
    /// `enable_global_motion` (AV2 § 5.4.6): gates `global_motion_params()`'s inter arm
    /// (`!FrameIsIntra && enable_global_motion`, § 5.18.9.1 mirror :7792).
    pub(crate) enable_global_motion: bool,
}

/// Sequence-derived scalars the core parser needs, gathered from a fully parsed
/// [`SequenceHeader`]. `None` when any required child config (partition, segment,
/// inter, screen-content, transform/quant/entropy, or tile) is absent — the header
/// was not fully parsed — in which case core parsing degrades to the prefix.
///
/// The § 5.18.6 / § 5.18.7 inputs are grouped into per-structure sub-views
/// ([`CoreSeqQuantView`], [`CoreSeqSegView`], [`CoreSeqTileView`]) so each child
/// parser names exactly the state it consumes.
///
/// Public (crate-private fields) so the [`crate::write`] frame-header writer can take a
/// `&CoreSeqView` and read the sequence state it needs to invert `parse_frame_header_core`;
/// external callers build one via [`CoreSeqView::from_sequence`] and treat it as opaque.
#[derive(Debug)]
#[non_exhaustive]
pub struct CoreSeqView {
    pub(crate) num_ref_frames: u32,
    pub(crate) order_hint_bits: u32,
    pub(crate) long_term_frame_id_bits: u32,
    pub(crate) enable_short_refresh_frame_flags: bool,
    pub(crate) monotonic_output_order_flag: bool,
    pub(crate) single_picture_header_flag: bool,
    pub(crate) max_mlayer_id: u8,
    pub(crate) frame_width_bits: u32,
    pub(crate) frame_height_bits: u32,
    pub(crate) max_frame_width: u32,
    pub(crate) max_frame_height: u32,
    pub(crate) seq_force_screen_content_tools: u8,
    pub(crate) seq_force_integer_mv: u8,
    pub(crate) allow_frame_max_bvp_drl_bits: bool,
    /// § 5.4.6 inter-config inputs consumed by the § 5.18.2 non-intra control region
    /// ([`crate::headers::frame::inter`]).
    pub(crate) inter: CoreSeqInterView,
    /// § 5.18.6 / § 5.18.7.8 / § 5.18.2-lossless-tail inputs (AV2 § 5.4.8).
    pub(crate) quant: CoreSeqQuantView,
    /// § 5.18.7.1 segmentation inputs (AV2 § 5.4.4).
    pub(crate) seg: CoreSeqSegView,
    /// § 5.18.7.2 tile-info inputs (AV2 § 5.4.2 / § 5.4.3 / § 5.4.8).
    pub(crate) tile: CoreSeqTileView,
    /// § 5.18.5.2 / § 5.18.7.9 / § 5.18.7.10 loop-filter inputs (AV2 § 5.4.10).
    pub(crate) filter: CoreSeqFilterView,
    /// § 5.18.7.11 loop-restoration tool flags (AV2 § 5.4.10).
    pub(crate) restoration: CoreSeqRestorationView,
    /// § 5.18.7.12 CCSO inputs (AV2 § 5.4.10 / § 5.4.1).
    pub(crate) ccso: CoreSeqCcsoView,
    /// `chroma_format_idc` (AV2 § 5.4.1): the § 6.4.1 SubsamplingX/Y for `lr_params()`'s
    /// chroma `LoopRestorationSize` derivation.
    pub(crate) chroma_format_idc: ChromaFormatIdc,
    /// `film_grain_params_present` (AV2 § 5.4.1): gates the § 5.18.10.1
    /// `film_grain_config()` `apply_grain` derivation. `Some(false)` when the sequence
    /// header did not signal grain, `Some(true)` when it did. `None` when the active
    /// sequence header was recorded from a **bounded** stop that ended before
    /// `film_grain_params_present` (read last in § 5.4.1, after the child configs), e.g.
    /// the bounded `sequence_tile_config()` residual: the flag is then genuinely unknown.
    /// The control region (frame size, output flags, order hint, tile/quant/segmentation)
    /// does not consume this flag, so the parser still reaches and reports those facts; it
    /// stops honestly only when `film_grain_config()` itself needs the unknown flag.
    pub(crate) film_grain_params_present: Option<bool>,
}

impl CoreSeqView {
    /// Gathers the sequence-derived state the frame-header core parse — and the inverse
    /// [`crate::write`] frame-header writer — need from a fully parsed [`SequenceHeader`]
    /// (AV2 v1.0.0 § 5.4.1). Returns `None` when any required child config is absent (the
    /// header was not fully parsed), so neither side operates on a partial sequence header.
    #[must_use]
    pub fn from_sequence(seq: &SequenceHeader) -> Option<Self> {
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
                enable_bawp: inter.enable_bawp,
                enable_global_motion: inter.enable_global_motion,
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
                // AV2 § 5.4.6: enable_df_sub_pu lives in the inter config; it gates the
                // § 5.18.5.2 inter-path allow_df_sub_pu read (inert on the intra path).
                enable_df_sub_pu: inter.enable_df_sub_pu,
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
/// `cur_mfh_id == 0` direct path the parser keeps `None` and uses sequence state. Public
/// (crate-private fields) so the [`crate::write`] frame-header writer can take an
/// `Option<&MfhFrameView>` and invert the `cur_mfh_id > 0` arms; build via
/// [`MfhFrameView::from_record`].
#[derive(Debug)]
#[non_exhaustive]
pub struct MfhFrameView {
    /// `(FrameWidth, FrameHeight)` default dimensions for the § 5.18.4.1 non-override
    /// path: `mfh_frame_width/height_minus_1[ cur_mfh_id ] + 1`, with the § 5.18.2
    /// omitted-size inference (:4101) already applied — when the MFH carried no
    /// frame-size payload, these equal the sequence `max_frame_width/height`.
    pub(crate) default_dims: (u32, u32),
    /// The § 5.18.7.1 MFH-gated segmentation inputs, `Some` only when
    /// `mfh_seg_info_present_flag` is set (the gate selecting the MFH branch).
    pub(crate) seg: Option<MfhSegView>,
    /// The § 5.18.5.2 MFH deblocking-update inputs: `mfh_deblocking_filter_update`
    /// and `mfh_apply_deblocking_filter[0..4]` (AV2 § 5.7), consulted by the
    /// `cur_mfh_id > 0` deblocking arm (mirror :5949).
    pub(crate) deblocking: MfhDeblockingView,
}

impl MfhFrameView {
    /// Resolves a [`MultiFrameHeaderRecord`]'s § 5.7 state against the active
    /// sequence header's maxima for the `cur_mfh_id > 0` core path (AV2 § 5.18.2),
    /// shared by the parser and the inverse [`crate::write`] frame-header writer.
    #[must_use]
    pub fn from_record(record: &MultiFrameHeaderRecord, seq: &CoreSeqView) -> Self {
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
    let prefix =
        parse_frame_header_prefix(reader, input.obu_type, Some(input.first_picture_in_tu))?;
    let mut core = init_core_from_prefix(&prefix, input.obu_type, input.first_picture_in_tu);

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
///
/// `first_picture_in_tu` is the known stateful `FirstPictureInTU`; the core's
/// `starts_cvs` is derived directly from it and `obu_type` per AV2 § 5.18.2
/// (`startCVS = obu_type == OBU_CLOSED_LOOP_KEY && FirstPictureInTU`), so the core
/// always carries a concrete `bool` and never unwraps the prefix's `Option` (the
/// prefix may be `None` only on the stateless front door, which does not reach here).
pub(crate) fn init_core_from_prefix(
    prefix: &FrameHeaderPrefix,
    obu_type: ObuType,
    first_picture_in_tu: bool,
) -> FrameHeaderCore {
    FrameHeaderCore {
        obu_type,
        status: FrameHeaderParseStatus::ActivationFieldsOnly,
        is_first: prefix.is_first,
        is_key_frame: prefix.is_key_frame,
        is_regular: prefix.is_regular,
        is_bridge: prefix.is_bridge,
        starts_cvs: obu_type == ObuType::ClosedLoopKey && first_picture_in_tu,
        cur_mfh_id: prefix.cur_mfh_id,
        seq_header_id_in_frame_header: prefix.seq_header_id_in_frame_header,
        referenced_sequence_header_id: prefix.referenced_sequence_header_id,
        show_existing_frame: None,
        frame_type: None,
        frame_is_intra: None,
        immediate_output_frame: None,
        implicit_output_frame: None,
        disable_cdf_update: None,
        order_hint_lsb: None,
        refresh_frame_flags: None,
        frame_size: None,
        frame_size_override_flag: None,
        bridge_frame_ref_idx: None,
        frame_to_show_map_idx: None,
        allow_screen_content_tools: None,
        force_integer_mv: None,
        allow_intrabc: None,
        intrabc: None,
        forbidden_ref_long_term_id: false,
        restricted_prediction_switch: None,
        long_term_id: None,
        ref_long_term_ids: Vec::new(),
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
        inter_tail: None,
        sef_film_grain: None,
        sef_trailing_bits: None,
        inter: None,
        reached_qm_reset: false,
        consumed_bits: 0,
    }
}

/// Parses `frame_header_info()` beyond the activation prefix (AV2 § 5.18.2), setting
/// `core`'s fields and stop [`FrameHeaderParseStatus`]. The reader starts positioned
/// just after the activation/reference fields.
pub(crate) fn parse_core_body(
    reader: &mut BitReader<'_>,
    core: &mut FrameHeaderCore,
    seq: &CoreSeqView,
    mfh: Option<&MfhFrameView>,
    reference_state: &FrameReferenceStateView<'_>,
) -> Result<()> {
    let obu_type = core.obu_type;

    // AV2 § 5.18.2 (mirror :4117-4123): the `if ( IsBridge )` read of bridge_frame_ref_idx
    // f(CeilLog2(NumRefFrames)) runs immediately after load_sequence_header() — BEFORE the
    // `if ( single_picture_header_flag )` branch (mirror :4131). So a bridge frame ALWAYS
    // reads bridge_frame_ref_idx, but whether it then takes the bridge inter path or the
    // single-picture path depends on single_picture_header_flag (codex F5).
    let bridge_frame_ref_idx = if core.is_bridge {
        let idx = reader.read_f(ceil_log2(seq.num_ref_frames))?;
        core.bridge_frame_ref_idx = Some(idx);
        Some(idx)
    } else {
        None
    };

    if seq.single_picture_header_flag {
        // AV2 § 5.18.2 (mirror :4131-4142): single_picture_header_flag forces a key frame and
        // skips the entire show-existing / frame-type / output-control block (including the
        // bridge's INTER_FRAME / immediate_output_frame = 0 assignments at mirror :4203-4205
        // / :4295-4313). This applies to a bridge frame too (it already read
        // bridge_frame_ref_idx above): the single-picture branch comes BEFORE the `if (
        // IsBridge ) FrameType = INTER_FRAME` else-arm, so a single-picture bridge becomes a
        // KEY_FRAME with FrameIsIntra = 1 / immediate_output_frame = 1 — NOT an inter bridge.
        core.show_existing_frame = Some(false);
        core.frame_type = Some(FrameType::Key);
        core.frame_is_intra = Some(true);
        core.immediate_output_frame = Some(true);
        core.implicit_output_frame = Some(false);
        // A single-picture bridge is a HYBRID, not the plain intra key path: FrameIsIntra == 1
        // and IsBridge == 1 hold together, so it still reads `bridge_frame_overwrite_flag` (mirror
        // :4423) and reaches the § 5.18.2 `IsBridge` early-return arm (:4971/:5045) instead of the
        // full intra structure cluster. Route it to the dedicated bridge tail, which reads exactly
        // the modeled prefix and stops at that arm. parse_intra_tail's invariant (TipFrameMode ==
        // TIP_FRAME_DISABLED and !IsBridge) does NOT hold here, so it must not be used.
        if let Some(bridge_frame_ref_idx) = bridge_frame_ref_idx {
            return parse_single_picture_bridge_tail(reader, core, seq, bridge_frame_ref_idx);
        }
        return parse_intra_tail(reader, core, seq, mfh, FrameType::Key, true);
    }

    // A non-single-picture bridge takes the IsBridge inter arm: FrameType = INTER_FRAME
    // (mirror :4203-4205), immediate_output_frame / implicit_output_frame = 0 (mirror
    // :4295-4313), and frame_size_override_flag / order_hint are NOT read (the bridge skips
    // the non-bridge :4351+ block), so it enters the inter control region directly.
    if let Some(bridge_frame_ref_idx) = bridge_frame_ref_idx {
        core.frame_type = Some(FrameType::Inter);
        core.frame_is_intra = Some(false);
        core.immediate_output_frame = Some(false);
        core.implicit_output_frame = Some(false);
        return parse_bridge_inter_path(reader, core, seq, bridge_frame_ref_idx, reference_state);
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
        // not compute here, so its value does not change any modeled bit position; it IS
        // recorded for the validator's § 7.3.8.9 OBU_SWITCH quantizer-matrix reset gate
        // (the reset applies only when restricted_prediction_switch == 1).
        core.restricted_prediction_switch = Some(reader.read_flag()?);
        FrameType::Switch
    } else if obu_type.is_tip_frame() {
        FrameType::Inter
    } else if obu_type == ObuType::ClosedLoopKey || obu_type == ObuType::OpenLoopKey {
        FrameType::Key
    } else {
        let frame_is_inter = reader.read_flag()?; // frame_is_inter f(1)
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
    //
    // mirror :4231-4239: `LongTermId = -1`, then for a KEY frame
    // `LongTermId = long_term_id_plus_1 - 1`. When `long_term_frame_id_bits == 0` the
    // `f(0)` read yields `long_term_id_plus_1 == 0`, so `LongTermId == -1` even for a KEY
    // frame — the `-1` "not a long-term frame" sentinel. The § 7.23 update stores this as
    // `RefLongTermId[i]` for the refreshed slots (mirror :14113).
    core.long_term_id = Some(-1);
    if frame_type == FrameType::Key {
        let long_term_id_plus_1 = reader.read_f(seq.long_term_frame_id_bits)?;
        core.long_term_id = Some(i64::from(long_term_id_plus_1) - 1);
    }
    if (obu_type == ObuType::RasFrame || obu_type == ObuType::OpenLoopKey)
        && seq.long_term_frame_id_bits != 0
    {
        // AV2 § 6.17.2: every ref_long_term_id[i] must differ from the reserved
        // (1 << long_term_frame_id_bits) - 1; record a violation for the validator.
        let reserved_long_term_id = (1u32 << seq.long_term_frame_id_bits).wrapping_sub(1);
        let num_key_ref_frames = reader.read_bits(3)?;
        let mut ref_long_term_ids = Vec::with_capacity(num_key_ref_frames as usize);
        for _ in 0..num_key_ref_frames {
            let ref_long_term_id = reader.read_f(seq.long_term_frame_id_bits)?;
            if ref_long_term_id == reserved_long_term_id {
                core.forbidden_ref_long_term_id = true;
            }
            // Recorded for the validator's § 6.17.2 RAS `long_term_id_in_use` check
            // (mirror :4615-4616) and `long_term_id_in_use()` (mirror :5529-5536).
            ref_long_term_ids.push(ref_long_term_id);
        }
        core.ref_long_term_ids = ref_long_term_ids;
    }

    // AV2 § 5.18.2 reset_qm() call site (mirror :4279-4283): the parse has now passed
    // `restricted_prediction_switch`, the `num_key_ref_frames` / `ref_long_term_id[i]` list,
    // and the SWITCH-restricted output flush — the exact point the spec calls `reset_qm()`,
    // BEFORE the output-control flags below and the inter control region. Record an explicit
    // "reached reset_qm with its trigger met" fact so a consumer can confirm the § 7.3.8.9
    // quantizer-matrix availability reset from the parsed bits even when the parse later
    // truncates inside the inter control region (codex F2). The trigger is exactly the spec's
    // `obu_type == OBU_RAS_FRAME || (obu_type == OBU_SWITCH && restricted_prediction_switch)`;
    // for a SWITCH the gate reads `restricted_prediction_switch` (set above on the SWITCH /
    // RAS frame-type arm), so an unread gate (`None`) leaves the fact `false` (unconfirmed).
    core.reached_qm_reset = obu_type == ObuType::RasFrame
        || (obu_type == ObuType::Switch && core.restricted_prediction_switch == Some(true));

    // AV2 § 5.18.2 output control (mirror :4295-4313). This block is in the non-SEF,
    // non-single-picture branch and applies to BOTH intra and inter frames. A bridge
    // frame already returned above; here `obu_type` is never OBU_BRIDGE_FRAME, so the
    // gate reduces to the OLK / monotonic-output checks.
    let immediate_output_frame = if obu_type == ObuType::OpenLoopKey {
        false
    } else {
        reader.read_flag()?
    };
    core.immediate_output_frame = Some(immediate_output_frame);
    let implicit_output_frame = if immediate_output_frame || seq.monotonic_output_order_flag {
        false
    } else {
        reader.read_flag()?
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

/// Parses the non-intra `frame_header_info()` path (AV2 § 5.18.2, mirror :4351-5343):
/// `frame_size_override_flag`, `order_hint`, the reference control region via
/// [`parse_inter_control_into`](crate::headers::frame::inter::parse_inter_control_into), and —
/// when that region reaches
/// [`InterStop::ReachedSharedTail`](crate::headers::frame::inter::InterStop) — the § 5.18.2
/// shared tail via
/// [`parse_inter_shared_tail`](crate::headers::frame::inter_shared_tail::parse_inter_shared_tail).
///
/// On `ReachedSharedTail` the shared tail parses `tile_info()` → `quantization_params()` →
/// `segmentation_params()` → … → the inter coding-mode arms → `film_grain_config()` for the
/// modeled minimal-tool single-reference inter subset, reaching the terminal
/// [`FrameHeaderParseStatus::InterHeaderComplete`] (or an honest
/// [`FrameHeaderParseStatus::UnsupportedUntilFeature`] for anything outside that subset). On
/// any other [`InterStop`](crate::headers::frame::inter::InterStop) the parse stops in the
/// control region with the unsupported-coverage status; the distinct stop class stays on
/// `core.inter.stop`. The inter facts are recorded on `core.inter` either way.
fn parse_inter_path(
    reader: &mut BitReader<'_>,
    core: &mut FrameHeaderCore,
    seq: &CoreSeqView,
    frame_type: FrameType,
    reference_state: &FrameReferenceStateView<'_>,
) -> Result<()> {
    use crate::headers::frame::inter::{InterFrameContext, InterStop, parse_inter_control_into};
    use crate::headers::frame::inter_shared_tail::parse_inter_shared_tail;

    let obu_type = core.obu_type;

    // The control region's facts accumulate in a caller-owned `control` so an EOF inside a
    // modeled field preserves the fields parsed before it (codex F2).
    let mut control = crate::headers::frame::inter::InterControl::default();

    // `shared_tail_ran` records whether the shared-tail parser set `core.status` itself
    // (the `ReachedSharedTail` continuation); `finish_inter_control` must then leave the
    // status untouched on `Ok`.
    let mut shared_tail_ran = false;

    let result = (|| -> Result<()> {
        // mirror :4353-4365: frame_size_override_flag. SWITCH_FRAME forces 1 (no bit);
        // single_picture_header_flag forces 0; otherwise f(1). The inter path is never a
        // single-picture frame (that path is intra-only above), so the gate is SWITCH vs read.
        let frame_size_override_flag = if frame_type == FrameType::Switch {
            true
        } else {
            reader.read_flag()?
        };
        core.frame_size_override_flag = Some(frame_size_override_flag);

        // mirror :4367: order_hint f(OrderHintBits); OrderHintLsbs = order_hint.
        let order_hint = reader.read_f(seq.order_hint_bits)?;
        core.order_hint_lsb = Some(order_hint);

        let inter_seq = build_inter_seq_view(seq);
        let ctx = InterFrameContext {
            obu_type,
            frame_type,
            is_bridge: false, // bridge frames take parse_bridge_inter_path, not this path
            bridge_frame_ref_idx: None,
            cur_mfh_id_is_zero: core.cur_mfh_id.is_zero(),
            order_hint,
        };

        parse_inter_control_into(
            reader,
            &inter_seq,
            &ctx,
            reference_state,
            frame_size_override_flag,
            &mut control,
        )?;

        // mirror :5183: when the control region converged on the shared tail, continue into
        // it. The shared tail reads from `control` (NumTotalRefs / ref_frame_idx /
        // frame_enabled_motion_modes) and the already-set `core` output flags / frame size;
        // it sets `core.status` itself (InterHeaderComplete or an honest coverage stop). An
        // EOF inside the modeled tail propagates out of this closure and
        // `finish_inter_control` converts it to StoppedInsideInterControl with the facts
        // parsed so far preserved. The control facts are lifted onto `core.inter` AFTER the
        // tail runs (it borrows `control`), so a non-tail-EOF still preserves them.
        if control.stop == Some(InterStop::ReachedSharedTail) {
            // The shared tail's tile_info() needs the reference-grounded FrameWidth/Height
            // (the control region resolved it on `control.frame_size`); lift it onto `core`
            // BEFORE the tail runs so the tile/GDF/LR geometry derivations see it. (The
            // refresh-flags / disable_cdf_update lift stays in finish_inter_control_with_tail;
            // only the size is read by the shared tail.) When the size is genuinely unknown
            // (a hit on an unmodeled ref slot), it stays None and the shared tail stops
            // honestly at its own frame_size guard.
            core.frame_size = control.frame_size;
            shared_tail_ran = true;
            parse_inter_shared_tail(reader, core, seq, &control, frame_type)?;
        }
        Ok(())
    })();

    finish_inter_control_with_tail(core, control, result, shared_tail_ran)
}

/// Records a parsed inter / bridge `control` onto `core` and sets the terminal status,
/// converting an [`Error::UnexpectedEof`] inside the modeled § 5.18.2 control region into a
/// facts-preserving truncation status (codex F2):
///
/// - `Ok(())`: the control region reached one of its modeled coverage stops. The facts are
///   preserved on `core.inter`; the distinct stop class lives in `control.stop`. The core
///   status is the unsupported-coverage class
///   ([`FrameHeaderParseStatus::UnsupportedUntilFeature`]) — never a truncation: the shared
///   structure cluster past the control region is unmodeled by construction.
/// - `Err(UnexpectedEof)`: the payload ran out inside a mandated control field. The fields
///   parsed before the EOF are intact on `control` and lifted onto `core.inter`; the core
///   status is [`FrameHeaderParseStatus::StoppedInsideInterControl`], which the validator's
///   `is_truncated_in_modeled_region()` partition routes to `frame-header/truncated-frame-
///   header`. Without this the `Err` would propagate out of `parse_frame_header_core` and the
///   validator's `.ok()` would drop ALL facts and the truncation (the PR #57/#59 regression
///   class).
/// - Any other `Err`: a genuine malformed-input error propagates unchanged.
fn finish_inter_control(
    core: &mut FrameHeaderCore,
    control: crate::headers::frame::inter::InterControl,
    result: Result<()>,
) -> Result<()> {
    // Lift the inter reference-grounded frame size / refresh flags onto the core so existing
    // state-supported diagnostics and the inspector see whatever parsed before any EOF.
    if let Some(size) = control.frame_size {
        core.frame_size = Some(size);
    }
    if let Some(flags) = control.refresh_frame_flags {
        core.refresh_frame_flags = Some(flags);
    }
    core.disable_cdf_update = control.disable_cdf_update;
    core.inter = Some(control);

    match result {
        Ok(()) => {
            core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
                feature_id: FRAME_HEADER_INFO_FEATURE,
            };
            Ok(())
        }
        // EOF inside the modeled § 5.18.2 control region: a payload-bounds truncation, not a
        // hard parse error. Keep the preserved facts and surface the truncation status.
        Err(Error::UnexpectedEof { .. }) => {
            core.status = FrameHeaderParseStatus::StoppedInsideInterControl;
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// Like [`finish_inter_control`], but for the non-bridge inter path that may have continued
/// past `InterStop::ReachedSharedTail` into the § 5.18.2 shared tail
/// ([`parse_inter_shared_tail`](crate::headers::frame::inter_shared_tail::parse_inter_shared_tail)).
///
/// `shared_tail_ran` is `true` when the shared-tail parser was invoked (the control region
/// reached `ReachedSharedTail`). In that case the shared-tail parser already set `core.status`
/// (the terminal [`FrameHeaderParseStatus::InterHeaderComplete`], an honest
/// [`FrameHeaderParseStatus::UnsupportedUntilFeature`] coverage stop, or a reserved
/// [`FrameHeaderParseStatus::StoppedBeforeWienerNsFilter`] branch), so on `Ok` the status is left
/// untouched. When `shared_tail_ran` is `false` (any other control-region stop) the status
/// is set to the unsupported-coverage class exactly as [`finish_inter_control`] does. An
/// [`Error::UnexpectedEof`] from anywhere in the closure — the control region OR the shared
/// tail — is converted to the facts-preserving
/// [`FrameHeaderParseStatus::StoppedInsideInterControl`] (both are modeled § 5.18.2 regions).
fn finish_inter_control_with_tail(
    core: &mut FrameHeaderCore,
    control: crate::headers::frame::inter::InterControl,
    result: Result<()>,
    shared_tail_ran: bool,
) -> Result<()> {
    // Lift the inter reference-grounded frame size / refresh flags onto the core so existing
    // state-supported diagnostics and the inspector see whatever parsed before any EOF. The
    // shared tail already wrote the cluster facts (tile/quant/segmentation/…) onto `core`;
    // these lift the control-region facts that live only on `control`.
    if let Some(size) = control.frame_size {
        core.frame_size = Some(size);
    }
    if let Some(flags) = control.refresh_frame_flags {
        core.refresh_frame_flags = Some(flags);
    }
    core.disable_cdf_update = control.disable_cdf_update;
    core.inter = Some(control);

    match result {
        Ok(()) => {
            if !shared_tail_ran {
                // A control-region coverage stop short of the shared tail (TIP-as-output,
                // bru-inactive / bridge, poisoned / unmodeled derivation): the shared tail
                // is unmodeled by construction here, so the unsupported-coverage status.
                core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
                    feature_id: FRAME_HEADER_INFO_FEATURE,
                };
            }
            // shared_tail_ran: parse_inter_shared_tail set the terminal status itself; leave it.
            Ok(())
        }
        // EOF inside the modeled § 5.18.2 control region OR shared tail: a payload-bounds
        // truncation, not a hard parse error. Keep the preserved facts and surface the
        // truncation status.
        Err(Error::UnexpectedEof { .. }) => {
            core.status = FrameHeaderParseStatus::StoppedInsideInterControl;
            Ok(())
        }
        Err(error) => Err(error),
    }
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
/// [`parse_inter_control_into`](crate::headers::frame::inter::parse_inter_control_into) with
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
    use crate::headers::frame::inter::{InterFrameContext, parse_inter_control_into};

    let inter_seq = build_inter_seq_view(seq);
    let ctx = InterFrameContext {
        obu_type: core.obu_type,
        frame_type: FrameType::Inter,
        is_bridge: true,
        bridge_frame_ref_idx: Some(bridge_frame_ref_idx),
        cur_mfh_id_is_zero: core.cur_mfh_id.is_zero(),
        // The bridge arm derives NumTotalRefs = 1 / ref_frame_idx[0] = bridge_frame_ref_idx
        // (mirror :4597/:4615) without calling get_ref_frames(); order_hint is inert here.
        order_hint: 0,
    };

    // The control region's facts accumulate in a caller-owned `control` so an EOF inside a
    // modeled bridge field preserves the fields parsed before it (codex F2).
    let mut control = crate::headers::frame::inter::InterControl::default();
    // frame_size_override_flag is never read on the bridge path; frame_size_with_bridge()
    // is selected unconditionally by the IsBridge arm (mirror :4627), so the flag is inert
    // for the bridge and passed as false.
    let result = parse_inter_control_into(
        reader,
        &inter_seq,
        &ctx,
        reference_state,
        false,
        &mut control,
    );

    finish_inter_control(core, control, result)
}

/// Parses a single-picture `IsBridge` frame's `frame_header_info()` tail (AV2 § 5.18.2).
///
/// A `single_picture_header_flag == 1` `OBU_BRIDGE_FRAME` is a hybrid: the single-picture
/// branch (mirror :4131-4142) forces `FrameType = KEY_FRAME` / `FrameIsIntra = 1` /
/// `immediate_output_frame = 1` BEFORE the `if ( IsBridge ) FrameType = INTER_FRAME` else-arm
/// (:4205), so the frame travels the *intra* (`FrameIsIntra`) reads — but `IsBridge` is still
/// set, so it ends on the shared `IsBridge` early-return arm, not the full intra structure
/// cluster.
///
/// The reader is positioned just after `bridge_frame_ref_idx` (mirror :4121). This reads, in
/// § 5.18.2 order:
/// - `bridge_frame_overwrite_flag` f(1) (:4423, guarded only by `if ( IsBridge )`).
/// - `refresh_frame_flags` — OVERWRITE-GATED per § 6.17.2 + AVM (see the FIDELITY note): when
///   `bridge_frame_overwrite_flag == 0` it is NOT present and is inferred `1 <<
///   bridge_frame_ref_idx` (no bits); when `== 1` it is read (the bridge arm — AVM
///   `has_refresh_frame_flags` f(1) + `frame_to_refresh` on the `enable_short_refresh_frame_flags`
///   path, else `f(NumRefFrames)`).
/// - `frame_size()` on the `FrameIsIntra` arm (:4565-4567). `frame_size_override_flag` is never
///   assigned on the `IsBridge` arm (:4343 runs instead of the :4357 else), so it keeps its
///   default `0` → the § 5.18.4.1 non-override default dimensions (no bits).
/// - `screen_content_params()` (:4569) and `intrabc_params()` (:4571, `FrameIsIntra`).
///
/// It then reaches the `if ( TipFrameMode == TIP_FRAME_AS_OUTPUT || bru_inactive || IsBridge )`
/// arm (:4971): for `IsBridge` this reads a zero-bit `tile_info()` (:4987; `uniform_tile_spacing_flag`
/// forced `1` and the `increment_tile_*_log2` loops gated behind `!IsBridge`), INFERS
/// `base_q_idx = RefBaseQIdx[bridge_frame_ref_idx]` from the referenced frame (:4997), SKIPS
/// `disable_cdf_update` (the :5039 else-arm), and forces the whole quant/segmentation/deblocking/
/// cdef/ccso/restoration cluster off with no bits (:5045-5083) — all reference-derived or no-bit,
/// so the `base_q_idx`/quant values stay unmodeled. Unlike the non-single bridge
/// ([`parse_bridge_inter_path`], whose `immediate_output_frame == 0` makes `apply_grain == 0`),
/// the arm's `film_grain_config()` (:5011 / § 5.18.10.1) is the LAST modeled read and IS decidable
/// without reference state: `apply_grain` is inferred (single-picture + `immediate_output_frame ==
/// 1`) and reads `fgm_id` f(3) + `grain_seed` f(16) when grain is present. This consumes that tail
/// (for `consumed_bits` accuracy + truncation detection) and then stops with
/// [`InterStop::BruInactiveOrBridgeReturn`](crate::headers::frame::inter::InterStop::BruInactiveOrBridgeReturn),
/// the parsed prefix preserved on `core.inter`, reporting
/// [`FrameHeaderParseStatus::UnsupportedUntilFeature`] (the reference-derived quant is unmodeled).
/// An EOF inside the modeled prefix or the grain tail is converted to the facts-preserving
/// [`FrameHeaderParseStatus::StoppedInsideInterControl`] by [`finish_inter_control`] (codex F2).
/// When `film_grain_params_present` is unknown (a bounded sequence-header stop), `apply_grain`
/// is undecidable, so the parse stops before the grain read (the same honest behavior as the
/// intra / SEF tails).
///
/// SPEC FIDELITY: the single-picture bridge is a degenerate corner where the normative spec is
/// internally inconsistent. (1) refresh_frame_flags: § 5.18.2 syntax would read it unconditionally
/// on the KEY arm (:4429-4445), but § 6.17.2 semantics (:4522-4524) says it is inferred from
/// `bridge_frame_ref_idx` when `bridge_frame_overwrite_flag == 0`; AVM follows § 6.17.2. Per the
/// maintainer decision this parser follows § 6.17.2 + AVM (overwrite-gated) so a validator matches
/// the reference decoder. (2) AVM's `setup_frame_size` also reads two `bridge_frame_max_width`/
/// `_height` fields the § 5.18.2 `FrameIsIntra` `frame_size()` does not — splot follows § 5.18.2
/// (no frame-size bits) here, a remaining documented divergence. dav2d does not model the
/// single-picture bridge at all. See `openspec/changes/frame-header-single-picture-bridge-fix`.
fn parse_single_picture_bridge_tail(
    reader: &mut BitReader<'_>,
    core: &mut FrameHeaderCore,
    seq: &CoreSeqView,
    bridge_frame_ref_idx: u32,
) -> Result<()> {
    use crate::headers::frame::inter::{InterControl, InterStop, TipFrameMode};

    // `PRIMARY_REF_NONE` (AV2 § 3): the IsBridge arm infers primary_ref_frame = PRIMARY_REF_NONE
    // (mirror :4345, no bits).
    const PRIMARY_REF_NONE: u8 = 7;

    // The bridge facts accumulate in a caller-owned `control` so an EOF inside a modeled field
    // preserves the fields parsed before it (codex F2); finish_inter_control lifts them onto core,
    // sets the terminal status, and converts an EOF into StoppedInsideInterControl.
    let mut control = InterControl::default();
    let result = (|| -> Result<()> {
        // mirror :4423-4427: bridge_frame_overwrite_flag f(1) — read on any IsBridge frame.
        let bridge_frame_overwrite_flag = reader.read_flag()?;
        control.bridge_frame_overwrite_flag = Some(bridge_frame_overwrite_flag);

        // refresh_frame_flags: SPEC CONTRADICTION at this corner. § 5.18.2 syntax (:4429-4445)
        // would read it UNCONDITIONALLY via the `if ( FrameType == KEY_FRAME )` arm (a
        // single-picture bridge has FrameType == KEY_FRAME, so the `else if ( IsBridge &&
        // !bridge_frame_overwrite_flag )` arm at :4489 is unreachable). But § 6.17.2 SEMANTICS
        // (06-syntax-structures-semantics.md :4522-4524) states unconditionally that
        // `bridge_frame_overwrite_flag == 0` means refresh_frame_flags is NOT present and is
        // inferred `1 << bridge_frame_ref_idx`, and AVM (decodeframe.c:8394-8422) implements
        // exactly that overwrite-gated reading. Per the maintainer decision (codex PR review),
        // splot follows § 6.17.2 + AVM — a validator must match the reference decoder so it does
        // not misparse a real (AVM-encoded) overwrite == 0 single-picture bridge by reading
        // NumRefFrames phantom bits. So:
        //   overwrite == 0 -> refresh_frame_flags = 1 << bridge_frame_ref_idx (no bits).
        //   overwrite == 1 -> read it (the bridge arm, mirror AVM: has_refresh_frame_flags f(1) +
        //                     frame_to_refresh on the short-flag path, else f(NumRefFrames)).
        let refresh_frame_flags = if !bridge_frame_overwrite_flag {
            1u32.wrapping_shl(bridge_frame_ref_idx)
        } else if seq.enable_short_refresh_frame_flags {
            if reader.read_flag()? {
                1u32.wrapping_shl(reader.read_f(ceil_log2(seq.num_ref_frames))?)
            } else {
                0
            }
        } else {
            reader.read_f(seq.num_ref_frames)?
        };
        control.refresh_frame_flags = Some(refresh_frame_flags);

        // mirror :4565-4567 / § 5.18.4.1: frame_size() on the FrameIsIntra arm.
        // frame_size_override_flag defaults to 0 (the IsBridge arm at :4343 never assigns it), so
        // this is the non-override default-dimensions path. A bridge frame always has
        // cur_mfh_id == 0 (mirror :4119 `if ( IsBridge ) cur_mfh_id = 0`, enforced in
        // parse_frame_header_prefix), so the default dims are the sequence maxima (§ 5.18.4.1
        // else-branch) — there is no cur_mfh_id > 0 / MFH-resolution case here. No bits are read.
        core.frame_size_override_flag = Some(false);
        control.frame_size = parse_frame_size(
            reader,
            false,
            seq.frame_width_bits,
            seq.frame_height_bits,
            Some((seq.max_frame_width, seq.max_frame_height)),
        )?;

        // mirror :4569 / § 5.18.3.3: screen_content_params().
        let scc = parse_screen_content_params_full(
            reader,
            seq.seq_force_screen_content_tools,
            seq.seq_force_integer_mv,
        )?;
        core.allow_screen_content_tools = Some(scc.allow_screen_content_tools);
        core.force_integer_mv = Some(scc.force_integer_mv);
        control.allow_screen_content_tools = Some(scc.allow_screen_content_tools);

        // mirror :4571 / § 5.18.3.4: intrabc_params() (FrameIsIntra == 1).
        let intrabc = parse_intrabc_params_full(reader, true, seq.allow_frame_max_bvp_drl_bits)?;
        core.allow_intrabc = Some(intrabc.allow_intrabc);
        core.intrabc = Some(intrabc);
        control.allow_intrabc = Some(intrabc.allow_intrabc);

        // mirror :4573-4575 / :4345: the FrameIsIntra arm tail — NumTotalRefs = 0,
        // TipFrameMode = TIP_FRAME_DISABLED (no bits); primary_ref_frame = PRIMARY_REF_NONE was
        // inferred on the IsBridge arm (:4345). Recorded for the inspector/validator.
        control.num_total_refs = Some(0);
        control.tip_frame_mode = Some(TipFrameMode::Disabled);
        control.primary_ref_frame = Some(PRIMARY_REF_NONE);

        // mirror :4971-5011: the IsBridge early-return arm. tile_info() reads ZERO bits for a
        // bridge (uniform_tile_spacing_flag forced 1, the increment loops gated behind !IsBridge,
        // :6599/:6615/:6645), and base_q_idx = RefBaseQIdx[bridge_frame_ref_idx] / DeltaQ are
        // reference-state derived (no bits, :4997) — those quant values stay unmodeled, so this
        // remains a BruInactiveOrBridgeReturn coverage stop. disable_cdf_update (the :5039 else-arm)
        // and the entire quant/segmentation/deblocking/cdef/ccso/restoration cluster (:5045-5083)
        // are SKIPPED (no bits).
        //
        // film_grain_config() (:5011 / § 5.18.10.1) is the LAST modeled frame-header read, and it
        // IS decidable without reference state: with single_picture_header_flag == 1 and
        // immediate_output_frame == 1, apply_grain is inferred (mirror :8165-8171 — 1 when
        // film_grain_params_present, else 0) and reads fgm_id f(3) + grain_seed f(16). Consume it so
        // consumed_bits covers the mandatory frame-header syntax and a truncation there surfaces as
        // StoppedInsideInterControl, not a silent coverage stop (codex review). The bridge frame is
        // unsupported coverage (base_q_idx unmodeled), so the parsed grain config is not exposed —
        // the read is for bit-accuracy + truncation detection only. If the active sequence header
        // was a bounded stop that never read film_grain_params_present, apply_grain is undecidable,
        // so stop before the grain read (the honest behavior the intra / SEF tails also use).
        if let Some(film_grain_params_present) = seq.film_grain_params_present {
            let input = FrameTailInput {
                // base_q_idx is reference-derived; film_grain_config() does not consult
                // coded_lossless, so the value supplied here is inert.
                coded_lossless: false,
                film_grain_params_present,
                single_picture_header_flag: true,
                immediate_output_frame: true,
                implicit_output_frame: false,
            };
            let _ = parse_film_grain_config(reader, &input)?;
        }
        control.stop = Some(InterStop::BruInactiveOrBridgeReturn);
        Ok(())
    })();

    finish_inter_control(core, control, result)
}

/// Parses the show-existing-frame sub-path (AV2 § 5.18.2), stopping before
/// `film_grain_config()`.
fn parse_show_existing_frame(
    reader: &mut BitReader<'_>,
    core: &mut FrameHeaderCore,
    seq: &CoreSeqView,
) -> Result<()> {
    core.frame_to_show_map_idx = Some(reader.read_f(ceil_log2(seq.num_ref_frames))?);
    let derive_sef_order_hint = reader.read_flag()?;
    if !derive_sef_order_hint {
        core.order_hint_lsb = Some(reader.read_f(seq.order_hint_bits)?);
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

/// Parses the intra-frame tail (AV2 § 5.18.2), from frame-size provenance through
/// `disable_cdf_update` and the structure cluster, stopping before
/// `deblocking_filter_params()` (§ 5.18.5.2).
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
        reader.read_flag()?
    };
    // Record the dims provenance for the §6.17.4.1 / §6.17.2 validator split: the
    // non-override path (`false`) derives FrameWidth/FrameHeight from the MFH default
    // dimensions (or the sequence maxima for cur_mfh_id == 0), the override path
    // (`true`) from this frame's explicit frame_width/height_minus_1 fields below.
    core.frame_size_override_flag = Some(frame_size_override_flag);

    // order_hint f(OrderHintBits); OrderHintLsbs = order_hint.
    core.order_hint_lsb = Some(reader.read_f(seq.order_hint_bits)?);
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
    let scc = parse_screen_content_params_full(
        reader,
        seq.seq_force_screen_content_tools,
        seq.seq_force_integer_mv,
    )?;
    core.allow_screen_content_tools = Some(scc.allow_screen_content_tools);
    core.force_integer_mv = Some(scc.force_integer_mv);
    let intrabc = parse_intrabc_params_full(reader, true, seq.allow_frame_max_bvp_drl_bits)?;
    core.allow_intrabc = Some(intrabc.allow_intrabc);
    core.intrabc = Some(intrabc);

    // Not a TIP-as-output / bru-inactive / bridge frame -> disable_cdf_update f(1)
    // (AV2 § 5.18.2 else-branch of `if ( bru_inactive || IsBridge )`).
    core.disable_cdf_update = Some(reader.read_flag()?);

    // On the intra path, no BRU / motion-field / TIP block reads before `tile_info()`.
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
/// When a plane signals a frame-level Wiener filter, `lr_params()` consumes the fixed-coded
/// `read_wienerns_filter()` bank and stores it on the completed `core.lr_params`. A reserved
/// [`FrameHeaderParseStatus::StoppedBeforeWienerNsFilter`] outcome remains possible only for
/// unsupported future branches. On error the partially-read fields stay `None`; the caller
/// decides whether a payload EOF in the loop-filter cluster (deblocking through ccso) is a
/// truncation (`StoppedInsideFilterParams`) or a hard failure (a tail EOF is handled here as
/// `StoppedInsideIntraTail`).
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
        // AV2 § 5.18.5.2 (mirror :5935): allow_df_sub_pu is read only on the
        // FrameType == INTER_FRAME path; this is the intra cluster, so it never fires.
        false,
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
    // subsampling drive the size signaling. A plane signalling frame_filters_on consumes
    // the fixed-coded read_wienerns_filter() bank and stores it on the completed
    // LrPlaneParams.
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
            // Reserved unsupported branch. Surface the real prefix on the dedicated partial
            // field; `lr_params` stays None so partial and complete parses cannot be
            // confused.
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
            let frame_to_refresh = reader.read_f(ceil_log2(seq.num_ref_frames))?;
            Ok(1u32.wrapping_shl(frame_to_refresh))
        } else {
            reader.read_f(seq.num_ref_frames)
        }
    } else if seq.enable_short_refresh_frame_flags {
        // INTRA_ONLY_FRAME with the compact signaling mode.
        let has_refresh_frame_flags = reader.read_flag()?;
        if has_refresh_frame_flags {
            let frame_to_refresh = reader.read_f(ceil_log2(seq.num_ref_frames))?;
            Ok(1u32.wrapping_shl(frame_to_refresh))
        } else {
            Ok(0)
        }
    } else {
        reader.read_f(seq.num_ref_frames)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

#[cfg(test)]
mod proptests;
