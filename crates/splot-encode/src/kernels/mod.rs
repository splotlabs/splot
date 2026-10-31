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

pub(crate) mod ccso;
mod cctx;
mod cfl;
mod colours;
pub(crate) mod compound;
mod distortion;
mod forward_transform;
pub(crate) mod gdf;
mod hash;
mod ist;
pub(crate) mod motion;
mod quantize;
pub(crate) mod rdoq;
pub(crate) mod tcq;
pub(crate) mod temporal_filter;
mod tpl;
pub(crate) mod wiener;

pub(crate) use forward_transform::ForwardTransform2d;
pub(crate) use quantize::Quantizer;
use splot_recon::SecondaryTransformLayout;

/// An invalid kernel argument.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum KernelError {
    /// The block width or height is not a power of two in `4..=256`.
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
    /// The quantizer step is below one reconstruction unit, or the
    /// denominator is not a power of two in `1..=8`.
    #[error("invalid quantizer q2={q2} dq_denom={dq_denom}")]
    InvalidQuantizer {
        /// The per-coefficient quantizer.
        q2: u32,
        /// The § 7.14.4 `dqDenom`.
        dq_denom: u32,
    },
    /// No move of one level by one gives the parity the block hides.
    #[error("no level move keeps the hidden parity")]
    HiddenParity,
    /// The TCQ trellis lost track of a path's level grid.
    #[error("inconsistent TCQ trellis state")]
    TrellisState,
    /// The deadzone rounding is above one half (`128` in Q8).
    #[error("invalid quantizer rounding {rounding_q8}/256")]
    InvalidRounding {
        /// The requested rounding in Q8.
        rounding_q8: u32,
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
    /// The 8x8 block.
    pub(crate) const SQUARE_8: Self = Self {
        width: 8,
        height: 8,
    };

    /// Validates a `width x height` block size.
    ///
    /// # Errors
    /// Returns [`KernelError::InvalidBlockDims`] if a side is not a power of two
    /// in `4..=256`.
    pub(crate) fn new(width: usize, height: usize) -> Result<Self, KernelError> {
        let valid = |side: usize| side.is_power_of_two() && (4..=256).contains(&side);
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

    /// Sum of squared differences of the first `min(a.len(), b.len())` samples.
    fn sse_row(&self, a: &[u16], b: &[u16]) -> u64;

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

    /// The row-major `src - pred` of the block into `out` (`width * height`
    /// values; a shorter `out` takes the rows that fit).
    fn residual(
        &self,
        src: &[u16],
        src_stride: usize,
        pred: &[u16],
        pred_stride: usize,
        dims: BlockDims,
        out: &mut [i32],
    );

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

    /// Quantizes `coeffs` into `levels` and writes the exact § 7.14.4
    /// reconstruction of each level into `dequant`. Index 0 uses the DC
    /// quantizer and every other index the AC quantizer. Returns the number of
    /// non-zero levels.
    ///
    /// # Errors
    /// Returns [`KernelError::BufferLength`] if the three buffers differ in
    /// length.
    fn quantize(
        &self,
        quantizer: &Quantizer,
        coeffs: &[i32],
        levels: &mut [i32],
        dequant: &mut [i32],
    ) -> Result<usize, KernelError>;

    /// The forward § 7.15.3 secondary transform of the adjusted `coeffs`
    /// in place ([`ist::forward_simd`]).
    ///
    /// # Errors
    /// Returns [`KernelError::BufferLength`] if `coeffs` misses a position of
    /// `layout`.
    fn forward_ist(
        &self,
        layout: &SecondaryTransformLayout,
        coeffs: &mut [i32],
    ) -> Result<(), KernelError>;

    /// `[Σd², Σd·e, Σe²]` of a source `s` and two predictions, with the
    /// per-sample `d·e` and `e²` into `de` and `ee` ([`compound::terms_simd`]).
    fn compound_terms(
        &self,
        s: &[u16],
        preds: (&[u16], &[u16]),
        de: &mut [i32],
        ee: &mut [i32],
    ) -> [i64; 3];

    /// `[Σm·de, Σm·ee, Σm²·ee]` of a blend mask ([`compound::mask_moments_simd`]).
    fn mask_moments(&self, mask: &[u8], de: &[i32], ee: &[i32]) -> [i64; 3];

    /// The difference-weighted mask of two predictions
    /// ([`compound::diff_mask_simd`]).
    fn diff_mask(&self, p0: &[u16], p1: &[u16], shift: u32, out: &mut [u8]);

    /// `[Σ u², Σ v², Σ u·v]` of U and V coefficients ([`cctx::stats_simd`]).
    fn cctx_stats(&self, u: &[i32], v: &[i32]) -> [i64; 3];

    /// The forward cross-chroma rotation `[cos, sin]` of U and V
    /// coefficients in place ([`cctx::rotate_simd`]).
    fn cctx_rotate(&self, u: &mut [i32], v: &mut [i32], rotation: [i32; 2]);

    /// `(min(distinct values, limit + 1), max - min)` of `block`
    /// ([`colours::colours_simd`]).
    fn block_colours(&self, block: &[u16], limit: usize) -> (usize, u16);

    /// The index of the color of `colors` nearest each sample of `samples`
    /// into `out` ([`colours::nearest_simd`]).
    fn nearest_colours(&self, samples: &[u16], colors: &[u16], out: &mut [u8]);

    /// The hash of each eight-sample run of `row` into `out`
    /// ([`hash::row_hashes_simd`]).
    fn row_hashes(&self, row: &[u16], out: &mut [u32]);

    /// The 8x8 block hash of each position of eight row-hash runs into
    /// `out` ([`hash::column_hashes_simd`]).
    fn column_hashes(&self, rows: [&[u32]; hash::HASH_SIDE], out: &mut [u32]);

    /// One chroma row of the luma rows above, at and below it, down-sampled
    /// by the CfL taps `weights` ([`cfl::downsample_simd`]).
    fn cfl_downsample(&self, weights: &[[i32; 3]; 3], rows: [&[u16]; 3], out: &mut [i32]);

    /// One TCQ trellis step: adds the branch metrics of every state to its
    /// path cost and keeps the cheaper branch into each next state
    /// ([`tcq::step_simd`]).
    fn tcq_step(
        &self,
        prev: &[u64; tcq::STATES],
        branches: &tcq::Branches,
        lambda: u64,
    ) -> tcq::Survivors;

    /// The amount each block of a temporal-model frame passes to its
    /// references ([`tpl::propagate_simd`]).
    fn tpl_propagate(
        &self,
        intra: &[u32],
        best: &[u32],
        received: &[u64],
        noise: u32,
        out: &mut [u64],
    );
    /// The 8x8 SADs of a block grid at four horizontal positions
    /// ([`motion::sad8x8_x4_simd`]).
    fn sad8x8_x4(
        &self,
        src: (&[u8], usize),
        refs: (&[u8], usize),
        cols: usize,
        out: &mut [[u32; motion::WAYS]],
    );

    /// The 2x2 sums of a grid of four-position SADs ([`motion::sad_quads_simd`]).
    fn sad_quads(
        &self,
        input: &[[u32; motion::WAYS]],
        cols: usize,
        out: &mut [[u32; motion::WAYS]],
    );

    /// Keeps each block's best of four positions ([`motion::keep_best_simd`]).
    fn keep_best(&self, sads: &[[u32; motion::WAYS]], mv: [i16; 2], best: &mut [motion::Motion]);

    /// One row of a plane halved by 2x2 means ([`motion::downscale_row_simd`]).
    fn downscale_row(&self, top: &[u8], bottom: &[u8], out: &mut [u8]);

    /// The weight of each 16x16 quadrant of a temporal-filter neighbour
    /// ([`temporal_filter::weights_simd`]).
    fn tf_weights(
        &self,
        errors: &[u32; temporal_filter::QUADRANTS],
        distances: &[u32; temporal_filter::QUADRANTS],
        decay: u32,
        out: &mut [u32; temporal_filter::QUADRANTS],
    );

    /// Adds `weight` times each sample of `pred` to `accum`
    /// ([`temporal_filter::accumulate_simd`]).
    fn tf_accumulate(&self, pred: &[u16], weight: u32, accum: &mut [u32]);

    /// The rounded quotients of `accum` by `count`
    /// ([`temporal_filter::normalize_simd`]).
    fn tf_normalize(&self, accum: &[u32], count: u32, out: &mut [u16]);
}

/// The scalar reference kernels.
#[cfg(test)]
pub(crate) struct ScalarKernels;

/// The `std::simd` kernels, bit-identical to [`ScalarKernels`].
pub(crate) struct SimdKernels;

/// The kernels the encoder uses.
pub(crate) fn kernels() -> &'static dyn EncoderKernels {
    &SimdKernels
}

