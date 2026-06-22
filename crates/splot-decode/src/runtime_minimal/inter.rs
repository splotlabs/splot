// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! First inter-frame decode frontier for the shared minimal-tier runtime.
//!
//! Decodes the verified minimal inter subset: a single 64x64 `OBU_REGULAR_TILE_GROUP`
//! frame with one reference, `skip_mode == 0`, `is_inter == 1`, `skip == 1`, the
//! single-reference zero-MV `GLOBALMV` mode, no residual, and § 7.13.3.18
//! zero-fraction motion compensation (a straight copy of the co-located reference
//! block). Everything outside that subset is rejected with a structured
//! `decode/unsupported-feature` diagnostic BEFORE any wrong output, so splot never
//! emits a confident-but-unverified inter frame.
//!
//! Feature tracking: `DECODE-FIRST-INTER-FRAME-FRONTIER`,
//! `DECODE-INTER-MODE-INFO`, `DECODE-INTER-ZERO-MV`,
//! `DECODE-INTER-MOTION-COMPENSATION`.

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::{
    FrameHeaderCore, FrameHeaderParseInput, FrameHeaderParseMode, FrameHeaderParseStatus,
    FrameReferenceStateView, FrameSize, TxMode, parse_frame_header_core,
};
use splot_core::headers::sequence::SequenceHeader;
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;
use splot_recon::{
    DecodedFrame, InterpolationFilter as ReconInterpolationFilter, ReferenceFrameStore,
    ReferenceSlot,
};

use super::{
    DecodeOptions, DecodePlannedObu, DecodeStreamPlan, IvfHeader, MinimalRuntimeFrame, Result,
    ensure_runtime_limits,
};
use crate::error::{DecodeError, DecodeUnsupportedFeature};

/// Feature id for the inter decode frontier diagnostics.
const FEATURE_ID: &str = "DECODE-FIRST-INTER-FRAME-FRONTIER";
const MATRIX_ROW: &str = "first-inter-frame-frontier";
const TIER_ID: &str = "general-inter-8bit420-frontier-v1";
const REMEDIATION: &str = "Inter decode is limited to the verified single-reference zero-MV skip subset; track DECODE-FIRST-INTER-FRAME-FRONTIER.";

const SPEC_HEADER: &str = "5.18.2";
const SPEC_MODE_INFO: &str = "5.20.7.6";
const SPEC_MV: &str = "7.11";
const SPEC_MC: &str = "7.13.3.18";
const SPEC_REFERENCE: &str = "7.23";

/// AV2 § 5.20.7.6 `single_mode == 0` -> `YMode = NEARMV` (the zero-MV mode over an
/// empty no-neighbour § 7.10 MV stack this frontier reconstructs).
const SINGLE_MODE_NEARMV: u8 = 0;
/// AV2 § 5.20.7.6 `single_mode == 1` -> `YMode = GLOBALMV` (the zero-MV mode over
/// identity global motion).
const SINGLE_MODE_GLOBALMV: u8 = 1;
/// AV2 § 5.20.7.6 `single_mode == 2` -> `YMode = NEWMV` (reads a § 5.20.7.20
/// SHELL-coded MV delta over the no-neighbour zero predictor).
const SINGLE_MODE_NEWMV: u8 = 2;

/// A modelled motion vector in eighth-pel units (AV2 § 7.11). The verified subset
/// only produces the zero vector; the newtype keeps the MV result explicit at the
/// motion-compensation boundary rather than a bare integer pair.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Mv {
    /// Row (vertical) component in eighth-pel units.
    pub(super) row: i32,
    /// Column (horizontal) component in eighth-pel units.
    pub(super) col: i32,
}

impl Mv {
    /// The zero motion vector (`GLOBALMV` over identity global motion / the
    /// no-neighbour zero predictor, AV2 § 7.11).
    const ZERO: Self = Self { row: 0, col: 0 };
}

