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
//!   region through `frame_size()`, `screen_content_params()`, `intrabc_params()`, and
//!   `disable_cdf_update`, stopping before `tile_info()`
//!   ([`FrameHeaderParseStatus::StoppedBeforeFilteringQuantSegmentation`]).

use crate::bitio::BitReader;
use crate::error::Result;
use crate::headers::frame::config::{parse_intrabc_params, parse_screen_content_params};
use crate::headers::frame::size::{FrameSize, ceil_log2, parse_frame_size};
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
    /// An intra frame's control region was read through `frame_size()`,
    /// `screen_content_params()`, `intrabc_params()`, and `disable_cdf_update`; the
    /// parser stopped before `tile_info()` / `quantization_params()` /
    /// `segmentation_params()` (§ 5.18.5 onward).
    StoppedBeforeFilteringQuantSegmentation,
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
            Self::StoppedBeforeFilteringQuantSegmentation => {
                "stopped_before_filtering_quant_segmentation"
            }
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
    /// Reserved for future `frame_size()` default-dimension resolution; the record
    /// type does not yet carry MFH frame dimensions.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// `bridge_frame_ref_idx`, when read (bridge frames).
    pub bridge_frame_ref_idx: Option<u32>,
    /// `frame_to_show_map_idx`, when read (show-existing-frame).
    pub frame_to_show_map_idx: Option<u32>,
    /// `allow_screen_content_tools`, when `screen_content_params()` was reached.
    pub allow_screen_content_tools: Option<bool>,
    /// `allow_intrabc`, when `intrabc_params()` was reached.
    pub allow_intrabc: Option<bool>,
    /// Bits consumed by this parse (not necessarily the whole frame header).
    pub consumed_bits: u64,
}

/// Matrix Feature ID for the frame-header-info coverage this phase does not model.
const FRAME_HEADER_INFO_FEATURE: &str = "AV2-5.18.2-FRAME-HEADER-INFO";

/// Sequence-derived scalars the core parser needs, gathered from a fully parsed
/// [`SequenceHeader`]. `None` when the inter or screen-content config is absent (the
/// header was not fully parsed), in which case core parsing degrades to the prefix.
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
}

impl CoreSeqView {
    fn from_sequence(seq: &SequenceHeader) -> Option<Self> {
        let inter = seq.inter.as_ref()?;
        let scc = seq.screen_content.as_ref()?;
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
        })
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
        parse_core_body(reader, &mut core, &seq)?;
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
        bridge_frame_ref_idx: None,
        frame_to_show_map_idx: None,
        allow_screen_content_tools: None,
        allow_intrabc: None,
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
        return parse_intra_tail(reader, core, seq, FrameType::Key, true);
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
        let num_key_ref_frames = reader.read_bits(3)?;
        for _ in 0..num_key_ref_frames {
            read_f(reader, seq.long_term_frame_id_bits)?; // ref_long_term_id[i]
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

    parse_intra_tail(reader, core, seq, frame_type, false)
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
/// `screen_content_params()` / `intrabc_params()` and `disable_cdf_update`, stopping
/// before `tile_info()`.
///
/// `single_picture` is `single_picture_header_flag` (forces `frame_size_override_flag
/// = 0`). For an intra frame `primary_ref_frame == PRIMARY_REF_NONE`, so no
/// primary-reference bits are read.
fn parse_intra_tail(
    reader: &mut BitReader<'_>,
    core: &mut FrameHeaderCore,
    seq: &CoreSeqView,
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
    let default_dims = if core.cur_mfh_id.is_zero() {
        Some((seq.max_frame_width, seq.max_frame_height))
    } else {
        // cur_mfh_id > 0 default dimensions come from the multi-frame header, which is
        // not modeled with frame dimensions yet — leave the size unknown.
        None
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

    // Not a TIP-as-output / bru-inactive / bridge frame -> disable_cdf_update f(1),
    // then tile_info() (§ 5.18.7), which this phase stops before.
    reader.read_bit()?; // disable_cdf_update
    core.status = FrameHeaderParseStatus::StoppedBeforeFilteringQuantSegmentation;
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::error::Error;
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
    fn base_seq() -> CoreSeqView {
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
        }
    }

    /// Parses the activation prefix then the core body, returning the result and the
    /// total bits consumed (prefix + body).
    fn parse_body(
        data: &[u8],
        obu_type: ObuType,
        first_picture_in_tu: bool,
        seq: &CoreSeqView,
    ) -> Result<(FrameHeaderCore, u64)> {
        let mut reader = BitReader::new(data, ByteOffset::new(0));
        let prefix = parse_frame_header_prefix(&mut reader, obu_type, first_picture_in_tu)?;
        let mut core = init_core_from_prefix(&prefix, obu_type);
        parse_core_body(&mut reader, &mut core, seq)?;
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
        let data = bits.into_bytes();
        let (core, consumed) =
            parse_body(&data, ObuType::ClosedLoopKey, true, &base_seq()).unwrap();

        assert_eq!(
            core.status,
            FrameHeaderParseStatus::StoppedBeforeFilteringQuantSegmentation
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
        // uvlc(0)=1 + uvlc(1)=3 prefix bits, then 33 core bits (1+1+1+4 control/output,
        // 24 frame_size, 1 allow_intrabc, 1 disable_cdf_update).
        assert_eq!(consumed, 4 + 33);
    }

    #[test]
    fn frame_header_core_reads_mfh_reference_path() {
        // CLK, cur_mfh_id == 2 (resolved via MFH); frame_size_override_flag == 0 leaves
        // the size unknown because cur_mfh_id > 0 default dims come from the MFH.
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
        let (core, _) = parse_body(&data, ObuType::ClosedLoopKey, true, &base_seq()).unwrap();

        assert_eq!(core.cur_mfh_id.get(), 2);
        assert_eq!(core.seq_header_id_in_frame_header, None);
        assert_eq!(core.order_hint_lsb, Some(7));
        assert_eq!(
            core.frame_size, None,
            "cur_mfh_id > 0 default dims are unmodeled"
        );
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::StoppedBeforeFilteringQuantSegmentation
        );
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
        let data = bits.into_bytes();
        let (core, _) = parse_body(&data, ObuType::RegularTileGroup, true, &base_seq()).unwrap();

        assert_eq!(core.frame_type, Some(FrameType::IntraOnly));
        assert_eq!(core.frame_is_intra, Some(true));
        assert_eq!(core.refresh_frame_flags, Some(0b0000_0101));
        assert_eq!(core.frame_size, Some(FrameSize::new(4096, 2304)));
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::StoppedBeforeFilteringQuantSegmentation
        );
    }

    #[test]
    fn frame_header_core_single_picture_path() {
        // single_picture_header_flag skips the frame-type/output block; frame_size uses
        // the default (max) dimensions.
        let mut seq = base_seq();
        seq.single_picture_header_flag = true;
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(0); // seq_header_id_in_frame_header
        // single_picture: no type/output bits; frame_size_override_flag inferred 0
        bits.f(9, 4); // order_hint
        // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
        bits.bit(0); // allow_intrabc
        bits.bit(0); // disable_cdf_update
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
            FrameHeaderParseStatus::StoppedBeforeFilteringQuantSegmentation
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
    use crate::span::ByteOffset;
    use proptest::prelude::*;

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
    }
}
