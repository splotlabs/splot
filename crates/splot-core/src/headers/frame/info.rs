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
//! - **Bridge frame** → reads `bridge_frame_ref_idx` then stops
//!   ([`FrameHeaderParseStatus::UnsupportedUntilFeature`]); the rest of a bridge frame
//!   needs reference-frame dimensions this phase does not model.
//! - **Show-existing-frame (SEF)** → reads `frame_to_show_map_idx`,
//!   `derive_sef_order_hint`, and `sef_order_hint`, then stops before
//!   `film_grain_config()` ([`FrameHeaderParseStatus::CoreFieldsOnly`]).
//! - **Inter / switch / TIP / RAS frame** → reads the frame-type field then stops
//!   ([`FrameHeaderParseStatus::UnsupportedUntilFeature`]); the inter reference map
//!   needs reference-frame state.
//! - **Intra frame (key / intra-only / single-picture)** → reads the full control
//!   region through `frame_size()`, `screen_content_params()`, `intrabc_params()`,
//!   `disable_cdf_update`, `tile_info()`, `quantization_params()`,
//!   `segmentation_params()`, `setup_qm_params()`, `delta_q_params()`, the
//!   § 5.18.2 lossless/`allow_tcq`/`allow_parity_hiding` tail, and the loop-filter
//!   cluster `deblocking_filter_params()` (§ 5.18.5.2), `gdf_params()` (§ 5.18.7.9),
//!   and `cdef_params()` (§ 5.18.7.10), stopping before `lr_params()` (loop
//!   restoration, § 5.18.7.11)
//!   ([`FrameHeaderParseStatus::StoppedBeforeLoopRestorationParams`]). A payload that
//!   runs out **inside** the loop-filter cluster instead keeps the already-parsed
//!   control-region facts and reports the truncation as
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
use crate::headers::frame::config::{parse_intrabc_params, parse_screen_content_params};
use crate::headers::frame::filtering::{
    CdefParams, CoreSeqFilterView, DeblockingFilterParams, GdfGeometry, GdfParams,
    MfhDeblockingView, parse_cdef_params, parse_deblocking_filter_params, parse_gdf_params,
};
use crate::headers::frame::quant::{
    CoreSeqQuantView, DeltaQParams, LosslessInfo, QuantizationParams, SetupQmParams,
    parse_delta_q_params, parse_lossless_info, parse_quantization_params, parse_setup_qm_params,
};
use crate::headers::frame::segmentation::{
    CoreSeqSegView, MfhSegView, SegmentationParams, parse_segmentation_params,
};
use crate::headers::frame::size::{FrameSize, ceil_log2, parse_frame_size};
use crate::headers::frame::tiling::{CoreSeqTileView, TileInfo, parse_tile_info};
use crate::headers::sequence::{SequenceHeader, SequenceHeaderId};
use crate::hls::{MfhId, MultiFrameHeaderRecord};
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrameHeaderParseStatus {
    /// Only the activation/reference fields were read — either the caller asked for
    /// [`FrameHeaderParseMode::ActivationPrefix`], or core mode lacked the sequence
    /// state (a fully parsed active sequence header) needed to continue.
    ActivationFieldsOnly,
    /// Core control fields were read and the parser stopped at a bounded point that is
    /// not the filtering/quantization/segmentation cluster — currently the
    /// show-existing-frame path, which stops before `film_grain_config()`.
    CoreFieldsOnly,
    /// The show-existing-frame path was consumed in full. Reserved: produced once
    /// `film_grain_config()` (§ 5.18.10) is modeled; the current SEF path returns
    /// [`Self::CoreFieldsOnly`].
    ShowExistingFrameComplete,
    /// An intra frame's control region was read through `disable_cdf_update`,
    /// `tile_info()` (§ 5.18.7.2), `quantization_params()` (§ 5.18.6.1),
    /// `segmentation_params()` (§ 5.18.7.1), `setup_qm_params()` (§ 5.18.6.2),
    /// `delta_q_params()` (§ 5.18.7.8), the § 5.18.2 lossless/`allow_tcq`/
    /// `allow_parity_hiding` tail, and the loop-filter cluster
    /// `deblocking_filter_params()` (§ 5.18.5.2), `gdf_params()` (§ 5.18.7.9), and
    /// `cdef_params()` (§ 5.18.7.10); the parser stopped before `lr_params()`
    /// (loop restoration, § 5.18.7.11).
    StoppedBeforeLoopRestorationParams,
    /// An intra frame's control region was read in full through the § 5.18.2
    /// lossless/`allow_tcq`/`allow_parity_hiding` tail, but the payload ran out
    /// **inside** the loop-filter cluster `deblocking_filter_params()` (§ 5.18.5.2),
    /// `gdf_params()` (§ 5.18.7.9), or `cdef_params()` (§ 5.18.7.10). The already-parsed
    /// control-region facts (frame size, output flags, tile/quant/segmentation) are
    /// intact and exposed; the cluster fields that were not reached stay `None`. The
    /// truncation itself is a payload-bounds condition, not a structural violation, so it
    /// is reported through this status rather than as a hard parse error — earlier
    /// state-supported diagnostics still see the preserved facts (the pre-cluster
    /// behavior, which stopped here before any filter read, is preserved). No
    /// full-payload trailing-bits conformance is implied.
    StoppedInsideFilterParams,
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
            Self::StoppedBeforeLoopRestorationParams => "stopped_before_loop_restoration_params",
            Self::StoppedInsideFilterParams => "stopped_inside_filter_params",
            Self::UnsupportedUntilFeature { .. } => "unsupported_until_feature",
        }
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

