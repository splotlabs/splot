// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame-header tile info (AV2 v1.0.0 § 5.18.7.2,
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-2`).
//!
//! Models `tile_info()` on the intra path: `MiCols` / `MiRows` are derived from
//! the parsed frame dimensions per `compute_image_size()` (AV2 § 5.18.4.4,
//! `#s-5-18-4-4`), the `reuse_tile_info` eligibility condition is evaluated
//! against the sequence tile layout (including `uniform_eligible()`), and the
//! explicit path reuses the § 5.18.7.3 `tile_params()` helper in
//! [`crate::tile`]. When the frame size is unknown (`cur_mfh_id > 0` default
//! dimensions come from the multi-frame header, which is not modeled), the
//! caller stops with an explicit partial status before reaching this parser.

use crate::bitio::BitReader;
use crate::error::{Error, Result};
use crate::headers::sequence::{
    LevelIdx, SequenceHeaderGeneral, SequencePartitionConfig, SequenceTileConfig,
    SequenceTqEntropyConfig, SuperblockSize, Tier,
};
use crate::tile::{
    ReuseTileParamsInput, TileParams, TileParamsInput, mi_width_log2, num_4x4_blocks_wide,
    parse_tile_layout, reuse_tile_params,
};

use super::size::FrameSize;

/// Sequence-derived inputs for `tile_info()` (AV2 v1.0.0 § 5.18.7.2), gathered
/// from the parsed general header, `sequence_partition_config()` (AV2 § 5.4.3),
/// `sequence_transform_quant_entropy_config()` (AV2 § 5.4.8), and
/// `sequence_tile_config()` (AV2 § 5.4.2).
///
/// The sequence tile layout (`SeqUniformTileSpacingFlag`, `SeqTileColsLog2`,
/// `SeqTileRowsLog2`, `SeqTileCols`, `SeqTileRows`, `SeqSbCols`, `SeqSbRows`) is
/// carried as the stored [`TileParams`] in [`Self::seq_tile_params`], and the
/// `SeqSbColStarts` / `SeqSbRowStarts` start arrays (recorded by § 5.4.2 parsing,
/// needed by the non-uniform `reuse_tile_params()` branch of § 5.18.7.4) are carried
/// in [`Self::seq_sb_col_starts`] / [`Self::seq_sb_row_starts`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSeqTileView {
    /// `seq_tile_info_present_flag` (AV2 § 5.4.2): `haveTileParams` for non-bridge
    /// frames (§ 5.18.7.2).
    pub seq_tile_info_present_flag: bool,
    /// `allow_tile_info_change` (AV2 § 5.4.2), gating the `reuse_tile_info` read
    /// (§ 5.18.7.2). Only consulted when tile info is signalled (`false` here when
    /// not signalled).
    pub allow_tile_info_change: bool,
    /// The sequence `tile_params()` layout (AV2 § 5.4.2 / § 5.18.7.3):
    /// `uniform_spacing` is `SeqUniformTileSpacingFlag`, `tile_cols_log2` /
    /// `tile_rows_log2` are `SeqTileColsLog2` / `SeqTileRowsLog2`, `sb_cols` /
    /// `sb_rows` are `SeqSbCols` / `SeqSbRows`. `None` when tile info is not
    /// signalled, or when `seq_level_idx` is a reserved level with no defined tile
    /// bit layout (a non-conformant stream).
    pub seq_tile_params: Option<TileParams>,
    /// `SeqSbColStarts[0..SeqTileCols]` (AV2 § 5.4.2), needed by the non-uniform
    /// `reuse_tile_params()` branch (§ 5.18.7.4); empty unless a non-reserved sequence
    /// tile layout is present. Bounded by `MAX_TILE_COLS`.
    pub seq_sb_col_starts: std::sync::Arc<[u32]>,
    /// `SeqSbRowStarts[0..SeqTileRows]` (AV2 § 5.4.2), the row companion of
    /// [`Self::seq_sb_col_starts`]. Bounded by `MAX_TILE_ROWS`.
    pub seq_sb_row_starts: std::sync::Arc<[u32]>,
    /// `get_seq_sb_size()` (AV2 § 5.18.7.6): the `seqSbSize` argument of
    /// `tile_params()` / `reuse_tile_params()` (§ 5.18.7.2).
    pub seq_sb_size: SuperblockSize,
    /// `use_256x256_superblock` (AV2 § 5.4.3), used by the frame `SbSize`
    /// derivation (§ 5.18.2; see [`Self::frame_sb_size`]).
    pub use_256x256_superblock: bool,
    /// `use_128x128_superblock` (AV2 § 5.4.3), used by the frame `SbSize`
    /// derivation (§ 5.18.2; see [`Self::frame_sb_size`]).
    pub use_128x128_superblock: bool,
    /// `enable_avg_cdf` (AV2 § 5.4.8), gating the `context_update_tile_id` read
    /// (§ 5.18.7.2).
    pub enable_avg_cdf: bool,
    /// `avg_cdf_type` (AV2 § 5.4.8), gating the `context_update_tile_id` read
    /// (§ 5.18.7.2).
    pub avg_cdf_type: u8,
    /// `seq_tier` (AV2 § 5.4.1), input to the § 5.18.7.3 level/tier scaling tables.
    pub seq_tier: Tier,
    /// `seq_level_idx` (AV2 § 5.4.1), input to the § 5.18.7.3 level/tier scaling
    /// tables.
    pub seq_level_idx: LevelIdx,
}

