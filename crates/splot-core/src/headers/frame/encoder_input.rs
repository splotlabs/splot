// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Encoder writer-input constructors for the § 5.4.1 sequence-derived views.
//!
//! The § 5.18.2 / § 5.20.1 frame-header + tile-group writers
//! ([`crate::write::write_frame_header_core`], [`crate::write::write_tile_group_obu`])
//! take parsed [`CoreSeqView`] / [`CoreSeqInterView`] models. These public constructors
//! let `splot-encode` build a minimal-intra view without a parsed [`SequenceHeader`]
//! (the maintainer-approved writer bridge). They live in their own module so the large
//! frame-header parser ([`super::info`]) does not grow with encoder surface; the views
//! are `#[non_exhaustive]` with crate-private fields, which a within-crate constructor
//! may still build.
//!
//! [`SequenceHeader`]: crate::headers::sequence::SequenceHeader

use crate::headers::frame::size::ceil_log2;
use crate::headers::frame::{
    CoreSeqCcsoView, CoreSeqFilterView, CoreSeqInterView, CoreSeqQuantView, CoreSeqRestorationView,
    CoreSeqSegView, CoreSeqTileView, CoreSeqView,
};
use crate::headers::sequence::ChromaFormatIdc;

impl CoreSeqInterView {
    /// Builds the all-disabled § 5.4.6 inter-config view a minimal intra sequence
    /// header signals (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-6`): every
    /// inter tool off and every motion mode disabled. The § 5.18.2 intra control region
    /// never reads these — an intra frame skips the inter tail — so this is the inert
    /// inter state a frame-header writer needs to invert `parse_frame_header_core` for a
    /// minimal intra frame.
    ///
    /// This is the public encoder writer-input constructor for the otherwise
    /// `#[non_exhaustive]`, crate-private-field [`CoreSeqInterView`]; it lets
    /// `splot-encode` build a [`CoreSeqView`] without a parsed [`SequenceHeader`].
    ///
    /// [`SequenceHeader`]: crate::headers::sequence::SequenceHeader
    #[must_use]
    pub fn new_minimal_intra() -> Self {
        Self {
            enable_ref_frame_mvs: false,
            explicit_ref_frame_map: false,
            enable_bru: false,
            enable_tip: false,
            seq_max_drl_bits_minus_1: 0,
            allow_frame_max_drl_bits: false,
            enable_flex_mvres: false,
            seq_frame_motion_modes_present_flag: false,
            // MOTION_MODES == 5 (§3); hardcoded here since the const is private to `info`.
            seq_enabled_motion_modes: [false; 5],
            enable_opfl_refine: 0,
        }
    }
}

