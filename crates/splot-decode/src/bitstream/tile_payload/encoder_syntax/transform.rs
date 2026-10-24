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
use splot_recon::SecondaryInverseTransform;

use super::super::CoeffBlock;
use super::super::general_intra_residual::reconstruct::resolve_secondary_inverse_transform;
use super::super::general_intra_residual::{
    IST_DIR_SIZE, IntraIstSyntax, LumaCoeffBlock, LumaTransformPartitionContext, TX_SET_HIGH_32,
    TX_SET_HIGH_64, TX_SET_INTRA_2, TX_SET_WIDE_32, TX_SET_WIDE_64, block_size_table_usize,
    dct_forced, inter_tx_type_symbols, intra_ist_most_probable_stx_selector, ist_eob_limit,
    long_tx_set_shape, long_tx_type_from_index, luma_transform_records_for_partition,
    luma_tx_partition_allows, md_idx_luma_tx_type, transform_set_from_flags,
};
use super::{
    EncoderSyntaxError, IntraTileSyntax, LeafBlock, Result, TransformBlock, TxType, rule, tx_log2,
};
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

/// `IST_DIR_SIZE` (§ 3): the `most_probable_stx_set` values.
pub const IST_SETS: u8 = IST_DIR_SIZE as u8;

/// `sec_tx_type` and `most_probable_stx_set` of an intra luma block (§ 5.20.8.2).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Ist {
    /// `sec_tx_type`, `0..=3`; `0` codes no secondary transform and ignores `set`.
    pub sec_tx_type: u8,
    /// `most_probable_stx_set`, below [`IST_SETS`].
    pub set: u8,
}

impl Ist {
    /// No secondary transform.
    pub const OFF: Self = Self {
        sec_tx_type: 0,
        set: 0,
    };

    /// The decoder's syntax of a coded `sec_tx_type`.
    pub(super) const fn syntax(self) -> Option<IntraIstSyntax> {
        if self.sec_tx_type == 0 {
            return None;
        }
        Some(IntraIstSyntax {
            sec_tx_type: self.sec_tx_type as usize,
            most_probable_stx_set: Some(self.set as usize),
        })
    }
}

/// One symbol: its CDF and its value.
type Step = (TileCdfSelector, usize);
/// The symbols of one choice, in order; inline so trials do not allocate.
type Steps = [Option<Step>; MAX_STEPS];
/// The most symbols one transform partition or type takes.
const MAX_STEPS: usize = 3;
const NO_STEPS: Steps = [None; MAX_STEPS];