impl CoreSeqTileView {
    /// Builds the tile view from the parsed general header, partition config,
    /// transform/quant/entropy config, and tile config (AV2 v1.0.0 § 5.4.1 /
    /// § 5.4.3 / § 5.4.8 / § 5.4.2).
    #[must_use]
    pub(crate) fn from_sequence_configs(
        general: &SequenceHeaderGeneral,
        partition: &SequencePartitionConfig,
        tq: &SequenceTqEntropyConfig,
        tile: &SequenceTileConfig,
    ) -> Self {
        Self {
            seq_tile_info_present_flag: tile.seq_tile_info_present_flag,
            allow_tile_info_change: tile.allow_tile_info_change.unwrap_or(false),
            seq_tile_params: tile.params,
            // Shared, not copied: the sequence's tile starts do not change
            // between frames, and this view is rebuilt for every one.
            seq_sb_col_starts: std::sync::Arc::clone(&tile.seq_sb_col_starts),
            seq_sb_row_starts: std::sync::Arc::clone(&tile.seq_sb_row_starts),
            seq_sb_size: partition.seq_sb_size(),
            use_256x256_superblock: partition.use_256x256_superblock,
            use_128x128_superblock: partition.use_128x128_superblock,
            enable_avg_cdf: tq.enable_avg_cdf,
            avg_cdf_type: tq.avg_cdf_type,
            seq_tier: general.seq_tier,
            seq_level_idx: general.seq_level_idx,
        }
    }

    /// Derives the frame `SbSize` (AV2 v1.0.0 § 5.18.2): with 256×256 superblocks
    /// it is `BLOCK_128X128` for an intra frame and `BLOCK_256X256` otherwise;
    /// with 128×128 superblocks `BLOCK_128X128`; else `BLOCK_64X64`.
    #[must_use]
    pub const fn frame_sb_size(&self, frame_is_intra: bool) -> SuperblockSize {
        if self.use_256x256_superblock {
            if frame_is_intra {
                SuperblockSize::Block128x128
            } else {
                SuperblockSize::Block256x256
            }
        } else if self.use_128x128_superblock {
            SuperblockSize::Block128x128
        } else {
            SuperblockSize::Block64x64
        }
    }
}

