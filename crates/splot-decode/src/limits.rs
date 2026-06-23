// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Source-backed runtime resource-limit API for future decode planning.
//!
//! Feature tracking: `DECODE-LIMITS-RUNTIME-API`.

use core::fmt;

const DEFAULT_MAX_INPUT_BYTES: u64 = 16 * 1024 * 1024;
// Keep defaults finite, but size traversal counts so the current ac0ej3 mission
// target can reach the runtime's honest unsupported-feature gates.
const DEFAULT_MAX_OBUS: u64 = 16_384;
const DEFAULT_MAX_IVF_FRAME_RECORDS: u64 = 4_096;
const DEFAULT_MAX_FRAMES_TO_DECODE: u64 = 8_192;
const DEFAULT_MAX_OUTPUT_FRAMES: u64 = 8_192;
const DEFAULT_MAX_FRAME_WIDTH: u64 = 4_096;
const DEFAULT_MAX_FRAME_HEIGHT: u64 = 4_096;
const DEFAULT_MAX_LUMA_SAMPLES_PER_FRAME: u64 = DEFAULT_MAX_FRAME_WIDTH * DEFAULT_MAX_FRAME_HEIGHT;
const DEFAULT_MAX_DECODED_FRAME_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_MAX_REFERENCE_SLOTS: u64 = 16;
const DEFAULT_MAX_REFERENCE_STORE_BYTES: u64 = 256 * 1024 * 1024;
const DEFAULT_MAX_TILE_COUNT: u64 = 4_096;
const DEFAULT_MAX_TILE_PARTITION_STEPS: u64 = 1_048_576;
const DEFAULT_MAX_TILE_PAYLOAD_BYTES: u64 = 16 * 1024 * 1024;
const DEFAULT_MAX_LOOP_RESTORATION_SOURCE_READS: u64 = DEFAULT_MAX_LUMA_SAMPLES_PER_FRAME * 96;
const DEFAULT_MAX_OUTPUT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_HOST_ALLOCATION_LEN: u64 = isize::MAX as u64;

/// Runtime decoder options supplied by the caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeOptions {
    limits: DecodeLimits,
}

impl DecodeOptions {
    /// Default decoder options with finite resource limits.
    pub const DEFAULT: Self = Self {
        limits: DecodeLimits::DEFAULT,
    };

    /// Creates decoder options from caller-provided resource limits.
    #[must_use]
    pub const fn new(limits: DecodeLimits) -> Self {
        Self { limits }
    }

    /// Returns the configured resource limits.
    #[must_use]
    pub const fn limits(self) -> DecodeLimits {
        self.limits
    }

