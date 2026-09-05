// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::io;

#[derive(Debug)]
pub(crate) struct BoundedCaptureWriter {
    bytes: Vec<u8>,
    max_bytes: usize,
}

impl BoundedCaptureWriter {
    pub(crate) const fn new(max_bytes: usize) -> Self {
        Self {
            bytes: Vec::new(),
            max_bytes,
        }
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl io::Write for BoundedCaptureWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Some(next_len) = self.bytes.len().checked_add(buf.len()) else {
            return Err(io::Error::other("fuzz writer byte count overflow"));
        };
        if next_len > self.max_bytes {
            return Err(io::Error::other("fuzz writer byte budget exhausted"));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
