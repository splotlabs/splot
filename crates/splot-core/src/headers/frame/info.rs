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
use crate::error::{Error, Result};
use crate::headers::frame::config::{
    IntrabcParams, parse_intrabc_params_full, parse_screen_content_params_full,
};
use crate::headers::frame::filtering::{
    CdefParams, DeblockingFilterParams, GdfGeometry, GdfParams, parse_cdef_params,
    parse_deblocking_filter_params, parse_gdf_params,
};
use crate::headers::frame::quant::{
    DeltaQParams, LosslessInfo, QuantizationParams, SetupQmParams, parse_delta_q_params,
    parse_lossless_info, parse_quantization_params, parse_setup_qm_params,
};
use crate::headers::frame::restoration::{
    CcsoParams, LrGeometry, LrParams, LrParseOutcome, LrPartialParams, parse_ccso_params,
    parse_lr_params,
};
use crate::headers::frame::segmentation::{SegmentationParams, parse_segmentation_params};
use crate::headers::frame::size::{FrameSize, ceil_log2, parse_frame_size};
use crate::headers::frame::tail::{
    FilmGrainConfig, FrameHeaderTail, FrameTailInput, parse_film_grain_config,
    parse_intra_tail as parse_intra_tail_grammar,
};
use crate::headers::frame::tiling::{TileInfo, parse_tile_info};
use crate::headers::sequence::{SequenceHeader, SequenceHeaderId};
use crate::hls::{MfhId, MultiFrameHeaderRecord};
use crate::types::ObuType;

use super::{FrameHeaderPrefix, parse_frame_header_prefix};

mod seq_view;
mod show_existing;
mod status;

pub use seq_view::{CoreSeqInterView, CoreSeqView, MfhFrameView};
pub use status::{FrameHeaderParseMode, FrameHeaderParseStatus, FrameType, SefTrailingBits};

use seq_view::all_frames_mask;
use show_existing::parse_show_existing_frame;

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
    /// Per-slot retained frame-level Wiener-NS filter class counts for Y/U/V. The inter
    /// `lr_params()` frame-filter dictionary uses these counts when reading the next
    /// frame's frame-level Wiener-NS match indices.
    pub lr_frame_filter_class_counts: Option<&'a [[u8; 3]]>,
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
            lr_frame_filter_class_counts: None,
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
        let mut view = Self::unknown();
        view.ref_valid = Some(ref_valid);
        view.ref_order_hint = Some(ref_order_hint);
        view.ref_frame_width = Some(ref_frame_width);
        view.ref_frame_height = Some(ref_frame_height);
        view
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
        let mut view =
            Self::from_slots(ref_valid, ref_order_hint, ref_frame_width, ref_frame_height);
        view.ref_base_q_idx = Some(ref_base_q_idx);
        view
    }

    /// Adds retained per-slot LR frame-filter class counts to an existing reference-state view.
    #[must_use]
    pub const fn with_lr_frame_filter_class_counts(mut self, counts: &'a [[u8; 3]]) -> Self {
        self.lr_frame_filter_class_counts = Some(counts);
        self
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
    /// parsed bits alone rather than requiring the whole core parse to complete.
    /// `false` for every frame type whose `reset_qm()` trigger is not met, and for a RAS /
    /// restricted SWITCH whose parse stops BEFORE the call site (truncated mid-prefix or
    /// mid-`ref_long_term_id` — the reset is then unconfirmed).
    pub reached_qm_reset: bool,
    /// Bits consumed by this parse (not necessarily the whole frame header).
    pub consumed_bits: u64,
}

