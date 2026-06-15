// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! IVF (`DKIF`) container parsing and writing for Annex B payloads.
//!
//! IVF is a simple non-normative container used by AV tooling. `splot` treats it
//! only as a byte envelope (`AV2-IVF-CONTAINER`): each frame payload remains
//! opaque here and is parsed by the Annex B parser at the stream layer.

use core::fmt;
use std::io;

use serde::{Deserialize, Serialize};
use zerocopy::byteorder::little_endian::{U16, U32};
use zerocopy::{FromBytes, Immutable, KnownLayout, Unaligned};

use crate::span::ByteOffset;

/// IVF file signature.
pub const IVF_SIGNATURE: [u8; 4] = *b"DKIF";

/// Size of the baseline IVF header in bytes.
pub const IVF_HEADER_SIZE: u16 = 32;

const IVF_HEADER_SIZE_BYTES: usize = 32;

/// Size of each IVF frame header in bytes.
pub const IVF_FRAME_HEADER_SIZE: usize = 12;

/// A parsed IVF file header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IvfHeader {
    /// IVF version field.
    pub version: u16,
    /// Declared header length in bytes.
    pub header_len: u16,
    /// Codec four-character code.
    pub fourcc: [u8; 4],
    /// Coded width in pixels.
    pub width: u16,
    /// Coded height in pixels.
    pub height: u16,
    /// Timebase denominator.
    pub timebase_denominator: u32,
    /// Timebase numerator.
    pub timebase_numerator: u32,
    /// Declared number of frames in the file.
    pub frame_count: u32,
    /// IVF unused header field.
    pub unused: u32,
}

impl IvfHeader {
    /// Creates a baseline 32-byte IVF header value for writer use.
    #[must_use]
    pub const fn new(
        fourcc: [u8; 4],
        width: u16,
        height: u16,
        timebase_denominator: u32,
        timebase_numerator: u32,
        frame_count: u32,
    ) -> Self {
        Self {
            version: 0,
            header_len: IVF_HEADER_SIZE,
            fourcc,
            width,
            height,
            timebase_denominator,
            timebase_numerator,
            frame_count,
            unused: 0,
        }
    }

    /// Returns the first byte offset after the declared IVF header.
    #[must_use]
    pub fn payload_start_offset(self) -> ByteOffset {
        ByteOffset::new(u64::from(self.header_len))
    }
}

/// One IVF frame record and its payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IvfFrame<'a> {
    /// Zero-based frame index in file order.
    pub index: usize,
    /// Absolute offset of the 12-byte IVF frame header.
    pub header_offset: ByteOffset,
    /// Absolute offset of the frame payload.
    pub payload_offset: ByteOffset,
    /// Declared frame payload size in bytes.
    pub size: u32,
    /// Presentation timestamp.
    pub pts: u64,
    /// Frame payload bytes.
    pub payload: &'a [u8],
}

/// As much of an IVF stream as could be parsed.
#[derive(Debug)]
pub struct PartialIvfParse<'a> {
    /// Parsed header, if a complete valid header was available.
    pub header: Option<IvfHeader>,
    /// Frames parsed before the first structural container error.
    pub frames: Vec<IvfFrame<'a>>,
    /// Non-fatal container warnings encountered while parsing complete frames.
    pub warnings: Vec<IvfWarning>,
    /// Container error that stopped frame parsing, if any.
    pub error: Option<IvfError>,
}

/// One step produced by [`IvfFrameCursor`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IvfFrameRead<'a> {
    /// A complete IVF frame record and payload.
    Frame(IvfFrame<'a>),
    /// A non-fatal IVF warning encountered after complete frames.
    Warning(IvfWarning),
    /// No more IVF bytes remain.
    End,
}

/// Stateful cursor over IVF frame records after a parsed header.
///
/// This exposes the single-sourced IVF frame parser one record at a time,
/// allowing higher-level crates to apply their own limits between frame records
/// without copying container parsing logic.
#[derive(Debug, Clone)]
pub struct IvfFrameCursor<'a> {
    input: &'a [u8],
    cursor: usize,
    frame_index: usize,
    finished: bool,
}

impl<'a> IvfFrameCursor<'a> {
    /// Creates a cursor positioned after `header`'s declared header length.
    #[must_use]
    pub fn new(input: &'a [u8], header: IvfHeader) -> Self {
        Self {
            input,
            cursor: usize::from(header.header_len),
            frame_index: 0,
            finished: false,
        }
    }

