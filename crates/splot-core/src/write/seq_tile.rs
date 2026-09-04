// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 sequence-header **filter / tile** writers and the composing
//! [`write_sequence_header`] (`ENC-BITSTREAM-WRITER`) — the inverses of the
//! § 5.4.10 / § 5.4.2 parsers and `parse_sequence_header` in
//! [`crate::headers::sequence`]:
//!
//! - [`write_sequence_filter_config`] — `sequence_filter_config()` (§ 5.4.10,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-10`).
//! - [`write_sequence_tile_config`] — `sequence_tile_config()` (§ 5.4.2,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-2`), delegating to a
//!   `write_tile_params` that re-derives the exact `tile_params()` signaled bits
//!   (§ 5.18.7.3, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-3`) from the
//!   stored [`TileParams`] / `SeqSbColStarts` / `SeqSbRowStarts` against the level/tier
//!   superblock scaling tables.
//! - [`write_sequence_header`] — `sequence_header_obu()` payload (§ 5.4.1,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-1`), composing the
//!   general fields, the § 5.4.3 – § 5.4.8 config cascade, the filter config, the tile
//!   config, and `film_grain_params_present` in § 5.4.1 read order. It writes the
//!   **payload only** — the OBU header (§ 5.2.2) and `trailing_bits()` (§ 5.2.3) are the
//!   caller's job, matching how the parser separates `open_bitstream_unit` from
//!   `parse_sequence_header`.
//!
//! Like the other config writers, this module is additive: it depends on the
//! model/parser read-only and serializes a parsed header back to bits via
//! [`BitWriter`]. The universal contract is semantic `read(write(x)) == x` for every
//! model the parser can produce:
//! `parse_sequence_header(write_sequence_header(h)) == h`.
//!
//! **Re-derive, never store raw bits.** The tile config does not store the signaled
//! `uniform_tile_spacing_flag` / `increment_tile_*_log2` / `width_in_sbs_minus_1`
//! `ns()` bits; it stores the *derived* [`TileParams`] and the superblock start arrays.
//! `write_tile_params` reproduces the exact bit sequence by re-running the parser's
//! § 5.18.7.3 derivation forward (using the parser's level/tier scaling table) and
//! a model whose derived state the parser could never have produced is rejected up front
//! with a typed [`WriteError`] *before any bit is written* (reject-before-write). A
//! reserved-level header the parser left as a bounded residual
//! (`SequenceHeader::unimplemented_at`) cannot be re-emitted at all and is rejected with
//! [`WriteError::UnwritableSequenceHeader`].

#[cfg(test)]
use crate::headers::sequence::Tier;
use crate::headers::sequence::{
    CdefOnSkipTxfm, SequenceFilterConfig, SequenceHeader, SequenceTileConfig, SuperblockSize,
};
use crate::tile::{
    MAX_TILE_AREA, MAX_TILE_COLS, MAX_TILE_ROWS, MAX_TILE_WIDTH, TileParams, TileParamsInput,
    mi_width_log2, num_4x4_blocks_wide, tile_log2, tile_scaling_factors, uniform_spacing,
};
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};
use crate::write::seq_config::{
    check_inter_encodable, check_intra_encodable, check_partition_encodable, check_scc_encodable,
    check_segment_encodable, check_tq_entropy_encodable, write_sequence_inter_config,
    write_sequence_intra_config, write_sequence_partition_config, write_sequence_scc_config,
    write_sequence_segment_config, write_sequence_transform_quant_entropy_config,
};
use crate::write::seq_header::{check_general_encodable, write_sequence_header_general};

/// `seq_level_idx` value reserved for the unconstrained (no-level) tile case
/// (AV2 v1.0.0 § 5.18.7.3: `if (seq_level_idx != 31)`). Duplicated locally because the
/// parser's copy in [`crate::tile`] is private.
const NO_LEVEL_IDX: u8 = 31;

/// Returns `Ok(())` if `value` fits in `width_bits`, else [`WriteError::ValueTooWide`].
fn check_field_width(value: u64, width_bits: u32) -> WriteResult<()> {
    let fits = width_bits >= 64 || value < (1u64 << width_bits);
    if fits {
        Ok(())
    } else {
        Err(WriteError::ValueTooWide { value, width_bits })
    }
}

