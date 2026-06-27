// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame-header loop-filter parameters: `deblocking_filter_params()`
//! (AV2 v1.0.0 § 5.18.5.2, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-5-2`),
//! `gdf_params()` (§ 5.18.7.9, `#s-5-18-7-9`), and `cdef_params()` (§ 5.18.7.10,
//! `#s-5-18-7-10`).
//!
//! These are the three § 5.18.2 tail structures parsed immediately after the
//! per-segment lossless/`allow_tcq`/`allow_parity_hiding` derivation (mirror :5297-5301)
//! and before `lr_params()` (loop restoration, § 5.18.7.11). They are fully determined
//! by the already-parsed § 5.4.10 `sequence_filter_config()` flags
//! ([`CoreSeqFilterView`]) plus the parsed frame state (`CodedLossless`, `NumPlanes`,
//! the frame `SbSize`, and the parsed `tile_info()` geometry); no reference-frame state
//! is needed, so they parse on the intra path.
//!
//! The `cur_mfh_id > 0` deblocking arm consults the resolved multi-frame header's
//! `mfh_deblocking_filter_update` / `mfh_apply_deblocking_filter` (§ 5.18.5.2, mirror
//! :5949), threaded in as [`MfhDeblockingView`]. On the `cur_mfh_id == 0` direct path
//! and on a `cur_mfh_id > 0` frame whose in-band MFH did not signal an update, the arm
//! reads `apply_deblocking_filter[0..]` from the bitstream instead.

use crate::bitio::BitReader;
use crate::error::{Error, Result};
use crate::headers::sequence::{CdefOnSkipTxfm, SuperblockSize};

/// `GDF_MIN_SIZE` (AV2 v1.0.0 § 3, `docs/spec/av2/1.0.0/03-symbols.md`): minimum size
/// of GDF blocks when `gdf_unit_matches_sb_size` is `0`.
const GDF_MIN_SIZE: u32 = 128;

/// `MI_SIZE` (AV2 v1.0.0 § 3): smallest mode-info block size in luma samples.
const MI_SIZE: u32 = 4;

/// `CdefStrengths` is `cdef_strengths_minus_1 + 1` with `cdef_strengths_minus_1`
/// read as `f(3)` (§ 5.18.7.10), so it is at most `8`. Bounds the per-strength loop
/// allocation.
const MAX_CDEF_STRENGTHS: usize = 8;

/// `Block_Width[SbSize]` (AV2 v1.0.0): the superblock width in luma samples.
const fn block_width(sb_size: SuperblockSize) -> u32 {
    match sb_size {
        SuperblockSize::Block64x64 => 64,
        SuperblockSize::Block128x128 => 128,
        SuperblockSize::Block256x256 => 256,
    }
}

/// Sequence-derived inputs the § 5.18.2 tail loop-filter structures consume, gathered
/// from `sequence_filter_config()` (AV2 v1.0.0 § 5.4.10).
///
/// Mirrors the gating fields each structure reads: `enable_cdef` / `enable_gdf` gate
/// CDEF / GDF (§ 5.18.7.10 / § 5.18.7.9), `gdf_unit_matches_sb_size` and
/// `disable_loopfilters_across_tiles` shape the GDF block size and per-block gate
/// (§ 5.18.7.9), `cdef_on_skip_txfm` selects the CDEF skip-txfm arm (§ 5.18.7.10),
/// `df_par_bits_minus_2` sets the `df_delta_q[i]` width (§ 5.18.5.2), and
/// `single_picture_header_flag` infers the GDF / CDEF frame-enable bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreSeqFilterView {
    /// `enable_cdef` (AV2 § 5.4.10): gates `cdef_params()` past the disabled return.
    pub enable_cdef: bool,
    /// `enable_gdf` (AV2 § 5.4.10): gates `gdf_params()` past the disabled return.
    pub enable_gdf: bool,
    /// `gdf_unit_matches_sb_size` (AV2 § 5.4.10): when set, `gdfBlkSize` is the
    /// superblock width (§ 5.18.7.9).
    pub gdf_unit_matches_sb_size: bool,
    /// `disable_loopfilters_across_tiles` (AV2 § 5.4.10): part of the `gdf_per_block`
    /// gate when the frame is multi-tile (§ 5.18.7.9).
    pub disable_loopfilters_across_tiles: bool,
    /// `CdefOnSkipTxfm` (AV2 § 5.4.10): selects the `cdef_on_skip_txfm_frame_enable`
    /// arm (§ 5.18.7.10).
    pub cdef_on_skip_txfm: CdefOnSkipTxfm,
    /// `df_par_bits_minus_2` (AV2 § 5.4.10): `dfParBits = df_par_bits_minus_2 + 2`
    /// is the `df_delta_q[i]` width (§ 5.18.5.2).
    pub df_par_bits_minus_2: u8,
    /// `enable_df_sub_pu` (AV2 § 5.4.10): gates the `allow_df_sub_pu` `f(1)` read in
    /// `deblocking_filter_params()` on the inter path (`enable_df_sub_pu && FrameType ==
    /// INTER_FRAME`, § 5.18.5.2 mirror :5935). Inert on the intra / switch path.
    pub enable_df_sub_pu: bool,
    /// `single_picture_header_flag` (AV2 § 5.4.1): infers `gdf_frame_enable` /
    /// `cdef_frame_enable` to `1` without reading a bit (§ 5.18.7.9 / § 5.18.7.10).
    pub single_picture_header_flag: bool,
}

/// The resolved multi-frame header's deblocking-filter update state for the
/// `cur_mfh_id > 0` arm of `deblocking_filter_params()` (AV2 v1.0.0 § 5.18.5.2, mirror
/// :5949).
///
/// Built only on the `cur_mfh_id > 0` path with a resolved in-band record; on the
/// `cur_mfh_id == 0` direct path (or a resolved record whose
/// `mfh_deblocking_filter_update` is `0`) the parser reads `apply_deblocking_filter`
/// from the bitstream instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MfhDeblockingView {
    /// `mfh_deblocking_filter_update[cur_mfh_id]` (AV2 § 5.7): selects the MFH arm.
    pub mfh_deblocking_filter_update: bool,
    /// `mfh_apply_deblocking_filter[cur_mfh_id][0..4]` (AV2 § 5.7): copied into
    /// `apply_deblocking_filter[i]` when the update bit is set, gated by `NumPlanes`.
    pub mfh_apply_deblocking_filter: [bool; 4],
}

/// Parsed `deblocking_filter_params()` (AV2 v1.0.0 § 5.18.5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeblockingFilterParams {
    /// `apply_deblocking_filter[0..4]`: luma vertical/horizontal and chroma
    /// vertical/horizontal enable flags (§ 5.18.5.2).
    pub apply_deblocking_filter: [bool; 4],
    /// `df_delta_q_present[0..4]`: whether each `df_delta_q[i]` was signalled.
    pub df_delta_q_present: [bool; 4],
    /// `DfDeltaQ[0..4]`: the derived deblocking delta-Q offsets (§ 5.18.5.2). When
    /// `apply_deblocking_filter[i]` is `0` the offset is `0`; when present it is
    /// `df_delta_q[i] - (1 << (dfParBits - 1))`; when absent it is `DfDeltaQ[0]` for
    /// `i == 1`, else `0`.
    pub df_delta_q: [i32; 4],
}

