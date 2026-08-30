// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::{compound_inter_post_round, round2_i32};
use std::simd::{Simd, cmp::SimdOrd, num::SimdInt};

pub(super) trait SubpelOutput<O> {
    fn one(&mut self, value: i32) -> O;

    fn sixteen(&mut self, values: Simd<i32, 16>, output: &mut [O]) {
        for (output, value) in output.iter_mut().zip(values.to_array()) {
            *output = self.one(value);
        }
    }

    fn eight(&mut self, values: Simd<i32, 8>, output: &mut [O]) {
        for (output, value) in output.iter_mut().zip(values.to_array()) {
            *output = self.one(value);
        }
    }

    fn four(&mut self, values: Simd<i32, 4>, output: &mut [O]) {
        for (output, value) in output.iter_mut().zip(values.to_array()) {
            *output = self.one(value);
        }
    }
}

pub(super) struct ScalarSubpelOutput<F>(pub(super) F);

impl<O, F: FnMut(i32) -> O> SubpelOutput<O> for ScalarSubpelOutput<F> {
    fn one(&mut self, value: i32) -> O {
        self.0(value)
    }
}

pub(super) struct ClippedU16SubpelOutput {
    pub(super) max_sample: i32,
}

impl ClippedU16SubpelOutput {
    #[allow(clippy::inline_always, reason = "direct u16 subpel output hot path")]
    #[inline(always)]
    fn clip<const LANES: usize>(&self, values: Simd<i32, LANES>) -> Simd<u16, LANES> {
        values
            .simd_max(Simd::splat(0))
            .simd_min(Simd::splat(self.max_sample))
            .cast()
    }
}

impl SubpelOutput<u16> for ClippedU16SubpelOutput {
    #[allow(clippy::inline_always, reason = "direct u16 subpel output hot path")]
    #[inline(always)]
    fn one(&mut self, value: i32) -> u16 {
        value.clamp(0, self.max_sample) as u16
    }

    #[allow(clippy::inline_always, reason = "direct u16 subpel output hot path")]
    #[inline(always)]
    fn sixteen(&mut self, values: Simd<i32, 16>, output: &mut [u16]) {
        output.copy_from_slice(&self.clip(values).to_array()); // splot-copy-ok: publish sixteen clipped SIMD prediction lanes
    }

    #[allow(clippy::inline_always, reason = "direct u16 subpel output hot path")]
    #[inline(always)]
    fn eight(&mut self, values: Simd<i32, 8>, output: &mut [u16]) {
        output.copy_from_slice(&self.clip(values).to_array()); // splot-copy-ok: publish eight clipped SIMD prediction lanes
    }

    #[allow(clippy::inline_always, reason = "direct u16 subpel output hot path")]
    #[inline(always)]
    fn four(&mut self, values: Simd<i32, 4>, output: &mut [u16]) {
        output.copy_from_slice(&self.clip(values).to_array()); // splot-copy-ok: publish four clipped SIMD prediction lanes
    }
}

pub(super) struct ClippedU8SubpelOutput;

impl ClippedU8SubpelOutput {
    #[allow(clippy::inline_always, reason = "direct u8 subpel output hot path")]
    #[inline(always)]
    fn clip<const LANES: usize>(values: Simd<i32, LANES>) -> Simd<u8, LANES> {
        values
            .simd_clamp(Simd::splat(0), Simd::splat(i32::from(u8::MAX)))
            .cast()
    }
}

impl SubpelOutput<u8> for ClippedU8SubpelOutput {
    #[allow(clippy::inline_always, reason = "direct u8 subpel output hot path")]
    #[inline(always)]
    fn one(&mut self, value: i32) -> u8 {
        value.clamp(0, i32::from(u8::MAX)) as u8
    }

    #[allow(clippy::inline_always, reason = "direct u8 subpel output hot path")]
    #[inline(always)]
    fn sixteen(&mut self, values: Simd<i32, 16>, output: &mut [u8]) {
        output.copy_from_slice(&Self::clip(values).to_array()); // splot-copy-ok: publish sixteen clipped SIMD prediction lanes
    }

    #[allow(clippy::inline_always, reason = "direct u8 subpel output hot path")]
    #[inline(always)]
    fn eight(&mut self, values: Simd<i32, 8>, output: &mut [u8]) {
        output.copy_from_slice(&Self::clip(values).to_array()); // splot-copy-ok: publish eight clipped SIMD prediction lanes
    }

