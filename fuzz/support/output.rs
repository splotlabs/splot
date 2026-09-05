// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::io;

#[derive(Debug)]
pub(crate) struct FailAfterBytes {
    pub(crate) bytes_written: usize,
    max_bytes: usize,
}

impl FailAfterBytes {
    pub(crate) const fn new(max_bytes: usize) -> Self {
        Self {
            bytes_written: 0,
            max_bytes,
        }
    }
}

impl io::Write for FailAfterBytes {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.bytes_written >= self.max_bytes {
            return Err(io::Error::other("fuzz writer byte budget exhausted"));
        }
        let allowed = (self.max_bytes - self.bytes_written).min(buf.len());
        self.bytes_written += allowed;
        Ok(allowed)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
