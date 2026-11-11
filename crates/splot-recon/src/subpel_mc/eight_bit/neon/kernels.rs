// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The kernels of one horizontal-sum form, defined by [`kernels`] in
//! `super::mla` and `super::sdot`.

/// Defines every kernel that runs a horizontal pass with `$features`, in a
/// module that supplies `taps`, `hrow`, `hpair` and `pair_columns`.
macro_rules! kernels {
    ($features:literal) => {
        /// Runs `job`.
        #[target_feature(enable = $features)]
        pub(super) fn entry(t: Neon, job: Job<'_, '_, '_>) {
            match job {
                Job::Predict(block, sink) => predict(t, block, sink),
                Job::Compound(blocks, forward, output, stride) => {
                    compound(t, blocks, forward, output, stride);
                }
                Job::Cell(blocks, size, forward, output, stride) => {
                    compound_cell(t, blocks, size, forward, output, stride);
                }
                Job::PutCell(block, output, stride) => put_cell(t, block, output, stride),
                Job::Program(program, size, written) => {
                    *written = if size == 4 {
                        run_program::<4>(t, program)
                    } else {
                        run_program::<8>(t, program)
                    };
                }
            }
        }

        /// Predicts `block`, which [`fits`], into an eight-bit `sink`.
        #[inline]
        #[target_feature(enable = $features)]
        fn predict(t: Neon, block: &Block<'_, u8>, sink: &mut Sink<'_>) {
            let w = block.w;
            if strip_fits(block) {
                match sink {
                    Sink::Put { output, stride } => {
                        return strips(t, block, &mut Put { output, stride: *stride });
                    }
                    Sink::Prep { output, stride } => {
                        return strips(t, block, &mut Prep { output, stride: *stride });
                    }
                    Sink::PrepNarrow { output, stride } => {
                        return strips(t, block, &mut Prep { output, stride: *stride });
                    }
                    Sink::AverageNarrow { pred0, forward, output, stride } => {
                        let forward = *forward;
                        let stride = *stride;
                        return strips(t, block, &mut Average { pred0, w, forward, output, stride });
                    }
                    Sink::Average { pred0, forward, output, stride } => {
                        let forward = *forward;
                        let stride = *stride;
                        return strips(t, block, &mut Average { pred0, w, forward, output, stride });
                    }
                    _ => {}
                }
            }
            match sink {
                Sink::Put { output, stride } => {
                    let (output, stride) = (&mut **output, *stride);
                    passes::<true>(t, block, &mut move |four, _, row, col, values| {
                        store_bytes(output, if four { 4 } else { 8 }, row * stride + col, stride, vqmovun_s16(values));
                    });
                }
                Sink::PutWide { output, stride, .. } => {
                    let (output, stride) = (&mut **output, *stride);
                    let (zero, max) = (vdupq_n_s16(0), vdupq_n_s16(255));
                    passes::<true>(t, block, &mut move |four, _, row, col, values| {
                        let values = vreinterpretq_u16_s16(vminq_s16(vmaxq_s16(values, zero), max));
                        store_words(t, output, if four { 4 } else { 8 }, row * stride + col, stride, values);
                    });
                }
                Sink::Prep { output, stride } => {
                    let (output, stride) = (&mut **output, *stride);
                    passes::<false>(t, block, &mut move |four, _, row, col, values| {
                        let at = row * stride + col;
                        let next = if four { at + stride } else { at + 4 };
                        store_i32(t, output, at, vmovl_s16(vget_low_s16(values)));
                        store_i32(t, output, next, vmovl_high_s16(values));
                    });
                }
                Sink::Average {
                    pred0,
                    forward,
                    output,
                    stride,
                } => {
                    let (pred0, forward, output, stride) = (&**pred0, *forward, &mut **output, *stride);
                    passes::<false>(t, block, &mut move |four, _, row, col, values| {
                        let at = row * w + col;
                        let first = vmovn_high_s32(
                            vmovn_s32(load_i32(t, pred0, at)),
                            load_i32(t, pred0, at + 4),
                        );
                        let values = vqmovun_s16(blend(first, values, forward));
                        store_bytes(output, if four { 4 } else { 8 }, row * stride + col, stride, values);
                    });
                }
                Sink::PrepNarrow { output, stride } => {
                    let (output, stride) = (&mut **output, *stride);
                    passes::<false>(t, block, &mut move |four, _, row, col, values| {
                        let at = row * stride + col;
                        let next = if four { at + stride } else { at + 4 };
                        store_i16(t, output, at, vget_low_s16(values));
                        store_i16(t, output, next, vget_high_s16(values));
                    });
                }
                Sink::AverageNarrow {
                    pred0,
                    forward,
                    output,
                    stride,
                } => {
                    let (pred0, forward, output, stride) = (&**pred0, *forward, &mut **output, *stride);
                    passes::<false>(t, block, &mut move |four, _, row, col, values| {
                        let at = row * w + col;
                        let first = vcombine_s16(load_i16(pred0, at), load_i16(pred0, at + 4));
                        let values = vqmovun_s16(blend(first, values, forward));
                        store_bytes(output, if four { 4 } else { 8 }, row * stride + col, stride, values);
                    });
                }
                Sink::AverageWide { .. } => {}
            }
        }

        /// Predicts a block of [`strip_fits`] into `out` in strips of 16
        /// columns, else 8.
        #[inline]
        #[target_feature(enable = $features)]
        fn strips<O: Out>(t: Neon, block: &Block<'_, u8>, out: &mut O) {
            let h = match block.horizontal.map_or(0, |taps| taps.count) {
                0 => 0,
                1 | 2 => 2,
                _ => 4,
            };
            let v = block.vertical.map_or(0, |taps| taps.count);
            strip_shapes!(block.w.is_multiple_of(16) && (h == 0 || v == 0), h, v, strip_block::<O>(t, block, out));
        }

        /// The strips of `N` columns of a block of [`strip_fits`] whose passes
        /// have `H` and `V` tap pairs: each window row of a strip read once,
        /// its horizontal sums or samples kept in a register ring for the
        /// vertical pass.
        #[inline(never)]
        #[target_feature(enable = $features)]
        fn strip_block<const H: usize, const V: usize, const N: usize, O: Out>(
            t: Neon,
            block: &Block<'_, u8>,
            out: &mut O,
        ) {
            let window = block.h + if V > 0 { 2 * V - 1 } else { 0 };
            let reach = if H > 0 && N == 16 { 24 } else { 16 };
            let (base, stride) = (block.start(0), block.stride);
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
            'strips: for col in (0..block.w).step_by(N) {
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

        /// Predicts both blocks of a compound cell of at most 64 samples (or of
        /// two full-sample blocks of any size), which [`fits`], and stores their
        /// COMPOUND_AVERAGE into `output`.
        #[inline]
        #[target_feature(enable = $features)]
        fn compound(
            t: Neon,
            blocks: [&Block<'_, u8>; 2],
            forward: i32,
            output: &mut [u8],
            stride: usize,
        ) {
            if blocks
                .iter()
                .all(|block| block.horizontal.is_none() && block.vertical.is_none())
            {
                fullpel(t, blocks, forward, output, stride);
                return;
            }
            if blocks.iter().all(|block| is_cell(block)) {
                if blocks[0].w == 4
                    && let (Some(first), Some(second)) =
                        (cell::<4>(t, blocks[0]), cell::<4>(t, blocks[1]))
                {
                    store_cell(first, second, forward, output, stride);
                    return;
                }
                if let (Some(first), Some(second)) = (cell::<8>(t, blocks[0]), cell::<8>(t, blocks[1]))
                {
                    store_cell(first, second, forward, output, stride);
                    return;
                }
            }
            let mut groups = [vdupq_n_s16(0); 8];
            let mut first = |_, group: usize, _, _, values| {
                if let Some(slot) = groups.get_mut(group) {
                    *slot = values;
                }
            };
            passes::<false>(t, blocks[0], &mut first);
            let mut second = |four: bool, group: usize, row: usize, col: usize, values| {
                let first = groups.get(group).copied().unwrap_or(values);
                let values = vqmovun_s16(blend(first, values, forward));
                store_bytes(output, if four { 4 } else { 8 }, row * stride + col, stride, values);
            };
            passes::<false>(t, blocks[1], &mut second);
        }

        /// [`compound`] of the blocks of a `size`-square cell of
        /// `subpel_mc::cell`, whose unremapped windows lie inside their clip rows.
        #[inline]
        #[target_feature(enable = $features)]
        fn compound_cell(
            t: Neon,
            blocks: [&Block<'_, u8>; 2],
            size: usize,
            forward: i32,
            output: &mut [u8],
            stride: usize,
        ) {
            if blocks[0].remap.is_none() && blocks[1].remap.is_none() {
                if size == 4 {
                    if let (Some(first), Some(second)) =
                        (cell::<4>(t, blocks[0]), cell::<4>(t, blocks[1]))
                    {
                        store_cell(first, second, forward, output, stride);
                        return;
                    }
                } else if let (Some(first), Some(second)) =
                    (cell::<8>(t, blocks[0]), cell::<8>(t, blocks[1]))
                {
                    store_cell(first, second, forward, output, stride);
                    return;
                }
            }
            compound(t, blocks, forward, output, stride);
        }

        /// Stores the `Clip1` single-reference prediction of a 4x4 or 8x8
        /// `block`, which [`fits`].
        #[inline]
        #[target_feature(enable = $features)]
        fn put_cell(
            t: Neon,
            block: &Block<'_, u8>,
            output: &mut [u8],
            stride: usize,
        ) {
            if is_cell(block) {
                if block.w == 4
                    && let Some(groups) = put_groups::<4>(t, block)
                {
                    store_put(groups, output, stride);
                    return;
                }
                if let Some(groups) = put_groups::<8>(t, block) {
                    store_put(groups, output, stride);
                    return;
                }
            }
            passes::<true>(t, block, &mut move |four, _, row, col, values| {
                store_bytes(
                    output,
                    if four { 4 } else { 8 },
                    row * stride + col,
                    stride,
                    vqmovun_s16(values),
                );
            });
        }

        /// The cells of `program` whose windows are read in place: the
        /// geometry and taps of each cell's references derived once, both
        /// planes stored from them.
        #[inline(never)]
        #[target_feature(enable = $features)]
        fn run_program<const W: usize>(
            t: Neon,
            program: Program<'_, '_>,
        ) -> super::super::super::cell::Written {
            let Program { mut planes, geometry, filter, cells, forward } = program;
            let mut written = [[0u64; 4]; 2];
            let stride = geometry.stride;
            for (index, &([row, col], placements)) in cells.iter().enumerate() {
                let place = |placement| super::super::super::cell::placed(geometry, filter, W, placement);
                let (Some(first), Some(second)) = (place(placements[0]), place(placements[1])) else {
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
                    let groups = |source: &[u8], place: super::super::super::cell::Placed| {
                        let pairs = |taps: Option<&Taps>| taps.map_or(0, |taps| taps.count);
                        let (horizontal, vertical) = (place.horizontal, place.vertical);
                        cells!(
                            pairs(horizontal),
                            pairs(vertical),
                            false(t, source, place.base, stride, horizontal, vertical)
                        )
                    };
                    if let (Some(first), Some(second)) =
                        (groups(sources[0], first), groups(sources[1], second))
                    {
                        store_cell(first, second, forward, output, *out_stride);
                        written[index / 64 % 4] |= 1 << (index % 64);
                    }
                }
            }
            written
        }

        /// The compound predictors of a `W`-square register cell, the rows of an
        /// 8x8 cell or the row pairs of a 4x4 one.
        #[inline]
        #[target_feature(enable = $features)]
        fn cell<const W: usize>(t: Neon, block: &Block<'_, u8>) -> Option<[int16x8_t; W]> {
            let pairs = |taps: Option<&Taps>| taps.map_or(0, |taps| taps.count);
            let (source, base, stride) = (block.source, block.start(0), block.stride);
            let (horizontal, vertical) = (block.horizontal, block.vertical);
            cells!(
                pairs(horizontal),
                pairs(vertical),
                false(t, source, base, stride, horizontal, vertical)
            )
        }

        /// The single-reference predictions of a `W`-square register cell, before
        /// `Clip1`.
        #[inline]
        #[target_feature(enable = $features)]
        fn put_groups<const W: usize>(t: Neon, block: &Block<'_, u8>) -> Option<[int16x8_t; W]> {
            let pairs = |taps: Option<&Taps>| taps.map_or(0, |taps| taps.count);
            let (source, base, stride) = (block.source, block.start(0), block.stride);
            let (horizontal, vertical) = (block.horizontal, block.vertical);
            cells!(
                pairs(horizontal),
                pairs(vertical),
                true(t, source, base, stride, horizontal, vertical)
            )
        }

        /// The groups of a register cell of `W` columns whose passes have `H` and `V`
        /// tap pairs: with `PUT` the single-reference predictions before `Clip1`,
        /// else the compound predictors.
        #[inline(never)]
        #[target_feature(enable = $features)]
        fn cell_groups<const PUT: bool, const W: usize, const H: usize, const V: usize>(
            t: Neon,
            source: &[u8],
            base: usize,
            stride: usize,
            horizontal: Option<&Taps>,
            vertical: Option<&Taps>,
        ) -> Option<[int16x8_t; W]> {
            let rows = if V > 0 { W + 2 * V - 1 } else { W };
            let (h_weights, v_weights) = (taps(t, horizontal), weights(t, vertical));
            let zero = vdupq_n_s16(0);
            let mut groups = [zero; W];
            let horizontal = |sums: int16x8_t| match (V, PUT) {
                (0, true) => vshrq_n_s16::<6>(vaddq_s16(sums, vdupq_n_s16(34))),
                _ => vrshrq_n_s16::<2>(sums),
            };
            let copy = |x: int16x8_t| if PUT { x } else { vshlq_n_s16::<4>(x) };
            let vertical16 = |sums: int16x8_t| {
                if PUT {
                    vrshrq_n_s16::<6>(sums)
                } else {
                    vrshrq_n_s16::<2>(sums)
                }
            };
            let vertical32 = |(low, high): Sums| {
                if PUT {
                    vrshrn_high_n_s32::<10>(vrshrn_n_s32::<10>(low), high)
                } else {
                    vrshrn_high_n_s32::<6>(vrshrn_n_s32::<6>(low), high)
                }
            };
            if W == 8 {
                let mut lines = [zero; 15];
                unroll!(row < rows, [0 1 2 3 4 5 6 7 8 9 10 11 12 13 14] => {
                    let bytes = load16(t, source, base + row * stride);
                    lines[row] = if H > 0 {
                        horizontal(hrow::<H>(bytes, h_weights))
                    } else {
                        widen(vget_low_u8(bytes))
                    };
                });
                unroll!(row < 8, [0 1 2 3 4 5 6 7] => {
                    let line = |k: usize| lines[row + k];
                    groups[row] = match (H, V) {
                        (_, 0) if H > 0 => lines[row],
                        (0, 0) => copy(lines[row]),
                        (0, _) => vertical16(sums16::<V>(v_weights, line)),
                        _ => vertical32(sums32::<V>(v_weights, line)),
                    };
                });
                return Some(groups);
            }
            let columns = pair_columns(t, None);
            let mut pairs = [zero; 8];
            unroll!(pair < rows.div_ceil(2), [0 1 2 3 4 5 6 7] => {
                let starts = [base + 2 * pair * stride, base + (2 * pair + 1) * stride];
                pairs[pair] = if H > 0 {
                    horizontal(hpair::<H>(source, starts, h_weights, &columns))
                } else {
                    widen(vcreate_u8(
                        u64::from(load_u32(source, starts[0])) | u64::from(load_u32(source, starts[1])) << 32,
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
                    (0, 0) => copy(pairs[group]),
                    (0, _) => vertical16(sums16::<V>(v_weights, tap)),
                    _ => vertical32(sums32::<V>(v_weights, tap)),
                };
            });
            Some(groups)
        }

        /// Runs the passes of `block`, handing `out` each
        /// group as `(group, row, col, values)`, strip by strip: with `PUT` the
        /// single-reference predictions before `Clip1`, else the compound
        /// predictors.
        #[inline]
        #[target_feature(enable = $features)]
        fn passes<const PUT: bool>(
            t: Neon,
            block: &Block<'_, u8>,
            out: &mut impl FnMut(bool, usize, usize, usize, int16x8_t),
        ) {
            let (h, v) = (
                block.horizontal.map_or(0, |taps| taps.count),
                block.vertical.map_or(0, |taps| taps.count),
            );
            specialize!(block.w == 4, h, v, run::<PUT>(t, block, out));
        }

        /// Dispatches one pass shape of `block`.
        #[inline]
        #[target_feature(enable = $features)]
        fn run<const PUT: bool, const W: usize, const H: usize, const V: usize>(
            t: Neon,
            block: &Block<'_, u8>,
            out: &mut impl FnMut(bool, usize, usize, usize, int16x8_t),
        ) {
            match (H, V) {
                (0, 0) => copy::<PUT, W>(t, block, out),
                (_, 0) => horizontal::<PUT, W, H>(t, block, out),
                (0, _) => vertical::<PUT, W, V>(t, block, out),
                _ => two_pass::<PUT, W, H, V>(t, block, out),
            }
        }

        /// A block with only a horizontal pass of `H` tap pairs.
        #[inline(never)]
        #[target_feature(enable = $features)]
        fn horizontal<const PUT: bool, const W: usize, const H: usize>(
            t: Neon,
            block: &Block<'_, u8>,
            out: &mut impl FnMut(bool, usize, usize, usize, int16x8_t),
        ) {
            let rows = block.h;
            let weights = taps(t, block.horizontal);
            let finish = |sums: int16x8_t| {
                if PUT {
                    vshrq_n_s16::<6>(vaddq_s16(sums, vdupq_n_s16(34)))
                } else {
                    vrshrq_n_s16::<2>(sums)
                }
            };
            let start = starts(block, rows);
            if W == 4 {
                let columns = pair_columns(t, block.remap);
                for row in (0..rows).step_by(2) {
                    let starts = [start(row), start(row + 1)];
                    out(
                        W == 4,
                        row / 2,
                        row,
                        0,
                        finish(hpair::<H>(block.source, starts, weights, &columns)),
                    );
                }
                return;
            }
            let remap = columns(t, block);
            for col in (0..block.w).step_by(8) {
                for row in 0..rows {
                    let sums = hrow::<H>(window(t, block.source, start(row) + col, remap), weights);
                    out(W == 4, col / 8 * rows + row, row, col, finish(sums));
                }
            }
        }

        /// Both passes of `block`, `H` and `V` tap pairs: the `Round2(sum,
        /// InterRound0)` horizontal rows of each strip in a register ring, two rows
        /// per vector for a 4-column block, whose last pair reads a row past the
        /// window that no tap weighs.
        #[inline(never)]
        #[target_feature(enable = $features)]
        fn two_pass<const PUT: bool, const W: usize, const H: usize, const V: usize>(
            t: Neon,
            block: &Block<'_, u8>,
            out: &mut impl FnMut(bool, usize, usize, usize, int16x8_t),
        ) {
            let rows = block.h;
            let (h_weights, v_weights) = (taps(t, block.horizontal), weights(t, block.vertical));
            let finish = |(low, high): Sums| {
                if PUT {
                    vrshrn_high_n_s32::<10>(vrshrn_n_s32::<10>(low), high)
                } else {
                    vrshrn_high_n_s32::<6>(vrshrn_n_s32::<6>(low), high)
                }
            };
            let start = starts(block, rows + 2 * V - 1);
            if W == 4 {
                let columns = pair_columns(t, block.remap);
                let line = |row: usize| {
                    let starts = [start(row), start(row + 1)];
                    vrshrq_n_s16::<2>(hpair::<H>(block.source, starts, h_weights, &columns))
                };
                let mut pairs: [int16x8_t; 4] =
                    core::array::from_fn(|k| if k < V { line(2 * k) } else { vdupq_n_s16(0) });
                for row in (0..rows).step_by(2) {
                    let next = line(row + 2 * V);
                    let pair = |k: usize| if k < V { pairs[k] } else { next };
                    let sums = sums32::<V>(v_weights, |k| {
                        if k % 2 == 0 {
                            pair(k / 2)
                        } else {
                            vextq_s16::<4>(pair(k / 2), pair(k / 2 + 1))
                        }
                    });
                    out(W == 4, row / 2, row, 0, finish(sums));
                    pairs = core::array::from_fn(|k| pair(k + 1));
                }
                return;
            }
            let remap = columns(t, block);
            for col in (0..block.w).step_by(8) {
                let line = |row: usize| {
                    let start = start(row) + col;
                    vrshrq_n_s16::<2>(hrow::<H>(window(t, block.source, start, remap), h_weights))
                };
                let mut ring: [int16x8_t; 7] = core::array::from_fn(|k| {
                    if k + 1 < 2 * V {
                        line(k)
                    } else {
                        vdupq_n_s16(0)
                    }
                });
                for row in 0..rows {
                    let next = line(row + 2 * V - 1);
                    let tap = |k: usize| if k + 1 < 2 * V { ring[k] } else { next };
                    out(
                        W == 4,
                        col / 8 * rows + row,
                        row,
                        col,
                        finish(sums32::<V>(v_weights, tap)),
                    );
                    ring = core::array::from_fn(|k| tap(k + 1));
                }
            }
        }
    };
}

pub(super) use kernels;