/// Decodes the minimal inter frame, given the post-key reference store, and returns
/// its displayed frame. The reference for slot `ref_frame_idx[0]` must be present.
///
/// This actually parses the § 5.18.2 inter frame header (to
/// [`FrameHeaderParseStatus::InterHeaderComplete`]), walks the § 5.20.3 partition
/// tree, reads the § 5.20.7.6 inter `mode_info` (`is_inter` / `skip` / `single_mode`)
/// from the tile arithmetic stream, derives the § 7.11 zero motion vector, runs the
/// § 7.13.3.18 zero-fraction motion-compensation copy, and validates § 8.2.4
/// `exit_symbol()`. No step is hardcoded: a wrong symbol read fails `exit_symbol()`.
#[allow(clippy::too_many_arguments)]
pub(super) fn decode_minimal_inter_frame(
    plan: &DecodeStreamPlan,
    candidate: &DecodePlannedObu,
    bytes: &[u8],
    frame_envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    options: DecodeOptions,
    header: IvfHeader,
    reference: &InterReferenceState<'_>,
) -> Result<MinimalRuntimeFrame> {
    let offset = frame_envelope.offset;

    if frame_envelope.header.obu_type != ObuType::RegularTileGroup {
        return Err(unsupported_at(
            "inter_unexpected_obu_type",
            offset,
            "minimal inter decode requires an OBU_REGULAR_TILE_GROUP inter frame",
            SPEC_HEADER,
        ));
    }

    // §5.18.2 inter frame-header parse using the post-key reference state.
    let core = parse_inter_frame_core(frame_envelope, sequence, reference)?;
    validate_inter_frame_core(&core, sequence, offset)?;

    let frame_size = core.frame_size.ok_or_else(|| {
        unsupported_at(
            "inter_missing_frame_size",
            offset,
            "minimal inter decode requires a parsed frame size",
            SPEC_HEADER,
        )
    })?;
    let frame_width = frame_size.width;
    let frame_height = frame_size.height;

    // §5.20.7.10 read_ref_frames: comp_mode == SINGLE_REFERENCE for this subset
    // (reference_select == 0), and read_single_ref() reads NO symbol when
    // NumTotalRefs == 1 (its loop bound `0 < NumTotalRefs - 1` is empty), returning
    // RefFrame[0] = 0. The reference slot is ref_frame_idx[RefFrame[0]] =
    // ref_frame_idx[0].
    let inter = core.inter.as_ref().ok_or_else(|| {
        unsupported_at(
            "inter_missing_control_region",
            offset,
            "minimal inter decode requires a parsed inter control region",
            SPEC_HEADER,
        )
    })?;
    let num_total_refs = inter.num_total_refs.unwrap_or(0);
    if num_total_refs != 1 {
        return Err(unsupported_at(
            "inter_unsupported_num_total_refs",
            offset,
            "minimal inter decode only supports a single reference frame (NumTotalRefs == 1); read_single_ref would otherwise read a single_ref symbol",
            SPEC_MODE_INFO,
        ));
    }
    let ref_slot = inter.ref_frame_idx.first().copied().ok_or_else(|| {
        unsupported_at(
            "inter_missing_ref_frame_idx",
            offset,
            "minimal inter decode requires a derived ref_frame_idx[0]",
            SPEC_HEADER,
        )
    })?;

    // §5.18.8.3 frame_reference_mode: reference_select must be 0 so read_ref_frames
    // infers comp_mode == SINGLE_REFERENCE with no comp_mode symbol read.
    let tail = core.inter_tail.as_ref().ok_or_else(|| {
        unsupported_at(
            "inter_missing_tail",
            offset,
            "minimal inter decode requires the parsed §5.18.2 inter tail",
            SPEC_HEADER,
        )
    })?;
    if tail.reference_select {
        return Err(unsupported_at(
            "inter_reference_select",
            offset,
            "minimal inter decode only supports reference_select == 0 (no compound reference selection)",
            SPEC_MODE_INFO,
        ));
    }
    // §5.18.9 global motion: the verified subset is identity global motion, so
    // GLOBALMV yields a zero MV. use_global_motion == 0 means GmType == IDENTITY for
    // every reference (the parser does not model warp global-motion params).
    if tail.use_global_motion {
        return Err(unsupported_at(
            "inter_use_global_motion",
            offset,
            "minimal inter decode only supports identity global motion (use_global_motion == 0), so GLOBALMV is the zero vector",
            SPEC_MV,
        ));
    }

    // Reference plane geometry must match the current frame (no scaling).
    let ref_frame = reference.frame_for_slot(ref_slot).ok_or_else(|| {
        unsupported_at(
            "inter_missing_reference_frame",
            offset,
            "minimal inter decode requires the referenced frame to be present in the reference store",
            SPEC_REFERENCE,
        )
    })?;
    let ref_luma = ref_frame.y();
    if ref_luma.visible_size().width() != frame_width as usize
        || ref_luma.visible_size().height() != frame_height as usize
    {
        return Err(unsupported_at(
            "inter_reference_resolution_mismatch",
            offset,
            "minimal inter decode only supports an unscaled reference (reference size must equal the current frame size)",
            SPEC_MC,
        ));
    }

    // Enforce the configured decode limits before allocating the output frame.
    let limits = options.limits();
    let tile_size = {
        let mut tile_plan = super::derive_inter_tile_plan(
            plan,
            candidate,
            bytes,
            frame_envelope,
            sequence,
            &core,
            options,
        )?;
        let tile = match tile_plan.work_units_mut() {
            [tile] => tile,
            _ => {
                return Err(unsupported_at(
                    "inter_unexpected_tile_work_units",
                    offset,
                    "minimal inter decode requires exactly one tile work unit",
                    SPEC_HEADER,
                ));
            }
        };
        tile.tile_size()
    };
    ensure_runtime_limits(limits, frame_width, frame_height, tile_size)?;

    // §5.20.7.6: the block's interpolation filter. For the SWITCHABLE frame filter
    // the block reads an `interp_filter` symbol; for a fixed frame filter the block
    // inherits it. The block decode resolves this and returns the recon-side filter
    // selector.
    let interpolation_filter = inter.interpolation_filter.ok_or_else(|| {
        unsupported_at(
            "inter_missing_interpolation_filter",
            offset,
            "minimal inter decode requires a parsed §5.18.5.1 interpolation_filter",
            SPEC_MC,
        )
    })?;

    // Decode the §5.20 inter mode info from the tile arithmetic stream and confirm
    // §8.2.4 exit_symbol(); the partition walk + symbol reads + exit check make this
    // a real decode, not a canned copy. Returns the §7.11 motion vector and the
    // §5.20.7.6 block interpolation filter.
    let block = decode_inter_block_and_mv(
        plan,
        candidate,
        bytes,
        frame_envelope,
        sequence,
        &core,
        options,
        interpolation_filter,
    )?;

    // §7.13.3.17 motion-vector scaling + §7.13.3.18 block inter prediction over the
    // unscaled reference. The zero-fraction (zero-MV) case reduces inside the kernel
    // to a straight reference-sample copy.
    let frame = motion_compensate_inter_block(
        ref_frame,
        block.mv,
        block.interp,
        frame_width,
        frame_height,
        offset,
        limits,
    )?;

    Ok(MinimalRuntimeFrame {
        frame,
        frame_rate_numerator: header.timebase_denominator,
        frame_rate_denominator: header.timebase_numerator,
    })
}

