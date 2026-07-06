// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Sequence-header and multi-frame-header input views for the AV2 § 5.18.2 frame-header
//! core parser.
//!
//! Gathers the state the bitstream does not repeat into the read-only views the parser —
//! and the inverse [`crate::write`] frame-header writer — consume: [`CoreSeqView`] (built
//! via [`CoreSeqView::from_sequence`]) with its § 5.4.6 inter sub-view
//! [`CoreSeqInterView`], and [`MfhFrameView`] (built via [`MfhFrameView::from_record`])
//! resolving a `cur_mfh_id > 0` reference. [`all_frames_mask`] is the shared
//! `(1 << NumRefFrames) - 1` helper.

use crate::headers::frame::filtering::{CoreSeqFilterView, MfhDeblockingView};
use crate::headers::frame::quant::CoreSeqQuantView;
use crate::headers::frame::restoration::{CoreSeqCcsoView, CoreSeqRestorationView};
use crate::headers::frame::segmentation::{CoreSeqSegView, MfhSegView};
use crate::headers::frame::tiling::CoreSeqTileView;
use crate::headers::sequence::{ChromaFormatIdc, SequenceHeader};
use crate::hls::MultiFrameHeaderRecord;

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
    pub(crate) enable_tip_output: bool,
    pub(crate) enable_tip_hole_fill: bool,
    pub(crate) enable_refinemv: bool,
    pub(crate) enable_tip_refinemv: bool,
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
        let filter = seq.filter.as_ref()?;
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
            inter: CoreSeqInterView {
                enable_ref_frame_mvs: inter.enable_ref_frame_mvs,
                explicit_ref_frame_map: inter.explicit_ref_frame_map,
                enable_bru: inter.enable_bru,
                enable_tip: inter.enable_tip,
                enable_tip_output: inter.enable_tip_output,
                enable_tip_hole_fill: inter.enable_tip_hole_fill,
                enable_refinemv: inter.enable_refinemv,
                enable_tip_refinemv: inter.enable_tip_refinemv,
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
            filter: CoreSeqFilterView {
                enable_cdef: filter.enable_cdef,
                enable_gdf: filter.enable_gdf,
                gdf_unit_matches_sb_size: filter.gdf_unit_matches_sb_size,
                disable_loopfilters_across_tiles: filter.disable_loopfilters_across_tiles,
                cdef_on_skip_txfm: filter.cdef_on_skip_txfm,
                df_par_bits_minus_2: filter.df_par_bits_minus_2,
                enable_df_sub_pu: inter.enable_df_sub_pu,
                single_picture_header_flag: general.single_picture_header_flag,
            },
            restoration: CoreSeqRestorationView {
                enable_restoration: filter.enable_restoration,
                lr_pc_wiener_disabled: filter.lr_pc_wiener_disabled,
                lr_wiener_nonsep_disabled: filter.lr_wiener_nonsep_disabled,
                lr_uv_pc_wiener_disabled: filter.lr_uv_pc_wiener_disabled,
                lr_uv_wiener_nonsep_disabled: filter.lr_uv_wiener_nonsep_disabled,
            },
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
        let default_dims = match record.mfh_frame_size {
            Some(size) => (
                size.width_minus_1.saturating_add(1),
                size.height_minus_1.saturating_add(1),
            ),
            None => (seq.max_frame_width, seq.max_frame_height),
        };
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
                _ => None,
            }
        } else {
            None
        };
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
pub(super) fn all_frames_mask(num_ref_frames: u32) -> u32 {
    if num_ref_frames >= u32::BITS {
        u32::MAX
    } else {
        (1u32 << num_ref_frames).wrapping_sub(1)
    }
}
