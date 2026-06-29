// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Private encoder residual calculation for future transform input.
//!
//! This module advances `ENC-RESIDUAL-FOUNDATION`. It computes the
//! non-normative encoder-side signal `source_sample - prediction_sample` for a
//! checked block. The sign convention is chosen to feed later forward transform
//! work whose closed-loop inverse path eventually reconstructs through AV2
//! §7.14.3 residual addition
//! (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-14-3`).
//!
//! The module does not emit syntax, own a writer, or produce [`crate::Packet`]
//! values.

#![allow(dead_code)]

use splot_recon::{PlaneId, PlaneRect, PlaneRef};

use crate::error::{Error, Result};

/// Row-major signed residual samples for one encoder block.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ResidualBlock {
    plane: PlaneId,
    block: PlaneRect,
    samples: Vec<i16>,
}

impl ResidualBlock {
    /// Computes `source_sample - prediction_sample` for a checked visible block.
    ///
    /// `block` coordinates are relative to the input plane's visible rectangle,
    /// not the backing allocation. `prediction` is row-strided block prediction
    /// data; rows may contain padding after the selected block width.
    pub(crate) fn from_plane_prediction(
        plane: PlaneId,
        input: PlaneRef<'_, u8>,
        block: PlaneRect,
        prediction: &[u8],
        prediction_stride_samples: usize,
    ) -> Result<Self> {
        validate_block_inside_visible_plane(plane, input, block)?;
        if prediction_stride_samples < block.width() {
            return Err(Error::ResidualPredictionStrideTooSmall {
                plane,
                stride_samples: prediction_stride_samples,
                width: block.width(),
            });
        }

        let expected_prediction =
            required_prediction_samples(plane, block, prediction_stride_samples)?;
        if prediction.len() < expected_prediction {
            return Err(Error::ResidualPredictionLengthMismatch {
                plane,
                expected: expected_prediction,
                actual: prediction.len(),
            });
        }

        let sample_count = block
            .width()
            .checked_mul(block.height())
            .ok_or(Error::ResidualSampleCountOverflow { plane, block })?;
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(sample_count)
            .map_err(|_| Error::ResidualAllocationFailed {
                plane,
                context: "residual block samples",
            })?;

        for (row_index, source_row) in input
            .visible_rows()
            .skip(block.y())
            .take(block.height())
            .enumerate()
        {
            let source = &source_row[block.x()..block.x() + block.width()];
            let prediction_start = row_index.checked_mul(prediction_stride_samples).ok_or(
                Error::ResidualPredictionSpanOverflow {
                    plane,
                    block,
                    stride_samples: prediction_stride_samples,
                },
            )?;
            let prediction_row = &prediction[prediction_start..prediction_start + block.width()];
            for (&source_sample, &prediction_sample) in source.iter().zip(prediction_row) {
                // splot-copy-ok: materialize signed residual samples as the
                samples.push(i16::from(source_sample) - i16::from(prediction_sample));
            }
        }

        debug_assert_eq!(samples.len(), sample_count);
        Ok(Self {
            plane,
            block,
            samples,
        })
    }

    /// Returns the source plane identity.
    pub(crate) const fn plane(&self) -> PlaneId {
        self.plane
    }

    /// Returns the visible-plane-relative block rectangle.
    pub(crate) const fn block(&self) -> PlaneRect {
        self.block
    }

    /// Returns row-major signed residual samples.
    pub(crate) fn samples(&self) -> &[i16] {
        &self.samples
    }
}

fn validate_block_inside_visible_plane(
    plane: PlaneId,
    input: PlaneRef<'_, u8>,
    block: PlaneRect,
) -> Result<()> {
    let visible_size = input.visible_size();
    if block.is_within(visible_size) {
        Ok(())
    } else {
        Err(Error::ResidualBlockOutOfBounds {
            plane,
            block,
            visible_size,
        })
    }
}

