// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

use std::io;
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use splot_decode::{
    DecodeContext, DecodeError, DecodeOptions, DecodeOutputOperation, DecodeRuntimeConfig,
};
use splot_parallel::ThreadCount;

#[path = "../support/decode_runtime.rs"]
mod decode_runtime;

const FIXTURE_MODE_FLAG: u8 = 0b1000_0000;
const FAILING_WRITER_FLAG: u8 = 0b0100_0000;
const MAX_RAW_INPUT_BYTES: usize = 4096;
const MAX_CAPTURE_BYTES: usize = 8 * 1024;
const MINIMAL_LUMA_BYTES: usize = 64 * 64;
const MINIMAL_CHROMA_BYTES: usize = 32 * 32;
const MINIMAL_PAYLOAD_BYTES: usize = MINIMAL_LUMA_BYTES + MINIMAL_CHROMA_BYTES * 2;
const MINIMAL_LUMA_SAMPLE: u8 = 128;
const MINIMAL_EXPECTED_RAW: &[u8] =
    include_bytes!("../../tests/conformance/vectors/valid/syn-flat-intra-64x64-minimal.raw");

static CONTEXT: OnceLock<Option<DecodeContext>> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let (flags, payload) = match data.split_first() {
        Some((flags, payload)) => (*flags, payload),
        None => (0, &[][..]),
    };

    let Some(context) = CONTEXT
        .get_or_init(|| {
            DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).ok()
        })
        .as_ref()
    else {
        return;
    };

    let fixture_bytes;
    let bitstream = if flags & FIXTURE_MODE_FLAG == 0 {
        let len = payload.len().min(MAX_RAW_INPUT_BYTES);
        &payload[..len]
    } else {
        fixture_bytes = decode_runtime::mutated_minimal_fixture(payload);
        fixture_bytes.as_slice()
    };

    let options = DecodeOptions::new(decode_runtime::limits(flags, bitstream.len()));
    if flags & FAILING_WRITER_FLAG == 0 {
        let mut writer = BoundedCaptureWriter::new(MAX_CAPTURE_BYTES);
        if context
            .decode_raw_bytes(bitstream, options, &mut writer)
            .is_ok()
        {
            if bitstream == decode_runtime::MINIMAL_FIXTURE {
                assert_minimal_raw_shape(writer.bytes());
            }
        }
    } else {
        let mut writer = FailAfterBytes::new(usize::from(payload.first().copied().unwrap_or(0)));
        if let Err(DecodeError::Output { source }) =
            context.decode_raw_bytes(bitstream, options, &mut writer)
        {
            assert_eq!(source.operation(), DecodeOutputOperation::WriteRawStream);
            assert_eq!(source.source_kind(), "io");
        }
    }
});

fn assert_minimal_raw_shape(bytes: &[u8]) {
    assert_eq!(bytes.len(), MINIMAL_PAYLOAD_BYTES);
    let (luma, _chroma) = bytes.split_at(MINIMAL_LUMA_BYTES);
    assert!(luma.iter().all(|sample| *sample == MINIMAL_LUMA_SAMPLE));
    assert_eq!(bytes, MINIMAL_EXPECTED_RAW);
}

#[derive(Debug)]
struct BoundedCaptureWriter {
    bytes: Vec<u8>,
    max_bytes: usize,
}

impl BoundedCaptureWriter {
    const fn new(max_bytes: usize) -> Self {
        Self {
            bytes: Vec::new(),
            max_bytes,
        }
    }

    fn bytes(&self) -> &[u8] {
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

#[derive(Debug)]
struct FailAfterBytes {
    bytes_written: usize,
    max_bytes: usize,
}

impl FailAfterBytes {
    const fn new(max_bytes: usize) -> Self {
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
