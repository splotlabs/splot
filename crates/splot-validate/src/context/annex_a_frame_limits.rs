// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame and tile limits for single-stream and multistream decoding (Annex A.4).

use super::*;

#[derive(Debug, Default)]
pub(super) struct FrameAllocations {
    pub(super) latest: Option<MultistreamDecoderOperation>,
    multistream: bool,
    saw_msdo: bool,
    // One finding per layer and limit suffices until a later CELU fixes the TU's mode (§ 7.4.1).
    limits_seen: [u8; 32],
    pending: Vec<Diagnostic>,
}

impl FrameAllocations {
    pub(super) fn note_msdo(&mut self) {
        self.saw_msdo = true;
        self.latest = None;
    }

    pub(super) fn complete_temporal_unit(&mut self, is_rap: bool, report: &mut ValidationReport) {
        if is_rap {
            self.multistream = self.saw_msdo;
        }
        self.saw_msdo = false;
        if self.multistream {
            report.diagnostics.append(&mut self.pending);
        } else {
            self.pending.clear();
        }
        self.limits_seen.fill(0);
    }

    pub(super) fn check_frame(
        &mut self,
        frame: Option<&FrameHeaderCore>,
        sequence: &SequenceHeader,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        let size = frame.and_then(|frame| frame.frame_size);
        let level = sequence.general.seq_level_idx.get();
        let Some(native) = level_limits(level) else {
            return;
        };
        let layer = obu.header.extended_layer_id;
        let allocated = self
            .latest
            .as_ref()
            .and_then(|msdo| msdo.frame_level_limits(level, layer))
            .unwrap_or(native);
        let mut check = |kind: Limit, value: u64, max: u64, scaled: u64| {
            if value > scaled {
                self.record_excess(kind, value, (max, scaled), level, obu, report);
            }
        };

        if let Some(size) = size {
            check(
                Limit::Width,
                u64::from(size.width),
                u64::from(native.max_width),
                u64::from(allocated.max_width),
            );
            check(
                Limit::Height,
                u64::from(size.height),
                u64::from(native.max_height),
                u64::from(allocated.max_height),
            );
            check(
                Limit::Area,
                u64::from(size.width) * u64::from(size.height),
                native.max_pic_size,
                allocated.max_pic_size,
            );
        }
        if let Some(tiles) = frame.and_then(|core| core.tile_info.as_ref()) {
            let cols = u64::from(tiles.tile_cols);
            check(
                Limit::Tiles,
                cols * u64::from(tiles.tile_rows),
                u64::from(native.max_tiles),
                u64::from(allocated.max_tiles),
            );
            check(
                Limit::TileCols,
                cols,
                u64::from(native.max_tile_cols),
                u64::from(allocated.max_tile_cols),
            );
            // TODO(spec: AV2-A-LEVELS-TIERS): check the Annex A.4 per-tile width and area limits.
        }
        let native_refs = native.excess_ref_frames(sequence, frame);
        let allocated_refs = allocated.excess_ref_frames(sequence, frame);
        if let Some((count, _)) = native_refs.or(allocated_refs) {
            check(
                Limit::References,
                u64::from(count),
                u64::from(native_refs.map_or(count, |(_, max)| max)),
                u64::from(allocated_refs.map_or(count, |(_, max)| max)),
            );
        }
        if let Some(size) = size
            && (size.width < MIN_FRAME_DIMENSION || size.height < MIN_FRAME_DIMENSION)
        {
            report.push(Diagnostic::error("annex-a/frame-size-below-minimum", format!(
                "FrameWidth {} / FrameHeight {} must both be at least {MIN_FRAME_DIMENSION} for seq_level_idx {level}",
                size.width, size.height
            )).with_spec_section("A.4").with_byte_offset(obu.offset));
        }
    }

    #[cold]
    fn record_excess(
        &mut self,
        kind: Limit,
        value: u64,
        (native, scaled): (u64, u64),
        level: u8,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
        let (output, bound, source) = if value > native {
            (&mut report.diagnostics, native, "sequence level")
        } else {
            let seen = &mut self.limits_seen[usize::from(obu.header.extended_layer_id.get())];
            let mask = 1 << kind as u8;
            if *seen & mask != 0 {
                return;
            }
            *seen |= mask;
            (&mut self.pending, scaled, "MSDO allocation")
        };
        output.push(kind.diagnostic(value, bound, source, level, obu));
    }
}

#[derive(Clone, Copy)]
enum Limit {
    Width,
    Height,
    Area,
    Tiles,
    TileCols,
    References,
}

impl Limit {
    #[cold]
    fn diagnostic(
        self,
        value: u64,
        bound: u64,
        source: &str,
        level: u8,
        obu: &ObuEnvelope<'_>,
    ) -> Diagnostic {
        let (metric, name, rule) = match self {
            Self::Width => ("FrameWidth", "MaxHSize", "annex-a/frame-size-exceeds-level"),
            Self::Height => (
                "FrameHeight",
                "MaxVSize",
                "annex-a/frame-size-exceeds-level",
            ),
            Self::Area => (
                "FrameWidth * FrameHeight",
                "MaxPicSize",
                "annex-a/frame-size-exceeds-level",
            ),
            Self::Tiles => ("NumTiles", "MaxTiles", "annex-a/tile-count-exceeds-level"),
            Self::TileCols => (
                "TileCols",
                "MaxTileCols",
                "annex-a/tile-count-exceeds-level",
            ),
            Self::References => (
                "NumRefFrames",
                "MaxLevelRefFrames",
                "annex-a/reference-count-exceeds-level",
            ),
        };
        Diagnostic::error(rule, format!(
            "{metric} {value} exceeds {name} {bound} under the {source} for extended layer {} (seq_level_idx {level})",
            obu.header.extended_layer_id.get()
        )).with_spec_section("A.4").with_byte_offset(obu.offset)
    }
}
