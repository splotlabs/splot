// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Luma transform partitions (§ 5.20.6.3) and luma transform types (§ 5.20.8.2)
//! of the encoder facade.
//!
//! Each choice maps to the symbols that select it by inverting the decoder's own
//! set, mapping, and geometry functions, so the facade writes exactly what the
//! decoder reads.

use splot_core::tables::conversion::{
    SIZE_TO_TX_PART_GROUP_LOOKUP, SIZE_TO_TX_TYPE_GROUP_VERT_AND_HORZ,
    SIZE_TO_TX_TYPE_GROUP_VERT_OR_HORZ, TX_SIZE_SQR,
};

use super::super::cdf::TileCdfSelector;
use super::super::general_intra_residual::{
    LumaTransformPartitionContext, TX_SET_HIGH_32, TX_SET_HIGH_64, TX_SET_INTRA_2, TX_SET_WIDE_32,
    TX_SET_WIDE_64, block_size_table_usize, dct_forced, inter_tx_type_symbols, long_tx_set_shape,
    long_tx_type_from_index, luma_transform_records_for_partition, luma_tx_partition_allows,
    md_idx_luma_tx_type, transform_set_from_flags,
};
use super::{EncoderSyntaxError, IntraTileSyntax, LeafBlock, Result, TransformBlock, TxType, rule};
use crate::pipeline::general_intra::luma_tx_partition_context;

pub use super::super::general_intra_residual::LumaTxPartition as TxPartition;

/// Every § 6.19.6.3 transform partition, in `txPartition` order.
pub const TX_PARTITIONS: [TxPartition; 8] = [
    TxPartition::None,
    TxPartition::Split,
    TxPartition::Horz,
    TxPartition::Vert,
    TxPartition::Horz4,
    TxPartition::Vert4,
    TxPartition::Horz5,
    TxPartition::Vert5,
];

/// One symbol: its CDF and its value.
type Step = (TileCdfSelector, usize);

const PARTITION: &str = "transform partition";

impl IntraTileSyntax {
    /// The luma transform partitions `leaf` can code; only `None` when the frame
    /// is not `TX_MODE_SELECT` or the block codes no partition symbol.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for an invalid block size.
    pub fn luma_tx_partitions(&self, leaf: &LeafBlock) -> Result<Vec<TxPartition>> {
        let mut out = Vec::new();
        for partition in TX_PARTITIONS {
            if self.tx_partition_steps(leaf, partition)?.is_some() {
                out.push(partition);
            }
        }
        Ok(out)
    }

    /// Sends the § 5.20.6.3 symbols that select `partition` for `leaf` to `put`
    /// as `(CDF row, value)`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Unsupported`] for a partition `leaf` cannot
    /// code, and the first error of `put`.
    pub fn code_luma_tx_partition<E: From<EncoderSyntaxError>>(
        &mut self,
        leaf: &LeafBlock,
        partition: TxPartition,
        mut put: impl FnMut(&mut [u16], usize) -> core::result::Result<(), E>,
    ) -> core::result::Result<(), E> {
        let steps =
            self.tx_partition_steps(leaf, partition)?
                .ok_or(EncoderSyntaxError::Unsupported(
                    "transform partition of this block",
                ))?;
        for (selector, value) in steps {
            put(self.tile_row(selector)?, value)?;
        }
        Ok(())
    }

    /// The luma types `tx` of the planned leaf can code when its `eob` is above
    /// one, in symbol order; only `DCT_DCT` when the type is inferred.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a chroma block or without a planned leaf.
    pub fn allowed_luma_tx_types(&mut self, tx: &TransformBlock) -> Result<Vec<TxType>> {
        Ok(self
            .luma_tx_type_options(tx, 2)?
            .into_iter()
            .map(|(tx_type, _)| tx_type)
            .collect())
    }

    /// The type the decoder infers for luma block `tx` with `eob`, or `None`
    /// when it reads a coded type.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a chroma block or without a planned leaf.
    pub fn inferred_luma_tx_type(
        &mut self,
        tx: &TransformBlock,
        eob: usize,
    ) -> Result<Option<TxType>> {
        let options = self.luma_tx_type_options(tx, eob)?;
        Ok(match options.as_slice() {
            [(tx_type, steps)] if steps.is_empty() => Some(*tx_type),
            _ => None,
        })
    }

