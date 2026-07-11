// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 frame-header **tile_info** writer (`ENC-BITSTREAM-WRITER`) — the byte-exact
//! inverse of the `tile_info()` parser (§ 5.18.7.2,
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-2`) in
//! [`crate::headers::frame`]:
//!
//! - [`write_tile_info`] — `tile_info()` on the intra path: the `reuse_tile_info`
//!   eligibility flag, then either the § 5.18.7.4 `reuse_tile_params()` re-derivation
//!   (no layout bits) or the explicit § 5.18.7.3 `tile_params()` bits, then the trailing
//!   `context_update_tile_id` / `tile_size_bytes_minus_1` fields.
//!
//! Like the other config writers, this module is additive: it depends on the
//! model/parser read-only and serializes a parsed [`TileInfo`] back to bits via
//! [`BitWriter`]. The universal contract is semantic `read(write(x)) == x` for every
//! model the parser can produce: `parse_tile_info(write_tile_info(info)) == info`.
//!
//! **Re-derive, never store raw bits.** The reuse eligibility and the per-axis grid are
//! recomputed exactly as the parser does (`uniform_eligible()` / the superblock grid),
//! so the writer lands on the same bit boundaries. The explicit branch reuses the
//! § 5.18.7.3 `write_tile_params` / `check_tile_params_encodable` from
//! [`crate::write::seq_tile`] against the [`TileParams`] the parser surfaced on
//! [`TileInfo::tile_params`]. A model whose stored fields the parser could never have
//! produced is rejected up front with a typed [`WriteError`] *before any bit is written*
//! (reject-before-write).

use crate::bitio::BitReader;
use crate::headers::frame::{CoreSeqTileView, FrameSize, TileInfo};
use crate::headers::sequence::SuperblockSize;
use crate::span::ByteOffset;
use crate::tile::{
    ReuseTileParamsInput, TileParams, TileParamsInput, mi_width_log2, num_4x4_blocks_wide,
    parse_tile_layout, reuse_tile_params,
};
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};
use crate::write::seq_tile::{compute_tile_grid, write_tile_params};

/// `uniform_eligible(tileLog2, sbNum)` (AV2 v1.0.0 § 5.18.7.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-2`): whether splitting `sbNum`
/// superblocks into `1 << tileLog2` uniform tiles leaves the last tile at least one
/// superblock wide. Recomputed identically to the parser's private `uniform_eligible`
/// (it is not exported), so the writer evaluates the same `reuse_tile_info` gate.
fn uniform_eligible(tile_log2: u8, sb_num: u32) -> bool {
    let tile_log2 = u32::from(tile_log2).min(64);
    let tile_num: i128 = 1 << tile_log2;
    let tile_width = (i128::from(sb_num) + tile_num - 1) >> tile_log2;
    let last_tile_width = i128::from(sb_num) - (tile_num - 1) * tile_width;
    tile_width >= 1 && last_tile_width >= 1
}

/// The re-derived `tile_info()` grid the writer needs before emitting any bit: the frame
/// `SbSize`, the `MiCols` / `MiRows` derived from the frame dimensions, and whether the
/// stored layout is `reuse_tile_info` eligible. Recomputed exactly as `parse_tile_info`
/// (AV2 v1.0.0 § 5.18.7.2, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-2`) does.
/// The `sbShift2` for the `MiColStarts` derivation is branch-dependent (it is
/// `Mi_Width_Log2[seqSbSize]` for a uniform layout, else `Mi_Width_Log2[SbSize]`) and is
/// computed per branch via [`branch_sb_shift2`].
struct TileInfoGrid {
    sb_size: SuperblockSize,
    mi_cols: u32,
    mi_rows: u32,
    eligible: bool,
}