    /// Returns a copy with the configured resource limits replaced.
    #[must_use]
    pub const fn with_limits(self, limits: DecodeLimits) -> Self {
        Self { limits }
    }
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Caller-provided resource limits for future decode planning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeLimits {
    max_input_bytes: DecodeLimitThreshold,
    max_obus: DecodeLimitThreshold,
    max_ivf_frame_records: DecodeLimitThreshold,
    max_frames_to_decode: DecodeLimitThreshold,
    max_output_frames: DecodeLimitThreshold,
    max_frame_width: DecodeLimitThreshold,
    max_frame_height: DecodeLimitThreshold,
    max_luma_samples_per_frame: DecodeLimitThreshold,
    max_decoded_frame_bytes: DecodeLimitThreshold,
    max_reference_slots: DecodeLimitThreshold,
    max_reference_store_bytes: DecodeLimitThreshold,
    max_tile_count: DecodeLimitThreshold,
    max_tile_partition_steps: DecodeLimitThreshold,
    max_tile_payload_bytes: DecodeLimitThreshold,
    max_loop_restoration_source_reads: DecodeLimitThreshold,
    max_output_bytes: DecodeLimitThreshold,
}

impl DecodeLimits {
    /// Default finite resource policy for CI, fuzzing, and early decoder work.
    pub const DEFAULT: Self = Self {
        max_input_bytes: DecodeLimitThreshold::Max(DEFAULT_MAX_INPUT_BYTES),
        max_obus: DecodeLimitThreshold::Max(DEFAULT_MAX_OBUS),
        max_ivf_frame_records: DecodeLimitThreshold::Max(DEFAULT_MAX_IVF_FRAME_RECORDS),
        max_frames_to_decode: DecodeLimitThreshold::Max(DEFAULT_MAX_FRAMES_TO_DECODE),
        max_output_frames: DecodeLimitThreshold::Max(DEFAULT_MAX_OUTPUT_FRAMES),
        max_frame_width: DecodeLimitThreshold::Max(DEFAULT_MAX_FRAME_WIDTH),
        max_frame_height: DecodeLimitThreshold::Max(DEFAULT_MAX_FRAME_HEIGHT),
        max_luma_samples_per_frame: DecodeLimitThreshold::Max(DEFAULT_MAX_LUMA_SAMPLES_PER_FRAME),
        max_decoded_frame_bytes: DecodeLimitThreshold::Max(DEFAULT_MAX_DECODED_FRAME_BYTES),
        max_reference_slots: DecodeLimitThreshold::Max(DEFAULT_MAX_REFERENCE_SLOTS),
        max_reference_store_bytes: DecodeLimitThreshold::Max(DEFAULT_MAX_REFERENCE_STORE_BYTES),
        max_tile_count: DecodeLimitThreshold::Max(DEFAULT_MAX_TILE_COUNT),
        max_tile_partition_steps: DecodeLimitThreshold::Max(DEFAULT_MAX_TILE_PARTITION_STEPS),
        max_tile_payload_bytes: DecodeLimitThreshold::Max(DEFAULT_MAX_TILE_PAYLOAD_BYTES),
        max_loop_restoration_source_reads: DecodeLimitThreshold::Max(
            DEFAULT_MAX_LOOP_RESTORATION_SOURCE_READS,
        ),
        max_output_bytes: DecodeLimitThreshold::Max(DEFAULT_MAX_OUTPUT_BYTES),
    };

    /// Returns an explicit policy with every limit set to zero.
    #[must_use]
    pub const fn zero() -> Self {
        Self::all(DecodeLimitThreshold::Max(0))
    }

    /// Returns an explicit policy with every limit unlimited.
    #[must_use]
    pub const fn unlimited() -> Self {
        Self::all(DecodeLimitThreshold::Unlimited)
    }

    const fn all(threshold: DecodeLimitThreshold) -> Self {
        Self {
            max_input_bytes: threshold,
            max_obus: threshold,
            max_ivf_frame_records: threshold,
            max_frames_to_decode: threshold,
            max_output_frames: threshold,
            max_frame_width: threshold,
            max_frame_height: threshold,
            max_luma_samples_per_frame: threshold,
            max_decoded_frame_bytes: threshold,
            max_reference_slots: threshold,
            max_reference_store_bytes: threshold,
            max_tile_count: threshold,
            max_tile_partition_steps: threshold,
            max_tile_payload_bytes: threshold,
            max_loop_restoration_source_reads: threshold,
            max_output_bytes: threshold,
        }
    }

    /// Returns the configured threshold for a typed limit name.
    #[must_use]
    pub const fn threshold(self, name: DecodeLimitName) -> DecodeLimitThreshold {
        match name {
            DecodeLimitName::MaxInputBytes => self.max_input_bytes,
            DecodeLimitName::MaxObus => self.max_obus,
            DecodeLimitName::MaxIvfFrameRecords => self.max_ivf_frame_records,
            DecodeLimitName::MaxFramesToDecode => self.max_frames_to_decode,
            DecodeLimitName::MaxOutputFrames => self.max_output_frames,
            DecodeLimitName::MaxFrameWidth => self.max_frame_width,
            DecodeLimitName::MaxFrameHeight => self.max_frame_height,
            DecodeLimitName::MaxLumaSamplesPerFrame => self.max_luma_samples_per_frame,
            DecodeLimitName::MaxDecodedFrameBytes => self.max_decoded_frame_bytes,
            DecodeLimitName::MaxReferenceSlots => self.max_reference_slots,
            DecodeLimitName::MaxReferenceStoreBytes => self.max_reference_store_bytes,
            DecodeLimitName::MaxTileCount => self.max_tile_count,
            DecodeLimitName::MaxTilePartitionSteps => self.max_tile_partition_steps,
            DecodeLimitName::MaxTilePayloadBytes => self.max_tile_payload_bytes,
            DecodeLimitName::MaxLoopRestorationSourceReads => {
                self.max_loop_restoration_source_reads
            }
            DecodeLimitName::MaxOutputBytes => self.max_output_bytes,
        }
    }

