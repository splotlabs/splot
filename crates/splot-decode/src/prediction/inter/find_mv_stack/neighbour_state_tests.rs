// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Reader-level digests of the neighbour state the tile parse leaves behind:
//! the `TileMiSizeState` edge lines, the SDP collocated facts and the
//! `NeighbourMvGrid` edges, plane and leaf records, each read only through the
//! accessors its consumers call, at positions decoded before the read.
//! `SPLOT_PRINT_NEIGHBOUR_DIGESTS=1 cargo test -p splot-decode neighbour_state -- --nocapture`
//! prints the current digests instead of checking them.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use core::cell::RefCell;
use core::fmt::{Debug, Write};
use core::ops::Range;

use super::neighbour_grid::{NeighbourFlags, NeighbourMvGrid};
use crate::bitstream::tile_payload::{
    BlockSize, DecodeBlockFrontier, IntraNeighbours, LeafNeighbours, PartitionNeighbours, UvEdges,
};

const READERS: [&str; 8] = [
    "mv_edges",
    "partition",
    "chroma_misize",
    "intra_nbrs",
    "uv_edges",
    "palette",
    "collocated",
    "mv_plane",
];

#[derive(Clone, Copy)]
struct Digest(u64);

impl Default for Digest {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Write for Digest {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        for byte in text.bytes() {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
        }
        Ok(())
    }
}

impl Digest {
    fn add(&mut self, value: impl Debug) {
        write!(self, "{value:?};").unwrap();
    }
}

#[derive(Default)]
struct Recorder {
    digests: [Digest; READERS.len()],
    counts: [usize; COUNTS.len()],
    last_row: usize,
    sb_h4: usize,
    fresh: bool,
}

const COUNTS: [&str; 10] = [
    "leaves",
    "chroma_part",
    "inter_frame_chroma_part",
    "palette_leaves",
    "intra",
    "inter",
    "compound",
    "tip",
    "warp",
    "intrabc",
];

thread_local! {
    static RECORDER: RefCell<Option<Recorder>> = const { RefCell::new(None) };
}

fn flags(flags: NeighbourFlags) -> [i32; 16] {
    [
        i32::from(flags.is_present()),
        i32::from(flags.is_inter()),
        i32::from(flags.ref_frame0()),
        flags.ref_frame1().map_or(-100, i32::from),
        i32::from(flags.newmv_for_list0()),
        i32::from(flags.newmv_for_list1()),
        i32::from(flags.skip_mode()),
        i32::from(flags.skip()),
        i32::from(flags.use_amvd()),
        i32::from(flags.masked_compound()),
        i32::from(flags.tip_size_16x16()),
        i32::from(flags.interp_filter()),
        i32::from(flags.is_warp()),
        i32::from(flags.is_local_warp()),
        i32::from(flags.use_most_probable_precision()),
        i32::from(flags.mv_precision()),
    ]
}

fn partition_left(neighbours: Option<PartitionNeighbours>) -> impl Debug {
    neighbours.map(|n| (n.left[0] & 0x3f, n.luma_left & 0x3f))
}

fn partition_above(neighbours: Option<PartitionNeighbours>) -> impl Debug {
    neighbours.map(|n| (n.above[0] & 0x3f, n.luma_above & 0x3f))
}

fn intra(neighbours: IntraNeighbours) -> impl Debug {
    (
        neighbours.joint_modes(),
        neighbours.y_mode_index_ctx(),
        neighbours.fsc_mode_ctx(),
        neighbours.use_dip_ctx(),
        neighbours.mrl_index_ctx(),
        neighbours.mrl_sec_index_ctx(),
        neighbours.smoothness(),
    )
}

fn uv(edges: UvEdges) -> impl Debug {
    (edges.cfl_ctx(), edges.smoothness())
}

fn size4(size: BlockSize) -> (usize, usize) {
    (size.num_4x4_wide().unwrap(), size.num_4x4_high().unwrap())
}

fn clip(start: usize, len: usize, bound: Range<usize>) -> Range<usize> {
    start.max(bound.start)..(start + len).min(bound.end)
}

