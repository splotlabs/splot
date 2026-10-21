// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Streaming YUV4MPEG2 (Y4M) reader. The container is not AV2 syntax.

use std::io::{self, Read};

use splot_recon::PixelFormat;

/// Longest accepted stream or frame header line, in bytes.
const MAX_HEADER_LEN: usize = 4096;

/// Errors of the Y4M reader.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Y4mError {
    /// Reading the input failed.
    #[error("Y4M read failed: {0}")]
    Io(#[from] io::Error),
    /// The stream does not start with `YUV4MPEG2 `.
    #[error("input is not a YUV4MPEG2 stream")]
    BadSignature,
    /// A header line has no newline within the length limit.
    #[error("Y4M header line is longer than {MAX_HEADER_LEN} bytes or unterminated")]
    HeaderTooLong,
    /// A required stream tag is absent.
    #[error("Y4M stream header has no {0} tag")]
    MissingTag(char),
    /// A tag value cannot be parsed.
    #[error("Y4M tag {tag} has invalid value {value:?}")]
    InvalidTag {
        /// Tag letter.
        tag: char,
        /// Raw value.
        value: String,
    },
    /// The colourspace is not supported.
    #[error("Y4M colourspace {0:?} is not supported")]
    UnsupportedColorspace(String),
    /// A frame does not start with `FRAME`.
    #[error("Y4M frame {0} does not start with FRAME")]
    BadFrameHeader(u64),
    /// The input ended inside a frame.
    #[error("Y4M frame {frame} is truncated: expected {expected} bytes, got {actual}")]
    Truncated {
        /// Zero-based frame number.
        frame: u64,
        /// Bytes expected.
        expected: usize,
        /// Bytes read.
        actual: usize,
    },
}

/// Parsed Y4M stream header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Y4mHeader {
    /// Luma width.
    pub width: u32,
    /// Luma height.
    pub height: u32,
    /// Frame rate as `(numerator, denominator)`.
    pub frame_rate: (u32, u32),
    /// Pixel aspect ratio as `(numerator, denominator)`; `(0, 0)` is unknown.
    pub aspect: (u32, u32),
    /// Interlacing tag: `p`, `t`, `b`, `m` or `?`.
    pub interlace: char,
    /// Chroma layout.
    pub format: PixelFormat,
    /// Bits per sample, `8..=16`. Samples above 8 bits are 2-byte little endian.
    pub bit_depth: u8,
}

impl Y4mHeader {
    /// Y, U and V plane sizes as `(width, height)`; chroma is `(0, 0)` for monochrome.
    #[must_use]
    pub fn plane_sizes(&self) -> [(usize, usize); 3] {
        let luma = (self.width as usize, self.height as usize);
        if self.format.is_monochrome() {
            return [luma, (0, 0), (0, 0)];
        }
        let sub_x = usize::from(self.format.subsampling_x());
        let sub_y = usize::from(self.format.subsampling_y());
        let chroma = ((luma.0 + sub_x) >> sub_x, (luma.1 + sub_y) >> sub_y);
        [luma, chroma, chroma]
    }

    fn bytes_per_sample(&self) -> usize {
        if self.bit_depth > 8 { 2 } else { 1 }
    }
}

/// Reads Y4M frames from any [`Read`]. Wrap unbuffered sources in a
/// [`std::io::BufReader`]; headers are read one byte at a time.
#[derive(Debug)]
pub struct Y4mReader<R> {
    reader: R,
    header: Y4mHeader,
    frames_read: u64,
    buffer: Vec<u8>,
}

impl<R: Read> Y4mReader<R> {
    /// Reads and parses the stream header.
    ///
    /// # Errors
    /// Returns a [`Y4mError`] for I/O failures and malformed or unsupported headers.
    pub fn new(mut reader: R) -> Result<Self, Y4mError> {
        let line = read_line(&mut reader)?.ok_or(Y4mError::BadSignature)?;
        let header = parse_stream_header(&line)?;
        Ok(Self {
            reader,
            header,
            frames_read: 0,
            buffer: Vec::new(),
        })
    }

