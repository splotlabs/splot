// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Bounded luma transform-partition units.

use splot_core::tables::conversion::{MAX_TX_SIZE_RECT, TX_HEIGHT, TX_WIDTH};

use super::{
    BlockSize, GeneralIntraResidualError, LumaTransformPartitionRecord, LumaTxPartition,
    invalid_reconstruction_state_error,
};

// AV2 § 5.20.6.3 (`docs/spec/av2/1.0.0/05-syntax-structures.md`) emits at most five units.
pub(super) const MAX_LUMA_TRANSFORM_PARTITION_UNITS: usize = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LumaTransformPartitionContext {
    block_size: BlockSize,
}

impl LumaTransformPartitionContext {
    #[must_use]
    pub(crate) const fn new(block_size: BlockSize) -> Self {
        Self { block_size }
    }

    pub(crate) const fn block_size(self) -> BlockSize {
        self.block_size
    }

    pub(crate) fn max_tx_size(self) -> usize {
        MAX_TX_SIZE_RECT[self.block_size.index()] as usize
    }
}

#[derive(Clone, Copy, Debug)]
struct Unit {
    x: u8,
    y: u8,
    tx_size: u8,
    middle: bool,
}

type Layout = Result<([Unit; MAX_LUMA_TRANSFORM_PARTITION_UNITS], u8), [u8; 2]>;

// (x, y, width, height) in quarters of the parent transform, in syntax order.
const PARTITIONS: [&[[u8; 4]]; 8] = [
    &[[0, 0, 4, 4]],
    &[[0, 0, 2, 2], [2, 0, 2, 2], [0, 2, 2, 2], [2, 2, 2, 2]],
    &[[0, 0, 4, 2], [0, 2, 4, 2]],
    &[[0, 0, 2, 4], [2, 0, 2, 4]],
    &[[0, 0, 4, 1], [0, 1, 4, 1], [0, 2, 4, 1], [0, 3, 4, 1]],
    &[[0, 0, 1, 4], [1, 0, 1, 4], [2, 0, 1, 4], [3, 0, 1, 4]],
    &[
        [0, 0, 2, 1],
        [2, 0, 2, 1],
        [0, 1, 4, 2],
        [0, 3, 2, 1],
        [2, 3, 2, 1],
    ],
    &[
        [0, 0, 1, 2],
        [0, 2, 1, 2],
        [1, 0, 2, 4],
        [3, 0, 1, 2],
        [3, 2, 1, 2],
    ],
];

const fn layout(tx: usize, partition: usize) -> Layout {
    let shapes = PARTITIONS[partition];
    let mut units = [Unit {
        x: 0,
        y: 0,
        tx_size: 0,
        middle: false,
    }; MAX_LUMA_TRANSFORM_PARTITION_UNITS];
    let w4 = TX_WIDTH[tx] as u8 / 4;
    let h4 = TX_HEIGHT[tx] as u8 / 4;
    let mut i = 0;
    while i < shapes.len() {
        let [x, y, width_quarters, height_quarters] = shapes[i];
        let width = (w4 * width_quarters) & !3;
        let height = (h4 * height_quarters) & !3;
        if width == 0 || height == 0 {
            return Err([width, height]);
        }
        let Ok(tx_size) =
            splot_recon::tx_size_index(width.trailing_zeros(), height.trailing_zeros())
        else {
            return Err([width, height]);
        };
        units[i] = Unit {
            x: w4 * x,
            y: h4 * y,
            tx_size: tx_size as u8,
            middle: shapes.len() == 5 && i > 0,
        };
        i += 1;
    }
    Ok((units, shapes.len() as u8))
}

static LAYOUTS: [[Layout; PARTITIONS.len()]; TX_WIDTH.len()] = {
    let mut table = [[Err([0, 0]); PARTITIONS.len()]; TX_WIDTH.len()];
    let mut tx = 0;
    while tx < table.len() {
        let mut partition = 0;
        while partition < PARTITIONS.len() {
            table[tx][partition] = layout(tx, partition);
            partition += 1;
        }
        tx += 1;
    }
    table
};

#[derive(Clone, Debug)]
pub(crate) struct LumaTransformPartitionUnits {
    x: usize,
    y: usize,
    units: &'static [Unit],
}

impl LumaTransformPartitionUnits {
    pub(super) fn new(
        x: usize,
        y: usize,
        tx: usize,
        partition: LumaTxPartition,
    ) -> Result<Self, GeneralIntraResidualError> {
        let layouts = LAYOUTS
            .get(tx)
            .ok_or_else(|| invalid_reconstruction_state_error("transform size"))?;
        match &layouts[partition as usize] {
            Ok((units, len)) => Ok(Self {
                x,
                y,
                units: &units[..usize::from(*len)],
            }),
            Err([width, height]) => Err(
                GeneralIntraResidualError::InvalidTransformPartitionDimensions {
                    width: usize::from(*width),
                    height: usize::from(*height),
                },
            ),
        }
    }
}

impl Iterator for LumaTransformPartitionUnits {
    type Item = LumaTransformPartitionRecord;

    fn next(&mut self) -> Option<Self::Item> {
        let (unit, rest) = self.units.split_first()?;
        self.units = rest;
        Some(LumaTransformPartitionRecord {
            x: self.x + usize::from(unit.x),
            y: self.y + usize::from(unit.y),
            tx_size: usize::from(unit.tx_size),
            middle: unit.middle,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.units.iter().size_hint()
    }
}

impl ExactSizeIterator for LumaTransformPartitionUnits {}
