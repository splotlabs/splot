// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Composing AV2 **intra frame-header** writer (`ENC-BITSTREAM-WRITER`) — the exact
//! inverse of [`parse_frame_header_core`](crate::headers::frame::parse_frame_header_core)
//! on the path that reaches
//! [`FrameHeaderParseStatus::IntraHeaderComplete`].
//!
//! [`write_frame_header_core`] emits the whole intra `frame_header_info()` (AV2 v1.0.0
//! § 5.18.2, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`) in read order: the
//! § 5.18.2 activation prefix (delegated to [`write_frame_header_prefix`]), the
//! frame-type-dependent control-region "glue" bits written directly here (the frame-type
//! arm, the long-term-id reads, the output-control flags, `order_hint`,
//! `refresh_frame_flags`, and `disable_cdf_update`), and each § 5.18 sub-structure
//! delegated to its existing sub-writer (`frame_size()`, `screen_content_params()`,
//! `intrabc_params()`, `tile_info()`, the quantization / segmentation cluster, the
//! loop-filter cluster, loop restoration / CCSO, and the § 5.18.2 intra tail).
//!
//! Only a model the parser could have produced on the `IntraHeaderComplete` path is
//! accepted: an internal pre-write check runs **in full before the first bit** and rejects
//! a non-intra status, a show-existing / inter model, a missing required `Option`, a
//! control-region inferred value that disagrees with its derivation, or an out-of-domain
//! coded field, with a typed
//! [`WriteError::NonCanonicalFrameHeader`]. Because the glue bits precede the delegated
//! sub-writers (each of which also validates before its own first bit), the whole header
//! is drafted into a **scratch [`BitWriter`]** first; the caller's `writer` is appended to
//! only on full success, so a sub-writer reject mid-compose never leaves a partial buffer
//! (the caller's writer is untouched, `bit_len() == 0`).

use crate::headers::frame::FrameTailInput;
use crate::headers::frame::{
    CoreSeqView, FrameHeaderCore, FrameHeaderParseStatus, FrameType, GdfGeometry, LrGeometry,
    MfhFrameView, ceil_log2,
};
use crate::types::ObuType;
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};
use crate::write::frame_config::{
    write_frame_size, write_intrabc_params, write_screen_content_params,
};
use crate::write::frame_filters::{
    write_cdef_params, write_deblocking_filter_params, write_gdf_params,
};
use crate::write::frame_header::write_frame_header_prefix;
use crate::write::frame_quant::{
    write_delta_q_params, write_lossless_info, write_quantization_params, write_setup_qm_params,
};
use crate::write::frame_restoration::{write_ccso_params, write_lr_params};
use crate::write::frame_segmentation::write_segmentation_params;
use crate::write::frame_tail::write_intra_tail;
use crate::write::frame_tiling::write_tile_info;

/// Rejects a [`FrameHeaderCore`] with a stable `what` label.
fn reject<T>(what: &'static str) -> WriteResult<T> {
    Err(WriteError::NonCanonicalFrameHeader { what })
}

/// Returns `f(n)` writability: `value` fits an `n`-bit fixed field (`n == 0` accepts only
/// `0`, matching the parser's `read_f` no-bit inference).
fn fits_in_f(value: u32, n: u32) -> bool {
    if n == 0 {
        value == 0
    } else if n >= u32::BITS {
        true
    } else {
        value < (1u32 << n)
    }
}

/// `allFrames = (1 << NumRefFrames) - 1` (AV2 § 5.18.2), saturating defensively — the
/// inverse-side mirror of the parser's `all_frames_mask`.
fn all_frames_mask(num_ref_frames: u32) -> u32 {
    if num_ref_frames >= u32::BITS {
        u32::MAX
    } else {
        (1u32 << num_ref_frames).wrapping_sub(1)
    }
}

/// How `refresh_frame_flags` is coded for the model's frame type and sequence flags
/// (AV2 § 5.18.2, the inverse of `read_refresh_frame_flags`). Computed once during
/// validation and reused when emitting the glue, so the two never disagree.
#[derive(Debug, Clone, Copy)]
enum RefreshFlagsArm {
    /// KEY closed-loop with `max_mlayer_id == 0`: `allFrames` inferred, no bit.
    InferredAllFrames,
    /// `frame_to_refresh` `f(CeilLog2(NumRefFrames))` such that
    /// `refresh_frame_flags == 1 << frame_to_refresh` (KEY short-refresh arm).
    ShortKey { frame_to_refresh: u32 },
    /// `has_refresh_frame_flags` `f(1)` then, when set, `frame_to_refresh`
    /// `f(CeilLog2(NumRefFrames))` (INTRA_ONLY short-refresh arm).
    ShortIntraOnly {
        has_flags: bool,
        frame_to_refresh: u32,
    },
    /// `refresh_frame_flags` `f(NumRefFrames)` direct (non-short arm).
    Direct,
}

