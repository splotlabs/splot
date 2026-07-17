// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::panic)]

use super::super::coeff_state::TileCoeffStateError;
use super::base_symbol::CoeffBaseSymbolRead;
use super::branch::{NonZeroCoeffBlockStart, NonZeroCoeffBlockStartInput};
use super::level_state::{CoeffLevelStateWriteError, apply_nonzero_coeff_base_levels};
use super::scan_walk::{NonZeroCoeffScanWalk, walk_nonzero_coeff_scan};
use super::test_support::setup_start_with_input;
use super::*;

const PAYLOAD_SUFFIXES: [[u8; 3]; 4] = [
    [0x00, 0x00, 0x80],
    [0xff, 0x00, 0x80],
    [0x55, 0xaa, 0x80],
    [0xff, 0xff, 0x80],
];
const SCAN: [u16; 4] = [0, 8, 1, 9];
const ALT_SCAN: [u16; 4] = [0, 8, 9, 1];
const LARGE_SCAN: [u16; 4] = [0, 1, 8, 63];

fn setup_start(payload: &[u8], w4: usize, h4: usize) -> Option<NonZeroCoeffBlockStart> {
    let (_, _, start) = setup_start_with_input(
        payload,
        NonZeroCoeffBlockStartInput {
            block: AllZeroCoeffBlockInput {
                plane: 0,
                x4: 0,
                y4: 0,
                w4,
                h4,
            },
            eob: NonZeroCoeffEobContextInput {
                plane: 0,
                is_inter: false,
                tx_width_log2: 3,
                tx_height_log2: 3,
                coeff_cdf_q_ctx: 0,
            },
        },
    )?;
    Some(start)
}

fn setup_reads<'scan>(
    payload: &[u8],
    scan: &'scan [u16],
    w4: usize,
    h4: usize,
) -> Option<(
    NonZeroCoeffBlockStart,
    NonZeroCoeffScanWalk<'scan>,
    Vec<CoeffBaseSymbolRead>,
)> {
    let start = setup_start(payload, w4, h4)?;
    if start.eob_read().eob().eob() != scan.len() {
        return None;
    }
    let walk = walk_nonzero_coeff_scan(&start, scan).ok()?;
    let reads = walk
        .entries()
        .enumerate()
        .map(|(index, entry)| CoeffBaseSymbolRead::for_test(entry, index as u32 + 1))
        .collect();
    Some((start, walk, reads))
}

fn find_payload() -> [u8; 5] {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            for suffix in PAYLOAD_SUFFIXES {
                let payload = [first, second, suffix[0], suffix[1], suffix[2]];
                if setup_start(&payload, 2, 2)
                    .is_some_and(|start| start.eob_read().eob().eob() == SCAN.len())
                {
                    return payload;
                }
            }
        }
    }
    panic!("no coefficient level state payload found");
}

#[test]
fn coefficient_level_state_write_sets_levels_from_scan_entries() {
    let payload = find_payload();
    let (start, walk, reads) = setup_reads(&payload, &SCAN, 2, 2).unwrap();
    let eob_read = start.eob_read();

    let state = apply_nonzero_coeff_base_levels(start, &walk, &reads).unwrap();

    assert_eq!(state.eob_read(), eob_read);
    let block = state.block();
    let mut expected = vec![0u8; block.level().len()];
    for read in &reads {
        let entry = read.entry();
        expected[entry.row() * block.width() + entry.col()] = u8::try_from(read.level()).unwrap();
        assert_eq!(
            block.level_at(entry.row(), entry.col()).unwrap(),
            read.level()
        );
    }
    assert_eq!(block.level(), expected.as_slice());
    assert!(block.quant_sign().iter().all(|sign| *sign == 0));
    assert!(block.quant().iter().all(|quant| *quant == 0));
}

#[test]
fn coefficient_level_state_write_rejects_read_count_mismatch() {
    let payload = find_payload();
    let (start, walk, mut reads) = setup_reads(&payload, &SCAN, 2, 2).unwrap();
    let before = start.clone();
    reads.pop();

    let err = apply_nonzero_coeff_base_levels(start.clone(), &walk, &reads).unwrap_err();

    assert!(matches!(
        err,
        CoeffLevelStateWriteError::ReadCountMismatch {
            reads: 3,
            entries: 4
        }
    ));
    assert_eq!(start, before);
    assert!(start.block().level().iter().all(|level| *level == 0));
}

#[test]
fn coefficient_level_state_write_rejects_scan_entry_mismatch() {
    let payload = find_payload();
    let (start, _walk, reads) = setup_reads(&payload, &SCAN, 2, 2).unwrap();
    let alt_walk = walk_nonzero_coeff_scan(&start, &ALT_SCAN).unwrap();

    let err = apply_nonzero_coeff_base_levels(start, &alt_walk, &reads).unwrap_err();

    assert!(matches!(
        err,
        CoeffLevelStateWriteError::ScanEntryMismatch { index: 0, .. }
    ));
}

#[test]
fn coefficient_level_state_write_rejects_mismatched_block_geometry() {
    let payload = find_payload();
    let (_large_start, large_walk, reads) = setup_reads(&payload, &LARGE_SCAN, 2, 2).unwrap();
    let small_start = setup_start(&payload, 1, 1).unwrap();

    let err = apply_nonzero_coeff_base_levels(small_start, &large_walk, &reads).unwrap_err();

    assert!(matches!(
        err,
        CoeffLevelStateWriteError::State(TileCoeffStateError::TransformCoordinateOutOfBounds {
            row: 7,
            col: 7,
            height: 4,
            width: 4
        })
    ));
}
