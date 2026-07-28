// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The driver's one-frame-deep parse/reconstruct overlap.
//!
//! On the pipelined path a frame's entropy pass runs as one worker task while
//! the driver reconstructs the previous frame. The two are independent: the
//! entropy pass reads no reference sample and no projected motion field, so the
//! task waits on nothing and the only thread that blocks is the driver, at the
//! scope join. The driver keeps at most one reconstruction owed, held here as a
//! [`PendingWalk`], and its bookkeeping runs between the two — the next frame's
//! header needs this frame's reference slot and CDFs, and both are entropy-pass
//! products.

use std::sync::Arc;

use crate::Result;
use crate::error::DecodeError;
use crate::prediction::inter;

use super::inflight::{FinishSpawner, PendingFinish};
use super::unsupported;

/// One frame whose entropy pass is done and whose reconstruction the driver
/// still owes, in the pipeline's active sample storage bit depth.
pub(super) enum PendingWalk {
    /// Eight-bit sample storage.
    Eight(inter::DeferredInterWalk<u8>, PendingFinish<u8>),
    /// Ten-bit sample storage.
    Ten(inter::DeferredInterWalk<u16>, PendingFinish<u16>),
}

impl PendingWalk {
    /// Runs the owed reconstruction and hands the frame's § 7.2 filter phase
    /// on, publishing the frame's motion field on the way.
    ///
    /// # Errors
    ///
    /// Returns the reconstruction's or an inline filter phase's diagnostic.
    pub(super) fn resume(
        self,
        spawner: &FinishSpawner<'_, '_>,
        scratch_eight: &mut inter::InterDecodeScratch<u8>,
        scratch_ten: &mut inter::InterDecodeScratch<u16>,
    ) -> Result<()> {
        let started = crate::timing::start();
        match self {
            Self::Eight(deferred, finish) => {
                let walked = deferred.reconstruct(scratch_eight)?;
                match spawner {
                    FinishSpawner::Deferred(scope) => finish.spawn_finish(walked, scope),
                    FinishSpawner::Inline => {
                        scratch_eight.recycle_frame_filter_records(finish.finish_inline(walked)?);
                    }
                }
            }
            Self::Ten(deferred, finish) => {
                let walked = deferred.reconstruct(scratch_ten)?;
                match spawner {
                    FinishSpawner::Deferred(scope) => finish.spawn_finish(walked, scope),
                    FinishSpawner::Inline => {
                        scratch_ten.recycle_frame_filter_records(finish.finish_inline(walked)?);
                    }
                }
            }
        }
        crate::timing::report("pass2_span", started);
        Ok(())
    }
}

/// Runs any owed reconstruction now, so the driver reaches a program point that
/// reads decoded samples with every frame before it complete.
///
/// # Errors
///
/// Returns the owed reconstruction's diagnostic.
pub(super) fn flush_pending(
    pending: &mut Option<PendingWalk>,
    spawner: &FinishSpawner<'_, '_>,
    scratch_eight: &mut inter::InterDecodeScratch<u8>,
    scratch_ten: &mut inter::InterDecodeScratch<u16>,
) -> Result<()> {
    match pending.take() {
        Some(owed) => owed.resume(spawner, scratch_eight, scratch_ten),
        None => Ok(()),
    }
}

/// Runs one frame's entropy pass beside the previous frame's reconstruction.
///
/// The entropy pass is spawned as a single ready task and the driver runs the
/// owed reconstruction itself, so the driver may steal the entropy task while
/// its own assisted waits donate to the pool. That is safe in one direction
/// only, and it is the direction this takes: the entropy pass waits on nothing,
/// while the reconstruction waits on reference rows the frames below it publish.
///
/// A reconstruction failure outranks an entropy-pass failure, since the frame it
/// belongs to decodes first.
///
/// # Errors
///
/// Returns the lower-indexed frame's diagnostic, or the scope's own when the
/// caller is not on a worker pool.
pub(super) fn parse_beside_pending<P: Send>(
    parse: impl FnOnce() -> P + Send,
    pending: Option<PendingWalk>,
    spawner: &FinishSpawner<'_, '_>,
    scratch_eight: &mut inter::InterDecodeScratch<u8>,
    scratch_ten: &mut inter::InterDecodeScratch<u16>,
) -> Result<P> {
    let Some(pending) = pending else {
        return Ok(parse());
    };
    let mut parsed = None;
    let mut resumed = None;
    let mut joined = None;
    let driver = std::thread::current().id();
    let stolen = std::sync::atomic::AtomicBool::new(false);
    let parsed_slot = &mut parsed;
    let resumed_slot = &mut resumed;
    let joined_slot = &mut joined;
    let stolen_flag = &stolen;
    splot_parallel::ready_task_scope(|scope| {
        scope.spawn(move |_| {
            if std::thread::current().id() == driver {
                stolen_flag.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            *parsed_slot = Some(parse());
        });
        *resumed_slot = Some(pending.resume(spawner, scratch_eight, scratch_ten));
        *joined_slot = crate::timing::start();
    })
    .map_err(|_| frame_task_scope())?;
    if joined.is_some() {
        crate::timing::report_detail(
            "parse_join_wait",
            joined,
            if stolen.load(std::sync::atomic::Ordering::Relaxed) {
                "on=driver"
            } else {
                "on=worker"
            },
        );
    }
    resumed.ok_or_else(frame_task_scope)??;
    parsed.ok_or_else(frame_task_scope)
}

/// Shares the driver's active sequence header with the frames it defers.
///
/// A deferred frame's reconstruction outlives the driver's borrow of the
/// header, and the header only ever changes at a frame the driver flushes
/// before, so the shared copy is made once per activation.
pub(super) fn shared_sequence(
    cached: &mut Option<Arc<splot_core::headers::sequence::SequenceHeader>>,
    sequence: &splot_core::headers::sequence::SequenceHeader,
) -> Arc<splot_core::headers::sequence::SequenceHeader> {
    Arc::clone(cached.get_or_insert_with(|| Arc::new(sequence.clone())))
}

fn frame_task_scope() -> DecodeError {
    unsupported(
        "frame_parse_task_scope",
        None,
        "internal invariant violation: a frame entropy pass task did not report an outcome",
    )
}