    /// Returns a copy with one typed limit threshold replaced.
    #[must_use]
    pub const fn with_limit(
        mut self,
        name: DecodeLimitName,
        threshold: DecodeLimitThreshold,
    ) -> Self {
        match name {
            DecodeLimitName::MaxInputBytes => self.max_input_bytes = threshold,
            DecodeLimitName::MaxObus => self.max_obus = threshold,
            DecodeLimitName::MaxIvfFrameRecords => self.max_ivf_frame_records = threshold,
            DecodeLimitName::MaxFramesToDecode => self.max_frames_to_decode = threshold,
            DecodeLimitName::MaxOutputFrames => self.max_output_frames = threshold,
            DecodeLimitName::MaxFrameWidth => self.max_frame_width = threshold,
            DecodeLimitName::MaxFrameHeight => self.max_frame_height = threshold,
            DecodeLimitName::MaxLumaSamplesPerFrame => {
                self.max_luma_samples_per_frame = threshold;
            }
            DecodeLimitName::MaxDecodedFrameBytes => self.max_decoded_frame_bytes = threshold,
            DecodeLimitName::MaxReferenceSlots => self.max_reference_slots = threshold,
            DecodeLimitName::MaxReferenceStoreBytes => {
                self.max_reference_store_bytes = threshold;
            }
            DecodeLimitName::MaxTileCount => self.max_tile_count = threshold,
            DecodeLimitName::MaxTilePartitionSteps => {
                self.max_tile_partition_steps = threshold;
            }
            DecodeLimitName::MaxTilePayloadBytes => self.max_tile_payload_bytes = threshold,
            DecodeLimitName::MaxLoopRestorationSourceReads => {
                self.max_loop_restoration_source_reads = threshold;
            }
            DecodeLimitName::MaxOutputBytes => self.max_output_bytes = threshold,
        }
        self
    }

    /// Checks an actual value against the configured threshold for a typed limit.
    #[must_use]
    pub const fn check(self, name: DecodeLimitName, actual: u64) -> DecodeLimitCheck {
        DecodeLimitCheck::new(name, self.threshold(name), actual)
    }

    /// Checks an actual value and returns a local error when the limit fails.
    pub fn ensure(self, name: DecodeLimitName, actual: u64) -> DecodeLimitResult<DecodeLimitCheck> {
        let check = self.check(name, actual);
        if check.is_allowed() {
            Ok(check)
        } else {
            Err(DecodeLimitError::LimitExceeded { check })
        }
    }

    /// Computes checked addition, then compares the sum against a typed limit.
    pub fn ensure_add(
        self,
        name: DecodeLimitName,
        left: u64,
        right: u64,
    ) -> DecodeLimitResult<DecodeLimitCheck> {
        let actual = left
            .checked_add(right)
            .ok_or(DecodeLimitError::ArithmeticOverflow {
                name,
                op: DecodeLimitOp::Add,
                left,
                right,
            })?;
        self.ensure(name, actual)
    }

    /// Computes checked multiplication, then compares the product against a typed limit.
    pub fn ensure_mul(
        self,
        name: DecodeLimitName,
        left: u64,
        right: u64,
    ) -> DecodeLimitResult<DecodeLimitCheck> {
        let actual = left
            .checked_mul(right)
            .ok_or(DecodeLimitError::ArithmeticOverflow {
                name,
                op: DecodeLimitOp::Mul,
                left,
                right,
            })?;
        self.ensure(name, actual)
    }