/// The `sbShift2` the parser uses for `MiColStarts[i] = sbColStarts[i] << sbShift2`
/// (AV2 v1.0.0 § 5.18.7.2 / § 5.18.7.3 / § 5.18.7.4,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-2`): `Mi_Width_Log2[seqSbSize]` for
/// a uniform layout (the uniform branch reassigns `sbShift`), else `Mi_Width_Log2[SbSize]`.
/// Clamped below 32 to match the parser's `<<`-overflow guard for hostile direct API usage.
fn branch_sb_shift2(uniform: bool, seq_sb_size: SuperblockSize, sb_size: SuperblockSize) -> u32 {
    let shift = if uniform {
        mi_width_log2(seq_sb_size)
    } else {
        mi_width_log2(sb_size)
    };
    shift.min(31)
}

/// Recomputes the pre-bit grid + reuse eligibility (AV2 v1.0.0 § 5.18.7.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-2`), or a typed [`WriteError`] for
/// the reserved-level case the parser stops on (`seq_tile_info_present_flag` set but
/// `seq_tile_params` `None`).
fn compute_tile_info_grid(
    tile: &CoreSeqTileView,
    frame_size: FrameSize,
    frame_is_intra: bool,
    is_bridge: bool,
) -> WriteResult<TileInfoGrid> {
    let sb_size = tile.frame_sb_size(frame_is_intra);
    let sb4x4 = num_4x4_blocks_wide(sb_size);
    let sb_shift = mi_width_log2(sb_size);
    let mi_cols = 2 * (frame_size.width.saturating_add(7) >> 3);
    let mi_rows = 2 * (frame_size.height.saturating_add(7) >> 3);
    let sb_cols = mi_cols.saturating_add(sb4x4 - 1) >> sb_shift;
    let sb_rows = mi_rows.saturating_add(sb4x4 - 1) >> sb_shift;

    let have_tile_params = !is_bridge && tile.seq_tile_info_present_flag;

    let eligible = match (have_tile_params, tile.seq_tile_params.as_ref()) {
        (false, _) => false,
        (true, None) => {
            return Err(WriteError::UnwritableSequenceHeader {
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

    Ok(TileInfoGrid {
        sb_size,
        mi_cols,
        mi_rows,
        eligible,
    })
}

/// Writes `tile_info()` (AV2 v1.0.0 § 5.18.7.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-2`), the byte-exact inverse of
/// [`crate::headers::frame::parse_tile_info`].
///
/// The gating inputs mirror the parser: `frame_size` is the parsed `FrameWidth` /
/// `FrameHeight`; `frame_is_intra` selects the frame `SbSize` (§ 5.18.2); `is_bridge`
/// forces `haveTileParams = 0`; `tip_frame_as_output` is
/// `TipFrameMode == TIP_FRAME_AS_OUTPUT` and gates the trailing
/// `context_update_tile_id` / `tile_size_bytes_minus_1` reads.
///
/// Field writes (in § 5.18.7.2 read order): `reuse_tile_info` `f(1)` (only when eligible
/// and `allow_tile_info_change`, else inferred with no bit); then either no layout bits
/// (reuse) or the explicit § 5.18.7.3 `tile_params()` bits; then, for a multi-tile,
/// non-bridge, non-TIP-as-output layout, `context_update_tile_id`
/// `f(TileRowsLog2 + TileColsLog2)` (only when `!enable_avg_cdf || !avg_cdf_type`, else
/// inferred `0`) and `tile_size_bytes_minus_1` `f(2)`.
///
/// The model is fully validated before any bit is written (reject-before-write).
///
/// # Errors
/// - [`WriteError::UnwritableSequenceHeader`] if `seq_tile_info_present_flag` is set but
///   the sequence tile layout is `None` (a reserved `seq_level_idx` the parser stops on).
/// - [`WriteError::NonCanonicalFrameHeader`] if a derived/inferred value disagrees with
///   the § 5.18.7.2 re-derivation: an inferred `reuse_tile_info` that does not match its
///   gate; reuse-branch counts / log2 / `mi_*_starts` that do not match the re-derived
///   `reuse_tile_params()`; a missing `tile_params` on the explicit branch; a
///   gated-off `context_update_tile_id != 0`; or `tile_size_bytes` whose presence
///   disagrees with the trailing-field gate.
/// - [`WriteError::ValueTooWide`] if `context_update_tile_id` does not fit its
///   `f(TileRowsLog2 + TileColsLog2)` field, or `tile_size_bytes` is outside `1..=4`.
/// - any § 5.18.7.3 [`WriteError`] from the explicit `tile_params()` re-derivation.
pub fn write_tile_info(
    writer: &mut BitWriter,
    info: &TileInfo,
    tile: &CoreSeqTileView,
    frame_size: FrameSize,
    frame_is_intra: bool,
    is_bridge: bool,
    tip_frame_as_output: bool,
) -> WriteResult<()> {
    let grid = check_tile_info_encodable(
        info,
        tile,
        frame_size,
        frame_is_intra,
        is_bridge,
        tip_frame_as_output,
    )?;

    if grid.eligible && tile.allow_tile_info_change {
        writer.write_flag(info.reuse_tile_info)?;
    }

    if !info.reuse_tile_info && !is_bridge {
        let params = info
            .tile_params
            .as_ref()
            .ok_or(WriteError::NonCanonicalFrameHeader {
                what: "tile_params",
            })?;
        let sb_shift2 = branch_sb_shift2(params.uniform_spacing, tile.seq_sb_size, grid.sb_size);
        let (sb_col_starts, sb_row_starts) = explicit_sb_starts(info, sb_shift2);
        write_tile_params(
            writer,
            params,
            &sb_col_starts,
            &sb_row_starts,
            &explicit_input(tile, frame_size, &grid, is_bridge),
        )?;
    }

    let multi_tile = info.tile_cols > 1 || info.tile_rows > 1;
    if multi_tile && !is_bridge && !tip_frame_as_output {
        if !tile.enable_avg_cdf || tile.avg_cdf_type == 0 {
            let n = u32::from(info.tile_rows_log2) + u32::from(info.tile_cols_log2);
            writer.write_bits(info.context_update_tile_id, n)?;
        }
        let tile_size_bytes = info
            .tile_size_bytes
            .ok_or(WriteError::NonCanonicalFrameHeader {
                what: "tile_size_bytes",
            })?;
        writer.write_bits(tile_size_bytes - 1, 2)?;
    }
    Ok(())
}

/// Recovers the explicit-branch `sbColStarts` / `sbRowStarts` from the stored
/// `mi_*_starts`: drop the trailing `MiCols` / `MiRows` sentinel and undo the `<< sbShift2`
/// (AV2 v1.0.0 § 5.18.7.2, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-2`:
/// `MiColStarts[i] = sbColStarts[i] << sbShift2`).
fn explicit_sb_starts(info: &TileInfo, sb_shift2: u32) -> (Vec<u32>, Vec<u32>) {
    let drop_last = |starts: &[u32]| -> Vec<u32> {
        let body = starts.split_last().map_or(&[][..], |(_, body)| body);
        body.iter().map(|&mi| mi >> sb_shift2).collect()
    };
    (
        drop_last(&info.mi_col_starts),
        drop_last(&info.mi_row_starts),
    )
}

/// Returns `true` if `starts` is a parser-reachable superblock-tile start array within a
/// `bound`-superblock dimension: non-empty, beginning at `0`, strictly increasing, and with
/// every start below `bound` (each tile is at least one superblock and lies inside the grid).
/// The explicit non-uniform `write_tile_params` path subtracts `bound - startSb` and
/// `next - startSb`, so a non-canonical model failing any of these would underflow-panic
/// rather than return a typed error; the writer rejects it before the replay.
fn valid_tile_starts(starts: &[u32], bound: u32) -> bool {
    !starts.is_empty()
        && starts[0] == 0
        && starts.last().is_some_and(|&last| last < bound)
        && starts.windows(2).all(|w| w[1] > w[0])
}

/// The superblock column / row counts for the frame `SbSize`
/// (`sbCols = (MiCols + sb4x4 - 1) >> sbShift`), matching the parser's § 5.18.7.2 derivation.
fn sb_dims(grid: &TileInfoGrid) -> (u32, u32) {
    let sb4x4 = num_4x4_blocks_wide(grid.sb_size);
    let sb_shift = mi_width_log2(grid.sb_size);
    let sb_cols = grid.mi_cols.saturating_add(sb4x4 - 1) >> sb_shift;
    let sb_rows = grid.mi_rows.saturating_add(sb4x4 - 1) >> sb_shift;
    (sb_cols, sb_rows)
}

/// Builds the `tile_params()` input for the explicit branch (AV2 v1.0.0 § 5.18.7.3,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-3`), exactly as the parser's call
/// site does: `tile_params(FrameWidth, FrameHeight, seqSbSize, SbSize, IsBridge)`.
fn explicit_input(
    tile: &CoreSeqTileView,
    frame_size: FrameSize,
    grid: &TileInfoGrid,
    is_bridge: bool,
) -> TileParamsInput {
    TileParamsInput {
        frame_width: frame_size.width,
        frame_height: frame_size.height,
        uniform_sb_size: tile.seq_sb_size,
        sb_size: grid.sb_size,
        is_bridge,
        seq_tier: tile.seq_tier,
        seq_level_idx: tile.seq_level_idx,
    }
}

/// Validates that `info` is a model the parser could have produced (AV2 v1.0.0 § 5.18.7.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-2`) and returns the re-derived
/// grid, before any bit is written (reject-before-write). Every reject path leaves
/// `writer.bit_len() == 0` because [`write_tile_info`] calls this first.
fn check_tile_info_encodable(
    info: &TileInfo,
    tile: &CoreSeqTileView,
    frame_size: FrameSize,
    frame_is_intra: bool,
    is_bridge: bool,
    tip_frame_as_output: bool,
) -> WriteResult<TileInfoGrid> {
    let grid = compute_tile_info_grid(tile, frame_size, frame_is_intra, is_bridge)?;

    let reuse_signaled = grid.eligible && tile.allow_tile_info_change;
    if !reuse_signaled {
        let inferred = grid.eligible;
        if info.reuse_tile_info != inferred {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "reuse_tile_info",
            });
        }
    }

    if info.reuse_tile_info {
        check_reuse_layout(info, tile, &grid)?;
    } else {
        check_explicit_layout(info, tile, frame_size, &grid, is_bridge)?;
    }

    check_trailing_fields(info, tile, is_bridge, tip_frame_as_output)?;
    Ok(grid)
}

/// Validates the reuse branch: the stored counts / log2 / `mi_*_starts` equal the
/// `reuse_tile_params()` re-derivation (AV2 v1.0.0 § 5.18.7.4,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-4`), built from the stored sequence
/// layout exactly as the parser's reuse arm does (uniform passes empty seq start slices;
/// non-uniform passes the recorded `SeqSbColStarts` / `SeqSbRowStarts`).
fn check_reuse_layout(
    info: &TileInfo,
    tile: &CoreSeqTileView,
    grid: &TileInfoGrid,
) -> WriteResult<()> {
    if info.tile_params.is_some() {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "tile_params",
        });
    }
    let seq = tile
        .seq_tile_params
        .as_ref()
        .ok_or(WriteError::NonCanonicalFrameHeader {
            what: "reuse_tile_info",
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
        seq_sb_size: tile.seq_sb_size,
        sb_size: grid.sb_size,
        mi_cols: grid.mi_cols,
        mi_rows: grid.mi_rows,
    });

    let sb_shift2 = branch_sb_shift2(seq.uniform_spacing, tile.seq_sb_size, grid.sb_size);
    let mut mi_col_starts: Vec<u32> = reused
        .sb_col_starts
        .iter()
        .map(|&start| start << sb_shift2)
        .collect();
    mi_col_starts.push(grid.mi_cols);
    let mut mi_row_starts: Vec<u32> = reused
        .sb_row_starts
        .iter()
        .map(|&start| start << sb_shift2)
        .collect();
    mi_row_starts.push(grid.mi_rows);

    if info.tile_cols != reused.tile_cols
        || info.tile_rows != reused.tile_rows
        || info.tile_cols_log2 != reused.tile_cols_log2
        || info.tile_rows_log2 != reused.tile_rows_log2
        || info.mi_col_starts != mi_col_starts
        || info.mi_row_starts != mi_row_starts
    {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "reuse_tile_params",
        });
    }
    Ok(())
}

