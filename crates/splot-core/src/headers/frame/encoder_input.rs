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

use crate::bitio::BitReader;
use crate::headers::frame::size::ceil_log2;
use crate::headers::frame::{
    CoreSeqCcsoView, CoreSeqFilterView, CoreSeqInterView, CoreSeqQuantView, CoreSeqRestorationView,
    CoreSeqSegView, CoreSeqTileView, CoreSeqView, FrameHeaderCore, FrameReferenceStateView,
    init_core_from_prefix, parse_core_body, parse_frame_header_prefix,
};
use crate::headers::sequence::ChromaFormatIdc;
use crate::headers::tile_group::{TileGroupFraming, TileGroupStructure};
use crate::obu::ObuHeader;
use crate::span::ByteOffset;
use crate::types::{EmbeddedLayerId, ExtendedLayerId, ObuType, TemporalLayerId};
use crate::write::{BitWriter, WriteError, WriteResult, write_annexb_obu, write_tile_group_obu};

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

    /// Builds the **single-picture** (`single_picture_header_flag == true`) variant of
    /// [`CoreSeqView::new_minimal_intra`]: the § 5.4.1
    /// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-1`) sequence-derived view a
    /// single-picture minimal-intra frame needs, the input to
    /// [`build_minimal_intra_clk_core`] / the frozen-tier intra frame-header writer.
    ///
    /// It is the non-single view ([`CoreSeqView::new_minimal_intra`]) with exactly the eight
    /// § 5.4.x inferences a single-picture sequence header forces, which a non-single header
    /// signals differently (every other tool stays disabled, a legal single-picture choice):
    ///
    /// - `single_picture_header_flag` (top-level + § 5.18.10 filter + CCSO mirrors): `true`.
    /// - § 5.4.6 `OrderHintBits = 0` (`#s-5-4-6` line 832): the frame `order_hint` is
    ///   `f(OrderHintBits)` = `f(0)`, i.e. **omitted** from the frame header.
    /// - § 5.4.6 `NumRefFrames = 2` (`#s-5-4-6` line 870).
    /// - § 5.4.7 `seq_force_screen_content_tools = SELECT_SCREEN_CONTENT_TOOLS = 2`
    ///   and `seq_force_integer_mv = SELECT_INTEGER_MV = 2` (`#s-5-4-7` lines 1074/1076):
    ///   the frame reads an explicit `allow_screen_content_tools` bit.
    /// - § 5.4.8 `(enable_avg_cdf, avg_cdf_type) = (true, 1)` (`#s-5-4-8`).
    /// - § 5.4.1 `monotonic_output_order_flag = true` (the single-picture general branch).
    ///
    /// Returns `None` on the same out-of-range maxima as [`CoreSeqView::new_minimal_intra`].
    #[must_use]
    pub fn new_minimal_intra_single_picture(
        max_frame_width: u32,
        max_frame_height: u32,
    ) -> Option<Self> {
        let mut view = Self::new_minimal_intra(max_frame_width, max_frame_height)?;
        // §5.4.1: the single_picture_header_flag mirror (top-level + filter + CCSO).
        view.single_picture_header_flag = true;
        view.filter.single_picture_header_flag = true;
        view.ccso.single_picture_header_flag = true;
        // §5.4.6: OrderHintBits = 0 (frame order_hint becomes f(0) -> omitted); NumRefFrames = 2.
        view.order_hint_bits = 0;
        view.num_ref_frames = 2;
        // §5.4.7: SELECT_SCREEN_CONTENT_TOOLS / SELECT_INTEGER_MV (both the sentinel value 2).
        view.seq_force_screen_content_tools = 2;
        view.seq_force_integer_mv = 2;
        // §5.4.8: single-picture sequence_tq_entropy_config infers (enable_avg_cdf, avg_cdf_type).
        view.tile.enable_avg_cdf = true;
        view.tile.avg_cdf_type = 1;
        // §5.4.1: the single-picture general branch infers monotonic_output_order_flag = true.
        view.monotonic_output_order_flag = true;
        Some(view)
    }
}