    /// Checks a limit and converts the accepted value to a host allocation length.
    pub fn ensure_allocation_len(
        self,
        name: DecodeLimitName,
        actual: u64,
    ) -> DecodeLimitResult<usize> {
        self.ensure(name, actual)?;
        if actual > MAX_HOST_ALLOCATION_LEN {
            return Err(DecodeLimitError::HostAllocationTooLarge { name, actual });
        }
        usize::try_from(actual)
            .map_err(|_| DecodeLimitError::HostAllocationTooLarge { name, actual })
    }

    /// Returns the maximum input byte threshold.
    #[must_use]
    pub const fn max_input_bytes(self) -> DecodeLimitThreshold {
        self.threshold(DecodeLimitName::MaxInputBytes)
    }

    /// Returns a copy with the maximum input byte threshold replaced.
    #[must_use]
    pub const fn with_max_input_bytes(self, threshold: DecodeLimitThreshold) -> Self {
        self.with_limit(DecodeLimitName::MaxInputBytes, threshold)
    }

    /// Returns the maximum OBU count threshold.
    #[must_use]
    pub const fn max_obus(self) -> DecodeLimitThreshold {
        self.threshold(DecodeLimitName::MaxObus)
    }

    /// Returns a copy with the maximum OBU count threshold replaced.
    #[must_use]
    pub const fn with_max_obus(self, threshold: DecodeLimitThreshold) -> Self {
        self.with_limit(DecodeLimitName::MaxObus, threshold)
    }

    /// Returns the maximum IVF frame-record traversal threshold.
    #[must_use]
    pub const fn max_ivf_frame_records(self) -> DecodeLimitThreshold {
        self.threshold(DecodeLimitName::MaxIvfFrameRecords)
    }

    /// Returns a copy with the maximum IVF frame-record threshold replaced.
    #[must_use]
    pub const fn with_max_ivf_frame_records(self, threshold: DecodeLimitThreshold) -> Self {
        self.with_limit(DecodeLimitName::MaxIvfFrameRecords, threshold)
    }

    /// Returns the maximum decoded frame count threshold.
    #[must_use]
    pub const fn max_frames_to_decode(self) -> DecodeLimitThreshold {
        self.threshold(DecodeLimitName::MaxFramesToDecode)
    }

    /// Returns a copy with the maximum decoded frame count threshold replaced.
    #[must_use]
    pub const fn with_max_frames_to_decode(self, threshold: DecodeLimitThreshold) -> Self {
        self.with_limit(DecodeLimitName::MaxFramesToDecode, threshold)
    }

    /// Returns the maximum output frame count threshold.
    #[must_use]
    pub const fn max_output_frames(self) -> DecodeLimitThreshold {
        self.threshold(DecodeLimitName::MaxOutputFrames)
    }

    /// Returns a copy with the maximum output frame count threshold replaced.
    #[must_use]
    pub const fn with_max_output_frames(self, threshold: DecodeLimitThreshold) -> Self {
        self.with_limit(DecodeLimitName::MaxOutputFrames, threshold)
    }

    /// Returns the maximum frame width threshold.
    #[must_use]
    pub const fn max_frame_width(self) -> DecodeLimitThreshold {
        self.threshold(DecodeLimitName::MaxFrameWidth)
    }

    /// Returns a copy with the maximum frame width threshold replaced.
    #[must_use]
    pub const fn with_max_frame_width(self, threshold: DecodeLimitThreshold) -> Self {
        self.with_limit(DecodeLimitName::MaxFrameWidth, threshold)
    }

    /// Returns the maximum frame height threshold.
    #[must_use]
    pub const fn max_frame_height(self) -> DecodeLimitThreshold {
        self.threshold(DecodeLimitName::MaxFrameHeight)
    }

    /// Returns a copy with the maximum frame height threshold replaced.
    #[must_use]
    pub const fn with_max_frame_height(self, threshold: DecodeLimitThreshold) -> Self {
        self.with_limit(DecodeLimitName::MaxFrameHeight, threshold)
    }