/// Derives, and validates the encodability of, the `refresh_frame_flags` coding arm for the
/// model's frame type and sequence flags (AV2 § 5.18.2). Returns the arm to emit, or rejects
/// when the stored `refresh_frame_flags` cannot be represented by the selected arm.
fn refresh_flags_arm(
    seq: &CoreSeqView,
    obu_type: ObuType,
    frame_type: FrameType,
    refresh_frame_flags: u32,
) -> WriteResult<RefreshFlagsArm> {
    let bits = ceil_log2(seq.num_ref_frames);
    // A single-set-bit `refresh_frame_flags` whose index fits `CeilLog2(NumRefFrames)`.
    let single_bit_index = |flags: u32| -> Option<u32> {
        if flags == 0 || !flags.is_power_of_two() {
            return None;
        }
        let index = flags.trailing_zeros();
        // `1 << frame_to_refresh` with `frame_to_refresh` coded in `bits`; the parser reads
        // `frame_to_refresh < 2^bits`, and `1u32.wrapping_shl(index)` must equal `flags`.
        if fits_in_f(index, bits) && 1u32.wrapping_shl(index) == flags {
            Some(index)
        } else {
            None
        }
    };

    if frame_type == FrameType::Key {
        if obu_type == ObuType::ClosedLoopKey && seq.max_mlayer_id == 0 {
            // No bit: the value is forced to `allFrames`.
            if refresh_frame_flags != all_frames_mask(seq.num_ref_frames) {
                return reject("refresh_frame_flags");
            }
            Ok(RefreshFlagsArm::InferredAllFrames)
        } else if seq.enable_short_refresh_frame_flags {
            match single_bit_index(refresh_frame_flags) {
                Some(frame_to_refresh) => Ok(RefreshFlagsArm::ShortKey { frame_to_refresh }),
                None => reject("refresh_frame_flags"),
            }
        } else if fits_in_f(refresh_frame_flags, seq.num_ref_frames) {
            Ok(RefreshFlagsArm::Direct)
        } else {
            reject("refresh_frame_flags")
        }
    } else if seq.enable_short_refresh_frame_flags {
        // INTRA_ONLY_FRAME short-refresh arm: a `0` value is the `has_refresh_frame_flags == 0`
        // form (no `frame_to_refresh`); a single set bit is the `== 1` form.
        if refresh_frame_flags == 0 {
            Ok(RefreshFlagsArm::ShortIntraOnly {
                has_flags: false,
                frame_to_refresh: 0,
            })
        } else {
            match single_bit_index(refresh_frame_flags) {
                Some(frame_to_refresh) => Ok(RefreshFlagsArm::ShortIntraOnly {
                    has_flags: true,
                    frame_to_refresh,
                }),
                None => reject("refresh_frame_flags"),
            }
        }
    } else if fits_in_f(refresh_frame_flags, seq.num_ref_frames) {
        Ok(RefreshFlagsArm::Direct)
    } else {
        reject("refresh_frame_flags")
    }
}

/// Emits the `refresh_frame_flags` glue for the selected arm (AV2 § 5.18.2), the inverse of
/// `read_refresh_frame_flags`. `bits = CeilLog2(NumRefFrames)`.
fn write_refresh_frame_flags(
    writer: &mut BitWriter,
    seq: &CoreSeqView,
    arm: RefreshFlagsArm,
    refresh_frame_flags: u32,
) -> WriteResult<()> {
    let bits = ceil_log2(seq.num_ref_frames);
    match arm {
        RefreshFlagsArm::InferredAllFrames => Ok(()),
        RefreshFlagsArm::ShortKey { frame_to_refresh } => write_f(writer, frame_to_refresh, bits),
        RefreshFlagsArm::ShortIntraOnly {
            has_flags,
            frame_to_refresh,
        } => {
            writer.write_bit(u8::from(has_flags))?;
            if has_flags {
                write_f(writer, frame_to_refresh, bits)?;
            }
            Ok(())
        }
        RefreshFlagsArm::Direct => write_f(writer, refresh_frame_flags, seq.num_ref_frames),
    }
}

/// Writes `f(n)`, treating `n == 0` as no bits (the value must be `0`), mirroring the
/// parser's `read_f` (AV2 v1.0.0 § 4.11.2,
/// `docs/spec/av2/1.0.0/04-conventions.md#s-4-11-2`).
fn write_f(writer: &mut BitWriter, value: u32, n: u32) -> WriteResult<()> {
    if n == 0 {
        // An `f(0)` field codes no bits; the value is inferred `0`.
        return Ok(());
    }
    writer.write_bits(value, n)
}

