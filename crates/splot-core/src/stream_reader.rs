// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Forward-only, `Read`-based temporal-unit demuxer (`INFRA-STREAMING-TU-READER`).
//!
//! [`TemporalUnitReader`] frames an AV2 bitstream into one temporal unit at a
//! time over a [`std::io::Read`] source, so a consumer (e.g. the streaming
//! validator) can bound peak input memory to a single unit instead of the whole
//! file. It is **forward-only** — it never seeks — so it works on pipes and
//! stdin, mirroring the dav2d Annex-B/Section-5 demuxers (which set `.seek =
//! NULL`) and AVM's `obudec_read_temporal_unit`.
//!
//! Two container formats are framed, reusing the existing slice parsers:
//!
//! - **Annex B** ([`crate::annexb`]): each yielded unit is one OBU's bytes
//!   (`[leb128 length ++ OBU]`), to be parsed at the carried absolute offset with
//!   [`crate::annexb::parse_annex_b_obus_partial_at`]. Peak memory ≈ one OBU.
//! - **IVF** ([`crate::ivf`]): each yielded unit is one frame payload, to be
//!   parsed at the carried payload offset. Peak memory ≈ one frame.
//!
//! The reader never copies the whole stream and never allocates a unit larger
//! than the configured per-unit cap (it returns [`ReaderError::UnitTooLarge`]
//! instead). It stays in safe Rust by the push-then-drop discipline: each unit
//! borrows the reused buffer and must be consumed before the next call.

use std::io::{self, Read};

use crate::ivf::{
    IVF_FRAME_HEADER_SIZE, IVF_HEADER_SIZE, IVF_SIGNATURE, IvfError, IvfHeader, IvfWarning,
    parse_ivf_header,
};
use crate::leb128::read_leb128;
use crate::span::ByteOffset;
use crate::stream::BitstreamFormat;

/// Default per-unit byte cap (256 MiB), matching the IVF tooling frame cap. A
/// declared unit larger than this is rejected rather than allocated.
pub const DEFAULT_MAX_UNIT_BYTES: usize = 256 * 1024 * 1024;

/// Maximum bytes appended to the reused buffer per read attempt. The buffer grows
/// only as bytes actually arrive, so a truncated unit that *declares* a large
/// (but not present) size never forces an allocation near the per-unit cap.
const READ_CHUNK_BYTES: usize = 64 * 1024;

const IVF_HEADER_SIZE_BYTES: usize = IVF_HEADER_SIZE as usize;

/// One framed unit borrowed from the reader's reused buffer.
///
/// The borrow is valid only until the next reader call (push-then-drop): parse or
/// copy what you need before requesting the next unit.
#[derive(Debug)]
pub enum StreamUnit<'a> {
    /// One Annex B OBU's bytes (`[leb128 length ++ OBU]`). Parse with
    /// [`crate::annexb::parse_annex_b_obus_partial_at`] at `offset` (the absolute
    /// offset of the length prefix) to reproduce the in-memory OBU envelope.
    AnnexBObu {
        /// Absolute offset of this OBU's length prefix in the stream.
        offset: ByteOffset,
        /// `[leb128 length ++ OBU]` bytes for exactly this OBU.
        bytes: &'a [u8],
    },
    /// One IVF frame payload. Parse with
    /// [`crate::annexb::parse_annex_b_obus_partial_at`] at `payload_offset`.
    IvfFrame {
        /// Absolute offset of the frame payload in the stream.
        payload_offset: ByteOffset,
        /// The frame payload bytes (Annex B OBUs).
        payload: &'a [u8],
    },
    /// A non-fatal IVF container warning, surfaced after the last complete frame.
    IvfWarning(IvfWarning),
}

/// An error that aborts streaming (as opposed to a malformed-but-reportable
/// bitstream, which is delivered in-band as unit bytes the consumer parses).
#[derive(Debug)]
pub enum ReaderError {
    /// A non-EOF I/O error from the underlying reader.
    Io(io::Error),
    /// A declared unit size exceeded the configured per-unit byte cap.
    UnitTooLarge {
        /// Absolute offset of the offending unit.
        offset: ByteOffset,
        /// Declared unit size in bytes.
        declared: u64,
        /// The configured cap.
        cap: usize,
    },
    /// A terminal IVF container structural error (header or frame framing).
    Ivf(IvfError),
}

