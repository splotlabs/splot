// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient `read_quant` syntax.
//!
//! Feature tracking: `DECODE-COEFF-READ-QUANT-SYNTAX`.

use std::collections::TryReserveError;

use splot_core::Error as CoreError;
use splot_core::symbol::SymbolDecoder;

use super::quant_state::CoeffQuantReadInput;
use super::scan_walk::{CoeffScanEntry, NonZeroCoeffScanWalk};

const MIN_M: u32 = 1;
const MAX_M: u32 = 6;
const MAX_COEFF_REM_BITS: u32 = 32;

/// Block-level facts for §5.20.7.28 `read_quant` parsing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffReadQuantConfig {
    /// Whether hidden parity is active for this transform block.
    pub(crate) is_hidden: bool,
    /// Whether TCQ is allowed by the caller's current coefficient path.
    pub(crate) allow_tcq: bool,
    /// Initial `hrLevelAvg` entering the scan-walk quant loop.
    pub(crate) hr_level_avg: u32,
}

/// Per-coefficient caller facts for §5.20.7.28 `read_quant`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffReadQuantInput {
    /// Checked scan entry this input belongs to.
    pub(crate) entry: CoeffScanEntry,
    /// Local `Level[row][col]` value entering `read_quant`.
    pub(crate) level: u32,
    /// Caller-derived `maxLevel` for this scan entry.
    pub(crate) max_level: u32,
}

/// Reached `read_quant` syntax path for one coefficient.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffReadQuantPath {
    /// `level < maxLevel - allowTcq`; no literal bits were consumed.
    BelowThreshold,
    /// The extended §5.20.7.28 path consumed q-length and remainder syntax.
    Extended {
        /// Clipped `m = Clip3(1, 6, GetMsb(predLevel))`.
        m: u32,
        /// `k = m + 1`.
        k: u32,
        /// `cMax = Min(m + 4, 6)`.
        c_max: u32,
        /// Final q-length loop index.
        q: u32,
        /// Coefficient-remainder bit width.
        length: u32,
        /// Base value before `coeff_rem`.
        x_base: u32,
        /// Decoded `coeff_rem` literal value.
        coeff_rem: u32,
        /// Extension value added to `quant`.
        x: u32,
    },
}

/// Decoded `read_quant` result for one ordinary non-FSC coefficient.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffReadQuant {
    quant: CoeffQuantReadInput,
    path: CoeffReadQuantPath,
}

impl CoeffReadQuant {
    /// Result shape consumed by the later quant-state writer.
    #[must_use]
    pub(crate) const fn quant_input(self) -> CoeffQuantReadInput {
        self.quant
    }

    /// Reached syntax path.
    #[must_use]
    pub(crate) const fn path(self) -> CoeffReadQuantPath {
        self.path
    }
}

/// Error returned by the coefficient `read_quant` syntax boundary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffReadQuantError {
    /// The number of per-entry inputs did not match the checked scan walk.
    #[error("coefficient read_quant input count {inputs} does not match scan entries {entries}")]
    InputCountMismatch {
        /// Caller-provided input count.
        inputs: usize,
        /// Checked scan-walk entry count.
        entries: usize,
    },
    /// One input was not paired with the matching checked scan entry.
    #[error(
        "coefficient read_quant input {index} targets {actual:?}, expected checked scan entry {expected:?}"
    )]
    ScanEntryMismatch {
        /// Input index.
        index: usize,
        /// Expected checked scan entry.
        expected: CoeffScanEntry,
        /// Caller-provided scan entry.
        actual: CoeffScanEntry,
    },
    /// `maxLevel - allowTcq` underflowed for caller-provided facts.
    #[error("coefficient read_quant input {index} has invalid maxLevel {max_level}")]
    InvalidMaxLevel {
        /// Input index.
        index: usize,
        /// Caller-provided max level.
        max_level: u32,
        /// Caller-provided TCQ allowance.
        allow_tcq: bool,
    },
    /// Reading a literal bit sequence failed.
    #[error("coefficient read_quant input {index} literal read failed for {syntax}: {source}")]
    LiteralRead {
        /// Input index.
        index: usize,
        /// Syntax element being read.
        syntax: &'static str,
        /// Source symbol-decoder error.
        #[source]
        source: CoreError,
    },
    /// A `read_quant` arithmetic operation overflowed the local type.
    #[error("coefficient read_quant input {index} overflowed during {operation}")]
    QuantOverflow {
        /// Input index.
        index: usize,
        /// Operation name.
        operation: &'static str,
    },
    /// Allocation for decoded `read_quant` records failed.
    #[error("coefficient read_quant allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
}