/// The fully-validated control-region facts a [`FrameHeaderCore`] carries on the
/// `IntraHeaderComplete` path, gathered once by [`check_frame_header_core_encodable`] so the
/// emit step never re-unwraps an `Option` or re-derives an arm.
struct IntraGlue {
    frame_type: FrameType,
    single_picture: bool,
    immediate_output_frame: bool,
    implicit_output_frame: bool,
    frame_size_override_flag: bool,
    order_hint_lsb: u32,
    refresh_frame_flags: u32,
    refresh_arm: RefreshFlagsArm,
    disable_cdf_update: bool,
    coded_lossless: bool,
}

/// Validates that `core` is a [`FrameHeaderCore`] the § 5.18.2
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`) parser could have produced on
/// the [`FrameHeaderParseStatus::IntraHeaderComplete`] path, gathering the control-region
/// facts the writer emits. Runs fully **before any bit** so the composition is
/// reject-before-write as a whole.
///
/// Rejects (with [`WriteError::NonCanonicalFrameHeader`], `what` naming the field): a status
/// other than `IntraHeaderComplete`; a non-intra / show-existing / inter model; any required
/// `Option` that is `None` on the intra path; a set `lr_params_partial`; a control-region
/// inferred value that disagrees with its derivation; a `refresh_frame_flags` the selected
/// arm cannot represent; or an out-of-domain coded field.
fn check_frame_header_core_encodable(
    core: &FrameHeaderCore,
    seq: &CoreSeqView,
    mfh: Option<&MfhFrameView>,
) -> WriteResult<IntraGlue> {
    // --- status / path gating -------------------------------------------------------
    if core.status != FrameHeaderParseStatus::IntraHeaderComplete {
        return reject("status");
    }
    if core.frame_is_intra != Some(true) {
        return reject("frame_is_intra");
    }
    if core.show_existing_frame == Some(true) {
        return reject("show_existing_frame");
    }
    // `IntraHeaderComplete` always carries a complete `lr_params()`; a set partial means the
    // parse stopped before the Wiener bank (a different status), so reject it.
    if core.lr_params_partial.is_some() {
        return reject("lr_params_partial");
    }

    let Some(frame_type) = core.frame_type else {
        return reject("frame_type");
    };
    // The intra path is only KEY or INTRA_ONLY; SWITCH/INTER are inter (rejected) and TIP is
    // inter. A model whose frame_type is not an intra type could not be IntraHeaderComplete.
    let frame_is_intra = matches!(frame_type, FrameType::Key | FrameType::IntraOnly);
    if !frame_is_intra {
        return reject("frame_type");
    }
    let obu_type = core.obu_type;
    // TIP frames are inter; a SEF obu_type never reaches the intra tail. The intra path is a
    // CLK/OLK key frame, a switch/RAS/regular frame derived to IntraOnly, or a single-picture
    // (any non-SEF, non-TIP) frame. Reject obu_types that the intra path can never carry.
    if obu_type.is_sef() || obu_type.is_tip_frame() {
        return reject("frame_type");
    }

    // --- single-picture inference ---------------------------------------------------
    let single_picture = seq.single_picture_header_flag;

    // --- required Option presence ---------------------------------------------------
    // Every Option below is `Some` on the IntraHeaderComplete path; a `None` is a model the
    // parser could not have produced. Validate presence up front (before any sub-structure
    // check) so the reject is attributed to the exact missing field.
    let Some(immediate_output_frame) = core.immediate_output_frame else {
        return reject("immediate_output_frame");
    };
    let Some(implicit_output_frame) = core.implicit_output_frame else {
        return reject("implicit_output_frame");
    };
    let Some(frame_size_override_flag) = core.frame_size_override_flag else {
        return reject("frame_size_override_flag");
    };
    let Some(order_hint_lsb) = core.order_hint_lsb else {
        return reject("order_hint_lsb");
    };
    let Some(refresh_frame_flags) = core.refresh_frame_flags else {
        return reject("refresh_frame_flags");
    };
    let Some(disable_cdf_update) = core.disable_cdf_update else {
        return reject("disable_cdf_update");
    };
    if core.allow_screen_content_tools.is_none() {
        return reject("allow_screen_content_tools");
    }
    if core.force_integer_mv.is_none() {
        return reject("force_integer_mv");
    }
    if core.allow_intrabc.is_none() {
        return reject("allow_intrabc");
    }
    if core.intrabc.is_none() {
        return reject("intrabc");
    }
    if core.frame_size.is_none() {
        return reject("frame_size");
    }
    if core.tile_info.is_none() {
        return reject("tile_info");
    }
    if core.quantization_params.is_none() {
        return reject("quantization_params");
    }
    if core.segmentation_params.is_none() {
        return reject("segmentation_params");
    }
    if core.setup_qm_params.is_none() {
        return reject("setup_qm_params");
    }
    if core.delta_q_params.is_none() {
        return reject("delta_q_params");
    }
    let Some(lossless_info) = core.lossless_info.as_ref() else {
        return reject("lossless_info");
    };
    if core.deblocking_filter_params.is_none() {
        return reject("deblocking_filter_params");
    }
    if core.gdf_params.is_none() {
        return reject("gdf_params");
    }
    if core.cdef_params.is_none() {
        return reject("cdef_params");
    }
    if core.lr_params.is_none() {
        return reject("lr_params");
    }
    if core.ccso_params.is_none() {
        return reject("ccso_params");
    }
    if core.intra_tail.is_none() {
        return reject("intra_tail");
    }
    // film_grain_config() consumes film_grain_params_present; IntraHeaderComplete implies it
    // was known (the parser stops honestly otherwise), so a None view here is non-canonical.
    if seq.film_grain_params_present.is_none() {
        return reject("film_grain_params_present");
    }

    // --- control-region inferred values vs derivation -------------------------------
    // immediate_output_frame: inferred `false` for an OLK (no bit), else coded. The
    // single-picture branch forces immediate_output_frame = 1 BEFORE the frame-type block (so an
    // OLK single-picture frame correctly carries `true`); only the non-single path infers the
    // OLK `false`, so the disagreement check is gated on `!single_picture` to match the emit.
    if !single_picture && obu_type == ObuType::OpenLoopKey && immediate_output_frame {
        return reject("immediate_output_frame");
    }
    // implicit_output_frame: inferred `false` when immediate_output_frame ||
    // monotonic_output_order_flag (no bit), else coded.
    if (immediate_output_frame || seq.monotonic_output_order_flag) && implicit_output_frame {
        return reject("implicit_output_frame");
    }
    // frame_size_override_flag: inferred `false` for a single-picture key frame (no bit).
    if single_picture && frame_size_override_flag {
        return reject("frame_size_override_flag");
    }
    // order_hint f(OrderHintBits): the stored LSBs must fit the coded width.
    if !fits_in_f(order_hint_lsb, seq.order_hint_bits) {
        return reject("order_hint_lsb");
    }

    // --- frame-type arm coded fields ------------------------------------------------
    // restricted_prediction_switch f(1) for SWITCH/RAS; a key frame never has it. (A SWITCH
    // or RAS frame derives to Switch/Inter, not an intra type, so on the intra path only a
    // single-picture frame can carry these obu_types — its frame_type is forced to KEY and
    // the bit is NOT read. So restricted_prediction_switch must be None on the intra path.)
    if core.restricted_prediction_switch.is_some() {
        return reject("restricted_prediction_switch");
    }
    // The single-picture branch returns before the frame-type / long-term reads (the whole
    // show-existing / frame-type / long-term / output block is skipped, mirror :4131-4142),
    // so long_term_id / ref_long_term_ids are never read and stay at their init defaults
    // (`None` / empty). Validate them only on the non-single-picture path.
    if !single_picture {
        // long_term_id (KEY): long_term_id + 1 == long_term_id_plus_1 must fit
        // f(long_term_frame_id_bits). For non-KEY intra (INTRA_ONLY) long_term_id is the `-1`
        // sentinel and codes nothing.
        if frame_type == FrameType::Key {
            let Some(long_term_id) = core.long_term_id else {
                return reject("long_term_id");
            };
            // `checked_add` so a constructed `long_term_id == i64::MAX` rejects rather than
            // panicking on the increment (workspace `overflow-checks = true` traps it).
            let Some(plus_1) = long_term_id.checked_add(1) else {
                return reject("long_term_id");
            };
            if plus_1 < 0
                || u64::try_from(plus_1)
                    .map(|v| v > u64::from(u32::MAX))
                    .unwrap_or(true)
            {
                return reject("long_term_id");
            }
            // `plus_1` is in `0..=u32::MAX`; the cast is exact.
            if !fits_in_f(plus_1 as u32, seq.long_term_frame_id_bits) {
                return reject("long_term_id");
            }
        }
    }
    // ref_long_term_id[i] (RAS / OLK with long_term_frame_id_bits != 0): num_key_ref_frames
    // f(3) then each id f(long_term_frame_id_bits). On the intra path this is reachable only
    // for an OLK (a RAS frame is not intra), and never under single_picture (the block is
    // skipped). The list length must fit f(3) and each id its field; a list present without
    // the gate, or absent with it, is non-canonical.
    let ref_lt_coded = !single_picture
        && (obu_type == ObuType::RasFrame || obu_type == ObuType::OpenLoopKey)
        && seq.long_term_frame_id_bits != 0;
    if ref_lt_coded {
        let len = core.ref_long_term_ids.len();
        if !fits_in_f(u32::try_from(len).unwrap_or(u32::MAX), 3) {
            return reject("ref_long_term_ids");
        }
        for &id in &core.ref_long_term_ids {
            if !fits_in_f(id, seq.long_term_frame_id_bits) {
                return reject("ref_long_term_ids");
            }
        }
    } else if !core.ref_long_term_ids.is_empty() {
        return reject("ref_long_term_ids");
    }

    // --- refresh_frame_flags arm ----------------------------------------------------
    let refresh_arm = refresh_flags_arm(seq, obu_type, frame_type, refresh_frame_flags)?;

    let coded_lossless = lossless_info.coded_lossless;

    // The `mfh` requirement: a cur_mfh_id > 0 intra frame needs the resolved MFH view to
    // invert the segmentation / deblocking / default-dimension arms. IntraHeaderComplete on a
    // cur_mfh_id > 0 frame implies the parser had the record, so a missing view is a model the
    // parser could not have produced.
    if !core.cur_mfh_id.is_zero() && mfh.is_none() {
        return reject("mfh_record");
    }

    Ok(IntraGlue {
        frame_type,
        single_picture,
        immediate_output_frame,
        implicit_output_frame,
        frame_size_override_flag,
        order_hint_lsb,
        refresh_frame_flags,
        refresh_arm,
        disable_cdf_update,
        coded_lossless,
    })
}

/// Writes the whole intra `frame_header_info()` (AV2 v1.0.0 § 5.18.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`), the exact inverse of
/// [`parse_frame_header_core`](crate::headers::frame::parse_frame_header_core) on the
/// [`FrameHeaderParseStatus::IntraHeaderComplete`] path.
///
/// `seq` is the active sequence-derived view ([`CoreSeqView::from_sequence`]); `mfh` is the
/// resolved multi-frame-header view ([`MfhFrameView::from_record`]) for a `cur_mfh_id > 0`
/// frame, `None` for the `cur_mfh_id == 0` direct path. The whole header is drafted into a
/// scratch [`BitWriter`] and appended to `writer` only on full success, so any reject — the
/// internal pre-write check **or** a delegated sub-writer's own check — leaves `writer`
/// untouched (reject-before-write for the entire composition).
///
/// # Errors
/// - [`WriteError::NonCanonicalFrameHeader`] if `core` is not a model the § 5.18.2 parser
///   could have produced on the intra path (wrong status, a show-existing / inter model, a
///   missing required `Option`, an inferred value that disagrees with its derivation, a
///   `refresh_frame_flags` the selected arm cannot represent, or an out-of-domain coded
///   field), or if a delegated sub-writer rejects its sub-structure.
/// - Any other [`WriteError`] a delegated sub-writer or descriptor raises (e.g. a width that
///   overflows its `f(n)` field).
pub fn write_frame_header_core(
    writer: &mut BitWriter,
    core: &FrameHeaderCore,
    seq: &CoreSeqView,
    mfh: Option<&MfhFrameView>,
) -> WriteResult<()> {
    let glue = check_frame_header_core_encodable(core, seq, mfh)?;

    // Draft the whole header into a scratch writer; commit to the caller's `writer` only on
    // full success so a sub-writer reject mid-compose never leaves a partial buffer.
    let mut scratch = BitWriter::new();
    write_intra_header_into(&mut scratch, core, seq, mfh, &glue)?;
    writer.append(&scratch)
}

