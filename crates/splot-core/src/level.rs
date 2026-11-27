// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Static frame and tile limits from AV2 v1.0.0 Annex A.4, Tables A.7–A.9.

use crate::headers::frame::FrameHeaderCore;
use crate::headers::sequence::SequenceHeader;
use crate::hls::MultistreamDecoderOperation;
use crate::types::{ExtendedLayerId, ObuType};

/// Frame and tile maxima for a table-mapped level (Annex A.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameLevelLimits {
    /// Maximum luma samples per picture, `MaxPicSize`.
    pub max_pic_size: u64,
    /// Maximum frame width in luma samples, `MaxHSize`.
    pub max_width: u32,
    /// Maximum frame height in luma samples, `MaxVSize`.
    pub max_height: u32,
    /// Maximum whole tiles per picture, `MaxTiles`.
    pub max_tiles: u32,
    /// Header-rate basis, `MaxHeaderRate`. The frame-header count limit also
    /// uses the sequence tier (Annex A.4).
    pub max_header_rate: u32,
    /// Maximum tile columns per picture, `MaxTileCols`.
    pub max_tile_cols: u32,
}

/// Looks up the static frame limits for a sequence level. Reserved levels
/// (22–30) and maximum parameters (31) have no row in Tables A.8 and A.9.
#[must_use]
pub fn level_limits(level_idx: u8) -> Option<FrameLevelLimits> {
    Some(group_limits(level_group(level_idx)?))
}

/// Display-rate and bitrate maxima for a table-mapped level (Annex A.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelRateLimits {
    /// `MaxDisplayRate` in luma samples per second (Table A.8).
    pub max_display_rate: u64,
    /// `MainMbps` in kbit/s (Table A.9).
    pub main_kbps: u32,
    /// `HighMbps` in kbit/s (Table A.9); `None` below level 4.0, where
    /// `seq_tier` is always 0.
    pub high_kbps: Option<u32>,
}

/// Looks up the display-rate and bitrate maxima of a sequence level, or
/// `None` for a reserved level and for maximum parameters (31).
#[must_use]
pub fn level_rate_limits(level_idx: u8) -> Option<LevelRateLimits> {
    // (MaxDisplayRate, MainMbps * 10, HighMbps * 10) of Tables A.8 and A.9.
    const RATES: [(u64, u16, u16); 22] = [
        (4_423_680, 15, 0),
        (8_363_520, 30, 0),
        (19_975_680, 60, 0),
        (31_950_720, 100, 0),
        (70_778_880, 120, 300),
        (141_557_760, 200, 500),
        (267_386_880, 300, 1000),
        (534_773_760, 400, 1600),
        (1_069_547_520, 600, 2400),
        (1_069_547_520, 600, 2400),
        (1_069_547_520, 600, 2400),
        (2_139_095_040, 1000, 4800),
        (4_278_190_080, 1600, 8000),
        (4_278_190_080, 1600, 8000),
        (4_278_190_080, 1600, 8000),
        (8_556_380_160, 2000, 9600),
        (17_112_760_320, 3200, 16000),
        (17_112_760_320, 3200, 16000),
        (17_112_760_320, 3200, 16000),
        (34_225_520_640, 4000, 19200),
        (68_451_041_280, 6400, 32000),
        (68_451_041_280, 6400, 32000),
    ];
    let &(max_display_rate, main, high) = RATES.get(usize::from(level_idx))?;
    Some(LevelRateLimits {
        max_display_rate,
        main_kbps: u32::from(main) * 100,
        high_kbps: (high > 0).then_some(u32::from(high) * 100),
    })
}

fn group_limits(group: usize) -> FrameLevelLimits {
    const LEVELS: [(u32, u16, u16, u8, u16); 9] = [
        (147_456, 640, 8, 4, 150),
        (278_784, 880, 8, 4, 150),
        (665_856, 1360, 16, 6, 150),
        (1_065_024, 1720, 16, 6, 150),
        (2_359_296, 2560, 32, 8, 300),
        (8_912_896, 4975, 64, 8, 300),
        (35_651_584, 9951, 128, 16, 300),
        (142_606_336, 19902, 256, 32, 960),
        (530_841_600, 38400, 512, 64, 960),
    ];
    let (pic_size, dimension, tiles, cols, headers) = LEVELS[group];
    FrameLevelLimits {
        max_pic_size: u64::from(pic_size),
        max_width: u32::from(dimension),
        max_height: u32::from(dimension),
        max_tiles: u32::from(tiles),
        max_tile_cols: u32::from(cols),
        max_header_rate: u32::from(headers),
    }
}

fn level_group(level_idx: u8) -> Option<usize> {
    const GROUP: [u8; 22] = [
        0, 1, 2, 3, 4, 4, 5, 5, 5, 5, 6, 6, 6, 6, 7, 7, 7, 7, 8, 8, 8, 8,
    ];
    GROUP.get(usize::from(level_idx)).copied().map(usize::from)
}

