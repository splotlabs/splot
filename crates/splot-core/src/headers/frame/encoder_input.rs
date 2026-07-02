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
use crate::headers::sequence::{ChromaFormatIdc, SequenceHeader, parse_sequence_header};
use crate::headers::tile_group::{TileGroupFraming, TileGroupStructure};
use crate::ivf::{IvfHeader, write_ivf_frame, write_ivf_header};
use crate::obu::{ObuHeader, ParsedObu};
use crate::span::ByteOffset;
use crate::types::{EmbeddedLayerId, ExtendedLayerId, GLOBAL_XLAYER_ID, ObuType, TemporalLayerId};
use crate::write::{
    BitWriter, WriteError, WriteResult, write_annexb_obu, write_obu_payload, write_tile_group_obu,
};

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
            enable_tip_output: false,
            seq_max_drl_bits_minus_1: 0,
            allow_frame_max_drl_bits: false,
            enable_flex_mvres: false,
            seq_frame_motion_modes_present_flag: false,
            seq_enabled_motion_modes: [false; 5],
            enable_opfl_refine: 0,
            enable_bawp: false,
            enable_global_motion: false,
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
                seq_level_idx: LevelIdx::from_bits(31),
            },
            filter: CoreSeqFilterView {
                enable_cdef: false,
                enable_gdf: false,
                gdf_unit_matches_sb_size: false,
                disable_loopfilters_across_tiles: false,
                cdef_on_skip_txfm: CdefOnSkipTxfm::Adaptive,
                df_par_bits_minus_2: 0,
                enable_df_sub_pu: false,
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
        view.single_picture_header_flag = true;
        view.filter.single_picture_header_flag = true;
        view.ccso.single_picture_header_flag = true;
        view.order_hint_bits = 0;
        view.num_ref_frames = 2;
        view.seq_force_screen_content_tools = 2;
        view.seq_force_integer_mv = 2;
        view.tile.enable_avg_cdf = true;
        view.tile.avg_cdf_type = 1;
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
    /// A `base_q_idx == 0` was requested. With the canonical body's zero quantizer deltas and
    /// disabled segmentation that would make `CodedLossless == 1`, changing the § 5.18.2 body
    /// bit layout the fixed canonical writer does not model.
    #[error("base_q_idx == 0 (CodedLossless) is not supported by the canonical minimal-intra body")]
    LosslessBaseQIdx,
}

/// The frozen minimal-intra conformance tier's `base_q_idx`. Any nonzero value keeps
/// `CodedLossless == 0` (so the canonical fixed body layout parses unchanged); 255 is the
/// historical frozen-tier choice. The general skip path supplies its own nonzero value.
const FROZEN_TIER_BASE_Q_IDX: u8 = 255;