/// Parsed `gdf_params()` (AV2 v1.0.0 § 5.18.7.9).
///
/// Fields after `gdf_frame_enable` are present only when GDF is frame-enabled (the
/// disabled path returns early per the mirror).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct GdfParams {
    /// `gdf_frame_enable`: `0` when `CodedLossless || !enable_gdf`, inferred `1` for a
    /// single picture, else read (§ 5.18.7.9).
    pub gdf_frame_enable: bool,
    /// `gdf_per_block`: read only when the frame exceeds `gdfBlkSize` or
    /// loop-filtering across tiles is disabled in a multi-tile frame, else inferred
    /// `0` (§ 5.18.7.9). `None` when GDF is not frame-enabled.
    pub gdf_per_block: Option<bool>,
    /// `gdf_pic_qc_idx` (`f(2)`), present only when GDF is frame-enabled.
    pub gdf_pic_qc_idx: Option<u8>,
    /// `gdf_pic_scale_idx` (`f(2)`), present only when GDF is frame-enabled.
    pub gdf_pic_scale_idx: Option<u8>,
}

/// One `(i)` strength set parsed in `cdef_params()` (AV2 v1.0.0 § 5.18.7.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CdefStrengthSet {
    /// `cdef_y_pri_strength[i]` (`0` when `cdef_y_pri_zero`, else `f(4)`).
    pub y_pri_strength: u8,
    /// `cdef_y_sec_strength[i]` (`f(2)`, with the `3 -> 4` remap applied).
    pub y_sec_strength: u8,
    /// `cdef_uv_pri_strength[i]` (`NumPlanes > 1` only; `0` when `cdef_uv_pri_zero`,
    /// else `f(4)`).
    pub uv_pri_strength: u8,
    /// `cdef_uv_sec_strength[i]` (`NumPlanes > 1` only; `f(2)`, with the `3 -> 4`
    /// remap applied).
    pub uv_sec_strength: u8,
}

/// Parsed `cdef_params()` (AV2 v1.0.0 § 5.18.7.10).
///
/// Fields after `cdef_frame_enable` are present only when CDEF is frame-enabled (the
/// disabled paths return early per the mirror).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CdefParams {
    /// `cdef_frame_enable`: `0` when `CodedLossless || !enable_cdef`, inferred `1` for
    /// a single picture, else read (§ 5.18.7.10).
    pub cdef_frame_enable: bool,
    /// `CdefDamping = cdef_damping_minus_3 + 3`, present only when CDEF is
    /// frame-enabled.
    pub cdef_damping: Option<u8>,
    /// `CdefStrengths = cdef_strengths_minus_1 + 1`, present only when CDEF is
    /// frame-enabled.
    pub cdef_strengths: Option<u8>,
    /// `cdef_on_skip_txfm_frame_enable`, present only when CDEF is frame-enabled.
    pub cdef_on_skip_txfm_frame_enable: Option<bool>,
    /// The `CdefStrengths` parsed strength sets, present only when CDEF is
    /// frame-enabled.
    pub strengths: Vec<CdefStrengthSet>,
}

/// Parses `deblocking_filter_params()` (AV2 v1.0.0 § 5.18.5.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-5-2`).
///
/// `read_allow_df_sub_pu` is the caller-derived `enable_df_sub_pu && FrameType ==
/// INTER_FRAME` gate (mirror :5935). On the intra / switch / SEF / writer paths it is
/// `false` (`FrameType != INTER_FRAME`), so the `allow_df_sub_pu` `f(1)` read does not
/// fire; on the inter path the caller passes `true` exactly when the sequence enabled the
/// tool, so the bit is consumed for alignment (the parsed `allow_df_sub_pu` value drives
/// reconstruction this phase does not model, so it is read but not surfaced).
///
/// `coded_lossless` is the frame `CodedLossless`; `num_planes` is `NumPlanes`
/// (`Monochrome ? 1 : 3`, § 6.4.1); `df_par_bits_minus_2` is the § 5.4.10 sequence
/// field. `mfh` is the resolved multi-frame header's deblocking-update state on the
/// `cur_mfh_id > 0` path (`None` on the direct path), gating the `apply_deblocking_filter`
/// source per § 5.18.5.2 (mirror :5949).
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the payload
/// ends mid-field, or [`Error::BitWidthTooLarge`](crate::error::Error::BitWidthTooLarge)
/// if `df_par_bits_minus_2` is large enough that `dfParBits = df_par_bits_minus_2 + 2`
/// exceeds the 32-bit `df_delta_q[i]` read width (an out-of-range, non-conformant
/// sequence field a direct/fuzz caller can supply).
pub fn parse_deblocking_filter_params(
    reader: &mut BitReader<'_>,
    coded_lossless: bool,
    num_planes: u8,
    df_par_bits_minus_2: u8,
    read_allow_df_sub_pu: bool,
    mfh: Option<&MfhDeblockingView>,
) -> Result<DeblockingFilterParams> {
    // AV2 § 5.18.5.2: if ( CodedLossless ) apply_deblocking_filter[0..4] = 0; return.
    if coded_lossless {
        return Ok(DeblockingFilterParams {
            apply_deblocking_filter: [false; 4],
            df_delta_q_present: [false; 4],
            df_delta_q: [0; 4],
        });
    }

    // AV2 § 5.18.5.2 (mirror :5935): if ( enable_df_sub_pu && FrameType == INTER_FRAME )
    // allow_df_sub_pu f(1); else allow_df_sub_pu = 0 (no bit). The caller folds the
    // sequence `enable_df_sub_pu` and the `FrameType == INTER_FRAME` test into
    // `read_allow_df_sub_pu`. The parsed value feeds the deblocking reconstruction this
    // phase does not model, so it is consumed for alignment only.
    if read_allow_df_sub_pu {
        let _allow_df_sub_pu = reader.read_flag()?;
    }

    let mut apply_deblocking_filter = [false; 4];
    // AV2 § 5.18.5.2: if ( mfh_deblocking_filter_update[cur_mfh_id] ) copy the MFH's
    // apply_deblocking_filter; else read apply_deblocking_filter[0]/[1] (and the chroma
    // pair when NumPlanes > 1 and either luma flag is set). Indices [2]/[3] default 0.
    let use_mfh_update = mfh.is_some_and(|view| view.mfh_deblocking_filter_update);
    if let Some(view) = mfh.filter(|view| view.mfh_deblocking_filter_update) {
        apply_deblocking_filter[0] = view.mfh_apply_deblocking_filter[0];
        apply_deblocking_filter[1] = view.mfh_apply_deblocking_filter[1];
        if num_planes > 1 && (apply_deblocking_filter[0] || apply_deblocking_filter[1]) {
            apply_deblocking_filter[2] = view.mfh_apply_deblocking_filter[2];
            apply_deblocking_filter[3] = view.mfh_apply_deblocking_filter[3];
        }
    }
    if !use_mfh_update {
        apply_deblocking_filter[0] = reader.read_flag()?;
        apply_deblocking_filter[1] = reader.read_flag()?;
        if num_planes > 1 && (apply_deblocking_filter[0] || apply_deblocking_filter[1]) {
            apply_deblocking_filter[2] = reader.read_flag()?;
            apply_deblocking_filter[3] = reader.read_flag()?;
        }
    }

    // AV2 § 5.18.5.2: dfParBits = df_par_bits_minus_2 + 2. The sequence parser reads
    // df_par_bits_minus_2 as f(2) (§ 5.4.10), so the conformant range is 0..=3 (dfParBits
    // 2..=5). A direct/fuzz caller is not bound by that read, so guard the read width here:
    // `df_delta_q[i]` is `f(dfParBits)` and `read_bits` rejects `n > 32`, so a dfParBits
    // beyond 32 is an impossible width. Reject it with the same structured
    // `BitWidthTooLarge` before the `DfDeltaQ` derivation. This mirrors `frame_size()`'s
    // reliance on `read_bits` to reject an over-wide `frame_width_bits`.
    let df_par_bits = u32::from(df_par_bits_minus_2) + 2;
    if df_par_bits > 32 {
        return Err(Error::BitWidthTooLarge {
            requested: df_par_bits,
            max: 32,
        });
    }
    // `1 << (dfParBits - 1)` and `df_delta_q[i] - half` are computed in i64 — like
    // `BitReader::read_su` — and narrowed: for dfParBits up to 32 the i32 forms would
    // shift past / overflow i32 and panic in debug. The conformant DfDeltaQ (dfParBits
    // 2..=5) always fits; the wider non-conformant inputs stay panic-free and produce a
    // bounded value rather than an unwind. `dfParBits >= 1` here, so the shift is valid.
    let half = 1i64 << (df_par_bits - 1);

    let mut df_delta_q_present = [false; 4];
    let mut df_delta_q = [0i32; 4];
    for i in 0..4 {
        // AV2 § 5.18.5.2: for ( i = 0; i < 4; i++ ).
        if apply_deblocking_filter[i] {
            df_delta_q_present[i] = reader.read_flag()?;
            if df_delta_q_present[i] {
                // AV2 § 5.18.5.2: df_delta_q[i] f(dfParBits);
                // DfDeltaQ[i] = df_delta_q[i] - (1 << (dfParBits - 1)).
                let raw = i64::from(reader.read_bits(df_par_bits)?);
                df_delta_q[i] = (raw - half) as i32;
            } else {
                // AV2 § 5.18.5.2: DfDeltaQ[i] = (i == 1) ? DfDeltaQ[0] : 0.
                df_delta_q[i] = if i == 1 { df_delta_q[0] } else { 0 };
            }
        } else {
            // AV2 § 5.18.5.2: DfDeltaQ[i] = 0.
            df_delta_q[i] = 0;
        }
    }

    Ok(DeblockingFilterParams {
        apply_deblocking_filter,
        df_delta_q_present,
        df_delta_q,
    })
}