    /// Sends the § 5.20.8.2 symbols that select `tx.tx_type` for luma block `tx`
    /// with `eob >= 1` to `put` as `(CDF row, value)`; nothing when the type is
    /// inferred.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Unsupported`] for a type the decoder does not
    /// read or infer for `tx` and `eob`, and the first error of `put`.
    pub fn code_luma_tx_type<E: From<EncoderSyntaxError>>(
        &mut self,
        tx: &TransformBlock,
        eob: usize,
        mut put: impl FnMut(&mut [u16], usize) -> core::result::Result<(), E>,
    ) -> core::result::Result<(), E> {
        let (_, steps) = self
            .luma_tx_type_options(tx, eob)?
            .into_iter()
            .find(|&(tx_type, _)| tx_type == tx.tx_type)
            .ok_or(EncoderSyntaxError::Unsupported(
                "luma transform type for this block and eob",
            ))?;
        for (selector, value) in steps {
            put(self.tile_row(selector)?, value)?;
        }
        Ok(())
    }

    /// The transform size a `partition` of `leaf` splits, or `None` when the plan
    /// keeps the luma plane whole.
    pub(super) fn partitioned_tx_size(
        &self,
        leaf: &LeafBlock,
        partition: TxPartition,
    ) -> Result<Option<usize>> {
        match self.tx_partition_steps(leaf, partition)? {
            None => Err(EncoderSyntaxError::Unsupported(
                "transform partition of this block",
            )),
            Some(steps) if steps.is_empty() => Ok(None),
            Some(_) => Ok(self
                .partition_context(leaf)?
                .map(|(context, _)| context.max_tx_size())),
        }
    }

    /// The § 5.20.6.3 partition context of `leaf` and its `(allowHorz, allowVert)`
    /// when it reads `tx_do_partition`.
    fn partition_context(
        &self,
        leaf: &LeafBlock,
    ) -> Result<Option<(LumaTransformPartitionContext, (bool, bool))>> {
        let Some(context) =
            luma_tx_partition_context(self.tools.tx_mode, leaf.frontier.b_size, false)
        else {
            return Ok(None);
        };
        Ok(luma_tx_partition_allows(context)
            .map_err(rule(PARTITION))?
            .map(|allows| (context, allows)))
    }

    /// The symbols that select `partition` for `leaf`, inverting
    /// `read_luma_tx_partition_type`; `None` when `leaf` cannot code it.
    fn tx_partition_steps(
        &self,
        leaf: &LeafBlock,
        partition: TxPartition,
    ) -> Result<Option<Vec<Step>>> {
        let Some((context, (allow_horz, allow_vert))) = self.partition_context(leaf)? else {
            return Ok((partition == TxPartition::None).then(Vec::new));
        };
        if partition != TxPartition::None
            && luma_transform_records_for_partition(0, 0, context.max_tx_size(), partition).is_err()
        {
            return Ok(None);
        }
        let mi_size = context.block_size().index();
        let group = |table: &[i32], name| {
            block_size_table_usize(table, name, mi_size).map_err(rule(PARTITION))
        };
        let mut steps = vec![(
            TileCdfSelector::TxDoPartition {
                fsc_mode: 0,
                is_inter: 0,
                txfm_split_group: group(
                    &SIZE_TO_TX_PART_GROUP_LOOKUP,
                    "Size_To_Tx_Part_Group_Lookup",
                )?,
            },
            usize::from(partition != TxPartition::None),
        )];
        if partition == TxPartition::None {
            return Ok(Some(steps));
        }
        if allow_horz && allow_vert {
            let Some(symbol) =
                (0..=u8::MAX).find(|&s| TxPartition::from_partition_type_symbol(s) == partition)
            else {
                return Ok(None);
            };
            steps.push((
                TileCdfSelector::TxPartitionType {
                    fsc_mode: 0,
                    is_inter: 0,
                    ctx: group(
                        &SIZE_TO_TX_TYPE_GROUP_VERT_AND_HORZ,
                        "Size_To_Tx_Type_Group_Vert_And_Horz",
                    )?,
                    reduced: self.tools.reduced_tx_part_set,
                },
                usize::from(symbol),
            ));
            return Ok(Some(steps));
        }
        let (two, four) = if allow_horz {
            (TxPartition::Horz, TxPartition::Horz4)
        } else {
            (TxPartition::Vert, TxPartition::Vert4)
        };
        let vert_or_horz = group(
            &SIZE_TO_TX_TYPE_GROUP_VERT_OR_HORZ,
            "Size_To_Tx_Type_Group_Vert_Or_Horz",
        )?;
        if vert_or_horz > 0 && !self.tools.reduced_tx_part_set {
            if partition != two && partition != four {
                return Ok(None);
            }
            steps.push((
                TileCdfSelector::Tx2Or3PartitionType {
                    fsc_mode: 0,
                    is_inter: 0,
                    ctx: vert_or_horz - 1,
                },
                usize::from(partition == four),
            ));
        } else if partition != two {
            return Ok(None);
        }
        Ok(Some(steps))
    }