fn required_prediction_samples(
    plane: PlaneId,
    block: PlaneRect,
    prediction_stride_samples: usize,
) -> Result<usize> {
    let last_row = block.height() - 1;
    last_row
        .checked_mul(prediction_stride_samples)
        .and_then(|offset| offset.checked_add(block.width()))
        .ok_or(Error::ResidualPredictionSpanOverflow {
            plane,
            block,
            stride_samples: prediction_stride_samples,
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use splot_recon::PlaneSize;

    fn size(width: usize, height: usize) -> PlaneSize {
        PlaneSize::new(width, height).unwrap()
    }

    fn rect(x: usize, y: usize, width: usize, height: usize) -> PlaneRect {
        PlaneRect::new(x, y, width, height).unwrap()
    }

    fn plane_ref(samples: &[u8], stride_samples: usize, visible: PlaneRect) -> PlaneRef<'_, u8> {
        PlaneRef::new(samples, stride_samples, visible).unwrap()
    }

    #[test]
    fn zero_residual_when_prediction_matches_source() {
        let source = [1_u8, 2, 3, 4];
        let prediction = [1_u8, 2, 3, 4];
        let residual = ResidualBlock::from_plane_prediction(
            PlaneId::Y,
            plane_ref(&source, 2, rect(0, 0, 2, 2)),
            rect(0, 0, 2, 2),
            &prediction,
            2,
        )
        .unwrap();

        assert_eq!(residual.plane(), PlaneId::Y);
        assert_eq!(residual.block(), rect(0, 0, 2, 2));
        assert_eq!(residual.samples(), &[0, 0, 0, 0]);
    }

    #[test]
    fn min_max_differences_are_signed_not_wrapping() {
        let source = [0_u8, 255, 128, 64];
        let prediction = [255_u8, 0, 128, 100];
        let residual = ResidualBlock::from_plane_prediction(
            PlaneId::Y,
            plane_ref(&source, 2, rect(0, 0, 2, 2)),
            rect(0, 0, 2, 2),
            &prediction,
            2,
        )
        .unwrap();

        assert_eq!(residual.samples(), &[-255, 255, 0, -36]);
    }

    #[test]
    fn checkerboard_and_gradient_block_is_row_major() {
        let source = [0_u8, 255, 32, 96, 255, 0, 64, 128, 10, 20, 30, 40];
        let prediction = [10_u8, 10, 10, 10, 50, 50, 50, 50];
        let residual = ResidualBlock::from_plane_prediction(
            PlaneId::Y,
            plane_ref(&source, 4, rect(0, 0, 4, 3)),
            rect(0, 0, 4, 2),
            &prediction,
            4,
        )
        .unwrap();

        assert_eq!(residual.samples(), &[-10, 245, 22, 86, 205, -50, 14, 78]);
    }

    #[test]
    fn odd_edge_block_honors_input_and_prediction_stride_padding() {
        let source = [
            0_u8, 1, 2, 3, 4, 99, 10, 11, 12, 13, 14, 99, 20, 21, 22, 23, 24, 99, 30, 31, 32, 33,
            34, 99, 40, 41, 42, 43, 44, 99,
        ];
        let prediction = [
            7_u8, 8, 9, 200, 200, 200, 200, 200, 17, 18, 19, 201, 201, 201, 201, 201, 27, 28, 29,
            202, 202, 202, 202, 202,
        ];
        let residual = ResidualBlock::from_plane_prediction(
            PlaneId::U,
            plane_ref(&source, 6, rect(0, 0, 5, 5)),
            rect(2, 2, 3, 3),
            &prediction,
            8,
        )
        .unwrap();

        assert_eq!(residual.block(), rect(2, 2, 3, 3));
        assert_eq!(residual.samples(), &[15, 15, 15, 15, 15, 15, 15, 15, 15]);
    }

    #[test]
    fn rejects_block_outside_visible_plane() {
        let source = [0_u8; 16];
        let err = ResidualBlock::from_plane_prediction(
            PlaneId::V,
            plane_ref(&source, 4, rect(0, 0, 4, 4)),
            rect(3, 0, 2, 2),
            &[0; 4],
            2,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            Error::ResidualBlockOutOfBounds {
                plane: PlaneId::V,
                visible_size,
                ..
            } if visible_size == size(4, 4)
        ));
    }

    #[test]
    fn rejects_prediction_stride_smaller_than_block_width() {
        let source = [0_u8; 16];
        let err = ResidualBlock::from_plane_prediction(
            PlaneId::Y,
            plane_ref(&source, 4, rect(0, 0, 4, 4)),
            rect(0, 0, 3, 2),
            &[0; 6],
            2,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            Error::ResidualPredictionStrideTooSmall {
                plane: PlaneId::Y,
                stride_samples: 2,
                width: 3
            }
        ));
    }

    #[test]
    fn rejects_truncated_prediction_buffer() {
        let source = [0_u8; 16];
        let err = ResidualBlock::from_plane_prediction(
            PlaneId::Y,
            plane_ref(&source, 4, rect(0, 0, 4, 4)),
            rect(0, 0, 3, 3),
            &[0; 10],
            4,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            Error::ResidualPredictionLengthMismatch {
                plane: PlaneId::Y,
                expected: 11,
                actual: 10
            }
        ));
    }
}
