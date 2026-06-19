// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Error and result types for the splot encoder API.

use splot_core::write::WriteError;
use splot_recon::{PlaneId, PlaneSize, ReconError};

use crate::config::{BitDepth, ChromaSubsampling};
use crate::context::{EncoderOperation, EncoderState};

/// An error from the splot encoder API.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The encoder worker pool could not be constructed.
    #[error("failed to construct the encoder worker pool: {source}")]
    Pool {
        /// The underlying parallel-runtime error.
        #[from]
        source: splot_parallel::ParallelError,
    },

    /// Input frame bit depth is outside the current supported subset.
    #[error("unsupported encoder input bit depth {bit_depth:?}; only 8-bit input is supported")]
    UnsupportedInputBitDepth {
        /// Requested input bit depth.
        bit_depth: BitDepth,
    },

    /// Input frame chroma layout is outside the current supported subset.
    #[error(
        "unsupported encoder input chroma layout {chroma_subsampling:?}; only YUV420 input is supported"
    )]
    UnsupportedInputChromaSubsampling {
        /// Requested input chroma layout.
        chroma_subsampling: ChromaSubsampling,
    },

    /// A required input plane is absent.
    #[error("missing required encoder input plane {plane:?}")]
    MissingInputPlane {
        /// Missing plane.
        plane: PlaneId,
    },

    /// A plane was supplied where the current format has no plane.
    #[error("unexpected encoder input plane {plane:?}")]
    UnexpectedInputPlane {
        /// Unexpected plane.
        plane: PlaneId,
    },

    /// A borrowed input plane failed geometry validation.
    #[error("invalid encoder input plane {plane:?}: {source}")]
    InputPlane {
        /// Failing plane.
        plane: PlaneId,
        /// Underlying reconstruction view validation error.
        #[source]
        source: ReconError,
    },

    /// A borrowed input plane's visible size does not match frame metadata.
    #[error("encoder input plane {plane:?} has visible size {actual:?}; expected {expected:?}")]
    InputPlaneSizeMismatch {
        /// Failing plane.
        plane: PlaneId,
        /// Expected visible size.
        expected: PlaneSize,
        /// Actual visible size.
        actual: PlaneSize,
    },

    /// Input frame dimensions do not match the encoder configuration.
    #[error(
        "encoder input frame has visible luma size {actual:?}; expected {expected_width}x{expected_height}"
    )]
    InputFrameSizeMismatch {
        /// Configured frame width in luma samples.
        expected_width: u32,
        /// Configured frame height in luma samples.
        expected_height: u32,
        /// Actual input frame visible luma size.
        actual: PlaneSize,
    },

    /// Input frame bit depth does not match the encoder configuration.
    #[error("encoder input frame bit depth {actual:?} does not match configured {expected:?}")]
    InputFrameBitDepthMismatch {
        /// Configured bit depth.
        expected: BitDepth,
        /// Actual input frame bit depth.
        actual: BitDepth,
    },

    /// Input frame chroma layout does not match the encoder configuration.
    #[error("encoder input frame chroma layout {actual:?} does not match configured {expected:?}")]
    InputFrameChromaSubsamplingMismatch {
        /// Configured chroma layout.
        expected: ChromaSubsampling,
        /// Actual input frame chroma layout.
        actual: ChromaSubsampling,
    },

    /// Chroma-size derivation failed for the input metadata.
    #[error("failed to derive encoder input chroma geometry: {source}")]
    InputChromaGeometry {
        /// Underlying chroma geometry error.
        #[source]
        source: ReconError,
    },

    /// A requested encoder residual block fell outside the input plane's visible area.
    #[error(
        "encoder residual block {block:?} for plane {plane:?} exceeds visible size {visible_size:?}"
    )]
    ResidualBlockOutOfBounds {
        /// Plane whose residual was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
        /// Visible plane size used for the bounds check.
        visible_size: PlaneSize,
    },

    /// A prediction row stride was too small for a residual block.
    #[error(
        "encoder residual prediction stride {stride_samples} for plane {plane:?} is smaller than block width {width}"
    )]
    ResidualPredictionStrideTooSmall {
        /// Plane whose residual was requested.
        plane: PlaneId,
        /// Supplied prediction row stride in samples.
        stride_samples: usize,
        /// Required block width in samples.
        width: usize,
    },

    /// A prediction buffer was too small for a residual block.
    #[error(
        "encoder residual prediction for plane {plane:?} needs at least {expected} samples, got {actual}"
    )]
    ResidualPredictionLengthMismatch {
        /// Plane whose residual was requested.
        plane: PlaneId,
        /// Minimum required prediction sample count.
        expected: usize,
        /// Actual supplied prediction sample count.
        actual: usize,
    },

    /// Residual prediction span arithmetic overflowed.
    #[error(
        "encoder residual prediction span overflowed for plane {plane:?}, block {block:?}, stride {stride_samples}"
    )]
    ResidualPredictionSpanOverflow {
        /// Plane whose residual was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
        /// Supplied prediction row stride in samples.
        stride_samples: usize,
    },

    /// Residual block sample-count arithmetic overflowed.
    #[error("encoder residual block sample count overflowed for plane {plane:?}, block {block:?}")]
    ResidualSampleCountOverflow {
        /// Plane whose residual was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
    },

    /// Residual block allocation failed.
    #[error("failed to allocate encoder residual storage for plane {plane:?}: {context}")]
    ResidualAllocationFailed {
        /// Plane whose residual was requested.
        plane: PlaneId,
        /// Short description of the failed allocation.
        context: &'static str,
    },

    /// A forward-transform block shape is outside the current supported subset.
    #[error(
        "encoder forward transform for plane {plane:?} supports only {expected_width}x{expected_height}, got block {block:?}"
    )]
    ForwardTransformUnsupportedShape {
        /// Plane whose transform was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
        /// Supported transform width in samples.
        expected_width: usize,
        /// Supported transform height in samples.
        expected_height: usize,
    },

    /// Forward-transform residual input had the wrong number of samples.
    #[error(
        "encoder forward transform for plane {plane:?}, block {block:?} needs {expected} residual samples, got {actual}"
    )]
    ForwardTransformInputLengthMismatch {
        /// Plane whose transform was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
        /// Expected residual sample count.
        expected: usize,
        /// Actual residual sample count.
        actual: usize,
    },

    /// The current forward-transform subset only supports uniform residual blocks.
    #[error(
        "encoder forward transform for plane {plane:?}, block {block:?} supports only uniform residuals; sample 0 is {first}, sample {mismatch_index} is {value}"
    )]
    ForwardTransformNonUniformResidual {
        /// Plane whose transform was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
        /// First residual sample.
        first: i32,
        /// Index of the first non-matching sample.
        mismatch_index: usize,
        /// Non-matching residual sample.
        value: i32,
    },

    /// Forward-transform coefficient arithmetic overflowed.
    #[error(
        "encoder forward transform coefficient overflowed for plane {plane:?}, block {block:?}, residual sample {sample}, scale {scale}"
    )]
    ForwardTransformCoefficientOverflow {
        /// Plane whose transform was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
        /// Residual sample being scaled.
        sample: i32,
        /// Scale applied to derive the coefficient.
        scale: i32,
    },

    /// A quantization block shape is outside the current supported subset.
    #[error(
        "encoder quantization for plane {plane:?} supports only {expected_width}x{expected_height}, got block {block:?}"
    )]
    QuantizationUnsupportedShape {
        /// Plane whose quantization was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
        /// Supported quantization width in samples.
        expected_width: usize,
        /// Supported quantization height in samples.
        expected_height: usize,
    },

    /// A fixed quantizer index is outside the active bit-depth range.
    #[error("encoder quantizer index {qindex} is outside {bit_depth:?} range 0..={max}")]
    QuantizationQIndexOutOfRange {
        /// Active decoded bit depth.
        bit_depth: splot_recon::BitDepth,
        /// Requested quantizer index.
        qindex: u32,
        /// Maximum legal quantizer index for the active bit depth.
        max: u32,
    },

    /// A dequant denominator is invalid.
    #[error("encoder quantization dequant denominator must be non-zero, got {dq_denom}")]
    QuantizationInvalidDequantDenominator {
        /// Requested dequant denominator.
        dq_denom: u32,
    },

    /// A transform coefficient is outside the supported dequant-visible range.
    #[error(
        "encoder quantization coefficient {coefficient_index} for plane {plane:?}, block {block:?} is {value}, outside {bit_depth:?} dequant-visible range {min}..={max}"
    )]
    QuantizationCoefficientOutOfRange {
        /// Plane whose quantization was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
        /// Row-major coefficient index.
        coefficient_index: usize,
        /// Transform coefficient value.
        value: i32,
        /// Minimum supported coefficient value.
        min: i32,
        /// Maximum supported coefficient value.
        max: i32,
        /// Active decoded bit depth.
        bit_depth: splot_recon::BitDepth,
    },

    /// Quantization coefficient arithmetic overflowed.
    #[error(
        "encoder quantization arithmetic overflowed while computing {context} for plane {plane:?}, block {block:?}, coefficient {coefficient_index}, value {value}, quantizer {quantizer}, denominator {dq_denom}"
    )]
    QuantizationCoefficientOverflow {
        /// Plane whose quantization was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
        /// Row-major coefficient index.
        coefficient_index: usize,
        /// Transform coefficient value.
        value: i32,
        /// Resolved quantizer value.
        quantizer: u32,
        /// Dequant denominator.
        dq_denom: u32,
        /// Short description of the failed calculation.
        context: &'static str,
    },

    /// A quantized coefficient would exceed the AV2 dequant product domain.
    #[error(
        "encoder quantization dequant product for plane {plane:?}, block {block:?}, coefficient {coefficient_index} would exceed {max_product}: abs(quantized) {quantized_abs} * quantizer {quantizer}"
    )]
    QuantizationDequantProductOverflow {
        /// Plane whose quantization was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
        /// Row-major coefficient index.
        coefficient_index: usize,
        /// Absolute quantized coefficient magnitude.
        quantized_abs: u64,
        /// Resolved quantizer value.
        quantizer: u32,
        /// Maximum supported product before AV2's 24-bit dequant mask would wrap.
        max_product: u64,
    },

    /// Decoder-visible dequantization rejected quantized encoder output.
    #[error(
        "encoder quantization dequant handoff failed for plane {plane:?}, block {block:?}: {source}"
    )]
    QuantizationDequant {
        /// Plane whose quantization was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
        /// Underlying reconstruction dequantization error.
        #[source]
        source: ReconError,
    },

    /// Coefficient tokenization received a plane outside the current minimal subset.
    #[error("encoder coefficient tokenization supports only luma, got plane {plane:?}")]
    CoefficientTokenizationUnsupportedPlane {
        /// Plane whose tokenization was requested.
        plane: PlaneId,
    },

    /// A coefficient-tokenization block shape is outside the current supported subset.
    #[error(
        "encoder coefficient tokenization for plane {plane:?} supports only {expected_width}x{expected_height}, got block {block:?}"
    )]
    CoefficientTokenizationUnsupportedShape {
        /// Plane whose tokenization was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
        /// Supported tokenization width in samples.
        expected_width: usize,
        /// Supported tokenization height in samples.
        expected_height: usize,
    },

    /// A coefficient-tokenization input had the wrong number of coefficients.
    #[error(
        "encoder coefficient tokenization for plane {plane:?}, block {block:?} needs {expected} coefficients, got {actual}"
    )]
    CoefficientTokenizationInputLengthMismatch {
        /// Plane whose tokenization was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
        /// Expected coefficient count.
        expected: usize,
        /// Actual coefficient count.
        actual: usize,
    },

    /// A coefficient-tokenization block would require non-neutral spatial contexts.
    #[error(
        "encoder coefficient tokenization for plane {plane:?}, block {block:?} currently supports only the top-left neutral spatial context"
    )]
    CoefficientTokenizationUnsupportedSpatialContext {
        /// Plane whose tokenization was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
    },

    /// Coefficient scan-order derivation failed.
    #[error(
        "encoder coefficient tokenization scan derivation failed for plane {plane:?}, block {block:?}: {source}"
    )]
    CoefficientTokenizationScan {
        /// Plane whose tokenization was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
        /// Underlying reconstruction scan error.
        #[source]
        source: ReconError,
    },

    /// The current coefficient-tokenization subset only supports DC-only blocks.
    #[error(
        "encoder coefficient tokenization for plane {plane:?}, block {block:?} supports only DC-only coefficients; coefficient {coefficient_index} is {value}"
    )]
    CoefficientTokenizationNonDcCoefficient {
        /// Plane whose tokenization was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
        /// Row-major coefficient index.
        coefficient_index: usize,
        /// Unsupported non-DC coefficient value.
        value: i32,
    },

    /// A coefficient magnitude requires syntax outside the current tokenization tier.
    #[error(
        "encoder coefficient tokenization for plane {plane:?}, block {block:?}, coefficient {coefficient_index} magnitude {magnitude} exceeds current base-symbol maximum {max_magnitude}"
    )]
    CoefficientTokenizationUnsupportedMagnitude {
        /// Plane whose tokenization was requested.
        plane: PlaneId,
        /// Visible-plane-relative block rectangle.
        block: splot_recon::PlaneRect,
        /// Row-major coefficient index.
        coefficient_index: usize,
        /// Unsupported absolute coefficient magnitude.
        magnitude: u32,
        /// Maximum magnitude covered by the current minimal base-symbol tier.
        max_magnitude: u32,
    },

    /// A chroma coefficient magnitude is outside the current coded-chroma tier.
    #[error(
        "encoder coded chroma coefficient tokenization for plane {plane:?} magnitude {magnitude} is outside the current base tier (1..={max_magnitude})"
    )]
    CoefficientTokenizationUnsupportedChromaMagnitude {
        /// Chroma plane whose tokenization was requested.
        plane: PlaneId,
        /// Unsupported absolute coefficient magnitude.
        magnitude: u32,
        /// Maximum magnitude covered by the current coded-chroma base tier.
        max_magnitude: u32,
    },

    /// Coefficient-tokenization allocation failed.
    #[error("failed to allocate encoder coefficient tokenization storage for {context}")]
    CoefficientTokenizationAllocationFailed {
        /// Short description of the failed allocation.
        context: &'static str,
    },

    /// A coefficient token carried a CDF selector outside the current minimal subset.
    #[error("unsupported encoder coefficient token CDF selector for {syntax}")]
    CoefficientTokenizationUnsupportedCdfSelector {
        /// Token syntax whose selector is unsupported.
        syntax: &'static str,
    },

    /// Writing a coefficient token through the symbol encoder failed.
    #[error("encoder coefficient tokenization symbol write failed for {syntax}: {source}")]
    CoefficientTokenizationSymbolWrite {
        /// Token syntax being written.
        syntax: &'static str,
        /// Source symbol-encoder error.
        #[source]
        source: WriteError,
    },

    /// Finalizing coefficient token symbol bytes failed.
    #[error("encoder coefficient tokenization symbol encoder finalization failed: {source}")]
    CoefficientTokenizationSymbolEncodeFinish {
        /// Source symbol-encoder error.
        #[source]
        source: WriteError,
    },

    /// Initializing the coefficient token symbol decoder failed.
    #[error("encoder coefficient tokenization symbol decoder initialization failed: {source}")]
    CoefficientTokenizationSymbolDecodeInit {
        /// Source symbol-decoder error.
        #[source]
        source: splot_core::Error,
    },

    /// Reading a coefficient token through the symbol decoder failed.
    #[error("encoder coefficient tokenization symbol read failed for {syntax}: {source}")]
    CoefficientTokenizationSymbolRead {
        /// Token syntax being read.
        syntax: &'static str,
        /// Source symbol-decoder error.
        #[source]
        source: splot_core::Error,
    },

    /// Decoded coefficient token symbol value did not match the encoded value.
    #[error(
        "encoder coefficient tokenization symbol mismatch for {syntax}: expected {expected}, decoded {actual}"
    )]
    CoefficientTokenizationSymbolMismatch {
        /// Token syntax being compared.
        syntax: &'static str,
        /// Encoded symbol value.
        expected: u8,
        /// Decoded symbol value.
        actual: u8,
    },

    /// Finalizing the coefficient token symbol decoder failed.
    #[error("encoder coefficient tokenization symbol decoder finalization failed: {source}")]
    CoefficientTokenizationSymbolDecodeFinish {
        /// Source symbol-decoder error.
        #[source]
        source: splot_core::Error,
    },

    /// Closed-loop reconstruction received an unsupported bit depth.
    #[error(
        "encoder closed-loop reconstruction supports only 8-bit input, got bit depth {bit_depth:?}"
    )]
    ClosedLoopUnsupportedBitDepth {
        /// Requested decoded bit depth.
        bit_depth: splot_recon::BitDepth,
    },

    /// Closed-loop reconstruction received a source view that is not 4x4.
    #[error(
        "encoder closed-loop reconstruction for plane {plane:?} supports only {expected_width}x{expected_height} source views, got visible size {actual:?}"
    )]
    ClosedLoopUnsupportedSourceSize {
        /// Plane whose reconstruction was requested.
        plane: PlaneId,
        /// Actual visible source size.
        actual: PlaneSize,
        /// Supported source width in samples.
        expected_width: usize,
        /// Supported source height in samples.
        expected_height: usize,
    },

    /// Decoder-visible intra prediction rejected closed-loop reconstruction.
    #[error("encoder closed-loop intra prediction failed for plane {plane:?}: {source}")]
    ClosedLoopPredict {
        /// Plane whose reconstruction was requested.
        plane: PlaneId,
        /// Underlying reconstruction prediction error.
        #[source]
        source: ReconError,
    },

    /// Decoder-visible dequantization rejected closed-loop reconstruction.
    #[error(
        "encoder closed-loop dequantization failed for plane {plane:?}, block {block:?}: {source}"
    )]
    ClosedLoopDequant {
        /// Plane whose reconstruction was requested.
        plane: PlaneId,
        /// Visible-plane-relative transform block rectangle.
        block: splot_recon::PlaneRect,
        /// Underlying reconstruction dequantization error.
        #[source]
        source: ReconError,
    },

    /// Resolving the decoder-visible transform shift rejected closed-loop reconstruction.
    #[error(
        "encoder closed-loop transform-shift derivation failed for plane {plane:?}, block {block:?}: {source}"
    )]
    ClosedLoopTransformShift {
        /// Plane whose reconstruction was requested.
        plane: PlaneId,
        /// Visible-plane-relative transform block rectangle.
        block: splot_recon::PlaneRect,
        /// Underlying reconstruction transform-shift error.
        #[source]
        source: ReconError,
    },

    /// Decoder-visible inverse transform rejected closed-loop reconstruction.
    #[error(
        "encoder closed-loop inverse transform failed for plane {plane:?}, block {block:?}: {source}"
    )]
    ClosedLoopInverseTransform {
        /// Plane whose reconstruction was requested.
        plane: PlaneId,
        /// Visible-plane-relative transform block rectangle.
        block: splot_recon::PlaneRect,
        /// Underlying reconstruction inverse-transform error.
        #[source]
        source: ReconError,
    },

    /// Decoder-visible residual addition rejected closed-loop reconstruction.
    #[error(
        "encoder closed-loop residual addition failed for plane {plane:?}, block {block:?}: {source}"
    )]
    ClosedLoopResidualAdd {
        /// Plane whose reconstruction was requested.
        plane: PlaneId,
        /// Visible-plane-relative transform block rectangle.
        block: splot_recon::PlaneRect,
        /// Underlying reconstruction residual-addition error.
        #[source]
        source: ReconError,
    },

    /// Building or freezing the closed-loop reconstruction workspace failed.
    #[error(
        "encoder closed-loop reconstruction workspace failed while preparing {context}: {source}"
    )]
    ClosedLoopWorkspace {
        /// Short description of the failed workspace step.
        context: &'static str,
        /// Underlying reconstruction workspace error.
        #[source]
        source: ReconError,
    },

    /// Intra-mode emission allocation failed.
    #[error("failed to allocate encoder intra-mode emission storage for {context}")]
    IntraModeEmissionAllocationFailed {
        /// Short description of the failed allocation.
        context: &'static str,
    },

    /// An intra-mode token carried a CDF selector outside the current minimal subset.
    #[error("unsupported encoder intra-mode token CDF selector for {syntax}")]
    IntraModeEmissionUnsupportedCdfSelector {
        /// Token syntax whose selector is unsupported.
        syntax: &'static str,
    },

    /// Writing an intra-mode token through the symbol encoder failed.
    #[error("encoder intra-mode emission symbol write failed for {syntax}: {source}")]
    IntraModeEmissionSymbolWrite {
        /// Token syntax being written.
        syntax: &'static str,
        /// Source symbol-encoder error.
        #[source]
        source: WriteError,
    },

    /// Finalizing intra-mode token symbol bytes failed.
    #[error("encoder intra-mode emission symbol encoder finalization failed: {source}")]
    IntraModeEmissionSymbolEncodeFinish {
        /// Source symbol-encoder error.
        #[source]
        source: WriteError,
    },

    /// Initializing the intra-mode token symbol decoder failed.
    #[error("encoder intra-mode emission symbol decoder initialization failed: {source}")]
    IntraModeEmissionSymbolDecodeInit {
        /// Source symbol-decoder error.
        #[source]
        source: splot_core::Error,
    },

    /// Reading an intra-mode token through the symbol decoder failed.
    #[error("encoder intra-mode emission symbol read failed for {syntax}: {source}")]
    IntraModeEmissionSymbolRead {
        /// Token syntax being read.
        syntax: &'static str,
        /// Source symbol-decoder error.
        #[source]
        source: splot_core::Error,
    },

    /// Decoded intra-mode token symbol value did not match the encoded value.
    #[error(
        "encoder intra-mode emission symbol mismatch for {syntax}: expected {expected}, decoded {actual}"
    )]
    IntraModeEmissionSymbolMismatch {
        /// Token syntax being compared.
        syntax: &'static str,
        /// Encoded symbol value.
        expected: u8,
        /// Decoded symbol value.
        actual: u8,
    },

    /// Finalizing the intra-mode token symbol decoder failed.
    #[error("encoder intra-mode emission symbol decoder finalization failed: {source}")]
    IntraModeEmissionSymbolDecodeFinish {
        /// Source symbol-decoder error.
        #[source]
        source: splot_core::Error,
    },

    /// Block-symbol trace allocation failed.
    #[error("failed to allocate encoder block-symbol trace storage for {context}")]
    BlockSymbolTraceAllocationFailed {
        /// Short description of the failed allocation.
        context: &'static str,
    },

    /// A block-symbol trace token carried a selector outside the current minimal subset.
    #[error("unsupported encoder block-symbol trace selector for token {index}")]
    BlockSymbolTraceUnsupportedSelector {
        /// Zero-based index of the unsupported token in the trace.
        index: usize,
    },

    /// Writing a block-symbol trace token through the symbol encoder failed.
    #[error("encoder block-symbol trace symbol write failed for token {index}: {source}")]
    BlockSymbolTraceSymbolWrite {
        /// Zero-based index of the failing token in the trace.
        index: usize,
        /// Source symbol-encoder error.
        #[source]
        source: WriteError,
    },

    /// Finalizing block-symbol trace symbol bytes failed.
    #[error("encoder block-symbol trace symbol encoder finalization failed: {source}")]
    BlockSymbolTraceSymbolEncodeFinish {
        /// Source symbol-encoder error.
        #[source]
        source: WriteError,
    },

    /// Initializing the block-symbol trace symbol decoder failed.
    #[error("encoder block-symbol trace symbol decoder initialization failed: {source}")]
    BlockSymbolTraceSymbolDecodeInit {
        /// Source symbol-decoder error.
        #[source]
        source: splot_core::Error,
    },

    /// Reading a block-symbol trace token through the symbol decoder failed.
    #[error("encoder block-symbol trace symbol read failed for token {index}: {source}")]
    BlockSymbolTraceSymbolRead {
        /// Zero-based index of the failing token in the trace.
        index: usize,
        /// Source symbol-decoder error.
        #[source]
        source: splot_core::Error,
    },

    /// Decoded block-symbol trace token value did not match the encoded value.
    #[error(
        "encoder block-symbol trace symbol mismatch for token {index}: expected {expected}, decoded {actual}"
    )]
    BlockSymbolTraceSymbolMismatch {
        /// Zero-based index of the mismatched token in the trace.
        index: usize,
        /// Encoded symbol value.
        expected: u8,
        /// Decoded symbol value.
        actual: u8,
    },

    /// A decoded bypass literal did not match the encoded full-width value.
    #[error(
        "encoder block-symbol trace bypass-literal mismatch for token {index} (width {width}): expected {expected}, decoded {actual}"
    )]
    BlockSymbolTraceLiteralMismatch {
        /// Zero-based index of the mismatched token in the trace.
        index: usize,
        /// Literal bit width.
        width: u32,
        /// Encoded full-width literal value.
        expected: u32,
        /// Decoded full-width literal value.
        actual: u32,
    },

    /// Finalizing the block-symbol trace symbol decoder failed.
    #[error("encoder block-symbol trace symbol decoder finalization failed: {source}")]
    BlockSymbolTraceSymbolDecodeFinish {
        /// Source symbol-decoder error.
        #[source]
        source: splot_core::Error,
    },

    /// An encoder lifecycle operation is invalid in the current context state.
    #[error("encoder operation {operation:?} is invalid while the context is {state:?}")]
    State {
        /// Requested operation.
        operation: EncoderOperation,
        /// Current context state.
        state: EncoderState,
    },
}

/// A specialized [`Result`](core::result::Result) for encoder context operations.
pub type Result<T> = core::result::Result<T, Error>;