impl CoreSeqView {
    /// Builds the AV2 § 5.4.1 (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-1`)
    /// sequence-derived view a minimal intra frame needs, the public encoder
    /// writer-input constructor for the otherwise `#[non_exhaustive]`,
    /// crate-private-field [`CoreSeqView`]. It lets `splot-encode` drive the
    /// `write_tile_group_obu` / `write_frame_header_core` writers without a parsed
    /// [`SequenceHeader`] (the alternative [`CoreSeqView::from_sequence`] input).
    ///
    /// Every sequence tool an intra frame does not use is disabled: no reference-frame
    /// state (§ 5.4.6 inter view all-off via [`CoreSeqInterView::new_minimal_intra`]),
    /// no segmentation/tiles/loop-filters/restoration/CCSO, no film grain. 8-bit YUV420,
    /// 3 planes. The configurable inputs are the § 5.4.1 frame-size maxima
    /// (`max_frame_width` / `max_frame_height`); `frame_width_bits` / `frame_height_bits`
    /// are derived from them via `ceil_log2`, so any in-range maxima can write an
    /// overridden frame size, not just those that fit 12 bits.
    ///
    /// This is the **non-single-picture** view (`single_picture_header_flag == false`):
    /// it is the exact shape the test `base_seq` helper builds, so the existing
    /// frame-header round-trip suite regresses it (`base_seq()` delegates here). The
    /// single-picture variant infers a different sequence shape (§ 5.4.6 `OrderHintBits
    /// = 0` / `NumRefFrames = 2`, § 5.4.1 SCC `SELECT` force fields, § 5.4.10
    /// `(enable_avg_cdf, avg_cdf_type) = (true, 1)`) across four § 5.4.1 config parsers
    /// and is a later, separately round-trip-verified constructor.
    ///
    /// Returns `None` if either maximum is outside `1..=65536`: § 5.4.1
    /// `frame_*_bits_minus_1` is `f(4)`, so `frame_*_bits` is `1..=16` and a real
    /// sequence header can only describe maxima up to `2^16` — a wider/zero maximum has
    /// no valid sequence header to invert.
    ///
    /// [`SequenceHeader`]: crate::headers::sequence::SequenceHeader
    #[must_use]
    pub fn new_minimal_intra(max_frame_width: u32, max_frame_height: u32) -> Option<Self> {
        use crate::headers::sequence::{CdefOnSkipTxfm, LevelIdx, SuperblockSize, Tier};
        // §5.4.1 dim bit-widths derived from the maxima so any in-range maxima can write
        // an overridden frame size (ceil_log2(4096) == 12 keeps base_seq); clamped to the
        // 1-bit spec minimum and gated to the writable 1..=2^16 maxima domain.
        let dim_bits = |max: u32| -> Option<u32> {
            (1..=(1u32 << 16))
                .contains(&max)
                .then(|| ceil_log2(max).max(1))
        };
        let frame_width_bits = dim_bits(max_frame_width)?;
        let frame_height_bits = dim_bits(max_frame_height)?;
        Some(Self {
            num_ref_frames: 8,
            order_hint_bits: 4,
            long_term_frame_id_bits: 0,
            enable_short_refresh_frame_flags: false,
            monotonic_output_order_flag: false,
            single_picture_header_flag: false,
            max_mlayer_id: 0,
            frame_width_bits,
            frame_height_bits,
            max_frame_width,
            max_frame_height,
            seq_force_screen_content_tools: 0,
            seq_force_integer_mv: 0,
            allow_frame_max_bvp_drl_bits: false,
            inter: CoreSeqInterView::new_minimal_intra(),
            quant: CoreSeqQuantView {
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
            seg: CoreSeqSegView {
                seq_seg_info_present_flag: false,
                seq_allow_seg_info_change: false,
                enable_ext_seg: false,
                max_segments: 8,
                seq_segment_info: None,
            },
            tile: CoreSeqTileView {
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
                // §A: the no-level / `Configurable` sentinel, so the §5.18.7.2 tile-info
                // writer's level-derived tile-width/area bounds do not constrain a
                // larger writer-input view (which the maxima would otherwise exceed).
                seq_level_idx: LevelIdx::from_bits(31),
            },
            filter: CoreSeqFilterView {
                enable_cdef: false,
                enable_gdf: false,
                gdf_unit_matches_sb_size: false,
                disable_loopfilters_across_tiles: false,
                cdef_on_skip_txfm: CdefOnSkipTxfm::Adaptive,
                df_par_bits_minus_2: 0,
                single_picture_header_flag: false,
            },
            restoration: CoreSeqRestorationView {
                enable_restoration: false,
                lr_pc_wiener_disabled: false,
                lr_wiener_nonsep_disabled: false,
                lr_uv_pc_wiener_disabled: false,
                lr_uv_wiener_nonsep_disabled: false,
            },
            ccso: CoreSeqCcsoView {
                enable_ccso: false,
                single_picture_header_flag: false,
            },
            chroma_format_idc: ChromaFormatIdc::Yuv420,
            film_grain_params_present: Some(false),
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn core_seq_inter_view_minimal_intra_is_all_disabled() {
        // The public encoder writer-input constructor yields the inert §5.4.6 inter view:
        // every tool off, every motion mode disabled. (CoreSeqInterView has no PartialEq,
        // so assert the fields directly; the frame-header writer round-trips additionally
        // exercise it through the minimal-intra seq view's inlined inter field.)
        let v = CoreSeqInterView::new_minimal_intra();
        assert!(!v.enable_ref_frame_mvs);
        assert!(!v.explicit_ref_frame_map);
        assert!(!v.enable_bru);
        assert!(!v.enable_tip);
        assert_eq!(v.seq_max_drl_bits_minus_1, 0);
        assert!(!v.allow_frame_max_drl_bits);
        assert!(!v.enable_flex_mvres);
        assert!(!v.seq_frame_motion_modes_present_flag);
        assert_eq!(v.seq_enabled_motion_modes, [false; 5]);
        assert_eq!(v.enable_opfl_refine, 0);
    }

    #[test]
    fn core_seq_view_minimal_intra_derives_dim_bits_and_is_non_single_picture() {
        // frame_width_bits / frame_height_bits are derived from the maxima so any
        // in-range maxima can write an overridden frame size; ceil_log2(4096) == 12
        // keeps the base_seq shape, ceil_log2(64) == 6 for the encoder's 64x64 tier.
        let base = CoreSeqView::new_minimal_intra(4096, 2304).unwrap();
        assert_eq!((base.frame_width_bits, base.frame_height_bits), (12, 12));
        assert_eq!((base.max_frame_width, base.max_frame_height), (4096, 2304));

        let small = CoreSeqView::new_minimal_intra(64, 64).unwrap();
        assert_eq!((small.frame_width_bits, small.frame_height_bits), (6, 6));

        // A 1-pixel maximum clamps to the 1-bit spec minimum; the largest f(4)-describable
        // maximum (2^16) uses 16 bits; a zero or wider-than-2^16 maximum (frame_*_bits would
        // exceed the f(4) range) has no valid §5.4.1 sequence header and is rejected.
        let bits = |max| CoreSeqView::new_minimal_intra(max, max).map(|v| v.frame_width_bits);
        assert_eq!(bits(1), Some(1));
        assert_eq!(bits(1 << 16), Some(16));
        assert_eq!(bits(0), None);
        assert_eq!(bits((1 << 16) + 1), None);

        // The constructor builds the non-single-picture shape; the single-picture
        // variant (different §5.4.1 inferences) is a separate later constructor.
        assert!(!base.single_picture_header_flag);
        assert!(!base.filter.single_picture_header_flag);
        assert!(!base.ccso.single_picture_header_flag);
    }
}
