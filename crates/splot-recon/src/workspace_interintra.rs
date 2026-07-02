// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.3.29 intra mode variant mask and § 7.13.3.30 interintra blend.

use super::{CurrentFramePlane, CurrentFrameWorkspace, block_rect};
use crate::{IntraRectBlockSize, PlaneId, ReconError, ReconSample, Result};

/// AV2 § 7.13.3.29 `Ii_Weights_1d[128]` smooth-interintra blending weights.
const II_WEIGHTS_1D: [u16; 128] = [
    60, 58, 56, 54, 52, 50, 48, 47, 45, 44, 42, 41, 39, 38, 37, 35, 34, 33, 32, 31, 30, 29, 28, 27,
    26, 25, 24, 23, 22, 22, 21, 20, 19, 19, 18, 18, 17, 16, 16, 15, 15, 14, 14, 13, 13, 12, 12, 12,
    11, 11, 10, 10, 10, 9, 9, 9, 8, 8, 8, 8, 7, 7, 7, 7, 6, 6, 6, 6, 6, 5, 5, 5, 5, 5, 4, 4, 4, 4,
    4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
];

/// The § 5.20.7.15 `interintra_mode` selecting the § 7.13.3.29 mask variant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterIntraMode {
    /// `II_DC_PRED`: constant weight 32.
    Dc,
    /// `II_V_PRED`: weights decay downward from the above edge.
    Vertical,
    /// `II_H_PRED`: weights decay rightward from the left edge.
    Horizontal,
    /// `II_SMOOTH_PRED`: weights decay along `Min(i, j)`.
    Smooth,
}

/// Returns the § 7.13.3.29 `Mask[i][j]` blending weight for one sample.
fn intra_mode_variant_weight(mode: InterIntraMode, i: usize, j: usize, size_scale: usize) -> u16 {
    let index = match mode {
        InterIntraMode::Dc => return 32,
        InterIntraMode::Vertical => i * size_scale,
        InterIntraMode::Horizontal => j * size_scale,
        InterIntraMode::Smooth => i.min(j) * size_scale,
    };
    II_WEIGHTS_1D[index.min(II_WEIGHTS_1D.len() - 1)]
}

impl<T: ReconSample> CurrentFrameWorkspace<T> {
    /// Blends caller-supplied intra prediction samples over the in-storage
    /// inter prediction with the AV2 § 7.13.3.29 smooth-interintra mask:
    /// `CurrFrame = Round2(m * IntraPred + (64 - m) * CurrFrame, 6)`
    /// (§ 7.13.3.30 interintra arm; the mask is regenerated at this plane's
    /// dimensions, so no chroma subsampling applies).
    ///
    /// # Errors
    /// Returns [`ReconError`] when the plane is absent, the target rectangle
    /// is out of bounds, `intra.len()` is not the rectangular sample count, or
    /// a blended sample cannot be stored.
    pub fn blend_smooth_interintra_rect(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
        mode: InterIntraMode,
        intra: &[T],
    ) -> Result<()> {
        if intra.len() != size.sample_count() {
            return Err(ReconError::WorkspaceWriteLengthMismatch {
                plane,
                expected: size.sample_count(),
                actual: intra.len(),
            });
        }
        let rect = block_rect(x, y, size)?;
        self.plane_mut(plane)?
            .blend_smooth_interintra_rect(rect, size, mode, intra)
    }
}

impl<T: ReconSample> CurrentFramePlane<T> {
    fn blend_smooth_interintra_rect(
        &mut self,
        rect: crate::PlaneRect,
        size: IntraRectBlockSize,
        mode: InterIntraMode,
        intra: &[T],
    ) -> Result<()> {
        self.ensure_rect(rect)?;
        let size_scale = 128 / size.width().max(size.height());
        for i in 0..size.height() {
            let row_start = self.sample_index(rect.x(), rect.y() + i)?;
            for j in 0..size.width() {
                let m = intra_mode_variant_weight(mode, i, j, size_scale);
                let pred0 = u32::from(intra[i * size.width() + j].to_u16());
                let pred1 = u32::from(self.samples[row_start + j].to_u16());
                let blended = (u32::from(m) * pred0 + (64 - u32::from(m)) * pred1 + 32) >> 6;
                let stored = u16::try_from(blended)
                    .ok()
                    .and_then(|value| T::try_from_u16(value).ok())
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "interintra blend sample storage",
                    })?;
                self.samples[row_start + j] = stored;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn ii_weights_table_matches_the_spec_end_points() {
        assert_eq!(II_WEIGHTS_1D[0], 60);
        assert_eq!(II_WEIGHTS_1D[18], 32);
        assert_eq!(II_WEIGHTS_1D[127], 1);
        assert_eq!(II_WEIGHTS_1D.len(), 128);
    }

    #[test]
    fn dc_mask_is_constant_32() {
        for (i, j) in [(0, 0), (3, 7), (63, 63)] {
            assert_eq!(intra_mode_variant_weight(InterIntraMode::Dc, i, j, 2), 32);
        }
    }

    #[test]
    fn directional_masks_index_the_expected_axis() {
        assert_eq!(
            intra_mode_variant_weight(InterIntraMode::Vertical, 4, 0, 2),
            II_WEIGHTS_1D[8]
        );
        assert_eq!(
            intra_mode_variant_weight(InterIntraMode::Horizontal, 0, 4, 2),
            II_WEIGHTS_1D[8]
        );
        assert_eq!(
            intra_mode_variant_weight(InterIntraMode::Smooth, 9, 4, 2),
            II_WEIGHTS_1D[8]
        );
    }
}