/// Emits the validated intra frame header into `scratch` in § 5.18.2 read order. Every
/// `Option` and arm consumed here was validated by [`check_frame_header_core_encodable`];
/// the `ok_or` fallbacks keep the writer panic-free under direct misuse without inventing
/// bits.
fn write_intra_header_into(
    scratch: &mut BitWriter,
    core: &FrameHeaderCore,
    seq: &CoreSeqView,
    mfh: Option<&MfhFrameView>,
    glue: &IntraGlue,
) -> WriteResult<()> {
    let obu_type = core.obu_type;

    // 1. Activation prefix — reconstruct a FrameHeaderPrefix from the core's prefix fields
    //    and delegate to the prefix writer (its own check then re-validates the derivation).
    let prefix = reconstruct_prefix(core)?;
    write_frame_header_prefix(scratch, &prefix)?;

    // 2. Control-region glue (frame-type-dependent; no existing sub-writer).
    //
    // A bridge frame reads bridge_frame_ref_idx f(CeilLog2(NumRefFrames)) before the
    // single-picture branch. On the IntraHeaderComplete path a bridge is reachable only
    // through single_picture_header_flag (a non-single bridge takes the inter path), so it
    // reads bridge_frame_ref_idx, then the single-picture intra tail.
    if core.is_bridge {
        let idx = core
            .bridge_frame_ref_idx
            .ok_or(WriteError::NonCanonicalFrameHeader {
                what: "bridge_frame_ref_idx",
            })?;
        let bits = ceil_log2(seq.num_ref_frames);
        if !fits_in_f(idx, bits) {
            return reject("bridge_frame_ref_idx");
        }
        write_f(scratch, idx, bits)?;
    }

    if !glue.single_picture {
        // Frame-type arm: only the non-single-picture intra arms are reachable here.
        // - CLK / OLK -> KEY, no bit.
        // - other non-key/non-tip non-switch/non-ras -> frame_is_inter f(1); IntraOnly => 0.
        // (SWITCH/RAS would derive to Switch/Inter, not an intra type, so they are not
        //  reachable on a non-single-picture intra path — rejected in validation.)
        let frame_is_key_obu =
            obu_type == ObuType::ClosedLoopKey || obu_type == ObuType::OpenLoopKey;
        if !frame_is_key_obu {
            // The remaining intra arm is the generic `frame_is_inter f(1)` read with a `0`
            // (IntraOnly) value. A switch/ras obu_type cannot be intra and non-single-picture.
            if obu_type == ObuType::Switch || obu_type == ObuType::RasFrame {
                return reject("frame_type");
            }
            scratch.write_bit(0)?; // frame_is_inter == 0 -> INTRA_ONLY_FRAME
        }

        // long_term_id_plus_1 (KEY) / num_key_ref_frames + ref_long_term_id[i] (OLK).
        if glue.frame_type == FrameType::Key {
            let long_term_id = core
                .long_term_id
                .ok_or(WriteError::NonCanonicalFrameHeader {
                    what: "long_term_id",
                })?;
            // Validated in range; `long_term_id + 1` is `0..=u32::MAX`.
            let plus_1 = u32::try_from(long_term_id + 1).map_err(|_| {
                WriteError::NonCanonicalFrameHeader {
                    what: "long_term_id",
                }
            })?;
            write_f(scratch, plus_1, seq.long_term_frame_id_bits)?;
        }
        let ref_lt_coded = (obu_type == ObuType::RasFrame || obu_type == ObuType::OpenLoopKey)
            && seq.long_term_frame_id_bits != 0;
        if ref_lt_coded {
            let num = u32::try_from(core.ref_long_term_ids.len()).map_err(|_| {
                WriteError::NonCanonicalFrameHeader {
                    what: "ref_long_term_ids",
                }
            })?;
            write_f(scratch, num, 3)?;
            for &id in &core.ref_long_term_ids {
                write_f(scratch, id, seq.long_term_frame_id_bits)?;
            }
        }

        // Output-control flags.
        if obu_type != ObuType::OpenLoopKey {
            scratch.write_bit(u8::from(glue.immediate_output_frame))?;
        }
        if !(glue.immediate_output_frame || seq.monotonic_output_order_flag) {
            scratch.write_bit(u8::from(glue.implicit_output_frame))?;
        }
    }

    // 3. parse_intra_tail order: frame_size_override_flag, order_hint, refresh_frame_flags,
    //    frame_size(), screen_content_params(), intrabc_params(), disable_cdf_update.
    if !glue.single_picture {
        scratch.write_bit(u8::from(glue.frame_size_override_flag))?;
    }
    write_f(scratch, glue.order_hint_lsb, seq.order_hint_bits)?;
    write_refresh_frame_flags(scratch, seq, glue.refresh_arm, glue.refresh_frame_flags)?;

    // frame_size(): default dims for the non-override path (cur_mfh_id == 0 -> seq maxima,
    // else MfhFrameView::default_dims).
    let default_dims = if core.cur_mfh_id.is_zero() {
        Some((seq.max_frame_width, seq.max_frame_height))
    } else {
        mfh.map(|view| view.default_dims)
    };
    let frame_size = core
        .frame_size
        .ok_or(WriteError::NonCanonicalFrameHeader { what: "frame_size" })?;
    write_frame_size(
        scratch,
        &frame_size,
        glue.frame_size_override_flag,
        seq.frame_width_bits,
        seq.frame_height_bits,
        default_dims,
    )?;

    let allow_scc = core
        .allow_screen_content_tools
        .ok_or(WriteError::NonCanonicalFrameHeader {
            what: "allow_screen_content_tools",
        })?;
    let force_imv = core
        .force_integer_mv
        .ok_or(WriteError::NonCanonicalFrameHeader {
            what: "force_integer_mv",
        })?;
    write_screen_content_params(
        scratch,
        allow_scc,
        force_imv,
        seq.seq_force_screen_content_tools,
        seq.seq_force_integer_mv,
    )?;

    let intrabc = core
        .intrabc
        .as_ref()
        .ok_or(WriteError::NonCanonicalFrameHeader { what: "intrabc" })?;
    write_intrabc_params(scratch, intrabc, true, seq.allow_frame_max_bvp_drl_bits)?;

    // disable_cdf_update f(1) — the intra-path bit read right after intrabc_params().
    scratch.write_bit(u8::from(glue.disable_cdf_update))?;

    // 4. Structure cluster: tile_info(), quantization_params(), segmentation_params(),
    //    setup_qm_params(), delta_q_params(), lossless tail.
    let tile_info = core
        .tile_info
        .as_ref()
        .ok_or(WriteError::NonCanonicalFrameHeader { what: "tile_info" })?;
    write_tile_info(
        scratch, tile_info, &seq.tile, frame_size, true, false, false,
    )?;

    let quantization =
        core.quantization_params
            .as_ref()
            .ok_or(WriteError::NonCanonicalFrameHeader {
                what: "quantization_params",
            })?;
    write_quantization_params(scratch, quantization, &seq.quant, false)?;

    let segmentation =
        core.segmentation_params
            .as_ref()
            .ok_or(WriteError::NonCanonicalFrameHeader {
                what: "segmentation_params",
            })?;
    let mfh_seg = mfh.and_then(|view| view.seg.as_ref());
    write_segmentation_params(scratch, segmentation, &seq.seg, mfh_seg)?;

    let qm = core
        .setup_qm_params
        .as_ref()
        .ok_or(WriteError::NonCanonicalFrameHeader {
            what: "setup_qm_params",
        })?;
    write_setup_qm_params(scratch, qm, &seq.quant, segmentation.segmentation_enabled)?;

    let delta_q = core
        .delta_q_params
        .as_ref()
        .ok_or(WriteError::NonCanonicalFrameHeader {
            what: "delta_q_params",
        })?;
    write_delta_q_params(scratch, delta_q, quantization.base_q_idx)?;

    let lossless = core
        .lossless_info
        .as_ref()
        .ok_or(WriteError::NonCanonicalFrameHeader {
            what: "lossless_info",
        })?;
    write_lossless_info(
        scratch,
        lossless,
        &seq.quant,
        quantization,
        qm,
        delta_q,
        segmentation,
        seq.seg.max_segments,
    )?;

    // 5. Loop-filter cluster: deblocking_filter_params(), gdf_params(), cdef_params().
    let coded_lossless = glue.coded_lossless;
    let deblocking =
        core.deblocking_filter_params
            .as_ref()
            .ok_or(WriteError::NonCanonicalFrameHeader {
                what: "deblocking_filter_params",
            })?;
    let mfh_deblocking = mfh.map(|view| &view.deblocking);
    write_deblocking_filter_params(
        scratch,
        deblocking,
        coded_lossless,
        seq.quant.num_planes,
        seq.filter.df_par_bits_minus_2,
        mfh_deblocking,
    )?;

    let gdf = core
        .gdf_params
        .as_ref()
        .ok_or(WriteError::NonCanonicalFrameHeader { what: "gdf_params" })?;
    // GdfGeometry mirrors the parser's construction in parse_filter_cluster: SbSize for an
    // intra frame, MiCols/MiRows from the start-array last() sentinels (0 when absent), and
    // the non-sentinel per-tile start slices.
    let geometry = GdfGeometry {
        sb_size: seq.tile.frame_sb_size(true),
        mi_cols: tile_info.mi_col_starts.last().copied().unwrap_or(0),
        mi_rows: tile_info.mi_row_starts.last().copied().unwrap_or(0),
        tile_cols: tile_info.tile_cols,
        tile_rows: tile_info.tile_rows,
        mi_col_starts: &tile_info.mi_col_starts,
        mi_row_starts: &tile_info.mi_row_starts,
    };
    write_gdf_params(scratch, gdf, coded_lossless, &seq.filter, geometry)?;

    let cdef = core
        .cdef_params
        .as_ref()
        .ok_or(WriteError::NonCanonicalFrameHeader {
            what: "cdef_params",
        })?;
    write_cdef_params(
        scratch,
        cdef,
        coded_lossless,
        seq.quant.num_planes,
        &seq.filter,
    )?;

    // 6. Loop restoration + CCSO: lr_params(), ccso_params().
    let lr = core
        .lr_params
        .as_ref()
        .ok_or(WriteError::NonCanonicalFrameHeader { what: "lr_params" })?;
    let lr_geometry = LrGeometry::new(seq.tile.frame_sb_size(true), seq.chroma_format_idc);
    write_lr_params(
        scratch,
        lr,
        coded_lossless,
        seq.quant.num_planes,
        &seq.restoration,
        lr_geometry,
        quantization.base_q_idx,
    )?;

    let ccso = core
        .ccso_params
        .as_ref()
        .ok_or(WriteError::NonCanonicalFrameHeader {
            what: "ccso_params",
        })?;
    write_ccso_params(
        scratch,
        ccso,
        coded_lossless,
        seq.quant.num_planes,
        &seq.ccso,
    )?;

    // 7. § 5.18.2 intra tail: read_tx_mode(), reduced_tx_set, film_grain_config().
    let tail = core
        .intra_tail
        .as_ref()
        .ok_or(WriteError::NonCanonicalFrameHeader { what: "intra_tail" })?;
    let film_grain_params_present =
        seq.film_grain_params_present
            .ok_or(WriteError::NonCanonicalFrameHeader {
                what: "film_grain_params_present",
            })?;
    let tail_input = FrameTailInput {
        coded_lossless,
        film_grain_params_present,
        single_picture_header_flag: seq.single_picture_header_flag,
        immediate_output_frame: glue.immediate_output_frame,
        implicit_output_frame: glue.implicit_output_frame,
    };
    write_intra_tail(scratch, tail, &tail_input)?;

    Ok(())
}

