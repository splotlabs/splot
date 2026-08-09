// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Terminal handles for entropy products named by provisional reference updates.

use std::sync::Arc;

use splot_parallel::{CompletionCell, Condition};

use crate::bitstream::tile_payload::{FrameCdfSubset, FrameSegmentIdMap};
use crate::filters::ccso::CcsoUnitGrid;

/// One entropy product that may be named before its parse job publishes it.
#[derive(Clone, Debug)]
pub(crate) struct EntropyProductHandle<T>(Arc<CompletionCell<Option<T>>>);

impl<T> EntropyProductHandle<T> {
    /// Names a product that is already available.
    pub(crate) fn settled(product: T) -> Self {
        Self(Arc::new(CompletionCell::completed(Some(product))))
    }

    /// Names a product whose entropy pass has not completed yet.
    pub(crate) fn pending() -> Self {
        Self(Arc::new(CompletionCell::new()))
    }

    /// Publishes the product exactly once.
    pub(crate) fn publish(&self, product: T) {
        let _ = self.0.set(Some(product));
    }

    /// Publishes terminal failure so dependent jobs are never stranded.
    pub(crate) fn fail(&self) {
        let _ = self.0.set(None);
    }

    /// Borrows the product after successful publication.
    pub(crate) fn product(&self) -> Option<&T> {
        self.0.get().and_then(Option::as_ref)
    }

    /// Admits a consumer after publication or failure.
    pub(crate) fn condition(&self) -> Condition<'_> {
        Condition::Completion(self.0.as_ref())
    }
}

pub(crate) type FrameCdfHandle = EntropyProductHandle<Arc<FrameCdfSubset>>;
pub(crate) type CcsoGridHandle = EntropyProductHandle<Option<Arc<CcsoUnitGrid>>>;
pub(crate) type SegmentIdMapHandle = EntropyProductHandle<Arc<FrameSegmentIdMap>>;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn publication_and_failure_are_terminal() {
        let published = FrameCdfHandle::pending();
        let cdfs = Arc::new(FrameCdfSubset::from_defaults());
        published.publish(Arc::clone(&cdfs));
        assert!(Arc::ptr_eq(published.product().unwrap(), &cdfs));

        let failed = FrameCdfHandle::pending();
        failed.fail();
        assert!(failed.product().is_none());
    }

    #[test]
    fn first_terminal_result_wins() {
        let handle = CcsoGridHandle::pending();
        handle.fail();
        handle.publish(None);
        assert!(handle.product().is_none());
    }
}
