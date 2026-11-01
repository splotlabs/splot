// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame overlap across the mini-GOPs of a batch.

use super::{IntraFrameCoder, Result};
use crate::encoder::{BatchOutput, BatchStep, FrameCoder};

/// Whether `coder` codes batches with frame overlap: the preset asks for it
/// and the frame reads no tool that needs its references finished as a
/// whole (warped motion, the TIP weight probe), nor anything the overlap
/// does not model (tiles, film grain, screen content).
pub(super) fn eligible(coder: &IntraFrameCoder) -> bool {
    let preset = &coder.preset;
    preset.frame_overlap
        && preset.warp == 0
        && preset.tip < 2
        && !preset.palette
        && !preset.intrabc
        && !coder.params.tools.film_grain
        && coder.tiles.cols_log2 == 0
        && coder.tiles.rows_log2 == 0
}

/// Codes `batch` in coding order, committing each frame and show-existing
/// step in turn.
pub(super) fn encode_batch(
    coder: &mut IntraFrameCoder,
    batch: &[Vec<Vec<BatchStep<'_>>>],
) -> Result<Vec<BatchOutput>> {
    let mut outputs = Vec::new();
    for wave in batch.iter().flatten() {
        let jobs: Vec<_> = wave
            .iter()
            .filter_map(|step| match *step {
                BatchStep::Code(source, plan, qp) => Some((source, plan, qp)),
                BatchStep::ShowExisting(_) => None,
            })
            .collect();
        let mut coded = coder.encode_wave(&jobs)?.into_iter();
        for step in wave {
            outputs.push(match *step {
                BatchStep::Code(..) => {
                    let mut frame = coded.next().ok_or(super::Error::Unsupported {
                        what: "a wave with fewer coded frames than planned",
                    })?;
                    coder.commit(&mut frame)?;
                    BatchOutput::Coded(frame)
                }
                BatchStep::ShowExisting(slot) => {
                    BatchOutput::Unit(coder.show_existing_frame(slot)?)
                }
            });
        }
    }
    Ok(outputs)
}
