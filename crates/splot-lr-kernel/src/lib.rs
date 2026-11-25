// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared Wiener tap geometry and 16-column restoration runs.

#[cfg(target_arch = "x86_64")]
pub mod avx2;

/// The symmetric taps `(dy, dx)` paired with `(-dy, -dx)` of one filter.
pub trait TapGeometry {
    /// Tap reach in rows and columns.
    const RADIUS: usize;
    /// Pair `j` of the symmetric taps.
    const TAPS: &'static [(isize, isize)];
}

/// Both `pmaddwd` weight vectors of one pair over four cells.
pub type MaddWeights = [[i16; 16]; 2];

/// One class of a [`TapGeometry`] filter: the weight of each pair, then the
/// center weight.
pub type LaneClass = ([i16; 16], i16);

/// The classes of the 4-column cells of a run.
#[derive(Clone, Copy)]
pub enum Classes<'a> {
    /// One class for every cell.
    Uniform(&'a LaneClass),
    /// Cell `k` of the run takes `table[cells[k] % table.len()]`.
    Cells {
        /// Class index of each 4-column cell in the run.
        cells: &'a [u8],
        /// Prepared coefficient classes.
        table: &'a [LaneClass],
    },
}

impl<'a> Classes<'a> {
    /// The classes of the four cells of group `group` of the run.
    #[inline]
    pub fn group(self, group: usize) -> Option<[&'a LaneClass; 4]> {
        match self {
            Self::Uniform(class) => Some([class; 4]),
            Self::Cells { cells, table } => {
                let ids = cells.get(group.checked_mul(4)?..)?.first_chunk::<4>()?;
                let class = |id: u8| table.get(usize::from(id) % table.len().max(1));
                Some([
                    class(ids[0])?,
                    class(ids[1])?,
                    class(ids[2])?,
                    class(ids[3])?,
                ])
            }
        }
    }
}

/// Number of luma Wiener NS coefficient slots (`WIENER_NS_LUMA_COEFFS`) consumed
/// by AV2 § 7.20.3 `Wiener_Ns_Config_Y`.
pub const WIENER_NS_LUMA_COEFFS: usize = 16;

/// Number of luma Wiener NS taps (`WIENER_NS_TAPS_Y`) in AV2 § 7.20.3.
pub const WIENER_NS_LUMA_TAPS: usize = 32;

/// Maximum absolute § 7.20.3 `Wiener_Ns_Config_Y` tap offset in either axis.
///
/// Callers that pre-resolve § 7.20.2 source samples may materialize a window
/// extending this many samples beyond the output block on every side; the
/// filter never reads farther.
pub const WIENER_NS_LUMA_TAP_RADIUS: usize = 4;

/// AV2 § 7.20.3 `Wiener_Ns_Config_Y`: `(dy, dx, coeff_index)`.
pub const WIENER_NS_CONFIG_Y: [(isize, isize, usize); WIENER_NS_LUMA_TAPS] = [
    (1, 0, 0),
    (-1, 0, 0),
    (0, 1, 1),
    (0, -1, 1),
    (2, 0, 2),
    (-2, 0, 2),
    (0, 2, 3),
    (0, -2, 3),
    (1, 1, 4),
    (-1, -1, 4),
    (-1, 1, 5),
    (1, -1, 5),
    (2, 1, 6),
    (-2, -1, 6),
    (2, -1, 7),
    (-2, 1, 7),
    (1, 2, 8),
    (-1, -2, 8),
    (1, -2, 9),
    (-1, 2, 9),
    (3, 0, 10),
    (-3, 0, 10),
    (0, 3, 11),
    (0, -3, 11),
    (4, 0, 12),
    (-4, 0, 12),
    (0, 4, 13),
    (0, -4, 13),
    (3, 3, 14),
    (-3, -3, 14),
    (3, -3, 15),
    (-3, 3, 15),
];

/// First tap `(dy, dx, coeff_index)` of each § 7.20.3 `Wiener_Ns_Config_Y`
/// symmetric pair; the partner tap is `(-dy, -dx, coeff_index)`.
pub const WIENER_NS_CONFIG_Y_PAIRS: [(isize, isize, usize); WIENER_NS_LUMA_COEFFS] = {
    let mut pairs = [(0isize, 0isize, 0usize); WIENER_NS_LUMA_COEFFS];
    let mut j = 0;
    while j < WIENER_NS_LUMA_COEFFS {
        pairs[j] = WIENER_NS_CONFIG_Y[2 * j];
        j += 1;
    }
    pairs
};