/// Writes `sequence_filter_config()` (AV2 v1.0.0 § 5.4.10,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-10`), the exact inverse of
/// [`crate::headers::sequence::parse_sequence_filter_config`].
///
/// `single_picture` (`single_picture_header_flag`) and `seq_sb_size`
/// (`get_seq_sb_size()`) are threaded in from the general / partition configs — never
/// re-derived. `single_picture` gates the `CdefOnSkipTxfm` encoding (a single-picture
/// header infers [`CdefOnSkipTxfm::Adaptive`] with no bits); `seq_sb_size` gates
/// `gdf_unit_matches_sb_size` (signaled only when `enable_gdf && seq_sb_size == 64×64`).
///
/// Field writes (in § 5.4.10 read order): `disable_loopfilters_across_tiles`,
/// `enable_cdef`, `enable_gdf` each `f(1)`; `gdf_unit_matches_sb_size` `f(1)` (only when
/// `enable_gdf && seq_sb_size == Block64x64`); `enable_restoration` `f(1)`; when
/// restoration is enabled, `lr_pc_wiener_disabled`, `lr_wiener_nonsep_disabled`,
/// `lr_tools_uv_present` each `f(1)` and `lr_uv_wiener_nonsep_disabled` `f(1)` (only when
/// `lr_tools_uv_present`, else mirrored from `lr_wiener_nonsep_disabled`); `enable_ccso`
/// `f(1)`; `ccso_unit_matches_sb_size` `f(1)` (only when `enable_ccso`); the
/// `CdefOnSkipTxfm` bit pattern (only when `!single_picture`); `df_par_bits_minus_2`
/// `f(2)`. `lr_uv_pc_wiener_disabled` is always inferred `= enable_restoration` and is
/// never signaled.
///
/// The model is fully validated before any bit is written.
///
/// # Errors
/// - [`WriteError::NonCanonicalSequenceValue`] if a derived/inferred field disagrees with
///   the § 5.4.10 re-derivation: `gdf_unit_matches_sb_size` set while its gate is false;
///   `lr_uv_pc_wiener_disabled != enable_restoration`; the restoration fields non-default
///   while `!enable_restoration`; `lr_uv_wiener_nonsep_disabled` not mirroring
///   `lr_wiener_nonsep_disabled` while `!lr_tools_uv_present`; `lr_tools_uv_present` set
///   while `!enable_restoration`; `ccso_unit_matches_sb_size` set while `!enable_ccso`; or
///   `cdef_on_skip_txfm != Adaptive` while `single_picture`.
/// - [`WriteError::ValueTooWide`] if `df_par_bits_minus_2` exceeds `f(2)`.
pub fn write_sequence_filter_config(
    writer: &mut BitWriter,
    config: &SequenceFilterConfig,
    single_picture: bool,
    seq_sb_size: SuperblockSize,
) -> WriteResult<()> {
    check_filter_encodable(config, single_picture, seq_sb_size)?;

    writer.write_flag(config.disable_loopfilters_across_tiles)?;
    writer.write_flag(config.enable_cdef)?;
    writer.write_flag(config.enable_gdf)?;
    if config.enable_gdf && seq_sb_size == SuperblockSize::Block64x64 {
        writer.write_flag(config.gdf_unit_matches_sb_size)?;
    }
    writer.write_flag(config.enable_restoration)?;
    if config.enable_restoration {
        writer.write_flag(config.lr_pc_wiener_disabled)?;
        writer.write_flag(config.lr_wiener_nonsep_disabled)?;
        writer.write_flag(config.lr_tools_uv_present)?;
        if config.lr_tools_uv_present {
            writer.write_flag(config.lr_uv_wiener_nonsep_disabled)?;
        }
    }
    writer.write_flag(config.enable_ccso)?;
    if config.enable_ccso {
        writer.write_flag(config.ccso_unit_matches_sb_size)?;
    }
    if !single_picture {
        match config.cdef_on_skip_txfm {
            CdefOnSkipTxfm::AlwaysOn => writer.write_bit(1)?,
            CdefOnSkipTxfm::Disabled => {
                writer.write_bit(0)?;
                writer.write_bit(1)?;
            }
            CdefOnSkipTxfm::Adaptive => {
                writer.write_bit(0)?;
                writer.write_bit(0)?;
            }
        }
    }
    writer.write_bits_u8(config.df_par_bits_minus_2, 2)?;
    Ok(())
}