const SUBSTREAM: [[(u16, u16, u8); 3]; 5] = [
    [(1600, 896, 7), (960, 576, 4), (640, 384, 3)],
    [(2560, 1472, 7), (1920, 1088, 4), (1280, 768, 3)],
    [(5120, 2280, 13), (3840, 2176, 8), (2560, 1472, 5)],
    [(10240, 5760, 26), (7680, 4320, 16), (5120, 2880, 11)],
    [(20480, 11520, 52), (15360, 8640, 32), (10240, 5760, 21)],
];

impl MultistreamDecoderOperation {
    fn allocation_scale(&self, xlayer: ExtendedLayerId) -> Option<(usize, u32, u32)> {
        let streams = self.sub_streams.get(..usize::from(self.sub_stream_count))?;
        if !streams.iter().any(|sub| sub.sub_xlayer_id == xlayer.get()) {
            return None;
        }
        if self.multistream_even_allocation_flag {
            return Some((1, 4, 1));
        }
        let large = streams.get(usize::from(self.multistream_large_picture_idc?))?;
        if xlayer.get() == large.sub_xlayer_id {
            Some((0, 3, 2))
        } else {
            Some((2, 9, 1))
        }
    }

    /// Computes the static frame and tile maxima in decoder mode 1 (Annex A.4).
    /// The native sequence level supplies the per-stream limits. The aggregate
    /// level and the stream's allocation can reduce them. Returns `None` for
    /// an undeclared layer or an unmapped sequence level. For aggregate levels
    /// 0–3, Splot uses the AVM level-4.0 width, height, tile-column and header-rate cells.
    #[must_use]
    pub fn frame_level_limits(
        &self,
        sequence_level: u8,
        xlayer: ExtendedLayerId,
    ) -> Option<FrameLevelLimits> {
        let (column, numerator, denominator) = self.allocation_scale(xlayer)?;
        let mut limits = level_limits(sequence_level)?;
        if self.multistream_level_idx == 31 {
            return Some(limits);
        }
        let group = level_group(self.multistream_level_idx)?;
        let aggregate = group_limits(group);
        limits.max_tiles = limits
            .max_tiles
            .min(aggregate.max_tiles * denominator / numerator);

        let (height, width, cols) = SUBSTREAM[group.saturating_sub(4)][column];
        limits.max_header_rate = limits.max_header_rate.min(132);
        limits.max_width = limits.max_width.min(u32::from(width));
        limits.max_height = limits.max_height.min(u32::from(height));
        limits.max_tile_cols = limits.max_tile_cols.min(u32::from(cols));
        limits.max_pic_size = limits
            .max_pic_size
            .min(u64::from(width) * u64::from(height));
        Some(limits)
    }
}

impl FrameLevelLimits {
    /// Checks the Annex A.4 video reference-frame limit. Returns the signaled
    /// count and the maximum on a proved violation. Without a frame, this
    /// checks the sequence allocation before a decoder creates its buffers.
    /// The syntax already bounds the count by `8 << explicit_num_ref_frames`.
    #[must_use]
    pub fn excess_ref_frames(
        &self,
        sequence: &SequenceHeader,
        frame: Option<&FrameHeaderCore>,
    ) -> Option<(u8, u8)> {
        if sequence.general.still_picture {
            return None;
        }
        let count = sequence.inter.as_ref()?.num_ref_frames;
        let reserve = frame.is_some_and(|frame| {
            (sequence.general.max_mlayer_id.get() != 0 || frame.obu_type != ObuType::ClosedLoopKey)
                && requires_second_decode(frame)
        });
        let slots = self.ref_frame_slots(sequence, reserve)?;
        (u64::from(count) > slots).then_some((count, slots as u8))
    }

    /// `8 * MaxPicSize / (max_frame_width * max_frame_height)` of
    /// `MaxLevelRefFrames` (Annex A.4), one less with `reserve`: a frame with
    /// `DecodeCount` 2 that is not a closed-loop key frame, or any such frame
    /// when `max_mlayer_id` is not 0.
    #[must_use]
    pub fn ref_frame_slots(&self, sequence: &SequenceHeader, reserve: bool) -> Option<u64> {
        let area = u64::from(sequence.general.max_frame_width.get())
            * u64::from(sequence.general.max_frame_height.get());
        Some(
            self.max_pic_size
                .checked_mul(8)?
                .checked_div(area)?
                .saturating_sub(u64::from(reserve)),
        )
    }
}

