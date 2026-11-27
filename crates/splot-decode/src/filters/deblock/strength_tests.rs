// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

type Sides = [[u16; GROUP]; 2];

/// The `qThr` and `side` words of each lane of `sides`, looked up lane by
/// lane in `cache`.
fn oracle(cache: &StrengthCache, sides: Sides) -> [[u16; GROUP]; 4] {
    let mut words = [[0; GROUP]; 4];
    for (index, qindexes) in sides.iter().enumerate() {
        for (lane, &qindex) in qindexes.iter().enumerate() {
            let (q_thr, side) = cache.get(u32::from(qindex));
            words[2 * index][lane] = q_thr.clamp(0, 0x7fff) as u16;
            words[2 * index + 1][lane] = side.clamp(0, 0x7fff) as u16;
        }
    }
    words
}

/// Asserts that one [`Strengths`] over `cache` gives every group of `groups`,
/// in order, the words of [`oracle`].
fn assert_groups_match<S: Simd>(simd: S, cache: &StrengthCache, groups: &[Sides]) {
    let mut strengths = Strengths::new(cache);
    for &sides in groups {
        let [curr, prev] = sides.map(|side| u16x16::from_slice(simd, &side));
        let words = strengths.of(simd, curr, prev).map(SimdBase::to_array);
        assert_eq!(words, oracle(cache, sides), "qindex lanes {sides:?}");
    }
}

fn lcg(state: &mut u64) -> usize {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    (*state >> 33) as usize
}

/// `qindex` values past both bit depths' `MaxQ`, at it and at zero.
const EXTREMES: [u16; 10] = [0, 1, 254, 255, 256, 302, 303, 304, 1000, u16::MAX - 1];

/// Lane groups of every shape: all lanes equal, each side uniform, all 32
/// lanes distinct, the [`EXTREMES`], and random draws from pools of 1 to 32
/// values.
fn groups() -> Vec<Sides> {
    let uniform = |curr: u16, prev: u16| [[curr; GROUP], [prev; GROUP]];
    let distinct = |index: usize| 3 + 9 * index as u16;
    let extremes = |shift: usize| {
        let lane = |index: usize| EXTREMES[(index + shift) % EXTREMES.len()];
        [
            core::array::from_fn(lane),
            core::array::from_fn(|index| lane(index + 3)),
        ]
    };
    let mut groups = vec![
        uniform(0, 0),
        uniform(120, 120),
        uniform(120, 120),
        uniform(303, 303),
        uniform(40, 200),
        uniform(200, 40),
        uniform(40, 40),
        [
            core::array::from_fn(distinct),
            core::array::from_fn(|index| distinct(index + GROUP)),
        ],
        extremes(0),
        extremes(5),
    ];
    let mut state = 0x5eed;
    for _ in 0..400 {
        let pool: [u16; 2 * GROUP] = core::array::from_fn(|_| (lcg(&mut state) % 304) as u16);
        let size = 1 + lcg(&mut state) % pool.len();
        let mut side = || core::array::from_fn(|_| pool[lcg(&mut state) % size]);
        groups.push([side(), side()]);
    }
    groups
}

#[test]
fn lane_strengths_match_the_lane_by_lane_lookup() {
    let groups = groups();
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        let cache = StrengthCache::new(-6, 3, bit_depth);
        baseline_simd!(simd => assert_groups_match(simd, &cache, &groups));
    }
}

#[test]
fn lane_strengths_keep_to_15_bits() {
    let cache = StrengthCache::new(0, 0, BitDepth::Ten);
    for (qindex, strengths) in [
        (7, (i32::MAX, i32::MAX)),
        (9, (0x8001, 3)),
        (11, (4, 0x8000)),
    ] {
        cache.values[qindex].set(strengths);
    }
    let lane = |index: usize| [7, 9, 11][index % 3];
    let sides = [
        core::array::from_fn(lane),
        core::array::from_fn(|index| lane(index + 1)),
    ];
    baseline_simd!(simd => assert_groups_match(simd, &cache, &[sides]));
}
