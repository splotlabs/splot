// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Data-driven intra prediction primitive.
//!
//! Feature tracking: `DECODE-GENERAL-INTRA-FRAME-FRONTIER`.

use crate::intra_dc_math::{
    IntraEdgeError, validate_intra_edge_samples, validate_output_shape, validate_sample_type,
};
use crate::math::round2_i32;
use crate::{BitDepth, IntraRectBlockSize, ReconError, ReconSample, Result};

const DIP_FEATURE_COUNT: usize = 11;
const DIP_GRID_SIDE: usize = 8;
const DIP_MODE_COUNT: usize = 6;

/// Edge identifier for DIP intra prediction inputs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum IntraDipEdge {
    /// Left edge samples, including the bottom-left DIP extension.
    Left,
    /// Above edge samples, including the top-right DIP extension.
    Above,
    /// Top-left corner sample `AboveRow[-1]`.
    TopLeft,
}

impl IntraDipEdge {
    /// Returns a stable human-readable edge name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Above => "above",
            Self::TopLeft => "top-left",
        }
    }
}

impl IntraEdgeError for IntraDipEdge {
    fn length_mismatch(self, expected: usize, actual: usize) -> ReconError {
        ReconError::IntraDipEdgeLengthMismatch {
            edge: self,
            expected,
            actual,
        }
    }

    fn sample_out_of_range(self, sample_index: usize, value: u16, max: u16) -> ReconError {
        ReconError::IntraDipSampleOutOfRange {
            edge: self,
            sample_index,
            value,
            max,
        }
    }
}

/// Caller-provided prepared edge samples for AV2 §7.13.2.3 DIP prediction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntraDipEdges<'a, T: ReconSample> {
    left: &'a [T],
    above: &'a [T],
    top_left: T,
}

impl<'a, T: ReconSample> IntraDipEdges<'a, T> {
    /// Creates a prepared DIP edge set.
    ///
    /// `left` must contain `height + height / 4` samples and `above` must
    /// contain `width + width / 4` samples. Availability and fallback
    /// preparation remain outside this type and are owned by the broader AV2
    /// §7.13.2.1 intra process.
    pub const fn new(left: &'a [T], above: &'a [T], top_left: T) -> Self {
        Self {
            left,
            above,
            top_left,
        }
    }

    /// Returns prepared left edge samples.
    pub const fn left_samples(self) -> &'a [T] {
        self.left
    }

    /// Returns prepared above edge samples.
    pub const fn above_samples(self) -> &'a [T] {
        self.above
    }

    /// Returns the prepared top-left sample.
    pub const fn top_left_sample(self) -> T {
        self.top_left
    }
}

/// Writes rectangular AV2 §7.13.2.3 data-driven intra prediction into caller storage.
///
/// `output` points at the top-left destination sample and `stride_samples` is
/// the distance between adjacent output rows. Samples outside the predicted
/// rectangle are left unchanged.
///
/// # Errors
/// Returns [`ReconError`] for unsupported sample type/bit depth combinations,
/// wrong DIP mode, wrong edge lengths, out-of-range edge samples, a too-small
/// stride, or a too-small output buffer.
pub fn predict_intra_dip_rect_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    dip_mode: usize,
    dip_transpose: bool,
    edges: IntraDipEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    validate_sample_type::<T>(bit_depth)?;
    validate_dip_mode(dip_mode)?;
    validate_dip_size(size)?;
    validate_dip_edges(bit_depth, size, edges)?;
    validate_output_shape(
        size,
        output.len(),
        stride_samples,
        "DIP intra prediction output buffer length",
    )?;

    let features = dip_features(size, edges, dip_transpose);
    predict_coarse_grid(
        bit_depth,
        size,
        dip_mode,
        dip_transpose,
        features,
        output,
        stride_samples,
    )?;
    interpolate_grid(bit_depth, size, edges, output, stride_samples)?;
    Ok(())
}

fn validate_dip_mode(dip_mode: usize) -> Result<()> {
    if dip_mode < DIP_MODE_COUNT {
        Ok(())
    } else {
        Err(ReconError::UnsupportedIntraDipMode {
            mode: dip_mode,
            max: DIP_MODE_COUNT - 1,
        })
    }
}

fn validate_dip_size(size: IntraRectBlockSize) -> Result<()> {
    if size.width() >= 4 && size.height() >= 4 {
        Ok(())
    } else {
        Err(ReconError::UnsupportedIntraDipBlockSize {
            width: size.width(),
            height: size.height(),
        })
    }
}

