// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Encoder hot kernels behind one dispatch trait.
//!
//! Every kernel has a scalar reference ([`ScalarKernels`]) and a `std::simd`
//! implementation ([`SimdKernels`]). The two give bit-identical results; the
//! property tests in each submodule assert this over random inputs, block
//! sizes, and bit depths. Encoder code calls [`kernels`] and never names an
//! implementation directly.
//!
//! The forward transform and the quantizer are the encoder-side inverses of the
//! `splot-recon` § 7.15.4 inverse transform and § 7.14.4 dequantizer: their
//! outputs use the scale that those decoder processes expect.

#![cfg_attr(not(test), allow(dead_code))]

mod distortion;
mod forward_transform;

pub(crate) use forward_transform::ForwardTransform2d;

/// An invalid kernel argument.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum KernelError {
    /// The block width or height is not a power of two in `4..=128`.
    #[error("invalid kernel block size {width}x{height}")]
    InvalidBlockDims {
        /// Requested width in samples.
        width: usize,
        /// Requested height in samples.
        height: usize,
    },
    /// The transform shape is not one of the AV2 `TX_SIZES_ALL` shapes.
    #[error("invalid transform shape log2 {log2_width}x{log2_height}")]
    InvalidTransformShape {
        /// Requested width base-2 logarithm.
        log2_width: u32,
        /// Requested height base-2 logarithm.
        log2_height: u32,
    },
    /// A caller buffer has the wrong length.
    #[error("{buffer} holds {actual} values, expected {expected}")]
    BufferLength {
        /// The buffer name.
        buffer: &'static str,
        /// The required length.
        expected: usize,
        /// The supplied length.
        actual: usize,
    },
}

/// A block size accepted by the distortion kernels: width and height are each
/// a power of two in `4..=128`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BlockDims {
    width: usize,
    height: usize,
}

impl BlockDims {
    /// Validates a `width x height` block size.
    ///
    /// # Errors
    /// Returns [`KernelError::InvalidBlockDims`] if a side is not a power of two
    /// in `4..=128`.
    pub(crate) fn new(width: usize, height: usize) -> Result<Self, KernelError> {
        let valid = |side: usize| side.is_power_of_two() && (4..=128).contains(&side);
        if valid(width) && valid(height) {
            Ok(Self { width, height })
        } else {
            Err(KernelError::InvalidBlockDims { width, height })
        }
    }

    /// The width in samples.
    pub(crate) fn width(self) -> usize {
        self.width
    }

    /// The height in samples.
    pub(crate) fn height(self) -> usize {
        self.height
    }
}

/// The encoder hot kernels.
///
/// Sample inputs are strided `u16` blocks (the encoder stores every bit depth
/// as `u16`): row `y` starts at `y * stride`, and each slice must hold at least
/// `(height - 1) * stride + width` samples with `stride >= width`. Rows that a
/// short slice does not hold are not counted.
pub(crate) trait EncoderKernels: Sync {
    /// Sum of absolute differences.
    fn sad(
        &self,
        src: &[u16],
        src_stride: usize,
        pred: &[u16],
        pred_stride: usize,
        dims: BlockDims,
    ) -> u32;

    /// Sum of squared differences.
    fn sse(
        &self,
        src: &[u16],
        src_stride: usize,
        pred: &[u16],
        pred_stride: usize,
        dims: BlockDims,
    ) -> u64;

    /// Sum of absolute unnormalized Hadamard-transformed differences, so a flat
    /// difference gives the SAD. Blocks with both sides at least 8 use 8x8
    /// tiles; other blocks use 4x4 tiles.
    fn satd(
        &self,
        src: &[u16],
        src_stride: usize,
        pred: &[u16],
        pred_stride: usize,
        dims: BlockDims,
    ) -> u64;

    /// Forward 2D transform of a row-major `residual` block of the original
    /// transform size into `coeffs` (the adjusted size of at most 32x32).
    ///
    /// # Errors
    /// Returns [`KernelError::BufferLength`] if a buffer length does not match
    /// `tx`.
    fn forward_transform_2d(
        &self,
        tx: &ForwardTransform2d,
        residual: &[i32],
        coeffs: &mut [i32],
    ) -> Result<(), KernelError>;
}

/// The scalar reference kernels.
pub(crate) struct ScalarKernels;

/// The `std::simd` kernels, bit-identical to [`ScalarKernels`].
pub(crate) struct SimdKernels;

/// The kernels the encoder uses.
pub(crate) fn kernels() -> &'static dyn EncoderKernels {
    &SimdKernels
}

impl EncoderKernels for ScalarKernels {
    fn sad(
        &self,
        src: &[u16],
        src_stride: usize,
        pred: &[u16],
        pred_stride: usize,
        dims: BlockDims,
    ) -> u32 {
        distortion::sad_scalar(src, src_stride, pred, pred_stride, dims)
    }

    fn sse(
        &self,
        src: &[u16],
        src_stride: usize,
        pred: &[u16],
        pred_stride: usize,
        dims: BlockDims,
    ) -> u64 {
        distortion::sse_scalar(src, src_stride, pred, pred_stride, dims)
    }

    fn satd(
        &self,
        src: &[u16],
        src_stride: usize,
        pred: &[u16],
        pred_stride: usize,
        dims: BlockDims,
    ) -> u64 {
        distortion::satd_scalar(src, src_stride, pred, pred_stride, dims)
    }

    fn forward_transform_2d(
        &self,
        tx: &ForwardTransform2d,
        residual: &[i32],
        coeffs: &mut [i32],
    ) -> Result<(), KernelError> {
        forward_transform::forward_scalar(tx, residual, coeffs)
    }
}

impl EncoderKernels for SimdKernels {
    fn sad(
        &self,
        src: &[u16],
        src_stride: usize,
        pred: &[u16],
        pred_stride: usize,
        dims: BlockDims,
    ) -> u32 {
        distortion::sad_simd(src, src_stride, pred, pred_stride, dims)
    }

    fn sse(
        &self,
        src: &[u16],
        src_stride: usize,
        pred: &[u16],
        pred_stride: usize,
        dims: BlockDims,
    ) -> u64 {
        distortion::sse_simd(src, src_stride, pred, pred_stride, dims)
    }

    fn satd(
        &self,
        src: &[u16],
        src_stride: usize,
        pred: &[u16],
        pred_stride: usize,
        dims: BlockDims,
    ) -> u64 {
        distortion::satd_simd(src, src_stride, pred, pred_stride, dims)
    }

    fn forward_transform_2d(
        &self,
        tx: &ForwardTransform2d,
        residual: &[i32],
        coeffs: &mut [i32],
    ) -> Result<(), KernelError> {
        forward_transform::forward_simd(tx, residual, coeffs)
    }
}