/// Validates that `config` is a model the § 5.4.10 parser could have produced.
fn check_filter_encodable(
    config: &SequenceFilterConfig,
    single_picture: bool,
    seq_sb_size: SuperblockSize,
) -> WriteResult<()> {
    let gdf_unit_signaled = config.enable_gdf && seq_sb_size == SuperblockSize::Block64x64;
    if !gdf_unit_signaled && config.gdf_unit_matches_sb_size {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "gdf_unit_matches_sb_size",
        });
    }
    if config.lr_uv_pc_wiener_disabled != config.enable_restoration {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "lr_uv_pc_wiener_disabled",
        });
    }
    if config.enable_restoration {
        if !config.lr_tools_uv_present
            && config.lr_uv_wiener_nonsep_disabled != config.lr_wiener_nonsep_disabled
        {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "lr_uv_wiener_nonsep_disabled",
            });
        }
    } else if config.lr_pc_wiener_disabled
        || config.lr_wiener_nonsep_disabled
        || config.lr_tools_uv_present
        || config.lr_uv_wiener_nonsep_disabled
    {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "restoration_subfields",
        });
    }
    if !config.enable_ccso && config.ccso_unit_matches_sb_size {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "ccso_unit_matches_sb_size",
        });
    }
    if single_picture && config.cdef_on_skip_txfm != CdefOnSkipTxfm::Adaptive {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "cdef_on_skip_txfm",
        });
    }
    check_field_width(u64::from(config.df_par_bits_minus_2), 2)?;
    Ok(())
}

/// The level/tier-derived bounds `tile_params()` (AV2 § 5.18.7.3) computes from the
/// frame dimensions before reading any bit: the superblock grid, the per-axis tile-count
/// log2 limits, and `minLog2Tiles`. Recomputed here exactly as
/// [`crate::tile::parse_tile_layout`] does, so the writer's increment-loop / `ns()`
/// inversions land on the same bit boundaries.
pub(crate) struct TileGrid {
    mi_cols: u32,
    mi_rows: u32,
    sb_cols: u32,
    sb_rows: u32,
    max_tile_width_sb: u32,
    min_log2_tile_cols: u8,
    max_log2_tile_cols: u8,
    max_log2_tile_rows: u8,
    min_log2_tiles: u8,
}

/// Recomputes the § 5.18.7.3 pre-bit grid/scaling bounds, or `None` for a reserved
/// `seq_level_idx` (no defined scaling factor — the parser's `Error::Unimplemented`
/// case, which the writer surfaces as [`WriteError::UnwritableSequenceHeader`]).
pub(crate) fn compute_tile_grid(input: &TileParamsInput) -> Option<TileGrid> {
    let sb4x4 = num_4x4_blocks_wide(input.sb_size);
    let sb_shift = mi_width_log2(input.sb_size);
    let mi_cols = 2 * (input.frame_width.saturating_add(7) >> 3);
    let mi_rows = 2 * (input.frame_height.saturating_add(7) >> 3);
    let sb_cols = mi_cols.saturating_add(sb4x4 - 1) >> sb_shift;
    let sb_rows = mi_rows.saturating_add(sb4x4 - 1) >> sb_shift;

    let level_idx = input.seq_level_idx.get();
    let (max_tile_width_sb, max_tile_area_sb) = if level_idx == NO_LEVEL_IDX {
        (sb_cols, sb_cols.saturating_mul(sb_rows))
    } else {
        let (width_sf, area_sf) = tile_scaling_factors(input.seq_tier, level_idx)?;
        let max_tile_width_sb = (width_sf * MAX_TILE_WIDTH) >> (sb_shift + 4);
        let max_tile_area_sb = (area_sf * MAX_TILE_AREA) >> (2 * (sb_shift + 2) + 2);
        (max_tile_width_sb, max_tile_area_sb)
    };

    let min_log2_tile_cols = tile_log2(max_tile_width_sb, sb_cols);
    let max_log2_tile_cols = tile_log2(1, sb_cols.min(MAX_TILE_COLS));
    let max_log2_tile_rows = tile_log2(1, sb_rows.min(MAX_TILE_ROWS));
    let min_log2_tiles =
        min_log2_tile_cols.max(tile_log2(max_tile_area_sb, sb_rows.saturating_mul(sb_cols)));

    Some(TileGrid {
        mi_cols,
        mi_rows,
        sb_cols,
        sb_rows,
        max_tile_width_sb,
        min_log2_tile_cols,
        max_log2_tile_cols,
        max_log2_tile_rows,
        min_log2_tiles,
    })
}

