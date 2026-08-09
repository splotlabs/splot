// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Bounded luma transform-partition units.

use super::{
    GeneralIntraResidualError, TransformPartitionUnsupported, unsupported_transform_partition,
};

// AV2 § 5.20.6.3 (`docs/spec/av2/1.0.0/05-syntax-structures.md`) emits at most five units.
pub(super) const MAX_LUMA_TRANSFORM_PARTITION_UNITS: usize = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LumaTransformPartitionContext {
    pub(super) mi_size: usize,
}

impl LumaTransformPartitionContext {
    #[must_use]
    pub(crate) const fn new(mi_size: usize) -> Self {
        Self { mi_size }
    }
}

pub(crate) struct LumaTransformPartitionUnits<T> {
    entries: [Option<T>; MAX_LUMA_TRANSFORM_PARTITION_UNITS],
    len: usize,
}

impl<T> LumaTransformPartitionUnits<T> {
    pub(crate) fn new() -> Self {
        Self {
            entries: core::array::from_fn(|_| None),
            len: 0,
        }
    }

    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
        self.entries[..self.len].iter().flatten()
    }

    pub(crate) fn push(&mut self, value: T) -> Result<(), GeneralIntraResidualError> {
        let entry = self.entries.get_mut(self.len).ok_or_else(|| {
            unsupported_transform_partition(TransformPartitionUnsupported::RecordCapacity)
        })?;
        *entry = Some(value);
        self.len += 1;
        Ok(())
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