/// The frame-level interpolation filter selected by `read_interpolation_filter()`
/// (AV2 v1.0.0 § 5.18.5.1, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-5-1`).
///
/// `is_filter_switchable` selects [`Self::Switchable`]; otherwise the explicit
/// `interpolation_filter` `f(2)` value names the fixed filter. The four fixed values
/// `0..3` are `EIGHTTAP`, `EIGHTTAP_SMOOTH`, `EIGHTTAP_SHARP`, and `BILINEAR`
/// respectively (AV2 § 3 interpolation-filter constants).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum InterpolationFilter {
    /// `EIGHTTAP` (`interpolation_filter == 0`).
    Eighttap,
    /// `EIGHTTAP_SMOOTH` (`interpolation_filter == 1`).
    EighttapSmooth,
    /// `EIGHTTAP_SHARP` (`interpolation_filter == 2`).
    EighttapSharp,
    /// `BILINEAR` (`interpolation_filter == 3`).
    Bilinear,
    /// `SWITCHABLE` (`is_filter_switchable == 1`).
    Switchable,
}

impl InterpolationFilter {
    /// Returns a stable snake-case label for tools and JSON output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Eighttap => "eighttap",
            Self::EighttapSmooth => "eighttap_smooth",
            Self::EighttapSharp => "eighttap_sharp",
            Self::Bilinear => "bilinear",
            Self::Switchable => "switchable",
        }
    }

    /// Maps the explicit `interpolation_filter` `f(2)` value (`0..3`) to its fixed
    /// filter (AV2 § 5.18.5.1).
    const fn from_fixed_code(code: u32) -> Self {
        match code & 0x3 {
            0 => Self::Eighttap,
            1 => Self::EighttapSmooth,
            2 => Self::EighttapSharp,
            // The mask restricts `code` to `0..=3`; `3` is the only remaining value.
            _ => Self::Bilinear,
        }
    }
}

/// Parses `read_interpolation_filter()` (AV2 v1.0.0 § 5.18.5.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-5-1`).
///
/// ```text
/// read_interpolation_filter( ) {
///     is_filter_switchable                         f(1)
///     if ( is_filter_switchable == 1 ) {
///         interpolation_filter = SWITCHABLE
///     } else {
///         interpolation_filter                     f(2)
///     }
/// }
/// ```
///
/// No reference-frame state gates this read, so it parses on the inter path once
/// reached.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the payload
/// ends before the `is_filter_switchable` bit or the `interpolation_filter` `f(2)` value.
pub fn read_interpolation_filter(reader: &mut BitReader<'_>) -> Result<InterpolationFilter> {
    // AV2 § 5.18.5.1: is_filter_switchable f(1).
    let is_filter_switchable = reader.read_flag()?;
    if is_filter_switchable {
        Ok(InterpolationFilter::Switchable)
    } else {
        // AV2 § 5.18.5.1: interpolation_filter f(2).
        let code = reader.read_bits(2)?;
        Ok(InterpolationFilter::from_fixed_code(code))
    }
}

/// Geometry inputs for the `gdf_per_block` gate of `gdf_params()` (AV2 v1.0.0
/// § 5.18.7.9), derived from the parsed `tile_info()` and frame `SbSize`.
#[derive(Debug, Clone, Copy)]
pub struct GdfGeometry<'a> {
    /// The frame `SbSize` (§ 5.18.2): selects `Block_Width[SbSize]` for `gdfBlkSize`.
    pub sb_size: SuperblockSize,
    /// `MiCols` (`MiColStarts[TileCols]`, § 5.18.4.4).
    pub mi_cols: u32,
    /// `MiRows` (`MiRowStarts[TileRows]`, § 5.18.4.4).
    pub mi_rows: u32,
    /// `TileCols`.
    pub tile_cols: u32,
    /// `TileRows`.
    pub tile_rows: u32,
    /// `MiColStarts[0..TileCols]` (the non-sentinel entries the alignment scan reads).
    pub mi_col_starts: &'a [u32],
    /// `MiRowStarts[0..TileRows]` (the non-sentinel entries the alignment scan reads).
    pub mi_row_starts: &'a [u32],
}