    /// Returns the maximum luma samples per frame threshold.
    #[must_use]
    pub const fn max_luma_samples_per_frame(self) -> DecodeLimitThreshold {
        self.threshold(DecodeLimitName::MaxLumaSamplesPerFrame)
    }

    /// Returns a copy with the maximum luma samples per frame threshold replaced.
    #[must_use]
    pub const fn with_max_luma_samples_per_frame(self, threshold: DecodeLimitThreshold) -> Self {
        self.with_limit(DecodeLimitName::MaxLumaSamplesPerFrame, threshold)
    }

    /// Returns the maximum decoded frame byte threshold.
    #[must_use]
    pub const fn max_decoded_frame_bytes(self) -> DecodeLimitThreshold {
        self.threshold(DecodeLimitName::MaxDecodedFrameBytes)
    }

    /// Returns a copy with the maximum decoded frame byte threshold replaced.
    #[must_use]
    pub const fn with_max_decoded_frame_bytes(self, threshold: DecodeLimitThreshold) -> Self {
        self.with_limit(DecodeLimitName::MaxDecodedFrameBytes, threshold)
    }

    /// Returns the maximum reference slot threshold.
    #[must_use]
    pub const fn max_reference_slots(self) -> DecodeLimitThreshold {
        self.threshold(DecodeLimitName::MaxReferenceSlots)
    }

    /// Returns a copy with the maximum reference slot threshold replaced.
    #[must_use]
    pub const fn with_max_reference_slots(self, threshold: DecodeLimitThreshold) -> Self {
        self.with_limit(DecodeLimitName::MaxReferenceSlots, threshold)
    }

    /// Returns the maximum reference-store byte threshold.
    #[must_use]
    pub const fn max_reference_store_bytes(self) -> DecodeLimitThreshold {
        self.threshold(DecodeLimitName::MaxReferenceStoreBytes)
    }

    /// Returns a copy with the maximum reference-store byte threshold replaced.
    #[must_use]
    pub const fn with_max_reference_store_bytes(self, threshold: DecodeLimitThreshold) -> Self {
        self.with_limit(DecodeLimitName::MaxReferenceStoreBytes, threshold)
    }

    /// Returns the maximum tile count threshold.
    #[must_use]
    pub const fn max_tile_count(self) -> DecodeLimitThreshold {
        self.threshold(DecodeLimitName::MaxTileCount)
    }

    /// Returns a copy with the maximum tile count threshold replaced.
    #[must_use]
    pub const fn with_max_tile_count(self, threshold: DecodeLimitThreshold) -> Self {
        self.with_limit(DecodeLimitName::MaxTileCount, threshold)
    }

    /// Returns the maximum tile partition traversal step threshold.
    #[must_use]
    pub const fn max_tile_partition_steps(self) -> DecodeLimitThreshold {
        self.threshold(DecodeLimitName::MaxTilePartitionSteps)
    }

    /// Returns a copy with the maximum tile partition traversal step threshold replaced.
    #[must_use]
    pub const fn with_max_tile_partition_steps(self, threshold: DecodeLimitThreshold) -> Self {
        self.with_limit(DecodeLimitName::MaxTilePartitionSteps, threshold)
    }

    /// Returns the maximum tile payload byte threshold.
    #[must_use]
    pub const fn max_tile_payload_bytes(self) -> DecodeLimitThreshold {
        self.threshold(DecodeLimitName::MaxTilePayloadBytes)
    }

    /// Returns a copy with the maximum tile payload byte threshold replaced.
    #[must_use]
    pub const fn with_max_tile_payload_bytes(self, threshold: DecodeLimitThreshold) -> Self {
        self.with_limit(DecodeLimitName::MaxTilePayloadBytes, threshold)
    }

    /// Returns the maximum loop-restoration source-read operation threshold.
    #[must_use]
    pub const fn max_loop_restoration_source_reads(self) -> DecodeLimitThreshold {
        self.threshold(DecodeLimitName::MaxLoopRestorationSourceReads)
    }