fn validate_dip_edges<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    edges: IntraDipEdges<'_, T>,
) -> Result<()> {
    validate_intra_edge_samples(
        IntraDipEdge::Left,
        edges.left,
        size.height() + (size.height() >> 2),
        bit_depth,
    )?;
    validate_intra_edge_samples(
        IntraDipEdge::Above,
        edges.above,
        size.width() + (size.width() >> 2),
        bit_depth,
    )?;
    validate_intra_edge_samples(
        IntraDipEdge::TopLeft,
        core::slice::from_ref(&edges.top_left),
        1,
        bit_depth,
    )
}

fn dip_features<T: ReconSample>(
    size: IntraRectBlockSize,
    edges: IntraDipEdges<'_, T>,
    dip_transpose: bool,
) -> [i32; DIP_FEATURE_COUNT] {
    let mut features = [0i32; DIP_FEATURE_COUNT];
    features[0] = i32::from(edges.top_left.to_u16());

    let above = dip_averages(edges.above, size.width(), size.log2_width());
    let left = dip_averages(edges.left, size.height(), size.log2_height());
    for index in 0..4 {
        features[index + 1] = if dip_transpose {
            left[index]
        } else {
            above[index]
        };
        features[index + 5] = if dip_transpose {
            above[index]
        } else {
            left[index]
        };
    }
    features[9] = if dip_transpose { left[4] } else { above[4] };
    features[10] = if dip_transpose { above[4] } else { left[4] };
    features
}

fn dip_averages<T: ReconSample>(samples: &[T], block_len: usize, log2_len: u8) -> [i32; 5] {
    let group_len = block_len >> 2;
    let group_log2 = log2_len - 2;
    let round = block_len >> 3;
    let mut averages = [0i32; 5];
    for (group, average) in averages.iter_mut().enumerate() {
        let start = group * group_len;
        let end = start + group_len;
        let sum = samples[start..end]
            .iter()
            .fold(0i32, |acc, sample| acc + i32::from(sample.to_u16()));
        *average = (sum + round as i32) >> group_log2;
    }
    averages
}

#[allow(clippy::too_many_arguments)]
fn predict_coarse_grid<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    dip_mode: usize,
    dip_transpose: bool,
    features: [i32; DIP_FEATURE_COUNT],
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let grid = dip_grid(size);
    let mut y = grid.step_y - 1;
    for gy in 0..grid.height {
        let iy = gy * grid.down_y;
        let mut x = grid.step_x - 1;
        for gx in 0..grid.width {
            let ix = gx * grid.down_x;
            let weight_index = if dip_transpose {
                ix * DIP_GRID_SIDE + iy
            } else {
                iy * DIP_GRID_SIDE + ix
            };
            let value = dip_weighted_sample(bit_depth, dip_mode, weight_index, features)?;
            output[y * stride_samples + x] = value;
            x += grid.step_x;
        }
        y += grid.step_y;
    }
    Ok(())
}

fn dip_weighted_sample<T: ReconSample>(
    bit_depth: BitDepth,
    dip_mode: usize,
    weight_index: usize,
    features: [i32; DIP_FEATURE_COUNT],
) -> Result<T> {
    let weights = &splot_tables::tables::loop_restoration::DIP_WEIGHTS[dip_mode][weight_index];
    let mut sum = 0i32;
    for (weight, feature) in weights.iter().zip(features) {
        sum += *weight * feature;
    }
    clip_to_sample(bit_depth, round2_i32(sum, 10))
}

fn clip_to_sample<T: ReconSample>(bit_depth: BitDepth, value: i32) -> Result<T> {
    let clipped = value.clamp(0, i32::from(bit_depth.max_sample()));
    let sample = u16::try_from(clipped).map_err(|_| ReconError::ArithmeticOverflow {
        context: "DIP intra prediction clipped sample",
    })?;
    T::try_from_u16(sample)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DipGrid {
    step_x: usize,
    step_y: usize,
    down_x: usize,
    down_y: usize,
    width: usize,
    height: usize,
    up_log2_x: u8,
    up_log2_y: u8,
}

fn dip_grid(size: IntraRectBlockSize) -> DipGrid {
    let width_log2 = size.log2_width() - 2;
    let height_log2 = size.log2_height() - 2;
    let (up_log2_x, down_log2_x) = split_dip_scale_log2(width_log2);
    let (up_log2_y, down_log2_y) = split_dip_scale_log2(height_log2);
    DipGrid {
        step_x: 1usize << up_log2_x,
        step_y: 1usize << up_log2_y,
        down_x: 1usize << down_log2_x,
        down_y: 1usize << down_log2_y,
        width: DIP_GRID_SIDE >> down_log2_x,
        height: DIP_GRID_SIDE >> down_log2_y,
        up_log2_x,
        up_log2_y,
    }
}

fn split_dip_scale_log2(log2_quarter_len: u8) -> (u8, u8) {
    if log2_quarter_len == 0 {
        (0, 1)
    } else {
        (log2_quarter_len - 1, 0)
    }
}

fn interpolate_grid<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    edges: IntraDipEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let grid = dip_grid(size);
    if grid.step_x > 1 {
        horizontal_interpolate(bit_depth, edges, grid, output, stride_samples)?;
    }
    if grid.step_y > 1 {
        vertical_interpolate(bit_depth, size, edges, grid, output, stride_samples)?;
    }
    Ok(())
}