impl Recorder {
    fn leaf(
        &mut self,
        frontier: &DecodeBlockFrontier,
        neighbours: &LeafNeighbours<'_>,
        grid: &NeighbourMvGrid,
        frame_is_intra: bool,
    ) {
        let [
            mv,
            partition,
            chroma_misize,
            intra_nbrs,
            uv_edges,
            palette,
            collocated,
            _,
        ] = &mut self.digests;
        let edges = neighbours.edges;
        let (r, c) = (frontier.r, frontier.c);
        let tile_rows = grid.origin_row..grid.origin_row + grid.mi_rows;
        let tile_cols = grid.origin_col..grid.origin_col + grid.mi_cols;
        self.last_row = r;
        self.fresh = true;
        self.counts[0] += 1;
        if frontier.is_chroma_part() {
            self.counts[1] += 1;
            self.counts[2] += usize::from(!frame_is_intra);
            collocated.add((
                (r, c),
                neighbours.collocated.y_mode_facts_at(r, c),
                neighbours.collocated.uses_fsc_at(r, c),
            ));
            let (w, h) = size4(frontier.b_size);
            for rr in clip(r, h, tile_rows.clone()) {
                partition.add(partition_left(edges.partition_neighbours(1, rr, c, 0, 0)));
            }
            for cc in clip(c, w, tile_cols.clone()) {
                partition.add(partition_above(edges.partition_neighbours(1, r, cc, 0, 0)));
            }
        } else {
            let (w, h) = size4(frontier.b_size);
            mv.add(grid.immediate_flags(r, c, w, h).map(flags));
            intra_nbrs.add(intra(edges.intra_neighbours(r, c, w, h)));
            for rr in clip(r, h, tile_rows.clone()) {
                mv.add(flags(grid.immediate_flags(rr, c, 1, 1)[0]));
                partition.add(partition_left(edges.partition_neighbours(0, rr, c, 0, 0)));
                intra_nbrs.add(intra(edges.intra_neighbours(rr, c, 1, 1)));
            }
            for cc in clip(c, w, tile_cols.clone()) {
                mv.add(flags(grid.immediate_flags(r, cc, 1, 1)[1]));
                partition.add(partition_above(edges.partition_neighbours(0, r, cc, 0, 0)));
                intra_nbrs.add(intra(edges.intra_neighbours(r, cc, 1, 1)));
            }
            if r & (self.sb_h4 - 1) == 0 {
                for col in clip(c.saturating_sub(1), w + 1, tile_cols.clone()) {
                    mv.add(
                        grid.sb_border_flags(c, r.wrapping_sub(1), [col, col])
                            .map(flags),
                    );
                }
            }
            let (cache, len) = edges.palette_cache(r, c);
            self.counts[3] += usize::from(len != 0);
            palette.add(&cache[..len]);
        }
        if frontier.has_chroma || frontier.is_chroma_part() {
            let chroma = frontier.chroma_ref_geometry();
            let (row, col) = (chroma.row(), chroma.col());
            let (w, h) = size4(chroma.size());
            uv_edges.add(uv(edges.uv_edges(row, col)));
            for rr in clip(row, h, tile_rows) {
                uv_edges.add(uv(edges.uv_edges(rr, col)));
                chroma_misize.add(partition_left(edges.partition_neighbours(1, rr, col, 0, 0)));
            }
            for cc in clip(col, w, tile_cols) {
                uv_edges.add(uv(edges.uv_edges(row, cc)));
                chroma_misize.add(partition_above(
                    edges.partition_neighbours(1, row, cc, 0, 0),
                ));
            }
        }
    }