    #[allow(clippy::inline_always, reason = "direct u8 subpel output hot path")]
    #[inline(always)]
    fn four(&mut self, values: Simd<i32, 4>, output: &mut [u8]) {
        output.copy_from_slice(&Self::clip(values).to_array()); // splot-copy-ok: publish four clipped SIMD prediction lanes
    }
}

pub(super) struct CompoundAverageSubpelOutput<'a> {
    pub(super) pred0: &'a [i32],
    pub(super) index: usize,
    pub(super) forward: i32,
    pub(super) backward: i32,
    pub(super) max_sample: i32,
}

#[allow(
    clippy::inline_always,
    reason = "measured compound-average subpel hot path"
)]
#[inline(always)]
fn blend_compound_average_lanes<const LANES: usize>(
    pred0: Simd<i32, LANES>,
    values: Simd<i32, LANES>,
    forward: i32,
    backward: i32,
) -> Simd<i32, LANES> {
    (pred0 * Simd::splat(forward)
        + values * Simd::splat(backward)
        + Simd::splat(1 << (3 + compound_inter_post_round())))
        >> (4 + compound_inter_post_round()) as i32
}

#[allow(
    clippy::inline_always,
    reason = "measured compound-average subpel hot path"
)]
#[inline(always)]
fn clip_blend_lanes<const LANES: usize>(
    blended: Simd<i32, LANES>,
    max_sample: i32,
) -> Simd<i32, LANES> {
    blended
        .simd_max(Simd::splat(0))
        .simd_min(Simd::splat(max_sample))
}

#[allow(
    clippy::inline_always,
    reason = "measured compound-average subpel hot path"
)]
#[inline(always)]
fn blend_compound_average_sample(
    pred0: i32,
    value: i32,
    forward: i32,
    backward: i32,
    max_sample: i32,
) -> i32 {
    round2_i32(
        forward * pred0 + backward * value,
        4 + compound_inter_post_round(),
    )
    .clamp(0, max_sample)
}

impl CompoundAverageSubpelOutput<'_> {
    #[cold]
    fn blend_cold<const LANES: usize>(&mut self, values: Simd<i32, LANES>) -> Simd<u16, LANES> {
        let pred0 = Simd::<i32, LANES>::from_slice(&self.pred0[self.index..]);
        self.index += LANES;
        clip_blend_lanes(
            blend_compound_average_lanes(pred0, values, self.forward, self.backward),
            self.max_sample,
        )
        .cast()
    }

    #[cold]
    fn one_cold(&mut self, value: i32) -> u16 {
        let pred0 = self.pred0[self.index];
        self.index += 1;
        blend_compound_average_sample(pred0, value, self.forward, self.backward, self.max_sample)
            as u16
    }

    #[allow(
        clippy::inline_always,
        reason = "measured compound-average subpel hot path"
    )]
    #[inline(always)]
    fn blend<const LANES: usize>(&mut self, values: Simd<i32, LANES>) -> Simd<u16, LANES> {
        if let Some(pred0) = self
            .pred0
            .get(self.index..)
            .and_then(|rest| rest.first_chunk::<LANES>())
        {
            self.index += LANES;
            return clip_blend_lanes(
                blend_compound_average_lanes(
                    Simd::from(*pred0),
                    values,
                    self.forward,
                    self.backward,
                ),
                self.max_sample,
            )
            .cast();
        }
        self.blend_cold(values)
    }
}

