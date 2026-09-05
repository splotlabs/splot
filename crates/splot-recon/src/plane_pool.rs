// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Spare storage for the frame-sized plane buffers of one decode.
//!
//! A frame holds three workspaces at once: the reconstruction target, the
//! sealed copy the deblock frontier reads, and the buffer the filter stages
//! publish into. None can be handed on by its creator: above a frame-pipelining
//! depth of one the last owner is whichever filter job finishes last, which is
//! not known until it does, so a refcounted owner is what the pipeline needs.
//!
//! One pool serves one decode, so concurrent decodes neither compete for each
//! other's spares nor thrash them against a different frame size.

use std::sync::Mutex;
/// Spare buffers held per sample depth.
///
/// One frame's worth of planes is three buffers, and a decode keeps at most a
/// few frames' workspaces in flight per depth, so this bounds the retained
/// storage at a handful of frames while still covering the deepest pipeline.
const MAX_SPARE_BUFFERS: usize = 24;

/// Buffers below this are tile- or test-sized rather than frame-sized, and
/// pooling them would only crowd out the ones worth keeping.
const MIN_POOLED_SAMPLES: usize = 1 << 14;

/// The plane storage one decode's retired workspaces leave for its next ones.
#[derive(Default)]
pub struct PlanePool {
    eight: Mutex<Vec<Vec<u8>>>,
    ten: Mutex<Vec<Vec<u16>>>,
}

impl core::fmt::Debug for PlanePool {
    /// Names the pool without locking it or printing a frame of samples.
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("PlanePool")
    }
}

impl PlanePool {
    /// Takes a spare buffer holding at least `samples`, or an empty one.
    ///
    /// A spare that is already `samples` long is worth much more than one that
    /// is merely large enough: the caller's `resize` then writes nothing. See
    /// [`Self::recycle`].
    pub(crate) fn take<T: PooledPlaneSamples>(&self, samples: usize) -> Vec<T> {
        let Ok(mut spares) = T::spares(self).lock() else {
            return Vec::new();
        };
        spares
            .iter()
            .position(|spare| spare.len() == samples)
            .or_else(|| spares.iter().position(|spare| spare.capacity() >= samples))
            .map_or_else(Vec::new, |index| spares.swap_remove(index))
    }

    /// Offers `buffer` back, keeping it only while there is room.
    ///
    /// The samples stay in place rather than being cleared. A workspace writes
    /// every sample it later reads -- that is what makes the retiring hand-off
    /// this pool serves alongside sound -- so clearing here would only add a
    /// multi-megabyte `resize` fill to the next frame that takes the buffer.
    pub(crate) fn recycle<T: PooledPlaneSamples>(&self, buffer: Vec<T>) {
        if buffer.capacity() < MIN_POOLED_SAMPLES {
            return;
        }
        if let Ok(mut spares) = T::spares(self).lock()
            && spares.len() < MAX_SPARE_BUFFERS
        {
            spares.push(buffer);
        }
    }
}

/// Selects the pool half holding this sample depth's spares.
///
/// Rust has neither generic fields nor generic statics, and [`crate::ReconSample`]
/// is sealed over exactly `u8` and `u16`, so this closed two-way dispatch is the
/// whole of the mechanism.
pub trait PooledPlaneSamples: Sized {
    #[doc(hidden)]
    fn spares(pool: &PlanePool) -> &Mutex<Vec<Vec<Self>>>;
}

impl PooledPlaneSamples for u8 {
    fn spares(pool: &PlanePool) -> &Mutex<Vec<Vec<Self>>> {
        &pool.eight
    }
}

impl PooledPlaneSamples for u16 {
    fn spares(pool: &PlanePool) -> &Mutex<Vec<Vec<Self>>> {
        &pool.ten
    }
}

#[cfg(test)]
mod tests {
    use super::{MIN_POOLED_SAMPLES, PlanePool};

    fn frame_sized() -> Vec<u16> {
        vec![0; MIN_POOLED_SAMPLES]
    }

    #[test]
    fn a_spare_returns_to_the_decode_that_retired_it() {
        let pool = PlanePool::default();
        pool.recycle(frame_sized());
        assert_eq!(
            pool.take::<u16>(MIN_POOLED_SAMPLES).len(),
            MIN_POOLED_SAMPLES
        );
    }

    #[test]
    fn concurrent_decodes_do_not_take_each_other_s_spares() {
        let decoding = PlanePool::default();
        let other_decode = PlanePool::default();
        decoding.recycle(frame_sized());

        assert!(
            other_decode.take::<u16>(MIN_POOLED_SAMPLES).is_empty(),
            "a second decode must not be served from the first one's spares"
        );
        assert_eq!(
            decoding.take::<u16>(MIN_POOLED_SAMPLES).len(),
            MIN_POOLED_SAMPLES,
            "and the first decode still has its own"
        );
    }

    #[test]
    fn the_two_sample_depths_do_not_share_storage() {
        let pool = PlanePool::default();
        pool.recycle(frame_sized());
        assert!(pool.take::<u8>(MIN_POOLED_SAMPLES).is_empty());
    }

    #[test]
    fn a_spare_of_the_wanted_length_is_preferred_over_a_merely_large_one() {
        let pool = PlanePool::default();
        let mut oversized = frame_sized();
        oversized.resize(MIN_POOLED_SAMPLES * 2, 0);
        pool.recycle(oversized);
        pool.recycle(frame_sized());

        assert_eq!(
            pool.take::<u16>(MIN_POOLED_SAMPLES).len(),
            MIN_POOLED_SAMPLES
        );
    }
}
