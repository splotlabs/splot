// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 frame-header **loop-restoration** and **CCSO** writers (`ENC-BITSTREAM-WRITER`) — the
//! inverses of the § 5.18.7.11 / § 5.18.7.12 parsers in [`crate::headers::frame`]:
//!
//! - [`write_lr_params`] — `lr_params()` (§ 5.18.7.11,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-11`): the per-plane `indexToTool`
//!   selection (`tool_index ns(n)`), the `frame_filters_on` gate, and the luma/chroma
//!   `LoopRestorationSize` size-shift signaling.
//! - [`write_ccso_params`] — `ccso_params()` (§ 5.18.7.12,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-12`): `ccso_frame_flag`, the
//!   per-plane `ccso_planes` arm, and the `(d0, d1, band)`-ordered `ccso_offset_idx tu(7)`
//!   table.
//!
//! Like the other frame-header config writers, this module is additive: it depends on the
//! model/parser read-only and serializes a parsed structure back to bits via [`BitWriter`].
//! Each writer threads the same gating inputs the parser receives (the sequence
//! [`CoreSeqRestorationView`] / [`CoreSeqCcsoView`], `coded_lossless`, `num_planes`, the
//! [`LrGeometry`]) and validates the whole structure before any bit is written
//! (reject-before-write): every reject path leaves `writer.bit_len() == 0`.
//!
//! **Hard residual — `frame_filters_on` is unwritable (§ 5.18.7.11).** The parser can model
//! the fixed-coded frame-level `read_wienerns_filter()` bank, but this writer does not yet
//! emit that bank. A model with `frame_filters_on` set is still rejected here
//! ([`WriteError::NonCanonicalFrameHeader`] with `what == "lr_frame_filters_on"`). This
//! writer ships the `frame_filters_on == false` surface only; the `frame_filters_on == true`
//! arm lands with the Wiener-bank writer.

use crate::headers::frame::{
    CCSO_INPUT_INTERVAL, CcsoParams, CoreSeqCcsoView, CoreSeqRestorationView, FrameRestorationType,
    LrGeometry, LrParams, RESTORATION_TILESIZE_MAX, ccso_quant_step, default_restoration_size,
    lr_plane_tool_table,
};
use crate::headers::sequence::SuperblockSize;
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};

/// `ccso_scale_idx` is `f(2)` (AV2 v1.0.0 § 5.18.7.12), so it fits `0..4`.
const CCSO_SCALE_IDX_MAX_PLUS_1: u8 = 4;
/// `ccso_quant_idx` is `f(2)` (AV2 v1.0.0 § 5.18.7.12), so it fits `0..4`.
const CCSO_QUANT_IDX_MAX_PLUS_1: u8 = 4;
/// `ccso_ext_filter` is `f(3)` (AV2 v1.0.0 § 5.18.7.12), so it fits `0..8`.
const CCSO_EXT_FILTER_MAX_PLUS_1: u8 = 8;
/// `ccso_offset_idx` is `tu(7)` (AV2 v1.0.0 § 5.18.7.12), so each value fits `0..=7`.
const CCSO_OFFSET_IDX_MAX: u8 = 7;