/// The decoded single inter block: its §7.11 motion vector and the §5.20.7.6
/// block interpolation filter (mapped to the recon-side §7.13.3.18 selector).
#[derive(Clone, Copy, Debug)]
pub(super) struct InterBlock {
    /// The decoded §7.11 motion vector (eighth-pel units).
    pub(super) mv: Mv,
    /// The §7.13.3.18 interpolation filter the block uses for motion compensation.
    pub(super) interp: ReconInterpolationFilter,
}

/// The post-key reference state the inter decode consumes: the §7.23
/// [`ReferenceFrameStore`] of borrowed decoded reference frames plus the modelled
/// §7.7/§7.23 reference metadata (`RefValid` / `RefOrderHint` / dims) the §5.18.2
/// inter header parse reads through [`FrameReferenceStateView`].
///
/// The store borrows the already-decoded key frame (it does not copy pixels): a
/// `ReferenceFrameStore<&DecodedFrame<u8>>` retains a reference handle per slot the
/// §7.23 `refresh_frame_flags` selected.
pub(super) struct InterReferenceState<'a> {
    /// §7.23 reference store, one borrowed decoded frame per refreshed slot.
    pub(super) store: &'a ReferenceFrameStore<&'a DecodedFrame<u8>>,
    /// `RefValid[i]` per slot.
    pub(super) ref_valid: Vec<bool>,
    /// `RefOrderHint[i]` per slot.
    pub(super) ref_order_hint: Vec<u32>,
    /// `RefFrameWidth[i]` per slot.
    pub(super) ref_frame_width: Vec<u32>,
    /// `RefFrameHeight[i]` per slot.
    pub(super) ref_frame_height: Vec<u32>,
}

