// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Bounded luma transform-partition units.

use splot_core::tables::conversion::MAX_TX_SIZE_RECT;

use super::BlockSize;

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

    pub(super) const fn block_size(self) -> BlockSize {
        self.block_size
    }

    pub(super) fn max_tx_size(self) -> usize {
        MAX_TX_SIZE_RECT[self.block_size.index()] as usize
    }
}

#[derive(Debug)]
pub(crate) struct LumaTransformPartitionUnits<T> {
    entries: [Option<T>; MAX_LUMA_TRANSFORM_PARTITION_UNITS],
}

impl<T> LumaTransformPartitionUnits<T> {
    pub(crate) fn one(value: T) -> Self {
        Self {
            entries: [Some(value), None, None, None, None],
        }
    }

    pub(crate) fn two(values: [T; 2]) -> Self {
        let [first, second] = values;
        Self {
            entries: [Some(first), Some(second), None, None, None],
        }
    }

    pub(crate) fn four(values: [T; 4]) -> Self {
        let [first, second, third, fourth] = values;
        Self {
            entries: [Some(first), Some(second), Some(third), Some(fourth), None],
        }
    }

    pub(crate) fn five(values: [T; 5]) -> Self {
        let [first, second, third, fourth, fifth] = values;
        Self {
            entries: [
                Some(first),
                Some(second),
                Some(third),
                Some(fourth),
                Some(fifth),
            ],
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.iter().flatten().count()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().flatten()
    }

    pub(crate) fn try_filter_map<U, E>(
        self,
        mut map: impl FnMut(T) -> Result<Option<U>, E>,
    ) -> Result<LumaTransformPartitionUnits<U>, E> {
        let mut entries = core::array::from_fn(|_| None);
        for (index, entry) in self.entries.into_iter().enumerate() {
            if let Some(value) = entry {
                entries[index] = map(value)?;
            }
        }
        Ok(LumaTransformPartitionUnits { entries })
    }

    pub(crate) fn try_map<U, E>(
        self,
        mut map: impl FnMut(T) -> Result<U, E>,
    ) -> Result<LumaTransformPartitionUnits<U>, E> {
        self.try_filter_map(|value| map(value).map(Some))
    }
}

impl<T> IntoIterator for LumaTransformPartitionUnits<T> {
    type Item = T;
    type IntoIter =
        core::iter::Flatten<core::array::IntoIter<Option<T>, MAX_LUMA_TRANSFORM_PARTITION_UNITS>>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter().flatten()
    }
}