    /// The stream header.
    #[must_use]
    pub fn header(&self) -> &Y4mHeader {
        &self.header
    }

    /// Reads the next frame as Y, U and V planes of `u16` samples, packed with
    /// stride equal to width. Chroma planes are empty for monochrome. Returns
    /// `None` at the end of the stream.
    ///
    /// # Errors
    /// Returns a [`Y4mError`] for I/O failures, a malformed frame header, or a
    /// truncated frame.
    pub fn read_frame(&mut self) -> Result<Option<[Vec<u16>; 3]>, Y4mError> {
        let Some(line) = read_line(&mut self.reader)? else {
            return Ok(None);
        };
        if line != b"FRAME" && !line.starts_with(b"FRAME ") {
            return Err(Y4mError::BadFrameHeader(self.frames_read));
        }
        let bytes_per_sample = self.header.bytes_per_sample();
        let sizes = self.header.plane_sizes();
        let expected = sizes.iter().map(|(w, h)| w * h).sum::<usize>() * bytes_per_sample;
        self.buffer.clear();
        // Grows with the data actually read, so a lying header cannot force a huge allocation.
        (&mut self.reader)
            .take(expected as u64)
            .read_to_end(&mut self.buffer)?;
        if self.buffer.len() != expected {
            return Err(Y4mError::Truncated {
                frame: self.frames_read,
                expected,
                actual: self.buffer.len(),
            });
        }
        self.frames_read += 1;

        let mut rest = self.buffer.as_slice();
        Ok(Some(sizes.map(|(width, height)| {
            let (bytes, tail) = rest.split_at(width * height * bytes_per_sample);
            rest = tail;
            if bytes_per_sample == 2 {
                bytes
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect()
            } else {
                bytes.iter().map(|&sample| u16::from(sample)).collect()
            }
        })))
    }
}

/// Reads one `\n`-terminated line without the newline. Returns `None` at a
/// clean end of input.
fn read_line(reader: &mut impl Read) -> Result<Option<Vec<u8>>, Y4mError> {
    let mut line = Vec::new();
    let mut byte = [0_u8];
    loop {
        if reader.read(&mut byte)? == 0 {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(Y4mError::HeaderTooLong)
            };
        }
        if byte[0] == b'\n' {
            return Ok(Some(line));
        }
        if line.len() == MAX_HEADER_LEN {
            return Err(Y4mError::HeaderTooLong);
        }
        line.push(byte[0]);
    }
}

fn parse_stream_header(line: &[u8]) -> Result<Y4mHeader, Y4mError> {
    let line = std::str::from_utf8(line).map_err(|_| Y4mError::BadSignature)?;
    let tags = line
        .strip_prefix("YUV4MPEG2 ")
        .ok_or(Y4mError::BadSignature)?;
    let (mut width, mut height, mut frame_rate) = (None, None, None);
    let mut aspect = (0, 0);
    let mut interlace = 'p';
    let (mut colorspace, mut extended) = (None, None);
    for token in tags.split(' ').filter(|token| !token.is_empty()) {
        let mut chars = token.chars();
        let tag = chars.next().unwrap_or(' ');
        let value = chars.as_str();
        let invalid = || Y4mError::InvalidTag {
            tag,
            value: value.to_owned(),
        };
        match tag {
            'W' => width = Some(parse_dimension(value).ok_or_else(invalid)?),
            'H' => height = Some(parse_dimension(value).ok_or_else(invalid)?),
            'F' => {
                let rate = parse_ratio(value).filter(|rate| rate.0 != 0 && rate.1 != 0);
                frame_rate = Some(rate.ok_or_else(invalid)?);
            }
            'A' => aspect = parse_ratio(value).ok_or_else(invalid)?,
            'I' if matches!(value, "p" | "?") => {
                interlace = value.chars().next().unwrap_or('p');
            }
            'I' => return Err(invalid()),
            'C' => colorspace = Some(value),
            'X' => {
                if let Some(subsampling) = value.strip_prefix("YSCSS=") {
                    extended = Some(subsampling);
                }
            }
            _ => {}
        }
    }
    let width = width.ok_or(Y4mError::MissingTag('W'))?;
    let height = height.ok_or(Y4mError::MissingTag('H'))?;
    let frame_rate = frame_rate.ok_or(Y4mError::MissingTag('F'))?;
    let colorspace = colorspace.or(extended).unwrap_or("420jpeg");
    let (format, bit_depth) = parse_colorspace(colorspace)
        .ok_or_else(|| Y4mError::UnsupportedColorspace(colorspace.to_owned()))?;
    Ok(Y4mHeader {
        width,
        height,
        frame_rate,
        aspect,
        interlace,
        format,
        bit_depth,
    })
}