/// Serializes the canonical 64x64 single-picture `OBU_CLOSED_LOOP_KEY` intra
/// `frame_header()` body (§ 5.18.2
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`), MSB-first, against the
/// [`CoreSeqView::new_minimal_intra_single_picture`] sequence shape, with `base_q_idx` as
/// the only variable field. Every other element is a fixed minimal-intra choice, so the
/// writes provably never overflow; the round-trip test proves the byte sequence parses back
/// to an `IntraHeaderComplete` core. `base_q_idx` must be nonzero (a zero value would make
/// `CodedLossless == 1` and change the body's bit layout); callers enforce that.
fn minimal_intra_clk_body_bytes(base_q_idx: u8) -> WriteResult<Vec<u8>> {
    let mut writer = BitWriter::new();
    writer.write_uvlc(0)?; // cur_mfh_id == 0 (direct reference, no MFH)
    writer.write_uvlc(0)?; // seq_header_id_in_frame_header == 0
    writer.write_bit(0)?; // allow_screen_content_tools (SCC == SELECT forces this; 0 => no force_integer_mv)
    writer.write_bit(0)?; // allow_intrabc
    writer.write_bit(0)?; // disable_cdf_update
    writer.write_bit(1)?; // uniform_tile_spacing_flag (64x64 -> 1 superblock -> no increment bits)
    writer.write_bits(u32::from(base_q_idx), 8)?; // base_q_idx (nonzero avoids CodedLossless)
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
///
/// # Errors
/// Returns [`MinimalIntraCoreError::Seq`] if the internal minimal-intra sequence view cannot
/// be built, or a parse error if the canonical body does not parse against it. (The frozen
/// tier's `base_q_idx` is nonzero, so [`MinimalIntraCoreError::LosslessBaseQIdx`] never fires.)
pub fn build_minimal_intra_clk_core()
-> Result<(FrameHeaderCore, CoreSeqView), MinimalIntraCoreError> {
    build_minimal_intra_clk_core_impl(FROZEN_TIER_BASE_Q_IDX)
}

/// Like [`build_minimal_intra_clk_core`] but at a caller-chosen `base_q_idx`, for the
/// general intra path whose coefficient symbols are coded at the q-context the decoder
/// derives from `base_q_idx` (`base_q_idx <= 90` selects q-context 0). The rest of the
/// canonical body is unchanged, so only the `base_q_idx` bits differ. `base_q_idx` must be
/// nonzero — a zero value makes `CodedLossless == 1` and changes the body layout, which the
/// fixed canonical body does not model (rejected with [`MinimalIntraCoreError::LosslessBaseQIdx`]).
fn build_minimal_intra_clk_core_impl(
    base_q_idx: u8,
) -> Result<(FrameHeaderCore, CoreSeqView), MinimalIntraCoreError> {
    use crate::headers::sequence::SuperblockSize;
    if base_q_idx == 0 {
        return Err(MinimalIntraCoreError::LosslessBaseQIdx);
    }
    let mut seq = CoreSeqView::new_minimal_intra_single_picture(FROZEN_TIER_DIM, FROZEN_TIER_DIM)
        .ok_or(MinimalIntraCoreError::Seq)?;
    seq.tile.seq_sb_size = SuperblockSize::Block64x64;
    seq.tile.use_128x128_superblock = false;
    let data = minimal_intra_clk_body_bytes(base_q_idx)?;
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
///
/// # Errors
/// Returns [`MinimalIntraTileGroupError::Core`] if the matched frame-header core cannot be
/// assembled, or [`MinimalIntraTileGroupError::Write`] if the § 5.19 tile-group payload cannot
/// be serialized (e.g. an empty `tile_data`, a § 8.2.2 zero-size-tile defect).
pub fn encode_minimal_intra_clk_tile_group_obu(
    tile_data: &[u8],
) -> Result<Vec<u8>, MinimalIntraTileGroupError> {
    encode_minimal_intra_clk_tile_group_obu_impl(FROZEN_TIER_BASE_Q_IDX, tile_data)
}

fn encode_minimal_intra_clk_tile_group_obu_impl(
    base_q_idx: u8,
    tile_data: &[u8],
) -> Result<Vec<u8>, MinimalIntraTileGroupError> {
    let (core, seq) = build_minimal_intra_clk_core_impl(base_q_idx)?;
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
///
/// # Errors
/// Returns [`MinimalIntraTileGroupError::Core`] if the matched frame-header core cannot be
/// assembled, or [`MinimalIntraTileGroupError::Write`] if the tile-group payload or the
/// Annex B OBU wrapper cannot be serialized (e.g. an empty `tile_data`).
pub fn encode_minimal_intra_clk_annexb_obu(
    tile_data: &[u8],
) -> Result<Vec<u8>, MinimalIntraTileGroupError> {
    encode_minimal_intra_clk_annexb_obu_impl(FROZEN_TIER_BASE_Q_IDX, tile_data)
}

fn encode_minimal_intra_clk_annexb_obu_impl(
    base_q_idx: u8,
    tile_data: &[u8],
) -> Result<Vec<u8>, MinimalIntraTileGroupError> {
    let payload = encode_minimal_intra_clk_tile_group_obu_impl(base_q_idx, tile_data)?;
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

/// Serializes a standalone AV2 `OBU_TEMPORAL_DELIMITER` (§ 5.5) in Annex B framing (§ B.2,
/// `docs/spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-2`): a
/// `leb128` size prefix, the § 5.2.2 OBU header, and an empty payload. The temporal delimiter
/// has no body; it marks the start of a temporal unit and is the first OBU the decoder's
/// minimal-tier IVF frame requires (before the sequence-header and frame OBUs).
///
/// The § 5.2.2 header is the no-extension `OBU_TEMPORAL_DELIMITER` header: `obu_mlayer_id` is
/// inferred `0` and `obu_xlayer_id` the global id (`31`) — the temporal delimiter and
/// `OBU_MSDO` are the two types that infer the global xlayer (§ 5.2.2). The canonical encoding
/// is the two bytes `[0x01, 0x08]`.
///
/// This is a public encoder writer-input primitive: a later brick concatenates it with the
/// sequence-header and frame OBUs into a temporal unit and an IVF stream.
///
/// # Errors
/// Returns [`WriteError`] if the Annex B OBU framing cannot be serialized. (Unreachable for
/// the fixed two-byte temporal-delimiter encoding; the `Result` honors the no-panic policy.)
pub fn encode_temporal_delimiter_obu() -> Result<Vec<u8>, WriteError> {
    let header = ObuHeader {
        has_header_extension: false,
        obu_type: ObuType::TemporalDelimiter,
        temporal_layer_id: TemporalLayerId::from_bits(0),
        embedded_layer_id: EmbeddedLayerId::from_bits(0),
        extended_layer_id: GLOBAL_XLAYER_ID,
        header_size_bytes: 1,
    };
    let mut writer = BitWriter::new();
    write_annexb_obu(&mut writer, &header, &[])?;
    Ok(writer.into_bytes())
}

/// The canonical 64x64 single-picture intra `OBU_SEQUENCE_HEADER` **payload** (§ 5.4,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4`): the 11 payload bytes of the
/// `OBU_SEQUENCE_HEADER` in the committed `syn-cos-intra-64x64-q180` conformance vector — the
/// `sequence_header()` body **plus** the § 5.2.1 / § 5.2.3 OBU tail (`obu_extension_flag = 0`
/// then `trailing_bits()`). It is tier-level — independent of the frame's `base_q_idx` and
/// coded tile content — so it is shared by every frame of the tier.
const MINIMAL_INTRA_SEQUENCE_HEADER_PAYLOAD: [u8; 11] = [
    0x82, 0x0a, 0x55, 0xff, 0xf0, 0xc0, 0x04, 0xd1, 0x16, 0xe0, 0x22,
];