    /// Returns whether unread IVF bytes remain.
    #[must_use]
    pub fn has_remaining(&self) -> bool {
        !self.finished && self.cursor < self.input.len()
    }

    /// Returns whether the cursor currently points at a complete frame header.
    #[must_use]
    pub fn has_complete_frame_header(&self) -> bool {
        self.has_remaining()
            && self.input.len().saturating_sub(self.cursor) >= IVF_FRAME_HEADER_SIZE
    }

    /// Returns the zero-based frame index that the next frame record would use.
    #[must_use]
    pub const fn next_frame_index(&self) -> usize {
        self.frame_index
    }

    /// Parses the next IVF frame record, warning, or end marker.
    ///
    /// # Errors
    /// Returns the first structural IVF frame-header or frame-payload error. The
    /// cursor is not advanced on error.
    pub fn next_frame_record(&mut self) -> Result<IvfFrameRead<'a>, IvfError> {
        if !self.has_remaining() {
            self.finished = true;
            return Ok(IvfFrameRead::End);
        }

        let remaining_header = self.input.len().saturating_sub(self.cursor);
        if remaining_header < IVF_FRAME_HEADER_SIZE {
            if self.frame_index > 0 {
                self.finished = true;
                return Ok(IvfFrameRead::Warning(
                    IvfWarning::TrailingPartialFrameHeader {
                        frame_index: self.frame_index,
                        offset: ByteOffset::new(self.input.len() as u64),
                        needed: IVF_FRAME_HEADER_SIZE.saturating_sub(remaining_header),
                    },
                ));
            }
            return Err(IvfError::TruncatedFrameHeader {
                frame_index: self.frame_index,
                offset: ByteOffset::new(self.input.len() as u64),
                needed: IVF_FRAME_HEADER_SIZE.saturating_sub(remaining_header),
            });
        }

        let size = read_u32_le(self.input, self.cursor).ok_or(IvfError::TruncatedFrameHeader {
            frame_index: self.frame_index,
            offset: ByteOffset::new(self.input.len() as u64),
            needed: IVF_FRAME_HEADER_SIZE.saturating_sub(remaining_header),
        })?;
        let pts = read_u64_le(self.input, self.cursor.saturating_add(4)).ok_or(
            IvfError::TruncatedFrameHeader {
                frame_index: self.frame_index,
                offset: ByteOffset::new(self.input.len() as u64),
                needed: IVF_FRAME_HEADER_SIZE.saturating_sub(remaining_header),
            },
        )?;

        let payload_start = self.cursor.saturating_add(IVF_FRAME_HEADER_SIZE);
        let remaining_payload = self.input.len().saturating_sub(payload_start);
        let size_usize = size as usize;
        if size_usize > remaining_payload {
            return Err(IvfError::TruncatedFramePayload {
                frame_index: self.frame_index,
                offset: ByteOffset::new(self.input.len() as u64),
                size,
                remaining: remaining_payload,
            });
        }

        let payload_end = payload_start.saturating_add(size_usize);
        let payload =
            self.input
                .get(payload_start..payload_end)
                .ok_or(IvfError::TruncatedFramePayload {
                    frame_index: self.frame_index,
                    offset: ByteOffset::new(self.input.len() as u64),
                    size,
                    remaining: remaining_payload,
                })?;

        let frame = IvfFrame {
            index: self.frame_index,
            header_offset: ByteOffset::new(self.cursor as u64),
            payload_offset: ByteOffset::new(payload_start as u64),
            size,
            pts,
            payload,
        };
        self.cursor = payload_end;
        self.frame_index = self.frame_index.saturating_add(1);
        Ok(IvfFrameRead::Frame(frame))
    }
}