/// Writes `sequence_tile_config()` (AV2 v1.0.0 § 5.4.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-2`), the exact inverse of
/// [`crate::headers::sequence::parse_sequence_tile_config`].
///
/// `input` is the `tile_params(maxFrameWidth, maxFrameHeight, seqSbSize, seqSbSize, 0)`
/// argument the parser builds from the general header and partition config (the sequence
/// call site always passes `is_bridge = false`; an `is_bridge = true` input is rejected up
/// front — see Errors).
///
/// Field writes (in § 5.4.2 read order): `seq_tile_info_present_flag` `f(1)`; when set,
/// `allow_tile_info_change` `f(1)` then the `tile_params()` bits (see
/// `write_tile_params`).
///
/// The model is fully validated before any bit is written.
///
/// # Errors
/// - [`WriteError::NonCanonicalSequenceValue`] (`what = "is_bridge"`) if `input.is_bridge`
///   is set — a bridge layout is a § 5.18.7.4 frame concept, never a § 5.4.2 sequence config.
/// - [`WriteError::UnwritableSequenceHeader`] if `seq_tile_info_present_flag` is set but
///   `params` is `None` (a reserved-level residual the parser could not model — its tile
///   bits were never parsed, so they cannot be re-emitted).
/// - [`WriteError::NonCanonicalSequenceValue`] if the `Option` payloads
///   (`allow_tile_info_change` / `params`) or the recorded `seq_sb_col_starts` /
///   `seq_sb_row_starts` disagree with the § 5.18.7.3 re-derivation.
/// - [`WriteError::ValueOutOfRange`] if a non-uniform `width_in_sbs_minus_1` /
///   `height_in_sbs_minus_1` lies outside its `ns()` domain.
pub fn write_sequence_tile_config(
    writer: &mut BitWriter,
    config: &SequenceTileConfig,
    input: TileParamsInput,
) -> WriteResult<()> {
    check_tile_encodable(config, &input)?;

    writer.write_flag(config.seq_tile_info_present_flag)?;
    if !config.seq_tile_info_present_flag {
        return Ok(());
    }
    let allow = config
        .allow_tile_info_change
        .ok_or(WriteError::NonCanonicalSequenceValue {
            what: "allow_tile_info_change",
        })?;
    let params = config
        .params
        .as_ref()
        .ok_or(WriteError::NonCanonicalSequenceValue {
            what: "tile_params",
        })?;
    writer.write_flag(allow)?;
    write_tile_params(
        writer,
        params,
        &config.seq_sb_col_starts,
        &config.seq_sb_row_starts,
        &input,
    )
}

/// Validates that `config` is a model the § 5.4.2 parser could have produced, including
/// the reserved-level residual and the full `tile_params()` derivation.
fn check_tile_encodable(config: &SequenceTileConfig, input: &TileParamsInput) -> WriteResult<()> {
    if input.is_bridge {
        return Err(WriteError::NonCanonicalSequenceValue { what: "is_bridge" });
    }

    if !config.seq_tile_info_present_flag {
        if config.allow_tile_info_change.is_some()
            || config.params.is_some()
            || !config.seq_sb_col_starts.is_empty()
            || !config.seq_sb_row_starts.is_empty()
        {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "seq_tile_info_present_flag",
            });
        }
        return Ok(());
    }

    let Some(params) = config.params.as_ref() else {
        return Err(WriteError::UnwritableSequenceHeader {
            feature: "AV2-5.4.2-SEQUENCE-TILE-CONFIG",
        });
    };
    if config.allow_tile_info_change.is_none() {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "allow_tile_info_change",
        });
    }

    let Some(grid) = compute_tile_grid(input) else {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "tile_params_level",
        });
    };
    check_tile_params_encodable(
        params,
        &config.seq_sb_col_starts,
        &config.seq_sb_row_starts,
        input,
        &grid,
    )
}

