// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.20 loop-restoration caller-resolved helpers.
//!
//! This module implements the scheduler-free AV2 § 7.20.2 source-sample
//! coordinate and source-frame selection process
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md#s-7-20-2)).
//! It does not read frame storage itself. Instead, it returns the clipped plane
//! coordinates and whether the caller must read `CurrFrame` or `CdefFrame`.
//!
//! Feature tracking: `RECON-LOOP-RESTORATION-SOURCE-SAMPLE`.

use crate::{PlaneId, ReconError, Result};

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
/// samples always use zero subsampling regardless of these fields.
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
}
