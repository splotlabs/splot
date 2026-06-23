// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.20 loop-restoration caller-resolved helpers.
//!
//! This module implements the scheduler-free AV2 § 7.20.2 source-sample
//! coordinate and source-frame selection process
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md#s-7-20-2)).
//! It also exposes an immutable frame-view wrapper that reads the selected
//! `CurrFrame` or `CdefFrame` sample after the same selector has resolved the
//! source coordinates.
//!
//! Feature tracking: `RECON-LOOP-RESTORATION-SOURCE-SAMPLE`,
//! `RECON-LOOP-RESTORATION-SOURCE-READ`.

use crate::{DecodedFrameInfo, FrameRef, PlaneId, PlaneSize, ReconError, ReconSample, Result};

/// Source frame selected by AV2 § 7.20.2 for a loop-restoration sample.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoopRestorationSource {
    /// Read from `CurrFrame`, before CDEF and CCSO are applied.
    CurrFrame,
    /// Read from `CdefFrame`, after CDEF and CCSO are applied.
    CdefFrame,
}

/// Caller-resolved luma-coordinate bounds for AV2 § 7.20.2 source samples.
///
/// The luma extents are inclusive and correspond to `LumaStartX`, `LumaEndX`,
/// `LumaStartY`, `LumaEndY`, `LumaStripeStartY`, and `LumaStripeEndY` from the
/// loop-restore-block process. `subsampling_x` and `subsampling_y` are the
/// sequence `SubsamplingX` / `SubsamplingY` values used for chroma planes; luma
/// samples always use zero subsampling regardless of these fields. The stripe y
/// range is treated as the caller-resolved subrange of the allowed luma y
/// extent and is rejected if it is inconsistent with that extent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoopRestorationSourceBounds {
    /// Inclusive minimum luma x coordinate available to loop restoration.
    pub luma_start_x: usize,
    /// Inclusive maximum luma x coordinate available to loop restoration.
    pub luma_end_x: usize,
    /// Inclusive minimum luma y coordinate available to loop restoration.
    pub luma_start_y: usize,
    /// Inclusive maximum luma y coordinate available to loop restoration.
    pub luma_end_y: usize,
    /// Inclusive luma y coordinate at the start of the current restoration stripe.
    pub luma_stripe_start_y: usize,
    /// Inclusive luma y coordinate at the end of the current restoration stripe.
    pub luma_stripe_end_y: usize,
    /// AV2 sequence `SubsamplingX` for chroma source-coordinate derivation.
    pub subsampling_x: u8,
    /// AV2 sequence `SubsamplingY` for chroma source-coordinate derivation.
    pub subsampling_y: u8,
}

/// Resolved AV2 § 7.20.2 source sample location.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoopRestorationSourceSample {
    /// Clipped x coordinate in the selected plane.
    pub x: usize,
    /// Clipped y coordinate in the selected plane.
    pub y: usize,
    /// Frame array the caller must sample.
    pub source: LoopRestorationSource,
}

/// Resolved AV2 § 7.20.2 source sample plus the selected frame value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoopRestorationSourceSampleValue<T: ReconSample> {
    /// Source sample coordinate and frame selection.
    pub sample: LoopRestorationSourceSample,
    /// Sample value read from the selected immutable frame view.
    pub value: T,
}