/// Writes `lr_params()` (AV2 v1.0.0 § 5.18.7.11,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-11`), the inverse of
/// [`crate::headers::frame::parse_lr_params`] on the `LrParseOutcome::Parsed` surface.
///
/// `base_q_idx` is unused (the parser reads it only for the `get_filter_set_index` derivation,
/// which signals no bits); it is threaded to mirror the parser signature.
///
/// **Hard residual.** A plane carrying `frame_filters_on == true` is rejected up front
/// (`what == "lr_frame_filters_on"`): this writer does not yet emit frame-level
/// `read_wienerns_filter()` bank syntax.
///
/// When `coded_lossless || !view.enable_restoration` the parser writes no bits and leaves
/// `uses_lr == false`, `planes` empty, and `loop_restoration_size == default_restoration_size`,
/// so the model must match. Otherwise the writer emits, per plane, the `tool_index ns(n)`
/// against the shared `indexToTool` table, the `frame_filters_on f(1)` `0`-bit for the
/// `WienerNonsep` / `Switchable` arm, then the luma/chroma size-shift flags. The model is fully
/// validated before any bit is written (reject-before-write).
///
/// # Errors
/// [`WriteError::NonCanonicalFrameHeader`] for any model this writer does not support or the
/// § 5.18.7.11 parser could not have produced: a plane with `frame_filters_on`
/// (`lr_frame_filters_on`); stray bank data (`lr_frame_filter_bank`); a disabled-arm model
/// that is non-default (`lr_disabled`); a plane count that disagrees with `num_planes`
/// (`lr_num_planes`); a restoration type absent from the (tool-disabled) `indexToTool` table
/// (`lr_tool_index`); a non-`None` `num_filter_classes` (`lr_num_filter_classes`); a stored
/// `loop_restoration_size` that is not the inferred default for an unused luma/chroma plane or
/// disagrees with `loop_restoration_size[1]` for plane 2 (`lr_size`); a size that is not an
/// exact power-of-two division of its base or whose shift is unreachable for the frame `SbSize`
/// (`lr_size`); or a `uses_lr` that disagrees with the per-plane derivation (`lr_uses_lr`).
pub fn write_lr_params(
    writer: &mut BitWriter,
    params: &LrParams,
    coded_lossless: bool,
    num_planes: u8,
    view: &CoreSeqRestorationView,
    geometry: LrGeometry,
    base_q_idx: u32,
) -> WriteResult<()> {
    // base_q_idx feeds get_filter_set_index (a SubclassLookup derivation only); no bits.
    let _ = base_q_idx;
    let plan = check_lr_encodable(params, coded_lossless, num_planes, view, geometry)?;

    if plan.disabled {
        // § 5.18.7.11: if ( CodedLossless || !enable_restoration ) no bits.
        return Ok(());
    }

    // § 5.18.7.11: for ( plane = 0; plane < NumPlanes; plane++ ).
    for (plane, plane_params) in params.planes.iter().enumerate() {
        let is_chroma = plane > 0;
        let (index_to_tool, _tools_count, n) = lr_plane_tool_table(view, is_chroma);
        // tool_index ns(n): the position in indexToTool whose value is this plane's tool id
        // (validated present up front).
        let tool = plane_params.restoration_type.to_tool();
        let tool_index = index_to_tool
            .iter()
            .position(|&t| t == tool)
            .map_or(0u32, |idx| idx as u32);
        writer.write_ns(tool_index, n)?;

        // § 5.18.7.11: r == RESTORE_WIENER_NONSEP || r == RESTORE_SWITCHABLE reads
        // frame_filters_on f(1). The hard residual guarantees it is false, so write a 0 bit.
        if matches!(
            plane_params.restoration_type,
            FrameRestorationType::WienerNonsep | FrameRestorationType::Switchable
        ) {
            writer.write_bit(0)?;
        }
    }

    // § 5.18.7.11: the luma/chroma size-shift signaling (after the plane loop).
    if plan.uses_luma_lr {
        // LoopRestorationSize[0] base is RESTORATION_TILESIZE_MAX.
        write_lr_size_shift(writer, plan.luma_shift, geometry.sb_size)?;
    }
    if plan.uses_chroma_lr {
        // LoopRestorationSize[1] base is RESTORATION_TILESIZE_MAX >> Max(SubsamplingX, Y).
        write_lr_size_shift(writer, plan.chroma_shift, geometry.sb_size)?;
    }
    Ok(())
}

/// The re-derived § 5.18.7.11 state the LR writer needs before emitting any bit: whether the
/// structure is disabled, the per-plane `uses_*` derivation, and the luma/chroma size shifts.
struct LrPlan {
    /// `coded_lossless || !enable_restoration`: the no-bits disabled arm.
    disabled: bool,
    /// `usesLumaLr`: plane 0 restoration != RESTORE_NONE (luma size flags coded).
    uses_luma_lr: bool,
    /// `usesChromaLr`: any plane > 0 restoration != RESTORE_NONE (chroma size flags coded).
    uses_chroma_lr: bool,
    /// The luma size shift (`LoopRestorationSize[0] == RESTORATION_TILESIZE_MAX >> shift`);
    /// `0` when `!uses_luma_lr` (unused).
    luma_shift: u32,
    /// The chroma size shift (`LoopRestorationSize[1] == base >> shift`); `0` when
    /// `!uses_chroma_lr` (unused).
    chroma_shift: u32,
}

