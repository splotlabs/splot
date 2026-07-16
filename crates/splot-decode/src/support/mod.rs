// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Decode support-tier capability gates and local limit helpers.

pub(crate) mod capability;
pub(crate) mod pipeline_limits;
pub(crate) mod reusable_scratch;

pub(crate) fn rect_index(
    row: usize,
    col: usize,
    origin: (usize, usize),
    size: (usize, usize),
) -> Option<usize> {
    let row = row.checked_sub(origin.0)?;
    let col = col.checked_sub(origin.1)?;
    if row >= size.0 || col >= size.1 {
        return None;
    }
    row.checked_mul(size.1)?.checked_add(col)
}
