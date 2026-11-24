// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Decoder output adapters for hash, raw, and Y4M publication.

pub(crate) mod film_grain;
pub(crate) mod hash;
pub(crate) mod raw;
pub(crate) mod y4m;

use splot_core::types::ExtendedLayerId;

use crate::error::Result;
use crate::pipeline::unsupported;

pub(crate) fn track_flat_output_xlayer(
    seen: &mut Option<ExtendedLayerId>,
    xlayer: ExtendedLayerId,
) -> Result<()> {
    match *seen {
        Some(first) if first != xlayer => Err(unsupported(
            "multiple_extended_layers_flat_output",
            None,
            "flat raw and Y4M output cannot mix frames from multiple extended layers; select one layer with --extended-layer",
        )),
        Some(_) => Ok(()),
        None => {
            *seen = Some(xlayer);
            Ok(())
        }
    }
}