/// Matrix Feature ID for the frame-header-info coverage this phase does not model.
const FRAME_HEADER_INFO_FEATURE: &str = "AV2-5.18.2-FRAME-HEADER-INFO";

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

    let prefix =
        parse_frame_header_prefix(reader, input.obu_type, Some(input.first_picture_in_tu))?;
    let mut core = init_core_from_prefix(&prefix, input.obu_type, input.first_picture_in_tu);

    if input.mode == FrameHeaderParseMode::Core
        && let Some(seq) = input.active_sequence.and_then(CoreSeqView::from_sequence)
    {
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

    let bridge_frame_ref_idx = if core.is_bridge {
        let idx = reader.read_f(ceil_log2(seq.num_ref_frames))?;
        core.bridge_frame_ref_idx = Some(idx);
        Some(idx)
    } else {
        None
    };

    if seq.single_picture_header_flag {
        core.show_existing_frame = Some(false);
        core.frame_type = Some(FrameType::Key);
        core.frame_is_intra = Some(true);
        core.immediate_output_frame = Some(true);
        core.implicit_output_frame = Some(false);
        if let Some(bridge_frame_ref_idx) = bridge_frame_ref_idx {
            return parse_single_picture_bridge_tail(reader, core, seq, bridge_frame_ref_idx);
        }
        return parse_intra_tail(reader, core, seq, mfh, FrameType::Key, true);
    }

    if let Some(bridge_frame_ref_idx) = bridge_frame_ref_idx {
        core.frame_type = Some(FrameType::Inter);
        core.frame_is_intra = Some(false);
        core.immediate_output_frame = Some(false);
        core.implicit_output_frame = Some(false);
        return parse_bridge_inter_path(reader, core, seq, bridge_frame_ref_idx, reference_state);
    }

    let show_existing_frame = obu_type.is_sef();
    core.show_existing_frame = Some(show_existing_frame);
    if show_existing_frame {
        return parse_show_existing_frame(reader, core, seq);
    }

    let frame_type = if obu_type == ObuType::Switch || obu_type == ObuType::RasFrame {
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

    core.long_term_id = Some(-1);
    if frame_type == FrameType::Key {
        let long_term_id_plus_1 = reader.read_f(seq.long_term_frame_id_bits)?;
        core.long_term_id = Some(i64::from(long_term_id_plus_1) - 1);
    }
    if (obu_type == ObuType::RasFrame || obu_type == ObuType::OpenLoopKey)
        && seq.long_term_frame_id_bits != 0
    {
        let reserved_long_term_id = (1u32 << seq.long_term_frame_id_bits).wrapping_sub(1);
        let num_key_ref_frames = reader.read_bits(3)?;
        let mut ref_long_term_ids = Vec::with_capacity(num_key_ref_frames as usize);
        for _ in 0..num_key_ref_frames {
            let ref_long_term_id = reader.read_f(seq.long_term_frame_id_bits)?;
            if ref_long_term_id == reserved_long_term_id {
                core.forbidden_ref_long_term_id = true;
            }
            ref_long_term_ids.push(ref_long_term_id);
        }
        core.ref_long_term_ids = ref_long_term_ids;
    }

    core.reached_qm_reset = obu_type == ObuType::RasFrame
        || (obu_type == ObuType::Switch && core.restricted_prediction_switch == Some(true));

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

    let mut control = crate::headers::frame::inter::InterControl::default();

    let mut shared_tail_ran = false;

    let result = (|| -> Result<()> {
        let frame_size_override_flag = if frame_type == FrameType::Switch {
            true
        } else {
            reader.read_flag()?
        };
        core.frame_size_override_flag = Some(frame_size_override_flag);

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

        if control.stop == Some(InterStop::ReachedSharedTail) {
            core.frame_size = control.frame_size;
            shared_tail_ran = true;
            parse_inter_shared_tail(reader, core, seq, &control, frame_type, reference_state)?;
        }
        Ok(())
    })();

    finish_inter_control_with_tail(core, control, result, shared_tail_ran)
}

/// Records a parsed inter / bridge `control` onto `core` and sets the terminal status,
/// converting an [`Error::UnexpectedEof`] inside the modeled § 5.18.2 control region into a
/// facts-preserving truncation status:
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
///   validator's `.ok()` would drop ALL facts and the truncation.
/// - Any other `Err`: a genuine malformed-input error propagates unchanged.
fn finish_inter_control(
    core: &mut FrameHeaderCore,
    control: crate::headers::frame::inter::InterControl,
    result: Result<()>,
) -> Result<()> {
    if let Some(size) = control.frame_size {
        core.frame_size = Some(size);
    }
    if let Some(flags) = control.refresh_frame_flags {
        core.refresh_frame_flags = Some(flags);
    }
    if let Some(force_integer_mv) = control.force_integer_mv {
        core.force_integer_mv = Some(force_integer_mv);
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
                core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
                    feature_id: FRAME_HEADER_INFO_FEATURE,
                };
            }
            Ok(())
        }
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
        enable_tip_output: seq.inter.enable_tip_output,
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
        order_hint: 0,
    };

    let mut control = crate::headers::frame::inter::InterControl::default();
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
/// [`FrameHeaderParseStatus::StoppedInsideInterControl`] by [`finish_inter_control`].
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

    const PRIMARY_REF_NONE: u8 = 7;

    let mut control = InterControl::default();
    let result = (|| -> Result<()> {
        let bridge_frame_overwrite_flag = reader.read_flag()?;
        control.bridge_frame_overwrite_flag = Some(bridge_frame_overwrite_flag);

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

        core.frame_size_override_flag = Some(false);
        control.frame_size = parse_frame_size(
            reader,
            false,
            seq.frame_width_bits,
            seq.frame_height_bits,
            Some((seq.max_frame_width, seq.max_frame_height)),
        )?;

        let scc = parse_screen_content_params_full(
            reader,
            seq.seq_force_screen_content_tools,
            seq.seq_force_integer_mv,
        )?;
        core.allow_screen_content_tools = Some(scc.allow_screen_content_tools);
        core.force_integer_mv = Some(scc.force_integer_mv);
        control.allow_screen_content_tools = Some(scc.allow_screen_content_tools);

        let intrabc = parse_intrabc_params_full(reader, true, seq.allow_frame_max_bvp_drl_bits)?;
        core.allow_intrabc = Some(intrabc.allow_intrabc);
        core.intrabc = Some(intrabc);
        control.allow_intrabc = Some(intrabc.allow_intrabc);

        control.num_total_refs = Some(0);
        control.tip_frame_mode = Some(TipFrameMode::Disabled);
        control.primary_ref_frame = Some(PRIMARY_REF_NONE);

        if let Some(film_grain_params_present) = seq.film_grain_params_present {
            let input = FrameTailInput {
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
    let frame_size_override_flag = if single_picture {
        false
    } else {
        reader.read_flag()?
    };
    core.frame_size_override_flag = Some(frame_size_override_flag);

    core.order_hint_lsb = Some(reader.read_f(seq.order_hint_bits)?);

    core.refresh_frame_flags = Some(read_refresh_frame_flags(
        reader,
        seq,
        core.obu_type,
        frame_type,
    )?);

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

    core.disable_cdf_update = Some(reader.read_flag()?);

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
    let Some(frame_size) = core.frame_size else {
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
    };

    core.tile_info = match parse_tile_info(reader, &seq.tile, frame_size, true, false, false) {
        Ok(tile_info) => Some(tile_info),
        Err(Error::Unimplemented { feature }) => {
            core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
                feature_id: feature,
            };
            return Ok(());
        }
        Err(error) => return Err(error),
    };

    let quantization = parse_quantization_params(reader, &seq.quant, false)?;
    core.quantization_params = Some(quantization);

    if !core.cur_mfh_id.is_zero() && mfh.is_none() {
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
    }
    let mfh_seg = mfh.and_then(|view| view.seg.as_ref());
    let segmentation = parse_segmentation_params(reader, &seq.seg, mfh_seg)?;

    let qm = parse_setup_qm_params(reader, &seq.quant, segmentation.segmentation_enabled)?;

    let delta_q = parse_delta_q_params(reader, quantization.base_q_idx)?;

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
    core.segmentation_params = Some(segmentation);
    core.setup_qm_params = Some(qm);
    core.delta_q_params = Some(delta_q);

    match parse_filter_cluster(reader, core, seq, mfh, coded_lossless) {
        Ok(()) => Ok(()),
        Err(Error::UnexpectedEof { .. }) => {
            core.status = FrameHeaderParseStatus::StoppedInsideFilterParams;
            Ok(())
        }
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
    let mfh_deblocking = mfh.map(|view| &view.deblocking);
    core.deblocking_filter_params = Some(parse_deblocking_filter_params(
        reader,
        coded_lossless,
        seq.quant.num_planes,
        seq.filter.df_par_bits_minus_2,
        false,
        mfh_deblocking,
    )?);

    let gdf = {
        let Some(tile_info) = core.tile_info.as_ref() else {
            core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
                feature_id: FRAME_HEADER_INFO_FEATURE,
            };
            return Ok(());
        };
        let geometry = GdfGeometry {
            sb_size: seq.tile.frame_sb_size(true),
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

    core.cdef_params = Some(parse_cdef_params(
        reader,
        coded_lossless,
        seq.quant.num_planes,
        &seq.filter,
    )?);

    let lr_geometry = LrGeometry::new(seq.tile.frame_sb_size(true), seq.chroma_format_idc);
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
            core.lr_params_partial = Some(partial);
            core.status = FrameHeaderParseStatus::StoppedBeforeWienerNsFilter { feature_id };
            return Ok(());
        }
    }

    core.ccso_params = Some(parse_ccso_params(
        reader,
        coded_lossless,
        seq.quant.num_planes,
        &seq.ccso,
    )?);

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
    let Some(film_grain_params_present) = seq.film_grain_params_present else {
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
    };
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
