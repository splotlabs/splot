// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::hint::black_box;
use std::time::Instant;

use splot_core::simd::{ResolvedBackend, SimdBackend, SimdPolicy, resolve_backend};

const ROWS: usize = 8;
const SPAN: usize = 20;
const BLOCKS: usize = 4;
const WINDOW: usize = 8;
const PACKED_BYTES: usize = BLOCKS * ROWS * WINDOW * 2;
const CASES: usize = 128;
const INITIAL_HASH: u64 = 0xcbf2_9ce4_8422_2325;

type Packed = [u8; PACKED_BYTES];
type Ranges = [u16; BLOCKS];

struct Case {
    storage: [Vec<u8>; 2],
    offset: [usize; 2],
    stride: usize,
}

impl Case {
    fn planes(&self) -> [&[u8]; 2] {
        [
            &self.storage[0][self.offset[0]..],
            &self.storage[1][self.offset[1]..],
        ]
    }
}

fn corpus() -> Vec<Case> {
    let mut seed = 0x9e37_79b9_u32;
    (0..CASES)
        .map(|index| {
            let strides = [20, 21, 24, 31, 32, 47, 64, 79, 128, 137, 256, 320];
            let stride = strides[index % strides.len()];
            let offset = [(3 * index) % 33, (5 * index + 1) % 35];
            let storage = std::array::from_fn(|plane| {
                let end = (ROWS - 1) * stride + SPAN + index % 3;
                (0..offset[plane] + end)
                    .map(|at| {
                        seed ^= seed << 13;
                        seed ^= seed >> 17;
                        seed ^= seed << 5;
                        let random = (seed >> 24) as u8;
                        let (row, column) = (at / stride, at % stride);
                        match (index / 3) % 6 {
                            0 => (plane * 255) as u8,
                            1 => random,
                            2 => 96 + (random & 7),
                            3 => ((column * column + row + 31 * plane) % 256) as u8,
                            4 if (row + plane) % 3 == 0 && column % 7 == 0 => 255,
                            4 => 0,
                            _ => ((row + column + plane) % 2 * 255) as u8,
                        }
                    })
                    .collect()
            });
            Case {
                storage,
                offset,
                stride,
            }
        })
        .collect()
}

fn reference(planes: [&[u8]; 2], stride: usize, bytes: &mut Packed) -> Option<Ranges> {
    let mut minima = [[u8::MAX; 2]; BLOCKS];
    let mut maxima = [[u8::MIN; 2]; BLOCKS];
    for row in 0..ROWS {
        let start = row * stride;
        let [Some(u), Some(v)] = planes.map(|plane| plane.get(start..start + SPAN)) else {
            return None;
        };
        for block in 0..BLOCKS {
            for column in 0..WINDOW {
                let sample = block * 4 + column;
                let slot = ((block * ROWS + row) * WINDOW + column) * 2;
                for (plane, value) in [u[sample], v[sample]].into_iter().enumerate() {
                    bytes[slot + plane] = value ^ 0x80;
                    minima[block][plane] = minima[block][plane].min(value);
                    maxima[block][plane] = maxima[block][plane].max(value);
                }
            }
        }
    }
    Some(std::array::from_fn(|block| {
        u16::from((maxima[block][0] - minima[block][0]).max(maxima[block][1] - minima[block][1]))
    }))
}