fn parse_dimension(value: &str) -> Option<u32> {
    value
        .parse()
        .ok()
        .filter(|&value| (1..=crate::config::MAX_FRAME_DIMENSION).contains(&value))
}

fn parse_ratio(value: &str) -> Option<(u32, u32)> {
    let (numerator, denominator) = value.split_once(':')?;
    Some((numerator.parse().ok()?, denominator.parse().ok()?))
}

/// Maps a `C` (or `XYSCSS=`) colourspace value to a layout and bit depth.
fn parse_colorspace(value: &str) -> Option<(PixelFormat, u8)> {
    let value = value.to_ascii_lowercase();
    let (format, depth) = if let Some(depth) = value.strip_prefix("mono") {
        (PixelFormat::Monochrome, depth)
    } else {
        let (layout, depth) = value.split_at_checked(3)?;
        let format = match layout {
            "420" => PixelFormat::Yuv420,
            "422" => PixelFormat::Yuv422,
            "444" => PixelFormat::Yuv444,
            _ => return None,
        };
        (format, depth)
    };
    let bit_depth = match (format, depth) {
        (_, "") | (PixelFormat::Yuv420, "jpeg" | "mpeg2" | "paldv") => 8,
        (PixelFormat::Monochrome, _) => depth.parse().ok()?,
        _ => depth.strip_prefix('p')?.parse().ok()?,
    };
    (8..=16).contains(&bit_depth).then_some((format, bit_depth))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(input: &[u8]) -> Result<Y4mReader<&[u8]>, Y4mError> {
        Y4mReader::new(input)
    }

    #[test]
    fn parses_tags_and_8_bit_frames() -> Result<(), Y4mError> {
        let mut input = b"YUV4MPEG2 W3 H2 F25:1 Ip A1:1 C420mpeg2 XCOLORRANGE=FULL\n".to_vec();
        input.extend_from_slice(b"FRAME\n");
        input.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        input.extend_from_slice(b"FRAME Ixyz\n");
        input.extend_from_slice(&[0; 10]);
        let mut reader = read(&input)?;
        let header = *reader.header();
        assert_eq!((header.width, header.height), (3, 2));
        assert_eq!((header.frame_rate, header.aspect), ((25, 1), (1, 1)));
        assert_eq!((header.format, header.bit_depth), (PixelFormat::Yuv420, 8));
        let frame = reader.read_frame()?;
        assert_eq!(
            frame,
            Some([vec![1, 2, 3, 4, 5, 6], vec![7, 8], vec![9, 10]])
        );
        assert!(reader.read_frame()?.is_some());
        assert_eq!(reader.read_frame()?, None);
        Ok(())
    }

    #[test]
    fn parses_high_bit_depth_little_endian() -> Result<(), Y4mError> {
        let mut input = b"YUV4MPEG2 W1 H1 F30000:1001 XYSCSS=444P10\nFRAME\n".to_vec();
        input.extend_from_slice(&[0xff, 0x03, 0x00, 0x02, 0x01, 0x00]);
        let mut reader = read(&input)?;
        assert_eq!(reader.header().format, PixelFormat::Yuv444);
        assert_eq!(reader.header().bit_depth, 10);
        assert_eq!(reader.read_frame()?, Some([vec![1023], vec![512], vec![1]]));
        Ok(())
    }

    #[test]
    fn maps_every_colourspace_tag() {
        let cases = [
            ("420jpeg", PixelFormat::Yuv420, 8),
            ("420paldv", PixelFormat::Yuv420, 8),
            ("420", PixelFormat::Yuv420, 8),
            ("420p10", PixelFormat::Yuv420, 10),
            ("422", PixelFormat::Yuv422, 8),
            ("444p16", PixelFormat::Yuv444, 16),
            ("mono", PixelFormat::Monochrome, 8),
            ("mono12", PixelFormat::Monochrome, 12),
        ];
        for (tag, format, depth) in cases {
            assert_eq!(parse_colorspace(tag), Some((format, depth)), "{tag}");
        }
        for tag in ["411", "444alpha", "420p7", "420p17", "422jpeg", ""] {
            assert_eq!(parse_colorspace(tag), None, "{tag}");
        }
    }

    #[test]
    fn rejects_malformed_input() {
        let cases: [(&[u8], &str); 7] = [
            (b"YUV4MPEG W2 H2 F1:1\n", "not a YUV4MPEG2"),
            (b"YUV4MPEG2 H2 F1:1\n", "no W tag"),
            (b"YUV4MPEG2 W0 H2 F1:1\n", "tag W"),
            (b"YUV4MPEG2 W2 H2 F0:1\n", "tag F"),
            (b"YUV4MPEG2 W2 H2 F1:1 Ix\n", "tag I"),
            (b"YUV4MPEG2 W2 H2 F1:1 C411\n", "colourspace \"411\""),
            (b"YUV4MPEG2 W2 H2 F1:1", "unterminated"),
        ];
        for (input, expected) in cases {
            let message = read(input).err().map(|error| error.to_string());
            assert!(
                message.as_deref().is_some_and(|m| m.contains(expected)),
                "{expected}: {message:?}"
            );
        }
    }

    #[test]
    fn rejects_bad_and_truncated_frames() -> Result<(), Y4mError> {
        let mut reader = read(b"YUV4MPEG2 W2 H2 F1:1\nFRAMX\n")?;
        assert!(matches!(
            reader.read_frame(),
            Err(Y4mError::BadFrameHeader(0))
        ));
        let mut reader = read(b"YUV4MPEG2 W2 H2 F1:1\nFRAME\n\x01\x02")?;
        assert!(matches!(
            reader.read_frame(),
            Err(Y4mError::Truncated {
                frame: 0,
                expected: 6,
                actual: 2
            })
        ));
        Ok(())
    }

    /// Reads the AVM sample clips when they exist locally; CI skips them.
    #[test]
    fn reads_local_park_joy_clips() -> Result<(), Y4mError> {
        let Some(home) = std::env::var_os("HOME") else {
            return Ok(());
        };
        let dir = std::path::Path::new(&home).join("Devel/avm/build");
        for (name, depth) in [
            ("park_joy_90p_8_420.y4m", 8),
            ("park_joy_90p_10_420.y4m", 10),
        ] {
            let Ok(file) = std::fs::File::open(dir.join(name)) else {
                continue;
            };
            let mut reader = Y4mReader::new(io::BufReader::new(file))?;
            assert_eq!(reader.header().bit_depth, depth);
            assert_eq!((reader.header().width, reader.header().height), (160, 90));
            let mut frames = 0;
            while let Some([y, u, v]) = reader.read_frame()? {
                assert_eq!((y.len(), u.len(), v.len()), (160 * 90, 80 * 45, 80 * 45));
                assert!(y.iter().all(|&s| s < 1 << depth));
                frames += 1;
            }
            assert!(frames > 0, "{name}");
        }
        Ok(())
    }
}
