// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2-derived decoded output format facts.

use crate::{PlaneSize, ReconError, Result};

/// AV2 decoded sample bit depth from § 6.4.1 Table 6.3.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BitDepth {
    /// 8-bit decoded output samples.
    Eight,
    /// 10-bit decoded output samples.
    Ten,
}

impl BitDepth {
    /// Converts AV2 § 6.4.1 `bit_depth_idc` into a bit-depth value.
    ///
    /// # Errors
    /// Returns [`ReconError::UnsupportedBitDepthIdc`] for reserved values.
    pub const fn from_av2_bit_depth_idc(idc: u8) -> Result<Self> {
        match idc {
            0 => Ok(Self::Ten),
            1 => Ok(Self::Eight),
            _ => Err(ReconError::UnsupportedBitDepthIdc { idc }),
        }
    }

    /// Returns the number of significant bits per decoded sample.
    pub const fn bits(self) -> u8 {
        match self {
            Self::Eight => 8,
            Self::Ten => 10,
        }
    }

    /// Returns the maximum legal decoded sample value for this bit depth.
    pub const fn max_sample(self) -> u16 {
        match self {
            Self::Eight => 255,
            Self::Ten => 1023,
        }
    }
}

/// AV2 decoded output chroma format from § 6.4.1 Table 6.2.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PixelFormat {
    /// Monochrome output, AV2 `CHROMA_FORMAT_400`.
    Monochrome,
    /// YUV 4:2:0 output, AV2 `CHROMA_FORMAT_420`.
    Yuv420,
    /// YUV 4:2:2 output, AV2 `CHROMA_FORMAT_422`.
    Yuv422,
    /// YUV 4:4:4 output, AV2 `CHROMA_FORMAT_444`.
    Yuv444,
}

impl PixelFormat {
    /// Converts AV2 § 6.4.1 `chroma_format_idc` into a pixel format.
    ///
    /// # Errors
    /// Returns [`ReconError::UnsupportedChromaFormatIdc`] for values above 3.
    pub const fn from_av2_chroma_format_idc(idc: u8) -> Result<Self> {
        match idc {
            0 => Ok(Self::Yuv420),
            1 => Ok(Self::Monochrome),
            2 => Ok(Self::Yuv444),
            3 => Ok(Self::Yuv422),
            _ => Err(ReconError::UnsupportedChromaFormatIdc { idc }),
        }
    }

    /// Returns the AV2 § 6.4.1 `chroma_format_idc` value for this format.
    pub const fn chroma_format_idc(self) -> u8 {
        match self {
            Self::Yuv420 => 0,
            Self::Monochrome => 1,
            Self::Yuv444 => 2,
            Self::Yuv422 => 3,
        }
    }

    /// Returns AV2 § 6.4.1 `SubsamplingX`.
    pub const fn subsampling_x(self) -> u8 {
        match self {
            Self::Yuv444 => 0,
            Self::Monochrome | Self::Yuv420 | Self::Yuv422 => 1,
        }
    }

    /// Returns AV2 § 6.4.1 `SubsamplingY`.
    pub const fn subsampling_y(self) -> u8 {
        match self {
            Self::Yuv422 | Self::Yuv444 => 0,
            Self::Monochrome | Self::Yuv420 => 1,
        }
    }

    /// Returns whether AV2 § 6.4.1 `Monochrome` is true.
    pub const fn is_monochrome(self) -> bool {
        matches!(self, Self::Monochrome)
    }

    /// Returns AV2 § 6.4.1 `NumPlanes`.
    pub const fn num_planes(self) -> usize {
        if self.is_monochrome() { 1 } else { 3 }
    }