#[cfg(test)]
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

    fn sse_row(&self, a: &[u16], b: &[u16]) -> u64 {
        distortion::sse_row_scalar(a, b)
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

    fn residual(
        &self,
        src: &[u16],
        src_stride: usize,
        pred: &[u16],
        pred_stride: usize,
        dims: BlockDims,
        out: &mut [i32],
    ) {
        distortion::residual_scalar(src, src_stride, pred, pred_stride, dims, out);
    }

    fn forward_transform_2d(
        &self,
        tx: &ForwardTransform2d,
        residual: &[i32],
        coeffs: &mut [i32],
    ) -> Result<(), KernelError> {
        forward_transform::forward_scalar(tx, residual, coeffs)
    }

    fn quantize(
        &self,
        quantizer: &Quantizer,
        coeffs: &[i32],
        levels: &mut [i32],
        dequant: &mut [i32],
    ) -> Result<usize, KernelError> {
        quantize::quantize_scalar(quantizer, coeffs, levels, dequant)
    }

    fn forward_ist(
        &self,
        layout: &SecondaryTransformLayout,
        coeffs: &mut [i32],
    ) -> Result<(), KernelError> {
        ist::forward_scalar(layout, coeffs)
    }

    fn compound_terms(
        &self,
        s: &[u16],
        preds: (&[u16], &[u16]),
        de: &mut [i32],
        ee: &mut [i32],
    ) -> [i64; 3] {
        compound::terms_scalar(s, preds, de, ee)
    }

    fn mask_moments(&self, mask: &[u8], de: &[i32], ee: &[i32]) -> [i64; 3] {
        compound::mask_moments_scalar(mask, de, ee)
    }

    fn diff_mask(&self, p0: &[u16], p1: &[u16], shift: u32, out: &mut [u8]) {
        compound::diff_mask_scalar(p0, p1, shift, out);
    }

    fn cctx_stats(&self, u: &[i32], v: &[i32]) -> [i64; 3] {
        cctx::stats_scalar(u, v)
    }

    fn cctx_rotate(&self, u: &mut [i32], v: &mut [i32], rotation: [i32; 2]) {
        cctx::rotate_scalar(u, v, rotation);
    }

    fn block_colours(&self, block: &[u16], limit: usize) -> (usize, u16) {
        colours::colours_scalar(block, limit)
    }

    fn nearest_colours(&self, samples: &[u16], colors: &[u16], out: &mut [u8]) {
        colours::nearest_scalar(samples, colors, out);
    }

    fn row_hashes(&self, row: &[u16], out: &mut [u32]) {
        hash::row_hashes_scalar(row, out);
    }

    fn column_hashes(&self, rows: [&[u32]; hash::HASH_SIDE], out: &mut [u32]) {
        hash::column_hashes_scalar(rows, out);
    }

    fn cfl_downsample(&self, weights: &[[i32; 3]; 3], rows: [&[u16]; 3], out: &mut [i32]) {
        cfl::downsample_scalar(weights, rows, out);
    }

    fn tcq_step(
        &self,
        prev: &[u64; tcq::STATES],
        branches: &tcq::Branches,
        lambda: u64,
    ) -> tcq::Survivors {
        tcq::step_scalar(prev, branches, lambda)
    }

    fn tpl_propagate(
        &self,
        intra: &[u32],
        best: &[u32],
        received: &[u64],
        noise: u32,
        out: &mut [u64],
    ) {
        tpl::propagate_scalar(intra, best, received, noise, out);
    }

    fn sad8x8_x4(
        &self,
        src: (&[u8], usize),
        refs: (&[u8], usize),
        cols: usize,
        out: &mut [[u32; motion::WAYS]],
    ) {
        motion::sad8x8_x4_scalar(src, refs, cols, out);
    }

    fn sad_quads(
        &self,
        input: &[[u32; motion::WAYS]],
        cols: usize,
        out: &mut [[u32; motion::WAYS]],
    ) {
        motion::sad_quads_scalar(input, cols, out);
    }

    fn keep_best(&self, sads: &[[u32; motion::WAYS]], mv: [i16; 2], best: &mut [motion::Motion]) {
        motion::keep_best_scalar(sads, mv, best);
    }

    fn downscale_row(&self, top: &[u8], bottom: &[u8], out: &mut [u8]) {
        motion::downscale_row_scalar(top, bottom, out);
    }

    fn tf_weights(
        &self,
        errors: &[u32; temporal_filter::QUADRANTS],
        distances: &[u32; temporal_filter::QUADRANTS],
        decay: u32,
        out: &mut [u32; temporal_filter::QUADRANTS],
    ) {
        temporal_filter::weights_scalar(errors, distances, decay, out);
    }

    fn tf_accumulate(&self, pred: &[u16], weight: u32, accum: &mut [u32]) {
        temporal_filter::accumulate_scalar(pred, weight, accum);
    }

    fn tf_normalize(&self, accum: &[u32], count: u32, out: &mut [u16]) {
        temporal_filter::normalize_scalar(accum, count, out);
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

    fn sse_row(&self, a: &[u16], b: &[u16]) -> u64 {
        distortion::sse_row_simd(a, b)
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

    fn residual(
        &self,
        src: &[u16],
        src_stride: usize,
        pred: &[u16],
        pred_stride: usize,
        dims: BlockDims,
        out: &mut [i32],
    ) {
        distortion::residual_simd(src, src_stride, pred, pred_stride, dims, out);
    }

    fn forward_transform_2d(
        &self,
        tx: &ForwardTransform2d,
        residual: &[i32],
        coeffs: &mut [i32],
    ) -> Result<(), KernelError> {
        forward_transform::forward_simd(tx, residual, coeffs)
    }

    fn quantize(
        &self,
        quantizer: &Quantizer,
        coeffs: &[i32],
        levels: &mut [i32],
        dequant: &mut [i32],
    ) -> Result<usize, KernelError> {
        quantize::quantize_simd(quantizer, coeffs, levels, dequant)
    }

    fn forward_ist(
        &self,
        layout: &SecondaryTransformLayout,
        coeffs: &mut [i32],
    ) -> Result<(), KernelError> {
        ist::forward_simd(layout, coeffs)
    }

    fn compound_terms(
        &self,
        s: &[u16],
        preds: (&[u16], &[u16]),
        de: &mut [i32],
        ee: &mut [i32],
    ) -> [i64; 3] {
        compound::terms_simd(s, preds, de, ee)
    }

    fn mask_moments(&self, mask: &[u8], de: &[i32], ee: &[i32]) -> [i64; 3] {
        compound::mask_moments_simd(mask, de, ee)
    }

    fn diff_mask(&self, p0: &[u16], p1: &[u16], shift: u32, out: &mut [u8]) {
        compound::diff_mask_simd(p0, p1, shift, out);
    }

    fn cctx_stats(&self, u: &[i32], v: &[i32]) -> [i64; 3] {
        cctx::stats_simd(u, v)
    }

    fn cctx_rotate(&self, u: &mut [i32], v: &mut [i32], rotation: [i32; 2]) {
        cctx::rotate_simd(u, v, rotation);
    }

    fn block_colours(&self, block: &[u16], limit: usize) -> (usize, u16) {
        colours::colours_simd(block, limit)
    }

    fn nearest_colours(&self, samples: &[u16], colors: &[u16], out: &mut [u8]) {
        colours::nearest_simd(samples, colors, out);
    }

    fn row_hashes(&self, row: &[u16], out: &mut [u32]) {
        hash::row_hashes_simd(row, out);
    }

    fn column_hashes(&self, rows: [&[u32]; hash::HASH_SIDE], out: &mut [u32]) {
        hash::column_hashes_simd(rows, out);
    }

    fn cfl_downsample(&self, weights: &[[i32; 3]; 3], rows: [&[u16]; 3], out: &mut [i32]) {
        cfl::downsample_simd(weights, rows, out);
    }

    fn tcq_step(
        &self,
        prev: &[u64; tcq::STATES],
        branches: &tcq::Branches,
        lambda: u64,
    ) -> tcq::Survivors {
        tcq::step_simd(prev, branches, lambda)
    }

    fn tpl_propagate(
        &self,
        intra: &[u32],
        best: &[u32],
        received: &[u64],
        noise: u32,
        out: &mut [u64],
    ) {
        tpl::propagate_simd(intra, best, received, noise, out);
    }

    fn sad8x8_x4(
        &self,
        src: (&[u8], usize),
        refs: (&[u8], usize),
        cols: usize,
        out: &mut [[u32; motion::WAYS]],
    ) {
        motion::sad8x8_x4_simd(src, refs, cols, out);
    }

    fn sad_quads(
        &self,
        input: &[[u32; motion::WAYS]],
        cols: usize,
        out: &mut [[u32; motion::WAYS]],
    ) {
        motion::sad_quads_simd(input, cols, out);
    }

    fn keep_best(&self, sads: &[[u32; motion::WAYS]], mv: [i16; 2], best: &mut [motion::Motion]) {
        motion::keep_best_simd(sads, mv, best);
    }

    fn downscale_row(&self, top: &[u8], bottom: &[u8], out: &mut [u8]) {
        motion::downscale_row_simd(top, bottom, out);
    }

    fn tf_weights(
        &self,
        errors: &[u32; temporal_filter::QUADRANTS],
        distances: &[u32; temporal_filter::QUADRANTS],
        decay: u32,
        out: &mut [u32; temporal_filter::QUADRANTS],
    ) {
        temporal_filter::weights_simd(errors, distances, decay, out);
    }

    fn tf_accumulate(&self, pred: &[u16], weight: u32, accum: &mut [u32]) {
        temporal_filter::accumulate_simd(pred, weight, accum);
    }

    fn tf_normalize(&self, accum: &[u32], count: u32, out: &mut [u16]) {
        temporal_filter::normalize_simd(accum, count, out);
    }
}