/// Errors produced by the IVF container parser.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IvfError {
    /// The input ended before a complete IVF header could be read.
    TruncatedHeader {
        /// First missing byte offset.
        offset: ByteOffset,
        /// Number of additional bytes required.
        needed: usize,
    },
    /// The first four bytes were not the IVF `DKIF` signature.
    InvalidSignature {
        /// Offset of the signature.
        offset: ByteOffset,
        /// Signature bytes that were present.
        signature: [u8; 4],
    },
    /// The header length field was smaller than the baseline 32-byte header.
    InvalidHeaderLength {
        /// Offset of the header length field.
        offset: ByteOffset,
        /// Declared header length.
        header_len: u16,
    },
    /// The input ended before a complete IVF frame header could be read.
    TruncatedFrameHeader {
        /// Zero-based frame index whose header was truncated.
        frame_index: usize,
        /// First missing byte offset.
        offset: ByteOffset,
        /// Number of additional bytes required.
        needed: usize,
    },
    /// The input ended before the declared frame payload was complete.
    TruncatedFramePayload {
        /// Zero-based frame index whose payload was truncated.
        frame_index: usize,
        /// First missing byte offset.
        offset: ByteOffset,
        /// Declared frame payload size in bytes.
        size: u32,
        /// Bytes available after the frame header.
        remaining: usize,
    },
}

impl IvfError {
    /// Returns the stable validator diagnostic rule id for this error.
    #[must_use]
    pub const fn rule_id(&self) -> &'static str {
        match self {
            Self::TruncatedHeader { .. } => "ivf/truncated-header",
            Self::InvalidSignature { .. } => "ivf/invalid-signature",
            Self::InvalidHeaderLength { .. } => "ivf/invalid-header-length",
            Self::TruncatedFrameHeader { .. } => "ivf/truncated-frame-header",
            Self::TruncatedFramePayload { .. } => "ivf/truncated-frame-payload",
        }
    }

    /// Returns the byte offset carried by this error.
    #[must_use]
    pub const fn offset(&self) -> ByteOffset {
        match self {
            Self::TruncatedHeader { offset, .. }
            | Self::InvalidSignature { offset, .. }
            | Self::InvalidHeaderLength { offset, .. }
            | Self::TruncatedFrameHeader { offset, .. }
            | Self::TruncatedFramePayload { offset, .. } => *offset,
        }
    }
}

impl fmt::Display for IvfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TruncatedHeader { offset, needed } => write!(
                f,
                "truncated IVF header at byte {offset}: needed {needed} more byte(s)"
            ),
            Self::InvalidSignature { signature, .. } => write!(
                f,
                "invalid IVF signature: expected DKIF, found 0x{:02X}{:02X}{:02X}{:02X}",
                signature[0], signature[1], signature[2], signature[3]
            ),
            Self::InvalidHeaderLength { header_len, .. } => write!(
                f,
                "invalid IVF header length: {header_len} byte(s), expected at least {IVF_HEADER_SIZE}"
            ),
            Self::TruncatedFrameHeader {
                frame_index,
                offset,
                needed,
            } => write!(
                f,
                "truncated IVF frame {frame_index} header at byte {offset}: needed {needed} more byte(s)"
            ),
            Self::TruncatedFramePayload {
                frame_index,
                offset,
                size,
                remaining,
            } => write!(
                f,
                "truncated IVF frame {frame_index} payload at byte {offset}: declared {size} byte(s), only {remaining} available"
            ),
        }
    }
}

impl std::error::Error for IvfError {}

/// Non-fatal warnings produced by the IVF container parser.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IvfWarning {
    /// EOF occurred while probing a trailing IVF frame header after earlier frames.
    TrailingPartialFrameHeader {
        /// Zero-based frame index whose trailing header was partial.
        frame_index: usize,
        /// First missing byte offset.
        offset: ByteOffset,
        /// Number of additional bytes required for a complete frame header.
        needed: usize,
    },
}

impl IvfWarning {
    /// Returns the stable validator diagnostic rule id for this warning.
    #[must_use]
    pub const fn rule_id(&self) -> &'static str {
        match self {
            Self::TrailingPartialFrameHeader { .. } => "ivf/trailing-partial-frame-header",
        }
    }

    /// Returns the byte offset carried by this warning.
    #[must_use]
    pub const fn offset(&self) -> ByteOffset {
        match self {
            Self::TrailingPartialFrameHeader { offset, .. } => *offset,
        }
    }
}

impl fmt::Display for IvfWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TrailingPartialFrameHeader {
                frame_index,
                offset,
                needed,
            } => write!(
                f,
                "trailing partial IVF frame {frame_index} header at byte {offset}: needed {needed} more byte(s); treating EOF as end-of-stream"
            ),
        }
    }
}

/// Returns `true` when `input` starts with the IVF `DKIF` signature.
#[must_use]
pub fn is_ivf(input: &[u8]) -> bool {
    input.starts_with(&IVF_SIGNATURE)
}