/// Reconstructs the [`FrameHeaderPrefix`](crate::headers::frame::FrameHeaderPrefix) the
/// activation-prefix parser would have produced for `core` (AV2 § 5.18.2), so the prefix
/// writer can emit and re-validate it. `consumed_bits` is recomputed by writing the
/// canonical activation `uvlc` fields to a scratch writer (the same bit count the prefix
/// writer derives), and `starts_cvs` is the prefix's `Option<bool>` form of the core's
/// concrete flag.
fn reconstruct_prefix(
    core: &FrameHeaderCore,
) -> WriteResult<crate::headers::frame::FrameHeaderPrefix> {
    use crate::headers::frame::{FrameHeaderPrefix, FrameHeaderPrefixStatus};

    // Recompute consumed_bits exactly as the activation fields would code (canonical uvlc).
    let mut probe = BitWriter::new();
    if !core.is_bridge {
        let cur = core.cur_mfh_id.get();
        if cur == u32::MAX {
            return Err(WriteError::ValueOutOfRange {
                descriptor: "uvlc",
                value: i64::from(cur),
            });
        }
        probe.write_uvlc(cur)?;
    }
    if core.cur_mfh_id.is_zero() {
        let raw =
            core.seq_header_id_in_frame_header
                .ok_or(WriteError::NonCanonicalFrameHeader {
                    what: "seq_header_id_in_frame_header",
                })?;
        if raw == u32::MAX {
            return Err(WriteError::ValueOutOfRange {
                descriptor: "uvlc",
                value: i64::from(raw),
            });
        }
        probe.write_uvlc(raw)?;
    }
    let consumed_bits = probe.bit_len();

    Ok(FrameHeaderPrefix {
        obu_type: core.obu_type,
        is_first: core.is_first,
        is_key_frame: core.is_key_frame,
        is_bridge: core.is_bridge,
        is_regular: core.is_regular,
        // The parser's startCVS is `Some(false)` for every non-CLK type and
        // `Some(first_picture_in_tu)` for a CLK; the core stores the concrete bool, which is
        // exactly that value, so lift it into the prefix's Option form. (A CLK accepts any of
        // Some(true)/Some(false)/None; a non-CLK must be Some(false), which a non-CLK core's
        // `false` satisfies.)
        starts_cvs: Some(core.starts_cvs),
        cur_mfh_id: core.cur_mfh_id,
        seq_header_id_in_frame_header: core.seq_header_id_in_frame_header,
        referenced_sequence_header_id: core.referenced_sequence_header_id,
        consumed_bits,
        status: FrameHeaderPrefixStatus::ActivationFieldsOnly,
    })
}

// The unit / byte-exact / rejection tests and the property tests live in sibling files (the
// advisory source-line limit); `include!` pastes them into this module so their `super::*`
// resolves to the writers and private helpers above.
#[cfg(test)]
include!("frame_header_core_tests.rs");
#[cfg(test)]
include!("frame_header_core_proptests.rs");