fn checksum(hash: u64, bytes: &Packed, ranges: Ranges) -> u64 {
    bytes
        .iter()
        .copied()
        .chain(ranges.into_iter().flat_map(u16::to_le_bytes))
        .fold(hash, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
}

fn verify(
    name: &str,
    cases: &[Case],
    gather: &impl Fn([&[u8]; 2], usize, &mut Packed) -> Option<Ranges>,
) -> Result<u64, String> {
    let mut hash = INITIAL_HASH;
    for (index, case) in cases.iter().enumerate() {
        let [u, v] = case.planes();
        let short = (index % ROWS) * case.stride + SPAN - 1;
        for (variant, planes) in [[u, v], [&u[..short], v], [u, &v[..short]]]
            .into_iter()
            .enumerate()
        {
            for sentinel in [0xa5, 0x5a] {
                let mut expected = [sentinel; PACKED_BYTES];
                let expected_ranges = reference(planes, case.stride, &mut expected);
                let mut guarded = [sentinel; PACKED_BYTES + 2];
                let actual = <&mut Packed>::try_from(&mut guarded[1..=PACKED_BYTES])
                    .map_err(|error| error.to_string())?;
                let ranges = gather(planes, case.stride, actual);
                if ranges != expected_ranges {
                    return Err(format!(
                        "{name} case {index}, stride {}, variant {variant}: ranges {ranges:?}, expected {expected_ranges:?}",
                        case.stride
                    ));
                }
                if let Some(position) = actual.iter().zip(&expected).position(|(a, b)| a != b) {
                    return Err(format!(
                        "{name} case {index}, stride {}, variant {variant}: byte {position} was {}, expected {}",
                        case.stride, actual[position], expected[position]
                    ));
                }
                if let Some(ranges) = ranges {
                    hash = checksum(hash, actual, ranges);
                }
                if guarded[0] != sentinel || guarded[PACKED_BYTES + 1] != sentinel {
                    return Err(format!(
                        "{name} case {index}: wrote outside the packed buffer"
                    ));
                }
            }
        }
    }
    Ok(hash)
}

fn measure(
    name: &str,
    cases: &[Case],
    iterations: usize,
    gather: impl Fn([&[u8]; 2], usize, &mut Packed) -> Option<Ranges>,
) -> Result<(), String> {
    let hash = verify(name, cases, &gather)?;
    println!(
        "{name} parity=ok cases={} checksum={hash:016x}",
        cases.len()
    );
    if iterations == 0 {
        return Ok(());
    }

    let mut packed = [0; PACKED_BYTES];
    let mut ranges = None;
    let start = Instant::now();
    for index in 0..iterations {
        let case = &cases[index % CASES];
        let (planes, stride) = black_box((case.planes(), case.stride));
        ranges = gather(planes, stride, &mut packed);
        black_box((&packed, ranges));
    }
    let elapsed = start.elapsed();
    let ranges = ranges.ok_or_else(|| format!("{name} timed gather failed"))?;
    let hash = checksum(INITIAL_HASH, &packed, ranges);
    println!(
        "{name} iterations={iterations} elapsed_ms={:.3} ns_per_call={:.2} final_checksum={hash:016x}",
        elapsed.as_secs_f64() * 1_000.0,
        elapsed.as_secs_f64() * 1_000_000_000.0 / iterations as f64
    );
    Ok(())
}

fn run() -> Result<(), String> {
    let mut sse2_only = false;
    let mut iterations = 1_000_000;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sse2-only" => sse2_only = true,
            "--iterations" => {
                iterations = args
                    .next()
                    .ok_or("--iterations requires a count")?
                    .parse()
                    .map_err(|error| format!("invalid iteration count: {error}"))?;
            }
            "--help" => {
                println!("Usage: cdef-micro [--sse2-only] [--iterations N] (0 runs parity only)");
                return Ok(());
            }
            _ => return Err(format!("unrecognized option {arg}")),
        }
    }

    let sse2 = resolve_backend(SimdPolicy::Sse2).map_err(|error| error.to_string())?;
    let ResolvedBackend::Sse2(sse2) = sse2 else {
        return Err("forced SSE2 did not yield the SSE2 backend".into());
    };
    let cases = corpus();
    let gather_sse2 = |planes: [&[u8]; 2], stride, packed: &mut Packed| {
        splot_cdef_base::gather_packed(sse2.token(), planes, stride, packed)
    };
    measure("sse2", &cases, iterations, gather_sse2)?;
    if sse2_only {
        let auto = resolve_backend(SimdPolicy::Auto).map_err(|error| error.to_string())?;
        if !matches!(auto, ResolvedBackend::Sse2(_)) || resolve_backend(SimdPolicy::Avx2).is_ok() {
            return Err("strict x86 baseline did not select SSE2 and reject forced AVX2".into());
        }
        println!("auto selected={} forced_avx2=rejected", auto.name());
        return Ok(());
    }

    let avx2 = resolve_backend(SimdPolicy::Avx2).map_err(|error| error.to_string())?;
    let ResolvedBackend::Avx2(avx2) = avx2 else {
        return Err("forced AVX2 did not yield the AVX2 backend".into());
    };
    measure("avx2", &cases, iterations, |planes, stride, packed| {
        splot_cdef_avx2::gather_packed(avx2.token(), planes, stride, packed)
    })?;

    let auto = resolve_backend(SimdPolicy::Auto).map_err(|error| error.to_string())?;
    println!("auto selected={}", auto.name());
    let gather_auto_avx2 = |token| {
        measure("auto", &cases, iterations, |planes, stride, packed| {
            splot_cdef_avx2::gather_packed(token, planes, stride, packed)
        })
    };
    match auto {
        ResolvedBackend::Sse2(_) => measure("auto", &cases, iterations, gather_sse2),
        ResolvedBackend::Avx2(backend) => gather_auto_avx2(backend.token()),
        ResolvedBackend::Avx512(backend) => gather_auto_avx2(backend.narrow().token()),
        _ => Err("Auto selected an unexpected backend for x86-64".into()),
    }
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
