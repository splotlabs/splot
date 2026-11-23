// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! CBR filler: `OBU_PADDING` (§ 5.16) where the decoder buffer would
//! overflow.

use super::*;
use splot_core::annexb::parse_annex_b_obus_partial;
use splot_core::types::ObuType;

use crate::error::Error;

/// 60 kbps at 30 fps: one frame period brings 2000 bits.
const KBPS: u64 = 60;
const PERIOD: u64 = KBPS * 1000 / 30;
/// A 250 ms buffer that starts half full, so a static clip fills it fast.
const BUFFER: (u64, u64) = (250, 125);

fn cbr(padding: bool, threads: usize) -> Config {
    let mut config = Config::new(SIZE.0, SIZE.1);
    config.preset = Preset::new(9).unwrap();
    config.rate_control = RateControlMode::Cbr {
        bitrate_kbps: KBPS as u32,
        buffer_ms: BUFFER.0 as u32,
        initial_ms: BUFFER.1 as u32,
    };
    config.cbr_padding = padding;
    config.threads = splot_parallel::ThreadCount::from(threads);
    config
}

/// The same frame every time.
fn still(config: &Config, _index: u64) -> SourceFrame {
    frame(config, 0, 0, 0)
}

/// The texture of [`frame`] panning by two samples a frame.
fn panning(config: &Config, index: u64) -> SourceFrame {
    frame(config, index, 2 * index as usize, 0)
}

/// `unit` without the padding OBU that ends it, and the padding's size.
fn strip(unit: &[u8]) -> (&[u8], usize) {
    let obus = parse_annex_b_obus_partial(unit).obus;
    let padding = obus
        .iter()
        .filter(|obu| obu.header.obu_type == ObuType::Padding)
        .count();
    assert!(padding <= 1, "{padding} padding OBUs in one unit");
    match obus.last() {
        Some(last) if last.header.obu_type == ObuType::Padding => {
            let body = 1 + last.payload.len();
            let size = body + (usize::BITS - body.leading_zeros()).div_ceil(7) as usize;
            (&unit[..unit.len() - size], size)
        }
        _ => (unit, 0),
    }
}

/// The largest excess over the buffer size of a decoder buffer that takes
/// `units` as [`underflows`] does, before it clamps to its size.
fn overflow(units: &[Vec<u8>]) -> u64 {
    let size = KBPS * BUFFER.0;
    let mut level = KBPS * BUFFER.1;
    let mut worst = 0;
    for unit in units {
        level = level.saturating_sub(unit.len() as u64 * 8) + PERIOD;
        worst = worst.max(level.saturating_sub(size));
        level = level.min(size);
    }
    worst
}

/// A still clip fills the buffer: from the first padding OBU on every unit
/// carries one frame period (to the byte the padding rounds up), so the
/// stream runs at the target; no unit underflows or overflows the decoder
/// buffer. Without padding the buffer overflows. The decoded frames are the
/// encoder recon, and the stream without its padding OBUs decodes to the
/// same frames. The bytes do not depend on the thread count.
#[test]
fn still_cbr_streams_pad_to_the_target() {
    let stream = encode_config(&cbr(true, 1), still);
    let padded: Vec<usize> = (0..stream.units.len())
        .filter(|&index| strip(&stream.units[index]).1 > 0)
        .collect();
    let first = padded[0];
    assert!(padded.len() > stream.units.len() / 2, "{padded:?}");
    let tail = &stream.units[first + 1..];
    for unit in tail {
        let bits = unit.len() as u64 * 8;
        assert!((PERIOD..PERIOD + 16).contains(&bits), "{bits} bits");
    }
    let mean = tail.iter().map(|unit| unit.len() as u64 * 8).sum::<u64>() / tail.len() as u64;
    assert!(mean.abs_diff(PERIOD) * 100 <= PERIOD, "{mean} bits a unit");
    assert!(underflows(&stream.units, KBPS, BUFFER).is_empty());
    assert!(overflow(&stream.units) <= 8, "{}", overflow(&stream.units));
    let plain = encode_config(&cbr(false, 1), still);
    assert!(overflow(&plain.units) > PERIOD / 2);

    let decoded = decode(&stream.units.concat());
    assert_same_samples(&decoded, &stream.expected, "padded");
    let stripped: Vec<u8> = stream
        .units
        .iter()
        .flat_map(|unit| strip(unit).0)
        .copied()
        .collect();
    assert_same_samples(&decode(&stripped), &decoded, "stripped");
    {
        let threads = 2;
        let other = encode_config(&cbr(true, threads), still);
        assert!(other.units == stream.units, "{threads} threads");
    }
}

/// A panning clip at a bitrate whose frames keep the buffer from filling
/// codes no padding: the same bytes as without it.
#[test]
fn streams_that_do_not_overflow_get_no_padding() {
    let stream = encode_config(&cbr(true, 1), panning);
    assert!(stream.units.iter().all(|unit| strip(unit).1 == 0));
    assert_eq!(stream.units, encode_config(&cbr(false, 1), panning).units);
}

/// Padding needs CBR, and the VBR first pass codes none.
#[test]
fn padding_without_cbr_is_refused() {
    let mut config = cbr(true, 1);
    config.rate_control = RateControlMode::Vbr { bitrate_kbps: 60 };
    assert!(matches!(config.validate(), Err(Error::PaddingWithoutCbr)));
    assert!(!config.first_pass_config().unwrap().cbr_padding);
    assert!(cbr(true, 1).validate().is_ok());
}

/// AVM skips the padding OBUs like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_padded_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let stream = encode_config(&cbr(true, 1), still);
    let splot = decode(&stream.units.concat());
    let avm = avm_decode(&avmdec, &stream.units, SIZE, BitDepth::Eight, "cbrpadding");
    assert!(avm == splot, "AVM output differs");
}