/// A read-only view of reference-frame buffer state for frame-header decisions.
///
/// This phase does not model the reference-frame buffers, so the validator passes
/// [`FrameReferenceStateView::unknown`] and the core parser does not yet branch on it.
/// The type exists so reference-state-dependent paths (explicit reference maps,
/// `frame_size_with_refs()`, show-existing-frame slot validity) can be added later
/// without changing the parser's call signature.
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

impl FrameReferenceStateView<'_> {
    /// A fully-unknown reference state (the only state this phase models).
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            ref_valid: None,
            ref_order_hint: None,
            ref_frame_width: None,
            ref_frame_height: None,
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
    /// Bits consumed by this parse (not necessarily the whole frame header).
    pub consumed_bits: u64,
}

/// Matrix Feature ID for the frame-header-info coverage this phase does not model.
const FRAME_HEADER_INFO_FEATURE: &str = "AV2-5.18.2-FRAME-HEADER-INFO";

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
    /// § 5.18.6 / § 5.18.7.8 / § 5.18.2-lossless-tail inputs (AV2 § 5.4.8).
    quant: CoreSeqQuantView,
    /// § 5.18.7.1 segmentation inputs (AV2 § 5.4.4).
    seg: CoreSeqSegView,
    /// § 5.18.7.2 tile-info inputs (AV2 § 5.4.2 / § 5.4.3 / § 5.4.8).
    tile: CoreSeqTileView,
    /// § 5.18.5.2 / § 5.18.7.9 / § 5.18.7.10 loop-filter inputs (AV2 § 5.4.10).
    filter: CoreSeqFilterView,
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
        parse_core_body(reader, &mut core, &seq, mfh_view.as_ref())?;
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
) -> Result<()> {
    let obu_type = core.obu_type;

    // AV2 § 5.18.2: a bridge frame reads bridge_frame_ref_idx f(CeilLog2(NumRefFrames))
    // immediately after load_sequence_header(). The rest of a bridge frame needs
    // reference-frame dimensions, so the parser stops here.
    if core.is_bridge {
        core.bridge_frame_ref_idx = Some(read_f(reader, ceil_log2(seq.num_ref_frames))?);
        core.frame_type = Some(FrameType::Inter);
        core.frame_is_intra = Some(false);
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
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
        reader.read_bit()?; // restricted_prediction_switch f(1)
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

    if !frame_is_intra {
        // Inter / switch / RAS / TIP: the remaining control fields and the inter
        // reference map need reference-frame state, so the parser stops here.
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
    }

    // AV2 § 5.18.2 output control (intra frames).
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

    parse_intra_tail(reader, core, seq, mfh, frame_type, false)
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
    // refresh_frame_flags = 0; immediate_output_frame = 1; FrameType comes from the
    // referenced slot (reference state), so it is left unknown. Stop before
    // film_grain_config() (§ 5.18.10).
    core.refresh_frame_flags = Some(0);
    core.immediate_output_frame = Some(true);
    core.status = FrameHeaderParseStatus::CoreFieldsOnly;
    Ok(())
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

    // AV2 § 5.18.2 tail (mirror :5297-5301): the loop-filter cluster
    // deblocking_filter_params() / gdf_params() / cdef_params(). A truncation INSIDE the
    // cluster must not discard the control-region facts already parsed above (frame size,
    // output flags, tile/quant/segmentation): before this cluster existed the parser
    // stopped here and returned Ok with exactly those facts, and the validator/inspect
    // call sites .ok() the result, so an Err would silently drop every earlier
    // state-supported diagnostic. parse_filter_cluster() therefore converts a payload-EOF
    // into the StoppedInsideFilterParams status (facts preserved, cluster fields left
    // None) and only propagates a genuine structural error.
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
/// `deblocking_filter_params()` (§ 5.18.5.2), `gdf_params()` (§ 5.18.7.9), and
/// `cdef_params()` (§ 5.18.7.10), in that order
/// (AV2 v1.0.0 § 5.18.2, mirror :5297-5301). All three are determined by the parsed
/// sequence filter config (§ 5.4.10), the frame state (`CodedLossless`, `NumPlanes`), the
/// parsed `tile_info()` geometry, and — on the `cur_mfh_id > 0` path — the resolved MFH's
/// deblocking-update state.
///
/// On success the three `core` filter fields are populated. On error the partially-read
/// fields stay `None`; the caller decides whether a payload EOF here is a truncation
/// (`StoppedInsideFilterParams`) or a hard failure.
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

    // AV2 § 5.18.2: lr_params() (loop restoration, § 5.18.7.11) is next; this phase
    // stops before it. The cluster parsed cleanly, so this is the terminal stop.
    core.status = FrameHeaderParseStatus::StoppedBeforeLoopRestorationParams;
    Ok(())
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
            quant,
            seg,
            tile,
            filter: base_filter(),
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
        let mut reader = BitReader::new(data, ByteOffset::new(0));
        let prefix = parse_frame_header_prefix(&mut reader, obu_type, first_picture_in_tu)?;
        let mut core = init_core_from_prefix(&prefix, obu_type);
        parse_core_body(&mut reader, &mut core, seq, mfh_view)?;
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
        let data = bits.into_bytes();
        let (core, consumed) =
            parse_body(&data, ObuType::ClosedLoopKey, true, &base_seq()).unwrap();

        assert_eq!(
            core.status,
            FrameHeaderParseStatus::StoppedBeforeLoopRestorationParams
        );
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
        // uvlc(0)=1 + uvlc(1)=3 prefix bits, then 33 core bits (1+1+1+4 control/output,
        // 24 frame_size, 1 allow_intrabc, 1 disable_cdf_update), then 14 structure
        // bits (3 tile_info, 8 base_q_idx, 1 segmentation_enabled, 1 using_qmatrix,
        // 1 delta_q_present), then 2 deblocking apply bits (GDF/CDEF disabled -> 0 bits).
        assert_eq!(consumed, 4 + 33 + 14 + 2);
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
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::StoppedBeforeLoopRestorationParams
        );
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
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::StoppedBeforeLoopRestorationParams
        );
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
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::StoppedBeforeLoopRestorationParams
        );
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
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::StoppedBeforeLoopRestorationParams
        );
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
        let data = bits.into_bytes();
        let (core, consumed) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();

        assert_eq!(
            core.status,
            FrameHeaderParseStatus::StoppedBeforeLoopRestorationParams
        );
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
        // 2 prefix bits + 33 control/size bits + 64 pre-filter structure bits (7 tile_info,
        // 8 base_q_idx, 25 segmentation, 13 setup_qm, 1 delta_q_present, 8 qm_index,
        // 1 allow_tcq, 1 allow_parity_hiding) + 30 loop-filter bits (7 deblocking,
        // 6 gdf, 17 cdef).
        assert_eq!(consumed, 2 + 33 + 64 + 30);
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
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::RegularTileGroup, true, &base_seq()).unwrap();

        assert_eq!(core.frame_type, Some(FrameType::IntraOnly));
        assert_eq!(core.frame_is_intra, Some(true));
        assert_eq!(core.refresh_frame_flags, Some(0b0000_0101));
        assert_eq!(core.frame_size, Some(FrameSize::new(4096, 2304)));
        assert_eq!(core.quantization_params.unwrap().base_q_idx, 45);
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::StoppedBeforeLoopRestorationParams
        );
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
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &seq).unwrap();

        assert_eq!(core.show_existing_frame, Some(false));
        assert_eq!(core.frame_type, Some(FrameType::Key));
        assert_eq!(core.immediate_output_frame, Some(true));
        assert_eq!(core.implicit_output_frame, Some(false));
        assert_eq!(core.order_hint_lsb, Some(9));
        assert_eq!(core.frame_size, Some(FrameSize::new(4096, 2304)));
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::StoppedBeforeLoopRestorationParams
        );
    }

    #[test]
    fn frame_header_core_bridge_reads_ref_idx_then_stops() {
        // Bridge frame: cur_mfh_id inferred 0, reads seq_header_id, then
        // bridge_frame_ref_idx f(CeilLog2(8) == 3); stops before reference-state syntax.
        let mut bits = Bits::default();
        bits.uvlc(4); // seq_header_id_in_frame_header (bridge infers cur_mfh_id == 0)
        bits.f(5, 3); // bridge_frame_ref_idx
        let data = bits.into_bytes();
        let (core, consumed) = parse_body(&data, ObuType::BridgeFrame, true, &base_seq()).unwrap();

        assert!(core.is_bridge);
        assert_eq!(core.bridge_frame_ref_idx, Some(5));
        assert_eq!(core.frame_type, Some(FrameType::Inter));
        assert_eq!(core.frame_is_intra, Some(false));
        assert!(matches!(
            core.status,
            FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
        ));
        // uvlc(4) is 5 bits; bridge_frame_ref_idx is 3 bits.
        assert_eq!(consumed, 8);
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
        assert_eq!(core.status, FrameHeaderParseStatus::CoreFieldsOnly);
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
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::RegularSef, true, &base_seq()).unwrap();

        assert_eq!(core.show_existing_frame, Some(true));
        assert_eq!(core.frame_to_show_map_idx, Some(2));
        assert_eq!(
            core.order_hint_lsb, None,
            "order hint is derived from the slot, not signaled"
        );
        assert_eq!(core.status, FrameHeaderParseStatus::CoreFieldsOnly);
    }

    #[test]
    fn frame_header_core_inter_stops_after_frame_type() {
        // Regular tile group, frame_is_inter == 1 -> INTER_FRAME; the inter reference
        // map needs reference state, so the parser stops after the frame-type field.
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.bit(1); // frame_is_inter == 1
        let data = bits.into_bytes();
        let (core, consumed) =
            parse_body(&data, ObuType::RegularTileGroup, true, &base_seq()).unwrap();

        assert_eq!(core.frame_type, Some(FrameType::Inter));
        assert_eq!(core.frame_is_intra, Some(false));
        assert!(matches!(
            core.status,
            FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
        ));
        // uvlc(0) + uvlc(0) + frame_is_inter == 3 bits.
        assert_eq!(consumed, 3);
    }

    #[test]
    fn frame_header_core_ras_reads_num_key_ref_frames_then_stops() {
        // RAS frame: restricted_prediction_switch f(1), then (long_term_frame_id_bits
        // != 0) num_key_ref_frames f(3) and the ref_long_term_id loop, before the
        // parser stops as a non-intra (switch) frame (AV2 § 5.18.2).
        let mut seq = base_seq();
        seq.long_term_frame_id_bits = 4;
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        bits.bit(0); // restricted_prediction_switch
        bits.f(2, 3); // num_key_ref_frames == 2
        bits.f(5, 4); // ref_long_term_id[0]
        bits.f(9, 4); // ref_long_term_id[1]
        let data = bits.into_bytes();
        let (core, consumed) = parse_body(&data, ObuType::RasFrame, true, &seq).unwrap();

        assert_eq!(core.frame_type, Some(FrameType::Switch));
        assert_eq!(core.frame_is_intra, Some(false));
        assert!(matches!(
            core.status,
            FrameHeaderParseStatus::UnsupportedUntilFeature { .. }
        ));
        // uvlc(0)+uvlc(0) (2) + restricted_prediction_switch (1) + num_key_ref (3) +
        // 2 * ref_long_term_id f(4) (8) == 14 bits.
        assert_eq!(consumed, 2 + 1 + 3 + 8);
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
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::OpenLoopKey, true, &seq).unwrap();

        assert_eq!(core.frame_type, Some(FrameType::Key));
        assert_eq!(core.frame_is_intra, Some(true));
        assert_eq!(core.immediate_output_frame, Some(false));
        assert_eq!(core.implicit_output_frame, Some(false));
        assert_eq!(core.order_hint_lsb, Some(2));
        assert_eq!(core.refresh_frame_flags, Some(0b0000_0101));
        assert_eq!(core.frame_size, Some(FrameSize::new(4096, 2304)));
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::StoppedBeforeLoopRestorationParams
        );
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
        )
            .prop_map(|(general, quant, seg, tile, filter)| {
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
                    quant,
                    seg,
                    tile,
                    filter,
                }
            })
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
                let _ = parse_core_body(&mut reader, &mut core, &seq, mfh_view.as_ref());
                prop_assert!(reader.consumed_bits() <= (data.len() as u64) * 8);
            }
        }
    }
}
