// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.3.2 partition decision boundary.
//!
//! Feature tracking: `DECODE-TILE-PARTITION-DECISION-BOUNDARY`.

use splot_core::Error as CoreError;
use splot_core::symbol::SymbolDecoder;

use super::cdf::context::{PartitionContextInput, RectPartitionType, SquareSplitContextInput};
use super::cdf::partition_read::PartitionEntrySymbolReadError;
use super::cdf::{TileCdfError, TileCdfSelector, TileCdfSubset};
use super::partition_allowed::PartitionDecisionFacts;

const EXT_PARTITION_TYPES: usize = 10;

/// § 5.20.3.2 partition type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PartitionType {
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

/// A set of partition types, bit `PartitionType::index` per member.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AllowedPartitions {
    bits: u16,
}

impl AllowedPartitions {
    #[must_use]
    pub(crate) fn new(flags: [bool; EXT_PARTITION_TYPES]) -> Self {
        Self::from_bits(flags.iter().enumerate().fold(0, |bits, (index, &allowed)| {
            bits | u16::from(allowed) << index
        }))
    }

    pub(crate) const fn from_bits(bits: u16) -> Self {
        Self {
            bits: bits & ((1 << EXT_PARTITION_TYPES) - 1),
        }
    }

    pub(crate) const fn bits(self) -> u16 {
        self.bits
    }

    pub(crate) const fn contains(self, partition: PartitionType) -> bool {
        self.bits >> partition.index() & 1 != 0
    }

    pub(crate) const fn count(self) -> usize {
        self.bits.count_ones() as usize
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

    const fn has_any_allowed(self, allowed: AllowedPartitions) -> bool {
        allowed.contains(self.non_ext)
            || allowed.contains(self.three_way)
            || allowed.contains(self.four_way_a)
            || allowed.contains(self.four_way_b)
    }
}

/// Everything `read_partition_decision` reads, small enough to pass in
/// registers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReadPartitionDecisionInput {
    pub(super) facts: PartitionDecisionFacts,
    pub(super) bru_active: bool,
    pub(super) partition_context: PartitionContextInput,
    pub(super) square_split_context: SquareSplitContextInput,
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
    #[error("partition decision selected disallowed partition {partition:?}")]
    FinalPartitionDisallowed { partition: PartitionType },
    #[error("partition decision symbol write failed: {0}")]
    Write(#[source] splot_core::write::WriteError),
}

/// The source of the § 5.20.3.2 partition symbols: the decoder reads them,
/// and the encoder facade answers and writes them for a chosen partition, so
/// both run the one `read_partition` branching.
pub(crate) trait PartitionSymbols {
    /// A boolean `S()` symbol coded with the row of `selector`.
    fn flag(
        &mut self,
        syntax: &'static str,
        selector: TileCdfSelector,
        cdfs: &mut TileCdfSubset,
    ) -> Result<bool, PartitionDecisionError>;

    /// The `uneven_4way_partition_type` `L(1)` literal.
    fn bit(&mut self) -> Result<bool, PartitionDecisionError>;
}

impl PartitionSymbols for SymbolDecoder<'_> {
    #[allow(
        clippy::inline_always,
        reason = "constant selectors fold into a direct row read"
    )]
    #[inline(always)]
    fn flag(
        &mut self,
        _syntax: &'static str,
        selector: TileCdfSelector,
        cdfs: &mut TileCdfSubset,
    ) -> Result<bool, PartitionDecisionError> {
        Ok(cdfs.read_partition_flag(selector, self)?)
    }

    #[inline]
    fn bit(&mut self) -> Result<bool, PartitionDecisionError> {
        Ok(self
            .read_literal(1)
            .map_err(PartitionDecisionError::Literal)?
            != 0)
    }
}

impl PartitionDecisionError {
    pub(crate) fn is_source_read_failure(&self) -> bool {
        match self {
            Self::Literal(_) => true,
            Self::Symbol(PartitionEntrySymbolReadError::Symbol(error)) => {
                !matches!(error, CoreError::InvalidSymbolCdf { .. })
            }
            _ => false,
        }
    }
}