/// Validates the explicit branch: the surfaced [`TileParams`] + the `sb_*_starts` recovered
/// from `mi_*_starts` are a layout the parser could have produced (AV2 v1.0.0 § 5.18.7.3,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-3`), and the stored counts / log2 /
/// `mi_*_starts` match that derivation exactly. For a bridge frame `tile_params()` writes no
/// bits (it infers the uniform layout), so the stored layout is validated against
/// `parse_tile_layout(is_bridge=true)` instead.
fn check_explicit_layout(
    info: &TileInfo,
    tile: &CoreSeqTileView,
    frame_size: FrameSize,
    grid: &TileInfoGrid,
    is_bridge: bool,
) -> WriteResult<()> {
    let params = info
        .tile_params
        .as_ref()
        .ok_or(WriteError::NonCanonicalFrameHeader {
            what: "tile_params",
        })?;

    let input = explicit_input(tile, frame_size, grid, is_bridge);

    if is_bridge {
        return check_bridge_layout(info, params, &input, grid);
    }
    if compute_tile_grid(&input).is_none() {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "tile_params_level",
        });
    }
    let sb_shift2 = branch_sb_shift2(params.uniform_spacing, tile.seq_sb_size, grid.sb_size);
    let (sb_col_starts, sb_row_starts) = explicit_sb_starts(info, sb_shift2);

    if params.uniform_spacing {
        if !sb_col_starts.windows(2).all(|w| w[1] > w[0]) {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "mi_col_starts",
            });
        }
        if !sb_row_starts.windows(2).all(|w| w[1] > w[0]) {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "mi_row_starts",
            });
        }
    } else {
        let (sb_cols, sb_rows) = sb_dims(grid);
        if !valid_tile_starts(&sb_col_starts, sb_cols) {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "mi_col_starts",
            });
        }
        if !valid_tile_starts(&sb_row_starts, sb_rows) {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "mi_row_starts",
            });
        }
    }

    let mut scratch = BitWriter::new();
    write_tile_params(&mut scratch, params, &sb_col_starts, &sb_row_starts, &input)?;
    let scratch_bytes = scratch.into_bytes();
    let mut reader = BitReader::new(&scratch_bytes, ByteOffset::new(0));
    let layout =
        parse_tile_layout(&mut reader, input).map_err(|_| WriteError::NonCanonicalFrameHeader {
            what: "tile_params_level",
        })?;

    let layout_sb_shift2 = layout.sb_shift2.min(31);
    let mut mi_col_starts: Vec<u32> = layout
        .sb_col_starts
        .iter()
        .map(|&start| start << layout_sb_shift2)
        .collect();
    mi_col_starts.push(grid.mi_cols);
    let mut mi_row_starts: Vec<u32> = layout
        .sb_row_starts
        .iter()
        .map(|&start| start << layout_sb_shift2)
        .collect();
    mi_row_starts.push(grid.mi_rows);

    if *params != layout.params
        || info.tile_cols != layout.params.tile_cols
        || info.tile_rows != layout.params.tile_rows
        || info.tile_cols_log2 != layout.params.tile_cols_log2
        || info.tile_rows_log2 != layout.params.tile_rows_log2
        || info.mi_col_starts != mi_col_starts
        || info.mi_row_starts != mi_row_starts
    {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "tile_params_summary",
        });
    }
    Ok(())
}