/// Writes the `tile_params()` (§ 5.18.7.3) bits at the sequence call site, the inverse of
/// the § 5.18.7.3 read loop in [`crate::tile::parse_tile_layout`].
///
/// The model stores the *derived* [`TileParams`] (counts, log2 sizes, the
/// `uniform_spacing` flag, the superblock grid) and the `sb_col_starts` /
/// `sb_row_starts` arrays — not the signaled bits. This function re-derives the exact
/// bits:
///
/// - `uniform_tile_spacing_flag` `f(1)` (the sequence call site is never a bridge).
/// - **Uniform branch:** the column increment run — `increment_tile_cols_log2` `f(1)`
///   ones until the loop target is reached, then a terminating `0` if the loop has not
///   hit `maxLog2TileCols`. The loop target is the smallest `tileColsLog2` (from
///   `minLog2TileCols` up) whose `uniform_spacing()` reproduces the stored column starts.
///   The row increment run is the analogue, starting from
///   `Max(minLog2Tiles - tileColsLog2, 0)`.
/// - **Non-uniform branch:** for each column start, `width_in_sbs_minus_1`
///   `ns(Min(sbCols - startSb, maxTileWidthSb))` of `size - 1`; then, with `maxTileAreaSb`
///   recomputed from `minLog2Tiles` and `maxTileHeightSb` from the widest column, for each
///   row start, `height_in_sbs_minus_1` `ns(Min(sbRows - startSb, maxTileHeightSb))`.
///
/// Pre-validated by [`check_tile_params_encodable`]; this function re-runs the same
/// derivation and so cannot fail on a validated model (it still propagates `ns()` errors
/// defensively).
pub(crate) fn write_tile_params(
    writer: &mut BitWriter,
    params: &TileParams,
    sb_col_starts: &[u32],
    sb_row_starts: &[u32],
    input: &TileParamsInput,
) -> WriteResult<()> {
    let grid = compute_tile_grid(input).ok_or(WriteError::NonCanonicalSequenceValue {
        what: "tile_params_level",
    })?;

    writer.write_flag(params.uniform_spacing)?;

    if params.uniform_spacing {
        let cols_target = uniform_cols_log2_target(input, &grid, sb_col_starts)?;
        write_increment_run(
            writer,
            grid.min_log2_tile_cols,
            cols_target,
            grid.max_log2_tile_cols,
        )?;
        let cols = uniform_spacing(cols_target, grid.mi_cols, input.uniform_sb_size);
        let tile_cols_log2 = tile_log2(1, cols.count);

        let min_log2_tile_rows = grid.min_log2_tiles.saturating_sub(tile_cols_log2);
        let rows_target =
            uniform_rows_log2_target(input, &grid, min_log2_tile_rows, sb_row_starts)?;
        write_increment_run(
            writer,
            min_log2_tile_rows,
            rows_target,
            grid.max_log2_tile_rows,
        )?;
    } else {
        write_non_uniform_tile_params(writer, sb_col_starts, sb_row_starts, &grid)?;
    }
    Ok(())
}

/// Writes the `increment_tile_*_log2` unary run for the uniform branch (§ 5.18.7.3): one
/// `1` bit per increment from `start` to `target`, then a terminating `0` when `target`
/// is below `max` (the parser breaks on a `0` bit; at `max` the loop exits without a bit).
fn write_increment_run(writer: &mut BitWriter, start: u8, target: u8, max: u8) -> WriteResult<()> {
    let mut current = start;
    while current < target {
        writer.write_bit(1)?;
        current += 1;
    }
    if current < max {
        writer.write_bit(0)?;
    }
    Ok(())
}

/// Recovers the column-increment loop target for the uniform branch: the smallest
/// `tileColsLog2` in `[minLog2TileCols, maxLog2TileCols]` whose `uniform_spacing()` at the
/// frame `miCols` reproduces the stored `sb_col_starts`. Picking the smallest is the
/// canonical (shortest-bit) reachable encoding; the parser, reading any longer run that
/// yielded the same starts, produces an identical model.
fn uniform_cols_log2_target(
    input: &TileParamsInput,
    grid: &TileGrid,
    sb_col_starts: &[u32],
) -> WriteResult<u8> {
    for target in grid.min_log2_tile_cols..=grid.max_log2_tile_cols {
        let cols = uniform_spacing(target, grid.mi_cols, input.uniform_sb_size);
        if cols.starts.as_ref() == sb_col_starts {
            return Ok(target);
        }
    }
    Err(WriteError::NonCanonicalSequenceValue {
        what: "seq_sb_col_starts",
    })
}

/// Recovers the row-increment loop target for the uniform branch: the smallest
/// `tileRowsLog2` in `[minLog2TileRows, maxLog2TileRows]` whose `uniform_spacing()`
/// reproduces the stored `sb_row_starts`.
fn uniform_rows_log2_target(
    input: &TileParamsInput,
    grid: &TileGrid,
    min_log2_tile_rows: u8,
    sb_row_starts: &[u32],
) -> WriteResult<u8> {
    for target in min_log2_tile_rows..=grid.max_log2_tile_rows {
        let rows = uniform_spacing(target, grid.mi_rows, input.uniform_sb_size);
        if rows.starts.as_ref() == sb_row_starts {
            return Ok(target);
        }
    }
    Err(WriteError::NonCanonicalSequenceValue {
        what: "seq_sb_row_starts",
    })
}