    /// Returns a copy with the maximum loop-restoration source-read operation threshold replaced.
    #[must_use]
    pub const fn with_max_loop_restoration_source_reads(
        self,
        threshold: DecodeLimitThreshold,
    ) -> Self {
        self.with_limit(DecodeLimitName::MaxLoopRestorationSourceReads, threshold)
    }

    /// Returns the maximum output byte threshold.
    #[must_use]
    pub const fn max_output_bytes(self) -> DecodeLimitThreshold {
        self.threshold(DecodeLimitName::MaxOutputBytes)
    }

    /// Returns a copy with the maximum output byte threshold replaced.
    #[must_use]
    pub const fn with_max_output_bytes(self, threshold: DecodeLimitThreshold) -> Self {
        self.with_limit(DecodeLimitName::MaxOutputBytes, threshold)
    }
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Stable typed name for a configured decode resource limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecodeLimitName {
    /// Maximum accepted input bytes.
    MaxInputBytes,
    /// Maximum number of traversed OBUs.
    MaxObus,
    /// Maximum number of traversed IVF frame records.
    MaxIvfFrameRecords,
    /// Maximum number of frames selected for decode.
    MaxFramesToDecode,
    /// Maximum number of emitted output frames.
    MaxOutputFrames,
    /// Maximum coded frame width in luma samples.
    MaxFrameWidth,
    /// Maximum coded frame height in luma samples.
    MaxFrameHeight,
    /// Maximum coded luma samples per frame.
    MaxLumaSamplesPerFrame,
    /// Maximum decoded frame bytes.
    MaxDecodedFrameBytes,
    /// Maximum reference slots.
    MaxReferenceSlots,
    /// Maximum reference-store bytes.
    MaxReferenceStoreBytes,
    /// Maximum tile count.
    MaxTileCount,
    /// Maximum tile partition traversal steps.
    MaxTilePartitionSteps,
    /// Maximum tile payload bytes.
    MaxTilePayloadBytes,
    /// Maximum loop-restoration source-read operations.
    MaxLoopRestorationSourceReads,
    /// Maximum output bytes.
    MaxOutputBytes,
}

impl DecodeLimitName {
    /// Stable list of every decode resource limit name.
    pub const ALL: [Self; 16] = [
        Self::MaxInputBytes,
        Self::MaxObus,
        Self::MaxIvfFrameRecords,
        Self::MaxFramesToDecode,
        Self::MaxOutputFrames,
        Self::MaxFrameWidth,
        Self::MaxFrameHeight,
        Self::MaxLumaSamplesPerFrame,
        Self::MaxDecodedFrameBytes,
        Self::MaxReferenceSlots,
        Self::MaxReferenceStoreBytes,
        Self::MaxTileCount,
        Self::MaxTilePartitionSteps,
        Self::MaxTilePayloadBytes,
        Self::MaxLoopRestorationSourceReads,
        Self::MaxOutputBytes,
    ];

    /// Returns the stable snake_case name used in policy surfaces.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MaxInputBytes => "max_input_bytes",
            Self::MaxObus => "max_obus",
            Self::MaxIvfFrameRecords => "max_ivf_frame_records",
            Self::MaxFramesToDecode => "max_frames_to_decode",
            Self::MaxOutputFrames => "max_output_frames",
            Self::MaxFrameWidth => "max_frame_width",
            Self::MaxFrameHeight => "max_frame_height",
            Self::MaxLumaSamplesPerFrame => "max_luma_samples_per_frame",
            Self::MaxDecodedFrameBytes => "max_decoded_frame_bytes",
            Self::MaxReferenceSlots => "max_reference_slots",
            Self::MaxReferenceStoreBytes => "max_reference_store_bytes",
            Self::MaxTileCount => "max_tile_count",
            Self::MaxTilePartitionSteps => "max_tile_partition_steps",
            Self::MaxTilePayloadBytes => "max_tile_payload_bytes",
            Self::MaxLoopRestorationSourceReads => "max_loop_restoration_source_reads",
            Self::MaxOutputBytes => "max_output_bytes",
        }
    }