/// Resolves the AV2 § 7.20.2 loop-restoration source sample.
///
/// `x` and `y` are input coordinates in the selected plane, before the § 7.20.2
/// clipping and stripe adjustment. The returned coordinates are also in the
/// selected plane. For chroma planes, luma extents and stripe bounds are shifted
/// by `subsampling_x` / `subsampling_y`; for luma, no shift is applied.
///
/// # Errors
/// Returns typed [`ReconError`] values when caller-resolved bounds are invalid,
/// subsampling values are outside the AV2 `0..=1` domain, or luma coordinates
/// cannot be represented for signed clipping.
pub fn loop_restoration_source_sample(
    plane: PlaneId,
    x: isize,
    y: isize,
    bounds: &LoopRestorationSourceBounds,
) -> Result<LoopRestorationSourceSample> {
    validate_source_bounds(bounds)?;
    let (sub_x, sub_y) = plane_subsampling(plane, bounds);

    let min_x = shifted_bound(bounds.luma_start_x, sub_x, "loop restoration min x")?;
    let max_x = shifted_bound(bounds.luma_end_x, sub_x, "loop restoration max x")?;
    let min_y = shifted_bound(bounds.luma_start_y, sub_y, "loop restoration min y")?;
    let max_y = shifted_bound(bounds.luma_end_y, sub_y, "loop restoration max y")?;
    let stripe_start = shifted_bound(
        bounds.luma_stripe_start_y,
        sub_y,
        "loop restoration stripe start y",
    )?;
    let stripe_end = shifted_bound(
        bounds.luma_stripe_end_y,
        sub_y,
        "loop restoration stripe end y",
    )?;

    let clipped_x = clip3(x, min_x, max_x);
    let mut clipped_y = clip3(y, min_y, max_y);
    let source = if clipped_y < stripe_start {
        clipped_y = clipped_y.max(stripe_start.saturating_sub(2));
        LoopRestorationSource::CurrFrame
    } else if clipped_y > stripe_end {
        clipped_y = clipped_y.min(stripe_end.saturating_add(2));
        LoopRestorationSource::CurrFrame
    } else {
        LoopRestorationSource::CdefFrame
    };

    Ok(LoopRestorationSourceSample {
        x: clipped_x as usize,
        y: clipped_y as usize,
        source,
    })
}

/// Resolves and reads the AV2 § 7.20.2 loop-restoration source sample.
///
/// This composes [`loop_restoration_source_sample`] with immutable
/// [`FrameRef`] reads. `curr_frame` is the pre-CDEF/CCSO `CurrFrame` source,
/// and `cdef_frame` is the post-CDEF/CCSO `CdefFrame` source named by AV2
/// § 7.20.2
/// ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md#s-7-20-2)).
/// Input and returned coordinates are current-plane coordinates, and are used
/// as absolute coded-storage coordinates for the selected [`PlaneRef`](crate::PlaneRef)
/// backing buffer. Any visible crop origin on the [`FrameRef`] is not applied to
/// the § 7.20.2 coordinate before reading.
///
/// # Errors
/// Returns typed [`ReconError`] values when selector bounds are invalid, the two
/// frame views do not describe the same frame metadata, the selected chroma
/// plane is absent, selected plane view geometry differs between source frames,
/// the backing storage cannot cover the coded plane coordinate, or the selected
/// sample cannot represent the frame bit depth.
pub fn loop_restoration_source_sample_value<T: ReconSample>(
    plane: PlaneId,
    x: isize,
    y: isize,
    bounds: &LoopRestorationSourceBounds,
    curr_frame: FrameRef<'_, T>,
    cdef_frame: FrameRef<'_, T>,
) -> Result<LoopRestorationSourceSampleValue<T>> {
    if curr_frame.info() != cdef_frame.info() {
        return Err(ReconError::LoopRestorationSourceFrameMismatch {
            field: "frame metadata",
        });
    }

    let sample = loop_restoration_source_sample(plane, x, y, bounds)?;
    validate_source_plane_pair(plane, curr_frame, cdef_frame)?;
    let source_frame = match sample.source {
        LoopRestorationSource::CurrFrame => curr_frame,
        LoopRestorationSource::CdefFrame => cdef_frame,
    };
    let value = read_frame_sample(source_frame, plane, sample.x, sample.y)?;

    Ok(LoopRestorationSourceSampleValue { sample, value })
}

fn validate_source_plane_pair<T: ReconSample>(
    plane: PlaneId,
    curr_frame: FrameRef<'_, T>,
    cdef_frame: FrameRef<'_, T>,
) -> Result<()> {
    let Some(curr_plane) = curr_frame.plane(plane) else {
        return Err(ReconError::MissingChromaPlane { plane });
    };
    let Some(cdef_plane) = cdef_frame.plane(plane) else {
        return Err(ReconError::MissingChromaPlane { plane });
    };
    if curr_plane.visible_rect() != cdef_plane.visible_rect()
        || curr_plane.stride_samples() != cdef_plane.stride_samples()
    {
        return Err(ReconError::LoopRestorationSourceFrameMismatch {
            field: "plane view geometry",
        });
    }
    Ok(())
}

