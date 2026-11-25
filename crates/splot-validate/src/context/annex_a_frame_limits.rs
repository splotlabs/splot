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
    pending: BTreeMap<(ExtendedLayerId, &'static str), Diagnostic>,
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
            report
                .diagnostics
                .extend(std::mem::take(&mut self.pending).into_values());
        } else {
            self.pending.clear();
        }
    }

    pub(super) fn check_frame(
        &mut self,
        frame: &FrameHeaderCore,
        sequence: &SequenceHeader,
        obu: &ObuEnvelope<'_>,
        report: &mut ValidationReport,
    ) {
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
        let mut check = |metric: &str,
                         value: u64,
                         name: &'static str,
                         max: u64,
                         scaled: u64,
                         rule: &'static str| {
            let diagnostic = |limit, source| {
                Diagnostic::error(
                    rule,
                    format!(
                        "{metric} {value} exceeds {name} {limit} under the {source} for extended layer {} (seq_level_idx {level})",
                        layer.get()
                    ),
                )
                .with_spec_section("A.4")
                .with_byte_offset(obu.offset)
            };
            if value > max {
                report.push(diagnostic(max, "sequence level"));
            } else if value > scaled {
                self.pending
                    .entry((layer, name))
                    .or_insert_with(|| diagnostic(scaled, "MSDO allocation"));
            }
        };

        if let Some(size) = frame.frame_size {
            let rule = "annex-a/frame-size-exceeds-level";
            check(
                "FrameWidth",
                u64::from(size.width),
                "MaxHSize",
                u64::from(native.max_width),
                u64::from(allocated.max_width),
                rule,
            );
            check(
                "FrameHeight",
                u64::from(size.height),
                "MaxVSize",
                u64::from(native.max_height),
                u64::from(allocated.max_height),
                rule,
            );
            check(
                "FrameWidth * FrameHeight",
                u64::from(size.width) * u64::from(size.height),
                "MaxPicSize",
                native.max_pic_size,
                allocated.max_pic_size,
                rule,
            );
        }
        if let Some(tiles) = &frame.tile_info {
            let cols = u64::from(tiles.tile_cols);
            let rule = "annex-a/tile-count-exceeds-level";
            check(
                "NumTiles",
                cols * u64::from(tiles.tile_rows),
                "MaxTiles",
                u64::from(native.max_tiles),
                u64::from(allocated.max_tiles),
                rule,
            );
            check(
                "TileCols",
                cols,
                "MaxTileCols",
                u64::from(native.max_tile_cols),
                u64::from(allocated.max_tile_cols),
                rule,
            );
            // TODO(spec: AV2-A-LEVELS-TIERS): check the Annex A.4 per-tile width and area limits.
        }
        let native_refs = native.excess_ref_frames(sequence, Some(frame));
        let allocated_refs = allocated.excess_ref_frames(sequence, Some(frame));
        if let Some((count, _)) = native_refs.or(allocated_refs) {
            check(
                "NumRefFrames",
                u64::from(count),
                "MaxLevelRefFrames",
                u64::from(native_refs.map_or(count, |(_, max)| max)),
                u64::from(allocated_refs.map_or(count, |(_, max)| max)),
                "annex-a/reference-count-exceeds-level",
            );
        }
        if let Some(size) = frame.frame_size
            && (size.width < MIN_FRAME_DIMENSION || size.height < MIN_FRAME_DIMENSION)
        {
            report.push(Diagnostic::error("annex-a/frame-size-below-minimum", format!(
                "FrameWidth {} / FrameHeight {} must both be at least {MIN_FRAME_DIMENSION} for seq_level_idx {level}",
                size.width, size.height
            )).with_spec_section("A.4").with_byte_offset(obu.offset));
        }
    }
}
