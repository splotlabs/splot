// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::delta_q_is_coded_for_block;

const BLOCK_64X64: usize = 12;
const BLOCK_128X128: usize = 15;

#[test]
fn skipped_full_superblock_omits_delta_q() {
    assert!(!delta_q_is_coded_for_block(BLOCK_128X128, 32, true).unwrap());
}

#[test]
fn non_skipped_or_subdivided_superblock_codes_delta_q() {
    assert!(delta_q_is_coded_for_block(BLOCK_128X128, 32, false).unwrap());
    assert!(delta_q_is_coded_for_block(BLOCK_64X64, 32, true).unwrap());
}