/// Validates a bridge-frame explicit layout (AV2 v1.0.0 § 5.18.7.3,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-3`): `tile_params()` infers the
/// uniform layout and reads no bits, so the stored [`TileParams`] / `mi_*_starts` must equal
/// what [`crate::tile::parse_tile_layout`] derives with `is_bridge = true` from an empty
/// reader (it never reads a bit on that path). Any divergence could not have been parsed
/// here.
fn check_bridge_layout(
    info: &TileInfo,
    params: &TileParams,
    input: &TileParamsInput,
    grid: &TileInfoGrid,
) -> WriteResult<()> {
    let mut reader = BitReader::new(&[], ByteOffset::new(0));
    let layout = parse_tile_layout(&mut reader, *input).map_err(|_| {
        WriteError::NonCanonicalFrameHeader {
            what: "tile_params_level",
        }
    })?;

    let sb_shift2 = layout.sb_shift2.min(31);
    let mut mi_col_starts: Vec<u32> = layout
        .sb_col_starts
        .iter()
        .map(|&start| start << sb_shift2)
        .collect();
    mi_col_starts.push(grid.mi_cols);
    let mut mi_row_starts: Vec<u32> = layout
        .sb_row_starts
        .iter()
        .map(|&start| start << sb_shift2)
        .collect();
    mi_row_starts.push(grid.mi_rows);

    if *params != layout.params
        || info.tile_cols != layout.params.tile_cols
        || info.tile_rows != layout.params.tile_rows
        || info.tile_cols_log2 != layout.params.tile_cols_log2
        || info.tile_rows_log2 != layout.params.tile_rows_log2
        || info.mi_col_starts != mi_col_starts
        || info.mi_row_starts != mi_row_starts
    {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "tile_params_summary",
        });
    }
    Ok(())
}