/// Error assembling the canonical minimal-intra sequence header
/// ([`build_minimal_intra_sequence_header`] / [`encode_minimal_intra_sequence_header_obu`]):
/// the canonical body either failed to parse or failed to serialize. Both arms are
/// unreachable for the fixed canonical body; they exist only to honor the no-panic policy.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MinimalIntraSequenceHeaderError {
    /// The canonical body could not be parsed into a [`SequenceHeader`].
    #[error("canonical sequence-header body did not parse: {0}")]
    Parse(#[from] crate::error::Error),
    /// The sequence-header OBU could not be serialized.
    #[error("sequence-header OBU serialization failed: {0}")]
    Write(#[from] WriteError),
}

/// Assembles the canonical minimal-intra [`SequenceHeader`] for the frozen 64x64
/// single-picture tier by **parsing** the committed conformance-vector payload (the
/// `MINIMAL_INTRA_SEQUENCE_HEADER_PAYLOAD` const) — the parse-backed model is conformant by
/// construction (it is the exact sequence header the decoder's minimal tier accepts).
///
/// # Errors
/// Returns [`MinimalIntraSequenceHeaderError::Parse`] if the committed canonical payload does
/// not parse into a [`SequenceHeader`]. (Unreachable for the fixed canonical body; the
/// `Result` honors the no-panic policy.)
pub fn build_minimal_intra_sequence_header()
-> Result<SequenceHeader, MinimalIntraSequenceHeaderError> {
    let mut reader = BitReader::new(&MINIMAL_INTRA_SEQUENCE_HEADER_PAYLOAD, ByteOffset::new(0));
    Ok(parse_sequence_header(&mut reader)?)
}

/// Serializes the canonical minimal-intra `OBU_SEQUENCE_HEADER` **payload** — the
/// `sequence_header()` body plus the § 5.2.1 / § 5.2.3 OBU tail (`obu_extension_flag = 0`
/// then `trailing_bits()`, since the sequence header is an extensible OBU). This is what
/// [`write_obu_payload`] emits (`write_sequence_header` writes the body alone, without the
/// tail), so the bytes match the committed conformance vector's sequence-header payload.
fn minimal_intra_sequence_header_payload() -> Result<Vec<u8>, MinimalIntraSequenceHeaderError> {
    let seq = build_minimal_intra_sequence_header()?;
    let mut writer = BitWriter::new();
    write_obu_payload(
        &mut writer,
        &ParsedObu::SequenceHeader(Box::new(seq)),
        ObuType::SequenceHeader.is_extensible_obu(),
        &[],
    )?;
    Ok(writer.into_bytes())
}

/// Serializes the canonical minimal-intra `OBU_SEQUENCE_HEADER` (§ 5.4) in Annex B framing
/// (§ B.2, `docs/spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-2`):
/// a `leb128` size prefix, the no-extension § 5.2.2 `OBU_SEQUENCE_HEADER` header (inferred
/// layer ids `0`), then the body-plus-tail payload of
/// [`build_minimal_intra_sequence_header`]. The result reproduces the committed conformance
/// vector's sequence-header OBU byte-for-byte.
///
/// This is the second of the two OBUs the decoder's minimal-tier IVF frame requires (after
/// the temporal delimiter, before the frame OBU). Assembling the three into a temporal unit
/// and an IVF stream — with the frame OBU made consistent with this sequence header — is a
/// later brick.
///
/// # Errors
/// Returns [`MinimalIntraSequenceHeaderError::Parse`] if the canonical payload does not parse,
/// or [`MinimalIntraSequenceHeaderError::Write`] if the payload or Annex B OBU wrapper cannot
/// be serialized. (Both are unreachable for the fixed canonical body.)
pub fn encode_minimal_intra_sequence_header_obu() -> Result<Vec<u8>, MinimalIntraSequenceHeaderError>
{
    let payload = minimal_intra_sequence_header_payload()?;
    let header = ObuHeader {
        has_header_extension: false,
        obu_type: ObuType::SequenceHeader,
        temporal_layer_id: TemporalLayerId::from_bits(0),
        embedded_layer_id: EmbeddedLayerId::from_bits(0),
        extended_layer_id: ExtendedLayerId::from_bits(0),
        header_size_bytes: 1,
    };
    let mut writer = BitWriter::new();
    write_annexb_obu(&mut writer, &header, &payload)?;
    Ok(writer.into_bytes())
}

/// Error assembling the canonical minimal-intra IVF temporal unit
/// ([`encode_minimal_intra_clk_ivf`]): one of the three OBUs could not be built, or the IVF
/// container could not be written.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MinimalIntraIvfError {
    /// The sequence-header OBU could not be assembled.
    #[error("sequence-header OBU assembly failed: {0}")]
    SequenceHeader(#[from] MinimalIntraSequenceHeaderError),
    /// The frame (tile-group) OBU could not be assembled.
    #[error("frame OBU assembly failed: {0}")]
    Frame(#[from] MinimalIntraTileGroupError),
    /// The temporal-delimiter OBU could not be serialized.
    #[error("temporal-delimiter OBU serialization failed: {0}")]
    Write(#[from] WriteError),
    /// The IVF container could not be written.
    #[error("IVF container write failed: {0}")]
    Ivf(#[from] std::io::Error),
}

/// Assembles the canonical minimal-intra 64x64 single-picture intra temporal unit as a
/// complete IVF stream: the AV2 Annex B temporal unit — `OBU_TEMPORAL_DELIMITER`,
/// `OBU_SEQUENCE_HEADER`, then the `OBU_CLOSED_LOOP_KEY` frame OBU, in the order the
/// decoder's minimal tier requires — inside one `AV02` 64x64 IVF frame
/// ([`write_ivf_header`] / [`write_ivf_frame`]).
///
/// The three OBUs are consistent: the frame OBU's frame header parses against this sequence
/// header (both describe the frozen 64x64 single-picture `Block64x64` tier — verified field
/// by field). `tile_data` is the § 8.2 coded bytes of the one 64x64 tile (`>= 1`); an empty
/// slice is rejected by the inner frame assembler.
///
/// This is the encoder writer-input bridge's container end-point: it emits a structurally
/// valid IVF whose OBUs and headers are consistent. It is **not** yet a hash-exact match to
/// the committed conformance vector — `tile_data` is a caller input, so a complete
/// spec-conformant coded tile (and thus a decode-hash match) is a later brick.
///
/// # Errors
/// Returns a [`MinimalIntraIvfError`] if any of the three OBUs cannot be assembled (e.g. an
/// empty `tile_data` rejected by the inner frame assembler) or the IVF container cannot be
/// written.
pub fn encode_minimal_intra_clk_ivf(tile_data: &[u8]) -> Result<Vec<u8>, MinimalIntraIvfError> {
    encode_minimal_intra_clk_ivf_impl(FROZEN_TIER_BASE_Q_IDX, tile_data)
}

/// Like [`encode_minimal_intra_clk_ivf`] but with a caller-chosen `base_q_idx`, for the
/// general intra path whose coded tile bytes are entropy-coded at the coefficient CDF
/// q-context the decoder derives from `base_q_idx` (`base_q_idx <= 90` → q-context 0). Only
/// the `base_q_idx` bits of the frame header differ from the frozen tier; the container is
/// otherwise identical. `base_q_idx` must be nonzero (a zero value is rejected by the inner
/// frame-header assembler as [`MinimalIntraCoreError::LosslessBaseQIdx`]).
///
/// This pairs with a coded `tile_data` whose symbols match `base_q_idx`'s q-context to emit a
/// decodable stream; the cross-crate decode oracle that proves it is a later brick.
///
/// # Errors
/// Returns a [`MinimalIntraIvfError`] if any of the three OBUs cannot be assembled — including
/// a `base_q_idx == 0` rejected as [`MinimalIntraCoreError::LosslessBaseQIdx`] or an empty
/// `tile_data` — or the IVF container cannot be written.
pub fn encode_minimal_intra_clk_ivf_with_base_q_idx(
    base_q_idx: u8,
    tile_data: &[u8],
) -> Result<Vec<u8>, MinimalIntraIvfError> {
    encode_minimal_intra_clk_ivf_impl(base_q_idx, tile_data)
}

/// Assembles the canonical minimal-intra 64x64 single-picture intra **temporal unit** (one coded
/// access unit) with a caller-chosen `base_q_idx`: the AV2 Annex B temporal unit —
/// `OBU_TEMPORAL_DELIMITER`, `OBU_SEQUENCE_HEADER`, then the `OBU_CLOSED_LOOP_KEY` frame OBU, each
/// in Annex B framing, in the order the decoder's minimal tier requires. This is the access-unit
/// bytes *inside* [`encode_minimal_intra_clk_ivf_with_base_q_idx`]'s IVF frame, without the IVF
/// file header / per-frame record — so it is self-delimiting and decodes directly (the decoder
/// auto-detects it as Annex B), and concatenating several yields a valid Annex B stream.
///
/// `base_q_idx <= 90` selects coefficient CDF q-context 0, matching a `tile_data` coded at that
/// q-context. `base_q_idx` must be nonzero. `tile_data` is the § 8.2 coded tile bytes (`>= 1`).
///
/// # Errors
/// Returns a [`MinimalIntraIvfError`] if the temporal-delimiter, sequence-header, or frame OBU
/// cannot be assembled — including a `base_q_idx == 0` rejected as
/// [`MinimalIntraCoreError::LosslessBaseQIdx`] or an empty `tile_data`.
pub fn encode_minimal_intra_clk_temporal_unit_with_base_q_idx(
    base_q_idx: u8,
    tile_data: &[u8],
) -> Result<Vec<u8>, MinimalIntraIvfError> {
    let mut temporal_unit = encode_temporal_delimiter_obu()?;
    temporal_unit.extend_from_slice(&encode_minimal_intra_sequence_header_obu()?);
    temporal_unit.extend_from_slice(&encode_minimal_intra_clk_annexb_obu_impl(
        base_q_idx, tile_data,
    )?);
    Ok(temporal_unit)
}

fn encode_minimal_intra_clk_ivf_impl(
    base_q_idx: u8,
    tile_data: &[u8],
) -> Result<Vec<u8>, MinimalIntraIvfError> {
    let temporal_unit =
        encode_minimal_intra_clk_temporal_unit_with_base_q_idx(base_q_idx, tile_data)?;

    let mut ivf = Vec::new();
    write_ivf_header(&mut ivf, &IvfHeader::new(*b"AV02", 64, 64, 30, 1, 1))?;
    write_ivf_frame(&mut ivf, 0, &temporal_unit)?;
    Ok(ivf)
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
        let base = CoreSeqView::new_minimal_intra(4096, 2304).unwrap();
        assert_eq!((base.frame_width_bits, base.frame_height_bits), (12, 12));
        assert_eq!((base.max_frame_width, base.max_frame_height), (4096, 2304));

        let small = CoreSeqView::new_minimal_intra(64, 64).unwrap();
        assert_eq!((small.frame_width_bits, small.frame_height_bits), (6, 6));

        let bits = |max| CoreSeqView::new_minimal_intra(max, max).map(|v| v.frame_width_bits);
        assert_eq!(bits(1), Some(1));
        assert_eq!(bits(1 << 16), Some(16));
        assert_eq!(bits(0), None);
        assert_eq!(bits((1 << 16) + 1), None);

        assert!(!base.single_picture_header_flag);
        assert!(!base.filter.single_picture_header_flag);
        assert!(!base.ccso.single_picture_header_flag);
    }

    #[test]
    fn new_minimal_intra_single_picture_applies_eight_spec_inferences() {
        let base = CoreSeqView::new_minimal_intra(64, 64).unwrap();
        let sp = CoreSeqView::new_minimal_intra_single_picture(64, 64).unwrap();

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

        assert!(CoreSeqView::new_minimal_intra_single_picture(0, 0).is_none());
        assert!(CoreSeqView::new_minimal_intra_single_picture((1 << 16) + 1, 64).is_none());
        assert_eq!((sp.max_frame_width, sp.max_frame_height), (64, 64));
        assert_eq!((sp.frame_width_bits, sp.frame_height_bits), (6, 6));
    }

    #[test]
    fn build_minimal_intra_clk_core_round_trips() {
        let (core, seq) = build_minimal_intra_clk_core().unwrap();
        assert_eq!((seq.max_frame_width, seq.max_frame_height), (64, 64));
        assert!(seq.single_picture_header_flag);
        assert_eq!(
            seq.tile.seq_sb_size,
            crate::headers::sequence::SuperblockSize::Block64x64
        );
        assert!(!seq.tile.use_128x128_superblock);

        assert_eq!(core.status, FrameHeaderParseStatus::IntraHeaderComplete);
        assert_eq!(core.frame_type, Some(FrameType::Key));
        assert_eq!(core.frame_size, Some(FrameSize::new(64, 64)));
        assert_eq!(core.order_hint_lsb, Some(0)); // order_hint f(0) yields 0
        assert_eq!(core.refresh_frame_flags, Some(3)); // all-frames inference (NumRefFrames == 2)
        assert_eq!(core.immediate_output_frame, Some(true));
        assert_eq!(core.implicit_output_frame, Some(false));

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
        let tile_data: Vec<u8> = (0u8..5).map(|b| b.wrapping_mul(37)).collect();
        let bytes = encode_minimal_intra_clk_tile_group_obu(&tile_data).unwrap();

        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
        let prefix =
            parse_tile_group_prefix(&mut reader, ObuType::ClosedLoopKey, Some(true)).unwrap();
        assert!(prefix.is_first_tile_group);
        assert!(prefix.frame_header_present_flag);
        assert!(prefix.frame_header.is_some());

        assert_eq!(
            &bytes[bytes.len() - tile_data.len()..],
            tile_data.as_slice()
        );
    }

    #[test]
    fn encode_minimal_intra_clk_tile_group_obu_rejects_empty_tile_data() {
        let err = encode_minimal_intra_clk_tile_group_obu(&[]).unwrap_err();
        assert!(matches!(err, MinimalIntraTileGroupError::Write(_)));
    }

    #[test]
    fn encode_minimal_intra_clk_annexb_obu_round_trips() {
        let tile_data: Vec<u8> = (0u8..5).map(|b| b.wrapping_mul(37)).collect();
        let bytes = encode_minimal_intra_clk_annexb_obu(&tile_data).unwrap();

        let parsed = parse_annex_b_obus_partial(&bytes);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.obus.len(), 1);
        let obu = &parsed.obus[0];
        assert_eq!(obu.header.obu_type, ObuType::ClosedLoopKey);
        assert!(!obu.header.has_header_extension);

        let payload = encode_minimal_intra_clk_tile_group_obu(&tile_data).unwrap();
        assert_eq!(obu.payload, payload.as_slice());
        assert_eq!(
            &obu.payload[obu.payload.len() - tile_data.len()..],
            tile_data.as_slice()
        );
    }

    #[test]
    fn encode_minimal_intra_clk_annexb_obu_rejects_empty_tile_data() {
        let err = encode_minimal_intra_clk_annexb_obu(&[]).unwrap_err();
        assert!(matches!(err, MinimalIntraTileGroupError::Write(_)));
    }

    #[test]
    fn encode_temporal_delimiter_obu_round_trips() {
        let bytes = encode_temporal_delimiter_obu().unwrap();
        assert_eq!(bytes, vec![0x01, 0x08]);

        let parsed = parse_annex_b_obus_partial(&bytes);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.obus.len(), 1);
        assert_eq!(parsed.obus[0].header.obu_type, ObuType::TemporalDelimiter);
        assert!(!parsed.obus[0].header.has_header_extension);
        assert!(parsed.obus[0].payload.is_empty());
    }

    #[test]
    fn minimal_intra_sequence_header_payload_round_trips() {
        let payload = minimal_intra_sequence_header_payload().unwrap();
        assert_eq!(payload, MINIMAL_INTRA_SEQUENCE_HEADER_PAYLOAD);
    }

    #[test]
    fn encode_minimal_intra_sequence_header_obu_matches_conformance_vector() {
        let bytes = encode_minimal_intra_sequence_header_obu().unwrap();
        let mut expected = vec![0x0c, 0x04];
        expected.extend_from_slice(&MINIMAL_INTRA_SEQUENCE_HEADER_PAYLOAD);
        assert_eq!(bytes, expected);

        let parsed = parse_annex_b_obus_partial(&bytes);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.obus.len(), 1);
        assert_eq!(parsed.obus[0].header.obu_type, ObuType::SequenceHeader);
        assert!(!parsed.obus[0].header.has_header_extension);
        assert_eq!(
            parsed.obus[0].payload,
            &MINIMAL_INTRA_SEQUENCE_HEADER_PAYLOAD[..]
        );
    }

    #[test]
    fn encode_minimal_intra_clk_ivf_assembles_consistent_temporal_unit() {
        let tile_data: Vec<u8> = (0u8..5).map(|b| b.wrapping_mul(37)).collect();
        let ivf = encode_minimal_intra_clk_ivf(&tile_data).unwrap();

        let parsed = crate::ivf::parse_ivf_partial(&ivf);
        assert!(parsed.error.is_none());
        let header = parsed.header.unwrap();
        assert_eq!(&header.fourcc, b"AV02");
        assert_eq!((header.width, header.height), (64, 64));
        assert_eq!(parsed.frames.len(), 1);

        let obus = parse_annex_b_obus_partial(parsed.frames[0].payload);
        assert!(obus.error.is_none());
        let types: Vec<_> = obus.obus.iter().map(|o| o.header.obu_type).collect();
        assert_eq!(
            types,
            vec![
                ObuType::TemporalDelimiter,
                ObuType::SequenceHeader,
                ObuType::ClosedLoopKey,
            ]
        );

        let seq = build_minimal_intra_sequence_header().unwrap();
        let view = CoreSeqView::from_sequence(&seq).unwrap();
        assert!(view.single_picture_header_flag);
        assert_eq!((view.max_frame_width, view.max_frame_height), (64, 64));
        assert_eq!(view.order_hint_bits, 0);
        assert_eq!(
            view.tile.seq_sb_size,
            crate::headers::sequence::SuperblockSize::Block64x64
        );
    }

    #[test]
    fn encode_minimal_intra_clk_ivf_rejects_empty_tile_data() {
        let err = encode_minimal_intra_clk_ivf(&[]).unwrap_err();
        assert!(matches!(err, MinimalIntraIvfError::Frame(_)));
    }

    #[test]
    fn ivf_with_base_q_idx_80_reproduces_the_avm_validated_q80_fixture() {
        const Q80_TILE_DATA: [u8; 10] =
            [0x00, 0x03, 0xb6, 0x27, 0x68, 0x56, 0x9a, 0x3f, 0x2f, 0x20];
        let fixture =
            std::fs::read("../../tests/conformance/vectors/valid/syn-flat-intra-64x64-q80.ivf")
                .unwrap();
        let emitted = encode_minimal_intra_clk_ivf_with_base_q_idx(80, &Q80_TILE_DATA).unwrap();
        assert_eq!(emitted, fixture);
    }

    #[test]
    fn ivf_with_base_q_idx_zero_is_rejected_as_lossless() {
        let err = encode_minimal_intra_clk_ivf_with_base_q_idx(0, &[0x01]).unwrap_err();
        assert!(matches!(
            err,
            MinimalIntraIvfError::Frame(MinimalIntraTileGroupError::Core(
                MinimalIntraCoreError::LosslessBaseQIdx
            ))
        ));
    }
}