/// The § 7.20.3 luma taps for the vector run.
pub struct LumaTaps;

impl TapGeometry for LumaTaps {
    const RADIUS: usize = WIENER_NS_LUMA_TAP_RADIUS;
    const TAPS: &'static [(isize, isize)] = &{
        let mut taps = [(0, 0); WIENER_NS_LUMA_COEFFS];
        let mut j = 0;
        while j < WIENER_NS_LUMA_COEFFS {
            taps[j] = (WIENER_NS_CONFIG_Y_PAIRS[j].0, WIENER_NS_CONFIG_Y_PAIRS[j].1);
            j += 1;
        }
        taps
    };
}

/// Runs the shared group loop over complete 16-column groups in `span`.
/// Each output row `r` uses source window rows `r..r + ROWS`. Each group
/// reads `REACH` source samples per row starting at its output column.
/// Returns `false` for invalid geometry or short source, output or class spans.
#[inline(always)]
pub fn filter_run_groups<T, O, const ROWS: usize, const REACH: usize>(
    window: &[&[T]],
    rows: usize,
    (x, end): (usize, usize),
    classes: Classes<'_>,
    (output, stride): (&mut [O], usize),
    weights_for: impl Fn([&LaneClass; 4]) -> [MaddWeights; 9],
    filter_row: impl Fn(&[&[T; REACH]; ROWS], &[MaddWeights; 9], &mut [O; 16]),
) -> bool {
    const MAX_ROWS: usize = 16;
    let span = end.saturating_sub(x) & !15;
    if span == 0 || rows == 0 {
        return true;
    }
    let (Some(halo_rows), Some(halo_cols)) = (ROWS.checked_sub(1), REACH.checked_sub(16)) else {
        return false;
    };
    if rows > MAX_ROWS.saturating_sub(halo_rows) {
        return false;
    }
    let need = rows + halo_rows;
    let Some(reach_end) = x
        .checked_add(span)
        .and_then(|end| end.checked_add(halo_cols))
    else {
        return false;
    };
    let Some(first) = window.first().and_then(|row| row.get(x..reach_end)) else {
        return false;
    };
    if window.len() < need {
        return false;
    }
    let mut source = [first; MAX_ROWS];
    for (slot, row) in source.iter_mut().zip(window).take(need) {
        match row.get(x..reach_end) {
            Some(row) => *slot = row,
            None => return false,
        }
    }
    let mut targets: [&mut [O]; MAX_ROWS] = Default::default();
    let mut rest = output;
    for (r, slot) in targets.iter_mut().enumerate().take(rows) {
        let skip = if r == 0 {
            Some(x)
        } else {
            stride.checked_sub(span)
        };
        let Some(after) = skip
            .and_then(|skip| rest.get_mut(skip..))
            .filter(|after| after.len() >= span)
        else {
            return false;
        };
        (*slot, rest) = after.split_at_mut(span);
    }
    let uniform = match classes {
        Classes::Uniform(class) => Some(weights_for([class; 4])),
        Classes::Cells { .. } => None,
    };
    for (index, col) in (0..span).step_by(16).enumerate() {
        let cell_weights;
        let weights = match &uniform {
            Some(weights) => weights,
            None => match classes.group(index) {
                Some(cells) => {
                    cell_weights = weights_for(cells);
                    &cell_weights
                }
                None => return false,
            },
        };
        let Some(head) = first.get(col..).and_then(<[T]>::first_chunk::<REACH>) else {
            return false;
        };
        let mut group = [head; MAX_ROWS];
        for (slot, row) in group.iter_mut().zip(&source).take(need) {
            match row.get(col..).and_then(<[T]>::first_chunk::<REACH>) {
                Some(row) => *slot = row,
                None => return false,
            }
        }
        for (r, target) in targets.iter_mut().enumerate().take(rows) {
            let (Some(rows_of), Some(target)) = (
                group.get(r..).and_then(<[_]>::first_chunk::<ROWS>),
                target.get_mut(col..).and_then(<[O]>::first_chunk_mut::<16>),
            ) else {
                return false;
            };
            filter_row(rows_of, weights, target);
        }
    }
    true
}