    /// Returns the unit for measured values compared against this limit.
    #[must_use]
    pub const fn unit(self) -> DecodeLimitUnit {
        match self {
            Self::MaxInputBytes
            | Self::MaxDecodedFrameBytes
            | Self::MaxReferenceStoreBytes
            | Self::MaxTilePayloadBytes
            | Self::MaxOutputBytes => DecodeLimitUnit::Bytes,
            Self::MaxFrameWidth | Self::MaxFrameHeight | Self::MaxLumaSamplesPerFrame => {
                DecodeLimitUnit::LumaSamples
            }
            Self::MaxObus
            | Self::MaxIvfFrameRecords
            | Self::MaxFramesToDecode
            | Self::MaxOutputFrames
            | Self::MaxReferenceSlots
            | Self::MaxTileCount
            | Self::MaxTilePartitionSteps
            | Self::MaxLoopRestorationSourceReads => DecodeLimitUnit::Count,
        }
    }
}

impl fmt::Display for DecodeLimitName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Unit used by a decode resource limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecodeLimitUnit {
    /// Bytes.
    Bytes,
    /// Counted items.
    Count,
    /// Luma samples.
    LumaSamples,
}

impl DecodeLimitUnit {
    /// Returns the stable snake_case unit spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bytes => "bytes",
            Self::Count => "count",
            Self::LumaSamples => "luma_samples",
        }
    }
}

impl fmt::Display for DecodeLimitUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Configured threshold for a decode resource limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecodeLimitThreshold {
    /// No finite threshold is configured.
    Unlimited,
    /// Inclusive maximum accepted value.
    Max(u64),
}

impl DecodeLimitThreshold {
    /// Creates an inclusive maximum threshold.
    #[must_use]
    pub const fn max(value: u64) -> Self {
        Self::Max(value)
    }

    /// Creates an unlimited threshold.
    #[must_use]
    pub const fn unlimited() -> Self {
        Self::Unlimited
    }

    /// Returns true when no finite threshold is configured.
    #[must_use]
    pub const fn is_unlimited(self) -> bool {
        matches!(self, Self::Unlimited)
    }

    /// Returns the configured maximum value, if the threshold is finite.
    #[must_use]
    pub const fn max_value(self) -> Option<u64> {
        match self {
            Self::Unlimited => None,
            Self::Max(value) => Some(value),
        }
    }

    /// Returns true when the actual value is accepted by this threshold.
    #[must_use]
    pub const fn allows(self, actual: u64) -> bool {
        match self {
            Self::Unlimited => true,
            Self::Max(maximum) => actual <= maximum,
        }
    }
}

impl fmt::Display for DecodeLimitThreshold {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unlimited => f.write_str("unlimited"),
            Self::Max(value) => write!(f, "{value}"),
        }
    }
}

/// Checked arithmetic operation used to derive a decode resource value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecodeLimitOp {
    /// Checked addition.
    Add,
    /// Checked multiplication.
    Mul,
}

impl DecodeLimitOp {
    /// Returns the stable operation spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Mul => "mul",
        }
    }
}

impl fmt::Display for DecodeLimitOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Result of a pure decode resource limit check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeLimitCheck {
    name: DecodeLimitName,
    threshold: DecodeLimitThreshold,
    actual: u64,
    unit: DecodeLimitUnit,
}

impl DecodeLimitCheck {
    /// Creates a resource limit check result.
    #[must_use]
    const fn new(name: DecodeLimitName, threshold: DecodeLimitThreshold, actual: u64) -> Self {
        Self {
            name,
            threshold,
            actual,
            unit: name.unit(),
        }
    }

    /// Returns the checked limit name.
    #[must_use]
    pub const fn name(self) -> DecodeLimitName {
        self.name
    }

    /// Returns the configured threshold.
    #[must_use]
    pub const fn threshold(self) -> DecodeLimitThreshold {
        self.threshold
    }

    /// Returns the measured actual value.
    #[must_use]
    pub const fn actual(self) -> u64 {
        self.actual
    }

    /// Returns the unit for the measured actual value.
    #[must_use]
    pub const fn unit(self) -> DecodeLimitUnit {
        self.unit
    }