fn horizontal_interpolate<T: ReconSample>(
    bit_depth: BitDepth,
    edges: IntraDipEdges<'_, T>,
    grid: DipGrid,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let mut y = grid.step_y - 1;
    for _ in 0..grid.height {
        let mut p1 = i32::from(edges.left[y].to_u16());
        let mut x = 0usize;
        for _ in 0..grid.width {
            let p0 = p1;
            p1 = i32::from(output[y * stride_samples + x + grid.step_x - 1].to_u16());
            for z in 0..grid.step_x - 1 {
                let z1 = z + 1;
                let value = (p0 * (grid.step_x - z1) as i32 + p1 * z1 as i32) >> grid.up_log2_x;
                output[y * stride_samples + x + z] = clip_to_sample(bit_depth, value)?;
            }
            x += grid.step_x;
        }
        y += grid.step_y;
    }
    Ok(())
}

fn vertical_interpolate<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    edges: IntraDipEdges<'_, T>,
    grid: DipGrid,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    for x in 0..size.width() {
        let mut p1 = i32::from(edges.above[x].to_u16());
        let mut y = 0usize;
        for _ in 0..grid.height {
            let p0 = p1;
            p1 = i32::from(output[(y + grid.step_y - 1) * stride_samples + x].to_u16());
            for z in 0..grid.step_y - 1 {
                let z1 = z + 1;
                let value = (p0 * (grid.step_y - z1) as i32 + p1 * z1 as i32) >> grid.up_log2_y;
                output[(y + z) * stride_samples + x] = clip_to_sample(bit_depth, value)?;
            }
            y += grid.step_y;
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn rect_size(log2_width: u8, log2_height: u8) -> IntraRectBlockSize {
        IntraRectBlockSize::new(log2_width, log2_height).unwrap()
    }

    #[test]
    fn flat_edges_stay_near_flat() {
        let size = rect_size(3, 4);
        let left = [128u8; 20];
        let above = [128u8; 10];
        for mode in 0..DIP_MODE_COUNT {
            for transpose in [false, true] {
                let mut output = [0u8; 128];
                predict_intra_dip_rect_into(
                    BitDepth::Eight,
                    size,
                    mode,
                    transpose,
                    IntraDipEdges::new(&left, &above, 128),
                    &mut output,
                    8,
                )
                .unwrap();
                assert!(output.iter().all(|sample| (127..=129).contains(sample)));
            }
        }
    }

    #[test]
    fn accepts_quarter_grid_blocks() {
        let size = rect_size(2, 2);
        let left = [80u8, 84, 88, 92, 96];
        let above = [180u8, 176, 172, 168, 164];
        for mode in 0..DIP_MODE_COUNT {
            for transpose in [false, true] {
                let mut output = [0u8; 16];
                predict_intra_dip_rect_into(
                    BitDepth::Eight,
                    size,
                    mode,
                    transpose,
                    IntraDipEdges::new(&left, &above, 128),
                    &mut output,
                    4,
                )
                .unwrap();
                assert!(output.iter().any(|&sample| sample > 0));
            }
        }
    }

    #[test]
    fn rejects_bad_mode() {
        let err = predict_intra_dip_rect_into(
            BitDepth::Eight,
            rect_size(3, 3),
            DIP_MODE_COUNT,
            false,
            IntraDipEdges::new(&[128u8; 10], &[128u8; 10], 128),
            &mut [0u8; 64],
            8,
        )
        .unwrap_err();
        assert_eq!(
            err,
            ReconError::UnsupportedIntraDipMode {
                mode: DIP_MODE_COUNT,
                max: DIP_MODE_COUNT - 1,
            }
        );
    }

    #[test]
    fn validates_extended_edges() {
        let err = predict_intra_dip_rect_into(
            BitDepth::Eight,
            rect_size(3, 3),
            0,
            false,
            IntraDipEdges::new(&[128u8; 9], &[128u8; 10], 128),
            &mut [0u8; 64],
            8,
        )
        .unwrap_err();
        assert_eq!(
            err,
            ReconError::IntraDipEdgeLengthMismatch {
                edge: IntraDipEdge::Left,
                expected: 10,
                actual: 9,
            }
        );
    }
}
