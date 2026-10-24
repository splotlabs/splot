// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Trellis-coded quantization (TCQ): the § 5.20.7.27 state machine, the
//! per-state reconstruction index, and the blocks that use it, all from the
//! decoder's own coefficient passes.

use std::borrow::Cow;

use super::super::coeff_loop::max_level::CoeffTransformClass;
use super::super::coeff_loop::quant_state::{
    CoeffQuantReadInput, CoeffQuantStateAccumulator, CoeffQuantStateConfig, next_tcq_state,
};
use super::super::coeff_loop::scan_walk::CoeffScanEntry;
use super::{EncoderSyntaxError, IntraTileSyntax, Result, TransformBlock, coefficient_scan, rule};

/// The number of `tcqState` values.
pub const TCQ_STATES: usize = 8;

/// `Tcq_Next_State[state][parity & 1]` (§ 5.20.7.27).
#[must_use]
pub fn tcq_next_state(state: usize, parity: u32) -> usize {
    next_tcq_state(state % TCQ_STATES, parity).unwrap_or(0)
}

/// The quantizer of `state`, `(tcqState >> 1) & 1`: it selects the
/// `coeff_base` rows (§ 8.3.2) and the reconstruction offset of a level.
#[must_use]
pub const fn tcq_quantizer(state: usize) -> usize {
    (state >> 1) & 1
}

/// The reconstruction index the decoder dequantizes for a level of magnitude
/// `quant` coded in `state`: `quant * 2 - q0` for a nonzero level.
///
/// # Errors
/// Returns [`EncoderSyntaxError::Rule`] for a magnitude the decoder rejects.
pub fn tcq_index(state: usize, quant: u32) -> Result<u32> {
    let mut walk = quant_walk();
    walk.set_tcq_q0(tcq_quantizer(state) == 1);
    walk.apply_entry(
        0,
        CoeffScanEntry::new(1, 1, 0, 1),
        false,
        CoeffQuantReadInput { quant },
    )
    .map(|write| write.quant().unsigned_abs())
    .map_err(rule("TCQ reconstruction index"))
}

impl IntraTileSyntax {
    /// Whether the levels of `tx` are TCQ levels (`useTcq` of § 5.20.7.27):
    /// the frame has `allow_tcq` and `tx` is a luma block of class
    /// `TX_CLASS_2D` without FSC. Such a block dequantizes with `dqDenom`
    /// doubled (§ 7.14.4).
    #[must_use]
    pub fn uses_tcq(&self, tx: &TransformBlock) -> bool {
        self.tools.tcq
            && !self.tools.scalar_trial
            && tx.plane == 0
            && CoeffTransformClass::from_plane_tx_type(tx.tx_type as usize)
                == CoeffTransformClass::TwoD
            && !(self.tools.enable_fsc && self.uses_fsc(tx))
    }

    /// Prices a search trial of a TCQ frame with plain levels: until set back,
    /// no block [`Self::uses_tcq`]. A trial coded so must not reach the bitstream.
    pub const fn set_scalar_trial(&mut self, on: bool) {
        self.tools.scalar_trial = on;
    }

    /// The decoder's `Quant` of row-major coded `levels` of `tx`: the levels
    /// themselves, or their reconstruction indices along the TCQ states of the
    /// scan when [`Self::uses_tcq`].
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for short `levels` or a magnitude the
    /// decoder rejects.
    pub fn decoder_quant<'a>(
        &self,
        tx: &TransformBlock,
        levels: &'a [i32],
    ) -> Result<Cow<'a, [i32]>> {
        if !self.uses_tcq(tx) {
            return Ok(Cow::Borrowed(levels));
        }
        let scan = coefficient_scan(tx)?;
        let short = || EncoderSyntaxError::Unsupported("levels shorter than the block");
        let mut out = levels.to_vec();
        let eob = scan
            .iter()
            .rposition(|&pos| levels.get(usize::from(pos)).is_some_and(|&l| l != 0))
            .map_or(0, |last| last + 1);
        let mut walk = quant_walk();
        for (c, &pos) in scan[..eob].iter().enumerate().rev() {
            let pos = usize::from(pos);
            let level = *levels.get(pos).ok_or_else(short)?;
            let entry = CoeffScanEntry::new(c, pos, 0, 0);
            let input = CoeffQuantReadInput {
                quant: level.unsigned_abs(),
            };
            out[pos] = walk
                .apply_entry(c, entry, level < 0, input)
                .map_err(rule("TCQ reconstruction index"))?
                .quant();
        }
        Ok(Cow::Owned(out))
    }
}

/// The decoder's quant pass of a TCQ block.
const fn quant_walk() -> CoeffQuantStateAccumulator {
    CoeffQuantStateAccumulator::new(CoeffQuantStateConfig {
        is_hidden: false,
        sum_abs1: 0,
        use_tcq: true,
        lossless: false,
    })
}
