// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The shared handle one frame publishes its AV2 § 7.9 motion field through.

use std::sync::Arc;

use splot_parallel::{CompletionCell, Condition};

use super::find_mv_stack::{
    MotionFieldLayout, TemporalMotionBand, TemporalMotionField, TemporalMotionFieldMetadata,
};

#[derive(Debug)]
struct MotionFieldPublication {
    layout: MotionFieldLayout,
    metadata: CompletionCell<Option<Arc<TemporalMotionFieldMetadata>>>,
    field: CompletionCell<Option<Arc<TemporalMotionField>>>,
    bands: Vec<CompletionCell<Option<TemporalMotionBand>>>,
}

/// One frame's § 7.9 temporal motion field, named before it is derived.
///
/// The canonical `PipelineFrame` owns this handle before reconstruction derives
/// the field. Reconstruction fills it exactly once; `RuntimeReferenceBuffer`
/// resolves it from the retained frame in `build_store`, and [`Self::field`]
/// fails closed rather than reporting an absent field as no motion.
#[derive(Clone, Debug)]
pub(crate) struct MotionFieldHandle(Arc<MotionFieldPublication>);

impl MotionFieldHandle {
    /// Names a field that is already derived.
    pub(crate) fn settled(field: TemporalMotionField) -> Self {
        let layout = field.layout();
        let metadata = Arc::new(field.metadata());
        let bands = field
            .clone()
            .into_bands()
            .into_iter()
            .map(|band| CompletionCell::completed(Some(band)))
            .collect();
        Self(Arc::new(MotionFieldPublication {
            layout,
            metadata: CompletionCell::completed(Some(metadata)),
            field: CompletionCell::completed(Some(Arc::new(field))),
            bands,
        }))
    }

    /// Names a pending field with enough geometry to create every row-band
    /// completion before its entropy pass starts.
    pub(crate) fn pending_with_layout(layout: MotionFieldLayout) -> Self {
        let bands = (0..layout.band_count())
            .map(|_| CompletionCell::new())
            .collect();
        Self(Arc::new(MotionFieldPublication {
            layout,
            metadata: CompletionCell::new(),
            field: CompletionCell::new(),
            bands,
        }))
    }

    /// Publishes the parse-derived semantic metadata independently of pixels.
    pub(crate) fn publish_metadata(&self, metadata: TemporalMotionFieldMetadata) {
        let _ = self.0.metadata.set(Some(Arc::new(metadata)));
    }

    /// Publishes the derived field, which every consumer then reads.
    ///
    /// A second publication is ignored: the first is the frame's field, and a
    /// handle is filled by exactly one reconstruction.
    pub(crate) fn publish(&self, field: TemporalMotionField) {
        self.publish_metadata(field.metadata());
        for (cell, band) in self.0.bands.iter().zip(field.clone().into_bands()) {
            let _ = cell.set(Some(band));
        }
        let _ = self.0.field.set(Some(Arc::new(field)));
    }

    /// Publishes one completed full-width source superblock row.
    pub(crate) fn publish_band(&self, index: usize, band: TemporalMotionBand) {
        if let Some(cell) = self.0.bands.get(index) {
            let _ = cell.set(Some(band));
        }
    }

    /// Rebuilds the terminal compatibility field after every band has landed.
    pub(crate) fn publish_whole_from_bands(&self) {
        let Some(metadata) = self
            .0
            .metadata
            .get()
            .and_then(Option::as_ref)
            .map(Arc::as_ref)
        else {
            let _ = self.0.field.set(None);
            return;
        };
        let bands = self
            .0
            .bands
            .iter()
            .map(|band| band.get().and_then(Option::as_ref).cloned())
            .collect::<Option<Vec<_>>>();
        let field = bands.and_then(|bands| {
            TemporalMotionField::from_bands(self.0.layout, metadata, bands).map(Arc::new)
        });
        let _ = self.0.field.set(field);
    }

    /// Publishes terminal failure so dependent scheduler jobs are released and
    /// fail closed instead of remaining stranded.
    pub(crate) fn fail(&self) {
        let _ = self.0.metadata.set(None);
        for band in &self.0.bands {
            let _ = band.set(None);
        }
        let _ = self.0.field.set(None);
    }

    /// Borrows the published field, or `None` while it is still owed.
    pub(crate) fn field(&self) -> Option<&Arc<TemporalMotionField>> {
        self.0.field.get().and_then(Option::as_ref)
    }

    /// Waits for terminal field publication while assisting the installed pool.
    pub(crate) fn wait_field(&self) {
        let _ = self.0.field.wait_with_pool_assist();
    }

    pub(crate) fn layout(&self) -> MotionFieldLayout {
        self.0.layout
    }

    pub(crate) fn metadata(&self) -> Option<&Arc<TemporalMotionFieldMetadata>> {
        self.0.metadata.get().and_then(Option::as_ref)
    }

    pub(crate) fn band(&self, index: usize) -> Option<&TemporalMotionBand> {
        self.0.bands.get(index)?.get().and_then(Option::as_ref)
    }

    pub(crate) fn band_publication(&self, index: usize) -> Option<&Option<TemporalMotionBand>> {
        self.0.bands.get(index)?.get()
    }

    pub(crate) fn metadata_condition(&self) -> Condition<'_> {
        Condition::completion(&self.0.metadata)
    }

    pub(crate) fn field_condition(&self) -> Condition<'_> {
        Condition::completion(&self.0.field)
    }

    pub(crate) fn band_condition(&self, index: usize) -> Option<Condition<'_>> {
        self.0.bands.get(index).map(Condition::completion)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settled_fields_always_publish_their_geometry_bands() -> Result<(), Box<dyn std::error::Error>>
    {
        let field = TemporalMotionField::new(40, 8).ok_or("motion field")?;
        let expected = field.clone();
        let layout = field.layout();
        let handle = MotionFieldHandle::settled(field);

        assert_eq!(handle.field().map(Arc::as_ref), Some(&expected));
        assert!(
            (0..layout.band_count())
                .all(|index| matches!(handle.band_publication(index), Some(Some(_))))
        );
        Ok(())
    }
}