/// Reads ordinary non-FSC AV2 §5.20.7.28 `read_quant` syntax.
///
/// The caller supplies scan-checked entries and the §5.20.7.27 facts that
/// choose `maxLevel`, hidden parity, TCQ allowance, and initial `hrLevelAvg`.
/// This helper consumes only the literal bits reached by
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28` and returns the
/// quant records consumed by the later quant-state writer. It does not read CDF
/// symbols, write coefficient state, update tile context lines, dequantize, or
/// invoke reconstruction.
pub(crate) fn read_nonzero_coeff_quants(
    symbols: &mut SymbolDecoder<'_>,
    walk: &NonZeroCoeffScanWalk,
    inputs: &[CoeffReadQuantInput],
    config: CoeffReadQuantConfig,
) -> Result<Vec<CoeffReadQuant>, CoeffReadQuantError> {
    let entries = walk.entries();
    if inputs.len() != entries.len() {
        return Err(CoeffReadQuantError::InputCountMismatch {
            inputs: inputs.len(),
            entries: entries.len(),
        });
    }
    for (index, (entry, input)) in entries
        .iter()
        .copied()
        .zip(inputs.iter().copied())
        .enumerate()
    {
        if input.entry != entry {
            return Err(CoeffReadQuantError::ScanEntryMismatch {
                index,
                expected: entry,
                actual: input.entry,
            });
        }
        input
            .max_level
            .checked_sub(u32::from(config.allow_tcq))
            .ok_or(CoeffReadQuantError::InvalidMaxLevel {
                index,
                max_level: input.max_level,
                allow_tcq: config.allow_tcq,
            })?;
    }

    let mut state = CoeffReadQuantState::new(config);
    let mut reads = Vec::new();
    reads.try_reserve(entries.len())?;
    for (index, input) in inputs.iter().copied().enumerate() {
        reads.push(state.read_one(symbols, index, input)?);
    }
    Ok(reads)
}

/// Stateful §5.20.7.28 `read_quant` stepper for interleaved coefficient loops.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffReadQuantState {
    is_hidden: bool,
    allow_tcq: bool,
    hr_level_avg: u32,
}

impl CoeffReadQuantState {
    /// Creates a `read_quant` state machine with the caller's initial facts.
    pub(crate) const fn new(config: CoeffReadQuantConfig) -> Self {
        Self {
            is_hidden: config.is_hidden,
            allow_tcq: config.allow_tcq,
            hr_level_avg: config.hr_level_avg,
        }
    }

    /// Reads one checked coefficient and updates the carried `hrLevelAvg`.
    pub(crate) fn read_one(
        &mut self,
        symbols: &mut SymbolDecoder<'_>,
        index: usize,
        input: CoeffReadQuantInput,
    ) -> Result<CoeffReadQuant, CoeffReadQuantError> {
        let threshold = input
            .max_level
            .checked_sub(u32::from(self.allow_tcq))
            .ok_or(CoeffReadQuantError::InvalidMaxLevel {
                index,
                max_level: input.max_level,
                allow_tcq: self.allow_tcq,
            })?;
        if input.level < threshold {
            return Ok(CoeffReadQuant {
                quant: CoeffQuantReadInput {
                    entry: input.entry,
                    quant: input.level,
                    hr_level_avg: self.hr_level_avg,
                },
                path: CoeffReadQuantPath::BelowThreshold,
            });
        }

        let lvl_shift = u32::from(input.entry.pos() == 0 && self.is_hidden);
        let pred_level = self.hr_level_avg >> lvl_shift;
        let m = get_msb(pred_level).clamp(MIN_M, MAX_M);
        let k = checked_add(index, m, 1, "m + 1")?;
        let c_max = (m + 4).min(6);

        let mut q = 0;
        while q < c_max {
            if read_one_bit(symbols, index, "q_length_bit")? {
                break;
            }
            q += 1;
        }

        let (length, x_base) = if q == c_max {
            let mut prefix = 0u32;
            while !read_one_bit(symbols, index, "golomb_length_bit")? {
                prefix = checked_add(index, prefix, 1, "golomb length prefix + 1")?;
                if prefix > MAX_COEFF_REM_BITS.saturating_sub(k) {
                    return Err(CoeffReadQuantError::QuantOverflow {
                        index,
                        operation: "coeff_rem literal width",
                    });
                }
            }
            let length = checked_add(index, prefix, k, "golomb length + k")?;
            let q_base = checked_shl_u64(index, u64::from(q), m, "q << m")?;
            let length_base = checked_shl_u64(index, 1, length, "1 << length")?;
            let k_base = checked_shl_u64(index, 1, k, "1 << k")?;
            (
                length,
                checked_u32(
                    index,
                    checked_add_u64(
                        index,
                        q_base,
                        checked_sub_u64(index, length_base, k_base, "1 << length - 1 << k")?,
                        "extended xBase",
                    )?,
                    "u64 xBase to u32",
                )?,
            )
        } else {
            (
                m,
                checked_u32(
                    index,
                    checked_shl_u64(index, u64::from(q), m, "q << m")?,
                    "u64 xBase to u32",
                )?,
            )
        };

        if length > MAX_COEFF_REM_BITS {
            return Err(CoeffReadQuantError::QuantOverflow {
                index,
                operation: "coeff_rem literal width",
            });
        }
        let coeff_rem = read_literal(symbols, index, length, "coeff_rem")?;
        let x = checked_u32(
            index,
            checked_add_u64(
                index,
                u64::from(x_base),
                u64::from(coeff_rem),
                "xBase + coeff_rem",
            )?,
            "u64 x to u32",
        )?;

        let shifted_x = checked_shl_u64(index, u64::from(x), lvl_shift, "x << lvlShift")?;
        let next_hr = checked_u32(
            index,
            checked_add_u64(
                index,
                shifted_x,
                u64::from(self.hr_level_avg),
                "x << lvlShift + hrLevelAvg",
            )? >> 1,
            "u64 hrLevelAvg to u32",
        )?;
        let quant_add = checked_u32(
            index,
            checked_shl_u64(
                index,
                u64::from(x),
                u32::from(self.allow_tcq),
                "x << allowTcq",
            )?,
            "u64 quant extension to u32",
        )?;
        let quant =
            input
                .level
                .checked_add(quant_add)
                .ok_or(CoeffReadQuantError::QuantOverflow {
                    index,
                    operation: "quant + x << allowTcq",
                })?;
        self.hr_level_avg = next_hr;

        Ok(CoeffReadQuant {
            quant: CoeffQuantReadInput {
                entry: input.entry,
                quant,
                hr_level_avg: next_hr,
            },
            path: CoeffReadQuantPath::Extended {
                m,
                k,
                c_max,
                q,
                length,
                x_base,
                coeff_rem,
                x,
            },
        })
    }
}

const fn get_msb(value: u32) -> u32 {
    if value == 0 {
        0
    } else {
        u32::BITS - 1 - value.leading_zeros()
    }
}

fn read_one_bit(
    symbols: &mut SymbolDecoder<'_>,
    index: usize,
    syntax: &'static str,
) -> Result<bool, CoeffReadQuantError> {
    Ok(read_literal(symbols, index, 1, syntax)? != 0)
}

fn read_literal(
    symbols: &mut SymbolDecoder<'_>,
    index: usize,
    width: u32,
    syntax: &'static str,
) -> Result<u32, CoeffReadQuantError> {
    symbols
        .read_literal(width)
        .map_err(|source| CoeffReadQuantError::LiteralRead {
            index,
            syntax,
            source,
        })
}

fn checked_add(
    index: usize,
    lhs: u32,
    rhs: u32,
    operation: &'static str,
) -> Result<u32, CoeffReadQuantError> {
    lhs.checked_add(rhs)
        .ok_or(CoeffReadQuantError::QuantOverflow { index, operation })
}

fn checked_shl_u64(
    index: usize,
    value: u64,
    shift: u32,
    operation: &'static str,
) -> Result<u64, CoeffReadQuantError> {
    value
        .checked_shl(shift)
        .ok_or(CoeffReadQuantError::QuantOverflow { index, operation })
}

fn checked_add_u64(
    index: usize,
    lhs: u64,
    rhs: u64,
    operation: &'static str,
) -> Result<u64, CoeffReadQuantError> {
    lhs.checked_add(rhs)
        .ok_or(CoeffReadQuantError::QuantOverflow { index, operation })
}

fn checked_sub_u64(
    index: usize,
    lhs: u64,
    rhs: u64,
    operation: &'static str,
) -> Result<u64, CoeffReadQuantError> {
    lhs.checked_sub(rhs)
        .ok_or(CoeffReadQuantError::QuantOverflow { index, operation })
}

fn checked_u32(
    index: usize,
    value: u64,
    operation: &'static str,
) -> Result<u32, CoeffReadQuantError> {
    u32::try_from(value).map_err(|_| CoeffReadQuantError::QuantOverflow { index, operation })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use splot_core::span::ByteOffset;
    use splot_core::symbol::{CdfUpdateMode, SymbolDecoderConfig};
    use splot_core::symbol_encoder::SymbolEncoder;

    use super::*;

    fn symbol_decoder(payload: &[u8]) -> SymbolDecoder<'_> {
        SymbolDecoder::with_base_and_config(
            payload,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
        )
        .unwrap()
    }

    /// Encodes the §5.20.7.28 `read_quant` bitstream for one Extended-path
    /// coefficient with `splot-core`'s `SymbolEncoder`, mirroring the decoder's
    /// reads exactly: the q-length unary code (`q` zeros, then a terminating `1`
    /// when `q < c_max`, otherwise the golomb length prefix of `golomb_prefix`
    /// zeros and a terminating `1`), followed by the `length`-bit `coeff_rem`
    /// literal. These are pure bypass writes, so no CDF is involved.
    fn encode_extended(
        enc: &mut SymbolEncoder,
        q: u32,
        c_max: u32,
        golomb_prefix: u32,
        length: u32,
        coeff_rem: u32,
    ) {
        for _ in 0..q {
            enc.write_bool(false).unwrap();
        }
        if q < c_max {
            enc.write_bool(true).unwrap();
        } else {
            for _ in 0..golomb_prefix {
                enc.write_bool(false).unwrap();
            }
            enc.write_bool(true).unwrap();
        }
        enc.write_literal(coeff_rem, length).unwrap();
    }

    fn walk() -> NonZeroCoeffScanWalk {
        NonZeroCoeffScanWalk::from_entries_for_test(vec![
            CoeffScanEntry::for_test(3, 9, 1, 1),
            CoeffScanEntry::for_test(2, 1, 0, 1),
            CoeffScanEntry::for_test(1, 8, 1, 0),
            CoeffScanEntry::for_test(0, 0, 0, 0),
        ])
    }

    fn input(entry: CoeffScanEntry, level: u32, max_level: u32) -> CoeffReadQuantInput {
        CoeffReadQuantInput {
            entry,
            level,
            max_level,
        }
    }

    fn config(hr_level_avg: u32) -> CoeffReadQuantConfig {
        CoeffReadQuantConfig {
            is_hidden: false,
            allow_tcq: false,
            hr_level_avg,
        }
    }

    #[test]
    fn read_quant_below_threshold_consumes_no_bits() {
        let walk = walk();
        let entries = walk.entries();
        let mut symbols = symbol_decoder(&[0x80]);
        let consumed_before = symbols.consumed_bits();
        let symbol_count_before = symbols.symbol_count();
        let inputs = [input(entries[0], 2, 5)];
        let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entries[0]]);

        let reads = read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config(7)).unwrap();

        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].path(), CoeffReadQuantPath::BelowThreshold);
        assert_eq!(reads[0].quant_input().quant, 2);
        assert_eq!(reads[0].quant_input().hr_level_avg, 7);
        assert_eq!(symbols.consumed_bits(), consumed_before);
        assert_eq!(symbols.symbol_count(), symbol_count_before);
    }

    #[test]
    fn read_quant_finite_q_length_updates_quant_and_hr_average() {
        let walk = walk();
        let entries = walk.entries();
        let mut symbols = symbol_decoder(&[0b0011_0100, 0x80]);
        let inputs = [input(entries[0], 3, 3)];
        let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entries[0]]);

        let reads = read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config(16)).unwrap();

        assert_eq!(reads[0].quant_input().quant, 45);
        assert_eq!(reads[0].quant_input().hr_level_avg, 29);
        assert_eq!(symbols.symbol_count(), 7);
        assert_eq!(
            reads[0].path(),
            CoeffReadQuantPath::Extended {
                m: 4,
                k: 5,
                c_max: 6,
                q: 2,
                length: 4,
                x_base: 32,
                coeff_rem: 10,
                x: 42,
            }
        );
    }

    #[test]
    fn read_quant_golomb_extension_path_reads_until_terminator() {
        let walk = walk();
        let entries = walk.entries();
        let mut symbols = symbol_decoder(&[0x03, 0x40, 0x80]);
        let inputs = [input(entries[0], 2, 2)];
        let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entries[0]]);

        let reads = read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config(1)).unwrap();

        assert_eq!(reads[0].quant_input().quant, 21);
        assert_eq!(reads[0].quant_input().hr_level_avg, 10);
        assert_eq!(symbols.symbol_count(), 10);
        assert_eq!(
            reads[0].path(),
            CoeffReadQuantPath::Extended {
                m: 1,
                k: 2,
                c_max: 5,
                q: 5,
                length: 3,
                x_base: 14,
                coeff_rem: 5,
                x: 19,
            }
        );
    }

    #[test]
    fn read_quant_hidden_dc_and_tcq_adjust_predicted_extension() {
        let walk = walk();
        let entries = walk.entries();
        let hidden_dc = entries[3];
        let mut symbols = symbol_decoder(&[0b1000_0100, 0x80]);
        let inputs = [input(hidden_dc, 2, 3)];
        let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![hidden_dc]);
        let config = CoeffReadQuantConfig {
            is_hidden: true,
            allow_tcq: true,
            hr_level_avg: 64,
        };

        let reads = read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config).unwrap();

        assert_eq!(reads[0].quant_input().quant, 4);
        assert_eq!(reads[0].quant_input().hr_level_avg, 33);
        assert_eq!(symbols.symbol_count(), 6);
        assert_eq!(
            reads[0].path(),
            CoeffReadQuantPath::Extended {
                m: 5,
                k: 6,
                c_max: 6,
                q: 0,
                length: 5,
                x_base: 0,
                coeff_rem: 1,
                x: 1,
            }
        );
    }

    #[test]
    fn read_quant_rejects_input_mismatch_before_consumption() {
        let walk = walk();
        let entries = walk.entries();
        let mut symbols = symbol_decoder(&[0xff, 0x80]);
        let consumed_before = symbols.consumed_bits();
        let symbol_count_before = symbols.symbol_count();
        let inputs = [input(entries[1], 3, 3)];
        let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entries[0]]);

        let err = read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config(1)).unwrap_err();

        assert!(matches!(
            err,
            CoeffReadQuantError::ScanEntryMismatch { index: 0, .. }
        ));
        assert_eq!(symbols.consumed_bits(), consumed_before);
        assert_eq!(symbols.symbol_count(), symbol_count_before);
    }

    #[test]
    fn read_quant_rejects_unterminated_golomb_prefix() {
        let walk = walk();
        let entries = walk.entries();
        let mut symbols = symbol_decoder(&[]);
        let inputs = [input(entries[0], 3, 3)];
        let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entries[0]]);

        let err = read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config(1)).unwrap_err();

        assert!(matches!(
            err,
            CoeffReadQuantError::QuantOverflow {
                operation: "coeff_rem literal width",
                ..
            }
        ));
    }

    #[test]
    fn read_quant_rejects_pathological_max_level_and_overflow() {
        let walk = walk();
        let entries = walk.entries();
        let mut invalid_symbols = symbol_decoder(&[0xff, 0x80]);
        let inputs = [input(entries[0], 0, 0)];
        let walk_one = NonZeroCoeffScanWalk::from_entries_for_test(vec![entries[0]]);
        let invalid = CoeffReadQuantConfig {
            is_hidden: false,
            allow_tcq: true,
            hr_level_avg: 1,
        };

        let err = read_nonzero_coeff_quants(&mut invalid_symbols, &walk_one, &inputs, invalid)
            .unwrap_err();

        assert!(matches!(
            err,
            CoeffReadQuantError::InvalidMaxLevel {
                index: 0,
                max_level: 0,
                allow_tcq: true,
            }
        ));

        let mut overflow_symbols = symbol_decoder(&[0b1100_0000, 0x80]);
        let overflow_inputs = [input(entries[0], u32::MAX, u32::MAX)];
        let err = read_nonzero_coeff_quants(
            &mut overflow_symbols,
            &walk_one,
            &overflow_inputs,
            config(1),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            CoeffReadQuantError::QuantOverflow {
                operation: "quant + x << allowTcq",
                ..
            }
        ));
    }

    #[test]
    fn read_quant_rejects_oversized_golomb_remainder_width() {
        let walk = walk();
        let entries = walk.entries();
        let mut symbols = symbol_decoder(&[0x00, 0x00, 0x00, 0x00, 0x08, 0x80]);
        let inputs = [input(entries[0], 2, 2)];
        let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entries[0]]);

        let err = read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config(1)).unwrap_err();

        assert!(matches!(
            err,
            CoeffReadQuantError::QuantOverflow {
                operation: "coeff_rem literal width",
                ..
            }
        ));
    }

    // --- §5.20.7.28 read_quant SymbolEncoder roundtrip proofs ---
    //
    // These drive the real `read_nonzero_coeff_quants` decode helper with bytes
    // produced by `SymbolEncoder` (the in-repo coder oracle), asserting the
    // decoded `quant` and `path` recover the coefficient the encoder wrote. The
    // expected `quant` is computed independently from the §5.20.7.28 formula in
    // the test, so a golomb-assembly decode bug surfaces as a mismatch. The
    // bypass write/read roundtrip itself is proven in splot-core; here the proof
    // is the read_quant magnitude-extension assembly.

    #[test]
    fn read_quant_finite_q_roundtrips_through_symbol_encoder() {
        // hr_level_avg = 16 -> pred_level = 16, m = 4, k = 5, c_max = 6.
        let entry = CoeffScanEntry::for_test(3, 9, 1, 1);
        let (m, k, c_max) = (4u32, 5u32, 6u32);
        let (q, coeff_rem) = (2u32, 10u32); // q < c_max -> finite-q path
        let length = m;
        let x_base = q << m;
        let x = x_base + coeff_rem;
        let level = 3u32;
        let expected_quant = level + x; // allow_tcq = 0 -> quant = level + x

        let mut enc = SymbolEncoder::new();
        encode_extended(&mut enc, q, c_max, 0, length, coeff_rem);
        let bytes = enc.finish().unwrap().into_bytes();

        let mut symbols = symbol_decoder(&bytes);
        let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entry]);
        let inputs = [input(entry, level, level)];
        let reads = read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config(16)).unwrap();

        assert_eq!(reads[0].quant_input().quant, expected_quant);
        assert_eq!(
            reads[0].path(),
            CoeffReadQuantPath::Extended {
                m,
                k,
                c_max,
                q,
                length,
                x_base,
                coeff_rem,
                x,
            }
        );
    }

    #[test]
    fn read_quant_golomb_extension_roundtrips_through_symbol_encoder() {
        // hr_level_avg = 1 -> pred_level = 1, m = 1, k = 2, c_max = 5.
        let entry = CoeffScanEntry::for_test(3, 9, 1, 1);
        let (m, k, c_max) = (1u32, 2u32, 5u32);
        let (golomb_prefix, coeff_rem) = (1u32, 5u32);
        let q = c_max; // q == c_max -> golomb extension path
        let length = golomb_prefix + k;
        let x_base = (q << m) + ((1 << length) - (1 << k));
        let x = x_base + coeff_rem;
        let level = 2u32;
        let expected_quant = level + x;

        let mut enc = SymbolEncoder::new();
        encode_extended(&mut enc, q, c_max, golomb_prefix, length, coeff_rem);
        let bytes = enc.finish().unwrap().into_bytes();

        let mut symbols = symbol_decoder(&bytes);
        let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entry]);
        let inputs = [input(entry, level, level)];
        let reads = read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config(1)).unwrap();

        assert_eq!(reads[0].quant_input().quant, expected_quant);
        assert_eq!(
            reads[0].path(),
            CoeffReadQuantPath::Extended {
                m,
                k,
                c_max,
                q,
                length,
                x_base,
                coeff_rem,
                x,
            }
        );
    }

    #[test]
    fn read_quant_finite_q_roundtrips_across_parameter_grid() {
        // hr_level_avg = 16 -> m = 4, k = 5, c_max = 6. Sweep the finite-q range
        // and several coeff_rem widths; every (q, coeff_rem) must roundtrip.
        let entry = CoeffScanEntry::for_test(3, 9, 1, 1);
        let (m, k, c_max) = (4u32, 5u32, 6u32);
        let level = 4u32;
        let mut cases = 0u32;
        for q in 0..c_max {
            for coeff_rem in [0u32, 1, 7, 15] {
                let length = m;
                let x_base = q << m;
                let x = x_base + coeff_rem;
                let expected_quant = level + x;

                let mut enc = SymbolEncoder::new();
                encode_extended(&mut enc, q, c_max, 0, length, coeff_rem);
                let bytes = enc.finish().unwrap().into_bytes();

                let mut symbols = symbol_decoder(&bytes);
                let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entry]);
                let inputs = [input(entry, level, level)];
                let reads =
                    read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config(16)).unwrap();

                assert_eq!(
                    reads[0].quant_input().quant,
                    expected_quant,
                    "q={q} coeff_rem={coeff_rem}"
                );
                assert_eq!(
                    reads[0].path(),
                    CoeffReadQuantPath::Extended {
                        m,
                        k,
                        c_max,
                        q,
                        length,
                        x_base,
                        coeff_rem,
                        x,
                    }
                );
                cases += 1;
            }
        }
        assert_eq!(cases, c_max * 4);
    }

    #[test]
    fn read_quant_multi_coeff_roundtrips_with_state_carry() {
        // Two coefficients in one stream: an Extended coefficient (writes bits)
        // followed by a BelowThreshold coefficient (writes none). Proves the
        // stateful scan-walk loop decodes both correctly from one encoder stream.
        let a = CoeffScanEntry::for_test(1, 8, 1, 0);
        let b = CoeffScanEntry::for_test(0, 0, 0, 0);
        // A: hr = 16 -> m = 4, c_max = 6; q = 1 (finite-q), coeff_rem = 3.
        let (m, c_max) = (4u32, 6u32);
        let (q, coeff_rem) = (1u32, 3u32);
        let length = m;
        let x = (q << m) + coeff_rem;
        let level_a = 4u32;
        let quant_a = level_a + x;
        // B: level 1 < threshold 5 -> BelowThreshold, quant = level, no bits.
        let level_b = 1u32;
        let max_b = 5u32;

        let mut enc = SymbolEncoder::new();
        encode_extended(&mut enc, q, c_max, 0, length, coeff_rem);
        let bytes = enc.finish().unwrap().into_bytes();

        let mut symbols = symbol_decoder(&bytes);
        let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![a, b]);
        let inputs = [input(a, level_a, level_a), input(b, level_b, max_b)];
        let reads = read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config(16)).unwrap();

        assert_eq!(reads[0].quant_input().quant, quant_a);
        assert_eq!(reads[1].quant_input().quant, level_b);
        assert_eq!(reads[1].path(), CoeffReadQuantPath::BelowThreshold);
    }
}