    fn resolved_unit(&mut self, grid: &NeighbourMvGrid) {
        if !core::mem::take(&mut self.fresh) {
            return;
        }
        let origin = grid.origin_row;
        let top = self.last_row - (self.last_row - origin) % self.sb_h4;
        let rows = top
            .saturating_sub(crate::tile::SB_ROW_WINDOW_ABOVE_ROWS)
            .max(origin)..(top + self.sb_h4).min(origin + grid.mi_rows);
        let plane = &mut self.digests[READERS.len() - 1];
        for r in rows {
            for c in grid.origin_col..grid.origin_col + grid.mi_cols {
                let (ri, ci) = (r as i32, c as i32);
                plane.add((grid.flags_at(ri, ci).map(flags), grid.base_c_at(ri, ci)));
                let cell = grid.get(ri, ci);
                plane.add(cell.map(|cell| {
                    let m = cell.motion;
                    (
                        flags(cell.flags),
                        [m.mv, m.mv1, m.sub_mv, m.sub_mv1],
                        m.warp_params(),
                        [m.is_global_mv(0), m.is_global_mv(1)],
                        m.cwp_weight,
                        [m.base_r, m.base_c, u32::from(m.bw4), u32::from(m.bh4)],
                    )
                }));
                plane.add(grid.probe(ri, ci).map(|leaf| {
                    (
                        flags(leaf.flags()),
                        leaf.base_c(),
                        leaf.cwp_weight(),
                        leaf.stored_warp(),
                        leaf.sub_mvs(),
                        [leaf.is_global_mv(0), leaf.is_global_mv(1)],
                    )
                }));
                if let Some(cell) = cell.filter(|cell| {
                    r >= top && (cell.motion.base_r, cell.motion.base_c) == (r as u32, c as u32)
                }) {
                    let f = cell.flags;
                    let kind = if !f.is_inter() {
                        4
                    } else if f.ref_frame0() == super::INTRABC_REF_FRAME {
                        9
                    } else if f.ref_frame0() == super::TIP_REF_FRAME {
                        7
                    } else if f.ref_frame1().is_some() {
                        6
                    } else if f.is_warp() {
                        8
                    } else {
                        5
                    };
                    self.counts[kind] += 1;
                }
            }
        }
    }
}

/// Records the neighbour state the leaf at `frontier` reads, before its parse.
pub(crate) fn observe_leaf(
    frontier: &DecodeBlockFrontier,
    neighbours: &LeafNeighbours<'_>,
    grid: &NeighbourMvGrid,
    sb_h4: usize,
    frame_is_intra: bool,
) {
    RECORDER.with_borrow_mut(|recorder| {
        if let Some(recorder) = recorder {
            recorder.sb_h4 = sb_h4;
            recorder.leaf(frontier, neighbours, grid, frame_is_intra);
        }
    });
}

/// Records the motion plane of the superblock row just resolved and the
/// window rows above it.
pub(crate) fn observe_resolved_unit(grid: &NeighbourMvGrid) {
    RECORDER.with_borrow_mut(|recorder| {
        if let Some(recorder) = recorder {
            recorder.resolved_unit(grid);
        }
    });
}