fn read_frame_sample<T: ReconSample>(
    frame: FrameRef<'_, T>,
    plane: PlaneId,
    x: usize,
    y: usize,
) -> Result<T> {
    if !T::supports_bit_depth(frame.info().bit_depth()) {
        return Err(ReconError::SampleTypeUnsupportedBitDepth {
            sample_type: T::TYPE_NAME,
            bit_depth: frame.info().bit_depth(),
        });
    }
    let Some(plane_ref) = frame.plane(plane) else {
        return Err(ReconError::MissingChromaPlane { plane });
    };
    let coded_size = coded_plane_size(frame.info(), plane)?;
    if x >= coded_size.width() || y >= coded_size.height() {
        return Err(ReconError::LoopRestorationSourceSampleOutOfBounds {
            plane,
            x,
            y,
            width: coded_size.width(),
            height: coded_size.height(),
        });
    }
    if plane_ref.stride_samples() < coded_size.width() {
        return Err(ReconError::StrideTooSmall {
            stride_samples: plane_ref.stride_samples(),
            storage_width: coded_size.width(),
        });
    }

    let row_start =
        y.checked_mul(plane_ref.stride_samples())
            .ok_or(ReconError::ArithmeticOverflow {
                context: "loop restoration source row offset",
            })?;
    let sample_index = row_start
        .checked_add(x)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "loop restoration source sample offset",
        })?;
    let required_len = sample_index
        .checked_add(1)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "loop restoration source sample length",
        })?;
    let Some(sample) = plane_ref.samples().get(sample_index) else {
        return Err(ReconError::BufferLengthMismatch {
            expected: required_len,
            actual: plane_ref.samples().len(),
        });
    };

    let value = sample.to_u16();
    let max = frame.info().bit_depth().max_sample();
    if value > max {
        return Err(ReconError::SampleOutOfRange {
            plane,
            sample_index,
            value,
            max,
        });
    }
    Ok(*sample)
}

fn coded_plane_size(info: DecodedFrameInfo, plane: PlaneId) -> Result<PlaneSize> {
    match plane {
        PlaneId::Y => Ok(info.coded_luma_size()),
        PlaneId::U | PlaneId::V => info
            .pixel_format()
            .chroma_size(info.coded_luma_size())?
            .ok_or(ReconError::MissingChromaPlane { plane }),
    }
}

fn validate_source_bounds(bounds: &LoopRestorationSourceBounds) -> Result<()> {
    if bounds.subsampling_x > 1 || bounds.subsampling_y > 1 {
        return Err(ReconError::LoopRestorationSourceInvalidSubsampling {
            subsampling_x: bounds.subsampling_x,
            subsampling_y: bounds.subsampling_y,
        });
    }
    if bounds.luma_start_x > bounds.luma_end_x {
        return Err(ReconError::LoopRestorationSourceInvalidBounds {
            field: "luma x range",
        });
    }
    if bounds.luma_start_y > bounds.luma_end_y {
        return Err(ReconError::LoopRestorationSourceInvalidBounds {
            field: "luma y range",
        });
    }
    if bounds.luma_stripe_start_y > bounds.luma_stripe_end_y {
        return Err(ReconError::LoopRestorationSourceInvalidBounds {
            field: "luma stripe y range",
        });
    }
    if bounds.luma_stripe_start_y < bounds.luma_start_y
        || bounds.luma_stripe_end_y > bounds.luma_end_y
    {
        return Err(ReconError::LoopRestorationSourceInvalidBounds {
            field: "luma stripe y bounds",
        });
    }
    Ok(())
}

const fn plane_subsampling(plane: PlaneId, bounds: &LoopRestorationSourceBounds) -> (u8, u8) {
    match plane {
        PlaneId::Y => (0, 0),
        PlaneId::U | PlaneId::V => (bounds.subsampling_x, bounds.subsampling_y),
    }
}

