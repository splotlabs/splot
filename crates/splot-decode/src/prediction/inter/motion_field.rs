// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The shared handle one frame publishes its AV2 § 7.9 motion field through.

use parking_lot::Mutex;
use std::sync::Arc;

use splot_parallel::{CompletionCell, Condition};

use super::find_mv_stack::{
    MotionFieldLayout, TemporalMotionBand, TemporalMotionBlock, TemporalMotionField,
    TemporalMotionFieldMetadata,
};

#[derive(Debug)]
struct MotionFieldPublication {
    layout: MotionFieldLayout,
    metadata: CompletionCell<Option<TemporalMotionFieldMetadata>>,
    field: CompletionCell<Option<Arc<TemporalMotionField>>>,
    bands: Vec<MotionBandSlot>,
    spare_field: Mutex<Option<Arc<TemporalMotionField>>>,
}

#[derive(Debug)]
struct MotionBandSlot {
    ready: CompletionCell<Option<TemporalMotionBand>>,
    building: Mutex<Option<TemporalMotionBand>>,
}

impl MotionBandSlot {
    fn new() -> Self {
        Self {
            ready: CompletionCell::new(),
            building: Mutex::new(None),
        }
    }
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
    #[cfg(test)]
    pub(crate) fn owner_count(&self) -> usize {
        Arc::strong_count(&self.0)
    }

    /// Names a field that is already derived.
    pub(crate) fn settled(field: TemporalMotionField) -> Self {
        let layout = field.layout();
        let metadata = field.metadata();
        let field = Arc::new(field);
        let mut bands = Vec::with_capacity(layout.band_count());
        TemporalMotionField::shared_bands(&field, |band| {
            bands.push(MotionBandSlot {
                ready: CompletionCell::completed(Some(band)),
                building: Mutex::new(None),
            });
        });
        Self(Arc::new(MotionFieldPublication {
            layout,
            metadata: CompletionCell::completed(Some(metadata)),
            field: CompletionCell::completed(Some(field)),
            bands,
            spare_field: Mutex::new(None),
        }))
    }

    /// Names a pending field with enough geometry to create every row-band
    /// completion before its entropy pass starts.
    pub(crate) fn pending_with_layout(layout: MotionFieldLayout) -> Self {
        let bands = (0..layout.band_count())
            .map(|_| MotionBandSlot::new())
            .collect();
        Self(Arc::new(MotionFieldPublication {
            layout,
            metadata: CompletionCell::new(),
            field: CompletionCell::new(),
            bands,
            spare_field: Mutex::new(Some(Arc::new(TemporalMotionField::empty()))),
        }))
    }

    /// Publishes the parse-derived semantic metadata independently of pixels.
    pub(crate) fn publish_metadata(&self, metadata: TemporalMotionFieldMetadata) {
        let _ = self.0.metadata.set(Some(metadata));
    }

    /// Publishes the derived field, which every consumer then reads.
    ///
    /// A second publication is ignored: the first is the frame's field, and a
    /// handle is filled by exactly one reconstruction.
    pub(crate) fn publish(&self, field: TemporalMotionField) {
        if self.0.field.get().is_some() {
            return;
        }
        self.publish_metadata(field.metadata());
        let Some(mut shared) = self.0.spare_field.lock().take() else {
            self.fail();
            return;
        };
        let Some(storage) = Arc::get_mut(&mut shared) else {
            *self.0.spare_field.lock() = Some(shared);
            self.fail();
            return;
        };
        *storage = field;
        let mut cells = self.0.bands.iter().take(self.0.layout.band_count());
        TemporalMotionField::shared_bands(&shared, |band| {
            if let Some(cell) = cells.next() {
                let _ = cell.ready.set(Some(band));
            }
        });
        let _ = self.0.field.set(Some(shared));
    }

    pub(crate) fn begin_bands(&self, metadata: &TemporalMotionFieldMetadata) -> crate::Result<()> {
        self.publish_metadata(metadata.clone());
        for (index, slot) in self
            .0
            .bands
            .iter()
            .take(self.0.layout.band_count())
            .enumerate()
        {
            let mut building = slot.building.lock();
            let band =
                building.get_or_insert_with(|| TemporalMotionBand::vacant(self.0.layout, metadata));
            band.reset(self.0.layout, metadata, index)?;
        }
        Ok(())
    }

    pub(crate) fn fold_band(
        &self,
        index: usize,
        records: &[TemporalMotionBlock],
    ) -> crate::Result<()> {
        let slot = self
            .0
            .bands
            .get(index)
            .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
        let mut building = slot.building.lock();
        let band = building
            .as_mut()
            .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
        band.record_blocks(records)
    }

    pub(crate) fn publish_builder_band(&self, index: usize) {
        if let Some(slot) = self.0.bands.get(index)
            && let Some(band) = slot.building.lock().take()
        {
            let _ = slot.ready.set(Some(band.into_shared()));
        }
    }

    /// Publishes the terminal view over the resident band buffers.
    pub(crate) fn publish_whole_from_bands(&self) {
        if self.0.field.get().is_some() {
            return;
        }
        let Some(metadata) = self.metadata() else {
            self.fail();
            return;
        };
        if (0..self.0.layout.band_count()).any(|index| self.band(index).is_none()) {
            self.fail();
            return;
        }
        let Some(mut field) = self.0.spare_field.lock().take() else {
            self.fail();
            return;
        };
        let valid = Arc::get_mut(&mut field).is_some_and(|storage| {
            storage.reset_from_bands(
                self.0.layout,
                metadata,
                (0..self.0.layout.band_count()).filter_map(|index| self.band(index).cloned()),
            )
        });
        if valid {
            let _ = self.0.field.set(Some(field));
        } else {
            *self.0.spare_field.lock() = Some(field);
            self.fail();
        }
    }