/// Validates the trailing `context_update_tile_id` / `tile_size_bytes` fields against the
/// read gate (AV2 v1.0.0 § 5.18.7.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-2`): present (and in range) exactly
/// when the multi-tile, non-bridge, non-TIP-as-output condition holds; otherwise inferred
/// absent. When the trailing block is read, `context_update_tile_id` is additionally gated
/// by `!enable_avg_cdf || !avg_cdf_type` (else inferred `0`, no bit) and must fit its
/// `f(TileRowsLog2 + TileColsLog2)` field.
fn check_trailing_fields(
    info: &TileInfo,
    tile: &CoreSeqTileView,
    is_bridge: bool,
    tip_frame_as_output: bool,
) -> WriteResult<()> {
    let multi_tile = info.tile_cols > 1 || info.tile_rows > 1;
    let trailing_read = multi_tile && !is_bridge && !tip_frame_as_output;
    if trailing_read {
        if !tile.enable_avg_cdf || tile.avg_cdf_type == 0 {
            let n = u32::from(info.tile_rows_log2) + u32::from(info.tile_cols_log2);
            if n > u32::BITS {
                return Err(WriteError::BitWidthTooLarge {
                    requested: n,
                    max: u32::BITS,
                });
            }
            let fits = n >= u32::BITS || info.context_update_tile_id < (1u32 << n);
            if !fits {
                return Err(WriteError::ValueTooWide {
                    value: u64::from(info.context_update_tile_id),
                    width_bits: n,
                });
            }
        } else if info.context_update_tile_id != 0 {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "context_update_tile_id",
            });
        }
        let tile_size_bytes = info
            .tile_size_bytes
            .ok_or(WriteError::NonCanonicalFrameHeader {
                what: "tile_size_bytes",
            })?;
        if !(1..=4).contains(&tile_size_bytes) {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "tile_size_bytes",
            });
        }
    } else {
        if info.context_update_tile_id != 0 {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "context_update_tile_id",
            });
        }
        if info.tile_size_bytes.is_some() {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "tile_size_bytes",
            });
        }
    }
    Ok(())
}

#[cfg(test)]
include!("frame_tiling_tests.rs");
#[cfg(test)]
include!("frame_tiling_proptests.rs");
