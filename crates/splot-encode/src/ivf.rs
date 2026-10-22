// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! IVF output for encoder packets.

use std::io::{self, Write};

use splot_core::ivf::{IvfHeader, write_ivf_frame, write_ivf_header};

use crate::config::Config;
use crate::encoder::Packet;

/// Writes packets to an `AV02` IVF stream. The header frame count stays `0`.
#[derive(Debug)]
pub struct IvfWriter<W> {
    writer: W,
}

impl<W: Write> IvfWriter<W> {
    /// Writes the IVF header for `config`; the time base is the inverse frame rate.
    ///
    /// # Errors
    /// Returns [`io::ErrorKind::InvalidInput`] when a dimension exceeds the
    /// 16-bit IVF field, or any write error.
    pub fn new(mut writer: W, config: &Config) -> io::Result<Self> {
        let dimension = |value: u32| {
            u16::try_from(value).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "IVF frame dimensions must fit 16 bits",
                )
            })
        };
        let (rate, scale) = config.frame_rate;
        let header = IvfHeader::new(
            *b"AV02",
            dimension(config.width)?,
            dimension(config.height)?,
            rate,
            scale,
            0,
        );
        write_ivf_header(&mut writer, &header)?;
        Ok(Self { writer })
    }

    /// Writes one packet as an IVF frame stamped with its pts. Packets come in
    /// coding order, one temporal unit each, and each outputs one frame, so the
    /// pts of consecutive IVF frames increase.
    ///
    /// # Errors
    /// Returns any write error.
    pub fn write_packet(&mut self, packet: &Packet) -> io::Result<()> {
        write_ivf_frame(&mut self.writer, packet.pts, &packet.data)
    }

    /// Flushes and returns the inner writer.
    ///
    /// # Errors
    /// Returns any flush error.
    pub fn finish(mut self) -> io::Result<W> {
        self.writer.flush()?;
        Ok(self.writer)
    }
}

#[cfg(test)]
mod tests {
    use splot_core::ivf::{IvfFrameCursor, IvfFrameRead, parse_ivf_header};

    use super::*;
    use crate::rate_control::FrameType;
    use crate::stats::FrameStats;

    #[test]
    fn writes_a_parseable_av02_stream() -> io::Result<()> {
        let mut config = Config::new(160, 90);
        config.frame_rate = (50, 1);
        let mut writer = IvfWriter::new(Vec::new(), &config)?;
        writer.write_packet(&Packet {
            data: vec![1, 2, 3],
            pts: 4,
            frame_type: FrameType::Key,
            stats: vec![FrameStats {
                frame_index: 0,
                pts: 4,
                qp: 1,
                bits: 24,
                psnr_y: 0.0,
                psnr_u: 0.0,
                psnr_v: 0.0,
                ssim_y: None,
            }],
        })?;
        let bytes = writer.finish()?;
        let header = parse_ivf_header(&bytes).map_err(io::Error::other)?;
        assert_eq!(header.fourcc, *b"AV02");
        assert_eq!((header.width, header.height), (160, 90));
        assert_eq!(
            (header.timebase_denominator, header.timebase_numerator),
            (50, 1)
        );
        let mut cursor = IvfFrameCursor::new(&bytes, header);
        let frame = cursor.next_frame_record().map_err(io::Error::other)?;
        assert!(matches!(frame, IvfFrameRead::Frame(f) if f.pts == 4 && f.payload == [1, 2, 3]));
        assert!(IvfWriter::new(Vec::new(), &Config::new(1 << 16, 1)).is_err());
        Ok(())
    }
}