/// The fixed-layout 32-byte IVF file header as it appears on the wire.
///
/// Private fixed-layout wire view (see [`docs/ZERO_COPY.md`](../../../docs/ZERO_COPY.md)):
/// [`parse_ivf_header`] borrows this from the input, then validates it into the
/// public [`IvfHeader`] domain type. The byteorder wrappers make every multi-byte
/// field little-endian and alignment-1, so the struct can be borrowed from an
/// unaligned `&[u8]`. This struct is never exposed in a public API. The byte
/// layout matches the original AV1/IVF `DKIF` header and is unchanged.
#[repr(C)]
#[derive(FromBytes, KnownLayout, Immutable, Unaligned)]
struct IvfFileHeaderWire {
    magic: [u8; 4],
    version: U16,
    header_len: U16,
    fourcc: [u8; 4],
    width: U16,
    height: U16,
    timebase_denominator: U32,
    timebase_numerator: U32,
    frame_count: U32,
    unused: U32,
}

/// Parses a complete IVF header.
///
/// Borrows the fixed-layout `IvfFileHeaderWire` from `input` and validates it
/// into the public [`IvfHeader`]; no bytes are copied until the validated fields
/// are read out.
///
/// # Errors
/// Returns [`IvfError`] for a truncated header, invalid signature, or header length
/// smaller than the 32-byte baseline. The parser never panics on malformed input.
pub fn parse_ivf_header(input: &[u8]) -> Result<IvfHeader, IvfError> {
    let (wire, _rest) =
        IvfFileHeaderWire::ref_from_prefix(input).map_err(|_| IvfError::TruncatedHeader {
            offset: ByteOffset::new(input.len() as u64),
            needed: IVF_HEADER_SIZE_BYTES.saturating_sub(input.len()),
        })?;

    if wire.magic != IVF_SIGNATURE {
        return Err(IvfError::InvalidSignature {
            offset: ByteOffset::new(0),
            signature: wire.magic,
        });
    }

    let header_len = wire.header_len.get();
    if header_len < IVF_HEADER_SIZE {
        return Err(IvfError::InvalidHeaderLength {
            offset: ByteOffset::new(6),
            header_len,
        });
    }
    if input.len() < usize::from(header_len) {
        return Err(IvfError::TruncatedHeader {
            offset: ByteOffset::new(input.len() as u64),
            needed: usize::from(header_len).saturating_sub(input.len()),
        });
    }

    Ok(IvfHeader {
        version: wire.version.get(),
        header_len,
        fourcc: wire.fourcc,
        width: wire.width.get(),
        height: wire.height.get(),
        timebase_denominator: wire.timebase_denominator.get(),
        timebase_numerator: wire.timebase_numerator.get(),
        frame_count: wire.frame_count.get(),
        unused: wire.unused.get(),
    })
}

/// Parses an IVF stream, retaining frames parsed before the first structural
/// container error.
#[must_use]
pub fn parse_ivf_partial(input: &[u8]) -> PartialIvfParse<'_> {
    let header = match parse_ivf_header(input) {
        Ok(header) => header,
        Err(error) => {
            return PartialIvfParse {
                header: None,
                frames: Vec::new(),
                warnings: Vec::new(),
                error: Some(error),
            };
        }
    };

    let mut frames = Vec::new();
    let mut warnings = Vec::new();
    let mut cursor = IvfFrameCursor::new(input, header);

    while cursor.has_remaining() {
        match cursor.next_frame_record() {
            Ok(IvfFrameRead::Frame(frame)) => frames.push(frame),
            Ok(IvfFrameRead::Warning(warning)) => {
                warnings.push(warning);
                break;
            }
            Ok(IvfFrameRead::End) => break,
            Err(error) => {
                return PartialIvfParse {
                    header: Some(header),
                    frames,
                    warnings,
                    error: Some(error),
                };
            }
        }
    }

    PartialIvfParse {
        header: Some(header),
        frames,
        warnings,
        error: None,
    }
}