impl std::fmt::Display for ReaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "input read error: {error}"),
            Self::UnitTooLarge {
                offset,
                declared,
                cap,
            } => write!(
                f,
                "temporal unit at byte {offset} declares {declared} byte(s), exceeding the {cap}-byte cap"
            ),
            Self::Ivf(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ReaderError {}

/// Forward-only, `Read`-based temporal-unit demuxer.
#[derive(Debug)]
pub struct TemporalUnitReader<R: Read> {
    inner: R,
    /// Reused unit buffer; holds one OBU/frame at a time.
    buf: Vec<u8>,
    /// Bytes read ahead (format probe / not yet consumed by unit logic). Drained
    /// before reading from `inner`.
    pending: Vec<u8>,
    pending_pos: usize,
    /// Absolute offset of the next byte to be consumed by unit logic.
    pos: u64,
    /// Per-unit byte cap.
    cap: usize,
    /// Detected format (resolved lazily on first read).
    format: Option<BitstreamFormat>,
    /// Parsed IVF header (once the IVF header has been consumed).
    ivf_header: Option<IvfHeader>,
    /// Next IVF frame index to assign.
    frame_index: usize,
    /// Once set, all further reads yield `Ok(None)`.
    done: bool,
}

impl<R: Read> TemporalUnitReader<R> {
    /// Creates a reader over `inner` with the [`DEFAULT_MAX_UNIT_BYTES`] cap.
    pub fn new(inner: R) -> Self {
        Self::with_max_unit_bytes(inner, DEFAULT_MAX_UNIT_BYTES)
    }

    /// Creates a reader over `inner` with an explicit per-unit byte cap.
    pub fn with_max_unit_bytes(inner: R, cap: usize) -> Self {
        Self {
            inner,
            buf: Vec::new(),
            pending: Vec::new(),
            pending_pos: 0,
            pos: 0,
            cap,
            format: None,
            ivf_header: None,
            frame_index: 0,
            done: false,
        }
    }

    /// Test-only: the reused buffer's current capacity, used to assert that peak
    /// input memory stays bounded by the largest unit, not the stream size.
    #[cfg(test)]
    fn buf_capacity(&self) -> usize {
        self.buf.capacity()
    }

    /// Reads the next framed unit, or `Ok(None)` at clean end of stream.
    ///
    /// # Errors
    /// Returns [`ReaderError`] for a non-EOF I/O error, an over-cap unit, or a
    /// terminal IVF container error. A malformed-but-reportable Annex B tail is
    /// **not** an error here: it is returned as unit bytes whose parse produces
    /// the diagnostic.
    pub fn next_unit(&mut self) -> Result<Option<StreamUnit<'_>>, ReaderError> {
        if self.done {
            return Ok(None);
        }
        match self.detect_format()? {
            BitstreamFormat::AnnexB => self.next_annexb_unit(),
            BitstreamFormat::Ivf => self.next_ivf_unit(),
        }
    }

    /// Resolves the container format on first use by probing up to 4 bytes.
    ///
    /// Matches [`crate::ivf::is_ivf`]: exactly `DKIF` ⇒ IVF; anything else
    /// (including fewer than 4 bytes) ⇒ Annex B. The probed bytes are buffered and
    /// later consumed as unit bytes, so nothing is lost.
    fn detect_format(&mut self) -> Result<BitstreamFormat, ReaderError> {
        if let Some(format) = self.format {
            return Ok(format);
        }
        let mut probe = Vec::with_capacity(IVF_SIGNATURE.len());
        let mut one = [0u8; 1];
        while probe.len() < IVF_SIGNATURE.len() {
            match self.inner.read(&mut one) {
                Ok(0) => break,
                Ok(_) => probe.push(one[0]),
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(ReaderError::Io(e)),
            }
        }
        let format = if probe.as_slice() == IVF_SIGNATURE {
            BitstreamFormat::Ivf
        } else {
            BitstreamFormat::AnnexB
        };
        self.pending = probe;
        self.pending_pos = 0;
        self.format = Some(format);
        Ok(format)
    }

    /// Reads one byte, draining `pending` first. `Ok(None)` at EOF.
    fn read_one(&mut self) -> Result<Option<u8>, ReaderError> {
        if self.pending_pos < self.pending.len() {
            let byte = self.pending[self.pending_pos];
            self.pending_pos += 1;
            self.pos += 1;
            return Ok(Some(byte));
        }
        let mut one = [0u8; 1];
        loop {
            match self.inner.read(&mut one) {
                Ok(0) => return Ok(None),
                Ok(_) => {
                    self.pos += 1;
                    return Ok(Some(one[0]));
                }
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(ReaderError::Io(e)),
            }
        }
    }

    /// Appends up to `want` bytes to `self.buf`, draining `pending` first. Returns
    /// the count read (`< want` only at EOF) and advances `pos` by that count.
    ///
    /// The buffer grows in bounded [`READ_CHUNK_BYTES`] increments as bytes
    /// arrive, so a truncated unit declaring a large size never eagerly allocates
    /// it; the per-unit cap remains the hard ceiling for units truly that large.
    fn read_into_buf(&mut self, want: usize) -> Result<usize, ReaderError> {
        let mut got = 0;
        let avail = self.pending.len() - self.pending_pos;
        let take = avail.min(want);
        if take > 0 {
            self.buf
                .extend_from_slice(&self.pending[self.pending_pos..self.pending_pos + take]);
            self.pending_pos += take;
            self.pos += take as u64;
            got += take;
        }
        while got < want {
            let need = (want - got).min(READ_CHUNK_BYTES);
            let start = self.buf.len();
            self.buf.resize(start + need, 0);
            match self.inner.read(&mut self.buf[start..]) {
                Ok(0) => {
                    self.buf.truncate(start);
                    break;
                }
                Ok(n) => {
                    self.buf.truncate(start + n);
                    self.pos += n as u64;
                    got += n;
                }
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {
                    self.buf.truncate(start);
                }
                Err(e) => {
                    self.buf.truncate(start);
                    return Err(ReaderError::Io(e));
                }
            }
        }
        Ok(got)
    }

    /// Frames the next Annex B OBU as `[leb128 length ++ OBU]`.
    fn next_annexb_unit(&mut self) -> Result<Option<StreamUnit<'_>>, ReaderError> {
        self.buf.clear();
        let unit_offset = self.pos;
        // Read the leb128 length prefix one byte at a time (at most 8; longer is
        // rejected by read_leb128 when the consumer reparses).
        loop {
            match self.read_one()? {
                None => {
                    if self.buf.is_empty() {
                        self.done = true;
                        return Ok(None);
                    }
                    // Truncated leb128 at EOF: yield the partial prefix; the
                    // consumer's parse reproduces UnexpectedEof / InvalidLeb128.
                    self.done = true;
                    return Ok(Some(StreamUnit::AnnexBObu {
                        offset: ByteOffset::new(unit_offset),
                        bytes: &self.buf,
                    }));
                }
                Some(byte) => {
                    self.buf.push(byte);
                    if byte & 0x80 == 0 || self.buf.len() == 8 {
                        break;
                    }
                }
            }
        }
        let Ok(leb) = read_leb128(&self.buf, ByteOffset::new(0)) else {
            // Invalid leb128 (e.g. 8-byte continuation): yield the prefix; the
            // consumer's parse reproduces the InvalidLeb128 error.
            self.done = true;
            return Ok(Some(StreamUnit::AnnexBObu {
                offset: ByteOffset::new(unit_offset),
                bytes: &self.buf,
            }));
        };
        let size = u64::from(leb.value);
        if size > self.cap as u64 {
            self.done = true;
            return Err(ReaderError::UnitTooLarge {
                offset: ByteOffset::new(unit_offset),
                declared: size,
                cap: self.cap,
            });
        }
        let want = size as usize;
        let got = self.read_into_buf(want)?;
        if got < want {
            // Truncated OBU payload: the consumer's parse reports
            // ObuPayloadOutOfRange with remaining == got.
            self.done = true;
        }
        Ok(Some(StreamUnit::AnnexBObu {
            offset: ByteOffset::new(unit_offset),
            bytes: &self.buf,
        }))
    }

    /// Frames the next IVF frame payload (parsing the file header on first use).
    fn next_ivf_unit(&mut self) -> Result<Option<StreamUnit<'_>>, ReaderError> {
        if self.ivf_header.is_none() {
            self.buf.clear();
            let got = self.read_into_buf(IVF_HEADER_SIZE_BYTES)?;
            if got == IVF_HEADER_SIZE_BYTES {
                let header_len = usize::from(u16::from_le_bytes([self.buf[6], self.buf[7]]));
                if header_len > IVF_HEADER_SIZE_BYTES {
                    let _ = self.read_into_buf(header_len - IVF_HEADER_SIZE_BYTES)?;
                }
            }
            match parse_ivf_header(&self.buf) {
                Ok(header) => self.ivf_header = Some(header),
                Err(error) => {
                    self.done = true;
                    return Err(ReaderError::Ivf(error));
                }
            }
        }

        self.buf.clear();
        let got = self.read_into_buf(IVF_FRAME_HEADER_SIZE)?;
        if got < IVF_FRAME_HEADER_SIZE {
            self.done = true;
            if got == 0 {
                // cursor == input length: a clean end, not a truncation.
                return Ok(None);
            }
            let needed = IVF_FRAME_HEADER_SIZE - got;
            let offset = ByteOffset::new(self.pos);
            if self.frame_index > 0 {
                return Ok(Some(StreamUnit::IvfWarning(
                    IvfWarning::TrailingPartialFrameHeader {
                        frame_index: self.frame_index,
                        offset,
                        needed,
                    },
                )));
            }
            return Err(ReaderError::Ivf(IvfError::TruncatedFrameHeader {
                frame_index: self.frame_index,
                offset,
                needed,
            }));
        }
        let size = u32::from_le_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]);
        let payload_offset = self.pos; // == frame header offset + 12
        if u64::from(size) > self.cap as u64 {
            self.done = true;
            return Err(ReaderError::UnitTooLarge {
                offset: ByteOffset::new(payload_offset),
                declared: u64::from(size),
                cap: self.cap,
            });
        }
        self.buf.clear();
        let want = size as usize;
        let got = self.read_into_buf(want)?;
        if got < want {
            self.done = true;
            return Err(ReaderError::Ivf(IvfError::TruncatedFramePayload {
                frame_index: self.frame_index,
                offset: ByteOffset::new(self.pos),
                size,
                remaining: got,
            }));
        }
        self.frame_index += 1;
        Ok(Some(StreamUnit::IvfFrame {
            payload_offset: ByteOffset::new(payload_offset),
            payload: &self.buf,
        }))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::ivf::write_ivf_header;
    use crate::test_support::ivf_with_frame;

    /// A `Read` that hands out at most one byte per call, to stress cross-read
    /// reassembly.
    struct OneByteAtATime<'a> {
        data: &'a [u8],
        pos: usize,
    }

    impl Read for OneByteAtATime<'_> {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if out.is_empty() || self.pos >= self.data.len() {
                return Ok(0);
            }
            out[0] = self.data[self.pos];
            self.pos += 1;
            Ok(1)
        }
    }

    fn collect_annexb_units(data: &[u8]) -> Vec<(u64, Vec<u8>)> {
        let mut reader = TemporalUnitReader::new(data);
        let mut out = Vec::new();
        // Raw Annex B yields only `AnnexBObu` units until the clean end.
        while let Some(StreamUnit::AnnexBObu { offset, bytes }) = reader.next_unit().unwrap() {
            out.push((offset.get(), bytes.to_vec()));
        }
        out
    }

    #[test]
    fn annexb_yields_each_obu_with_base_offset() {
        // TD (size 1) then SequenceHeader (size 2: header 0x04 + payload 0xAB).
        let data = [0x01, 0x08, 0x02, 0x04, 0xAB];
        let units = collect_annexb_units(&data);
        assert_eq!(
            units,
            vec![(0, vec![0x01, 0x08]), (2, vec![0x02, 0x04, 0xAB])]
        );
    }

    #[test]
    fn annexb_clean_eof_yields_none() {
        let mut reader = TemporalUnitReader::new(&[][..]);
        assert!(reader.next_unit().unwrap().is_none());
    }

    #[test]
    fn reused_buffer_stays_bounded_by_largest_unit() {
        // 1000 one-byte OBUs: the stream dwarfs any single unit, but the reader
        // reuses one buffer, so its capacity tracks the largest unit (a couple of
        // bytes), not the stream length.
        let data: Vec<u8> = [0x01u8, 0x08].repeat(1000);
        let mut reader = TemporalUnitReader::new(&data[..]);
        let mut count = 0;
        while reader.next_unit().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 1000);
        assert!(
            reader.buf_capacity() < 64,
            "reused buffer grew with the stream: capacity {} vs stream {}",
            reader.buf_capacity(),
            data.len()
        );
    }

    #[test]
    fn declared_large_but_truncated_unit_does_not_eagerly_allocate() {
        // An Annex-B OBU declaring ~200 MiB (under the 256 MiB cap) with only a few
        // bytes present must not balloon the reused buffer toward the declared size
        // before EOF: growth is bounded by READ_CHUNK_BYTES, not the declared size.
        let mut data = vec![0x80, 0x80, 0x80, 0x64]; // leb128(200 MiB)
        data.extend_from_slice(&[0x08; 4]);
        let mut reader = TemporalUnitReader::new(&data[..]);
        let _ = reader.next_unit().unwrap(); // yields the partial tail
        assert!(
            reader.buf_capacity() < (1 << 20),
            "reused buffer eagerly grew toward the declared size: capacity {}",
            reader.buf_capacity()
        );
    }

    #[test]
    fn ivf_declared_large_but_truncated_frame_does_not_eagerly_allocate() {
        // The IVF call site shares `read_into_buf`: a frame header declaring
        // ~200 MiB (under the cap) with no payload present must not eagerly
        // allocate the declared size before hitting EOF.
        let mut data = Vec::new();
        write_ivf_header(&mut data, &IvfHeader::new(*b"AV02", 16, 16, 24, 1, 1)).unwrap();
        data.extend_from_slice(&(200u32 * 1024 * 1024).to_le_bytes()); // frame size ~200 MiB
        data.extend_from_slice(&0u64.to_le_bytes()); // pts; no payload follows
        let mut reader = TemporalUnitReader::new(&data[..]);
        let err = reader.next_unit().unwrap_err();
        assert!(matches!(
            err,
            ReaderError::Ivf(IvfError::TruncatedFramePayload { .. })
        ));
        assert!(
            reader.buf_capacity() < (1 << 20),
            "reused buffer eagerly grew toward the declared frame size: capacity {}",
            reader.buf_capacity()
        );
    }

    #[test]
    fn annexb_one_byte_at_a_time_matches_whole_buffer() {
        let data = [0x01, 0x08, 0x02, 0x04, 0xAB];
        let whole = collect_annexb_units(&data);

        let mut reader = TemporalUnitReader::new(OneByteAtATime {
            data: &data,
            pos: 0,
        });
        let mut chunked = Vec::new();
        while let Some(StreamUnit::AnnexBObu { offset, bytes }) = reader.next_unit().unwrap() {
            chunked.push((offset.get(), bytes.to_vec()));
        }
        assert_eq!(whole, chunked);
    }

    #[test]
    fn annexb_truncated_payload_yields_partial_tail() {
        // size=5 declared, only one payload byte present.
        let data = [0x05, 0x08];
        let units = collect_annexb_units(&data);
        // The reader yields the partial tail; the consumer's parse reports the error.
        assert_eq!(units, vec![(0, vec![0x05, 0x08])]);
    }

    #[test]
    fn ivf_yields_frame_payload_with_offset() {
        let data = ivf_with_frame(&[0x01, 0x08]);
        let mut reader = TemporalUnitReader::new(&data[..]);
        assert!(matches!(
            reader.next_unit().unwrap(),
            Some(StreamUnit::IvfFrame { payload_offset, payload })
                if payload_offset == ByteOffset::new(44) && payload == [0x01u8, 0x08].as_slice()
        ));
        assert!(reader.next_unit().unwrap().is_none());
    }

    #[test]
    fn ivf_one_byte_at_a_time_reassembles_frame() {
        // The 32-byte file header, the 12-byte frame header, and the payload all
        // arrive one byte per `read`; the frame must still reassemble.
        let data = ivf_with_frame(&[0x01, 0x08, 0x02, 0x04, 0xAB]);
        let mut reader = TemporalUnitReader::new(OneByteAtATime {
            data: &data,
            pos: 0,
        });
        assert!(matches!(
            reader.next_unit().unwrap(),
            Some(StreamUnit::IvfFrame { payload, .. })
                if payload == [0x01u8, 0x08, 0x02, 0x04, 0xAB].as_slice()
        ));
        assert!(reader.next_unit().unwrap().is_none());
    }

    #[test]
    fn ivf_truncated_initial_frame_header_is_error() {
        let mut data = Vec::new();
        write_ivf_header(&mut data, &IvfHeader::new(*b"AV02", 16, 16, 24, 1, 1)).unwrap();
        data.extend_from_slice(&[0x05, 0x00]);
        let mut reader = TemporalUnitReader::new(&data[..]);
        let err = reader.next_unit().unwrap_err();
        assert!(matches!(
            err,
            ReaderError::Ivf(IvfError::TruncatedFrameHeader {
                frame_index: 0,
                offset,
                needed: 10,
            }) if offset == ByteOffset::new(data.len() as u64)
        ));
    }

    #[test]
    fn ivf_trailing_partial_header_after_frame_is_warning() {
        let mut data = ivf_with_frame(&[0x01, 0x08]);
        data.extend_from_slice(&[0x05, 0x00]);
        let mut reader = TemporalUnitReader::new(&data[..]);
        assert!(matches!(
            reader.next_unit().unwrap(),
            Some(StreamUnit::IvfFrame { .. })
        ));
        let warning = reader.next_unit().unwrap().expect("a warning");
        assert!(matches!(
            warning,
            StreamUnit::IvfWarning(IvfWarning::TrailingPartialFrameHeader {
                frame_index: 1,
                offset,
                needed: 10,
            }) if offset == ByteOffset::new(data.len() as u64)
        ));
        assert!(reader.next_unit().unwrap().is_none());
    }

    #[test]
    fn annexb_unit_over_cap_is_error() {
        // OBU declares size 1000 with a small cap.
        let mut data = Vec::new();
        // leb128(1000) = 0xE8 0x07
        data.extend_from_slice(&[0xE8, 0x07]);
        data.extend_from_slice(&[0u8; 8]);
        let mut reader = TemporalUnitReader::with_max_unit_bytes(&data[..], 16);
        let err = reader.next_unit().unwrap_err();
        assert!(matches!(
            err,
            ReaderError::UnitTooLarge {
                declared: 1000,
                cap: 16,
                ..
            }
        ));
    }

    #[test]
    fn ivf_truncated_frame_payload_is_error() {
        // One good frame, then a frame header declaring 5 bytes with only 2 present.
        let mut data = ivf_with_frame(&[0x01, 0x08]);
        data.extend_from_slice(&5u32.to_le_bytes());
        data.extend_from_slice(&3u64.to_le_bytes());
        data.extend_from_slice(&[0x01, 0x08]);
        let mut reader = TemporalUnitReader::new(&data[..]);
        assert!(matches!(
            reader.next_unit().unwrap(),
            Some(StreamUnit::IvfFrame { .. })
        ));
        let err = reader.next_unit().unwrap_err();
        assert!(matches!(
            err,
            ReaderError::Ivf(IvfError::TruncatedFramePayload {
                frame_index: 1,
                offset,
                size: 5,
                remaining: 2,
            }) if offset == ByteOffset::new(data.len() as u64)
        ));
    }

    #[test]
    fn ivf_yields_each_obu_of_a_multi_obu_frame_as_one_payload() {
        // One frame whose payload holds two OBUs; the reader yields the whole
        // payload, leaving per-OBU parsing to the consumer.
        let data = ivf_with_frame(&[0x01, 0x08, 0x02, 0x04, 0xAB]);
        let mut reader = TemporalUnitReader::new(&data[..]);
        assert!(matches!(
            reader.next_unit().unwrap(),
            Some(StreamUnit::IvfFrame { payload, .. })
                if payload == [0x01u8, 0x08, 0x02, 0x04, 0xAB].as_slice()
        ));
        assert!(reader.next_unit().unwrap().is_none());
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// Drains a reader to exhaustion (bounded), asserting only that it never
    /// panics and always terminates.
    fn drain(data: &[u8]) {
        let mut reader = TemporalUnitReader::new(data);
        for _ in 0..100_000 {
            match reader.next_unit() {
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => break,
            }
        }
    }

    proptest! {
        /// The reader must never panic on arbitrary input.
        #[test]
        fn reader_never_panics(data in proptest::collection::vec(any::<u8>(), 0..2048)) {
            drain(&data);
        }

        /// The IVF branch must never panic on arbitrary DKIF-prefixed input.
        #[test]
        fn ivf_reader_never_panics(tail in proptest::collection::vec(any::<u8>(), 0..2048)) {
            let mut data = Vec::with_capacity(IVF_SIGNATURE.len() + tail.len());
            data.extend_from_slice(&IVF_SIGNATURE);
            data.extend_from_slice(&tail);
            drain(&data);
        }
    }
}