    /// Derives the § 7.21.2 chroma output size from a visible luma size.
    ///
    /// Returns `Ok(None)` for monochrome output because U and V are absent.
    ///
    /// # Errors
    /// Returns [`ReconError::ArithmeticOverflow`] if adding the AV2 subsampling
    /// offset would overflow.
    pub fn chroma_size(self, luma_size: PlaneSize) -> Result<Option<PlaneSize>> {
        if self.is_monochrome() {
            return Ok(None);
        }

        let sub_x = usize::from(self.subsampling_x());
        let sub_y = usize::from(self.subsampling_y());
        let width = luma_size
            .width()
            .checked_add(sub_x)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "chroma width",
            })?
            >> sub_x;
        let height =
            luma_size
                .height()
                .checked_add(sub_y)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "chroma height",
                })?
                >> sub_y;

        PlaneSize::new(width, height).map(Some)
    }
}

/// Decoded output plane identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PlaneId {
    /// Luma plane.
    Y,
    /// Blue-difference chroma plane.
    U,
    /// Red-difference chroma plane.
    V,
}

impl PlaneId {
    /// Returns the canonical plane order index: Y = 0, U = 1, V = 2.
    pub const fn index(self) -> usize {
        match self {
            Self::Y => 0,
            Self::U => 1,
            Self::V => 2,
        }
    }

    /// Returns the canonical plane name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Y => "Y",
            Self::U => "U",
            Self::V => "V",
        }
    }
}

mod private {
    pub trait Sealed {}

    impl Sealed for u8 {}
    impl Sealed for u16 {}
}

/// Sealed decoded sample storage type supported by `splot-recon`.
pub trait ReconSample: private::Sealed + Copy + Default + Send + Sync + 'static {
    /// Human-readable Rust type name used in diagnostics.
    const TYPE_NAME: &'static str;

    /// Maximum value representable by this storage type.
    const MAX_VALUE: u16;

    /// Converts the sample to `u16` for bit-depth range validation.
    fn to_u16(self) -> u16;

    /// Converts a decoded sample value into this storage type.
    ///
    /// # Errors
    /// Returns [`ReconError::SampleValueUnsupportedStorage`] if `value` cannot
    /// be represented by this storage type.
    fn try_from_u16(value: u16) -> Result<Self>;

    /// Returns whether this storage type can represent the supplied bit depth.
    fn supports_bit_depth(bit_depth: BitDepth) -> bool {
        Self::MAX_VALUE >= bit_depth.max_sample()
    }

    /// Reinterprets a sample slice as `u16` storage when this type is `u16`,
    /// letting reference readers borrow plane storage without a widening
    /// copy; `None` for narrower storage types.
    fn u16_slice(samples: &[Self]) -> Option<&[u16]>;

    /// Reinterprets a mutable sample slice as `u16` storage when this type is
    /// `u16`; `None` for narrower storage types.
    fn u16_slice_mut(samples: &mut [Self]) -> Option<&mut [u16]>;

    /// Reinterprets a mutable sample slice as `u8` storage when this type is
    /// `u8`; `None` for wider storage types.
    fn u8_slice_mut(samples: &mut [Self]) -> Option<&mut [u8]>;

    /// Process-global retained pool of transient reconstruction-workspace plane
    /// sample buffers for this storage type.
    #[doc(hidden)]
    fn recon_plane_pool() -> &'static std::sync::Mutex<Vec<Vec<Self>>>;
}

impl ReconSample for u8 {
    const TYPE_NAME: &'static str = "u8";
    const MAX_VALUE: u16 = u8::MAX as u16;

    fn to_u16(self) -> u16 {
        u16::from(self)
    }

    fn try_from_u16(value: u16) -> Result<Self> {
        u8::try_from(value).map_err(|_| ReconError::SampleValueUnsupportedStorage {
            sample_type: Self::TYPE_NAME,
            value,
            max: Self::MAX_VALUE,
        })
    }
    fn u16_slice(_samples: &[Self]) -> Option<&[u16]> {
        None
    }

    fn u16_slice_mut(_samples: &mut [Self]) -> Option<&mut [u16]> {
        None
    }

    fn u8_slice_mut(samples: &mut [Self]) -> Option<&mut [u8]> {
        Some(samples)
    }

    fn recon_plane_pool() -> &'static std::sync::Mutex<Vec<Vec<Self>>> {
        static POOL: std::sync::Mutex<Vec<Vec<u8>>> = std::sync::Mutex::new(Vec::new());
        &POOL
    }
}