impl<'a> InterReferenceState<'a> {
    /// Returns the decoded reference frame stored in `slot`, if any.
    fn frame_for_slot(&self, slot: u32) -> Option<&DecodedFrame<u8>> {
        let slot = ReferenceSlot::new(slot as usize).ok()?;
        self.store.get(slot).ok().flatten().copied()
    }

    /// Builds the §5.18.2 frame-header reference-state view borrowing this state.
    fn header_view(&self) -> FrameReferenceStateView<'_> {
        FrameReferenceStateView::from_slots(
            &self.ref_valid,
            &self.ref_order_hint,
            &self.ref_frame_width,
            &self.ref_frame_height,
        )
    }
}

/// Parses the §5.18.2 inter frame header to [`FrameHeaderParseStatus::InterHeaderComplete`]
/// using the post-key reference state.
fn parse_inter_frame_core(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    reference: &InterReferenceState<'_>,
) -> Result<FrameHeaderCore> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    // §5.19 tile_group_obu: the inter OBU_REGULAR_TILE_GROUP carries the frame
    // header in its first tile group (tile_start_and_end_present == 0 -> is_first ==
    // 1). Read the leading bit and require the first-tile-group form.
    let is_first_tile_group = reader.read_bit().map_err(|_| {
        unsupported_at(
            "inter_tile_group_prefix_parse",
            envelope.offset,
            "minimal inter decode requires a parseable first tile-group prefix",
            SPEC_HEADER,
        )
    })? != 0;
    if !is_first_tile_group {
        return Err(unsupported_at(
            "inter_non_first_tile_group",
            envelope.offset,
            "minimal inter decode requires the frame header in the first tile group",
            SPEC_HEADER,
        ));
    }
    let input = FrameHeaderParseInput {
        obu_type: envelope.header.obu_type,
        first_picture_in_tu: false,
        active_sequence: Some(sequence),
        mfh_record: None,
        reference_state: reference.header_view(),
        mode: FrameHeaderParseMode::Core,
    };
    parse_frame_header_core(&mut reader, &input).map_err(|_| {
        unsupported_at(
            "inter_frame_header_parse",
            envelope.offset,
            "minimal inter decode requires a fully parseable §5.18.2 inter frame header",
            SPEC_HEADER,
        )
    })
}

