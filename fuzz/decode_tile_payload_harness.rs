// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Fuzzing-only harnesses for crate-private decoder frontiers.
//!
//! This module is compiled only with the `fuzzing` feature. It exposes compact
//! primitive outcome data for the external `fuzz/` crate without making
//! crate-private tile-payload implementation types part of the production API.
//!
//! Feature tracking: `CONF-TILE-PAYLOAD-DECODE-FUZZ`.

use splot_core::headers::tile_group::parse_tile_group_framing;
use splot_core::span::ByteOffset;
use splot_core::symbol::CdfUpdateMode;
use splot_core::types::ObuType;

use crate::bitstream::tile_payload::{
    TileFrameFacts, TileGridFacts, TilePayloadBoundaryInput, plan_tile_payload_boundary,
};
use crate::{DecodeLimitThreshold, DecodeLimits};

const MI_COL_STARTS: [u32; 2] = [0, 16];
const MI_ROW_STARTS: [u32; 2] = [0, 16];
const GOOD_PAYLOAD_FLAG: u8 = 0b0000_0100;
const MAX_TILE_PAYLOAD_BYTES: usize = 128;
const MAX_GOOD_TILE_MUTATIONS: usize = 8;
const GOOD_TILE_PAYLOAD: [u8; 2] = [0x12, 0xFB];
/// Outcome from one tile-payload fuzzing harness call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TilePayloadFuzzOutcome {
    /// Boundary outcome when the tile-payload boundary accepted the input.
    pub boundary: Option<TilePayloadBoundaryFuzzOutcome>,
    /// Whether the harness reached a typed error.
    pub typed_error: bool,
}

/// Compact outcome from a successful tile-payload boundary plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TilePayloadBoundaryFuzzOutcome {
    /// Planned tile number.
    pub tile_num: u32,
    /// Planned tile row.
    pub tile_row: u32,
    /// Planned tile column.
    pub tile_col: u32,
    /// Number of bytes exposed to the future tile decoder.
    pub tile_bytes_len: usize,
    /// Planned `tileSize`.
    pub tile_size: u64,
    /// Bits consumed by §8.2.2 `init_symbol(tileSize)`.
    pub symbol_consumed_bits: u64,
    /// Signed `SymbolMaxBits` after `init_symbol(tileSize)`.
    pub symbol_max_bits: i64,
    /// Whether the symbol decoder boundary enables CDF updates.
    pub symbol_cdf_update_enabled: bool,
    /// Whether the attached tile CDF boundary enables CDF updates.
    pub cdf_update_enabled: bool,
}

/// Runs one bounded tile-payload fuzzing case.
#[must_use]
pub fn run_tile_payload_decode_fuzz_case(data: &[u8]) -> TilePayloadFuzzOutcome {
    let Some((&flags, rest)) = data.split_first() else {
        return typed_error();
    };
    let Some((&payload_len_seed, rest)) = rest.split_first() else {
        return typed_error();
    };
    let Some((&limit_seed, rest)) = rest.split_first() else {
        return typed_error();
    };
    let Some((&detail_seed, rest)) = rest.split_first() else {
        return typed_error();
    };

    let payload_storage;
    let payload = if flags & GOOD_PAYLOAD_FLAG == 0 {
        let len = usize::from(payload_len_seed)
            .min(MAX_TILE_PAYLOAD_BYTES)
            .min(rest.len());
        &rest[..len]
    } else {
        payload_storage = mutated_good_tile_payload(detail_seed, rest);
        payload_storage.as_slice()
    };

    let limits = fuzz_limits(limit_seed);
    let tile_size_bytes = 1 + u32::from((flags >> 5) & 0b0000_0011);
    let mode = (flags >> 3) & 0b0000_0011;
    let (tg_start, tg_end, framing_is_bridge) = match mode {
        0 => (0, 0, false),
        1 => (0, 1, false),
        2 => (1, 0, false),
        _ => (0, 1, true),
    };
    let framing = parse_tile_group_framing(
        payload,
        tg_start,
        tg_end,
        tile_size_bytes,
        framing_is_bridge,
    );
    let frame = frame_facts(flags, detail_seed);
    let grid = TileGridFacts::new(1, 1, &MI_COL_STARTS, &MI_ROW_STARTS);
    let input =
        TilePayloadBoundaryInput::new(payload, ByteOffset::new(128), &framing, grid, frame, limits);

    let plan = match plan_tile_payload_boundary(&input) {
        Ok(plan) => plan,
        Err(error) => {
            let _ = error.to_string();
            return typed_error();
        }
    };
    let boundary = boundary_outcome(&plan);
    TilePayloadFuzzOutcome {
        boundary,
        typed_error: false,
    }
}

fn typed_error() -> TilePayloadFuzzOutcome {
    TilePayloadFuzzOutcome {
        boundary: None,
        typed_error: true,
    }
}

fn mutated_good_tile_payload(detail_seed: u8, bytes: &[u8]) -> Vec<u8> {
    let mut payload = GOOD_TILE_PAYLOAD.to_vec();
    let mutation_count = usize::from(detail_seed).min(MAX_GOOD_TILE_MUTATIONS);
    for chunk in bytes.chunks_exact(2).take(mutation_count) {
        let index = usize::from(chunk[0]) % payload.len();
        payload[index] = chunk[1];
    }
    payload
}

fn fuzz_limits(seed: u8) -> DecodeLimits {
    let max = DecodeLimitThreshold::Max;
    let tile_payload_bytes = 1 + u64::from(seed & 0b0111_1111);
    let tile_count = 1 + u64::from((seed >> 6) & 0b0000_0011);
    let partition_steps = 1 + u64::from(seed & 0b0011_1111);

    DecodeLimits::DEFAULT
        .with_max_tile_payload_bytes(max(tile_payload_bytes.min(MAX_TILE_PAYLOAD_BYTES as u64)))
        .with_max_tile_count(max(tile_count))
        .with_max_tile_partition_steps(max(partition_steps.min(64)))
        .with_max_luma_samples_per_frame(max(256))
}

fn frame_facts(flags: u8, detail_seed: u8) -> TileFrameFacts {
    let obu_type = if detail_seed & 0b1000_0000 == 0 {
        ObuType::ClosedLoopKey
    } else {
        ObuType::RasFrame
    };
    TileFrameFacts::new(
        obu_type,
        detail_seed & 0b0100_0000 == 0,
        detail_seed & 0b0001_0000 == 0,
        255,
        flags & 0b0000_0001 != 0,
    )
}

fn boundary_outcome(
    plan: &crate::bitstream::tile_payload::DecodeTilePayloadPlan<'_>,
) -> Option<TilePayloadBoundaryFuzzOutcome> {
    let [unit] = plan.work_units() else {
        return None;
    };
    Some(TilePayloadBoundaryFuzzOutcome {
        tile_num: unit.tile_num(),
        tile_row: unit.tile_row(),
        tile_col: unit.tile_col(),
        tile_bytes_len: unit.tile_bytes().len(),
        tile_size: unit.tile_size(),
        symbol_consumed_bits: unit.symbol().consumed_bits(),
        symbol_max_bits: unit.symbol().symbol_max_bits(),
        symbol_cdf_update_enabled: unit.symbol().cdf_update_mode() == CdfUpdateMode::Enabled,
        cdf_update_enabled: unit.cdf().update_mode() == CdfUpdateMode::Enabled,
    })
}
