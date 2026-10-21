// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_encode::{BitDepth, Config, SourceFrame};

fuzz_target!(|data: &[u8]| {
    let [w, h, flags, s0, s1, s2, rest @ ..] = data else {
        return;
    };
    let mut config = Config::new(u32::from(w % 17), u32::from(h % 17));
    config.bit_depth = if flags & 1 == 0 {
        BitDepth::Eight
    } else {
        BitDepth::Ten
    };
    let strides = [s0, s1, s2].map(|&s| usize::from(s % 20));
    let (a, rest) = rest.split_at(rest.len() / 3);
    let (b, c) = rest.split_at(rest.len() / 2);

    let frame = if flags & 2 == 0 {
        SourceFrame::from_u8(&config, 0, [(a, strides[0]), (b, strides[1]), (c, strides[2])])
    } else {
        let words = |bytes: &[u8]| -> Vec<u16> {
            bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect()
        };
        let (a, b, c) = (words(a), words(b), words(c));
        SourceFrame::from_u16(
            &config,
            0,
            [(&a, strides[0]), (&b, strides[1]), (&c, strides[2])],
        )
    };
    let Ok(frame) = frame else {
        return;
    };
    let max = config.bit_depth.max_sample();
    for (plane, size) in frame.planes().iter().zip(frame.plane_sizes()) {
        assert_eq!(plane.len(), size.width() * size.height());
        assert!(plane.iter().all(|&s| s <= max));
    }
});