/// Writes the non-uniform column / row `ns()` runs (§ 5.18.7.3), the inverse of the two
/// `for ( ; startSb < sb*; )` loops. Tile sizes are recovered as the deltas between
/// consecutive `sb_*_starts` (the last running to the grid edge), so
/// `width_in_sbs_minus_1 = size - 1`.
fn write_non_uniform_tile_params(
    writer: &mut BitWriter,
    sb_col_starts: &[u32],
    sb_row_starts: &[u32],
    grid: &TileGrid,
) -> WriteResult<()> {
    let mut widest_tile_sb = 1u32;
    for (i, &start_sb) in sb_col_starts.iter().enumerate() {
        let next = sb_col_starts.get(i + 1).copied().unwrap_or(grid.sb_cols);
        let size_sb = next - start_sb;
        widest_tile_sb = widest_tile_sb.max(size_sb);
        let n = (grid.sb_cols - start_sb).min(grid.max_tile_width_sb);
        writer.write_ns(size_sb - 1, n)?;
    }

    let max_tile_area_sb = if grid.min_log2_tiles > 0 {
        grid.sb_rows.saturating_mul(grid.sb_cols) >> (u32::from(grid.min_log2_tiles) + 1)
    } else {
        grid.sb_rows.saturating_mul(grid.sb_cols)
    };
    let max_tile_height_sb = (max_tile_area_sb / widest_tile_sb).max(1);

    for (i, &start_sb) in sb_row_starts.iter().enumerate() {
        let next = sb_row_starts.get(i + 1).copied().unwrap_or(grid.sb_rows);
        let size_sb = next - start_sb;
        let max_height = (grid.sb_rows - start_sb).min(max_tile_height_sb);
        writer.write_ns(size_sb - 1, max_height)?;
    }
    Ok(())
}

/// Validates that `params` / the start arrays are a layout the § 5.18.7.3 parser could
/// have produced, before any tile bit is written. Re-derives the layout forward and
/// checks the stored summary against it, and that every `ns()` value is in domain.
fn check_tile_params_encodable(
    params: &TileParams,
    sb_col_starts: &[u32],
    sb_row_starts: &[u32],
    input: &TileParamsInput,
    grid: &TileGrid,
) -> WriteResult<()> {
    if params.sb_cols != grid.sb_cols || params.sb_rows != grid.sb_rows {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "tile_params_grid",
        });
    }

    check_starts_monotonic(sb_col_starts, grid.sb_cols, "seq_sb_col_starts")?;
    check_starts_monotonic(sb_row_starts, grid.sb_rows, "seq_sb_row_starts")?;
    if sb_col_starts.len() != params.tile_cols as usize
        || sb_row_starts.len() != params.tile_rows as usize
    {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "tile_params_counts",
        });
    }

    if params.uniform_spacing {
        let cols_target = uniform_cols_log2_target(input, grid, sb_col_starts)?;
        let cols = uniform_spacing(cols_target, grid.mi_cols, input.uniform_sb_size);
        let tile_cols_log2 = tile_log2(1, cols.count);
        let min_log2_tile_rows = grid.min_log2_tiles.saturating_sub(tile_cols_log2);
        let rows_target = uniform_rows_log2_target(input, grid, min_log2_tile_rows, sb_row_starts)?;
        let rows = uniform_spacing(rows_target, grid.mi_rows, input.uniform_sb_size);
        if cols.count != params.tile_cols
            || rows.count != params.tile_rows
            || tile_cols_log2 != params.tile_cols_log2
            || tile_log2(1, rows.count) != params.tile_rows_log2
            || !params.covers_cols
            || !params.covers_rows
        {
            return Err(WriteError::NonCanonicalSequenceValue {
                what: "tile_params_uniform",
            });
        }
    } else {
        check_non_uniform_layout(params, sb_col_starts, sb_row_starts, grid)?;
    }
    Ok(())
}

/// Validates a non-uniform layout's `ns()` domains and that the re-derived counts /
/// log2 / coverage match the stored summary.
fn check_non_uniform_layout(
    params: &TileParams,
    sb_col_starts: &[u32],
    sb_row_starts: &[u32],
    grid: &TileGrid,
) -> WriteResult<()> {
    let mut widest_tile_sb = 1u32;
    for (i, &start_sb) in sb_col_starts.iter().enumerate() {
        let next = sb_col_starts.get(i + 1).copied().unwrap_or(grid.sb_cols);
        let size_sb = next - start_sb;
        widest_tile_sb = widest_tile_sb.max(size_sb);
        let n = (grid.sb_cols - start_sb).min(grid.max_tile_width_sb);
        if size_sb > n {
            return Err(WriteError::ValueOutOfRange {
                descriptor: "ns",
                value: i64::from(size_sb - 1),
            });
        }
    }
    let tile_cols_log2 = tile_log2(1, sb_col_starts.len() as u32);

    let max_tile_area_sb = if grid.min_log2_tiles > 0 {
        grid.sb_rows.saturating_mul(grid.sb_cols) >> (u32::from(grid.min_log2_tiles) + 1)
    } else {
        grid.sb_rows.saturating_mul(grid.sb_cols)
    };
    let max_tile_height_sb = (max_tile_area_sb / widest_tile_sb).max(1);

    for (i, &start_sb) in sb_row_starts.iter().enumerate() {
        let next = sb_row_starts.get(i + 1).copied().unwrap_or(grid.sb_rows);
        let size_sb = next - start_sb;
        let max_height = (grid.sb_rows - start_sb).min(max_tile_height_sb);
        if size_sb > max_height {
            return Err(WriteError::ValueOutOfRange {
                descriptor: "ns",
                value: i64::from(size_sb - 1),
            });
        }
    }
    let tile_rows_log2 = tile_log2(1, sb_row_starts.len() as u32);

    if tile_cols_log2 != params.tile_cols_log2
        || tile_rows_log2 != params.tile_rows_log2
        || !params.covers_cols
        || !params.covers_rows
    {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "tile_params_non_uniform",
        });
    }
    Ok(())
}