impl ReconSample for u16 {
    const TYPE_NAME: &'static str = "u16";
    const MAX_VALUE: u16 = u16::MAX;

    fn to_u16(self) -> u16 {
        self
    }

    fn try_from_u16(value: u16) -> Result<Self> {
        Ok(value)
    }
    fn u16_slice(samples: &[Self]) -> Option<&[u16]> {
        Some(samples)
    }

    fn u16_slice_mut(samples: &mut [Self]) -> Option<&mut [u16]> {
        Some(samples)
    }

    fn u8_slice_mut(_samples: &mut [Self]) -> Option<&mut [u8]> {
        None
    }

    fn recon_plane_pool() -> &'static std::sync::Mutex<Vec<Vec<Self>>> {
        static POOL: std::sync::Mutex<Vec<Vec<u16>>> = std::sync::Mutex::new(Vec::new());
        &POOL
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn size(width: usize, height: usize) -> PlaneSize {
        PlaneSize::new(width, height).unwrap()
    }

    #[test]
    fn bit_depth_idc_matches_av2_table() {
        assert_eq!(BitDepth::from_av2_bit_depth_idc(0), Ok(BitDepth::Ten));
        assert_eq!(BitDepth::from_av2_bit_depth_idc(1), Ok(BitDepth::Eight));
        assert!(matches!(
            BitDepth::from_av2_bit_depth_idc(2),
            Err(ReconError::UnsupportedBitDepthIdc { idc: 2 })
        ));
        assert_eq!(BitDepth::Eight.bits(), 8);
        assert_eq!(BitDepth::Ten.bits(), 10);
        assert_eq!(BitDepth::Eight.max_sample(), 255);
        assert_eq!(BitDepth::Ten.max_sample(), 1023);
    }

    #[test]
    fn sample_conversion_rejects_values_outside_storage_type() {
        assert_eq!(u8::try_from_u16(255), Ok(255));
        assert!(matches!(
            u8::try_from_u16(256),
            Err(ReconError::SampleValueUnsupportedStorage {
                sample_type: "u8",
                value: 256,
                max: 255
            })
        ));
        assert_eq!(u16::try_from_u16(1023), Ok(1023));
    }

    #[test]
    fn chroma_format_idc_matches_av2_table() {
        assert_eq!(
            PixelFormat::from_av2_chroma_format_idc(0),
            Ok(PixelFormat::Yuv420)
        );
        assert_eq!(
            PixelFormat::from_av2_chroma_format_idc(1),
            Ok(PixelFormat::Monochrome)
        );
        assert_eq!(
            PixelFormat::from_av2_chroma_format_idc(2),
            Ok(PixelFormat::Yuv444)
        );
        assert_eq!(
            PixelFormat::from_av2_chroma_format_idc(3),
            Ok(PixelFormat::Yuv422)
        );
        assert!(matches!(
            PixelFormat::from_av2_chroma_format_idc(4),
            Err(ReconError::UnsupportedChromaFormatIdc { idc: 4 })
        ));

        assert_eq!(PixelFormat::Yuv420.subsampling_x(), 1);
        assert_eq!(PixelFormat::Yuv420.subsampling_y(), 1);
        assert_eq!(PixelFormat::Monochrome.num_planes(), 1);
        assert_eq!(PixelFormat::Yuv444.num_planes(), 3);
    }

    #[test]
    fn chroma_size_uses_output_process_rounding() {
        let luma = size(5, 3);
        assert_eq!(PixelFormat::Yuv420.chroma_size(luma), Ok(Some(size(3, 2))));
        assert_eq!(PixelFormat::Yuv422.chroma_size(luma), Ok(Some(size(3, 3))));
        assert_eq!(PixelFormat::Yuv444.chroma_size(luma), Ok(Some(size(5, 3))));
        assert_eq!(PixelFormat::Monochrome.chroma_size(luma), Ok(None));
    }
}