/// Writes a baseline 32-byte IVF header.
///
/// # Errors
/// Returns any I/O error from `writer`, or [`io::ErrorKind::InvalidInput`] if the
/// supplied header length is smaller than 32 bytes.
pub fn write_ivf_header<W: io::Write + ?Sized>(
    writer: &mut W,
    header: &IvfHeader,
) -> io::Result<()> {
    if header.header_len < IVF_HEADER_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "IVF header length must be at least 32 bytes",
        ));
    }

    writer.write_all(&IVF_SIGNATURE)?;
    writer.write_all(&header.version.to_le_bytes())?;
    writer.write_all(&header.header_len.to_le_bytes())?;
    writer.write_all(&header.fourcc)?;
    writer.write_all(&header.width.to_le_bytes())?;
    writer.write_all(&header.height.to_le_bytes())?;
    writer.write_all(&header.timebase_denominator.to_le_bytes())?;
    writer.write_all(&header.timebase_numerator.to_le_bytes())?;
    writer.write_all(&header.frame_count.to_le_bytes())?;
    writer.write_all(&header.unused.to_le_bytes())?;

    let extra = usize::from(header.header_len.saturating_sub(IVF_HEADER_SIZE));
    if extra > 0 {
        writer.write_all(&vec![0; extra])?;
    }
    Ok(())
}

/// Writes one IVF frame record and payload.
///
/// # Errors
/// Returns any I/O error from `writer`, or [`io::ErrorKind::InvalidInput`] if the
/// payload is larger than the IVF 32-bit frame-size field can represent.
pub fn write_ivf_frame<W: io::Write + ?Sized>(
    writer: &mut W,
    pts: u64,
    payload: &[u8],
) -> io::Result<()> {
    let size = u32::try_from(payload.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "IVF frame payload length exceeds u32::MAX",
        )
    })?;
    writer.write_all(&size.to_le_bytes())?;
    writer.write_all(&pts.to_le_bytes())?;
    writer.write_all(payload)?;
    Ok(())
}