/// Validates that `starts` begins at 0, is strictly increasing, and stays below `bound`
/// (so the recovered per-tile delta sizes are all >= 1 and the last tile runs to `bound`).
fn check_starts_monotonic(starts: &[u32], bound: u32, what: &'static str) -> WriteResult<()> {
    if starts.is_empty() || starts[0] != 0 {
        return Err(WriteError::NonCanonicalSequenceValue { what });
    }
    let mut prev = starts[0];
    for &start in &starts[1..] {
        if start <= prev || start >= bound {
            return Err(WriteError::NonCanonicalSequenceValue { what });
        }
        prev = start;
    }
    if prev >= bound {
        return Err(WriteError::NonCanonicalSequenceValue { what });
    }
    Ok(())
}

/// Writes the `sequence_header_obu()` payload (AV2 v1.0.0 § 5.4.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-1`), the inverse of
/// [`crate::headers::sequence::parse_sequence_header`].
///
/// Writes the § 5.4.1 payload in read order: the general fields (§ 5.4.1), then the
/// partition (§ 5.4.3), segment (§ 5.4.4), intra (§ 5.4.5), inter (§ 5.4.6), scc
/// (§ 5.4.7), transform/quant/entropy (§ 5.4.8) configs, then the filter config
/// (§ 5.4.10), the tile config (§ 5.4.2), and finally `film_grain_params_present`. The
/// gating values (`monochrome`, `single_picture`, `seq_sb_size`, and the
/// `tile_params()` input) are recomputed exactly as `parse_sequence_header` does and
/// threaded into the child writers.
///
/// This writes the **payload only**: the OBU header (§ 5.2.2) and `trailing_bits()`
/// (§ 5.2.3) are composed by the caller (matching how the parser separates
/// `open_bitstream_unit` from `parse_sequence_header`).
///
/// The whole header is validated up front: every child config's `check_*_encodable`
/// runs (transitively, via the child writers) before any bit is emitted, so a rejected
/// header leaves `writer` unchanged.
///
/// # Errors
/// - [`WriteError::UnwritableSequenceHeader`] if `header.unimplemented_at` is set or the
///   tile config carries a reserved-level residual: the un-modeled tail cannot be
///   re-emitted, so the header is rejected before any bit.
/// - [`WriteError::WriterNotByteAligned`] if `writer` is not on a byte boundary (the
///   § 5.4.1 payload begins byte-aligned, immediately after the OBU header).
/// - any child-config [`WriteError`] (field width, descriptor domain, or a non-canonical
///   derived/inferred value) — all raised before the first bit.
pub fn write_sequence_header(writer: &mut BitWriter, header: &SequenceHeader) -> WriteResult<()> {
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }
    check_sequence_header_encodable(header)?;

    let general = &header.general;
    let monochrome = general.chroma_format_idc.is_monochrome();
    let single_picture = general.single_picture_header_flag;

    write_sequence_header_general(writer, general)?;

    let partition = unwrap_config(header.partition.as_ref(), "partition")?;
    write_sequence_partition_config(writer, partition, monochrome, single_picture)?;
    let seq_sb_size = partition.seq_sb_size();

    write_sequence_segment_config(writer, unwrap_config(header.segment.as_ref(), "segment")?)?;
    write_sequence_intra_config(
        writer,
        unwrap_config(header.intra.as_ref(), "intra")?,
        monochrome,
    )?;
    write_sequence_inter_config(
        writer,
        unwrap_config(header.inter.as_ref(), "inter")?,
        single_picture,
    )?;
    write_sequence_scc_config(
        writer,
        unwrap_config(header.screen_content.as_ref(), "screen_content")?,
        single_picture,
    )?;
    write_sequence_transform_quant_entropy_config(
        writer,
        unwrap_config(
            header.transform_quant_entropy.as_ref(),
            "transform_quant_entropy",
        )?,
        monochrome,
        single_picture,
    )?;
    write_sequence_filter_config(
        writer,
        unwrap_config(header.filter.as_ref(), "filter")?,
        single_picture,
        seq_sb_size,
    )?;

    // AV2 § 5.4.2: tile_params(maxFrameWidth, maxFrameHeight, seqSbSize, seqSbSize, 0).
    let tile_params_input = TileParamsInput {
        frame_width: general.max_frame_width.get(),
        frame_height: general.max_frame_height.get(),
        uniform_sb_size: seq_sb_size,
        sb_size: seq_sb_size,
        is_bridge: false,
        seq_tier: general.seq_tier,
        seq_level_idx: general.seq_level_idx,
    };
    write_sequence_tile_config(
        writer,
        unwrap_config(header.tile.as_ref(), "tile")?,
        tile_params_input,
    )?;

    let film_grain =
        header
            .film_grain_params_present
            .ok_or(WriteError::NonCanonicalSequenceValue {
                what: "film_grain_params_present",
            })?;
    writer.write_flag(film_grain)?;
    Ok(())
}

