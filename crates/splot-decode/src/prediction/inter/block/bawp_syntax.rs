// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The block adaptive weighted prediction syntax of a single-reference block
//! (§ 5.20.7.6): its read gate, its symbols and the reference distance its
//! § 7.13.3.25 process uses.

#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BawpParseInput {
    pub(crate) allow_bawp: bool,
    pub(crate) reference_scaled: bool,
    pub(crate) frame_is_switch: bool,
    pub(crate) single_mode: u8,
    pub(crate) use_amvd: bool,
    pub(crate) n4w: usize,
    pub(crate) n4h: usize,
    pub(crate) has_chroma: bool,
    pub(crate) avail_up: bool,
    pub(crate) avail_left: bool,
}

/// Whether a single-reference block of `input` reads `use_bawp`
/// (§ 5.20.7.6).
pub(crate) const fn bawp_signal_allowed(input: BawpParseInput) -> bool {
    input.allow_bawp
        && !input.reference_scaled
        && !input.frame_is_switch
        && input.single_mode != SINGLE_MODE_GLOBALMV
        && input.n4w >= 2
        && input.n4h >= 2
}

pub(crate) fn read_bawp_syntax(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: BawpParseInput,
    tile_offset: ByteOffset,
) -> Result<BawpSyntax> {
    if !bawp_signal_allowed(input) {
        return Ok(BawpSyntax::default());
    }

    let use_bawp = cdfs
        .read_block_symbol_trace(TileCdfSelector::UseBawp, symbols)
        .map_err(|error| symbol_read_error(error, tile_offset))?;
    if use_bawp.get() == 0 {
        return Ok(BawpSyntax::default());
    }

    let list_index = explicit_bawp_context(input.single_mode, input.use_amvd);
    let explicit_bawp = cdfs
        .read_block_symbol_trace(TileCdfSelector::ExplicitBawp { ctx: list_index }, symbols)
        .map_err(|error| symbol_read_error(error, tile_offset))?;
    let explicit = explicit_bawp.get() != 0;
    let explicit_scale_positive = if explicit {
        cdfs.read_block_symbol_trace(TileCdfSelector::ExplicitBawpScale, symbols)
            .map_err(|error| symbol_read_error(error, tile_offset))?
            .get()
            != 0
    } else {
        false
    };
    let chroma = if input.has_chroma {
        cdfs.read_block_symbol_trace(TileCdfSelector::UseBawpChroma, symbols)
            .map_err(|error| symbol_read_error(error, tile_offset))?
            .get()
            != 0
    } else {
        false
    };

    Ok(BawpSyntax {
        enabled: true,
        explicit,
        explicit_scale_positive,
        list_index: list_index as u8,
        ref_dist_gt4: false,
        chroma,
        avail_up: input.avail_up,
        avail_left: input.avail_left,
    })
}

pub(crate) fn explicit_bawp_context(single_mode: u8, use_amvd: bool) -> usize {
    if single_mode == SINGLE_MODE_NEARMV {
        0
    } else if single_mode == SINGLE_MODE_NEWMV && use_amvd {
        1
    } else {
        2
    }
}
