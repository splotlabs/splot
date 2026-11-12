// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The kernels of one horizontal-sum form, defined by [`kernels`] in
//! `super::mla` and `super::sdot`.

/// Defines every kernel that runs a horizontal pass with `$features`, in a
/// module that supplies `taps`, `hline`, `hrow` and `hpair`.
macro_rules! kernels {
    ($features:literal) => {
        /// Predicts `block`, which [`fits`], into `out`: a 4-column block in
        /// row pairs, else in strips of 16 columns, or 8 for a two-pass
        /// block.
        #[inline]
        #[target_feature(enable = $features)]
        pub(super) fn strips<O: Out>(t: Neon, block: &Block<'_, u8>, out: &mut O) {
            let (h, v) = (pairs(block.horizontal), pairs(block.vertical));
            let key = 5 * h.div_ceil(2) + v;
            if block.w == 4 {
                pair_shapes!(key, pair_block::<O>(t, block, out));
                return;
            }
            let sixteen = block.w.is_multiple_of(16) && (h == 0 || v == 0);
            strip_shapes!(key + 15 * usize::from(sixteen), strip_block::<O>(t, block, out));
        }

        /// The strips of `N` columns of `block` whose passes have `H` and `V`
        /// tap pairs: each window row of a strip read once, its horizontal
        /// sums or samples kept in a register ring for the vertical pass.
        #[inline(never)]
        #[target_feature(enable = $features)]
        fn strip_block<const H: usize, const V: usize, const N: usize, O: Out>(
            t: Neon,
            block: &Block<'_, u8>,
            out: &mut O,
        ) {
            let window = block.h + if V > 0 { 2 * V - 1 } else { 0 };
            let reach = if H > 0 && N == 16 { 24 } else { 16 };
            let (base, stride) = (block.origin, block.stride);
            let (h_taps, v_weights) = (taps(t, block.horizontal), weights(t, block.vertical));
            let forward = out.forward();
            let bias = if V == 0 && O::PUT { 34 } else { 2 };
            let line = |row: &[u8]| -> Option<[int16x8_t; 2]> {
                let a = u8x16::load_array_ref(t, row.first_chunk::<16>()?).into();
                if H == 0 {
                    let a: uint8x16_t = a;
                    return Some([widen(vget_low_u8(a)), vreinterpretq_s16_u16(vmovl_high_u8(a))]);
                }
                let b = if N == 16 {
                    u8x16::load_array_ref(t, row.get(8..)?.first_chunk::<16>()?).into()
                } else {
                    a
                };
                let sums = hline::<H, N>(a, b, h_taps, bias);
                Some(if V == 0 && O::PUT { sums } else { sums.map(|x| vshrq_n_s16::<2>(x)) })
            };
            let vsum = |phase: usize, ring: &[[int16x8_t; 2]; 8], next: [int16x8_t; 2], half: usize| {
                let tap = |k: usize| {
                    if k + 1 < 2 * V { ring[(phase + k) % (2 * V)][half] } else { next[half] }
                };
                if H == 0 {
                    let sums = sums16::<V>(v_weights, tap);
                    if O::PUT { sums } else { vrshrq_n_s16::<2>(sums) }
                } else {
                    let (low, high) = sums32::<V>(v_weights, tap);
                    if O::PUT {
                        vrshrn_high_n_s32::<10>(vrshrn_n_s32::<10>(low), high)
                    } else {
                        vrshrn_high_n_s32::<6>(vrshrn_n_s32::<6>(low), high)
                    }
                }
            };
            let emit = |row: O::Row<'_>, values: [int16x8_t; 2]| {
                if O::PUT {
                    let bytes = match (H, V) {
                        (0, 0) => vqmovun_high_s16(vqmovun_s16(values[0]), values[1]),
                        (_, 0) => vqshrun_high_n_s16::<6>(vqshrun_n_s16::<6>(values[0]), values[1]),
                        (0, _) => vqrshrun_high_n_s16::<6>(vqrshrun_n_s16::<6>(values[0]), values[1]),
                        _ => vqmovun_high_s16(vqmovun_s16(values[0]), values[1]),
                    };
                    O::bytes::<N>(t, row, bytes);
                } else {
                    let values = if H == 0 && V == 0 { values.map(|x| vshlq_n_s16::<4>(x)) } else { values };
                    if O::AVERAGE {
                        let first = O::first::<N>(t, &row);
                        let bytes = if forward == 8 {
                            let low = vqrshrun_n_s16::<5>(vaddq_s16(first[0], values[0]));
                            vqrshrun_high_n_s16::<5>(low, vaddq_s16(first[1], values[1]))
                        } else {
                            let low = vqmovun_s16(blend(first[0], values[0], forward));
                            vqmovun_high_s16(low, blend(first[1], values[1], forward))
                        };
                        O::bytes::<N>(t, row, bytes);
                    } else {
                        O::preds::<N>(t, row, values);
                    }
                }
            };
            let zero = vdupq_n_s16(0);
            let period = 2 * V;
            'strips: for col in (0..block.w).step_by(N).map(|col| col.min(block.w - N)) {
                let (Some((mut source, last)), Some((targets, target))) = (
                    rows(block.source, base + col, stride, reach, window),
                    out.rows(col, N, block.h),
                ) else {
                    return;
                };
                if V == 0 {
                    for (row, target) in source.zip(targets) {
                        let Some(values) = line(row) else { return };
                        emit(target, values);
                    }
                    if let Some(values) = line(last) {
                        emit(target, values);
                    }
                    continue;
                }
                let mut ring = [[zero; 2]; 8];
                unroll!(k < period - 1, [0 1 2 3 4 5 6] => {
                    let Some(values) = source.next().and_then(line) else { return };
                    ring[k] = values;
                });
                let vertical = |phase: usize, ring: &[[int16x8_t; 2]; 8], next: [int16x8_t; 2]| {
                    let low = vsum(phase, ring, next, 0);
                    [low, if N == 16 { vsum(phase, ring, next, 1) } else { low }]
                };
                let (mut rows, mut target) = (source.zip(targets), Some(target));
                loop {
                    unroll!(phase < period, [0 1 2 3 4 5 6 7] => {
                        let Some((row, next_target)) = rows.next() else {
                            if let (Some(next), Some(target)) = (line(last), target.take()) {
                                emit(target, vertical(phase, &ring, next));
                            }
                            continue 'strips;
                        };
                        let Some(next) = line(row) else { return };
                        emit(next_target, vertical(phase, &ring, next));
                        ring[(phase + period - 1) % period] = next;
                    });
                }
            }
        }

        /// A 4-column `block` whose passes have `H` and `V` tap pairs, two rows
        /// per vector: the horizontal sums or samples of each window row pair
        /// in a register ring, whose last pair reads a row past the window
        /// that no tap weighs.
        #[inline(never)]
        #[target_feature(enable = $features)]
        fn pair_block<const H: usize, const V: usize, O: Out>(
            t: Neon,
            block: &Block<'_, u8>,
            out: &mut O,
        ) {
            let (source, stride) = (block.source, block.stride);
            let start = |row: usize| block.origin + row * stride;
            let (h_taps, v_weights) = (taps(t, block.horizontal), weights(t, block.vertical));
            let forward = out.forward();
            let bias = if V == 0 && O::PUT { 34 } else { 2 };
            let line = |row: usize| -> int16x8_t {
                if H == 0 {
                    let (a, b) = (load_u32(source, start(row)), load_u32(source, start(row + 1)));
                    return widen(vcreate_u8(u64::from(a) | u64::from(b) << 32));
                }
                let (a, b) = (load8(source, start(row)), load8(source, start(row + 1)));
                let sums = hpair(a, b, h_taps, bias);
                if V == 0 && O::PUT { sums } else { vshrq_n_s16::<2>(sums) }
            };
            let vertical = |pairs: &[int16x8_t; 4], next: int16x8_t| -> int16x8_t {
                let pair = |k: usize| if k < V { pairs[k] } else { next };
                let tap = |k: usize| {
                    if k % 2 == 0 { pair(k / 2) } else { vextq_s16::<4>(pair(k / 2), pair(k / 2 + 1)) }
                };
                if H == 0 {
                    let sums = sums16::<V>(v_weights, tap);
                    if O::PUT { sums } else { vrshrq_n_s16::<2>(sums) }
                } else {
                    let (low, high) = sums32::<V>(v_weights, tap);
                    if O::PUT {
                        vrshrn_high_n_s32::<10>(vrshrn_n_s32::<10>(low), high)
                    } else {
                        vrshrn_high_n_s32::<6>(vrshrn_n_s32::<6>(low), high)
                    }
                }
            };
            let emit = |a: O::Row<'_>, b: O::Row<'_>, values: int16x8_t| {
                if O::PUT {
                    let bytes = match (H, V) {
                        (0, 0) => vqmovun_s16(values),
                        (_, 0) => vqshrun_n_s16::<6>(values),
                        (0, _) => vqrshrun_n_s16::<6>(values),
                        _ => vqmovun_s16(values),
                    };
                    O::bytes_pair(t, a, b, vcombine_u8(bytes, bytes));
                    return;
                }
                let values = if H == 0 && V == 0 { vshlq_n_s16::<4>(values) } else { values };
                if O::AVERAGE {
                    let first = O::first_pair(t, &a, &b);
                    let bytes = if forward == 8 {
                        vqrshrun_n_s16::<5>(vaddq_s16(first, values))
                    } else {
                        vqmovun_s16(blend(first, values, forward))
                    };
                    O::bytes_pair(t, a, b, vcombine_u8(bytes, bytes));
                } else {
                    O::preds_pair(t, a, b, values);
                }
            };
            let Some((mut targets, last)) = out.rows(0, 4, block.h) else {
                return;
            };
            let mut last = Some(last);
            let mut ring = [vdupq_n_s16(0); 4];
            unroll!(k < V, [0 1 2 3] => {
                ring[k] = line(2 * k);
            });
            for row in (0..block.h).step_by(2) {
                let (Some(a), Some(b)) = (targets.next(), targets.next().or_else(|| last.take()))
                else {
                    return;
                };
                if V == 0 {
                    emit(a, b, line(row));
                    continue;
                }
                let next = line(row + 2 * V);
                emit(a, b, vertical(&ring, next));
                ring = core::array::from_fn(|k| if k + 1 < V { ring[k + 1] } else { next });
            }
        }

        /// Stores the COMPOUND_AVERAGE of a 4x4 or 8x8 [`is_cell`] pair
        /// predicted in registers into `output`.
        #[inline]
        #[target_feature(enable = $features)]
        pub(super) fn compound_cell(
            t: Neon,
            blocks: [&Block<'_, u8>; 2],
            forward: i32,
            output: &mut [u8],
            stride: usize,
        ) {
            if blocks[0].w == 8 {
                store_cell(t, cell_of::<8>(t, blocks[0]), cell_of::<8>(t, blocks[1]), forward, output, stride);
            } else {
                store_cell(t, cell_of::<4>(t, blocks[0]), cell_of::<4>(t, blocks[1]), forward, output, stride);
            }
        }

        /// Stores the COMPOUND_AVERAGE of a compound pair of one shape into
        /// `output`, the first predictor staged for the second.
        #[inline]
        #[target_feature(enable = $features)]
        pub(super) fn compound_strips(
            t: Neon,
            blocks: [&Block<'_, u8>; 2],
            forward: i32,
            output: &mut [u8],
            stride: usize,
        ) {
            let (w, h) = (blocks[0].w, blocks[0].h);
            super::super::super::with_compound_pred0_narrow(w * h, |pred0| {
                strips(t, blocks[0], &mut Prep { output: &mut *pred0, stride: w });
                strips(t, blocks[1], &mut Average { pred0, w, forward, output, stride });
            });
        }

        /// The cells of `program` whose windows are read in place or by
        /// clamped rows and columns: the window of each reference derived
        /// once, both planes stored from it.
        #[inline(never)]
        #[target_feature(enable = $features)]
        pub(super) fn run_program<const W: usize>(t: Neon, program: Program<'_, '_>) -> Written {
            let Program { mut planes, geometry, filter, cells, forward } = program;
            let mut written = [[0u64; 4]; 2];
            let place = |(start, bounds): CellPlacement, geometry: Geometry| {
                let window = Window::of_filters([filter; 2], W, W, start);
                let (horizontal, vertical) = (window.horizontal, window.vertical);
                let base = window.origin(geometry, bounds).filter(|_| is_cell(W, W, horizontal, vertical))?;
                Some((base, horizontal, vertical))
            };
            for (index, &([row, col], placements)) in cells.iter().enumerate() {
                let (Some(first), Some(second)) = (place(placements[0], geometry[0]), place(placements[1], geometry[1])) else {
                    if clamped_cell::<W>(t, &mut planes, (geometry, filter, forward), ([row, col], placements)) {
                        for written in &mut written {
                            written[index / 64 % 4] |= 1 << (index % 64);
                        }
                    }
                    continue;
                };
                for (plane, written) in planes.iter_mut().zip(&mut written) {
                    let Some((sources, output, out_stride)) = plane else { continue };
                    let at = row as usize * *out_stride + col as usize;
                    let Some(output) = output
                        .get_mut(at..)
                        .filter(|output| (W - 1) * *out_stride + W <= output.len())
                    else {
                        continue;
                    };
                    let [(base0, h0, v0), (base1, h1, v1)] = [first, second];
                    let groups0 = cells!(pairs(h0), pairs(v0), false, (t, sources[0], (base0, geometry[0][0]), &IN_PLACE, h0, v0));
                    let groups1 = cells!(pairs(h1), pairs(v1), false, (t, sources[1], (base1, geometry[1][0]), &IN_PLACE, h1, v1));
                    store_cell(t, groups0, groups1, forward, output, *out_stride);
                    written[index / 64 % 4] |= 1 << (index % 64);
                }
            }
            written
        }

        /// A cell of [`run_program`] with a window that is not read in place,
        /// read by clamped rows and columns into every plane; `false` without
        /// writing for a cell [`is_cell`] does not admit, an empty clip range
        /// or an output that cannot hold it.
        #[cold]
        #[inline(never)]
        #[target_feature(enable = $features)]
        fn clamped_cell<const W: usize>(
            t: Neon,
            planes: &mut [Option<ProgramPlane<'_, '_>>; 2],
            (geometry, filter, forward): ([Geometry; 2], usize, i32),
            ([row, col], placements): super::super::super::cell::Cell,
        ) -> bool {
            let (Some(first), Some(second)) =
                (place::<W>(filter, placements[0], geometry[0]), place::<W>(filter, placements[1], geometry[1]))
            else {
                return false;
            };
            let fits = planes.iter().flatten().all(|(_, output, stride)| {
                (row as usize * stride + col as usize).checked_add((W - 1) * stride + W)
                    .is_some_and(|end| end <= output.len())
            });
            if !fits {
                return false;
            }
            for (sources, output, out_stride) in planes.iter_mut().flatten() {
                let at = row as usize * *out_stride + col as usize;
                let Some(output) = output.get_mut(at..) else { continue };
                let groups = (placed_groups::<W>(t, sources[0], first), placed_groups::<W>(t, sources[1], second));
                store_cell(t, groups.0, groups.1, forward, output, *out_stride);
            }
            true
        }

        /// The compound predictors of a `W`-square register cell of
        /// `source` read by its [`Placed`] reads.
        #[inline]
        #[target_feature(enable = $features)]
        fn placed_groups<const W: usize>(t: Neon, source: &[u8], (reads, clamped, h, v): Placed) -> [int16x8_t; W] {
            let place = (reads.base, reads.stride);
            if clamped {
                cells!(pairs(h), pairs(v), true, (t, source, place, &reads, h, v))
            } else {
                cells!(pairs(h), pairs(v), false, (t, source, place, &IN_PLACE, h, v))
            }
        }

        /// Stores the COMPOUND_AVERAGE of the `W`-square cell of `windows`
        /// within `bounds`, each read in place or by clamped rows and columns
        /// of its reference, into `output`; `false` without writing for a
        /// cell [`is_cell`] does not admit, an empty clip range or an output
        /// that cannot hold it.
        #[inline(never)]
        #[target_feature(enable = $features)]
        pub(super) fn windows_cell<const W: usize>(t: Neon, job: Windows<'_, '_>) -> bool {
            let Windows(references, windows, bounds, forward, output, stride) = job;
            let placed = |index: usize| place_reads::<W>(windows[index], references[index].1, bounds[index]);
            let (Some(first), Some(second)) = (placed(0), placed(1)) else {
                return false;
            };
            if (W - 1) * stride + W > output.len() || stride < W {
                return false;
            }
            let groups = (placed_groups::<W>(t, references[0].0, first), placed_groups::<W>(t, references[1].0, second));
            store_cell(t, groups.0, groups.1, forward, output, stride);
            true
        }

        /// The compound predictors of a `W`-square register cell, which
        /// [`is_cell`] admits: the rows of an 8x8 cell or the row pairs of a
        /// 4x4 one.
        #[inline]
        #[target_feature(enable = $features)]
        fn cell_of<const W: usize>(t: Neon, block: &Block<'_, u8>) -> [int16x8_t; W] {
            let (horizontal, vertical) = (block.horizontal, block.vertical);
            let place = (block.origin, block.stride);
            cells!(pairs(horizontal), pairs(vertical), false, (t, block.source, place, &IN_PLACE, horizontal, vertical))
        }

        /// The compound predictors of a register cell of `W` columns whose
        /// passes have `H` and `V` tap pairs.
        #[inline(never)]
        #[target_feature(enable = $features)]
        fn cell_groups<const W: usize, const H: usize, const V: usize, const C: bool>(
            t: Neon,
            source: &[u8],
            (base, stride): (usize, usize),
            clamped: &Reads,
            horizontal: Option<&Taps>,
            vertical: Option<&Taps>,
        ) -> [int16x8_t; W] {
            let Reads { top, first, last, columns: [start, _, high], .. } = *clamped;
            let gather = {
                let lanes = vaddq_s8(vdupq_n_s8(start as i8), i8x16::load_array(t, BYTE_LANES).into());
                let high = vdupq_n_s8(high.min(15) as i8);
                vreinterpretq_u8_s8(vminq_s8(vmaxq_s8(lanes, vdupq_n_s8(0)), high))
            };
            let at = |row: usize| {
                if C {
                    base + top.saturating_add(row as i32).max(first).min(last) as usize * stride
                } else {
                    base + row * stride
                }
            };
            let load16 = |at: usize| {
                let bytes = load16(t, source, at);
                if C { vqtbl1q_u8(bytes, gather) } else { bytes }
            };
            let load8 = |at: usize| if C { vget_low_u8(load16(at)) } else { load8(source, at) };
            let load_u32 = |at: usize| {
                if C { vget_lane_u32::<0>(vreinterpret_u32_u8(load8(at))) } else { load_u32(source, at) }
            };
            let rows = if V > 0 { W + 2 * V - 1 } else { W };
            let (h_weights, v_weights) = (taps(t, horizontal), weights(t, vertical));
            let zero = vdupq_n_s16(0);
            let mut groups = [zero; W];
            let vertical16 = |sums: int16x8_t| vrshrq_n_s16::<2>(sums);
            let vertical32 = |(low, high): Sums| vrshrn_high_n_s32::<6>(vrshrn_n_s32::<6>(low), high);
            if W == 8 {
                let mut lines = [zero; 15];
                unroll!(row < rows, [0 1 2 3 4 5 6 7 8 9 10 11 12 13 14] => {
                    let bytes = load16(at(row));
                    lines[row] = if H > 0 {
                        vrshrq_n_s16::<2>(hrow::<H>(bytes, h_weights))
                    } else {
                        widen(vget_low_u8(bytes))
                    };
                });
                unroll!(row < 8, [0 1 2 3 4 5 6 7] => {
                    let line = |k: usize| lines[row + k];
                    groups[row] = match (H, V) {
                        (_, 0) if H > 0 => lines[row],
                        (0, 0) => vshlq_n_s16::<4>(lines[row]),
                        (0, _) => vertical16(sums16::<V>(v_weights, line)),
                        _ => vertical32(sums32::<V>(v_weights, line)),
                    };
                });
                return groups;
            }
            let mut pairs = [zero; 8];
            unroll!(pair < rows.div_ceil(2), [0 1 2 3 4 5 6 7] => {
                let starts = [at(2 * pair), at(2 * pair + 1)];
                pairs[pair] = if H > 0 {
                    let (a, b) = (load8(starts[0]), load8(starts[1]));
                    vshrq_n_s16::<2>(hpair(a, b, h_weights, 2))
                } else {
                    widen(vcreate_u8(
                        u64::from(load_u32(starts[0])) | u64::from(load_u32(starts[1])) << 32,
                    ))
                };
            });
            unroll!(group < 2, [0 1] => {
                let tap = |k: usize| {
                    if k.is_multiple_of(2) {
                        pairs[group + k / 2]
                    } else {
                        vextq_s16::<4>(pairs[group + k / 2], pairs[group + k / 2 + 1])
                    }
                };
                groups[group] = match (H, V) {
                    (_, 0) if H > 0 => pairs[group],
                    (0, 0) => vshlq_n_s16::<4>(pairs[group]),
                    (0, _) => vertical16(sums16::<V>(v_weights, tap)),
                    _ => vertical32(sums32::<V>(v_weights, tap)),
                };
            });
            groups
        }
    };
}

pub(super) use kernels;
