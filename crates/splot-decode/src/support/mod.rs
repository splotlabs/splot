// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Decode support-tier capability gates and local limit helpers.

pub(crate) mod capability;
pub(crate) mod decode_buffers;
pub(crate) mod pipeline_limits;
pub(crate) mod reusable_scratch;

/// Fills one short per-block run of a mode-info grid without a `memset` call.
///
/// Runs are block widths in 4x4 units, at most 32. Two overlapping fixed-size
/// stores cover every length from the store width to twice it.
#[inline]
pub(crate) fn fill_mi_run<T: Copy>(run: &mut [T], value: T) {
    fn ends<T: Copy, const N: usize>(run: &mut [T], value: T) {
        let len = run.len();
        run[..N].copy_from_slice(&[value; N]);
        run[len - N..].copy_from_slice(&[value; N]);
    }
    match run.len() {
        0 => {}
        len @ 1..=3 => {
            run[0] = value;
            run[len / 2] = value;
            run[len - 1] = value;
        }
        4..=7 => ends::<T, 4>(run, value),
        8..=15 => ends::<T, 8>(run, value),
        16..=32 => ends::<T, 16>(run, value),
        _ => run.fill(value),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn mi_run_fill_writes_exactly_the_run() {
        for len in 0..=40 {
            let mut cells = [0u8; 44];
            super::fill_mi_run(&mut cells[2..2 + len], 7);
            for (index, &cell) in cells.iter().enumerate() {
                let inside = (2..2 + len).contains(&index);
                assert_eq!(cell, if inside { 7 } else { 0 }, "len {len} index {index}");
            }
        }
    }
}