/// Whether the `gdf_per_block` bit is coded in `gdf_params()` (AV2 v1.0.0 § 5.18.7.9,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-9`).
///
/// Returns `true` when the frame exceeds `gdfBlkSize` in either dimension, or when
/// loop-filtering across tiles is disabled in a multi-tile frame; otherwise the bit is
/// inferred `0` and `false` is returned. Encapsulates the full `gdfBlkSize` derivation
/// (the `Max(Block_Width[SbSize], GDF_MIN_SIZE)` default, the `gdf_unit_matches_sb_size`
/// override, and the `Block64x64` tile-start alignment scan) so the parser
/// ([`parse_gdf_params`]) and the writer
/// (`crate::write::frame_filters::write_gdf_params`) share one source of truth and never
/// drift. `gdf_frame_enable` must already be `1` for this gate to be consulted.
pub(crate) fn gdf_per_block_is_coded(filter: CoreSeqFilterView, geometry: GdfGeometry<'_>) -> bool {
    // § 5.18.7.9: gdfBlkSize derivation.
    let sb_block_width = block_width(geometry.sb_size);
    // gdfBlkSize = Max(Block_Width[SbSize], GDF_MIN_SIZE).
    let mut gdf_blk_size = sb_block_width.max(GDF_MIN_SIZE);
    if filter.gdf_unit_matches_sb_size {
        // gdfBlkSize = Block_Width[SbSize].
        gdf_blk_size = sb_block_width;
    } else if geometry.sb_size == SuperblockSize::Block64x64 {
        // Scan tile-start alignment: a |= MiColStarts[i] for i < TileCols, then
        // a |= MiRowStarts[i] for i < TileRows; if ( a & 16 ) gdfBlkSize = 64.
        let mut a = 0u32;
        for &start in geometry
            .mi_col_starts
            .iter()
            .take(geometry.tile_cols as usize)
        {
            a |= start;
        }
        for &start in geometry
            .mi_row_starts
            .iter()
            .take(geometry.tile_rows as usize)
        {
            a |= start;
        }
        if a & 16 != 0 {
            gdf_blk_size = 64;
        }
    }

    // § 5.18.7.9: gdf_per_block f(1) when MiCols*MI_SIZE > gdfBlkSize ||
    // MiRows*MI_SIZE > gdfBlkSize || (disable_loopfilters_across_tiles &&
    // (TileRows > 1 || TileCols > 1)); else gdf_per_block = 0.
    let frame_exceeds_block = geometry.mi_cols.saturating_mul(MI_SIZE) > gdf_blk_size
        || geometry.mi_rows.saturating_mul(MI_SIZE) > gdf_blk_size;
    let multi_tile = geometry.tile_rows > 1 || geometry.tile_cols > 1;
    frame_exceeds_block || (filter.disable_loopfilters_across_tiles && multi_tile)
}

/// Parses `gdf_params()` (AV2 v1.0.0 § 5.18.7.9,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-9`).
///
/// `coded_lossless` is the frame `CodedLossless`; `filter` carries the § 5.4.10
/// `enable_gdf` / `gdf_unit_matches_sb_size` / `disable_loopfilters_across_tiles` flags
/// and `single_picture_header_flag`; `geometry` is the parsed `tile_info()` geometry
/// used to evaluate the `gdf_per_block` gate (via `gdf_per_block_is_coded`).
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the payload
/// ends mid-field.
pub fn parse_gdf_params(
    reader: &mut BitReader<'_>,
    coded_lossless: bool,
    filter: &CoreSeqFilterView,
    geometry: GdfGeometry<'_>,
) -> Result<GdfParams> {
    // AV2 § 5.18.7.9: if ( CodedLossless || !enable_gdf ) gdf_frame_enable = 0.
    if coded_lossless || !filter.enable_gdf {
        return Ok(GdfParams {
            gdf_frame_enable: false,
            gdf_per_block: None,
            gdf_pic_qc_idx: None,
            gdf_pic_scale_idx: None,
        });
    }

    // AV2 § 5.18.7.9: single picture infers gdf_frame_enable = 1; else f(1).
    let gdf_frame_enable = if filter.single_picture_header_flag {
        true
    } else {
        reader.read_flag()?
    };
    if !gdf_frame_enable {
        // AV2 § 5.18.7.9: if ( !gdf_frame_enable ) return.
        return Ok(GdfParams {
            gdf_frame_enable: false,
            gdf_per_block: None,
            gdf_pic_qc_idx: None,
            gdf_pic_scale_idx: None,
        });
    }

    // AV2 § 5.18.7.9: gdf_per_block f(1) when coded (the gdfBlkSize / tile gate, shared
    // with the writer via gdf_per_block_is_coded); else inferred 0.
    let gdf_per_block = if gdf_per_block_is_coded(*filter, geometry) {
        reader.read_flag()?
    } else {
        false
    };

    // AV2 § 5.18.7.9: gdf_pic_qc_idx f(2); gdf_pic_scale_idx f(2).
    let gdf_pic_qc_idx = reader.read_bits_u8(2)?;
    let gdf_pic_scale_idx = reader.read_bits_u8(2)?;

    Ok(GdfParams {
        gdf_frame_enable: true,
        gdf_per_block: Some(gdf_per_block),
        gdf_pic_qc_idx: Some(gdf_pic_qc_idx),
        gdf_pic_scale_idx: Some(gdf_pic_scale_idx),
    })
}

