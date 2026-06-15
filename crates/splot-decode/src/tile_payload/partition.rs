// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.3.2 partition decision boundary.
//!
//! Feature tracking: `DECODE-TILE-PARTITION-DECISION-BOUNDARY`.

use splot_core::Error as CoreError;
use splot_core::symbol::{Symbol, SymbolDecoder};

use super::cdf::context::{PartitionContextInput, RectPartitionType, SquareSplitContextInput};
use super::cdf::partition_read::PartitionEntrySymbolReadError;
use super::cdf::{TileCdfError, TileCdfSubset};

const EXT_PARTITION_TYPES: usize = 10;

/// AV2 § 6.19.3 partition values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub(crate) enum PartitionType {
    /// `PARTITION_NONE`.
    None = 0,
    /// `PARTITION_HORZ`.
    Horz = 1,
    /// `PARTITION_VERT`.
    Vert = 2,
    /// `PARTITION_HORZ_3`.
    Horz3 = 3,
    /// `PARTITION_VERT_3`.
    Vert3 = 4,
    /// `PARTITION_HORZ_4A`.
    Horz4A = 5,
    /// `PARTITION_HORZ_4B`.
    Horz4B = 6,
    /// `PARTITION_VERT_4A`.
    Vert4A = 7,
    /// `PARTITION_VERT_4B`.
    Vert4B = 8,
    /// `PARTITION_SPLIT`.
    Split = 9,
}

impl PartitionType {
    const ALL: [Self; EXT_PARTITION_TYPES] = [
        Self::None,
        Self::Horz,
        Self::Vert,
        Self::Horz3,
        Self::Vert3,
        Self::Horz4A,
        Self::Horz4B,
        Self::Vert4A,
        Self::Vert4B,
        Self::Split,
    ];

    const HORZ_FAMILY: [Self; 4] = [Self::Horz, Self::Horz3, Self::Horz4A, Self::Horz4B];
    const VERT_FAMILY: [Self; 4] = [Self::Vert, Self::Vert3, Self::Vert4A, Self::Vert4B];

    const fn index(self) -> usize {
        self as usize
    }
}

/// Caller-provided AV2 § 5.20.3.2 allowed partition set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AllowedPartitions {
    flags: [bool; EXT_PARTITION_TYPES],
}

impl AllowedPartitions {
    /// Creates an allowed partition set in AV2 partition enum order.
    #[must_use]
    pub(crate) const fn new(flags: [bool; EXT_PARTITION_TYPES]) -> Self {
        Self { flags }
    }

    fn contains(self, partition: PartitionType) -> bool {
        self.flags[partition.index()]
    }

    fn count(self) -> usize {
        self.flags.iter().filter(|allowed| **allowed).count()
    }

    fn only(self) -> Option<PartitionType> {
        let mut found = None;
        for partition in PartitionType::ALL {
            if self.contains(partition) {
                if found.is_some() {
                    return None;
                }
                found = Some(partition);
            }
        }
        found
    }

    fn any(self, partitions: &[PartitionType]) -> bool {
        partitions
            .iter()
            .copied()
            .any(|partition| self.contains(partition))
    }
}

/// Inputs for one AV2 § 5.20.3.2 partition decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReadPartitionDecisionInput<'a> {
    allowed: AllowedPartitions,
    implied_partition: Option<PartitionType>,
    bru_active: bool,
    rect_type: Option<RectPartitionType>,
    partition_context: PartitionContextInput<'a>,
    square_split_context: SquareSplitContextInput<'a>,
}

impl<'a> ReadPartitionDecisionInput<'a> {
    /// Creates one partition decision input from already-derived caller facts.
    #[must_use]
    pub(crate) const fn new(
        allowed: AllowedPartitions,
        implied_partition: Option<PartitionType>,
        bru_active: bool,
        rect_type: Option<RectPartitionType>,
        partition_context: PartitionContextInput<'a>,
        square_split_context: SquareSplitContextInput<'a>,
    ) -> Self {
        Self {
            allowed,
            implied_partition,
            bru_active,
            rect_type,
            partition_context,
            square_split_context,
        }
    }
}