/// Validates an [`LrParams`] is a model the § 5.18.7.11
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-11`) parser could have produced, and
/// re-derives the [`LrPlan`], before any bit is written.
fn check_lr_encodable(
    params: &LrParams,
    coded_lossless: bool,
    num_planes: u8,
    view: &CoreSeqRestorationView,
    geometry: LrGeometry,
) -> WriteResult<LrPlan> {
    // Hard residual (§ 5.18.7.11 / § 5.20.10.6): the parser can carry a completed
    // frame-level Wiener NS bank, but this writer does not yet emit that syntax.
    if params.planes.iter().any(|p| p.frame_filters_on) {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "lr_frame_filters_on",
        });
    }
    if params.planes.iter().any(|p| p.frame_filter_bank.is_some()) {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "lr_frame_filter_bank",
        });
    }

    // § 6.4.1: the chroma subsampling domain is {0, 1}. `LrGeometry`'s fields are public, so a
    // caller can construct a value outside that domain; the LoopRestorationSize shifts
    // (`RESTORATION_TILESIZE_MAX >> (3 + maxSubsampling)`, incl. inside `default_restoration_size`
    // below) would then shift past the u32 width and panic in debug. Reject it up front so the
    // writer never panics on a hostile geometry (a real stream's geometry comes from
    // `LrGeometry::new`, which clamps to {0, 1}).
    if geometry.subsampling_x > 1 || geometry.subsampling_y > 1 {
        return Err(WriteError::NonCanonicalFrameHeader { what: "lr_size" });
    }

    if coded_lossless || !view.enable_restoration {
        // § 5.18.7.11: the disabled arm leaves UsesLr = 0, no per-plane state, and the default
        // LoopRestorationSize; a non-default model could not have been produced.
        if params.uses_lr
            || !params.planes.is_empty()
            || params.loop_restoration_size != default_restoration_size(geometry)
        {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "lr_disabled",
            });
        }
        return Ok(LrPlan {
            disabled: true,
            uses_luma_lr: false,
            uses_chroma_lr: false,
            luma_shift: 0,
            chroma_shift: 0,
        });
    }

    // § 5.18.7.11: the enabled arm parses exactly NumPlanes per-plane entries.
    if params.planes.len() != usize::from(num_planes) {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "lr_num_planes",
        });
    }

    let mut uses_luma_lr = false;
    let mut uses_chroma_lr = false;
    for (plane, plane_params) in params.planes.iter().enumerate() {
        let is_chroma = plane > 0;
        let (index_to_tool, _tools_count, n) = lr_plane_tool_table(view, is_chroma);
        // The plane's restoration type must be SELECTABLE by the parser's tool_index ns(n): its
        // position in indexToTool must exist AND be < n. RESTORE_SWITCHABLE sits at index
        // toolsCount but is only reachable when allowSwitchable (n == toolsCount + 1); a plain
        // `.contains()` would wrongly accept Switchable when only one switchable tool is enabled
        // (n == toolsCount), and write_ns(toolsCount, toolsCount) would then reject mid-write
        // (partial buffer). Reject up front instead.
        let tool = plane_params.restoration_type.to_tool();
        match index_to_tool.iter().position(|&t| t == tool) {
            Some(idx) if (idx as u32) < n => {}
            _ => {
                return Err(WriteError::NonCanonicalFrameHeader {
                    what: "lr_tool_index",
                });
            }
        }

        // § 5.18.7.11: num_filter_classes is Some only when frame_filters_on[0] (excluded by
        // the hard residual), so it must be None on this surface.
        if plane_params.num_filter_classes.is_some() {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "lr_num_filter_classes",
            });
        }

        if plane_params.restoration_type != FrameRestorationType::None {
            if plane == 0 {
                uses_luma_lr = true;
            } else {
                uses_chroma_lr = true;
            }
        }
    }

    // § 5.18.7.11: UsesLr = usesLumaLr || usesChromaLr.
    if params.uses_lr != (uses_luma_lr || uses_chroma_lr) {
        return Err(WriteError::NonCanonicalFrameHeader { what: "lr_uses_lr" });
    }

    let max_subsampling = u32::from(geometry.subsampling_x.max(geometry.subsampling_y));

    // § 5.18.7.11: LoopRestorationSize[0]. When usesLumaLr the size is signaled (derive its
    // shift); otherwise it stays at its inferred default RESTORATION_TILESIZE_MAX >> 3.
    let luma_shift = if uses_luma_lr {
        lr_size_to_shift(
            params.loop_restoration_size[0],
            RESTORATION_TILESIZE_MAX,
            geometry.sb_size,
        )?
    } else {
        if params.loop_restoration_size[0] != RESTORATION_TILESIZE_MAX >> 3 {
            return Err(WriteError::NonCanonicalFrameHeader { what: "lr_size" });
        }
        0
    };

    // § 5.18.7.11: LoopRestorationSize[1]. Base is RESTORATION_TILESIZE_MAX >> max_subsampling.
    let chroma_base = RESTORATION_TILESIZE_MAX >> max_subsampling;
    let chroma_shift = if uses_chroma_lr {
        lr_size_to_shift(
            params.loop_restoration_size[1],
            chroma_base,
            geometry.sb_size,
        )?
    } else {
        if params.loop_restoration_size[1] != RESTORATION_TILESIZE_MAX >> (3 + max_subsampling) {
            return Err(WriteError::NonCanonicalFrameHeader { what: "lr_size" });
        }
        0
    };

    // § 5.18.7.11: LoopRestorationSize[2] = LoopRestorationSize[1].
    if params.loop_restoration_size[2] != params.loop_restoration_size[1] {
        return Err(WriteError::NonCanonicalFrameHeader { what: "lr_size" });
    }

    Ok(LrPlan {
        disabled: false,
        uses_luma_lr,
        uses_chroma_lr,
        luma_shift,
        chroma_shift,
    })
}

/// Recovers the size `shift` such that `size == base >> shift` (the inverse of
/// `read_lr_size_shift`'s `RESTORATION_TILESIZE_MAX >> shift` derivation, AV2 § 5.18.7.11),
/// then validates the shift is reachable for `sb_size`. Requires `size != 0` and an exact
/// power-of-two division `base == size << shift`; rejects anything else with
/// [`WriteError::NonCanonicalFrameHeader`] (`lr_size`).
fn lr_size_to_shift(size: u32, base: u32, sb_size: SuperblockSize) -> WriteResult<u32> {
    // size != 0 (avoid the divide-by-zero / trailing_zeros(0) traps) and size must divide base
    // exactly as a power of two: base == size << shift for shift = log2(base / size).
    if size == 0 || base < size {
        return Err(WriteError::NonCanonicalFrameHeader { what: "lr_size" });
    }
    let ratio = base / size;
    let shift = ratio.trailing_zeros();
    // Reject a non-power-of-two ratio (base/size not exactly 2^shift) or a non-exact division.
    if size << shift != base {
        return Err(WriteError::NonCanonicalFrameHeader { what: "lr_size" });
    }
    // The shift must be reachable for this SbSize (mirror read_lr_size_shift's inferences).
    lr_shift_is_reachable(shift, sb_size)?;
    Ok(shift)
}

/// Validates a recovered size `shift` is one `read_lr_size_shift` could have produced for the
/// frame `SbSize` (AV2 § 5.18.7.11, mirror :7287-7369): `BLOCK_256X256` reaches only shifts
/// `0` / `1`; `BLOCK_128X128` reaches `0` / `1` / `2`; every other `SbSize` reaches `0..=3`.
fn lr_shift_is_reachable(shift: u32, sb_size: SuperblockSize) -> WriteResult<()> {
    let reachable = match sb_size {
        // half (shift 1) for all; else shift 0 (no max/quarter flags follow).
        SuperblockSize::Block256x256 => shift <= 1,
        // half (1); max (0); else shift 2 (no quarter flag follows).
        SuperblockSize::Block128x128 => shift <= 2,
        // half (1); max (0); quarter (2) : otherwise 3.
        SuperblockSize::Block64x64 => shift <= 3,
    };
    if reachable {
        Ok(())
    } else {
        Err(WriteError::NonCanonicalFrameHeader { what: "lr_size" })
    }
}

/// Writes the luma/chroma restoration size-shift flags (the inverse of `read_lr_size_shift`,
/// AV2 § 5.18.7.11, mirror :7287-7369): the `*_use_half_size` / `*_use_max_size` /
/// `*_use_quarter_size` flag cascade with the same `SbSize`-dependent inferences the reader
/// applies. The `shift` is already validated reachable for `sb_size`.
fn write_lr_size_shift(
    writer: &mut BitWriter,
    shift: u32,
    sb_size: SuperblockSize,
) -> WriteResult<()> {
    // *_use_half_size f(1): shift == 1 (reachable for every SbSize).
    if shift == 1 {
        return writer.write_bit(1);
    }
    writer.write_bit(0)?;
    if sb_size == SuperblockSize::Block256x256 {
        // shift 0 is inferred here; any other shift is unreachable (validated up front).
        return Ok(());
    }
    // *_use_max_size f(1): shift == 0.
    if shift == 0 {
        return writer.write_bit(1);
    }
    writer.write_bit(0)?;
    if sb_size == SuperblockSize::Block128x128 {
        // shift 2 is inferred here; any other shift is unreachable (validated up front).
        return Ok(());
    }
    // *_use_quarter_size f(1): shift == 2 -> 1, shift == 3 -> 0.
    writer.write_flag(shift == 2)
}

/// Writes `ccso_params()` (AV2 v1.0.0 § 5.18.7.12,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-12`), the inverse of
/// [`crate::headers::frame::parse_ccso_params`]. Byte-exact via the surfaced
/// [`ccso_offset_idx`](crate::headers::frame::CcsoPlaneParams::ccso_offset_idx).
///
/// When `coded_lossless || !view.enable_ccso` the parser writes no bits and leaves
/// `ccso_frame_flag == None` with `planes` empty, so the model must match. Otherwise
/// `ccso_frame_flag` is inferred `Some(true)` for a single picture (no bit) or written `f(1)`;
/// when `Some(false)` the structure returns with no further per-plane bits. When `Some(true)`,
/// each plane writes `ccso_planes f(1)` and, when set, the `ccso_bo_only` / `ccso_scale_idx` /
/// `ccso_quant_idx` / `ccso_ext_filter` / `ccso_edge_clf` / `ccso_max_band_log2` arm followed by
/// the `(d0, d1, band)`-ordered `ccso_offset_idx tu(7)` table. The model is fully validated
/// before any bit is written (reject-before-write).
///
/// # Errors
/// [`WriteError::NonCanonicalFrameHeader`] for any model the § 5.18.7.12 parser could not have
/// produced: a disabled-arm model that is non-default (`ccso_disabled`); a single-picture
/// `ccso_frame_flag` that is not `Some(true)` (`ccso_frame_flag`); a missing `ccso_frame_flag`
/// on the coded arm (`ccso_frame_flag`); a non-empty `planes` when the flag is `Some(false)`
/// (`ccso_frame_disabled`); a plane count that disagrees with `num_planes` (`ccso_num_planes`);
/// a `Some` field on a disabled plane or a `None` field on an enabled plane
/// (`ccso_plane_fields`); an out-of-domain `ccso_scale_idx` / `ccso_quant_idx` /
/// `ccso_ext_filter` / `ccso_max_band_log2` (`ccso_*`); an inferred field that is non-default in
/// the `ccso_bo_only` / `quantStep == 0` arms (`ccso_bo_only_fields` / `ccso_edge_clf`); an
/// `ccso_offset_idx` length that disagrees with `maxEdgeInterval^2 * maxBand`
/// (`ccso_offset_idx_len`); or an offset value above `7` (`ccso_offset_idx`).
pub fn write_ccso_params(
    writer: &mut BitWriter,
    params: &CcsoParams,
    coded_lossless: bool,
    num_planes: u8,
    view: &CoreSeqCcsoView,
) -> WriteResult<()> {
    check_ccso_encodable(params, coded_lossless, num_planes, view)?;

    if coded_lossless || !view.enable_ccso {
        // § 5.18.7.12: if ( CodedLossless || !enable_ccso ) no bits.
        return Ok(());
    }

    // § 5.18.7.12: single picture infers ccso_frame_flag = 1 (no bit); else f(1). Validated
    // Some above; pattern-match to avoid an unwrap.
    let Some(frame_flag) = params.ccso_frame_flag else {
        return Ok(());
    };
    if !view.single_picture_header_flag {
        writer.write_flag(frame_flag)?;
    }
    if !frame_flag {
        // § 5.18.7.12: if ( !ccso_frame_flag ) return.
        return Ok(());
    }

    // § 5.18.7.12: for ( plane = 0; plane < NumPlanes; plane++ ).
    for plane in &params.planes {
        // ccso_planes[plane] f(1).
        writer.write_flag(plane.ccso_planes)?;
        if !plane.ccso_planes {
            continue;
        }
        // Every field is Some here (validated up front); pattern-match to avoid unwraps.
        let (
            Some(bo_only),
            Some(scale_idx),
            Some(quant_idx),
            Some(ext_filter),
            Some(edge_clf),
            Some(max_band_log2),
        ) = (
            plane.ccso_bo_only,
            plane.ccso_scale_idx,
            plane.ccso_quant_idx,
            plane.ccso_ext_filter,
            plane.ccso_edge_clf,
            plane.ccso_max_band_log2,
        )
        else {
            // Unreachable after validation; never write a partial plane.
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_plane_fields",
            });
        };

        // ccso_bo_only[plane] f(1); ccso_scale_idx[plane] f(2).
        writer.write_flag(bo_only)?;
        writer.write_bits_u8(scale_idx, 2)?;
        if !bo_only {
            // ccso_quant_idx[plane] f(2); ccso_ext_filter[plane] f(3).
            writer.write_bits_u8(quant_idx, 2)?;
            writer.write_bits_u8(ext_filter, 3)?;
            // quantStep != 0 -> ccso_edge_clf[plane] f(1); else inferred 0 (no bit).
            if ccso_quant_step(scale_idx, quant_idx) != 0 {
                writer.write_flag(edge_clf)?;
            }
        }
        // n = 2 + ccso_bo_only; ccso_max_band_log2[plane] f(n).
        let band_bits = 2 + u32::from(bo_only);
        writer.write_bits_u8(max_band_log2, band_bits)?;

        // maxEdgeInterval = bo_only ? 1 : CCSO_INPUT_INTERVAL - ccso_edge_clf;
        // maxBand = 1 << ccso_max_band_log2. The offset table is (d0, d1, band)-ordered.
        for &offset in &plane.ccso_offset_idx {
            // ccso_offset_idx tu(7) (validated <= 7 up front).
            writer.write_tu(u32::from(offset), u32::from(CCSO_OFFSET_IDX_MAX))?;
        }
    }
    Ok(())
}