fn read_u32_le(input: &[u8], offset: usize) -> Option<u32> {
    let bytes = input.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64_le(input: &[u8], offset: usize) -> Option<u64> {
    let bytes = input.get(offset..offset.checked_add(8)?)?;
    Some(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn header_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        write_ivf_header(&mut bytes, &IvfHeader::new(*b"AV02", 1920, 1080, 24, 1, 1)).unwrap();
        bytes
    }

    #[test]
    fn parses_valid_header() {
        let header = parse_ivf_header(&header_bytes()).unwrap();
        assert_eq!(header.version, 0);
        assert_eq!(header.header_len, IVF_HEADER_SIZE);
        assert_eq!(header.fourcc, *b"AV02");
        assert_eq!(header.width, 1920);
        assert_eq!(header.height, 1080);
        assert_eq!(header.timebase_denominator, 24);
        assert_eq!(header.timebase_numerator, 1);
        assert_eq!(header.frame_count, 1);
        assert_eq!(header.payload_start_offset(), ByteOffset::new(32));
    }

    #[test]
    fn invalid_signature_is_error() {
        let mut bytes = header_bytes();
        bytes[0..4].copy_from_slice(b"FIKD");
        assert!(matches!(
            parse_ivf_header(&bytes),
            Err(IvfError::InvalidSignature { signature, .. }) if signature == *b"FIKD"
        ));
    }

    #[test]
    fn truncated_header_is_error() {
        assert!(matches!(
            parse_ivf_header(&header_bytes()[..20]),
            Err(IvfError::TruncatedHeader {
                offset,
                needed: 12
            }) if offset == ByteOffset::new(20)
        ));
    }

    #[test]
    fn short_header_length_is_error() {
        let mut bytes = header_bytes();
        bytes[6..8].copy_from_slice(&31u16.to_le_bytes());
        assert!(matches!(
            parse_ivf_header(&bytes),
            Err(IvfError::InvalidHeaderLength {
                offset,
                header_len: 31
            }) if offset == ByteOffset::new(6)
        ));
    }

    #[test]
    fn parses_frame_payload_and_offsets() {
        let mut bytes = header_bytes();
        write_ivf_frame(&mut bytes, 7, &[0x01, 0x08]).unwrap();
        let parsed = parse_ivf_partial(&bytes);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.warnings, Vec::new());
        assert_eq!(parsed.frames.len(), 1);
        let frame = parsed.frames[0];
        assert_eq!(frame.index, 0);
        assert_eq!(frame.header_offset, ByteOffset::new(32));
        assert_eq!(frame.payload_offset, ByteOffset::new(44));
        assert_eq!(frame.size, 2);
        assert_eq!(frame.pts, 7);
        assert_eq!(frame.payload, &[0x01, 0x08]);
    }

    #[test]
    fn trailing_partial_frame_header_after_prefix_is_warning() {
        let mut bytes = header_bytes();
        write_ivf_frame(&mut bytes, 0, &[0x01, 0x08]).unwrap();
        bytes.extend_from_slice(&[0x05, 0x00]);
        let parsed = parse_ivf_partial(&bytes);
        assert_eq!(parsed.frames.len(), 1);
        assert_eq!(parsed.error, None);
        assert_eq!(
            parsed.warnings,
            vec![IvfWarning::TrailingPartialFrameHeader {
                frame_index: 1,
                offset: ByteOffset::new(bytes.len() as u64),
                needed: 10,
            }]
        );
    }

    #[test]
    fn truncated_initial_frame_header_is_error() {
        let mut bytes = header_bytes();
        bytes.extend_from_slice(&[0x05, 0x00]);
        let parsed = parse_ivf_partial(&bytes);
        assert_eq!(parsed.frames.len(), 0);
        assert_eq!(parsed.warnings, Vec::new());
        assert!(matches!(
            parsed.error,
            Some(IvfError::TruncatedFrameHeader {
                frame_index: 0,
                offset,
                needed: 10
            }) if offset == ByteOffset::new(bytes.len() as u64)
        ));
    }

    #[test]
    fn frame_cursor_retry_preserves_truncated_initial_frame_header_error() {
        let mut bytes = header_bytes();
        bytes.extend_from_slice(&[0x05, 0x00]);
        let header = parse_ivf_header(&bytes).unwrap();
        let mut cursor = IvfFrameCursor::new(&bytes, header);

        let first = cursor.next_frame_record();
        let second = cursor.next_frame_record();

        assert_eq!(first, second);
        assert!(matches!(
            first,
            Err(IvfError::TruncatedFrameHeader {
                frame_index: 0,
                offset,
                needed: 10
            }) if offset == ByteOffset::new(bytes.len() as u64)
        ));
    }

    #[test]
    fn truncated_frame_payload_keeps_prefix() {
        let mut bytes = header_bytes();
        write_ivf_frame(&mut bytes, 0, &[0x01, 0x08]).unwrap();
        bytes.extend_from_slice(&5u32.to_le_bytes());
        bytes.extend_from_slice(&3u64.to_le_bytes());
        bytes.extend_from_slice(&[0x01, 0x08]);
        let parsed = parse_ivf_partial(&bytes);
        assert_eq!(parsed.frames.len(), 1);
        assert!(matches!(
            parsed.error,
            Some(IvfError::TruncatedFramePayload {
                frame_index: 1,
                offset,
                size: 5,
                remaining: 2
            }) if offset == ByteOffset::new(bytes.len() as u64)
        ));
    }

    #[test]
    fn writer_round_trips() {
        let mut bytes = Vec::new();
        let header = IvfHeader::new(*b"AV02", 16, 16, 30, 1, 1);
        write_ivf_header(&mut bytes, &header).unwrap();
        write_ivf_frame(&mut bytes, 9, &[0x01, 0x08]).unwrap();
        let parsed = parse_ivf_partial(&bytes);
        assert_eq!(parsed.header, Some(header));
        assert_eq!(parsed.frames[0].pts, 9);
        assert_eq!(parsed.frames[0].payload, &[0x01, 0x08]);
    }

    #[test]
    fn parsers_never_panic_on_short_inputs() {
        for len in 0..IVF_HEADER_SIZE {
            let bytes = vec![0; usize::from(len)];
            let _ = parse_ivf_header(&bytes);
            let _ = parse_ivf_partial(&bytes);
        }
    }

    #[test]
    fn wire_header_matches_baseline_layout() {
        // The zerocopy wire view must stay exactly the 32-byte baseline header so
        // `ref_from_prefix` borrows the right field offsets.
        assert_eq!(
            core::mem::size_of::<IvfFileHeaderWire>(),
            IVF_HEADER_SIZE_BYTES
        );
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// The IVF parser must never panic on arbitrary input bytes.
        #[test]
        fn ivf_parsers_never_panic(data in proptest::collection::vec(any::<u8>(), 0..2048)) {
            let _ = parse_ivf_header(&data);
            let _ = parse_ivf_partial(&data);
        }
    }
}