    /// Every type luma block `tx` with `eob` can carry and the symbols that
    /// select it, inverting `read_active_luma_transform_type`; one inferred
    /// `DCT_DCT` without symbols when `dct_forced`.
    fn luma_tx_type_options(
        &mut self,
        tx: &TransformBlock,
        eob: usize,
    ) -> Result<Vec<(TxType, Vec<Step>)>> {
        if tx.plane != 0 {
            return Err(EncoderSyntaxError::Unsupported(
                "coded transform type of a chroma block",
            ));
        }
        let reduced = self.tools.reduced_tx_set;
        let is_inter = self.leaf_is_inter();
        let tx_set = transform_set_from_flags(reduced, false, 0, tx.tx_size, is_inter)
            .map_err(rule("transform set"))?;
        if dct_forced(is_inter, 0, eob, false, reduced, tx_set) {
            return Ok(vec![(TxType::DctDct, Vec::new())]);
        }
        if is_inter {
            let paths =
                inter_tx_type_symbols(tx.tx_size, tx_set, eob, |row| self.arity(row).unwrap_or(0))
                    .map_err(rule("transform type"))?;
            let mut out: Vec<(TxType, Vec<Step>)> = Vec::new();
            for (tx_type, steps) in paths {
                let tx_type = TxType::from_value(tx_type)?;
                if out.iter().all(|&(seen, _)| seen != tx_type) {
                    out.push((tx_type, steps));
                }
            }
            if let Some(dct) = out
                .iter()
                .position(|&(tx_type, _)| tx_type == TxType::DctDct)
            {
                out[..=dct].rotate_right(1);
            }
            return Ok(out);
        }
        let tx_size_sqr = TX_SIZE_SQR
            .get(tx.tx_size)
            .and_then(|&sqr| usize::try_from(sqr).ok())
            .ok_or(EncoderSyntaxError::Unsupported("transform size"))?;
        let mut out: Vec<(TxType, Vec<Step>)> = Vec::new();
        let mut push = |tx_type: usize, steps: Vec<Step>| -> Result<()> {
            let tx_type = TxType::from_value(tx_type)?;
            if out.iter().all(|&(seen, _)| seen != tx_type) {
                out.push((tx_type, steps));
            }
            Ok(())
        };
        if matches!(
            tx_set,
            TX_SET_WIDE_64 | TX_SET_HIGH_64 | TX_SET_WIDE_32 | TX_SET_HIGH_32
        ) {
            let shape = long_tx_set_shape(tx_set);
            let row = TileCdfSelector::IntraTxTypeLong { tx_size_sqr };
            let arity = self.arity(row)?;
            for is_long_side_dct in [1, 0] {
                if shape.long_side_dct_is_forced && is_long_side_dct == 0 {
                    continue;
                }
                for index in 0..arity {
                    let mut steps = Vec::with_capacity(2);
                    if !shape.long_side_dct_is_forced {
                        steps.push((
                            TileCdfSelector::IsLongSideDct { is_inter: 0 },
                            is_long_side_dct,
                        ));
                    }
                    steps.push((row, index));
                    push(
                        long_tx_type_from_index(shape, is_long_side_dct, index),
                        steps,
                    )?;
                }
            }
        } else {
            let row = if tx_set == TX_SET_INTRA_2 {
                TileCdfSelector::IntraTxTypeSet2 { tx_size_sqr }
            } else {
                TileCdfSelector::IntraTxTypeSet1 { tx_size_sqr }
            };
            let context = self.planned_luma_context()?;
            for index in 0..self.arity(row)? {
                let tx_type = md_idx_luma_tx_type(tx.tx_size, context, index)
                    .map_err(rule("transform type"))?;
                push(tx_type, vec![(row, index)])?;
            }
        }
        Ok(out)
    }

    /// The number of symbols a CDF row codes.
    fn arity(&mut self, selector: TileCdfSelector) -> Result<usize> {
        Ok(self.tile_row(selector)?.len().saturating_sub(1))
    }
}