/// Parsed `tile_info()` (AV2 v1.0.0 § 5.18.7.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-2`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TileInfo {
    /// `reuse_tile_info` (read when eligible and `allow_tile_info_change`, else
    /// inferred per § 5.18.7.2).
    pub reuse_tile_info: bool,
    /// `TileCols`.
    pub tile_cols: u32,
    /// `TileRows`.
    pub tile_rows: u32,
    /// `TileColsLog2`.
    pub tile_cols_log2: u8,
    /// `TileRowsLog2`.
    pub tile_rows_log2: u8,
    /// `MiColStarts[0..=TileCols]` (`sbColStarts[i] << sbShift2`, with
    /// `MiColStarts[TileCols] = MiCols`).
    pub mi_col_starts: crate::tile::TileStarts,
    /// `MiRowStarts[0..=TileRows]` (`sbRowStarts[i] << sbShift2`, with
    /// `MiRowStarts[TileRows] = MiRows`).
    pub mi_row_starts: crate::tile::TileStarts,
    /// `context_update_tile_id` (read only for a multi-tile, non-bridge,
    /// non-TIP-as-output layout when `!enable_avg_cdf || !avg_cdf_type`; else `0`).
    pub context_update_tile_id: u32,
    /// `TileSizeBytes = tile_size_bytes_minus_1 + 1`, present only when the
    /// multi-tile condition of § 5.18.7.2 read it.
    pub tile_size_bytes: Option<u32>,
    /// The full [`TileParams`] the explicit `tile_params()` branch (§ 5.18.7.3,
    /// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-3`) derived, surfaced for
    /// the writer (the byte-exact inverse re-runs that derivation, which needs the
    /// `uniform_spacing` flag and the superblock grid that the other [`TileInfo`] fields
    /// discard). `Some` on the explicit branch; `None` on the reuse branch (which writes
    /// no `tile_params()` bits — the layout is recomputed from the stored sequence layout).
    pub tile_params: Option<TileParams>,
}

/// More tile starts than AV2 allows for one axis.
fn tile_starts_overflow(reader: &BitReader<'_>) -> Error {
    Error::InvalidTileParams {
        offset: reader.byte_offset(),
        bit_offset: reader.bit_offset(),
        kind: crate::error::TileParamsErrorKind::TileColsOutOfRange,
    }
}

