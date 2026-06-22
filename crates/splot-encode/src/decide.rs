// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Encoder decision seams: the trait boundaries where coding decisions are made.
//!
//! The minimal working encoder makes every decision trivially (a fixed partition, DC
//! intra mode, the largest transform, a constant quantizer). To keep the optimization
//! features that come later (RDO, rate control, mode/partition/transform search) **additive
//! rather than a rewrite**, each decision is taken behind a small trait — a *seam* — with a
//! trivial `Fixed*`/`Constant*` implementation now. A later phase swaps in a search-driven
//! implementation behind the same trait; the bitstream serializers that consume the decision
//! never change.
//!
//! The planned seams (added one per brick, each byte-identical to the prior fixed behaviour):
//!
//! | Seam | implementation now | grows into |
//! |------|--------------------|------------|
//! | [`RateController`] | [`ConstantQp`] | CRF → 2-pass → CBR/VBR, λ↔QP, lookahead |
//! | `PartitionDecider` (future) | fixed | recursive RD partition search |
//! | `IntraModeDecider` (future) | DC_PRED | intra-mode RDO |
//! | `TransformDecider` (future) | DCT largest | transform type + size search |
//! | `CostModel` (future) | distortion-only | the λ-weighted RD objective |
//!
//! Rate/cost estimates that later seams need are derived from the AV2 entropy coder
//! (`encode_block_symbol_trace`), never from AV1 rate tables — rav1e and SVT-AV1 inform the
//! *structure* of these seams only (see `docs/references/`), not AV2 syntax or numbers.

/// The quantizer-decision seam: chooses the frame's `base_q_idx` (AV2 § 5.18.6.1;
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-1`), and — in later phases —
/// per-superblock / per-block quantizer deltas and frame-type-aware quantizers.
///
/// The minimal encoder is constant-QP ([`ConstantQp`]); rate-controlled implementations
/// (CRF, two-pass, CBR/VBR) plug in behind this same trait without touching the header
/// writer or the coefficient path.
pub(crate) trait RateController {
    /// The frame-header `base_q_idx` for the frame currently being encoded.
    fn frame_base_q_idx(&self) -> u8;
}

/// Constant-QP rate control: every frame is coded at the configured fixed quantizer
/// ([`crate::EncoderConfig::qp`]). The degenerate, max-pruning case of the
/// [`RateController`] seam — no rate feedback, no per-block deltas.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct ConstantQp {
    qp: u8,
}

impl ConstantQp {
    /// Creates a constant-QP rate controller coding every frame at `qp`.
    pub(crate) const fn new(qp: u8) -> Self {
        Self { qp }
    }
}

impl RateController for ConstantQp {
    fn frame_base_q_idx(&self) -> u8 {
        self.qp
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn constant_qp_returns_the_configured_quantizer() {
        let rc = ConstantQp::new(80);
        assert_eq!(rc.frame_base_q_idx(), 80);
        // A second value confirms it is not hardcoded.
        assert_eq!(ConstantQp::new(40).frame_base_q_idx(), 40);
    }

    #[test]
    fn constant_qp_is_used_through_the_rate_controller_trait() {
        // Exercise the decision through the seam (trait object), the way callers consume it.
        let rc = ConstantQp::new(55);
        let seam: &dyn RateController = &rc;
        assert_eq!(seam.frame_base_q_idx(), 55);
    }
}
