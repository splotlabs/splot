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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub(crate) enum PartitionType {
    None = 0,
    Horz = 1,
    Vert = 2,
    Horz3 = 3,
    Vert3 = 4,
    Horz4A = 5,
    Horz4B = 6,
    Vert4A = 7,
    Vert4B = 8,
    Split = 9,
}

impl PartitionType {
    pub(crate) const ALL: [Self; EXT_PARTITION_TYPES] = [
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

    pub(crate) const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AllowedPartitions {
    flags: [bool; EXT_PARTITION_TYPES],
}

impl AllowedPartitions {
    #[must_use]
    pub(crate) const fn new(flags: [bool; EXT_PARTITION_TYPES]) -> Self {
        Self { flags }
    }

    pub(crate) fn contains(self, partition: PartitionType) -> bool {
        self.flags[partition.index()]
    }

    pub(crate) fn count(self) -> usize {
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RectPartitionFamily {
    non_ext: PartitionType,
    three_way: PartitionType,
    four_way_a: PartitionType,
    four_way_b: PartitionType,
}

impl RectPartitionFamily {
    const HORZ: Self = Self {
        non_ext: PartitionType::Horz,
        three_way: PartitionType::Horz3,
        four_way_a: PartitionType::Horz4A,
        four_way_b: PartitionType::Horz4B,
    };
    const VERT: Self = Self {
        non_ext: PartitionType::Vert,
        three_way: PartitionType::Vert3,
        four_way_a: PartitionType::Vert4A,
        four_way_b: PartitionType::Vert4B,
    };

    const fn for_rect_type(rect_type: RectPartitionType) -> Self {
        match rect_type {
            RectPartitionType::Horz => Self::HORZ,
            RectPartitionType::Vert => Self::VERT,
        }
    }

    const fn final_partition(
        self,
        do_ext_partition: bool,
        do_uneven_4way_partition: bool,
        uneven_4way_partition_type: bool,
    ) -> PartitionType {
        if !do_ext_partition {
            return self.non_ext;
        }
        if !do_uneven_4way_partition {
            return self.three_way;
        }
        if uneven_4way_partition_type {
            self.four_way_b
        } else {
            self.four_way_a
        }
    }

    fn has_any_allowed(self, allowed: AllowedPartitions) -> bool {
        allowed.contains(self.non_ext)
            || allowed.contains(self.three_way)
            || allowed.contains(self.four_way_a)
            || allowed.contains(self.four_way_b)
    }
}

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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ReadPartitionDecisionTrace {
    pub(crate) do_split: Option<bool>,
    pub(crate) do_square_split: Option<bool>,
    pub(crate) rect_type: Option<RectPartitionType>,
    pub(crate) do_ext_partition: Option<bool>,
    pub(crate) do_uneven_4way_partition: Option<bool>,
    pub(crate) uneven_4way_partition_type: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReadPartitionDecision {
    pub(crate) partition: PartitionType,
    pub(crate) trace: ReadPartitionDecisionTrace,
}

impl ReadPartitionDecision {
    const fn new(partition: PartitionType, trace: ReadPartitionDecisionTrace) -> Self {
        Self { partition, trace }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PartitionDecisionError {
    #[error("partition decision has no allowed partitions")]
    EmptyAllowedSet,
    #[error("partition decision CDF selection failed: {0}")]
    Cdf(#[from] TileCdfError),
    #[error("partition decision symbol read failed: {0}")]
    Symbol(#[from] PartitionEntrySymbolReadError),
    #[error("partition decision literal read failed: {0}")]
    Literal(#[source] CoreError),
    #[error("{syntax} decoded symbol {symbol} is outside the boolean domain")]
    BooleanSymbolOutOfRange { syntax: &'static str, symbol: u8 },
    #[error("partition decision selected disallowed partition {partition:?}")]
    FinalPartitionDisallowed { partition: PartitionType },
}

pub(crate) fn read_partition_decision(
    input: ReadPartitionDecisionInput<'_>,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<ReadPartitionDecision, PartitionDecisionError> {
    if input.allowed.count() == 0 {
        return Err(PartitionDecisionError::EmptyAllowedSet);
    }

    let trace = ReadPartitionDecisionTrace::default();
    if let Some(partition) = input.implied_partition
        && input.allowed.contains(partition)
    {
        return Ok(ReadPartitionDecision::new(partition, trace));
    }

    if let Some(partition) = input.allowed.only() {
        return Ok(ReadPartitionDecision::new(partition, trace));
    }

    if !input.bru_active {
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
    let partition = read_rect_partition(input, rect_type, &mut trace, cdfs, symbols)?;
    if !input.allowed.contains(partition) {
        return Err(PartitionDecisionError::FinalPartitionDisallowed { partition });
    }

    Ok(ReadPartitionDecision::new(partition, trace))
}

fn read_rect_partition(
    input: ReadPartitionDecisionInput<'_>,
    rect_type: RectPartitionType,
    trace: &mut ReadPartitionDecisionTrace,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
) -> Result<PartitionType, PartitionDecisionError> {
    let family = RectPartitionFamily::for_rect_type(rect_type);
    let non_ext_allowed = input.allowed.contains(family.non_ext);
    let ext_allowed3 = input.allowed.contains(family.three_way);
    let ext_allowed4 =
        input.allowed.contains(family.four_way_a) || input.allowed.contains(family.four_way_b);
    let ext_allowed = ext_allowed3 || ext_allowed4;

    let do_ext_partition = if non_ext_allowed && ext_allowed {
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
        ext_allowed
    };

    let do_uneven_4way_partition = if !do_ext_partition {
        false
    } else if ext_allowed3 && ext_allowed4 {
        let value = read_partition_bool(
            "do_uneven_4way_partition",
            input
                .partition_context
                .do_uneven_4way_partition_selector(rect_type)?,
            cdfs,
            symbols,
        )?;
        trace.do_uneven_4way_partition = Some(value);
        value
    } else {
        ext_allowed4
    };

    let uneven_4way_partition_type = if do_uneven_4way_partition {
        let value = symbols
            .read_literal(1)
            .map_err(PartitionDecisionError::Literal)?
            != 0;
        trace.uneven_4way_partition_type = Some(value);
        value
    } else {
        false
    };

    Ok(family.final_partition(
        do_ext_partition,
        do_uneven_4way_partition,
        uneven_4way_partition_type,
    ))
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

    let allow_horz = RectPartitionFamily::HORZ.has_any_allowed(input.allowed);
    let allow_vert = RectPartitionFamily::VERT.has_any_allowed(input.allowed);
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
    let symbol = cdfs.read_partition_entry_symbol(selector, symbols)?;
    symbol_to_bool(syntax, symbol)
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

#[cfg(test)]
#[path = "partition_tests.rs"]
mod tests;