/// Parses `cdef_params()` (AV2 v1.0.0 § 5.18.7.10,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-10`).
///
/// `coded_lossless` is the frame `CodedLossless`; `num_planes` is `NumPlanes`; `filter`
/// carries the § 5.4.10 `enable_cdef`, `cdef_on_skip_txfm`, and
/// `single_picture_header_flag` flags.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the payload
/// ends mid-field.
pub fn parse_cdef_params(
    reader: &mut BitReader<'_>,
    coded_lossless: bool,
    num_planes: u8,
    filter: &CoreSeqFilterView,
) -> Result<CdefParams> {
    // AV2 § 5.18.7.10: if ( CodedLossless || !enable_cdef ) cdef_frame_enable = 0; return.
    if coded_lossless || !filter.enable_cdef {
        return Ok(CdefParams {
            cdef_frame_enable: false,
            cdef_damping: None,
            cdef_strengths: None,
            cdef_on_skip_txfm_frame_enable: None,
            strengths: Vec::new(),
        });
    }

    // AV2 § 5.18.7.10: single picture infers cdef_frame_enable = 1; else f(1).
    let cdef_frame_enable = if filter.single_picture_header_flag {
        true
    } else {
        reader.read_flag()?
    };
    if !cdef_frame_enable {
        // AV2 § 5.18.7.10: if ( !cdef_frame_enable ) return.
        return Ok(CdefParams {
            cdef_frame_enable: false,
            cdef_damping: None,
            cdef_strengths: None,
            cdef_on_skip_txfm_frame_enable: None,
            strengths: Vec::new(),
        });
    }

    // AV2 § 5.18.7.10: cdef_damping_minus_3 f(2); CdefDamping = cdef_damping_minus_3 + 3.
    let cdef_damping = reader.read_bits_u8(2)? + 3;
    // cdef_strengths_minus_1 f(3); CdefStrengths = cdef_strengths_minus_1 + 1 (1..=8).
    let cdef_strengths = reader.read_bits_u8(3)? + 1;

    // AV2 § 5.18.7.10: cdef_on_skip_txfm_frame_enable per CdefOnSkipTxfm.
    let cdef_on_skip_txfm_frame_enable = match filter.cdef_on_skip_txfm {
        CdefOnSkipTxfm::Adaptive => reader.read_flag()?,
        CdefOnSkipTxfm::AlwaysOn => true,
        CdefOnSkipTxfm::Disabled => false,
    };

    // AV2 § 5.18.7.10: for ( i = 0; i < CdefStrengths; i++ ). CdefStrengths <= 8.
    let mut strengths = Vec::with_capacity(usize::from(cdef_strengths).min(MAX_CDEF_STRENGTHS));
    for _ in 0..cdef_strengths {
        // cdef_y_pri_zero f(1); cdef_y_pri_strength[i] = 0 or f(4).
        let y_pri_strength = if reader.read_flag()? {
            0
        } else {
            reader.read_bits_u8(4)?
        };
        // cdef_y_sec_strength[i] f(2); if ( == 3 ) += 1.
        let mut y_sec_strength = reader.read_bits_u8(2)?;
        if y_sec_strength == 3 {
            y_sec_strength += 1;
        }
        let (uv_pri_strength, uv_sec_strength) = if num_planes > 1 {
            // cdef_uv_pri_zero f(1); cdef_uv_pri_strength[i] = 0 or f(4).
            let uv_pri = if reader.read_flag()? {
                0
            } else {
                reader.read_bits_u8(4)?
            };
            // cdef_uv_sec_strength[i] f(2); if ( == 3 ) += 1.
            let mut uv_sec = reader.read_bits_u8(2)?;
            if uv_sec == 3 {
                uv_sec += 1;
            }
            (uv_pri, uv_sec)
        } else {
            (0, 0)
        };
        strengths.push(CdefStrengthSet {
            y_pri_strength,
            y_sec_strength,
            uv_pri_strength,
            uv_sec_strength,
        });
    }

    Ok(CdefParams {
        cdef_frame_enable: true,
        cdef_damping: Some(cdef_damping),
        cdef_strengths: Some(cdef_strengths),
        cdef_on_skip_txfm_frame_enable: Some(cdef_on_skip_txfm_frame_enable),
        strengths,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::span::ByteOffset;
    use crate::test_support::base_geometry;

    use crate::test_bits::Bits;

    fn reader(data: &[u8]) -> BitReader<'_> {
        BitReader::new(data, ByteOffset::new(0))
    }

    fn base_filter() -> CoreSeqFilterView {
        CoreSeqFilterView {
            enable_cdef: true,
            enable_gdf: true,
            gdf_unit_matches_sb_size: false,
            disable_loopfilters_across_tiles: false,
            cdef_on_skip_txfm: CdefOnSkipTxfm::Adaptive,
            df_par_bits_minus_2: 0,
            enable_df_sub_pu: false,
            single_picture_header_flag: false,
        }
    }

    // ---- deblocking ----

    #[test]
    fn deblocking_coded_lossless_reads_no_bits() {
        let mut r = reader(&[]);
        let params = parse_deblocking_filter_params(&mut r, true, 3, 0, false, None).unwrap();
        assert_eq!(params.apply_deblocking_filter, [false; 4]);
        assert_eq!(params.df_delta_q, [0; 4]);
        assert_eq!(r.consumed_bits(), 0);
    }

    #[test]
    fn deblocking_direct_reads_apply_and_delta_q() {
        // NumPlanes == 3, df_par_bits_minus_2 == 0 -> dfParBits = 2.
        let mut bits = Bits::default();
        bits.bit(1); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        // luma flag set -> chroma pair read.
        bits.bit(1); // apply_deblocking_filter[2]
        bits.bit(0); // apply_deblocking_filter[3]
        // i == 0: present, df_delta_q[0] f(2) == 3 -> 3 - 2 == 1.
        bits.bit(1); // df_delta_q_present[0]
        bits.f(3, 2); // df_delta_q[0]
        // i == 1: apply == 0 -> outer else: DfDeltaQ[1] = 0 (no bits).
        // i == 2: present, df_delta_q[2] f(2) == 0 -> 0 - 2 == -2.
        bits.bit(1); // df_delta_q_present[2]
        bits.f(0, 2); // df_delta_q[2]
        // i == 3: apply == 0 -> DfDeltaQ[3] = 0.
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_deblocking_filter_params(&mut r, false, 3, 0, false, None).unwrap();
        assert_eq!(params.apply_deblocking_filter, [true, false, true, false]);
        assert_eq!(params.df_delta_q_present, [true, false, true, false]);
        assert_eq!(params.df_delta_q, [1, 0, -2, 0]);
    }

    #[test]
    fn deblocking_inter_reads_allow_df_sub_pu_before_apply() {
        // AV2 § 5.18.5.2 (mirror :5935): on the inter path (read_allow_df_sub_pu == true)
        // allow_df_sub_pu f(1) is read FIRST, then apply_deblocking_filter[0]/[1]. The
        // parsed allow_df_sub_pu drives reconstruction, not the returned struct, so the
        // proof is the bit ALIGNMENT: with allow_df_sub_pu == 1 consumed first, the
        // following apply bits land where expected.
        let mut bits = Bits::default();
        bits.bit(1); // allow_df_sub_pu (consumed, not surfaced)
        bits.bit(1); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        // luma flag set -> chroma pair read.
        bits.bit(0); // apply_deblocking_filter[2]
        bits.bit(0); // apply_deblocking_filter[3]
        // i == 0: present == 0 -> DfDeltaQ[0] = 0.
        bits.bit(0); // df_delta_q_present[0]
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_deblocking_filter_params(&mut r, false, 3, 0, true, None).unwrap();
        assert_eq!(params.apply_deblocking_filter, [true, false, false, false]);
        // 1 (allow_df_sub_pu) + 2 (apply[0]/[1]) + 2 (chroma pair) + 1 (df_delta_q_present[0]).
        assert_eq!(r.consumed_bits(), 6);
    }

    #[test]
    fn deblocking_intra_skips_allow_df_sub_pu() {
        // read_allow_df_sub_pu == false (intra / switch path): no allow_df_sub_pu bit; the
        // first read IS apply_deblocking_filter[0].
        let mut bits = Bits::default();
        bits.bit(1); // apply_deblocking_filter[0]
        bits.bit(0); // apply_deblocking_filter[1]
        bits.bit(0); // apply_deblocking_filter[2]
        bits.bit(0); // apply_deblocking_filter[3]
        bits.bit(0); // df_delta_q_present[0]
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_deblocking_filter_params(&mut r, false, 3, 0, false, None).unwrap();
        assert_eq!(params.apply_deblocking_filter, [true, false, false, false]);
        // No allow_df_sub_pu bit: 2 (apply[0]/[1]) + 2 (chroma pair) + 1 (present[0]).
        assert_eq!(r.consumed_bits(), 5);
    }

    #[test]
    fn deblocking_index_one_absent_delta_inherits_index_zero() {
        // apply[0]==1, apply[1]==1, both present so the i==1 absent-inference path is
        // exercised: present[1]==0 -> DfDeltaQ[1] = DfDeltaQ[0] (the (i==1) ? branch).
        let mut bits = Bits::default();
        bits.bit(1); // apply[0]
        bits.bit(1); // apply[1]
        // monochrome (NumPlanes == 1) -> no chroma pair.
        bits.bit(1); // df_delta_q_present[0]
        bits.f(3, 2); // df_delta_q[0] f(2) == 3 -> 3 - 2 == 1
        bits.bit(0); // df_delta_q_present[1] == 0 -> DfDeltaQ[1] = DfDeltaQ[0] == 1
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_deblocking_filter_params(&mut r, false, 1, 0, false, None).unwrap();
        assert_eq!(params.apply_deblocking_filter, [true, true, false, false]);
        assert_eq!(params.df_delta_q_present, [true, false, false, false]);
        assert_eq!(params.df_delta_q, [1, 1, 0, 0]);
    }

    #[test]
    fn deblocking_monochrome_skips_chroma_pair() {
        // NumPlanes == 1 -> apply[2]/[3] stay 0 even with luma flags set.
        let mut bits = Bits::default();
        bits.bit(1); // apply[0]
        bits.bit(1); // apply[1]
        // no chroma pair (monochrome)
        bits.bit(0); // df_delta_q_present[0]
        bits.bit(0); // df_delta_q_present[1]
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_deblocking_filter_params(&mut r, false, 1, 0, false, None).unwrap();
        assert_eq!(params.apply_deblocking_filter, [true, true, false, false]);
        assert_eq!(params.df_delta_q, [0; 4]);
    }

    #[test]
    fn deblocking_mfh_update_copies_apply_no_bits() {
        // MFH update set: apply comes from the record, only delta-Q present bits read.
        let mfh = MfhDeblockingView {
            mfh_deblocking_filter_update: true,
            mfh_apply_deblocking_filter: [true, false, true, true],
        };
        let mut bits = Bits::default();
        // apply == [true, false, true, false-or-true]: chroma pair copied because
        // apply[0] is set and NumPlanes > 1 -> apply[2]=true, apply[3]=true.
        bits.bit(0); // df_delta_q_present[0]
        // i == 1: apply == 0, DfDeltaQ[1] = DfDeltaQ[0] = 0.
        bits.bit(0); // df_delta_q_present[2]
        bits.bit(0); // df_delta_q_present[3]
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params =
            parse_deblocking_filter_params(&mut r, false, 3, 0, false, Some(&mfh)).unwrap();
        assert_eq!(params.apply_deblocking_filter, [true, false, true, true]);
        assert_eq!(params.df_delta_q, [0; 4]);
        // 3 present bits read, no apply bits.
        assert_eq!(r.consumed_bits(), 3);
    }

    #[test]
    fn deblocking_mfh_update_zero_reads_apply_from_bitstream() {
        // Record present but update == 0 -> apply bits are read from the bitstream.
        let mfh = MfhDeblockingView {
            mfh_deblocking_filter_update: false,
            mfh_apply_deblocking_filter: [true, true, true, true],
        };
        let mut bits = Bits::default();
        bits.bit(0); // apply[0]
        bits.bit(0); // apply[1]
        // both luma flags 0 -> no chroma pair.
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params =
            parse_deblocking_filter_params(&mut r, false, 3, 0, false, Some(&mfh)).unwrap();
        assert_eq!(params.apply_deblocking_filter, [false; 4]);
        assert_eq!(r.consumed_bits(), 2);
    }

    #[test]
    fn deblocking_eof_is_structured_error() {
        let mut r = reader(&[]);
        assert!(matches!(
            parse_deblocking_filter_params(&mut r, false, 3, 0, false, None),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn deblocking_df_par_bits_widens_delta_q_read() {
        // df_par_bits_minus_2 == 3 -> dfParBits = 5, half = 16.
        let mut bits = Bits::default();
        bits.bit(1); // apply[0]
        bits.bit(0); // apply[1]
        // monochrome to keep it simple: NumPlanes == 1.
        bits.bit(1); // df_delta_q_present[0]
        bits.f(20, 5); // df_delta_q[0] == 20 -> 20 - 16 == 4
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_deblocking_filter_params(&mut r, false, 1, 3, false, None).unwrap();
        assert_eq!(params.df_delta_q[0], 4);
    }

    #[test]
    fn deblocking_oversized_df_par_bits_is_structured_error_not_panic() {
        // The in-tree sequence parser reads df_par_bits_minus_2 as f(2) (0..=3), but a
        // direct/fuzz caller can construct a CoreSeqFilterView (or pass the field) outside
        // that range. dfParBits = df_par_bits_minus_2 + 2 then exceeds the 32-bit
        // `df_delta_q[i]` read width, which the workspace's no-reachable-panic rule requires
        // be rejected with a structured BitWidthTooLarge rather than letting `read_bits`
        // attempt an impossible width. A buffer of 0xFF bytes large enough to satisfy the
        // apply reads keeps the failure the width, not EOF. df_par_bits_minus_2 == 31 ->
        // dfParBits = 33 (> 32) is the first value the read width rejects; everything larger
        // does too. (The in-range-but-wide boundary dfParBits == 32 is covered separately,
        // where the i64 DfDeltaQ derivation keeps the shift/subtraction panic-free.)
        let data = [0xFFu8; 16];
        for df_par_bits_minus_2 in [31u8, 32, 200, u8::MAX] {
            let mut r = reader(&data);
            let result =
                parse_deblocking_filter_params(&mut r, false, 1, df_par_bits_minus_2, false, None);
            assert!(
                matches!(result, Err(Error::BitWidthTooLarge { .. })),
                "df_par_bits_minus_2 == {df_par_bits_minus_2} must yield BitWidthTooLarge, got {result:?}"
            );
        }
    }

    #[test]
    fn deblocking_max_width_df_par_bits_reads_without_overflow_panic() {
        // df_par_bits_minus_2 == 30 -> dfParBits = 32: the read width is in range (<= 32),
        // but the `1 << (dfParBits - 1)` half (2^31) and the `df_delta_q[i] - half`
        // subtraction would overflow i32 and panic in debug if computed in i32. They are
        // computed in i64 and narrowed (the result always fits i32), so a constructed view
        // at the boundary returns Ok with a bounded value rather than panicking. apply[0] set
        // + present so the f(32) read and the subtraction both fire.
        let mut bits = Bits::default();
        bits.bit(1); // apply[0]
        bits.bit(0); // apply[1]
        // monochrome (NumPlanes == 1) -> no chroma pair.
        bits.bit(1); // df_delta_q_present[0]
        bits.f(1, 32); // df_delta_q[0] == 1 -> 1 - 2^31 == i32::MIN + 1
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_deblocking_filter_params(&mut r, false, 1, 30, false, None).unwrap();
        assert_eq!(params.df_delta_q[0], (1i64 - (1i64 << 31)) as i32);
    }

    // ---- interpolation filter (§ 5.18.5.1) ----

    #[test]
    fn interpolation_filter_switchable_reads_one_bit() {
        // is_filter_switchable == 1 -> SWITCHABLE (no interpolation_filter f(2)).
        let mut bits = Bits::default();
        bits.bit(1); // is_filter_switchable
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let filter = read_interpolation_filter(&mut r).unwrap();
        assert_eq!(filter, InterpolationFilter::Switchable);
        assert_eq!(r.consumed_bits(), 1);
    }

    #[test]
    fn interpolation_filter_fixed_reads_two_bit_code() {
        // is_filter_switchable == 0 -> interpolation_filter f(2). Each code maps to its
        // fixed filter.
        for (code, expected) in [
            (0u32, InterpolationFilter::Eighttap),
            (1, InterpolationFilter::EighttapSmooth),
            (2, InterpolationFilter::EighttapSharp),
            (3, InterpolationFilter::Bilinear),
        ] {
            let mut bits = Bits::default();
            bits.bit(0); // is_filter_switchable
            bits.f(code, 2); // interpolation_filter
            let data = bits.into_bytes();
            let mut r = reader(&data);
            let filter = read_interpolation_filter(&mut r).unwrap();
            assert_eq!(filter, expected, "code {code}");
            assert_eq!(r.consumed_bits(), 3);
        }
    }

    #[test]
    fn interpolation_filter_eof_is_structured_error() {
        // No bits -> EOF before is_filter_switchable.
        let data: [u8; 0] = [];
        let mut r = reader(&data);
        assert!(matches!(
            read_interpolation_filter(&mut r),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    // ---- gdf ----

    #[test]
    fn gdf_coded_lossless_disables_no_bits() {
        let mut r = reader(&[]);
        let params = parse_gdf_params(&mut r, true, &base_filter(), base_geometry()).unwrap();
        assert!(!params.gdf_frame_enable);
        assert_eq!(r.consumed_bits(), 0);
    }

    #[test]
    fn gdf_disabled_seq_flag_no_bits() {
        let mut filter = base_filter();
        filter.enable_gdf = false;
        let mut r = reader(&[]);
        let params = parse_gdf_params(&mut r, false, &filter, base_geometry()).unwrap();
        assert!(!params.gdf_frame_enable);
        assert_eq!(r.consumed_bits(), 0);
    }

    #[test]
    fn gdf_single_picture_infers_enable() {
        let mut filter = base_filter();
        filter.single_picture_header_flag = true;
        // gdf_frame_enable inferred 1 (no bit). Frame exceeds block -> gdf_per_block f(1).
        let mut bits = Bits::default();
        bits.bit(1); // gdf_per_block
        bits.f(2, 2); // gdf_pic_qc_idx
        bits.f(1, 2); // gdf_pic_scale_idx
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_gdf_params(&mut r, false, &filter, base_geometry()).unwrap();
        assert!(params.gdf_frame_enable);
        assert_eq!(params.gdf_per_block, Some(true));
        assert_eq!(params.gdf_pic_qc_idx, Some(2));
        assert_eq!(params.gdf_pic_scale_idx, Some(1));
    }

    #[test]
    fn gdf_frame_enable_false_returns_early() {
        // Not single picture, read gdf_frame_enable == 0 -> return.
        let mut bits = Bits::default();
        bits.bit(0); // gdf_frame_enable
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_gdf_params(&mut r, false, &base_filter(), base_geometry()).unwrap();
        assert!(!params.gdf_frame_enable);
        assert_eq!(params.gdf_per_block, None);
        assert_eq!(r.consumed_bits(), 1);
    }

    #[test]
    fn gdf_small_single_tile_infers_per_block_zero() {
        // A frame at or below gdfBlkSize in a single tile -> gdf_per_block inferred 0.
        // SbSize 128x128 -> gdfBlkSize = Max(128, 128) = 128. MiCols*4 == 128, so the
        // frame does not exceed the block; single tile, loopfilters allowed across.
        let geom = GdfGeometry {
            sb_size: SuperblockSize::Block128x128,
            mi_cols: 32, // 32 * 4 == 128, not > 128
            mi_rows: 32,
            tile_cols: 1,
            tile_rows: 1,
            mi_col_starts: &[0],
            mi_row_starts: &[0],
        };
        let mut bits = Bits::default();
        bits.bit(1); // gdf_frame_enable (read; not single picture)
        // no gdf_per_block bit (inferred 0)
        bits.f(0, 2); // gdf_pic_qc_idx
        bits.f(0, 2); // gdf_pic_scale_idx
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_gdf_params(&mut r, false, &base_filter(), geom).unwrap();
        assert_eq!(params.gdf_per_block, Some(false));
        // 1 enable + 4 qc/scale, no per-block bit.
        assert_eq!(r.consumed_bits(), 5);
    }

    #[test]
    fn gdf_eof_is_structured_error() {
        let mut r = reader(&[]);
        // not single picture -> must read gdf_frame_enable, but no data.
        assert!(matches!(
            parse_gdf_params(&mut r, false, &base_filter(), base_geometry()),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    // ---- cdef ----

    #[test]
    fn cdef_coded_lossless_disables_no_bits() {
        let mut r = reader(&[]);
        let params = parse_cdef_params(&mut r, true, 3, &base_filter()).unwrap();
        assert!(!params.cdef_frame_enable);
        assert_eq!(r.consumed_bits(), 0);
    }

    #[test]
    fn cdef_disabled_seq_flag_no_bits() {
        let mut filter = base_filter();
        filter.enable_cdef = false;
        let mut r = reader(&[]);
        let params = parse_cdef_params(&mut r, false, 3, &filter).unwrap();
        assert!(!params.cdef_frame_enable);
        assert_eq!(r.consumed_bits(), 0);
    }

    #[test]
    fn cdef_reads_strength_sets() {
        // Not single picture; cdef_frame_enable read; 2 strengths; adaptive skip-txfm.
        let mut bits = Bits::default();
        bits.bit(1); // cdef_frame_enable
        bits.f(1, 2); // cdef_damping_minus_3 -> CdefDamping = 4
        bits.f(1, 3); // cdef_strengths_minus_1 -> CdefStrengths = 2
        bits.bit(1); // cdef_on_skip_txfm_frame_enable (adaptive -> read)
        // strength 0
        bits.bit(0); // cdef_y_pri_zero == 0 -> read f(4)
        bits.f(9, 4); // cdef_y_pri_strength[0]
        bits.f(3, 2); // cdef_y_sec_strength[0] == 3 -> 4
        bits.bit(1); // cdef_uv_pri_zero == 1 -> 0
        bits.f(2, 2); // cdef_uv_sec_strength[0]
        // strength 1
        bits.bit(1); // cdef_y_pri_zero == 1 -> 0
        bits.f(1, 2); // cdef_y_sec_strength[1]
        bits.bit(0); // cdef_uv_pri_zero == 0 -> read f(4)
        bits.f(5, 4); // cdef_uv_pri_strength[1]
        bits.f(3, 2); // cdef_uv_sec_strength[1] == 3 -> 4
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_cdef_params(&mut r, false, 3, &base_filter()).unwrap();
        assert!(params.cdef_frame_enable);
        assert_eq!(params.cdef_damping, Some(4));
        assert_eq!(params.cdef_strengths, Some(2));
        assert_eq!(params.cdef_on_skip_txfm_frame_enable, Some(true));
        assert_eq!(params.strengths.len(), 2);
        assert_eq!(params.strengths[0].y_pri_strength, 9);
        assert_eq!(params.strengths[0].y_sec_strength, 4); // 3 -> 4
        assert_eq!(params.strengths[0].uv_pri_strength, 0);
        assert_eq!(params.strengths[0].uv_sec_strength, 2);
        assert_eq!(params.strengths[1].y_pri_strength, 0);
        assert_eq!(params.strengths[1].y_sec_strength, 1);
        assert_eq!(params.strengths[1].uv_pri_strength, 5);
        assert_eq!(params.strengths[1].uv_sec_strength, 4); // 3 -> 4
    }

    #[test]
    fn cdef_monochrome_skips_uv_reads() {
        let mut bits = Bits::default();
        bits.bit(1); // cdef_frame_enable
        bits.f(0, 2); // cdef_damping_minus_3
        bits.f(0, 3); // cdef_strengths_minus_1 -> 1 strength
        bits.bit(1); // cdef_on_skip_txfm_frame_enable
        bits.bit(0); // cdef_y_pri_zero
        bits.f(7, 4); // cdef_y_pri_strength[0]
        bits.f(1, 2); // cdef_y_sec_strength[0]
        // no UV reads (monochrome)
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_cdef_params(&mut r, false, 1, &base_filter()).unwrap();
        assert_eq!(params.strengths.len(), 1);
        assert_eq!(params.strengths[0].y_pri_strength, 7);
        assert_eq!(params.strengths[0].uv_pri_strength, 0);
        assert_eq!(params.strengths[0].uv_sec_strength, 0);
    }

    #[test]
    fn cdef_skip_txfm_always_on_and_disabled_infer_no_bit() {
        // always-on -> cdef_on_skip_txfm_frame_enable = 1 with no bit.
        let mut filter = base_filter();
        filter.cdef_on_skip_txfm = CdefOnSkipTxfm::AlwaysOn;
        let mut bits = Bits::default();
        bits.bit(1); // cdef_frame_enable
        bits.f(0, 2); // damping
        bits.f(0, 3); // strengths -> 1
        // no skip-txfm bit
        bits.bit(1); // cdef_y_pri_zero
        bits.f(0, 2); // cdef_y_sec_strength
        bits.bit(1); // cdef_uv_pri_zero
        bits.f(0, 2); // cdef_uv_sec_strength
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_cdef_params(&mut r, false, 3, &filter).unwrap();
        assert_eq!(params.cdef_on_skip_txfm_frame_enable, Some(true));

        filter.cdef_on_skip_txfm = CdefOnSkipTxfm::Disabled;
        let mut bits = Bits::default();
        bits.bit(1); // cdef_frame_enable
        bits.f(0, 2); // damping
        bits.f(0, 3); // strengths -> 1
        bits.bit(1); // cdef_y_pri_zero
        bits.f(0, 2); // cdef_y_sec_strength
        bits.bit(1); // cdef_uv_pri_zero
        bits.f(0, 2); // cdef_uv_sec_strength
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_cdef_params(&mut r, false, 3, &filter).unwrap();
        assert_eq!(params.cdef_on_skip_txfm_frame_enable, Some(false));
    }

    #[test]
    fn cdef_single_picture_infers_frame_enable() {
        let mut filter = base_filter();
        filter.single_picture_header_flag = true;
        let mut bits = Bits::default();
        // no cdef_frame_enable bit (inferred 1)
        bits.f(0, 2); // damping
        bits.f(0, 3); // strengths -> 1
        bits.bit(1); // cdef_on_skip_txfm (adaptive -> read)
        bits.bit(1); // cdef_y_pri_zero
        bits.f(0, 2); // cdef_y_sec_strength
        bits.bit(1); // cdef_uv_pri_zero
        bits.f(0, 2); // cdef_uv_sec_strength
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_cdef_params(&mut r, false, 3, &filter).unwrap();
        assert!(params.cdef_frame_enable);
        assert_eq!(params.cdef_strengths, Some(1));
    }

    #[test]
    fn cdef_eof_is_structured_error() {
        let mut r = reader(&[]);
        assert!(matches!(
            parse_cdef_params(&mut r, false, 3, &base_filter()),
            Err(Error::UnexpectedEof { .. })
        ));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::headers::sequence::CdefOnSkipTxfm;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    fn arbitrary_skip_txfm() -> impl Strategy<Value = CdefOnSkipTxfm> {
        prop_oneof![
            Just(CdefOnSkipTxfm::Adaptive),
            Just(CdefOnSkipTxfm::AlwaysOn),
            Just(CdefOnSkipTxfm::Disabled),
        ]
    }

    fn arbitrary_sb_size() -> impl Strategy<Value = SuperblockSize> {
        prop_oneof![
            Just(SuperblockSize::Block64x64),
            Just(SuperblockSize::Block128x128),
            Just(SuperblockSize::Block256x256),
        ]
    }

    proptest! {
        /// The deblocking parser must never panic on arbitrary input and state.
        #[test]
        fn parse_deblocking_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..32),
            coded_lossless in any::<bool>(),
            num_planes in prop_oneof![Just(1u8), Just(3u8)],
            // Widened to the full u8 range: a direct/fuzz caller is not bound by the
            // sequence parser's f(2) read, so df_par_bits_minus_2 outside 0..=3 must not
            // panic (it returns a structured BitWidthTooLarge instead).
            df_par_bits_minus_2 in any::<u8>(),
            // The inter-path allow_df_sub_pu gate (enable_df_sub_pu && INTER): both arms
            // must stay panic-free.
            read_allow_df_sub_pu in any::<bool>(),
            mfh in proptest::option::of((any::<bool>(), any::<[bool; 4]>())),
        ) {
            let view = mfh.map(|(update, apply)| MfhDeblockingView {
                mfh_deblocking_filter_update: update,
                mfh_apply_deblocking_filter: apply,
            });
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_deblocking_filter_params(
                &mut reader,
                coded_lossless,
                num_planes,
                df_par_bits_minus_2,
                read_allow_df_sub_pu,
                view.as_ref(),
            );
        }

        /// The GDF parser must never panic on arbitrary input, state, and geometry.
        #[test]
        fn parse_gdf_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..32),
            coded_lossless in any::<bool>(),
            enable_gdf in any::<bool>(),
            gdf_unit_matches_sb_size in any::<bool>(),
            disable_loopfilters_across_tiles in any::<bool>(),
            single_picture_header_flag in any::<bool>(),
            sb_size in arbitrary_sb_size(),
            mi_cols in 0u32..=65536,
            mi_rows in 0u32..=65536,
            tile_cols in 1u32..=64,
            tile_rows in 1u32..=64,
            col_starts in proptest::collection::vec(0u32..=65536, 0..=64),
            row_starts in proptest::collection::vec(0u32..=65536, 0..=64),
        ) {
            let filter = CoreSeqFilterView {
                enable_cdef: true,
                enable_gdf,
                gdf_unit_matches_sb_size,
                disable_loopfilters_across_tiles,
                cdef_on_skip_txfm: CdefOnSkipTxfm::Adaptive,
                df_par_bits_minus_2: 0,
                enable_df_sub_pu: false,
                single_picture_header_flag,
            };
            let geometry = GdfGeometry {
                sb_size,
                mi_cols,
                mi_rows,
                tile_cols,
                tile_rows,
                mi_col_starts: &col_starts,
                mi_row_starts: &row_starts,
            };
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_gdf_params(&mut reader, coded_lossless, &filter, geometry);
        }

        /// The CDEF parser must never panic on arbitrary input and state.
        #[test]
        fn parse_cdef_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..48),
            coded_lossless in any::<bool>(),
            enable_cdef in any::<bool>(),
            num_planes in prop_oneof![Just(1u8), Just(3u8)],
            single_picture_header_flag in any::<bool>(),
            cdef_on_skip_txfm in arbitrary_skip_txfm(),
        ) {
            let filter = CoreSeqFilterView {
                enable_cdef,
                enable_gdf: true,
                gdf_unit_matches_sb_size: false,
                disable_loopfilters_across_tiles: false,
                cdef_on_skip_txfm,
                df_par_bits_minus_2: 0,
                enable_df_sub_pu: false,
                single_picture_header_flag,
            };
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_cdef_params(&mut reader, coded_lossless, num_planes, &filter);
        }
    }
}