/// Returns the child config or a [`WriteError::NonCanonicalSequenceValue`] labeled
/// `what` — a fully-parsed header always has every child `Some`, so a `None` here is a
/// hand-built model the parser could never have produced.
fn unwrap_config<'a, T>(config: Option<&'a T>, what: &'static str) -> WriteResult<&'a T> {
    config.ok_or(WriteError::NonCanonicalSequenceValue { what })
}

/// Validates that the WHOLE `header` is re-emittable before any bit is written, so a
/// rejected header leaves the writer untouched. Rejects the un-modeled reserved-level
/// residual and the missing-child case, then runs every child config's
/// `check_*_encodable` (general, partition, segment, intra, inter, scc, tq-entropy,
/// filter, tile) with the same gating values `write_sequence_header` will use. This is
/// the up-front face of reject-before-write for the composite § 5.4.1 structure: no bit
/// is emitted for any header that any child would reject mid-stream.
fn check_sequence_header_encodable(header: &SequenceHeader) -> WriteResult<()> {
    if let Some(feature) = header.unimplemented_at {
        return Err(WriteError::UnwritableSequenceHeader { feature });
    }
    if let Some(tile) = header.tile.as_ref()
        && let Some(feature) = tile.unimplemented_at()
    {
        return Err(WriteError::UnwritableSequenceHeader { feature });
    }

    let general = &header.general;
    let monochrome = general.chroma_format_idc.is_monochrome();
    let single_picture = general.single_picture_header_flag;

    let partition = unwrap_config(header.partition.as_ref(), "partition")?;
    let segment = unwrap_config(header.segment.as_ref(), "segment")?;
    let intra = unwrap_config(header.intra.as_ref(), "intra")?;
    let inter = unwrap_config(header.inter.as_ref(), "inter")?;
    let screen_content = unwrap_config(header.screen_content.as_ref(), "screen_content")?;
    let tq_entropy = unwrap_config(
        header.transform_quant_entropy.as_ref(),
        "transform_quant_entropy",
    )?;
    let filter = unwrap_config(header.filter.as_ref(), "filter")?;
    let tile = unwrap_config(header.tile.as_ref(), "tile")?;
    if header.film_grain_params_present.is_none() {
        return Err(WriteError::NonCanonicalSequenceValue {
            what: "film_grain_params_present",
        });
    }

    check_general_encodable(general)?;
    check_partition_encodable(partition, monochrome, single_picture)?;
    let seq_sb_size = partition.seq_sb_size();
    check_segment_encodable(segment)?;
    check_intra_encodable(*intra, monochrome)?;
    check_inter_encodable(inter, single_picture)?;
    check_scc_encodable(*screen_content, single_picture)?;
    check_tq_entropy_encodable(tq_entropy, monochrome, single_picture)?;
    check_filter_encodable(filter, single_picture, seq_sb_size)?;

    let tile_params_input = TileParamsInput {
        frame_width: general.max_frame_width.get(),
        frame_height: general.max_frame_height.get(),
        uniform_sb_size: seq_sb_size,
        sb_size: seq_sb_size,
        is_bridge: false,
        seq_tier: general.seq_tier,
        seq_level_idx: general.seq_level_idx,
    };
    check_tile_encodable(tile, &tile_params_input)?;
    Ok(())
}

#[cfg(test)]
include!("seq_tile_tests.rs");
#[cfg(test)]
include!("seq_tile_proptests.rs");