/// Validates a [`CcsoParams`] is a model the § 5.18.7.12
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-12`) parser could have produced,
/// before any bit is written.
fn check_ccso_encodable(
    params: &CcsoParams,
    coded_lossless: bool,
    num_planes: u8,
    view: &CoreSeqCcsoView,
) -> WriteResult<()> {
    if coded_lossless || !view.enable_ccso {
        // § 5.18.7.12: the disabled arm leaves ccso_frame_flag None and no planes.
        if params.ccso_frame_flag.is_some() || !params.planes.is_empty() {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_disabled",
            });
        }
        return Ok(());
    }

    // § 5.18.7.12: single picture infers ccso_frame_flag = 1; else it is read f(1) (Some).
    if view.single_picture_header_flag {
        if params.ccso_frame_flag != Some(true) {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_frame_flag",
            });
        }
    } else if params.ccso_frame_flag.is_none() {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "ccso_frame_flag",
        });
    }

    match params.ccso_frame_flag {
        Some(false) => {
            // § 5.18.7.12: if ( !ccso_frame_flag ) return; no per-plane state.
            if !params.planes.is_empty() {
                return Err(WriteError::NonCanonicalFrameHeader {
                    what: "ccso_frame_disabled",
                });
            }
        }
        Some(true) => {
            // § 5.18.7.12: the coded arm parses exactly NumPlanes entries.
            if params.planes.len() != usize::from(num_planes) {
                return Err(WriteError::NonCanonicalFrameHeader {
                    what: "ccso_num_planes",
                });
            }
            for plane in &params.planes {
                check_ccso_plane_encodable(plane)?;
            }
        }
        None => {
            // Only reachable on the non-single-picture arm, already rejected above.
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_frame_flag",
            });
        }
    }
    Ok(())
}

/// Validates one [`CcsoPlaneParams`](crate::headers::frame::CcsoPlaneParams) is a plane the
/// § 5.18.7.12 parser could have produced, before any bit is written.
fn check_ccso_plane_encodable(plane: &crate::headers::frame::CcsoPlaneParams) -> WriteResult<()> {
    if !plane.ccso_planes {
        // § 5.18.7.12: a disabled plane reads no fields and codes no offsets.
        if plane.ccso_bo_only.is_some()
            || plane.ccso_scale_idx.is_some()
            || plane.ccso_quant_idx.is_some()
            || plane.ccso_ext_filter.is_some()
            || plane.ccso_edge_clf.is_some()
            || plane.ccso_max_band_log2.is_some()
            || !plane.ccso_offset_idx.is_empty()
        {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_plane_fields",
            });
        }
        return Ok(());
    }

    // § 5.18.7.12: an enabled plane has every field present.
    let (
        Some(bo_only),
        Some(scale_idx),
        Some(quant_idx),
        Some(ext_filter),
        Some(edge_clf),
        Some(max_band_log2),
    ) = (
        plane.ccso_bo_only,
        plane.ccso_scale_idx,
        plane.ccso_quant_idx,
        plane.ccso_ext_filter,
        plane.ccso_edge_clf,
        plane.ccso_max_band_log2,
    )
    else {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "ccso_plane_fields",
        });
    };

    // ccso_scale_idx f(2): 0..=3.
    if scale_idx >= CCSO_SCALE_IDX_MAX_PLUS_1 {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "ccso_scale_idx",
        });
    }

    if bo_only {
        // § 5.18.7.12: bo_only infers quant_idx = 0, ext_filter = 0, edge_clf = 0 (no bits).
        if quant_idx != 0 || ext_filter != 0 || edge_clf {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_bo_only_fields",
            });
        }
    } else {
        // ccso_quant_idx f(2): 0..=3.
        if quant_idx >= CCSO_QUANT_IDX_MAX_PLUS_1 {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_quant_idx",
            });
        }
        // ccso_ext_filter f(3): 0..=7.
        if ext_filter >= CCSO_EXT_FILTER_MAX_PLUS_1 {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_ext_filter",
            });
        }
        // quantStep == 0 infers ccso_edge_clf = 0 (no bit); a stored true is unproducible.
        if ccso_quant_step(scale_idx, quant_idx) == 0 && edge_clf {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "ccso_edge_clf",
            });
        }
    }

    // ccso_max_band_log2 f(2 + bo_only): the field width caps the value.
    let band_bits = 2 + u32::from(bo_only);
    if u32::from(max_band_log2) >= (1u32 << band_bits) {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "ccso_max_band_log2",
        });
    }

    // The offset table length must equal maxEdgeInterval^2 * maxBand.
    let max_edge_interval = if bo_only {
        1u32
    } else {
        // edge_clf is 0/1 and CCSO_INPUT_INTERVAL == 3, so this subtraction is >= 2 (no underflow).
        CCSO_INPUT_INTERVAL - u32::from(edge_clf)
    };
    // max_band_log2 <= 7 (validated by the field width above), so 1 << it never overflows u32.
    let max_band = 1u32 << u32::from(max_band_log2);
    let expected = (max_edge_interval * max_edge_interval * max_band) as usize;
    if plane.ccso_offset_idx.len() != expected {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "ccso_offset_idx_len",
        });
    }
    // Each offset is tu(7): 0..=7.
    if plane
        .ccso_offset_idx
        .iter()
        .any(|&v| v > CCSO_OFFSET_IDX_MAX)
    {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "ccso_offset_idx",
        });
    }
    Ok(())
}

// The unit/reject tests and the property tests live in sibling files (each kept under the
// advisory source-line limit); `include!` pastes them into this module so their `super::*`
// resolves to the writers and private helpers above.
#[cfg(test)]
include!("frame_restoration_tests.rs");
#[cfg(test)]
include!("frame_restoration_proptests.rs");