fn shifted_bound(value: usize, shift: u8, context: &'static str) -> Result<isize> {
    let shifted = value >> usize::from(shift);
    isize::try_from(shifted).map_err(|_| ReconError::ArithmeticOverflow { context })
}

const fn clip3(value: isize, min: isize, max: isize) -> isize {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{BitDepth, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneRect, PlaneSize};

    const BOUNDS_420: LoopRestorationSourceBounds = LoopRestorationSourceBounds {
        luma_start_x: 0,
        luma_end_x: 31,
        luma_start_y: 0,
        luma_end_y: 63,
        luma_stripe_start_y: 8,
        luma_stripe_end_y: 55,
        subsampling_x: 1,
        subsampling_y: 1,
    };

    fn size(width: usize, height: usize) -> PlaneSize {
        PlaneSize::new(width, height).unwrap()
    }

    fn rect(x: usize, y: usize, width: usize, height: usize) -> PlaneRect {
        PlaneRect::new(x, y, width, height).unwrap()
    }

    fn info_with_bit_depth(
        bit_depth: BitDepth,
        pixel_format: PixelFormat,
        coded: PlaneSize,
        visible: PlaneRect,
    ) -> DecodedFrameInfo {
        DecodedFrameInfo::new(OutputIndex::new(0), bit_depth, pixel_format, coded, visible).unwrap()
    }

    fn info(pixel_format: PixelFormat, coded: PlaneSize, visible: PlaneRect) -> DecodedFrameInfo {
        info_with_bit_depth(BitDepth::Eight, pixel_format, coded, visible)
    }

    fn yuv420_frame<'a>(
        frame_info: DecodedFrameInfo,
        y: &'a [u8],
        u: &'a [u8],
        v: &'a [u8],
    ) -> FrameRef<'a, u8> {
        FrameRef::new(
            frame_info,
            crate::PlaneRef::new(y, 4, rect(0, 0, 4, 4)).unwrap(),
            Some(crate::PlaneRef::new(u, 2, rect(0, 0, 2, 2)).unwrap()),
            Some(crate::PlaneRef::new(v, 2, rect(0, 0, 2, 2)).unwrap()),
        )
        .unwrap()
    }

    fn monochrome_frame<'a>(
        frame_info: DecodedFrameInfo,
        y: &'a [u8],
        stride_samples: usize,
        visible_rect: PlaneRect,
    ) -> FrameRef<'a, u8> {
        FrameRef::new(
            frame_info,
            crate::PlaneRef::new(y, stride_samples, visible_rect).unwrap(),
            None,
            None,
        )
        .unwrap()
    }

    fn monochrome_frame_u16<'a>(
        frame_info: DecodedFrameInfo,
        y: &'a [u16],
        stride_samples: usize,
        visible_rect: PlaneRect,
    ) -> FrameRef<'a, u16> {
        FrameRef::new(
            frame_info,
            crate::PlaneRef::new(y, stride_samples, visible_rect).unwrap(),
            None,
            None,
        )
        .unwrap()
    }

    #[test]
    fn luma_inside_stripe_reads_cdef_frame() {
        let sample = loop_restoration_source_sample(PlaneId::Y, 12, 16, &BOUNDS_420).unwrap();

        assert_eq!(
            sample,
            LoopRestorationSourceSample {
                x: 12,
                y: 16,
                source: LoopRestorationSource::CdefFrame,
            }
        );
    }

    #[test]
    fn luma_above_stripe_reads_curr_frame_clamped_to_two_lines() {
        let sample = loop_restoration_source_sample(PlaneId::Y, -20, -20, &BOUNDS_420).unwrap();

        assert_eq!(
            sample,
            LoopRestorationSourceSample {
                x: 0,
                y: 6,
                source: LoopRestorationSource::CurrFrame,
            }
        );
    }

    #[test]
    fn luma_below_stripe_reads_curr_frame_clamped_to_two_lines() {
        let sample = loop_restoration_source_sample(PlaneId::Y, 40, 200, &BOUNDS_420).unwrap();

        assert_eq!(
            sample,
            LoopRestorationSourceSample {
                x: 31,
                y: 57,
                source: LoopRestorationSource::CurrFrame,
            }
        );
    }

    #[test]
    fn chroma_uses_subsampled_bounds_and_stripe() {
        let sample = loop_restoration_source_sample(PlaneId::U, 40, 40, &BOUNDS_420).unwrap();

        assert_eq!(
            sample,
            LoopRestorationSourceSample {
                x: 15,
                y: 29,
                source: LoopRestorationSource::CurrFrame,
            }
        );
    }

    #[test]
    fn luma_ignores_sequence_subsampling() {
        let sample = loop_restoration_source_sample(PlaneId::Y, 40, 40, &BOUNDS_420).unwrap();

        assert_eq!(
            sample,
            LoopRestorationSourceSample {
                x: 31,
                y: 40,
                source: LoopRestorationSource::CdefFrame,
            }
        );
    }

    #[test]
    fn rejects_invalid_subsampling() {
        let bounds = LoopRestorationSourceBounds {
            subsampling_x: 2,
            ..BOUNDS_420
        };

        let err = loop_restoration_source_sample(PlaneId::U, 0, 0, &bounds).unwrap_err();

        assert_eq!(
            err,
            ReconError::LoopRestorationSourceInvalidSubsampling {
                subsampling_x: 2,
                subsampling_y: 1,
            }
        );
    }

    #[test]
    fn rejects_invalid_luma_range() {
        let bounds = LoopRestorationSourceBounds {
            luma_start_x: 32,
            luma_end_x: 31,
            ..BOUNDS_420
        };

        let err = loop_restoration_source_sample(PlaneId::Y, 0, 0, &bounds).unwrap_err();

        assert_eq!(
            err,
            ReconError::LoopRestorationSourceInvalidBounds {
                field: "luma x range",
            }
        );
    }

    #[test]
    fn rejects_stripe_outside_luma_range() {
        let bounds = LoopRestorationSourceBounds {
            luma_stripe_end_y: 64,
            ..BOUNDS_420
        };

        let err = loop_restoration_source_sample(PlaneId::Y, 0, 0, &bounds).unwrap_err();

        assert_eq!(
            err,
            ReconError::LoopRestorationSourceInvalidBounds {
                field: "luma stripe y bounds",
            }
        );
    }

    #[test]
    fn rejects_inverted_stripe_range() {
        let bounds = LoopRestorationSourceBounds {
            luma_stripe_start_y: 56,
            luma_stripe_end_y: 55,
            ..BOUNDS_420
        };

        let err = loop_restoration_source_sample(PlaneId::Y, 0, 0, &bounds).unwrap_err();

        assert_eq!(
            err,
            ReconError::LoopRestorationSourceInvalidBounds {
                field: "luma stripe y range",
            }
        );
    }

    #[test]
    fn rejects_unrepresentable_luma_bound() {
        let bounds = LoopRestorationSourceBounds {
            luma_end_x: usize::MAX,
            ..BOUNDS_420
        };

        let err = loop_restoration_source_sample(PlaneId::Y, 0, 0, &bounds).unwrap_err();

        assert_eq!(
            err,
            ReconError::ArithmeticOverflow {
                context: "loop restoration max x",
            }
        );
    }

    #[test]
    fn sample_value_reads_cdef_frame_inside_stripe() {
        let frame_info = info(PixelFormat::Yuv420, size(4, 4), rect(0, 0, 4, 4));
        let curr_y = [10_u8; 16];
        let cdef_y = [
            100, 101, 102, 103, 110, 111, 112, 113, 120, 121, 122, 123, 130, 131, 132, 133,
        ];
        let curr_uv = [20_u8; 4];
        let cdef_u = [30_u8; 4];
        let cdef_v = [40_u8; 4];
        let curr = yuv420_frame(frame_info, &curr_y, &curr_uv, &curr_uv);
        let cdef = yuv420_frame(frame_info, &cdef_y, &cdef_u, &cdef_v);
        let bounds = LoopRestorationSourceBounds {
            luma_start_x: 0,
            luma_end_x: 3,
            luma_start_y: 0,
            luma_end_y: 3,
            luma_stripe_start_y: 0,
            luma_stripe_end_y: 3,
            subsampling_x: 1,
            subsampling_y: 1,
        };

        let sample =
            loop_restoration_source_sample_value(PlaneId::Y, 2, 1, &bounds, curr, cdef).unwrap();

        assert_eq!(
            sample,
            LoopRestorationSourceSampleValue {
                sample: LoopRestorationSourceSample {
                    x: 2,
                    y: 1,
                    source: LoopRestorationSource::CdefFrame,
                },
                value: 112,
            }
        );
    }

    #[test]
    fn sample_value_reads_curr_frame_above_stripe_after_two_line_clamp() {
        let frame_info = info(PixelFormat::Monochrome, size(4, 8), rect(0, 0, 4, 8));
        let curr_y = [
            0, 1, 2, 3, 10, 11, 12, 13, 20, 21, 22, 23, 30, 31, 32, 33, 40, 41, 42, 43, 50, 51, 52,
            53, 60, 61, 62, 63, 70, 71, 72, 73,
        ];
        let cdef_y = [200_u8; 32];
        let curr = monochrome_frame(frame_info, &curr_y, 4, rect(0, 0, 4, 8));
        let cdef = monochrome_frame(frame_info, &cdef_y, 4, rect(0, 0, 4, 8));
        let bounds = LoopRestorationSourceBounds {
            luma_start_x: 0,
            luma_end_x: 3,
            luma_start_y: 0,
            luma_end_y: 7,
            luma_stripe_start_y: 4,
            luma_stripe_end_y: 7,
            subsampling_x: 0,
            subsampling_y: 0,
        };

        let sample =
            loop_restoration_source_sample_value(PlaneId::Y, 1, -5, &bounds, curr, cdef).unwrap();

        assert_eq!(
            sample,
            LoopRestorationSourceSampleValue {
                sample: LoopRestorationSourceSample {
                    x: 1,
                    y: 2,
                    source: LoopRestorationSource::CurrFrame,
                },
                value: 21,
            }
        );
    }

    #[test]
    fn sample_value_reads_chroma_cdef_frame_with_subsampled_bounds() {
        let frame_info = info(PixelFormat::Yuv420, size(4, 4), rect(0, 0, 4, 4));
        let curr_y = [10_u8; 16];
        let cdef_y = [11_u8; 16];
        let curr_u = [1, 2, 3, 4];
        let curr_v = [5, 6, 7, 8];
        let cdef_u = [20, 21, 22, 23];
        let cdef_v = [30, 31, 32, 33];
        let curr = yuv420_frame(frame_info, &curr_y, &curr_u, &curr_v);
        let cdef = yuv420_frame(frame_info, &cdef_y, &cdef_u, &cdef_v);
        let bounds = LoopRestorationSourceBounds {
            luma_start_x: 0,
            luma_end_x: 3,
            luma_start_y: 0,
            luma_end_y: 3,
            luma_stripe_start_y: 0,
            luma_stripe_end_y: 3,
            subsampling_x: 1,
            subsampling_y: 1,
        };

        let sample =
            loop_restoration_source_sample_value(PlaneId::U, 3, 3, &bounds, curr, cdef).unwrap();

        assert_eq!(
            sample,
            LoopRestorationSourceSampleValue {
                sample: LoopRestorationSourceSample {
                    x: 1,
                    y: 1,
                    source: LoopRestorationSource::CdefFrame,
                },
                value: 23,
            }
        );
    }

    #[test]
    fn sample_value_reads_coded_storage_coordinates_despite_visible_rect_origin() {
        let frame_info = info(PixelFormat::Monochrome, size(4, 4), rect(1, 1, 2, 2));
        let curr_y = [0_u8; 16];
        let cdef_y = [0, 1, 2, 3, 10, 11, 12, 13, 20, 21, 22, 23, 30, 31, 32, 33];
        let curr = monochrome_frame(frame_info, &curr_y, 4, rect(1, 1, 2, 2));
        let cdef = monochrome_frame(frame_info, &cdef_y, 4, rect(1, 1, 2, 2));
        let bounds = LoopRestorationSourceBounds {
            luma_start_x: 0,
            luma_end_x: 3,
            luma_start_y: 0,
            luma_end_y: 3,
            luma_stripe_start_y: 0,
            luma_stripe_end_y: 3,
            subsampling_x: 0,
            subsampling_y: 0,
        };

        let sample =
            loop_restoration_source_sample_value(PlaneId::Y, 3, 3, &bounds, curr, cdef).unwrap();

        assert_eq!(
            sample,
            LoopRestorationSourceSampleValue {
                sample: LoopRestorationSourceSample {
                    x: 3,
                    y: 3,
                    source: LoopRestorationSource::CdefFrame,
                },
                value: 33,
            }
        );
    }

    #[test]
    fn sample_value_rejects_mismatched_frame_info() {
        let curr_info = info(PixelFormat::Monochrome, size(4, 4), rect(0, 0, 4, 4));
        let cdef_info = DecodedFrameInfo::new(
            OutputIndex::new(1),
            BitDepth::Eight,
            PixelFormat::Monochrome,
            size(4, 4),
            rect(0, 0, 4, 4),
        )
        .unwrap();
        let curr_y = [10_u8; 16];
        let cdef_y = [20_u8; 16];
        let curr = monochrome_frame(curr_info, &curr_y, 4, rect(0, 0, 4, 4));
        let cdef = monochrome_frame(cdef_info, &cdef_y, 4, rect(0, 0, 4, 4));
        let bounds = LoopRestorationSourceBounds {
            luma_start_x: 0,
            luma_end_x: 3,
            luma_start_y: 0,
            luma_end_y: 3,
            luma_stripe_start_y: 0,
            luma_stripe_end_y: 3,
            subsampling_x: 0,
            subsampling_y: 0,
        };

        let err = loop_restoration_source_sample_value(PlaneId::Y, 0, 0, &bounds, curr, cdef)
            .unwrap_err();

        assert_eq!(
            err,
            ReconError::LoopRestorationSourceFrameMismatch {
                field: "frame metadata",
            }
        );
    }

    #[test]
    fn sample_value_rejects_mismatched_plane_view_geometry() {
        let frame_info = info(PixelFormat::Monochrome, size(4, 4), rect(0, 0, 2, 2));
        let curr_y = [10_u8; 16];
        let cdef_y = [20_u8; 16];
        let curr = monochrome_frame(frame_info, &curr_y, 4, rect(0, 0, 2, 2));
        let cdef = monochrome_frame(frame_info, &cdef_y, 4, rect(1, 1, 2, 2));
        let bounds = LoopRestorationSourceBounds {
            luma_start_x: 0,
            luma_end_x: 1,
            luma_start_y: 0,
            luma_end_y: 1,
            luma_stripe_start_y: 0,
            luma_stripe_end_y: 1,
            subsampling_x: 0,
            subsampling_y: 0,
        };

        let err = loop_restoration_source_sample_value(PlaneId::Y, 0, 0, &bounds, curr, cdef)
            .unwrap_err();

        assert_eq!(
            err,
            ReconError::LoopRestorationSourceFrameMismatch {
                field: "plane view geometry",
            }
        );
    }

    #[test]
    fn sample_value_rejects_sample_outside_coded_plane() {
        let frame_info = info(PixelFormat::Monochrome, size(4, 4), rect(0, 0, 4, 4));
        let curr_y = [10_u8; 16];
        let cdef_y = [20_u8; 16];
        let curr = monochrome_frame(frame_info, &curr_y, 4, rect(0, 0, 4, 4));
        let cdef = monochrome_frame(frame_info, &cdef_y, 4, rect(0, 0, 4, 4));
        let bounds = LoopRestorationSourceBounds {
            luma_start_x: 0,
            luma_end_x: 3,
            luma_start_y: 0,
            luma_end_y: 7,
            luma_stripe_start_y: 0,
            luma_stripe_end_y: 7,
            subsampling_x: 0,
            subsampling_y: 0,
        };

        let err = loop_restoration_source_sample_value(PlaneId::Y, 0, 6, &bounds, curr, cdef)
            .unwrap_err();

        assert_eq!(
            err,
            ReconError::LoopRestorationSourceSampleOutOfBounds {
                plane: PlaneId::Y,
                x: 0,
                y: 6,
                width: 4,
                height: 4,
            }
        );
    }

    #[test]
    fn sample_value_rejects_u8_storage_for_ten_bit_frame() {
        let frame_info = info_with_bit_depth(
            BitDepth::Ten,
            PixelFormat::Monochrome,
            size(4, 4),
            rect(0, 0, 4, 4),
        );
        let curr_y = [10_u8; 16];
        let cdef_y = [20_u8; 16];
        let curr = monochrome_frame(frame_info, &curr_y, 4, rect(0, 0, 4, 4));
        let cdef = monochrome_frame(frame_info, &cdef_y, 4, rect(0, 0, 4, 4));
        let bounds = LoopRestorationSourceBounds {
            luma_start_x: 0,
            luma_end_x: 3,
            luma_start_y: 0,
            luma_end_y: 3,
            luma_stripe_start_y: 0,
            luma_stripe_end_y: 3,
            subsampling_x: 0,
            subsampling_y: 0,
        };

        let err = loop_restoration_source_sample_value(PlaneId::Y, 0, 0, &bounds, curr, cdef)
            .unwrap_err();

        assert_eq!(
            err,
            ReconError::SampleTypeUnsupportedBitDepth {
                sample_type: "u8",
                bit_depth: BitDepth::Ten,
            }
        );
    }

    #[test]
    fn sample_value_rejects_source_sample_above_bit_depth() {
        let frame_info = info_with_bit_depth(
            BitDepth::Ten,
            PixelFormat::Monochrome,
            size(4, 4),
            rect(0, 0, 4, 4),
        );
        let curr_y = [0_u16; 16];
        let mut cdef_y = [0_u16; 16];
        cdef_y[0] = 1024;
        let curr = monochrome_frame_u16(frame_info, &curr_y, 4, rect(0, 0, 4, 4));
        let cdef = monochrome_frame_u16(frame_info, &cdef_y, 4, rect(0, 0, 4, 4));
        let bounds = LoopRestorationSourceBounds {
            luma_start_x: 0,
            luma_end_x: 3,
            luma_start_y: 0,
            luma_end_y: 3,
            luma_stripe_start_y: 0,
            luma_stripe_end_y: 3,
            subsampling_x: 0,
            subsampling_y: 0,
        };

        let err = loop_restoration_source_sample_value(PlaneId::Y, 0, 0, &bounds, curr, cdef)
            .unwrap_err();

        assert_eq!(
            err,
            ReconError::SampleOutOfRange {
                plane: PlaneId::Y,
                sample_index: 0,
                value: 1024,
                max: 1023,
            }
        );
    }

    #[test]
    fn sample_value_rejects_missing_chroma_plane() {
        let frame_info = info(PixelFormat::Monochrome, size(4, 4), rect(0, 0, 4, 4));
        let curr_y = [10_u8; 16];
        let cdef_y = [20_u8; 16];
        let curr = monochrome_frame(frame_info, &curr_y, 4, rect(0, 0, 4, 4));
        let cdef = monochrome_frame(frame_info, &cdef_y, 4, rect(0, 0, 4, 4));
        let bounds = LoopRestorationSourceBounds {
            luma_start_x: 0,
            luma_end_x: 3,
            luma_start_y: 0,
            luma_end_y: 3,
            luma_stripe_start_y: 0,
            luma_stripe_end_y: 3,
            subsampling_x: 0,
            subsampling_y: 0,
        };

        let err = loop_restoration_source_sample_value(PlaneId::U, 0, 0, &bounds, curr, cdef)
            .unwrap_err();

        assert_eq!(err, ReconError::MissingChromaPlane { plane: PlaneId::U });
    }
}