/// Error assembling the canonical minimal-intra CLK [`FrameHeaderCore`]
/// ([`build_minimal_intra_clk_core`]). Every arm is unreachable for the fixed canonical
/// 64x64 frozen tier; they exist only to honor the no-panic library policy (no
/// `unwrap`/`expect` on the internal sequence-view / `BitWriter` / parser results).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MinimalIntraCoreError {
    /// The internal canonical 64x64 single-picture sequence view could not be built
    /// (unreachable: 64 is inside the valid `1..=2^16` maxima domain).
    #[error("canonical minimal-intra sequence view could not be built")]
    Seq,
    /// The fixed canonical body failed to serialize through [`BitWriter`].
    #[error("canonical minimal-intra body serialization failed: {0}")]
    Body(#[from] WriteError),
    /// The parser rejected the canonical body.
    #[error("canonical minimal-intra body did not parse: {0}")]
    Parse(#[from] crate::error::Error),
}

/// Serializes the canonical 64x64, `base_q_idx == 255` single-picture
/// `OBU_CLOSED_LOOP_KEY` intra `frame_header()` body (§ 5.18.2
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`), MSB-first, against the
/// [`CoreSeqView::new_minimal_intra_single_picture`] sequence shape. Every element is a
/// fixed minimal-intra choice, so the writes provably never overflow; the round-trip test
/// proves the byte sequence parses back to an `IntraHeaderComplete` core.
fn minimal_intra_clk_body_bytes() -> WriteResult<Vec<u8>> {
    let mut writer = BitWriter::new();
    writer.write_uvlc(0)?; // cur_mfh_id == 0 (direct reference, no MFH)
    writer.write_uvlc(0)?; // seq_header_id_in_frame_header == 0
    // single-picture: no frame_type / show_existing / output-flag / frame_size_override bits.
    // order_hint: f(OrderHintBits == 0) -> omitted (no bit).
    // refresh_frame_flags: CLK + max_mlayer_id == 0 -> all-frames inference (no bit, derives 3).
    // frame_size: non-override -> the seq 64x64 maxima (no bits).
    writer.write_bit(0)?; // allow_screen_content_tools (SCC == SELECT forces this; 0 => no force_integer_mv)
    writer.write_bit(0)?; // allow_intrabc
    writer.write_bit(0)?; // disable_cdf_update
    writer.write_bit(1)?; // uniform_tile_spacing_flag (64x64 -> 1 superblock -> no increment bits)
    writer.write_bits(255, 8)?; // base_q_idx (!= 0 avoids CodedLossless)
    writer.write_bit(0)?; // segmentation_enabled
    writer.write_bit(0)?; // using_qmatrix
    writer.write_bit(0)?; // delta_q_present
    writer.write_bit(0)?; // apply_deblocking_filter[0]
    writer.write_bit(0)?; // apply_deblocking_filter[1]
    writer.write_bit(0)?; // tx_mode_select
    writer.write_bits(0, 2)?; // reduced_tx_set
    Ok(writer.into_bytes())
}

/// The frozen minimal-intra tier's frame dimension: a single 64x64 superblock. The
/// canonical body's bit layout is matched to it (single-superblock tile info, omitted
/// override frame size), so the assembler builds the sequence view at this dimension
/// itself rather than accepting one.
const FROZEN_TIER_DIM: u32 = 64;

/// Assembles the canonical minimal-intra `OBU_CLOSED_LOOP_KEY` frame header for the frozen
/// 64x64, `base_q_idx == 255` single-picture tier and returns it paired with the matching
/// sequence view, by **parsing** the canonical § 5.18.2 body (`minimal_intra_clk_body_bytes`)
/// against an internally built [`CoreSeqView::new_minimal_intra_single_picture`] — the
/// parse-backed assembler is conformant by construction (it inverts the same parser the
/// decoder runs). The returned [`FrameHeaderCore`] has status
/// [`FrameHeaderParseStatus::IntraHeaderComplete`].
///
/// The sequence view is built here, not taken as a parameter: the canonical body's bit
/// layout depends on the exact § 5.4.x single-picture inferences (notably `OrderHintBits ==
/// 0`, the SCC `SELECT` force fields, and the 64x64 single-superblock tiling), so any other
/// view would mis-parse these fixed bits into a different (but still complete) core. The
/// body also references **sequence header 0** (`seq_header_id_in_frame_header == 0`), the
/// frozen tier's only sequence header.
///
/// This is the public encoder writer-input assembler: with the returned `(core, seq)` pair
/// and [`crate::write::write_frame_header_core`] it lets `splot-encode` emit the frozen-tier
/// frame header without a parsed [`SequenceHeader`].
///
/// [`SequenceHeader`]: crate::headers::sequence::SequenceHeader
/// [`FrameHeaderParseStatus::IntraHeaderComplete`]: super::FrameHeaderParseStatus::IntraHeaderComplete
pub fn build_minimal_intra_clk_core()
-> Result<(FrameHeaderCore, CoreSeqView), MinimalIntraCoreError> {
    use crate::headers::sequence::SuperblockSize;
    let mut seq = CoreSeqView::new_minimal_intra_single_picture(FROZEN_TIER_DIM, FROZEN_TIER_DIM)
        .ok_or(MinimalIntraCoreError::Seq)?;
    // Frozen 64x64 tier: one 64x64 superblock — the root partition the decode minimal runtime
    // frontier expects. This is a frozen-tier choice, not a § 5.4 single-picture inference, so
    // it is set here, not in `new_minimal_intra_single_picture` (whose default `Block128x128`
    // is shared with the `base_seq` round-trip suite). A 64x64 frame is one superblock at
    // either SB size, so the canonical body bit-sequence is unchanged (the
    // `uniform_tile_spacing_flag` reads zero increment bits either way, and `enable_gdf ==
    // false` means the `seq_sb_size`-dependent GDF read never fires).
    seq.tile.seq_sb_size = SuperblockSize::Block64x64;
    seq.tile.use_128x128_superblock = false;
    let data = minimal_intra_clk_body_bytes()?;
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let prefix = parse_frame_header_prefix(&mut reader, ObuType::ClosedLoopKey, Some(true))?;
    let mut core = init_core_from_prefix(&prefix, ObuType::ClosedLoopKey, true);
    parse_core_body(
        &mut reader,
        &mut core,
        &seq,
        None,
        &FrameReferenceStateView::unknown(),
    )?;
    core.consumed_bits = reader.consumed_bits();
    Ok((core, seq))
}

/// Error assembling the canonical minimal-intra `OBU_CLOSED_LOOP_KEY` tile-group payload
/// ([`encode_minimal_intra_clk_tile_group_obu`]): the frame-header core could not be built,
/// or the § 5.19 tile-group payload could not be serialized.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MinimalIntraTileGroupError {
    /// The frame-header core could not be assembled.
    #[error("frame-header core assembly failed: {0}")]
    Core(#[from] MinimalIntraCoreError),
    /// The § 5.19 tile-group OBU payload could not be serialized (e.g. empty `tile_data`,
    /// which is a § 8.2.2 zero-size-tile defect).
    #[error("tile-group OBU payload serialization failed: {0}")]
    Write(#[from] WriteError),
}

/// Assembles the canonical minimal-intra `OBU_CLOSED_LOOP_KEY` § 5.19 `tile_group_obu()`
/// payload (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-19`) for the frozen 64x64
/// single-picture tier: it builds the matched `(FrameHeaderCore, CoreSeqView)` via
/// [`build_minimal_intra_clk_core`], frames `tile_data` as the single (last) tile of the
/// first tile group ([`TileGroupStructure::single_tile_first_group`] /
/// [`TileGroupFraming::single_tile`]), and drives the § 5.19 / § 5.20.1 writer
/// [`crate::write::write_tile_group_obu`].
///
/// `tile_data` is the § 8.2 entropy-coded bytes of the one 64x64 tile (`>= 1` byte; an empty
/// slice is a § 8.2.2 zero-size-tile defect the writer rejects). The returned bytes are the
/// `tile_group_obu()` payload — the embedded frame header, the § 5.20.1 tile framing, and the
/// tile data — **not** the § 5.2.2 OBU header / size wrapper (a later bridge step). The lone
/// last tile reads no size field, so `tile_data` is the byte-aligned trailing region of the
/// payload.
///
/// This is the public encoder writer-input bridge end-point: it connects the header
/// assembler to the tile-group writer, so `splot-encode` can emit a first tile-group payload
/// from coded tile bytes without a parsed [`SequenceHeader`].
///
/// [`SequenceHeader`]: crate::headers::sequence::SequenceHeader
pub fn encode_minimal_intra_clk_tile_group_obu(
    tile_data: &[u8],
) -> Result<Vec<u8>, MinimalIntraTileGroupError> {
    let (core, seq) = build_minimal_intra_clk_core()?;
    let structure = TileGroupStructure::single_tile_first_group();
    let framing = TileGroupFraming::single_tile(tile_data.len() as u64);
    let mut writer = BitWriter::new();
    write_tile_group_obu(
        &mut writer,
        &core,
        &seq,
        None,
        true,
        &structure,
        &framing,
        &[tile_data],
        true,
    )?;
    Ok(writer.into_bytes())
}

/// Wraps the canonical minimal-intra `OBU_CLOSED_LOOP_KEY` `tile_group_obu()` payload
/// ([`encode_minimal_intra_clk_tile_group_obu`]) in AV2 Annex B framing (§ B.2,
/// `docs/spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-2`): a
/// `leb128` total-size prefix, the § 5.2.2 OBU header,
/// then the payload. The result is a **self-delimiting** Annex B OBU that reparses to exactly
/// one `OBU_CLOSED_LOOP_KEY` carrying the payload — one step beyond the bare `tile_group_obu()`
/// payload (which has no length framing).
///
/// The § 5.2.2 header is the no-extension CLK header: `obu_mlayer_id` and `obu_xlayer_id` are
/// inferred `0` (CLK does not require the global xlayer — only `OBU_MSDO` /
/// `OBU_TEMPORAL_DELIMITER` do, § 5.2.2). `tile_data` is the § 8.2 coded tile bytes (`>= 1`);
/// an empty slice is rejected by the inner assembler.
///
/// This is the public encoder writer-input bridge end-point that emits a complete,
/// self-delimiting OBU. A temporal-delimiter + sequence-header OBU and a full Annex B / IVF
/// stream around it are later bricks; this is a single frame OBU, not yet a decodable
/// temporal unit.
pub fn encode_minimal_intra_clk_annexb_obu(
    tile_data: &[u8],
) -> Result<Vec<u8>, MinimalIntraTileGroupError> {
    let payload = encode_minimal_intra_clk_tile_group_obu(tile_data)?;
    // § 5.2.2: the no-extension OBU_CLOSED_LOOP_KEY header (inferred layer ids 0).
    let header = ObuHeader {
        has_header_extension: false,
        obu_type: ObuType::ClosedLoopKey,
        temporal_layer_id: TemporalLayerId::from_bits(0),
        embedded_layer_id: EmbeddedLayerId::from_bits(0),
        extended_layer_id: ExtendedLayerId::from_bits(0),
        header_size_bytes: 1,
    };
    let mut writer = BitWriter::new();
    write_annexb_obu(&mut writer, &header, &payload)?;
    Ok(writer.into_bytes())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::annexb::parse_annex_b_obus_partial;
    use crate::headers::frame::{FrameHeaderParseStatus, FrameSize, FrameType};
    use crate::headers::tile_group::parse_tile_group_prefix;
    use crate::write::write_frame_header_core;

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
        // variant (different §5.4.1 inferences) is a separate constructor (below).
        assert!(!base.single_picture_header_flag);
        assert!(!base.filter.single_picture_header_flag);
        assert!(!base.ccso.single_picture_header_flag);
    }

    #[test]
    fn new_minimal_intra_single_picture_applies_eight_spec_inferences() {
        let base = CoreSeqView::new_minimal_intra(64, 64).unwrap();
        let sp = CoreSeqView::new_minimal_intra_single_picture(64, 64).unwrap();

        // The eight §5.4.x single-picture inferences that differ from the non-single view.
        assert!(sp.single_picture_header_flag && !base.single_picture_header_flag);
        assert!(sp.filter.single_picture_header_flag && !base.filter.single_picture_header_flag);
        assert!(sp.ccso.single_picture_header_flag && !base.ccso.single_picture_header_flag);
        assert_eq!((base.order_hint_bits, sp.order_hint_bits), (4, 0)); // §5.4.6 OrderHintBits = 0
        assert_eq!((base.num_ref_frames, sp.num_ref_frames), (8, 2)); // §5.4.6 NumRefFrames = 2
        assert_eq!(
            (
                base.seq_force_screen_content_tools,
                sp.seq_force_screen_content_tools
            ),
            (0, 2) // §5.4.7 SELECT_SCREEN_CONTENT_TOOLS
        );
        assert_eq!((base.seq_force_integer_mv, sp.seq_force_integer_mv), (0, 2)); // §5.4.7 SELECT_INTEGER_MV
        assert_eq!(
            (base.tile.enable_avg_cdf, sp.tile.enable_avg_cdf),
            (false, true)
        );
        assert_eq!((base.tile.avg_cdf_type, sp.tile.avg_cdf_type), (0, 1)); // §5.4.8
        assert!(sp.monotonic_output_order_flag && !base.monotonic_output_order_flag);

        // Out-of-range maxima reject like the non-single ctor; the rest is inherited unchanged.
        assert!(CoreSeqView::new_minimal_intra_single_picture(0, 0).is_none());
        assert!(CoreSeqView::new_minimal_intra_single_picture((1 << 16) + 1, 64).is_none());
        assert_eq!((sp.max_frame_width, sp.max_frame_height), (64, 64));
        assert_eq!((sp.frame_width_bits, sp.frame_height_bits), (6, 6));
    }

    #[test]
    fn build_minimal_intra_clk_core_round_trips() {
        // Self-contained: it builds the matching 64x64 single-picture view itself and
        // returns the (core, seq) pair, so a caller cannot mis-pair the body and view.
        let (core, seq) = build_minimal_intra_clk_core().unwrap();
        assert_eq!((seq.max_frame_width, seq.max_frame_height), (64, 64));
        assert!(seq.single_picture_header_flag);
        // Frozen 64x64 tier: a single 64x64 superblock (the decode minimal runtime frontier's
        // root partition), not the new_minimal_intra default Block128x128.
        assert_eq!(
            seq.tile.seq_sb_size,
            crate::headers::sequence::SuperblockSize::Block64x64
        );
        assert!(!seq.tile.use_128x128_superblock);

        // Parses to a complete intra header with the derived single-picture CLK facts.
        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        assert_eq!(core.frame_type, Some(FrameType::Key));
        assert_eq!(core.frame_size, Some(FrameSize::new(64, 64)));
        assert_eq!(core.order_hint_lsb, Some(0)); // order_hint f(0) yields 0
        assert_eq!(core.refresh_frame_flags, Some(3)); // all-frames inference (NumRefFrames == 2)
        assert_eq!(core.immediate_output_frame, Some(true));
        assert_eq!(core.implicit_output_frame, Some(false));

        // Round-trips: the existing §5.18.2 writer re-emits a stream reparsing to an equal core.
        let mut writer = BitWriter::new();
        write_frame_header_core(&mut writer, &core, &seq, None, true).unwrap();
        let bytes = writer.into_bytes();
        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
        let prefix =
            parse_frame_header_prefix(&mut reader, ObuType::ClosedLoopKey, Some(true)).unwrap();
        let mut reparsed = init_core_from_prefix(&prefix, ObuType::ClosedLoopKey, true);
        parse_core_body(
            &mut reader,
            &mut reparsed,
            &seq,
            None,
            &FrameReferenceStateView::unknown(),
        )
        .unwrap();
        reparsed.consumed_bits = reader.consumed_bits();
        assert_eq!(reparsed, core);
    }

    #[test]
    fn encode_minimal_intra_clk_tile_group_obu_round_trips() {
        // Five coded tile bytes with a distinct marker each (the writer-test pattern).
        let tile_data: Vec<u8> = (0u8..5).map(|b| b.wrapping_mul(37)).collect();
        let bytes = encode_minimal_intra_clk_tile_group_obu(&tile_data).unwrap();

        // The payload is a valid first tile group carrying an embedded frame header.
        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
        let prefix =
            parse_tile_group_prefix(&mut reader, ObuType::ClosedLoopKey, Some(true)).unwrap();
        assert!(prefix.is_first_tile_group);
        assert!(prefix.frame_header_present_flag);
        assert!(prefix.frame_header.is_some());

        // The lone (last) tile reads no size field and takes the byte-aligned remainder, so the
        // coded tile bytes are the trailing region of the payload.
        assert_eq!(
            &bytes[bytes.len() - tile_data.len()..],
            tile_data.as_slice()
        );
    }

    #[test]
    fn encode_minimal_intra_clk_tile_group_obu_rejects_empty_tile_data() {
        // §8.2.2: a zero-size tile is a framing defect the writer rejects — a typed error, no panic.
        let err = encode_minimal_intra_clk_tile_group_obu(&[]).unwrap_err();
        assert!(matches!(err, MinimalIntraTileGroupError::Write(_)));
    }

    #[test]
    fn encode_minimal_intra_clk_annexb_obu_round_trips() {
        let tile_data: Vec<u8> = (0u8..5).map(|b| b.wrapping_mul(37)).collect();
        let bytes = encode_minimal_intra_clk_annexb_obu(&tile_data).unwrap();

        // The Annex B stream reparses cleanly as exactly one OBU_CLOSED_LOOP_KEY.
        let parsed = parse_annex_b_obus_partial(&bytes);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.obus.len(), 1);
        let obu = &parsed.obus[0];
        assert_eq!(obu.header.obu_type, ObuType::ClosedLoopKey);
        assert!(!obu.header.has_header_extension);

        // The carried payload is exactly the tile_group_obu() payload (ending in the tile bytes).
        let payload = encode_minimal_intra_clk_tile_group_obu(&tile_data).unwrap();
        assert_eq!(obu.payload, payload.as_slice());
        assert_eq!(
            &obu.payload[obu.payload.len() - tile_data.len()..],
            tile_data.as_slice()
        );
    }

    #[test]
    fn encode_minimal_intra_clk_annexb_obu_rejects_empty_tile_data() {
        // Propagates the inner zero-size-tile rejection — a typed error, no panic.
        let err = encode_minimal_intra_clk_annexb_obu(&[]).unwrap_err();
        assert!(matches!(err, MinimalIntraTileGroupError::Write(_)));
    }
}