/// Parses `tile_info()` (AV2 v1.0.0 § 5.18.7.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-2`).
///
/// `frame_size` is the parsed `FrameWidth` / `FrameHeight` (`MiCols` / `MiRows`
/// are derived per § 5.18.4.4); `frame_is_intra` selects the frame `SbSize`
/// (§ 5.18.2); `is_bridge` forces `haveTileParams = 0`; `tip_frame_as_output` is
/// `TipFrameMode == TIP_FRAME_AS_OUTPUT` (always `false` on the intra path) and
/// gates the trailing `context_update_tile_id` / `tile_size_bytes_minus_1` reads.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) or a typed
/// descriptor error if the payload ends or is malformed mid-field, and
/// [`Error::InvalidTileParams`](crate::error::Error::InvalidTileParams) when an explicit
/// non-uniform layout exceeds the § 6.17.7.2 tile-count limits. Returns
/// [`Error::Unimplemented`](crate::error::Error::Unimplemented) when the layout
/// depends on unmodeled state (a reserved `seq_level_idx` leaves
/// [`CoreSeqTileView::seq_tile_params`] `None` so the reuse eligibility cannot be
/// evaluated).
pub fn parse_tile_info(
    reader: &mut BitReader<'_>,
    tile: &CoreSeqTileView,
    frame_size: FrameSize,
    frame_is_intra: bool,
    is_bridge: bool,
    tip_frame_as_output: bool,
) -> Result<TileInfo> {
    let sb_size = tile.frame_sb_size(frame_is_intra);
    let sb4x4 = num_4x4_blocks_wide(sb_size);
    let sb_shift = mi_width_log2(sb_size);
    let mi_cols = 2 * (frame_size.width.saturating_add(7) >> 3);
    let mi_rows = 2 * (frame_size.height.saturating_add(7) >> 3);
    let sb_cols = mi_cols.saturating_add(sb4x4 - 1) >> sb_shift;
    let sb_rows = mi_rows.saturating_add(sb4x4 - 1) >> sb_shift;

    let have_tile_params = !is_bridge && tile.seq_tile_info_present_flag;

    let seq = tile.seq_tile_params.as_ref();

    let eligible = match (have_tile_params, seq) {
        (false, _) => false,
        (true, None) => {
            return Err(Error::Unimplemented {
                feature: "AV2-5.18.7-SEGMENTATION-TILING",
            });
        }
        (true, Some(seq)) => {
            if seq.uniform_spacing {
                uniform_eligible(seq.tile_rows_log2, sb_rows)
                    && uniform_eligible(seq.tile_cols_log2, sb_cols)
            } else {
                seq.sb_cols == sb_cols && seq.sb_rows == sb_rows
            }
        }
    };

    let reuse_tile_info = if eligible {
        if tile.allow_tile_info_change {
            reader.read_flag()?
        } else {
            true
        }
    } else {
        false
    };

    let seq_sb_size = tile.seq_sb_size;

    let mut explicit_tile_params: Option<TileParams> = None;
    let (
        sb_col_starts,
        sb_row_starts,
        tile_cols,
        tile_rows,
        tile_cols_log2,
        tile_rows_log2,
        sb_shift2,
    ) = if reuse_tile_info {
        let seq = seq.ok_or(Error::Unimplemented {
            feature: "AV2-5.18.7-SEGMENTATION-TILING",
        })?;
        let (seq_sb_row_starts, seq_sb_col_starts): (&[u32], &[u32]) = if seq.uniform_spacing {
            (&[], &[])
        } else {
            (&tile.seq_sb_row_starts, &tile.seq_sb_col_starts)
        };
        let reused = reuse_tile_params(ReuseTileParamsInput {
            uniform_spacing: seq.uniform_spacing,
            seq_sb_row_starts,
            seq_tile_rows: seq.tile_rows,
            seq_tile_rows_log2: seq.tile_rows_log2,
            seq_sb_col_starts,
            seq_tile_cols: seq.tile_cols,
            seq_tile_cols_log2: seq.tile_cols_log2,
            seq_sb_size,
            sb_size,
            mi_cols,
            mi_rows,
        });
        (
            reused.sb_col_starts,
            reused.sb_row_starts,
            reused.tile_cols,
            reused.tile_rows,
            reused.tile_cols_log2,
            reused.tile_rows_log2,
            reused.sb_shift2,
        )
    } else {
        let layout = parse_tile_layout(
            reader,
            TileParamsInput {
                frame_width: frame_size.width,
                frame_height: frame_size.height,
                uniform_sb_size: seq_sb_size,
                sb_size,
                is_bridge,
                seq_tier: tile.seq_tier,
                seq_level_idx: tile.seq_level_idx,
            },
        )?;
        explicit_tile_params = Some(layout.params);
        (
            layout.sb_col_starts,
            layout.sb_row_starts,
            layout.params.tile_cols,
            layout.params.tile_rows,
            layout.params.tile_cols_log2,
            layout.params.tile_rows_log2,
            layout.sb_shift2,
        )
    };

    let sb_shift2 = sb_shift2.min(31);
    let mut mi_col_starts = crate::tile::TileStarts::default();
    for &start in sb_col_starts.iter() {
        mi_col_starts
            .push(start << sb_shift2)
            .ok_or_else(|| tile_starts_overflow(reader))?;
    }
    mi_col_starts
        .push(mi_cols)
        .ok_or_else(|| tile_starts_overflow(reader))?;
    let mut mi_row_starts = crate::tile::TileStarts::default();
    for &start in sb_row_starts.iter() {
        mi_row_starts
            .push(start << sb_shift2)
            .ok_or_else(|| tile_starts_overflow(reader))?;
    }
    mi_row_starts
        .push(mi_rows)
        .ok_or_else(|| tile_starts_overflow(reader))?;

    let (context_update_tile_id, tile_size_bytes) =
        if (tile_cols > 1 || tile_rows > 1) && !is_bridge && !tip_frame_as_output {
            let context_update_tile_id = if !tile.enable_avg_cdf || tile.avg_cdf_type == 0 {
                reader.read_bits(u32::from(tile_rows_log2) + u32::from(tile_cols_log2))?
            } else {
                0
            };
            let tile_size_bytes_minus_1 = reader.read_bits(2)?;
            (context_update_tile_id, Some(tile_size_bytes_minus_1 + 1))
        } else {
            (0, None)
        };

    Ok(TileInfo {
        reuse_tile_info,
        tile_cols,
        tile_rows,
        tile_cols_log2,
        tile_rows_log2,
        mi_col_starts,
        mi_row_starts,
        context_update_tile_id,
        tile_size_bytes,
        tile_params: explicit_tile_params,
    })
}