/// `steps` inline.
fn inline_steps(steps: &[Step]) -> Result<Steps> {
    let mut out = NO_STEPS;
    if steps.len() > MAX_STEPS {
        return Err(EncoderSyntaxError::Unsupported(
            "transform type symbol count",
        ));
    }
    for (slot, &step) in out.iter_mut().zip(steps) {
        *slot = Some(step);
    }
    Ok(out)
}

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
        for (selector, value) in steps.into_iter().flatten() {
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
            [(tx_type, steps)] if steps.iter().all(Option::is_none) => Some(*tx_type),
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
        for (selector, value) in steps.into_iter().flatten() {
            put(self.tile_row(selector)?, value)?;
        }
        if !self.ist_coded(tx, eob)? {
            if tx.ist.sec_tx_type != 0 {
                return Err(EncoderSyntaxError::Unsupported("IST for this block and eob").into());
            }
            return Ok(());
        }
        let ist = tx.ist;
        if ist.sec_tx_type > 3 || (ist.sec_tx_type > 0 && ist.set >= self.ist_sets(tx)?) {
            return Err(EncoderSyntaxError::Unsupported("IST type or set").into());
        }
        let row = TileCdfSelector::SecTxType {
            is_inter: 0,
            tx_size_sqr: tx_size_sqr(tx.tx_size)?,
        };
        put(self.tile_row(row)?, ist.sec_tx_type.into())?;
        if ist.sec_tx_type > 0 {
            put(self.tile_row(Self::ist_set_row(tx)?)?, ist.set.into())?;
        }
        Ok(())
    }

    /// The `most_probable_stx_set` values luma block `tx` can code.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for an invalid transform size.
    pub fn ist_sets(&mut self, tx: &TransformBlock) -> Result<u8> {
        let row = Self::ist_set_row(tx)?;
        u8::try_from(self.arity(row)?).map_err(|_| EncoderSyntaxError::Unsupported("IST sets"))
    }

    fn ist_set_row(tx: &TransformBlock) -> Result<TileCdfSelector> {
        intra_ist_most_probable_stx_selector(tx.tx_size, tx.tx_type as usize)
            .map_err(rule("IST set row"))
    }

    /// Whether intra luma block `tx` with `eob` codes `sec_tx_type` (§ 5.20.8.2).
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] without a planned leaf.
    pub fn ist_coded(&self, tx: &TransformBlock, eob: usize) -> Result<bool> {
        if !self.tools.enable_intra_ist
            || tx.plane != 0
            || eob <= 1
            || self.leaf_is_inter()
            || !matches!(tx.tx_type, TxType::DctDct | TxType::AdstAdst)
            || self.planned_luma_context()?.y_mode().is_paeth()
        {
            return Ok(false);
        }
        Ok(eob <= ist_eob_limit(tx.tx_size, tx.tx_type as usize).map_err(rule("IST limit"))?)
    }

    /// The decoder's § 7.15.3 secondary transform of `tx` in the planned leaf;
    /// `None` without one.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] without a planned leaf or for an IST
    /// `tx` cannot code.
    pub fn secondary_transform(
        &self,
        tx: &TransformBlock,
    ) -> Result<Option<SecondaryInverseTransform>> {
        let Some(ist) = tx.ist.syntax() else {
            return Ok(None);
        };
        let block = LumaCoeffBlock {
            intra_ist: Some(ist),
            ..LumaCoeffBlock::empty(tx.tx_type as usize, false)
        };
        let (log2_w, log2_h) = tx_log2(tx.tx_size)?;
        resolve_secondary_inverse_transform(
            CoeffBlock::new(&block, &[]).map_err(rule("IST block"))?,
            log2_w,
            log2_h,
            self.tools.bit_depth,
            Some(self.planned_luma_context()?),
        )
        .map_err(rule("IST parameters"))
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
            Some(steps) if steps.iter().all(Option::is_none) => Ok(None),
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
    ) -> Result<Option<Steps>> {
        let Some((context, (allow_horz, allow_vert))) = self.partition_context(leaf)? else {
            return Ok((partition == TxPartition::None).then_some(NO_STEPS));
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
        let mut steps = NO_STEPS;
        steps[0] = Some((
            TileCdfSelector::TxDoPartition {
                fsc_mode: 0,
                is_inter: 0,
                txfm_split_group: group(
                    &SIZE_TO_TX_PART_GROUP_LOOKUP,
                    "Size_To_Tx_Part_Group_Lookup",
                )?,
            },
            usize::from(partition != TxPartition::None),
        ));
        if partition == TxPartition::None {
            return Ok(Some(steps));
        }
        if allow_horz && allow_vert {
            let Some(symbol) =
                (0..=u8::MAX).find(|&s| TxPartition::from_partition_type_symbol(s) == partition)
            else {
                return Ok(None);
            };
            steps[1] = Some((
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
            steps[1] = Some((
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
    ) -> Result<Vec<(TxType, Steps)>> {
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
            return Ok(vec![(TxType::DctDct, NO_STEPS)]);
        }
        if is_inter {
            let mut row_error = None;
            let paths = inter_tx_type_symbols(tx.tx_size, tx_set, eob, |row| {
                self.arity(row).unwrap_or_else(|error| {
                    row_error.get_or_insert(error);
                    0
                })
            })
            .map_err(rule("transform type"))?;
            if let Some(error) = row_error {
                return Err(error);
            }
            let mut out: Vec<(TxType, Steps)> = Vec::new();
            for (tx_type, steps) in paths {
                let tx_type = TxType::from_value(tx_type)?;
                if out.iter().all(|&(seen, _)| seen != tx_type) {
                    out.push((tx_type, inline_steps(&steps)?));
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
        let tx_size_sqr = tx_size_sqr(tx.tx_size)?;
        let mut out: Vec<(TxType, Steps)> = Vec::new();
        let mut push = |tx_type: usize, steps: Steps| -> Result<()> {
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
                    let long_side = (
                        TileCdfSelector::IsLongSideDct { is_inter: 0 },
                        is_long_side_dct,
                    );
                    let steps = if shape.long_side_dct_is_forced {
                        [Some((row, index)), None, None]
                    } else {
                        [Some(long_side), Some((row, index)), None]
                    };
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
                push(tx_type, [Some((row, index)), None, None])?;
            }
        }
        Ok(out)
    }

    /// The number of symbols a CDF row codes.
    fn arity(&mut self, selector: TileCdfSelector) -> Result<usize> {
        Ok(self.tile_row(selector)?.len().saturating_sub(1))
    }
}

/// `Tx_Size_Sqr[tx_size]`.
fn tx_size_sqr(tx_size: usize) -> Result<usize> {
    TX_SIZE_SQR
        .get(tx_size)
        .and_then(|&sqr| usize::try_from(sqr).ok())
        .ok_or(EncoderSyntaxError::Unsupported("transform size"))
}
