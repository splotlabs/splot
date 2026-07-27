// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The shared handle one frame publishes its AV2 § 7.9 motion field through.

use std::sync::{Arc, OnceLock};

use super::TemporalMotionField;

/// One frame's § 7.9 temporal motion field, named before it is derived.
///
/// A frame's reference update is recorded from its header, while the field
/// itself lands at the end of that frame's reconstruction, which a pipelined
/// driver runs after the update. The handle bridges the two: the update stores
/// it unfilled, the reconstruction fills it exactly once, and a later frame's
/// temporal prelude reads it through [`Self::field`], which fails closed rather
/// than reporting an absent field as no motion.
#[derive(Clone, Debug)]
pub(crate) struct MotionFieldHandle(Arc<OnceLock<Arc<TemporalMotionField>>>);

impl MotionFieldHandle {
    /// Names a field that is already derived.
    pub(crate) fn settled(field: TemporalMotionField) -> Self {
        let cell = OnceLock::new();
        let _ = cell.set(Arc::new(field));
        Self(Arc::new(cell))
    }

    /// Names a field whose frame has not reconstructed yet.
    pub(crate) fn pending() -> Self {
        Self(Arc::new(OnceLock::new()))
    }

    /// Publishes the derived field, which every consumer then reads.
    ///
    /// A second publication is ignored: the first is the frame's field, and a
    /// handle is filled by exactly one reconstruction.
    pub(crate) fn publish(&self, field: TemporalMotionField) {
        let _ = self.0.set(Arc::new(field));
    }

    /// Borrows the published field, or `None` while it is still owed.
    pub(crate) fn field(&self) -> Option<&Arc<TemporalMotionField>> {
        self.0.get()
    }
}