/// Trace of branch-local syntax consumed by one partition decision.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ReadPartitionDecisionTrace {
    /// Decoded `do_split`, when read.
    pub(crate) do_split: Option<bool>,
    /// Decoded `do_square_split`, when read.
    pub(crate) do_square_split: Option<bool>,
    /// Decoded `rect_type`, when read.
    pub(crate) rect_type: Option<RectPartitionType>,
    /// Decoded `do_ext_partition`, when read.
    pub(crate) do_ext_partition: Option<bool>,
    /// Decoded `do_uneven_4way_partition`, when read.
    pub(crate) do_uneven_4way_partition: Option<bool>,
    /// Decoded `uneven_4way_partition_type`, when read.
    pub(crate) uneven_4way_partition_type: Option<bool>,
}

/// Result of one AV2 § 5.20.3.2 partition decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReadPartitionDecision {
    /// Final partition outcome.
    pub(crate) partition: PartitionType,
    /// Branch-local syntax consumption trace.
    pub(crate) trace: ReadPartitionDecisionTrace,
}

impl ReadPartitionDecision {
    const fn new(partition: PartitionType, trace: ReadPartitionDecisionTrace) -> Self {
        Self { partition, trace }
    }
}

/// Error returned by the crate-private partition decision boundary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PartitionDecisionError {
    /// The caller-provided allowed set was empty.
    #[error("partition decision has no allowed partitions")]
    EmptyAllowedSet,
    /// The CDF context selector failed.
    #[error("partition decision CDF selection failed: {0}")]
    Cdf(#[from] TileCdfError),
    /// An `S()` partition symbol read failed.
    #[error("partition decision symbol read failed: {0}")]
    Symbol(#[from] PartitionEntrySymbolReadError),
    /// An `L(1)` literal read failed.
    #[error("partition decision literal read failed: {0}")]
    Literal(#[source] CoreError),
    /// A binary `S()` returned a value outside the expected boolean domain.
    #[error("{syntax} decoded symbol {symbol} is outside the boolean domain")]
    BooleanSymbolOutOfRange {
        /// Syntax element name.
        syntax: &'static str,
        /// Raw decoded symbol.
        symbol: u8,
    },
    /// The caller-provided implied partition was not in the allowed set.
    #[error("partition decision implied disallowed partition {partition:?}")]
    ImpliedPartitionDisallowed {
        /// Implied partition from caller facts.
        partition: PartitionType,
    },
    /// Inactive BRU mode would return `PARTITION_NONE`, but it is disallowed.
    #[error("inactive BRU partition decision selected disallowed partition {partition:?}")]
    InactiveBruPartitionDisallowed {
        /// Partition selected by the inactive BRU branch.
        partition: PartitionType,
    },
    /// A final table result was inconsistent with caller-provided allowed facts.
    #[error("partition decision selected disallowed partition {partition:?}")]
    FinalPartitionDisallowed {
        /// Final partition from the decision branch.
        partition: PartitionType,
    },
}

/// Runs one AV2 § 5.20.3.2 partition decision over caller-provided facts.
pub(crate) fn read_partition_decision(
    input: ReadPartitionDecisionInput<'_>,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<ReadPartitionDecision, PartitionDecisionError> {
    if input.allowed.count() == 0 {
        return Err(PartitionDecisionError::EmptyAllowedSet);
    }

    let trace = ReadPartitionDecisionTrace::default();
    if let Some(partition) = input.implied_partition {
        if !input.allowed.contains(partition) {
            // AV2 §5.20.3.2 falls through when an implied partition is not
            // allowed. Once `partition_implied` and `init_allowed_partitions`
            // are derived by the same caller, this state means those caller
            // facts disagree, so this boundary rejects it before reading syntax.
            return Err(PartitionDecisionError::ImpliedPartitionDisallowed { partition });
        }
        return Ok(ReadPartitionDecision::new(partition, trace));
    }

    if let Some(partition) = input.allowed.only() {
        return Ok(ReadPartitionDecision::new(partition, trace));
    }

    if !input.bru_active {
        if !input.allowed.contains(PartitionType::None) {
            // AV2 §5.20.3.2 returns PARTITION_NONE unconditionally outside
            // BRU-active mode. A spec-derived allowed set that excludes NONE in
            // this branch is an internal caller-fact invariant violation.
            return Err(PartitionDecisionError::InactiveBruPartitionDisallowed {
                partition: PartitionType::None,
            });
        }
        return Ok(ReadPartitionDecision::new(PartitionType::None, trace));
    }

    let mut trace = trace;
    if input.allowed.contains(PartitionType::None) {
        let do_split = read_partition_bool(
            "do_split",
            input.partition_context.do_split_selector()?,
            cdfs,
            symbols,
        )?;
        trace.do_split = Some(do_split);
        if !do_split {
            return Ok(ReadPartitionDecision::new(PartitionType::None, trace));
        }
    }

    if input.allowed.contains(PartitionType::Split) {
        let do_square_split = read_partition_bool(
            "do_square_split",
            input.square_split_context.do_square_split_selector()?,
            cdfs,
            symbols,
        )?;
        trace.do_square_split = Some(do_square_split);
        if do_square_split {
            return Ok(ReadPartitionDecision::new(PartitionType::Split, trace));
        }
    }

    let rect_type = resolve_rect_type(input, &mut trace, cdfs, symbols)?;
    let (non_ext_allowed, ext_allowed3, ext_allowed4) = match rect_type {
        RectPartitionType::Horz => (
            input.allowed.contains(PartitionType::Horz),
            input.allowed.contains(PartitionType::Horz3),
            input.allowed.contains(PartitionType::Horz4A)
                || input.allowed.contains(PartitionType::Horz4B),
        ),
        RectPartitionType::Vert => (
            input.allowed.contains(PartitionType::Vert),
            input.allowed.contains(PartitionType::Vert3),
            input.allowed.contains(PartitionType::Vert4A)
                || input.allowed.contains(PartitionType::Vert4B),
        ),
    };

    let do_ext_partition = if non_ext_allowed && (ext_allowed3 || ext_allowed4) {
        let value = read_partition_bool(
            "do_ext_partition",
            input
                .partition_context
                .do_ext_partition_selector(rect_type)?,
            cdfs,
            symbols,
        )?;
        trace.do_ext_partition = Some(value);
        value
    } else {
        ext_allowed3 || ext_allowed4
    };

    let mut do_uneven_4way_partition = false;
    let mut uneven_4way_partition_type = false;
    if do_ext_partition {
        if ext_allowed3 && ext_allowed4 {
            do_uneven_4way_partition = read_partition_bool(
                "do_uneven_4way_partition",
                input
                    .partition_context
                    .do_uneven_4way_partition_selector(rect_type)?,
                cdfs,
                symbols,
            )?;
            trace.do_uneven_4way_partition = Some(do_uneven_4way_partition);
        } else {
            do_uneven_4way_partition = ext_allowed4;
        }

        if do_uneven_4way_partition {
            uneven_4way_partition_type = symbols
                .read_literal(1)
                .map_err(PartitionDecisionError::Literal)?
                != 0;
            trace.uneven_4way_partition_type = Some(uneven_4way_partition_type);
        }
    }

    let partition = rect_part_table(
        do_ext_partition,
        do_uneven_4way_partition,
        uneven_4way_partition_type,
        rect_type,
    );
    if !input.allowed.contains(partition) {
        // AV2 §5.20.3.2 returns the Rect_Part_Table result. If the caller's
        // allowed facts reject that result, future full traversal must treat it
        // as an invalid derived-state invariant rather than continuing decode.
        return Err(PartitionDecisionError::FinalPartitionDisallowed { partition });
    }

    Ok(ReadPartitionDecision::new(partition, trace))
}

fn resolve_rect_type(
    input: ReadPartitionDecisionInput<'_>,
    trace: &mut ReadPartitionDecisionTrace,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<RectPartitionType, PartitionDecisionError> {
    if let Some(rect_type) = input.rect_type {
        return Ok(rect_type);
    }

    let allow_horz = input.allowed.any(&PartitionType::HORZ_FAMILY);
    let allow_vert = input.allowed.any(&PartitionType::VERT_FAMILY);
    if !allow_horz {
        return Ok(RectPartitionType::Vert);
    }
    if !allow_vert {
        return Ok(RectPartitionType::Horz);
    }

    let rect_type = read_partition_bool(
        "rect_type",
        input.partition_context.rect_type_selector()?,
        cdfs,
        symbols,
    )?;
    let rect_type = if rect_type {
        RectPartitionType::Vert
    } else {
        RectPartitionType::Horz
    };
    trace.rect_type = Some(rect_type);
    Ok(rect_type)
}

fn read_partition_bool(
    syntax: &'static str,
    selector: super::cdf::TileCdfSelector,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<bool, PartitionDecisionError> {
    symbol_to_bool(syntax, cdfs.read_partition_entry_symbol(selector, symbols)?)
}

fn symbol_to_bool(syntax: &'static str, symbol: Symbol) -> Result<bool, PartitionDecisionError> {
    match symbol.get() {
        0 => Ok(false),
        1 => Ok(true),
        value => Err(PartitionDecisionError::BooleanSymbolOutOfRange {
            syntax,
            symbol: value,
        }),
    }
}

fn rect_part_table(
    do_ext_partition: bool,
    do_uneven_4way_partition: bool,
    uneven_4way_partition_type: bool,
    rect_type: RectPartitionType,
) -> PartitionType {
    match (
        do_ext_partition,
        do_uneven_4way_partition,
        uneven_4way_partition_type,
        rect_type,
    ) {
        (false, _, _, RectPartitionType::Horz) => PartitionType::Horz,
        (false, _, _, RectPartitionType::Vert) => PartitionType::Vert,
        (true, false, _, RectPartitionType::Horz) => PartitionType::Horz3,
        (true, false, _, RectPartitionType::Vert) => PartitionType::Vert3,
        (true, true, false, RectPartitionType::Horz) => PartitionType::Horz4A,
        (true, true, false, RectPartitionType::Vert) => PartitionType::Vert4A,
        (true, true, true, RectPartitionType::Horz) => PartitionType::Horz4B,
        (true, true, true, RectPartitionType::Vert) => PartitionType::Vert4B,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use splot_core::span::ByteOffset;
    use splot_core::symbol::{CdfUpdateMode, SymbolDecoderConfig};

    use super::super::cdf::FrameCdfSubset;
    use super::*;

    const BLOCK_4X4: usize = 0;
    const BLOCK_16X16: usize = 6;
    const BLOCK_32X32: usize = 9;

    fn allowed(partitions: &[PartitionType]) -> AllowedPartitions {
        let mut flags = [false; EXT_PARTITION_TYPES];
        for partition in partitions {
            flags[partition.index()] = true;
        }
        AllowedPartitions::new(flags)
    }

    fn partition_context() -> PartitionContextInput<'static> {
        static LEFT0: [usize; 32] = [BLOCK_4X4; 32];
        static LEFT1: [usize; 32] = [BLOCK_4X4; 32];
        static ABOVE0: [usize; 32] = [BLOCK_4X4; 32];
        static ABOVE1: [usize; 32] = [BLOCK_4X4; 32];
        PartitionContextInput::new(BLOCK_32X32, 0, 0, 0, [&LEFT0, &LEFT1], [&ABOVE0, &ABOVE1])
            .unwrap()
    }

    fn square_context() -> SquareSplitContextInput<'static> {
        static ROW0: [usize; 2] = [BLOCK_4X4; 2];
        static ROW1: [usize; 2] = [BLOCK_4X4; 2];
        static GRID0: [&[usize]; 2] = [&ROW0, &ROW1];
        static GRID1: [&[usize]; 2] = [&ROW0, &ROW1];
        SquareSplitContextInput::new(BLOCK_16X16, 0, 0, 0, false, false, [&GRID0, &GRID1]).unwrap()
    }

    fn input(
        allowed: AllowedPartitions,
        implied_partition: Option<PartitionType>,
        bru_active: bool,
        rect_type: Option<RectPartitionType>,
    ) -> ReadPartitionDecisionInput<'static> {
        ReadPartitionDecisionInput::new(
            allowed,
            implied_partition,
            bru_active,
            rect_type,
            partition_context(),
            square_context(),
        )
    }

    fn decoder(payload: &'static [u8], update_mode: CdfUpdateMode) -> SymbolDecoder<'static> {
        SymbolDecoder::with_base_and_config(
            payload,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(update_mode),
        )
        .unwrap()
    }

    fn cdfs() -> TileCdfSubset {
        FrameCdfSubset::from_defaults().tile_copy()
    }

    fn decision(
        input: ReadPartitionDecisionInput<'static>,
        payload: &'static [u8],
    ) -> (
        Result<ReadPartitionDecision, PartitionDecisionError>,
        TileCdfSubset,
        SymbolDecoder<'static>,
    ) {
        let mut cdfs = cdfs();
        let mut symbols = decoder(payload, CdfUpdateMode::Enabled);
        let result = read_partition_decision(input, &mut cdfs, &mut symbols);
        (result, cdfs, symbols)
    }

    #[test]
    fn implied_partition_returns_without_symbol_consumption() {
        let mut cdfs = cdfs();
        let before = cdfs.clone();
        let mut symbols = decoder(&[0x00, 0x80], CdfUpdateMode::Enabled);

        let result = read_partition_decision(
            input(
                allowed(&[PartitionType::Horz, PartitionType::Vert]),
                Some(PartitionType::Vert),
                true,
                None,
            ),
            &mut cdfs,
            &mut symbols,
        )
        .unwrap();

        assert_eq!(result.partition, PartitionType::Vert);
        assert_eq!(result.trace, ReadPartitionDecisionTrace::default());
        assert_eq!(symbols.symbol_count(), 0);
        assert_eq!(cdfs, before);
    }

    #[test]
    fn single_allowed_returns_in_spec_order_without_symbol_consumption() {
        let (result, cdfs_after, symbols) = decision(
            input(allowed(&[PartitionType::Horz4B]), None, true, None),
            &[0xFF, 0xFF],
        );

        assert_eq!(result.unwrap().partition, PartitionType::Horz4B);
        assert_eq!(symbols.symbol_count(), 0);
        assert_eq!(cdfs_after, cdfs());
    }

    #[test]
    fn inactive_bru_returns_none_without_symbol_consumption() {
        let (result, cdfs_after, symbols) = decision(
            input(
                allowed(&[
                    PartitionType::None,
                    PartitionType::Horz,
                    PartitionType::Vert,
                ]),
                None,
                false,
                None,
            ),
            &[0xFF, 0xFF],
        );

        assert_eq!(result.unwrap().partition, PartitionType::None);
        assert_eq!(symbols.symbol_count(), 0);
        assert_eq!(cdfs_after, cdfs());
    }

    #[test]
    fn disallowed_implied_partition_is_rejected_before_symbol_consumption() {
        let mut cdfs = cdfs();
        let before = cdfs.clone();
        let mut symbols = decoder(&[0xFF, 0xFF], CdfUpdateMode::Enabled);

        let err = read_partition_decision(
            input(
                allowed(&[PartitionType::Horz]),
                Some(PartitionType::Vert),
                true,
                None,
            ),
            &mut cdfs,
            &mut symbols,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            PartitionDecisionError::ImpliedPartitionDisallowed {
                partition: PartitionType::Vert
            }
        ));
        assert_eq!(symbols.symbol_count(), 0);
        assert_eq!(cdfs, before);
    }

    #[test]
    fn inactive_bru_disallowed_none_is_rejected_before_symbol_consumption() {
        let mut cdfs = cdfs();
        let before = cdfs.clone();
        let mut symbols = decoder(&[0xFF, 0xFF], CdfUpdateMode::Enabled);

        let err = read_partition_decision(
            input(
                allowed(&[PartitionType::Horz, PartitionType::Vert]),
                None,
                false,
                None,
            ),
            &mut cdfs,
            &mut symbols,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            PartitionDecisionError::InactiveBruPartitionDisallowed {
                partition: PartitionType::None
            }
        ));
        assert_eq!(symbols.symbol_count(), 0);
        assert_eq!(cdfs, before);
    }

    #[test]
    fn do_split_false_returns_none_and_stops() {
        let (result, cdfs_after, symbols) = decision(
            input(
                allowed(&[
                    PartitionType::None,
                    PartitionType::Split,
                    PartitionType::Horz,
                ]),
                None,
                true,
                Some(RectPartitionType::Horz),
            ),
            &[0x00, 0x80],
        );
        let result = result.unwrap();

        assert_eq!(result.partition, PartitionType::None);
        assert_eq!(result.trace.do_split, Some(false));
        assert_eq!(result.trace.do_square_split, None);
        assert_eq!(symbols.symbol_count(), 1);
        assert_ne!(cdfs_after, cdfs());
    }

    #[test]
    fn square_split_true_returns_split_before_rect_symbols() {
        let (result, _, symbols) = decision(
            input(
                allowed(&[
                    PartitionType::Split,
                    PartitionType::Horz,
                    PartitionType::Vert,
                ]),
                None,
                true,
                None,
            ),
            &[0xFF, 0xFF],
        );
        let result = result.unwrap();

        assert_eq!(result.partition, PartitionType::Split);
        assert_eq!(result.trace.do_split, None);
        assert_eq!(result.trace.do_square_split, Some(true));
        assert_eq!(result.trace.rect_type, None);
        assert_eq!(symbols.symbol_count(), 1);
    }

    #[test]
    fn rect_type_symbol_selects_vertical_non_extended_partition() {
        let (result, _, symbols) = decision(
            input(
                allowed(&[PartitionType::Horz, PartitionType::Vert]),
                None,
                true,
                None,
            ),
            &[0xFF, 0xFF],
        );
        let result = result.unwrap();

        assert_eq!(result.partition, PartitionType::Vert);
        assert_eq!(result.trace.do_square_split, None);
        assert_eq!(result.trace.rect_type, Some(RectPartitionType::Vert));
        assert_eq!(symbols.symbol_count(), 1);
    }

    #[test]
    fn forced_horizontal_rect_reads_ext_symbol_once() {
        let (result, _, symbols) = decision(
            input(
                allowed(&[PartitionType::Horz, PartitionType::Horz3]),
                None,
                true,
                Some(RectPartitionType::Horz),
            ),
            &[0xFF, 0xFF],
        );
        let result = result.unwrap();

        assert_eq!(result.partition, PartitionType::Horz3);
        assert_eq!(result.trace.do_ext_partition, Some(true));
        assert_eq!(result.trace.do_uneven_4way_partition, None);
        assert_eq!(symbols.symbol_count(), 1);
    }

    #[test]
    fn uneven_four_way_literal_selects_table_variant() {
        let (result, _, symbols) = decision(
            input(
                allowed(&[
                    PartitionType::Horz,
                    PartitionType::Horz3,
                    PartitionType::Horz4A,
                    PartitionType::Horz4B,
                ]),
                None,
                true,
                Some(RectPartitionType::Horz),
            ),
            &[0xFF, 0xFF],
        );
        let result = result.unwrap();

        assert_eq!(result.partition, PartitionType::Horz4B);
        assert_eq!(result.trace.do_ext_partition, Some(true));
        assert_eq!(result.trace.do_uneven_4way_partition, Some(true));
        assert_eq!(result.trace.uneven_4way_partition_type, Some(true));
        assert_eq!(symbols.symbol_count(), 3);
    }

    #[test]
    fn uneven_four_way_without_three_way_reads_only_literal() {
        let (result, _, symbols) = decision(
            input(
                allowed(&[
                    PartitionType::Vert,
                    PartitionType::Vert4A,
                    PartitionType::Vert4B,
                ]),
                None,
                true,
                Some(RectPartitionType::Vert),
            ),
            &[0xFF, 0xFF],
        );
        let result = result.unwrap();

        assert_eq!(result.partition, PartitionType::Vert4B);
        assert_eq!(result.trace.do_ext_partition, Some(true));
        assert_eq!(result.trace.do_uneven_4way_partition, None);
        assert_eq!(result.trace.uneven_4way_partition_type, Some(true));
        assert_eq!(symbols.symbol_count(), 2);
    }

    #[test]
    fn empty_allowed_set_is_rejected_before_symbol_consumption() {
        let mut cdfs = cdfs();
        let before = cdfs.clone();
        let mut symbols = decoder(&[0x80, 0x00], CdfUpdateMode::Enabled);

        let err = read_partition_decision(
            input(
                AllowedPartitions::new([false; EXT_PARTITION_TYPES]),
                None,
                true,
                None,
            ),
            &mut cdfs,
            &mut symbols,
        )
        .unwrap_err();

        assert!(matches!(err, PartitionDecisionError::EmptyAllowedSet));
        assert_eq!(symbols.symbol_count(), 0);
        assert_eq!(cdfs, before);
    }

    #[test]
    fn impossible_final_table_result_is_rejected() {
        let mut cdfs = cdfs();
        let mut symbols = decoder(&[0x00, 0x80], CdfUpdateMode::Enabled);

        let err = read_partition_decision(
            input(
                allowed(&[PartitionType::Vert, PartitionType::Vert3]),
                None,
                true,
                Some(RectPartitionType::Horz),
            ),
            &mut cdfs,
            &mut symbols,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            PartitionDecisionError::FinalPartitionDisallowed {
                partition: PartitionType::Horz
            }
        ));
    }

    #[test]
    fn cdf_selector_error_fails_before_symbol_consumption() {
        static EMPTY: [usize; 0] = [];
        static ROW0: [usize; 2] = [BLOCK_4X4; 2];
        static ROW1: [usize; 2] = [BLOCK_4X4; 2];
        static GRID0: [&[usize]; 2] = [&ROW0, &ROW1];
        static GRID1: [&[usize]; 2] = [&ROW0, &ROW1];
        let input = ReadPartitionDecisionInput::new(
            allowed(&[PartitionType::None, PartitionType::Horz]),
            None,
            true,
            Some(RectPartitionType::Horz),
            PartitionContextInput::new(BLOCK_32X32, 0, 1, 0, [&EMPTY, &EMPTY], [&EMPTY, &EMPTY])
                .unwrap(),
            SquareSplitContextInput::new(BLOCK_16X16, 0, 0, 0, false, false, [&GRID0, &GRID1])
                .unwrap(),
        );
        let mut cdfs = cdfs();
        let before = cdfs.clone();
        let mut symbols = decoder(&[0x00, 0x80], CdfUpdateMode::Enabled);

        let err = read_partition_decision(input, &mut cdfs, &mut symbols).unwrap_err();

        assert!(matches!(
            err,
            PartitionDecisionError::Cdf(TileCdfError::PartitionNeighborOutOfRange { .. })
        ));
        assert_eq!(symbols.symbol_count(), 0);
        assert_eq!(cdfs, before);
    }

    #[test]
    fn empty_payload_literal_branch_is_deterministic() {
        let mut cdfs = cdfs();
        let mut symbols = decoder(&[], CdfUpdateMode::Enabled);

        let result = read_partition_decision(
            input(
                allowed(&[PartitionType::Vert4A, PartitionType::Vert4B]),
                None,
                true,
                Some(RectPartitionType::Vert),
            ),
            &mut cdfs,
            &mut symbols,
        )
        .unwrap();

        assert_eq!(result.partition, PartitionType::Vert4A);
        assert_eq!(result.trace.uneven_4way_partition_type, Some(false));
        assert_eq!(symbols.symbol_count(), 1);
    }

    #[test]
    fn repeated_inputs_are_deterministic() {
        let input = input(
            allowed(&[
                PartitionType::Split,
                PartitionType::Horz,
                PartitionType::Vert,
            ]),
            None,
            true,
            None,
        );

        let (first, first_cdfs, first_symbols) = decision(input, &[0x00, 0x80]);
        let (second, second_cdfs, second_symbols) = decision(input, &[0x00, 0x80]);

        assert_eq!(first.unwrap(), second.unwrap());
        assert_eq!(first_cdfs, second_cdfs);
        assert_eq!(
            first_symbols.consumed_bits().get(),
            second_symbols.consumed_bits().get()
        );
    }

    #[test]
    fn bounded_payload_matrix_never_panics() {
        let allowed_sets = [
            allowed(&[
                PartitionType::None,
                PartitionType::Split,
                PartitionType::Horz,
            ]),
            allowed(&[
                PartitionType::Split,
                PartitionType::Horz,
                PartitionType::Vert,
            ]),
            allowed(&[
                PartitionType::Horz,
                PartitionType::Horz3,
                PartitionType::Horz4A,
            ]),
            allowed(&[
                PartitionType::Vert,
                PartitionType::Vert3,
                PartitionType::Vert4A,
            ]),
        ];
        let payloads: [&[u8]; 5] = [&[], &[0x00], &[0x80], &[0x00, 0x80], &[0xFF, 0xFF]];

        for allowed in allowed_sets {
            for payload in payloads {
                let mut cdfs = cdfs();
                let mut symbols = SymbolDecoder::with_base_and_config(
                    payload,
                    ByteOffset::new(0),
                    SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
                )
                .unwrap();
                let _ = read_partition_decision(
                    input(allowed, None, true, None),
                    &mut cdfs,
                    &mut symbols,
                );
            }
        }
    }
}