/// `uniform_eligible(tileLog2, sbNum)` (AV2 v1.0.0 § 5.18.7.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-2`): whether splitting
/// `sbNum` superblocks into `1 << tileLog2` uniform tiles leaves every tile (in
/// particular the last) at least one superblock wide.
///
/// `lastTileWidth = sbNum - (tileNum - 1) * tileWidth` can be negative, so the
/// arithmetic uses `i128` (the spec works on unbounded integers).
fn uniform_eligible(tile_log2: u8, sb_num: u32) -> bool {
    let tile_log2 = u32::from(tile_log2).min(64);
    let tile_num: i128 = 1 << tile_log2;
    let tile_width = (i128::from(sb_num) + tile_num - 1) >> tile_log2;
    let last_tile_width = i128::from(sb_num) - (tile_num - 1) * tile_width;
    tile_width >= 1 && last_tile_width >= 1
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::span::ByteOffset;
    use crate::test_support::base_view;

    use crate::test_bits::Bits;

    /// A stored uniform 2x2 sequence tile layout for a 4x4-superblock frame.
    fn uniform_2x2_seq_params() -> TileParams {
        TileParams {
            tile_cols: 2,
            tile_rows: 2,
            tile_cols_log2: 1,
            tile_rows_log2: 1,
            sb_cols: 4,
            sb_rows: 4,
            uniform_spacing: true,
            covers_cols: true,
            covers_rows: true,
        }
    }

    fn parse(view: &CoreSeqTileView, data: &[u8], frame: FrameSize) -> Result<TileInfo> {
        let mut reader = BitReader::new(data, ByteOffset::new(0));
        parse_tile_info(&mut reader, view, frame, true, false, false)
    }

    #[test]
    fn single_tile_reads_only_uniform_flag_and_no_context_fields() {
        let mut bits = Bits::default();
        bits.bit(1); // uniform_tile_spacing_flag
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let info = parse_tile_info(
            &mut reader,
            &base_view(),
            FrameSize::new(16, 8),
            true,
            false,
            false,
        )
        .unwrap();
        assert!(!info.reuse_tile_info);
        assert_eq!(info.tile_cols, 1);
        assert_eq!(info.tile_rows, 1);
        assert_eq!(info.tile_cols_log2, 0);
        assert_eq!(info.tile_rows_log2, 0);
        assert_eq!(info.mi_col_starts.as_ref(), [0, 4].as_slice());
        assert_eq!(info.mi_row_starts.as_ref(), [0, 2].as_slice());
        assert_eq!(info.context_update_tile_id, 0);
        assert_eq!(info.tile_size_bytes, None);
        assert_eq!(reader.consumed_bits(), 1);
    }

    #[test]
    fn explicit_multi_tile_reads_context_update_and_tile_size_bytes() {
        let mut bits = Bits::default();
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(1); // increment_tile_cols_log2 = 1
        bits.bit(0); // increment_tile_cols_log2 = 0
        bits.bit(0); // increment_tile_rows_log2 = 0
        bits.f(1, 1); // context_update_tile_id
        bits.f(3, 2); // tile_size_bytes_minus_1 -> TileSizeBytes = 4
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let info = parse_tile_info(
            &mut reader,
            &base_view(),
            FrameSize::new(256, 256),
            true,
            false,
            false,
        )
        .unwrap();
        assert!(!info.reuse_tile_info);
        assert_eq!(info.tile_cols, 2);
        assert_eq!(info.tile_rows, 1);
        assert_eq!(info.tile_cols_log2, 1);
        assert_eq!(info.tile_rows_log2, 0);
        assert_eq!(info.mi_col_starts.as_ref(), [0, 32, 64].as_slice());
        assert_eq!(info.mi_row_starts.as_ref(), [0, 64].as_slice());
        assert_eq!(info.context_update_tile_id, 1);
        assert_eq!(info.tile_size_bytes, Some(4));
        assert_eq!(reader.consumed_bits(), 7);
    }

    #[test]
    fn reuse_path_recomputes_uniform_layout_at_frame_size() {
        let mut view = base_view();
        view.seq_tile_info_present_flag = true;
        view.allow_tile_info_change = true;
        view.seq_tile_params = Some(uniform_2x2_seq_params());
        let mut bits = Bits::default();
        bits.bit(1); // reuse_tile_info
        bits.f(2, 2); // context_update_tile_id (n = 1 + 1)
        bits.f(1, 2); // tile_size_bytes_minus_1 -> TileSizeBytes = 2
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let info = parse_tile_info(
            &mut reader,
            &view,
            FrameSize::new(256, 256),
            true,
            false,
            false,
        )
        .unwrap();
        assert!(info.reuse_tile_info);
        assert_eq!(info.tile_cols, 2);
        assert_eq!(info.tile_rows, 2);
        assert_eq!(info.tile_cols_log2, 1);
        assert_eq!(info.tile_rows_log2, 1);
        assert_eq!(info.mi_col_starts.as_ref(), [0, 32, 64].as_slice());
        assert_eq!(info.mi_row_starts.as_ref(), [0, 32, 64].as_slice());
        assert_eq!(info.context_update_tile_id, 2);
        assert_eq!(info.tile_size_bytes, Some(2));
        assert_eq!(reader.consumed_bits(), 5);
    }

    #[test]
    fn reuse_is_inferred_without_allow_tile_info_change() {
        let mut view = base_view();
        view.seq_tile_info_present_flag = true;
        view.seq_tile_params = Some(uniform_2x2_seq_params());
        let mut bits = Bits::default();
        bits.f(0, 2); // context_update_tile_id
        bits.f(0, 2); // tile_size_bytes_minus_1 -> TileSizeBytes = 1
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let info = parse_tile_info(
            &mut reader,
            &view,
            FrameSize::new(256, 256),
            true,
            false,
            false,
        )
        .unwrap();
        assert!(info.reuse_tile_info);
        assert_eq!(info.tile_cols, 2);
        assert_eq!(info.tile_rows, 2);
        assert_eq!(info.tile_size_bytes, Some(1));
        assert_eq!(reader.consumed_bits(), 4);
    }

    #[test]
    fn ineligible_uniform_sequence_layout_skips_reuse_bit() {
        let mut view = base_view();
        view.seq_tile_info_present_flag = true;
        view.allow_tile_info_change = true;
        view.seq_tile_params = Some(uniform_2x2_seq_params());
        let mut bits = Bits::default();
        bits.bit(1); // uniform_tile_spacing_flag (tile_params)
        let data = bits.into_bytes();
        let info = parse(&view, &data, FrameSize::new(16, 8)).unwrap();
        assert!(!info.reuse_tile_info);
        assert_eq!(info.tile_cols, 1);
        assert_eq!(info.tile_rows, 1);
    }

    #[test]
    fn avg_cdf_gating_skips_context_update_tile_id() {
        let mut view = base_view();
        view.enable_avg_cdf = true;
        view.avg_cdf_type = 1;
        let mut bits = Bits::default();
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(1); // increment_tile_cols_log2 = 1
        bits.bit(0); // increment_tile_cols_log2 = 0
        bits.bit(0); // increment_tile_rows_log2 = 0
        bits.f(2, 2); // tile_size_bytes_minus_1 -> TileSizeBytes = 3
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let info = parse_tile_info(
            &mut reader,
            &view,
            FrameSize::new(256, 256),
            true,
            false,
            false,
        )
        .unwrap();
        assert_eq!(info.tile_cols, 2);
        assert_eq!(info.context_update_tile_id, 0);
        assert_eq!(info.tile_size_bytes, Some(3));
        assert_eq!(reader.consumed_bits(), 6);
    }

    #[test]
    fn bridge_frame_reads_no_bits_for_minimal_layout() {
        let mut view = base_view();
        view.seq_tile_info_present_flag = true;
        view.allow_tile_info_change = true;
        view.seq_tile_params = Some(uniform_2x2_seq_params());
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        let info = parse_tile_info(
            &mut reader,
            &view,
            FrameSize::new(256, 256),
            true,
            true,
            false,
        )
        .unwrap();
        assert!(!info.reuse_tile_info);
        assert_eq!(info.tile_cols, 1);
        assert_eq!(info.tile_rows, 1);
        assert_eq!(info.context_update_tile_id, 0);
        assert_eq!(info.tile_size_bytes, None);
        assert_eq!(reader.consumed_bits(), 0);
    }

    #[test]
    fn tip_frame_as_output_skips_context_fields() {
        let mut bits = Bits::default();
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(1); // increment_tile_cols_log2 = 1
        bits.bit(0); // increment_tile_cols_log2 = 0
        bits.bit(0); // increment_tile_rows_log2 = 0
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let info = parse_tile_info(
            &mut reader,
            &base_view(),
            FrameSize::new(256, 256),
            true,
            false,
            true,
        )
        .unwrap();
        assert_eq!(info.tile_cols, 2);
        assert_eq!(info.context_update_tile_id, 0);
        assert_eq!(info.tile_size_bytes, None);
        assert_eq!(reader.consumed_bits(), 4);
    }

    #[test]
    fn non_uniform_sequence_reuse_passes_recorded_starts_through() {
        let mut view = base_view();
        view.seq_tile_info_present_flag = true;
        let mut params = uniform_2x2_seq_params();
        params.uniform_spacing = false;
        view.seq_tile_params = Some(params);
        view.seq_sb_col_starts = std::sync::Arc::from(vec![0, 2]);
        view.seq_sb_row_starts = std::sync::Arc::from(vec![0, 2]);
        let mut bits = Bits::default();
        bits.f(2, 2); // context_update_tile_id (n = TileRowsLog2 + TileColsLog2 = 2)
        bits.f(1, 2); // tile_size_bytes_minus_1 -> TileSizeBytes = 2
        let data = bits.into_bytes();
        let info = parse(&view, &data, FrameSize::new(256, 256)).unwrap();
        assert!(info.reuse_tile_info);
        assert_eq!(info.tile_cols, 2);
        assert_eq!(info.tile_rows, 2);
        assert_eq!(info.tile_cols_log2, 1);
        assert_eq!(info.tile_rows_log2, 1);
        assert_eq!(info.mi_col_starts.as_ref(), [0, 32, 64].as_slice());
        assert_eq!(info.mi_row_starts.as_ref(), [0, 32, 64].as_slice());
        assert_eq!(info.context_update_tile_id, 2);
        assert_eq!(info.tile_size_bytes, Some(2));
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let _ = parse_tile_info(
            &mut reader,
            &view,
            FrameSize::new(256, 256),
            true,
            false,
            false,
        );
        assert_eq!(reader.consumed_bits(), 4);
    }

    #[test]
    fn non_uniform_sequence_reuse_gate_mismatch_parses_fresh() {
        let mut view = base_view();
        view.seq_tile_info_present_flag = true;
        view.allow_tile_info_change = true;
        let mut params = uniform_2x2_seq_params();
        params.uniform_spacing = false;
        view.seq_tile_params = Some(params);
        view.seq_sb_col_starts = std::sync::Arc::from(vec![0, 2]);
        view.seq_sb_row_starts = std::sync::Arc::from(vec![0, 2]);
        let mut bits = Bits::default();
        bits.bit(1); // uniform_tile_spacing_flag (fresh tile_params)
        let data = bits.into_bytes();
        let info = parse(&view, &data, FrameSize::new(16, 8)).unwrap();
        assert!(!info.reuse_tile_info);
        assert_eq!(info.tile_cols, 1);
        assert_eq!(info.tile_rows, 1);
    }

    #[test]
    fn non_uniform_sequence_reuse_eof_in_context_fields() {
        let mut view = base_view();
        view.seq_tile_info_present_flag = true;
        let mut params = uniform_2x2_seq_params();
        params.uniform_spacing = false;
        view.seq_tile_params = Some(params);
        view.seq_sb_col_starts = std::sync::Arc::from(vec![0, 2]);
        view.seq_sb_row_starts = std::sync::Arc::from(vec![0, 2]);
        assert!(matches!(
            parse(&view, &[], FrameSize::new(256, 256)),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn missing_sequence_layout_with_present_flag_is_unimplemented() {
        let mut view = base_view();
        view.seq_tile_info_present_flag = true;
        assert!(matches!(
            parse(&view, &[], FrameSize::new(16, 8)),
            Err(Error::Unimplemented {
                feature: "AV2-5.18.7-SEGMENTATION-TILING"
            })
        ));
    }

    #[test]
    fn truncated_tile_info_is_a_typed_eof_error() {
        assert!(matches!(
            parse(&base_view(), &[], FrameSize::new(256, 256)),
            Err(Error::UnexpectedEof { .. })
        ));

        let mut bits = Bits::default();
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(1); // increment_tile_cols_log2 = 1
        bits.bit(0); // increment_tile_cols_log2 = 0
        bits.bit(1); // increment_tile_rows_log2 = 1
        bits.bit(0); // increment_tile_rows_log2 = 0
        let data = bits.into_bytes();
        assert_eq!(data.len(), 1);
        assert!(matches!(
            parse(&base_view(), &data, FrameSize::new(256, 256)),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn uniform_eligible_matches_spec_formula() {
        assert!(uniform_eligible(0, 1));
        assert!(uniform_eligible(1, 4));
        assert!(!uniform_eligible(1, 1));
        assert!(!uniform_eligible(2, 5));
        assert!(!uniform_eligible(0, 0));
        assert!(!uniform_eligible(255, 4));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    fn sb_flags(idx: u8) -> (bool, bool) {
        match idx % 3 {
            0 => (false, false),
            1 => (false, true),
            _ => (true, false),
        }
    }

    fn sb_size(idx: u8) -> SuperblockSize {
        match idx % 3 {
            0 => SuperblockSize::Block64x64,
            1 => SuperblockSize::Block128x128,
            _ => SuperblockSize::Block256x256,
        }
    }

    proptest! {
        /// `tile_info()` must never panic on arbitrary payloads, frame sizes, and
        /// sequence tile state across the AV2-legal domain.
        #[test]
        fn parse_tile_info_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..64),
            frame_width in 1u32..=65536,
            frame_height in 1u32..=65536,
            present in any::<bool>(),
            allow_change in any::<bool>(),
            has_params in any::<bool>(),
            uniform in any::<bool>(),
            tile_cols in 0u32..=64,
            tile_rows in 0u32..=64,
            tile_cols_log2 in 0u8..=8,
            tile_rows_log2 in 0u8..=8,
            seq_sb_cols in 0u32..=2048,
            seq_sb_rows in 0u32..=2048,
            sb in any::<u8>(),
            seq_sb in any::<u8>(),
            enable_avg_cdf in any::<bool>(),
            avg_cdf_type in 0u8..=3,
            tier_high in any::<bool>(),
            level in 0u8..=31,
            frame_is_intra in any::<bool>(),
            is_bridge in any::<bool>(),
            tip_frame_as_output in any::<bool>(),
            seq_sb_col_starts in proptest::collection::vec(0u32..=4096, 0..=64),
            seq_sb_row_starts in proptest::collection::vec(0u32..=4096, 0..=64),
        ) {
            let (use_256, use_128) = sb_flags(sb);
            let view = CoreSeqTileView {
                seq_tile_info_present_flag: present,
                allow_tile_info_change: allow_change,
                seq_tile_params: has_params.then_some(TileParams {
                    tile_cols,
                    tile_rows,
                    tile_cols_log2,
                    tile_rows_log2,
                    sb_cols: seq_sb_cols,
                    sb_rows: seq_sb_rows,
                    uniform_spacing: uniform,
                    covers_cols: true,
                    covers_rows: true,
                }),
                seq_sb_col_starts: std::sync::Arc::from(seq_sb_col_starts),
                seq_sb_row_starts: std::sync::Arc::from(seq_sb_row_starts),
                seq_sb_size: sb_size(seq_sb),
                use_256x256_superblock: use_256,
                use_128x128_superblock: use_128,
                enable_avg_cdf,
                avg_cdf_type,
                seq_tier: if tier_high { Tier::High } else { Tier::Main },
                seq_level_idx: LevelIdx::from_bits(level),
            };
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_tile_info(
                &mut reader,
                &view,
                FrameSize::new(frame_width, frame_height),
                frame_is_intra,
                is_bridge,
                tip_frame_as_output,
            );
        }
    }
}
