// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Global motion of inter frames (§ 5.18.9.1).

use crate::tile_writer::{CompoundMode, InterMode, InterPrediction, LeafChoice};

/// Whether `choice` predicts with the global motion of its references:
/// `GLOBALMV` or `GLOBAL_GLOBALMV`.
pub(crate) fn is_global(choice: &LeafChoice) -> bool {
    match choice.inter {
        Some(InterPrediction::Single(modes)) => modes.mode == InterMode::Global,
        Some(InterPrediction::Compound(modes)) => modes.mode == CompoundMode::GlobalGlobal,
        _ => false,
    }
}
