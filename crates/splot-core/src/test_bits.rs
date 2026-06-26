// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared MSB-first bit writer for building header/syntax payloads in tests.
//!
//! Every `crates/splot-core` test module that hand-builds a byte payload used the
//! same `Bits` writer; this is the single canonical copy they share. Test-only:
//! the module is `#[cfg(test)]` and carries no runtime cost.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

/// MSB-first bit writer: push bits, then pack them into bytes.
#[derive(Default)]
pub(crate) struct Bits {
    /// One entry per bit (0 or 1), MSB first.
    pub(crate) bits: Vec<u8>,
}

impl Bits {
    pub(crate) fn bit(&mut self, bit: u8) {
        self.bits.push(bit & 1);
    }

    pub(crate) fn f(&mut self, value: u32, width: u32) {
        for shift in (0..width).rev() {
            self.bit(((value >> shift) & 1) as u8);
        }
    }

    pub(crate) fn uvlc(&mut self, value: u32) {
        let code_num = value + 1;
        let leading_zeros = u32::BITS - 1 - code_num.leading_zeros();
        for _ in 0..leading_zeros {
            self.bit(0);
        }
        self.bit(1);
        if leading_zeros > 0 {
            self.f(code_num - (1 << leading_zeros), leading_zeros);
        }
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        let mut bytes = Vec::new();
        for chunk in self.bits.chunks(8) {
            let mut byte = 0u8;
            for (i, bit) in chunk.iter().enumerate() {
                byte |= *bit << (7 - i);
            }
            bytes.push(byte);
        }
        bytes
    }

    pub(crate) fn align(&mut self) {
        while !self.bits.len().is_multiple_of(8) {
            self.bit(0);
        }
    }

    pub(crate) fn append(&mut self, other: &Bits) {
        self.bits.extend_from_slice(&other.bits);
    }

    pub(crate) fn bit_len(&self) -> usize {
        self.bits.len()
    }

    pub(crate) fn extend(&mut self, other: Bits) {
        self.bits.extend(other.bits);
    }

    pub(crate) fn leb128_byte(&mut self, value: u32) {
        assert!(value < 128, "test helper only encodes single-byte leb128");
        self.f(value, 8);
    }

    pub(crate) fn ns(&mut self, value: u32, n: u32) {
        let w = u32::BITS - n.leading_zeros();
        let m = (1u32 << w) - n;
        if value < m {
            self.f(value, w - 1);
        } else {
            self.f(value + m, w);
        }
    }

    pub(crate) fn ptl(&mut self, profile: u32, level: u32, tier: u8, mlayer: u32, reserved: u32) {
        self.f(profile, 5);
        self.f(level, 5);
        self.bit(tier);
        self.f(mlayer, 3);
        self.f(reserved, 2);
    }

    pub(crate) fn raw(&mut self, s: &str) {
        for c in s.chars() {
            match c {
                '0' => self.bit(0),
                '1' => self.bit(1),
                _ => {}
            }
        }
    }

    pub(crate) fn rg(&mut self, value: u32, n: u32) {
        let q = value >> n;
        let remainder = value & ((1 << n) - 1);
        for _ in 0..q {
            self.bit(1);
        }
        self.bit(0);
        self.f(remainder, n);
    }

    pub(crate) fn su(&mut self, value: i32, width: u32) {
        self.f((value as u32) & ((1u32 << width) - 1), width);
    }

    pub(crate) fn svlc(&mut self, value: i32) {
        // Invert half = (uvlc + 1) >> 1 with sign in the uvlc parity.
        let uvlc = if value > 0 {
            2 * (value as u32) - 1
        } else {
            2 * (value.unsigned_abs())
        };
        self.uvlc(uvlc);
    }

    pub(crate) fn tu(&mut self, value: u32, mx: u32) {
        for _ in 0..value {
            self.bit(1);
        }
        if value < mx {
            self.bit(0);
        }
    }
}
