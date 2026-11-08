// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Decode support-tier capability gates and local limit helpers.

pub(crate) mod capability;
pub(crate) mod decode_buffers;
pub(crate) mod pipeline_limits;
pub(crate) mod reusable_scratch;

/// `range.step_by(step)` by addition: `step_by` divides by a runtime step.
pub(crate) fn step_range(
    range: core::ops::Range<usize>,
    step: usize,
) -> impl Iterator<Item = usize> {
    core::iter::successors(Some(range.start), move |at| at.checked_add(step.max(1)))
        .take_while(move |&at| at < range.end)
}

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

/// The OR of one short per-block run of a mode-info grid, as overlapping
/// word loads (the OR does not mind bytes read twice).
#[allow(clippy::inline_always, reason = "measured: per-block context read")]
#[inline(always)]
pub(crate) fn or_mi_run(run: &[u8]) -> u8 {
    fn word<const N: usize>(bytes: Option<&[u8; N]>) -> u64 {
        let mut word = [0u8; 8];
        if let Some(bytes) = bytes {
            word[..N].copy_from_slice(bytes);
        }
        u64::from_le_bytes(word)
    }
    let len = run.len();
    let word = match len {
        0 => 0,
        1..=3 => u64::from(run[0] | run[len / 2] | run[len - 1]),
        4..=7 => word::<4>(run.first_chunk()) | word::<4>(run.last_chunk()),
        8..=15 => word::<8>(run.first_chunk()) | word::<8>(run.last_chunk()),
        16..=32 => {
            let head = run.get(8..).unwrap_or_default();
            let tail = run.get(..len - 8).unwrap_or_default();
            word::<8>(run.first_chunk())
                | word::<8>(head.first_chunk())
                | word::<8>(tail.last_chunk())
                | word::<8>(run.last_chunk())
        }
        _ => u64::from(run.iter().fold(0, |acc, &v| acc | v)),
    };
    let word = word | (word >> 32);
    let word = word | (word >> 16);
    (word | (word >> 8)) as u8
}

#[cfg(test)]
mod tests {
    #[test]
    fn mi_run_or_covers_exactly_the_run() {
        for len in 0..=40 {
            for hot in 0..len {
                let mut cells = [0u8; 44];
                cells[2 + hot] = 0x21;
                cells[1] = 0x80;
                cells[2 + len] = 0x80;
                assert_eq!(
                    super::or_mi_run(&cells[2..2 + len]),
                    0x21,
                    "len {len} hot {hot}"
                );
            }
        }
    }

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
