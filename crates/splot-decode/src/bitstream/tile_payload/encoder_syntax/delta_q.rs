// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Per-superblock quantizer index deltas (§ 5.20.5.11, § 5.18.7.8).

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::SymbolSink;
use splot_recon::{BitDepth, max_quantizer_index};

use super::super::cdf::TileCdfSelector;
use super::{EncoderSyntaxError, IntraTileSyntax, LeafBlock, Result, rule};
use crate::filters::wienerns_lr::tx_records::{
    DELTA_Q_SMALL, delta_q_is_coded_for_block, updated_current_q_index,
};

/// The `CurrentQIndex` state of one tile.
#[derive(Clone, Copy, Debug)]
pub(super) struct DeltaQ {
    /// `delta_q_res` of a frame with `delta_q_present`.
    res: Option<u8>,
    max_q: i32,
    /// `CurrentQIndex` after the last coded delta.
    coded: i32,
    /// The qindex of the superblock's blocks and its `reducedDeltaQIndex`.
    planned: (i32, i32),
    /// `ReadDeltas`.
    pending: bool,
}

impl DeltaQ {
    /// The state at the start of a tile: `CurrentQIndex = base_q_idx`.
    pub(super) fn new(core: &FrameHeaderCore, base_q_idx: u32, bit_depth: BitDepth) -> Self {
        let base = base_q_idx as i32;
        Self {
            res: core
                .delta_q_params
                .filter(|dq| dq.delta_q_present)
                .map(|dq| dq.delta_q_res),
            max_q: max_quantizer_index(bit_depth) as i32,
            coded: base,
            planned: (base, 0),
            pending: false,
        }
    }
}

impl IntraTileSyntax {
    /// The qindex of the blocks of the current superblock (§ 7.14.1 `get_qindex`).
    #[must_use]
    pub const fn qindex(&self) -> u32 {
        self.tools.qindex
    }

    /// Plans the qindex of the blocks of the following superblocks: the
    /// codable `CurrentQIndex` nearest `target`. Returns it.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Unsupported`] when the frame does not
    /// code `delta_q_present`.
    pub fn set_superblock_qindex(&mut self, target: u32) -> Result<u32> {
        let state = &mut self.delta_q;
        let res = state.res.ok_or(EncoderSyntaxError::Unsupported(
            "delta-q without delta_q_present",
        ))?;
        let step = 1 << res;
        let target = i32::try_from(target).unwrap_or(i32::MAX).min(state.max_q);
        let reduced = (target - state.coded + step / 2).div_euclid(step);
        let qindex = updated_current_q_index(state.coded, reduced, res, state.max_q);
        state.planned = (qindex, reduced);
        self.apply_qindex(qindex);
        Ok(qindex as u32)
    }

    /// Writes `delta_q_abs` and its escape and sign bits when `leaf`, coded
    /// with `skip_txfm = skip`, is the first leaf of its superblock that reads
    /// them; a superblock-sized skipped leaf keeps the previous `CurrentQIndex`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for a write failure.
    pub fn write_delta_qindex(
        &mut self,
        leaf: &LeafBlock,
        skip: bool,
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        if !self.delta_q.pending {
            return Ok(());
        }
        self.delta_q.pending = false;
        let coded = delta_q_is_coded_for_block(leaf.block_size(), self.sb_size4, skip)
            .map_err(rule("delta-q block size"))?;
        let (qindex, reduced) = if coded {
            self.delta_q.planned
        } else {
            (self.delta_q.coded, 0)
        };
        self.delta_q.coded = qindex;
        self.apply_qindex(qindex);
        if !coded {
            return Ok(());
        }
        let abs = reduced.unsigned_abs() as usize;
        let small = abs.min(DELTA_Q_SMALL);
        let cdf = self.tile_row(TileCdfSelector::DeltaQ)?;
        symbols
            .symbol(cdf, Symbol::new(small as u8))
            .map_err(rule("delta_q_abs"))?;
        if small == DELTA_Q_SMALL {
            let rest = (abs - (DELTA_Q_SMALL - 2)) as u32;
            let rem_bits = rest.ilog2();
            symbols
                .literal(rem_bits - 1, 3)
                .and_then(|()| symbols.literal(rest - (1 << rem_bits), rem_bits))
                .map_err(rule("delta_q_rem_bits"))?;
        }
        if abs != 0 {
            symbols
                .literal(u32::from(reduced < 0), 1)
                .map_err(rule("delta_q_sign_bit"))?;
        }
        Ok(())
    }

    /// Starts `ReadDeltas` for a new superblock.
    pub(super) const fn begin_delta_q(&mut self) {
        self.delta_q.pending = self.delta_q.res.is_some();
    }

    fn apply_qindex(&mut self, qindex: i32) {
        self.tools.qindex = qindex as u32;
        if let Some(inter) = &mut self.inter {
            inter.tile.set_qindex(qindex as u32);
        }
    }
}