/// Gates the parsed inter frame header to the verified minimal subset (no tools, no
/// filters, no grain, single 64x64 tile, an immediate-output inter frame).
fn validate_inter_frame_core(
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    offset: ByteOffset,
) -> Result<()> {
    if core.status != FrameHeaderParseStatus::InterHeaderComplete {
        return Err(unsupported_at(
            "inter_incomplete_frame_header",
            offset,
            "minimal inter decode requires a complete §5.18.2 inter frame header (InterHeaderComplete)",
            SPEC_HEADER,
        ));
    }
    if core.frame_is_intra != Some(false) || core.is_key_frame {
        return Err(unsupported_at(
            "inter_not_inter_frame",
            offset,
            "minimal inter decode requires a non-intra, non-key inter frame",
            SPEC_HEADER,
        ));
    }
    if core.show_existing_frame != Some(false) || core.immediate_output_frame != Some(true) {
        return Err(unsupported_at(
            "inter_unsupported_output_control",
            offset,
            "minimal inter decode requires one immediate-output inter frame (no show-existing-frame indirection)",
            SPEC_HEADER,
        ));
    }
    match core.frame_size {
        Some(FrameSize {
            width: super::MINIMAL_WIDTH,
            height: super::MINIMAL_HEIGHT,
            ..
        }) => {}
        _ => {
            return Err(unsupported_at(
                "inter_unsupported_frame_size",
                offset,
                "minimal inter decode currently accepts only the verified 64x64 frame size",
                SPEC_HEADER,
            ));
        }
    }
    let Some(tile_info) = core.tile_info.as_ref() else {
        return Err(unsupported_at(
            "inter_missing_tile_info",
            offset,
            "minimal inter decode requires a parsed one-tile frame layout",
            SPEC_HEADER,
        ));
    };
    if tile_info.tile_cols != 1 || tile_info.tile_rows != 1 {
        return Err(unsupported_at(
            "inter_multi_tile_frame",
            offset,
            "minimal inter decode supports one tile",
            SPEC_HEADER,
        ));
    }
    // No-tool, no-filter, no-grain inter frame: the verified subset has every
    // §5.18 tool disabled, matching the flat synthetic fixture.
    if core
        .quantization_params
        .is_none_or(|quant| quant.base_q_idx == 0)
        || core
            .segmentation_params
            .as_ref()
            .is_none_or(|seg| seg.segmentation_enabled)
        || core.setup_qm_params.is_none_or(|qm| qm.using_qmatrix)
        || core
            .delta_q_params
            .is_none_or(|delta| delta.delta_q_present)
        || core
            .lossless_info
            .as_ref()
            .is_none_or(|lossless| lossless.coded_lossless)
        || core
            .deblocking_filter_params
            .is_none_or(|filter| filter.apply_deblocking_filter != [false; 4])
        || core.gdf_params.is_none_or(|gdf| gdf.gdf_frame_enable)
        || core
            .cdef_params
            .as_ref()
            .is_none_or(|cdef| cdef.cdef_frame_enable)
        || core.lr_params.as_ref().is_none_or(|lr| lr.uses_lr)
        || core
            .ccso_params
            .as_ref()
            .is_none_or(|ccso| ccso.ccso_frame_flag.is_some() || !ccso.planes.is_empty())
        || core.inter_tail.as_ref().is_none_or(|tail| {
            // §5.20.7.1 calls `read_skip_mode()` (which reads a `skip_mode` symbol when
            // `skip_mode_present` and compound refs are allowed) BEFORE `read_is_inter()`.
            // The verified subset's block decode reads `is_inter` first, so a frame with
            // `skip_mode_present` set would desync — reject it here.
            tail.apply_grain || tail.tx_mode != TxMode::Largest || tail.skip_mode_present
        })
        // §5.20.7.6: any frame-enabled motion mode makes the block read a motion-mode
        // (`inter_intra` / warp / local-warp) symbol the verified SIMPLE-mode block does
        // not read. A SWITCHABLE interpolation filter IS admitted (the block decode reads
        // the per-block `interp_filter` symbol); a fixed frame filter supplies it with no
        // symbol. Reject so an admitted inter frame can never silently desync the §8.2
        // arithmetic decoder past the `exit_symbol()` bit-count backstop.
        || core
            .inter
            .as_ref()
            .is_none_or(|inter| {
                inter
                    .frame_enabled_motion_modes
                    .is_some_and(|modes| modes.iter().any(|&enabled| enabled))
            })
        // §5.20.7.6 MvPrecision derivation: `enable_flex_mvres && UsePerBlockMvPrecision
        // && has_newmv` reads `use_most_probable_precision` (and maybe `pb_mv_precision`)
        // before assign_mv; `enable_adaptive_mvd` allows `use_amvd` after single_mode. The
        // verified NEWMV block reads neither, so reject a frame whose sequence enables
        // flexible MV resolution or adaptive MVD. `enable_bawp` (-> `allow_bawp` per §5.18.2)
        // makes §5.20.7.6 read a `use_bawp` symbol after single_mode for an unscaled
        // single-ref block with Min(w,h) >= 8 and YMode != GLOBALMV (i.e. exactly the
        // verified NEARMV/NEWMV 64x64 block) that this decoder does not read — reject it so
        // an admitted frame can never desync the §8.2 decoder past the bit-count-only
        // `exit_symbol()` backstop.
        || sequence.inter.as_ref().is_none_or(|seq_inter| {
            seq_inter.enable_flex_mvres
                || seq_inter.enable_adaptive_mvd
                || seq_inter.enable_bawp
        })
    {
        return Err(unsupported_at(
            "inter_unsupported_frame_tools",
            offset,
            "minimal inter decode requires the verified no-tool, no-filter, no-grain, TX_MODE_LARGEST inter frame header",
            SPEC_HEADER,
        ));
    }
    Ok(())
}

mod block;
mod mc;
mod mv_scaling;
mod read_mv;

use block::decode_inter_block_and_mv;
use mc::motion_compensate_inter_block;

#[cfg(test)]
mod tests;

fn unsupported_at(
    reason: &'static str,
    byte_offset: ByteOffset,
    message: &'static str,
    spec_section: &'static str,
) -> DecodeError {
    DecodeError::UnsupportedFeature {
        unsupported: Box::new(DecodeUnsupportedFeature::new(
            reason,
            TIER_ID,
            MATRIX_ROW,
            FEATURE_ID,
            spec_section,
            message,
            REMEDIATION,
            Some(byte_offset),
        )),
    }
}