    /// Returns true when the check passes the configured threshold.
    #[must_use]
    pub const fn is_allowed(self) -> bool {
        self.threshold.allows(self.actual)
    }

    /// Returns true when the check exceeds the configured threshold.
    #[must_use]
    pub const fn is_exceeded(self) -> bool {
        !self.is_allowed()
    }
}

/// Typed local failure from a decode resource limit helper.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecodeLimitError {
    /// A measured value exceeded its configured threshold.
    LimitExceeded {
        /// Failed check metadata.
        check: DecodeLimitCheck,
    },
    /// Checked arithmetic overflowed while deriving a measured value.
    ArithmeticOverflow {
        /// Limit being derived.
        name: DecodeLimitName,
        /// Checked operation that overflowed.
        op: DecodeLimitOp,
        /// Left operand supplied to the checked operation.
        left: u64,
        /// Right operand supplied to the checked operation.
        right: u64,
    },
    /// An accepted value cannot be represented as a host allocation length.
    HostAllocationTooLarge {
        /// Limit being converted to a host allocation length.
        name: DecodeLimitName,
        /// Accepted value that cannot fit in `usize`.
        actual: u64,
    },
}

impl DecodeLimitError {
    /// Returns the failed or derived limit name.
    #[must_use]
    pub const fn name(self) -> DecodeLimitName {
        match self {
            Self::LimitExceeded { check } => check.name(),
            Self::ArithmeticOverflow { name, .. } | Self::HostAllocationTooLarge { name, .. } => {
                name
            }
        }
    }

    /// Returns the failed limit check, if this error came from comparison.
    #[must_use]
    pub const fn check(self) -> Option<DecodeLimitCheck> {
        match self {
            Self::LimitExceeded { check } => Some(check),
            Self::ArithmeticOverflow { .. } | Self::HostAllocationTooLarge { .. } => None,
        }
    }

    /// Returns the arithmetic operation, if this error came from arithmetic.
    #[must_use]
    pub const fn op(self) -> Option<DecodeLimitOp> {
        match self {
            Self::ArithmeticOverflow { op, .. } => Some(op),
            Self::LimitExceeded { .. } | Self::HostAllocationTooLarge { .. } => None,
        }
    }

    /// Returns the left arithmetic operand, if this error came from arithmetic.
    #[must_use]
    pub const fn left(self) -> Option<u64> {
        match self {
            Self::ArithmeticOverflow { left, .. } => Some(left),
            Self::LimitExceeded { .. } | Self::HostAllocationTooLarge { .. } => None,
        }
    }

    /// Returns the right arithmetic operand, if this error came from arithmetic.
    #[must_use]
    pub const fn right(self) -> Option<u64> {
        match self {
            Self::ArithmeticOverflow { right, .. } => Some(right),
            Self::LimitExceeded { .. } | Self::HostAllocationTooLarge { .. } => None,
        }
    }

    /// Returns the actual value, if this error carries one.
    #[must_use]
    pub const fn actual(self) -> Option<u64> {
        match self {
            Self::LimitExceeded { check } => Some(check.actual()),
            Self::HostAllocationTooLarge { actual, .. } => Some(actual),
            Self::ArithmeticOverflow { .. } => None,
        }
    }
}

impl fmt::Display for DecodeLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::LimitExceeded { check } => write!(
                f,
                "decode limit {} exceeded: actual {} {} is greater than {}",
                check.name(),
                check.actual(),
                check.unit(),
                check.threshold()
            ),
            Self::ArithmeticOverflow {
                name,
                op,
                left,
                right,
            } => write!(
                f,
                "decode limit {name} overflow while deriving value with {op}: {left}, {right}"
            ),
            Self::HostAllocationTooLarge { name, actual } => write!(
                f,
                "decode limit {name} value {actual} cannot fit in a host allocation length"
            ),
        }
    }
}

impl std::error::Error for DecodeLimitError {}

/// Result alias for decode limit helpers.
pub type DecodeLimitResult<T> = Result<T, DecodeLimitError>;

#[cfg(test)]
mod tests;