    /// Reclaims internal views before checking independent field and band readers.
    pub(crate) fn try_retire(&mut self) -> bool {
        let Some(publication) = Arc::get_mut(&mut self.0) else {
            return false;
        };
        if let Some(field) = publication.field.get_mut().and_then(Option::take) {
            *publication.spare_field.get_mut() = Some(field);
        }
        for slot in &mut publication.bands {
            if let Some(band) = slot.ready.get_mut().and_then(Option::take)
                && !band.is_field_view()
            {
                *slot.building.get_mut() = Some(band);
            }
            slot.ready.reset();
        }
        if let Some(field) = publication.spare_field.get_mut().as_mut() {
            let Some(field) = Arc::get_mut(field) else {
                return false;
            };
            field.retire_bands();
        }
        if publication.bands.iter_mut().any(|slot| {
            slot.building
                .get_mut()
                .as_mut()
                .is_some_and(|band| !band.is_exclusive())
        }) {
            return false;
        }
        publication.field.reset();
        publication.metadata.reset();
        true
    }

    pub(crate) fn reset_layout(&mut self, layout: MotionFieldLayout) -> crate::Result<()> {
        let publication = Arc::get_mut(&mut self.0)
            .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
        publication.layout = layout;
        if publication.bands.len() < layout.band_count() {
            publication
                .bands
                .resize_with(layout.band_count(), MotionBandSlot::new);
        }
        Ok(())
    }

    /// Publishes terminal failure so dependent scheduler jobs are released and
    /// fail closed instead of remaining stranded.
    pub(crate) fn fail(&self) {
        let _ = self.0.metadata.set(None);
        for band in self.0.bands.iter().take(self.0.layout.band_count()) {
            let _ = band.ready.set(None);
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

    pub(crate) fn metadata(&self) -> Option<&TemporalMotionFieldMetadata> {
        self.0.metadata.get().and_then(Option::as_ref)
    }

    pub(crate) fn band(&self, index: usize) -> Option<&TemporalMotionBand> {
        self.0
            .bands
            .get(index)?
            .ready
            .get()
            .and_then(Option::as_ref)
    }

    pub(crate) fn band_publication(&self, index: usize) -> Option<&Option<TemporalMotionBand>> {
        self.0.bands.get(index)?.ready.get()
    }

    pub(crate) fn metadata_condition(&self) -> Condition<'_> {
        Condition::completion(&self.0.metadata)
    }

    pub(crate) fn field_condition(&self) -> Condition<'_> {
        Condition::completion(&self.0.field)
    }

    pub(crate) fn band_condition(&self, index: usize) -> Option<Condition<'_>> {
        self.0
            .bands
            .get(index)
            .filter(|_| index < self.0.layout.band_count())
            .map(|slot| Condition::completion(&slot.ready))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resident_fields_wait_for_independent_readers_before_reuse()
    -> Result<(), Box<dyn std::error::Error>> {
        let field = TemporalMotionField::new(40, 8).ok_or("field")?;
        let layout = field.layout();
        let metadata = field.metadata();
        let mut handle = MotionFieldHandle::pending_with_layout(layout);
        let publication = Arc::as_ptr(&handle.0);
        let slots = handle.0.bands.as_ptr();
        let mut terminal = None;
        for _ in 0..1200 {
            handle.begin_bands(&metadata)?;
            for index in 0..layout.band_count() {
                handle.publish_builder_band(index);
            }
            handle.publish_whole_from_bands();
            let reader = handle.field().ok_or("published field")?.clone();
            let address = Arc::as_ptr(&reader);
            assert_eq!(*terminal.get_or_insert(address), address);
            let band = handle.band(0).ok_or("published band")?.clone();
            let saved = reader.clone();
            assert!(!handle.try_retire());
            assert_eq!(reader.as_ref(), saved.as_ref());
            drop(saved);
            drop(reader);
            assert!(!handle.try_retire());
            drop(band);
            assert!(handle.try_retire());
            handle.reset_layout(layout)?;
            assert!(handle.metadata().is_none());
            assert_eq!(Arc::as_ptr(&handle.0), publication);
            assert_eq!(handle.0.bands.as_ptr(), slots);
        }
        let smaller = MotionFieldLayout::new(4, 4, 16).ok_or("smaller layout")?;
        handle.reset_layout(smaller)?;
        handle.begin_bands(&metadata)?;
        for index in 0..smaller.band_count() {
            handle.publish_builder_band(index);
        }
        handle.publish_whole_from_bands();
        assert_eq!(handle.field().ok_or("smaller field")?.layout(), smaller);
        assert!(handle.band_condition(smaller.band_count()).is_none());
        assert!(handle.try_retire());
        handle.reset_layout(layout)?;
        handle.begin_bands(&metadata)?;
        handle.fail();
        assert!(handle.try_retire());
        Ok(())
    }

    #[test]
    fn settled_fields_always_publish_their_geometry_bands() -> Result<(), Box<dyn std::error::Error>>
    {
        let field = TemporalMotionField::new(40, 8).ok_or("motion field")?;
        let expected = field.clone();
        let layout = field.layout();
        let handle = MotionFieldHandle::settled(field);

        assert_eq!(handle.field().map(Arc::as_ref), Some(&expected));
        assert_eq!(handle.metadata(), Some(&expected.metadata()));
        assert!(
            (0..layout.band_count())
                .all(|index| matches!(handle.band_publication(index), Some(Some(_))))
        );
        Ok(())
    }
}