#[allow(clippy::inline_always, reason = "measured: once per partition node")]
#[inline(always)]
pub(crate) fn read_partition_decision<S: PartitionSymbols + ?Sized>(
    input: ReadPartitionDecisionInput,
    cdfs: &mut TileCdfSubset,
    symbols: &mut S,
) -> Result<PartitionType, PartitionDecisionError> {
    let allowed = input.facts.allowed();
    if allowed.count() == 0 {
        return Err(PartitionDecisionError::EmptyAllowedSet);
    }

    if let Some(partition) = input.facts.unread_partition() {
        return Ok(partition);
    }

    if !input.bru_active {
        return Ok(PartitionType::None);
    }

    if allowed.contains(PartitionType::None) {
        let do_split = symbols.flag(
            "do_split",
            input.partition_context.do_split_selector(),
            cdfs,
        )?;
        if !do_split {
            return Ok(PartitionType::None);
        }
    }

    if allowed.contains(PartitionType::Split) {
        let do_square_split = symbols.flag(
            "do_square_split",
            input.square_split_context.do_square_split_selector(),
            cdfs,
        )?;
        if do_square_split {
            return Ok(PartitionType::Split);
        }
    }

    let (facts, context) = (input.facts, input.partition_context);
    let rect_type = resolve_rect_type(facts, &context, cdfs, symbols)?;
    let partition = read_rect_partition(facts, &context, rect_type, cdfs, symbols)?;
    if !allowed.contains(partition) {
        return Err(PartitionDecisionError::FinalPartitionDisallowed { partition });
    }

    Ok(partition)
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "measured: the walk keeps the context unpacked"
)]
fn read_rect_partition<S: PartitionSymbols + ?Sized>(
    facts: PartitionDecisionFacts,
    context: &PartitionContextInput,
    rect_type: RectPartitionType,
    cdfs: &mut TileCdfSubset,
    symbols: &mut S,
) -> Result<PartitionType, PartitionDecisionError> {
    let family = RectPartitionFamily::for_rect_type(rect_type);
    let allowed = facts.allowed();
    let non_ext_allowed = allowed.contains(family.non_ext);
    let ext_allowed3 = allowed.contains(family.three_way);
    let ext_allowed4 = allowed.contains(family.four_way_a) || allowed.contains(family.four_way_b);
    let ext_allowed = ext_allowed3 || ext_allowed4;

    let do_ext_partition = if non_ext_allowed && ext_allowed {
        symbols.flag(
            "do_ext_partition",
            context.do_ext_partition_selector(rect_type),
            cdfs,
        )?
    } else {
        ext_allowed
    };

    let do_uneven_4way_partition = if !do_ext_partition {
        false
    } else if ext_allowed3 && ext_allowed4 {
        symbols.flag(
            "do_uneven_4way_partition",
            context.do_uneven_4way_partition_selector(rect_type),
            cdfs,
        )?
    } else {
        ext_allowed4
    };

    let uneven_4way_partition_type = if do_uneven_4way_partition {
        symbols.bit()?
    } else {
        false
    };

    Ok(family.final_partition(
        do_ext_partition,
        do_uneven_4way_partition,
        uneven_4way_partition_type,
    ))
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "measured: the walk keeps the context unpacked"
)]
fn resolve_rect_type<S: PartitionSymbols + ?Sized>(
    facts: PartitionDecisionFacts,
    context: &PartitionContextInput,
    cdfs: &mut TileCdfSubset,
    symbols: &mut S,
) -> Result<RectPartitionType, PartitionDecisionError> {
    if let Some(rect_type) = facts.rect_type() {
        return Ok(rect_type);
    }

    let allow_horz = RectPartitionFamily::HORZ.has_any_allowed(facts.allowed());
    let allow_vert = RectPartitionFamily::VERT.has_any_allowed(facts.allowed());
    if !allow_horz {
        return Ok(RectPartitionType::Vert);
    }
    if !allow_vert {
        return Ok(RectPartitionType::Horz);
    }

    let rect_type = symbols.flag("rect_type", context.rect_type_selector(), cdfs)?;
    Ok(if rect_type {
        RectPartitionType::Vert
    } else {
        RectPartitionType::Horz
    })
}

#[cfg(test)]
#[path = "partition_tests.rs"]
mod tests;