impl SubpelOutput<u16> for CompoundAverageSubpelOutput<'_> {
    #[allow(
        clippy::inline_always,
        reason = "measured compound-average subpel hot path"
    )]
    #[inline(always)]
    fn one(&mut self, value: i32) -> u16 {
        if let Some(&pred0) = self.pred0.get(self.index) {
            self.index += 1;
            return blend_compound_average_sample(
                pred0,
                value,
                self.forward,
                self.backward,
                self.max_sample,
            ) as u16;
        }
        self.one_cold(value)
    }

    #[allow(
        clippy::inline_always,
        reason = "measured compound-average subpel hot path"
    )]
    #[inline(always)]
    fn sixteen(&mut self, values: Simd<i32, 16>, output: &mut [u16]) {
        output.copy_from_slice(&self.blend(values).to_array()); // splot-copy-ok: publish sixteen blended SIMD prediction lanes
    }

    #[allow(
        clippy::inline_always,
        reason = "measured compound-average subpel hot path"
    )]
    #[inline(always)]
    fn eight(&mut self, values: Simd<i32, 8>, output: &mut [u16]) {
        output.copy_from_slice(&self.blend(values).to_array()); // splot-copy-ok: publish eight blended SIMD prediction lanes
    }

    #[allow(
        clippy::inline_always,
        reason = "measured compound-average subpel hot path"
    )]
    #[inline(always)]
    fn four(&mut self, values: Simd<i32, 4>, output: &mut [u16]) {
        output.copy_from_slice(&self.blend(values).to_array()); // splot-copy-ok: publish four blended SIMD prediction lanes
    }
}

pub(super) struct CompoundAverageSubpelOutputU8<'a> {
    pub(super) pred0: &'a [i32],
    pub(super) index: usize,
    pub(super) forward: i32,
    pub(super) backward: i32,
}

impl CompoundAverageSubpelOutputU8<'_> {
    #[cold]
    fn blend_cold<const LANES: usize>(&mut self, values: Simd<i32, LANES>) -> Simd<u8, LANES> {
        let pred0 = Simd::<i32, LANES>::from_slice(&self.pred0[self.index..]);
        self.index += LANES;
        clip_blend_lanes(
            blend_compound_average_lanes(pred0, values, self.forward, self.backward),
            i32::from(u8::MAX),
        )
        .cast()
    }

    #[cold]
    fn one_cold(&mut self, value: i32) -> u8 {
        let pred0 = self.pred0[self.index];
        self.index += 1;
        blend_compound_average_sample(
            pred0,
            value,
            self.forward,
            self.backward,
            i32::from(u8::MAX),
        ) as u8
    }

    #[allow(
        clippy::inline_always,
        reason = "measured compound-average subpel hot path"
    )]
    #[inline(always)]
    fn blend<const LANES: usize>(&mut self, values: Simd<i32, LANES>) -> Simd<u8, LANES> {
        if let Some(pred0) = self
            .pred0
            .get(self.index..)
            .and_then(|rest| rest.first_chunk::<LANES>())
        {
            self.index += LANES;
            return clip_blend_lanes(
                blend_compound_average_lanes(
                    Simd::from(*pred0),
                    values,
                    self.forward,
                    self.backward,
                ),
                i32::from(u8::MAX),
            )
            .cast();
        }
        self.blend_cold(values)
    }
}

impl SubpelOutput<u8> for CompoundAverageSubpelOutputU8<'_> {
    #[allow(
        clippy::inline_always,
        reason = "measured compound-average subpel hot path"
    )]
    #[inline(always)]
    fn one(&mut self, value: i32) -> u8 {
        if let Some(&pred0) = self.pred0.get(self.index) {
            self.index += 1;
            return blend_compound_average_sample(
                pred0,
                value,
                self.forward,
                self.backward,
                i32::from(u8::MAX),
            ) as u8;
        }
        self.one_cold(value)
    }

    #[allow(
        clippy::inline_always,
        reason = "measured compound-average subpel hot path"
    )]
    #[inline(always)]
    fn sixteen(&mut self, values: Simd<i32, 16>, output: &mut [u8]) {
        output.copy_from_slice(&self.blend(values).to_array()); // splot-copy-ok: publish sixteen blended SIMD prediction lanes
    }

    #[allow(
        clippy::inline_always,
        reason = "measured compound-average subpel hot path"
    )]
    #[inline(always)]
    fn eight(&mut self, values: Simd<i32, 8>, output: &mut [u8]) {
        output.copy_from_slice(&self.blend(values).to_array()); // splot-copy-ok: publish eight blended SIMD prediction lanes
    }

    #[allow(
        clippy::inline_always,
        reason = "measured compound-average subpel hot path"
    )]
    #[inline(always)]
    fn four(&mut self, values: Simd<i32, 4>, output: &mut [u8]) {
        output.copy_from_slice(&self.blend(values).to_array()); // splot-copy-ok: publish four blended SIMD prediction lanes
    }
}
