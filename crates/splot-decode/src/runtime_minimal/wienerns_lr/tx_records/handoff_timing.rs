// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::time::Instant;

use crate::tile_payload::LumaCoeffBlock;

#[derive(Default)]
pub(super) struct HandoffTiming {
    pub(super) enabled: bool,
    pub(super) blocks: usize,
    pub(super) luma_blocks: usize,
    pub(super) chroma_blocks: usize,
    pub(super) skipped_blocks: usize,
    luma_records: usize,
    max_luma_records: usize,
    prelude_ns: u128,
    mode_ns: u128,
    tx_records_ns: u128,
    far_edge_ns: u128,
    residual_ns: u128,
    intrabc_state_ns: u128,
    luma_coeff_ns: u128,
    luma_sink_ns: u128,
    chroma_coeff_ns: u128,
    chroma_sink_ns: u128,
    decoded_luma_records: usize,
    decoded_chroma_groups: usize,
    luma_all_zero: usize,
    chroma_all_zero: usize,
}

impl HandoffTiming {
    pub(super) fn new() -> Self {
        Self {
            enabled: crate::timing::enabled(),
            ..Self::default()
        }
    }

    pub(super) fn start(&self) -> Option<Instant> {
        self.enabled.then(Instant::now)
    }

    fn elapsed_ns(started: Option<Instant>) -> u128 {
        started.map_or(0, |started| started.elapsed().as_nanos())
    }

    pub(super) fn add_prelude(&mut self, started: Option<Instant>) {
        if !self.enabled {
            return;
        }
        self.prelude_ns += Self::elapsed_ns(started);
    }

    pub(super) fn add_mode(&mut self, started: Option<Instant>) {
        if !self.enabled {
            return;
        }
        self.mode_ns += Self::elapsed_ns(started);
    }

    pub(super) fn add_tx_records(&mut self, started: Option<Instant>, records: usize) {
        if !self.enabled {
            return;
        }
        self.tx_records_ns += Self::elapsed_ns(started);
        self.luma_records = self.luma_records.saturating_add(records);
        self.max_luma_records = self.max_luma_records.max(records);
    }

    pub(super) fn add_far_edge(&mut self, started: Option<Instant>) {
        if !self.enabled {
            return;
        }
        self.far_edge_ns += Self::elapsed_ns(started);
    }

    pub(super) fn add_residual(&mut self, started: Option<Instant>) {
        if !self.enabled {
            return;
        }
        self.residual_ns += Self::elapsed_ns(started);
    }

    pub(super) fn add_intrabc_state(&mut self, started: Option<Instant>) {
        if !self.enabled {
            return;
        }
        self.intrabc_state_ns += Self::elapsed_ns(started);
    }

    fn add_luma_coeff(&mut self, started: Option<Instant>) {
        if !self.enabled {
            return;
        }
        self.luma_coeff_ns += Self::elapsed_ns(started);
        self.decoded_luma_records = self.decoded_luma_records.saturating_add(1);
    }

    pub(super) fn add_luma_coeff_block(
        &mut self,
        started: Option<Instant>,
        block: &LumaCoeffBlock,
    ) {
        self.add_luma_coeff(started);
        if block.all_zero {
            self.luma_all_zero = self.luma_all_zero.saturating_add(1);
        }
    }

    pub(super) fn add_luma_sink(&mut self, started: Option<Instant>) {
        if !self.enabled {
            return;
        }
        self.luma_sink_ns += Self::elapsed_ns(started);
    }

    fn add_chroma_coeff(&mut self, started: Option<Instant>) {
        if !self.enabled {
            return;
        }
        self.chroma_coeff_ns += Self::elapsed_ns(started);
    }

    pub(super) fn add_chroma_coeff_block(
        &mut self,
        started: Option<Instant>,
        block: &LumaCoeffBlock,
    ) {
        self.add_chroma_coeff(started);
        if block.all_zero {
            self.chroma_all_zero = self.chroma_all_zero.saturating_add(1);
        }
    }

    pub(super) fn add_chroma_sink(&mut self, started: Option<Instant>) {
        if !self.enabled {
            return;
        }
        self.chroma_sink_ns += Self::elapsed_ns(started);
        self.decoded_chroma_groups = self.decoded_chroma_groups.saturating_add(1);
    }

    pub(super) fn report(&self) {
        if !self.enabled {
            return;
        }
        eprintln!(
            "splot.decode_timing ac0ej3_tx_handoff_counts blocks={} luma_blocks={} chroma_blocks={} skipped_blocks={} luma_records={} max_luma_records={}",
            self.blocks,
            self.luma_blocks,
            self.chroma_blocks,
            self.skipped_blocks,
            self.luma_records,
            self.max_luma_records
        );
        eprintln!(
            "splot.decode_timing ac0ej3_tx_handoff_breakdown prelude_ms={:.3} mode_ms={:.3} tx_records_ms={:.3} far_edge_ms={:.3} residual_ms={:.3} intrabc_state_ms={:.3}",
            ns_to_ms(self.prelude_ns),
            ns_to_ms(self.mode_ns),
            ns_to_ms(self.tx_records_ns),
            ns_to_ms(self.far_edge_ns),
            ns_to_ms(self.residual_ns),
            ns_to_ms(self.intrabc_state_ns)
        );
        eprintln!(
            "splot.decode_timing ac0ej3_tx_residual_breakdown luma_records={} chroma_groups={} luma_all_zero={} chroma_all_zero={} luma_coeff_ms={:.3} luma_sink_ms={:.3} chroma_coeff_ms={:.3} chroma_sink_ms={:.3}",
            self.decoded_luma_records,
            self.decoded_chroma_groups,
            self.luma_all_zero,
            self.chroma_all_zero,
            ns_to_ms(self.luma_coeff_ns),
            ns_to_ms(self.luma_sink_ns),
            ns_to_ms(self.chroma_coeff_ns),
            ns_to_ms(self.chroma_sink_ns)
        );
    }
}

fn ns_to_ms(ns: u128) -> f64 {
    ns as f64 / 1_000_000.0
}