fn requires_second_decode(frame: &FrameHeaderCore) -> bool {
    frame
        .intrabc
        .is_some_and(|p| p.allow_global_intrabc == Some(true))
        && (frame
            .deblocking_filter_params
            .as_ref()
            .is_some_and(|p| p.apply_deblocking_filter[0] || p.apply_deblocking_filter[1])
            || frame
                .cdef_params
                .as_ref()
                .is_some_and(|p| p.cdef_frame_enable)
            || frame.ccso_params.as_ref().is_some_and(|p| {
                p.ccso_frame_flag == Some(true) || p.planes.iter().any(|plane| plane.ccso_planes)
            })
            || frame.lr_params.as_ref().is_some_and(|p| p.uses_lr)
            || frame
                .gdf_params
                .as_ref()
                .is_some_and(|p| p.gdf_frame_enable))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annexb::parse_annex_b_obus;
    use crate::bitio::BitReader;
    use crate::headers::frame::{
        DeblockingFilterParams, FrameHeaderParseInput, FrameReferenceStateView, IntrabcParams,
        parse_frame_header_core,
    };
    use crate::obu::{ParsedObu, PayloadStatus};
    use crate::types::EmbeddedLayerId;

    #[test]
    fn second_decode_reserves_a_reference_slot_only_for_filtered_global_intrabc() {
        let data = include_bytes!(
            "../../../tests/conformance/vectors/valid/syn-msdo-two-streams-8bit.obu"
        );
        let obus = parse_annex_b_obus(data).unwrap();
        let sh = obus
            .iter()
            .find(|obu| obu.header.obu_type == ObuType::SequenceHeader)
            .unwrap();
        let PayloadStatus::Parsed(ParsedObu::SequenceHeader(mut seq)) =
            sh.payload_status().unwrap()
        else {
            unreachable!()
        };
        let key = obus
            .iter()
            .find(|obu| obu.header.obu_type == ObuType::ClosedLoopKey)
            .unwrap();
        let mut frame = parse_frame_header_core(
            &mut BitReader::new(key.payload, key.payload_offset()),
            &FrameHeaderParseInput {
                obu_type: key.header.obu_type,
                obu_mlayer_id: key.header.embedded_layer_id,
                obu_tlayer_id: key.header.temporal_layer_id,
                first_picture_in_tu: true,
                active_sequence: Some(&seq),
                mfh_record: None,
                reference_state: FrameReferenceStateView::unknown(),
            },
        )
        .unwrap();
        seq.inter.as_mut().unwrap().num_ref_frames = 8;
        let mut limits = level_limits(0).unwrap();
        limits.max_pic_size = u64::from(seq.general.max_frame_width.get())
            * u64::from(seq.general.max_frame_height.get());
        assert_eq!(limits.excess_ref_frames(&seq, None), None);
        frame.intrabc = Some(IntrabcParams {
            allow_intrabc: true,
            allow_global_intrabc: Some(true),
            allow_local_intrabc: Some(true),
            change_bvp_drl: None,
            max_bvp_drl_bits_minus_1: None,
        });
        frame.deblocking_filter_params = Some(DeblockingFilterParams::new(
            [true, false, false, false],
            [false; 4],
            [0; 4],
        ));
        assert_eq!(limits.excess_ref_frames(&seq, Some(&frame)), None);
        frame.obu_type = ObuType::OpenLoopKey;
        assert_eq!(limits.excess_ref_frames(&seq, Some(&frame)), Some((8, 7)));
        frame.obu_type = ObuType::ClosedLoopKey;
        seq.general.max_mlayer_id = EmbeddedLayerId::from_bits(1);
        assert_eq!(limits.excess_ref_frames(&seq, Some(&frame)), Some((8, 7)));
        frame.intrabc.as_mut().unwrap().allow_global_intrabc = Some(false);
        assert_eq!(limits.excess_ref_frames(&seq, Some(&frame)), None);
        seq.general.still_picture = true;
        assert_eq!(limits.excess_ref_frames(&seq, None), None);
    }

    #[test]
    fn rate_limits_follow_tables_a8_and_a9() {
        let row = |level| level_rate_limits(level).unwrap();
        assert_eq!(row(0).max_display_rate, 4_423_680);
        assert_eq!((row(0).main_kbps, row(0).high_kbps), (1_500, None));
        assert_eq!(row(3).high_kbps, None);
        assert_eq!((row(4).main_kbps, row(4).high_kbps), (12_000, Some(30_000)));
        assert_eq!(row(9).max_display_rate, 1_069_547_520);
        assert_eq!(
            (row(19).main_kbps, row(19).high_kbps),
            (400_000, Some(1_920_000))
        );
        assert_eq!(row(21).max_display_rate, 68_451_041_280);
        assert_eq!(
            (row(21).main_kbps, row(21).high_kbps),
            (640_000, Some(3_200_000))
        );
        assert!(level_rate_limits(22).is_none() && level_rate_limits(31).is_none());
    }
}
