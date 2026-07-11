// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Loop-restoration unit symbol reads for the partition walk.

use super::lr_records::{
    LrSourceBlockDerivation, LrUnitRestorationType, TileLoopRestorationRootFrontier,
    WienerNsLrUnitActivity, WienerNsLrUnitFilter, ceil_unit_index, count_units_in_frame,
    record_active_wiener_ns_source_blocks_for_unit,
};
use super::{
    DecodeLimitName, DecodeLimits, SymbolDecoder, TilePartitionBounds, TilePartitionCall,
    TilePartitionFrameFacts, TilePartitionLoopRestorationPlaneTool,
    TilePartitionLoopRestorationState, TilePartitionTraversalError, TilePartitionTraversalInput,
    call_in_frame, checked_add, checked_mul, checked_mul_shifted, checked_shl, checked_sub,
    ensure_supported_traversal_frame, plane_range_for_tree_type, plane_subsampling,
    root_partition_call, symbol_decoder_for_work_unit,
};

pub(super) const MI_SIZE: usize = 4;
const LR_BANK_SIZE: usize = 4;
const WIENER_NS_LUMA_COEFFS: usize = 16;
pub(super) const WIENER_NS_CHROMA_COEFFS: usize = 18;
const WIENER_NS_SHORT_COEFFS: usize = 6;
const WIENER_NS_LUMA_SUBSETS: usize = 4;
const WIENER_NS_CHROMA_SUBSETS: usize = 3;
const WIENER_NS_TAPS_K: [[u8; WIENER_NS_CHROMA_COEFFS]; 2] = [
    [6, 6, 5, 5, 5, 5, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4],
    [6, 6, 5, 5, 5, 5, 5, 5, 5, 5, 4, 4, 4, 4, 4, 4, 4, 4],
];
const WIENER_NS_TAPS_MIN: [[i16; WIENER_NS_CHROMA_COEFFS]; 2] = [
    [
        -24, -24, -14, -14, -16, -16, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8,
    ],
    [
        -24, -24, -14, -14, -16, -16, -16, -16, -16, -16, -8, -8, -8, -8, -8, -8, -8, -8,
    ],
];
const WIENER_NS_TAPS_PRESENT: [[[bool; WIENER_NS_CHROMA_COEFFS]; WIENER_NS_LUMA_SUBSETS]; 2] = [
    [
        [
            true, true, true, true, true, true, false, false, false, false, false, false, false,
            false, false, false, false, false,
        ],
        [
            true, true, false, false, false, false, true, true, true, true, true, true, false,
            false, false, false, false, false,
        ],
        [
            true, true, true, true, true, true, true, true, true, true, true, true, false, false,
            false, false, false, false,
        ],
        [
            true, true, true, true, true, true, true, true, true, true, true, true, true, true,
            true, true, false, false,
        ],
    ],
    [
        [
            true, true, true, true, true, true, false, false, false, false, false, false, false,
            false, false, false, false, false,
        ],
        [
            true, true, true, true, true, true, true, true, true, true, false, false, false, false,
            false, false, false, false,
        ],
        [
            true, true, true, true, true, true, true, true, true, true, true, true, true, true,
            true, true, true, true,
        ],
        [false; WIENER_NS_CHROMA_COEFFS],
    ],
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WienerNsUnitFilterState {
    pub(super) bank_size: [usize; 3],
    bank_ptr: [usize; 3],
    bank: [[[i16; WIENER_NS_CHROMA_COEFFS]; LR_BANK_SIZE]; 3],
}

impl Default for WienerNsUnitFilterState {
    fn default() -> Self {
        let mut bank = [[[0i16; WIENER_NS_CHROMA_COEFFS]; LR_BANK_SIZE]; 3];
        for (plane, plane_bank) in bank.iter_mut().enumerate() {
            let plane_index = usize::from(plane > 0);
            for slot in plane_bank {
                for (j, coeff) in slot.iter_mut().enumerate() {
                    *coeff = wiener_ns_initial_tap_value(plane_index, j);
                }
            }
        }
        Self {
            bank_size: [0; 3],
            bank_ptr: [0; 3],
            bank,
        }
    }
}

pub(crate) fn consume_tile_loop_restoration_root_frontier(
    input: TilePartitionTraversalInput<'_, '_, '_>,
) -> Result<TileLoopRestorationRootFrontier, TilePartitionTraversalError> {
    let TilePartitionTraversalInput {
        work_unit,
        frame,
        context: _,
        limits,
    } = input;
    ensure_supported_traversal_frame(frame, true)?;

    let mut cdfs = work_unit.cdf().tile_cdfs().clone();
    let mut lr_activity = WienerNsLrUnitActivity::retaining_source_blocks();
    let mut symbols = symbol_decoder_for_work_unit(work_unit)?;
    let tile_bounds = TilePartitionBounds::from_work_unit(work_unit);
    let root = root_partition_call(work_unit, frame);
    limits.ensure(DecodeLimitName::MaxTilePartitionSteps, 1)?;
    if call_in_frame(frame, root) {
        read_loop_restoration_for_call(
            frame,
            root,
            tile_bounds,
            &mut cdfs,
            &mut symbols,
            &mut lr_activity,
            limits,
        )?;
    }
    *work_unit.cdf_mut().tile_cdfs_mut() = cdfs;
    Ok(TileLoopRestorationRootFrontier {
        symbol_count_after: symbols.symbol_count(),
        consumed_bits_after: symbols.consumed_bits().get(),
        lr_units_consumed: lr_activity.units_consumed,
        active_wiener_ns_units: lr_activity.active_units,
        selections: lr_activity.selections,
        active_source_blocks: lr_activity.active_source_blocks,
    })
}

pub(super) fn read_loop_restoration_for_call(
    frame: TilePartitionFrameFacts,
    call: TilePartitionCall,
    tile_bounds: TilePartitionBounds,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    lr_activity: &mut WienerNsLrUnitActivity,
    limits: DecodeLimits,
) -> Result<(), TilePartitionTraversalError> {
    if call.b_size != frame.sb_size {
        return Ok(());
    }
    let TilePartitionLoopRestorationState::Frame(lr) = frame.loop_restoration else {
        return Ok(());
    };
    let w = call.b_size.num_4x4_wide()?;
    let h = call.b_size.num_4x4_high()?;
    let (plane_start, plane_end) = plane_range_for_tree_type(call.tree_type, frame.num_planes);
    for plane in plane_start..plane_end.min(3) {
        let tool = lr.plane_tool[plane];
        if tool == TilePartitionLoopRestorationPlaneTool::None {
            continue;
        }
        read_lr_units_for_plane(
            plane,
            tool,
            lr.unit_size[plane],
            lr.frame_filters_on[plane],
            frame,
            call,
            tile_bounds,
            w,
            h,
            cdfs,
            symbols,
            lr_activity,
            limits,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn read_lr_units_for_plane(
    plane: usize,
    tool: TilePartitionLoopRestorationPlaneTool,
    unit_size: usize,
    frame_filters_on: bool,
    frame: TilePartitionFrameFacts,
    call: TilePartitionCall,
    tile_bounds: TilePartitionBounds,
    w: usize,
    h: usize,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    lr_activity: &mut WienerNsLrUnitActivity,
    limits: DecodeLimits,
) -> Result<(), TilePartitionTraversalError> {
    if unit_size == 0 {
        return Err(
            TilePartitionTraversalError::InvalidLoopRestorationUnitSize { plane, unit_size },
        );
    }
    let (sub_x, sub_y) = plane_subsampling(frame, plane);
    let sample_step_x = MI_SIZE >> sub_x;
    let sample_step_y = MI_SIZE >> sub_y;

    let mi_cols = checked_sub(
        "lr_mi_cols",
        tile_bounds.mi_col_end,
        tile_bounds.mi_col_start,
    )?;
    let mi_rows = checked_sub(
        "lr_mi_rows",
        tile_bounds.mi_row_end,
        tile_bounds.mi_row_start,
    )?;
    let frame_cols = checked_mul_shifted("lr_frame_cols", mi_cols, MI_SIZE, sub_x)?;
    let frame_rows = checked_mul_shifted("lr_frame_rows", mi_rows, MI_SIZE, sub_y)?;
    let lr_row_offset =
        checked_mul_shifted("lr_row_offset", tile_bounds.mi_row_start, MI_SIZE, sub_y)? / unit_size;
    let lr_col_offset =
        checked_mul_shifted("lr_col_offset", tile_bounds.mi_col_start, MI_SIZE, sub_x)? / unit_size;
    let c = checked_sub("lr_c", call.c, tile_bounds.mi_col_start)?;
    let r = checked_sub("lr_r", call.r, tile_bounds.mi_row_start)?;

    let unit_rows = count_units_in_frame(unit_size, frame_rows)?;
    let unit_cols = count_units_in_frame(unit_size, frame_cols)?;
    let unit_row_start = ceil_unit_index(
        checked_mul("lr_unit_row_start", r, sample_step_y)?,
        unit_size,
    )?;
    let unit_col_start = ceil_unit_index(
        checked_mul("lr_unit_col_start", c, sample_step_x)?,
        unit_size,
    )?;
    let unit_row_end = unit_rows.min(ceil_unit_index(
        checked_mul(
            "lr_unit_row_end",
            checked_add("lr_r_end", r, h)?,
            sample_step_y,
        )?,
        unit_size,
    )?);
    let unit_col_end = unit_cols.min(ceil_unit_index(
        checked_mul(
            "lr_unit_col_end",
            checked_add("lr_c_end", c, w)?,
            sample_step_x,
        )?,
        unit_size,
    )?);

    for unit_row in unit_row_start..unit_row_end {
        for unit_col in unit_col_start..unit_col_end {
            let unit_row = checked_add("lr_unit_row", unit_row, lr_row_offset)?;
            let unit_col = checked_add("lr_unit_col", unit_col, lr_col_offset)?;
            let restoration_type = match tool {
                TilePartitionLoopRestorationPlaneTool::None => LrUnitRestorationType::None,
                TilePartitionLoopRestorationPlaneTool::WienerNs => read_wiener_ns_lr_unit(
                    plane,
                    frame_filters_on,
                    unit_row,
                    unit_col,
                    cdfs,
                    symbols,
                    lr_activity,
                    limits,
                )?,
                TilePartitionLoopRestorationPlaneTool::PcWiener => {
                    read_pc_wiener_lr_unit(plane, unit_row, unit_col, cdfs, symbols, lr_activity)?
                }
                TilePartitionLoopRestorationPlaneTool::Switchable => {
                    read_switchable_lr_unit(plane, unit_row, unit_col, cdfs, symbols, lr_activity)?
                }
            };
            if restoration_type.is_active() {
                record_active_wiener_ns_source_blocks_for_unit(
                    LrSourceBlockDerivation {
                        restoration_type,
                        plane,
                        unit_size,
                        unit_row,
                        unit_col,
                        frame,
                        tile_bounds,
                        sub_x,
                        sub_y,
                    },
                    limits,
                    lr_activity,
                )?;
            }
        }
    }
    Ok(())
}

fn read_pc_wiener_lr_unit(
    plane: usize,
    unit_row: usize,
    unit_col: usize,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    lr_activity: &mut WienerNsLrUnitActivity,
) -> Result<LrUnitRestorationType, TilePartitionTraversalError> {
    let use_pc_wiener = cdfs
        .with_row_mut(super::cdf::TileCdfSelector::UsePcWiener, |row| {
            symbols.read_symbol(row)
        })??
        .get()
        != 0;
    let restoration_type = if use_pc_wiener {
        LrUnitRestorationType::PcWiener
    } else {
        LrUnitRestorationType::None
    };
    lr_activity.record(plane, unit_row, unit_col, restoration_type)?;
    Ok(restoration_type)
}

fn read_switchable_lr_unit(
    plane: usize,
    unit_row: usize,
    unit_col: usize,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    lr_activity: &mut WienerNsLrUnitActivity,
) -> Result<LrUnitRestorationType, TilePartitionTraversalError> {
    for (tool, restoration_type) in [LrUnitRestorationType::None, LrUnitRestorationType::PcWiener]
        .into_iter()
        .enumerate()
    {
        let found = cdfs
            .with_row_mut(
                super::cdf::TileCdfSelector::FlexRestorationType { tool, plane },
                |row| symbols.read_symbol(row),
            )??
            .get()
            != 0;
        if found {
            lr_activity.record(plane, unit_row, unit_col, restoration_type)?;
            return Ok(restoration_type);
        }
    }
    let restoration_type = LrUnitRestorationType::WienerNonsep;
    lr_activity.record(plane, unit_row, unit_col, restoration_type)?;
    Ok(restoration_type)
}

#[allow(clippy::too_many_arguments)]
fn read_wiener_ns_lr_unit(
    plane: usize,
    frame_filters_on: bool,
    unit_row: usize,
    unit_col: usize,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    lr_activity: &mut WienerNsLrUnitActivity,
    limits: DecodeLimits,
) -> Result<LrUnitRestorationType, TilePartitionTraversalError> {
    let use_wiener_ns = cdfs
        .with_row_mut(super::cdf::TileCdfSelector::UseWienerNs, |row| {
            symbols.read_symbol(row)
        })??
        .get()
        != 0;
    let restoration_type = if use_wiener_ns {
        LrUnitRestorationType::WienerNonsep
    } else {
        LrUnitRestorationType::None
    };
    lr_activity.record(plane, unit_row, unit_col, restoration_type)?;
    if use_wiener_ns && !frame_filters_on {
        let filter =
            read_wiener_ns_unit_filter(plane, cdfs, symbols, &mut lr_activity.unit_filter_state)?;
        lr_activity.record_unit_filter(
            WienerNsLrUnitFilter {
                plane,
                unit_row,
                unit_col,
                coeff_count: wiener_ns_coeff_count(plane),
                coeffs: filter,
            },
            limits,
        )?;
    }
    Ok(restoration_type)
}
pub(super) fn read_wiener_ns_unit_filter(
    plane: usize,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    state: &mut WienerNsUnitFilterState,
) -> Result<[i16; WIENER_NS_CHROMA_COEFFS], TilePartitionTraversalError> {
    let merged = read_wiener_ns_raw_literal(symbols, 1)? != 0;
    let previous_bank_size = state.bank_size[plane];
    let mut ref_from_last = 0usize;
    while ref_from_last < previous_bank_size.saturating_sub(1) {
        let use_bank = read_wiener_ns_raw_literal(symbols, 1)? != 0;
        if use_bank {
            break;
        }
        ref_from_last = checked_add("wiener_ns_bank_ref", ref_from_last, 1)?;
    }
    if merged {
        if state.bank_size[plane] == 0 {
            let coeffs = state.bank[plane][0];
            add_wiener_ns_unit_filter_to_bank(state, plane, coeffs)?;
            return Ok(coeffs);
        }
        let ref_index = wiener_ns_bank_ref_index(state, plane, ref_from_last)?;
        return Ok(state.bank[plane][ref_index]);
    }

    let ref_index = wiener_ns_bank_ref_index(state, plane, ref_from_last)?;
    let ref_coeffs = state.bank[plane][ref_index];
    let subset = read_wiener_ns_subset_symbol(plane, cdfs, symbols)?;
    let wiener_ns_uv_sym = if plane > 0 && subset > 0 {
        cdfs.with_row_mut(super::cdf::TileCdfSelector::WienerNsUvSym, |row| {
            symbols.read_symbol(row)
        })??
        .get()
            != 0
    } else {
        false
    };

    let plane_index = usize::from(plane > 0);
    let n_coeffs = wiener_ns_coeff_count(plane);
    let mut coeffs = [0i16; WIENER_NS_CHROMA_COEFFS];
    let mut j = 0usize;
    while j < n_coeffs {
        if WIENER_NS_TAPS_PRESENT[plane_index][subset][j] {
            let min = WIENER_NS_TAPS_MIN[plane_index][j];
            let ref_symb = ref_coeffs[j].checked_sub(min).ok_or(
                TilePartitionTraversalError::CoordinateUnderflow {
                    coordinate: "wiener_ns_ref_symb",
                    base: ref_coeffs[j] as usize,
                    offset: min.unsigned_abs() as usize,
                },
            )?;
            let decoded = read_wiener_ns_4part_wref(
                WIENER_NS_TAPS_K[plane_index][j],
                usize::try_from(ref_symb).map_err(|_| {
                    TilePartitionTraversalError::CoordinateOverflow {
                        coordinate: "wiener_ns_ref_symb",
                        base: ref_symb as usize,
                        offset: 0,
                    }
                })?,
                cdfs,
                symbols,
            )?;
            let value = i32::try_from(decoded).map_err(|_| {
                TilePartitionTraversalError::CoordinateOverflow {
                    coordinate: "wiener_ns_coeff",
                    base: decoded,
                    offset: 0,
                }
            })? + i32::from(min);
            coeffs[j] = i16::try_from(value).map_err(|_| {
                TilePartitionTraversalError::CoordinateOverflow {
                    coordinate: "wiener_ns_coeff",
                    base: decoded,
                    offset: min.unsigned_abs() as usize,
                }
            })?;
        }
        if plane > 0 && j >= WIENER_NS_SHORT_COEFFS && wiener_ns_uv_sym {
            let next_j = checked_add("wiener_ns_coeff_index", j, 1)?;
            if next_j < n_coeffs {
                coeffs[next_j] = coeffs[j];
            }
            j = checked_add("wiener_ns_coeff_index", j, 2)?;
        } else {
            j = checked_add("wiener_ns_coeff_index", j, 1)?;
        }
    }
    add_wiener_ns_unit_filter_to_bank(state, plane, coeffs)?;
    Ok(coeffs)
}

fn read_wiener_ns_subset_symbol(
    plane: usize,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<usize, TilePartitionTraversalError> {
    let num_subsets = if plane > 0 {
        WIENER_NS_CHROMA_SUBSETS
    } else {
        WIENER_NS_LUMA_SUBSETS
    };
    let mut subset = 0usize;
    while subset < num_subsets.saturating_sub(1) {
        let wiener_ns_length = cdfs.with_row_mut(
            super::cdf::TileCdfSelector::WienerNsLength {
                plane_ctx: plane.min(1),
            },
            |row| symbols.read_symbol(row),
        )??;
        if wiener_ns_length.get() == 0 {
            break;
        }
        subset = checked_add("wiener_ns_subset", subset, 1)?;
    }
    Ok(subset)
}

const fn wiener_ns_coeff_count(plane: usize) -> usize {
    if plane > 0 {
        WIENER_NS_CHROMA_COEFFS
    } else {
        WIENER_NS_LUMA_COEFFS
    }
}

const fn wiener_ns_initial_tap_value(plane_index: usize, j: usize) -> i16 {
    WIENER_NS_TAPS_MIN[plane_index][j] + ((1i16 << WIENER_NS_TAPS_K[plane_index][j]) >> 1)
}

fn wiener_ns_bank_ref_index(
    state: &WienerNsUnitFilterState,
    plane: usize,
    ref_from_last: usize,
) -> Result<usize, TilePartitionTraversalError> {
    let bank_size = state.bank_size[plane];
    if bank_size == 0 {
        return Ok(0);
    }
    if ref_from_last >= bank_size {
        return Err(TilePartitionTraversalError::CoordinateOverflow {
            coordinate: "wiener_ns_bank_ref",
            base: ref_from_last,
            offset: bank_size,
        });
    }
    let ptr = state.bank_ptr[plane];
    if ptr < ref_from_last {
        checked_add("wiener_ns_bank_ref_index", ptr, LR_BANK_SIZE)
            .and_then(|base| checked_sub("wiener_ns_bank_ref_index", base, ref_from_last))
    } else {
        checked_sub("wiener_ns_bank_ref_index", ptr, ref_from_last)
    }
}

fn add_wiener_ns_unit_filter_to_bank(
    state: &mut WienerNsUnitFilterState,
    plane: usize,
    coeffs: [i16; WIENER_NS_CHROMA_COEFFS],
) -> Result<(), TilePartitionTraversalError> {
    if state.bank_size[plane] < LR_BANK_SIZE {
        state.bank_ptr[plane] = state.bank_size[plane];
        state.bank_size[plane] = checked_add("wiener_ns_bank_size", state.bank_size[plane], 1)?;
    } else {
        state.bank_ptr[plane] =
            checked_add("wiener_ns_bank_ptr", state.bank_ptr[plane], 1)? % LR_BANK_SIZE;
    }
    state.bank[plane][state.bank_ptr[plane]] = coeffs;
    Ok(())
}

fn read_wiener_ns_4part_wref(
    k: u8,
    ref_symb: usize,
    cdfs: &mut super::cdf::TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<usize, TilePartitionTraversalError> {
    let wiener_ns_base = cdfs
        .with_row_mut(super::cdf::TileCdfSelector::WienerNsBase, |row| {
            symbols.read_symbol(row)
        })??
        .get() as usize;
    let nsymb_bits = usize::from(k);
    let part_bits = [
        checked_sub("wiener_ns_4part_bits", nsymb_bits, 3)?,
        checked_sub("wiener_ns_4part_bits", nsymb_bits, 3)?,
        checked_sub("wiener_ns_4part_bits", nsymb_bits, 2)?,
        checked_sub("wiener_ns_4part_bits", nsymb_bits, 1)?,
    ];
    let part_offsets = [
        0usize,
        checked_shl("wiener_ns_4part_offset", 1, part_bits[0])?,
        checked_shl("wiener_ns_4part_offset", 1, part_bits[2])?,
        checked_shl("wiener_ns_4part_offset", 1, part_bits[3])?,
    ];
    let bits =
        *part_bits
            .get(wiener_ns_base)
            .ok_or(TilePartitionTraversalError::CoordinateOverflow {
                coordinate: "wiener_ns_4part_part",
                base: wiener_ns_base,
                offset: 0,
            })?;
    let bits =
        u32::try_from(bits).map_err(|_| TilePartitionTraversalError::CoordinateOverflow {
            coordinate: "wiener_ns_4part_bits",
            base: usize::from(k),
            offset: 0,
        })?;
    let literal = usize::try_from(read_wiener_ns_raw_literal(symbols, bits)?).map_err(|_| {
        TilePartitionTraversalError::CoordinateOverflow {
            coordinate: "wiener_ns_4part_literal",
            base: usize::from(k),
            offset: 0,
        }
    })?;
    let offset = *part_offsets.get(wiener_ns_base).ok_or(
        TilePartitionTraversalError::CoordinateOverflow {
            coordinate: "wiener_ns_4part_part",
            base: wiener_ns_base,
            offset: 0,
        },
    )?;
    let symbol = checked_add("wiener_ns_4part_symbol", literal, offset)?;
    let n = checked_shl("wiener_ns_4part_range", 1, nsymb_bits)?;
    inverse_recenter_finite_nonneg(n, ref_symb, symbol)
}

fn read_wiener_ns_raw_literal(
    symbols: &mut SymbolDecoder<'_>,
    bits: u32,
) -> Result<u32, TilePartitionTraversalError> {
    let value = symbols.read_literal(bits)?;
    Ok(value)
}

fn inverse_recenter_finite_nonneg(
    n: usize,
    r: usize,
    v: usize,
) -> Result<usize, TilePartitionTraversalError> {
    if n == 0 || r >= n || v >= n {
        return Err(TilePartitionTraversalError::CoordinateOverflow {
            coordinate: "wiener_ns_recenter",
            base: r,
            offset: v,
        });
    }
    if checked_mul("wiener_ns_recenter", r, 2)? <= n {
        inverse_recenter_nonneg(r, v)
    } else {
        let mirrored_r = checked_sub("wiener_ns_recenter", n - 1, r)?;
        let mirrored = inverse_recenter_nonneg(mirrored_r, v)?;
        checked_sub("wiener_ns_recenter", n - 1, mirrored)
    }
}

fn inverse_recenter_nonneg(r: usize, v: usize) -> Result<usize, TilePartitionTraversalError> {
    if v > checked_mul("wiener_ns_recenter", r, 2)? {
        return Ok(v);
    }
    if v & 1 == 0 {
        checked_add("wiener_ns_recenter", v >> 1, r)
    } else {
        checked_sub(
            "wiener_ns_recenter",
            r,
            checked_add("wiener_ns_recenter", v, 1)? >> 1,
        )
    }
}