fn digest_line(name: &str) -> String {
    let path = format!(
        "{}/../../tests/conformance/vectors/valid/{name}.ivf",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(&path).expect(&path);
    let (recorder, decoded) = splot_parallel::run_on_thread("neighbour-state", || {
        RECORDER.set(Some(Recorder::default()));
        let options = crate::DecodeOptions::default();
        let session = crate::pipeline::DecodeSession::new(core::num::NonZeroUsize::MIN, None);
        let mut source =
            crate::bitstream::byte_stream::ReaderUnits::new(bytes.as_slice(), options.limits());
        let decoded =
            crate::pipeline::emit_frames(&mut source, &options, &session, |_| Ok(()), |_| Ok(()))
                .is_ok();
        (RECORDER.take().unwrap(), decoded)
    })
    .unwrap();
    let mut line = format!("{name}: decoded={decoded}");
    for (reader, digest) in READERS.iter().zip(recorder.digests) {
        write!(line, " {reader}={:016x}", digest.0).unwrap();
    }
    for (kind, count) in COUNTS.iter().zip(recorder.counts) {
        write!(line, " {kind}={count}").unwrap();
    }
    line
}

const EXPECTED: &[&str] = &[
    "syn-orderhint-wrap-64x64: decoded=true mv_edges=4fc1a1347ed3fee5 partition=01fb07d4f0702f7b chroma_misize=debb2717ada5e222 intra_nbrs=d8e89f1d164920ba uv_edges=22724ddc19bb535c palette=e4dc592b5c8cde52 collocated=b08199c0b017842a mv_plane=b6ed25befad27aaa leaves=3754 chroma_part=154 inter_frame_chroma_part=101 palette_leaves=70 intra=439 inter=2225 compound=854 tip=54 warp=14 intrabc=14",
    "syn-sdp-intra-128x128: decoded=true mv_edges=c4477937d810fe5d partition=cc55b4f61f448e00 chroma_misize=13efa6b3cfc623af intra_nbrs=0bf1a278833a0dab uv_edges=7888ddb297eb9e1c palette=55fd8e5ef9650153 collocated=feb1dd1100eea3af mv_plane=71908972e831c237 leaves=402 chroma_part=128 inter_frame_chroma_part=0 palette_leaves=0 intra=274 inter=0 compound=0 tip=0 warp=0 intrabc=0",
    "syn-bru-multitile-256x128-q220: decoded=true mv_edges=786bfb42472f5855 partition=2f65cb6331b5bc30 chroma_misize=7154688fb528c57c intra_nbrs=423f311042aefeaa uv_edges=06e7670cf990f64c palette=9ff9c6bfb206a76c collocated=f080247b55fb7b3a mv_plane=863c0a52060d70c1 leaves=860 chroma_part=105 inter_frame_chroma_part=1 palette_leaves=0 intra=219 inter=106 compound=0 tip=15 warp=0 intrabc=415",
    "syn-4frame-refmvbank-compound-256x128: decoded=true mv_edges=054c3c0ed603b519 partition=5d0d4d8efc123e21 chroma_misize=9592e5fbc78ad2a1 intra_nbrs=35d1c0b5a67a4d3c uv_edges=da315279b64bbba7 palette=66b7d79eb8041043 collocated=4f27cf23ca07a984 mv_plane=2cb7f9743c78a82e leaves=1757 chroma_part=347 inter_frame_chroma_part=0 palette_leaves=0 intra=18 inter=584 compound=138 tip=0 warp=81 intrabc=589",
    "syn-2frame-multirow-inter-64x256-10bit-q100: decoded=true mv_edges=7382f0f173097a55 partition=d2e799f00ceb9a97 chroma_misize=b9d18888a0ef38e1 intra_nbrs=2fa4b1d5896d7971 uv_edges=34823c62259b60c6 palette=ccdb6302d6ee51e9 collocated=73f3e09a5128bbeb mv_plane=9cd73e5cd05f071e leaves=373 chroma_part=193 inter_frame_chroma_part=15 palette_leaves=73 intra=99 inter=74 compound=0 tip=0 warp=1 intrabc=6",
    "syn-fsc-intra-128x128: decoded=true mv_edges=09372fcb4dfd87c5 partition=452a71287b2af442 chroma_misize=12d22324e426f607 intra_nbrs=d19978dbb210a9d0 uv_edges=a3edd55c123043d4 palette=b991db2eef4802d4 collocated=cbf29ce484222325 mv_plane=b673f97e300786cd leaves=164 chroma_part=0 inter_frame_chroma_part=0 palette_leaves=58 intra=116 inter=0 compound=0 tip=0 warp=0 intrabc=48",
    "syn-4frame-sb256-offframe-chunk-inter-264x136-q100: decoded=true mv_edges=cfe940c113bab52a partition=61dec0e1bf9c93e7 chroma_misize=b45f044536b275a5 intra_nbrs=17960c48dd68f83f uv_edges=14d4fd32d6b3748d palette=46a5bedbee8679d4 collocated=fd0e2795b7dbfd4d mv_plane=ef55ef5fcf4a0975 leaves=68 chroma_part=25 inter_frame_chroma_part=0 palette_leaves=8 intra=28 inter=8 compound=0 tip=2 warp=3 intrabc=2",
];

#[test]
fn neighbour_state_digests_match_the_reference() {
    let print = std::env::var_os("SPLOT_PRINT_NEIGHBOUR_DIGESTS").is_some();
    for expected in EXPECTED {
        let name = expected.split(':').next().unwrap();
        let actual = digest_line(name);
        if print {
            println!("    \"{actual}\",");
            continue;
        }
        let expected: Vec<_> = expected.split(' ').collect();
        let differs: Vec<_> = actual
            .split(' ')
            .filter(|token| !expected.contains(token))
            .collect();
        assert!(differs.is_empty(), "{name}: now {}", differs.join(" "));
    }
}
