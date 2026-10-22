# Changelog

Encoder milestones. Rate points are luma PSNR and kilobits per frame from
`splot encode --stats`; BD-rate is against AVM on the same clip and settings.

## Unreleased

### Encoder 4:2:2 and 4:4:4

- `splot encode` codes 4:2:2 and 4:4:4 Y4M input (`C422`, `C444`, `p10`
  forms) at 8 and 10 bits, key and inter frames. Monochrome input fails with
  `UnsupportedChroma`. 4:2:0 output is byte-identical to before.
- The `encoder_syntax` facade takes the subsampling from the sequence header
  for its frame workspace, `BlockDecoded` state, chroma smooth grid and
  rollback rectangles.
- park_joy 160x90, 10 frames, preset 6, `--qp 120`, `--keyint 30`; `splot
  decode` and `avmdec` outputs are byte-identical:

  | Clip | Bits/frame | PSNR Y / U / V (dB) |
  |---|---|---|
  | 8-bit 4:2:0 (reference) | 6330 | 35.20 / 38.77 / 41.31 |
  | 8-bit 4:2:2 | 6488 | 35.21 / 38.93 / 41.20 |
  | 8-bit 4:4:4 | 7042 | 35.19 / 38.68 / 41.44 |
  | 10-bit 4:2:2 | 23958 | 45.92 / 47.13 / 48.03 |
  | 10-bit 4:4:4 | 28859 | 45.85 / 47.06 / 47.79 |

### Encoder milestone 5: threading, SIMD coverage, perf harness

- Tiles: `--tiles COLSxROWS` codes uniform tiles in parallel, one facade per
  tile. Niklas 720p, preset 8: BD-rate +1.3/+2.5/+3.7% at 2/4/8 tiles.
- Frame waves: frames that read the same references code in parallel and
  commit in coding order. Shown leaf frames are non-reference frames, except
  the last leaf of each mini-GOP (§ 5.18.2 order hints): BD-rate -0.31%.
- Output is byte-identical at every thread count; tests check 1/2/8 threads
  with one and two tiles.
- Fix: encoder grid checkpoints lost the motion records of the best partition
  candidate after a rollback, so later trials read wrong neighbour vectors
  (since milestone 3). Inter frames now use 20-34% fewer bits at equal PSNR.
- Speed: region-local snapshots, less trial waste, and SIMD forms of the
  residual, forward-transform pair sums, directional intra (zone 2, bilinear)
  and CfL prediction. Paris CIF, 17 frames, preset 6, base_q_idx 120:
  milestone 4 took 120 s at 1 and 8 threads; now 90 s at 1 thread, 69 s at
  8 threads, 30 s at 8 threads with 2x2 tiles.
- SIMD: every encoder kernel has a scalar twin under bit-exact property tests;
  93.6% of hot-kernel samples are in SIMD code (`encode-perf --coverage`).
- Harness: `cargo xtask encode-perf` (instruction counts, CI perf job) and
  `cargo xtask bd-rate`.
- Against AVM `--cpu-used=6`, base_q_idx 85..190, luma BD-rate:

  | Clip | Coding | splot preset | BD-rate |
  |---|---|---|---|
  | park_joy 160x90, 10 frames | intra | 0 / 6 / 10 | -4.7% / +2.6% / +24.9% |
  | paris CIF, 5 frames | intra | 6 / 10 | +10.4% / +52.9% |
  | park_joy 160x90, 10 frames | inter, keyint 30 | 6 | +19.8% (milestone 3: +37.7%) |
  | paris CIF, 10 frames | inter, keyint 30 | 6 | +30.7% |

  Inter AVM runs use `--lag-in-frames=16`; AVM also codes tools splot does
  not code yet (temporal MVs, warped and overlapped motion, optical flow, TIP).

### Encoder milestone 4: lookahead, temporal model and rate control

- A lookahead plans exactly `lookahead_frames` frames, so output ignores the
  call pattern. A frame whose best costs against the previous frame reach 70%
  of its intra costs, 20 points above the previous frame's, and that is a
  mini-GOP past the last key frame, starts a key frame (noise makes none).
- A temporal model propagates block costs back through the planned mini-GOP
  references. Frames that later frames reuse get up to 32 lower qindex, and
  deeper mini-GOP layers get 3 more per layer.
- Rate control: `--crf N` (base qindex `4 * N`, plus 48 at 10 bits, plus the
  offsets), `--bitrate KBPS` with `--rc vbr|cbr`, or a caller's controller.
  VBR and CBR fit a base qindex per window to a learnt model; drift is bounded.
- Rate points, preset 6, 8 threads, keyint 120, luma, CRF 22/30/37/45
  against constant qp 80/110/140/170 at matched average bitrate: park_joy
  160x90 10 frames BD-rate -7.2%; niklas 720p 10 frames BD-rate -8.1%.
  Bitrate accuracy, park_joy 300 kbps: VBR +9.0%, CBR +0.3%.

### Encoder milestone 3: inter coding

- Inter frame headers are parse-backed like key frames; `EncoderReferences`
  wraps the decoder's own reference buffer, so the encoder never re-derives
  § 7.7/§ 7.23 reference state.
- Hierarchical mini-GOPs (preset `mini_gop_size` 8/4/2/1): each mini-GOP codes
  its last frame first as a hidden frame, then the middle frames, and shows
  hidden frames with show-existing frames; each packet is one § 7.3.6
  temporal unit in coding order, and `--keyint` starts key frames.
- Inter-frame leaves RD-compare intra coding with GLOBALMV, NEARMV over the
  DRL, and NEWMV, from every valid reference. NEWMV takes a pyramid
  full-sample search with half- and quarter-sample refinement through the
  decoder's interpolation. `skip_txfm` and the inter transform sets are coded.
- `encoder_syntax` runs the decoder's own MV stack, resolution, motion
  compensation and inter residual reconstruction; publication fails when the
  decoder resolves another vector. A new MV writer inverts `read_mv`.
- Rate points against all-intra inter frames, 10 frames, keyint 30, one
  thread, base_q_idx 80/120/160, luma: park_joy 160x90 preset 6 BD-rate
  -34%, encode time +64..93%; niklas 720p preset 10 BD-rate -43%, encode
  time +26..71%.
- Compound and skip-mode blocks: inter frames set `reference_select` and
  `skip_mode_present` when the preset's `compound_candidates` is nonzero.
  Leaves add skip-mode trials and a SATD-ranked compound shortlist
  (COMPOUND_AVERAGE; NEAR_NEARMV over both DRLs, GLOBAL_GLOBALMV, NEW_NEWMV,
  NEAR_NEWMV, NEW_NEARMV, JOINT_NEWMV) over the pairs of the three closest
  references, seeded by the single-reference search. The facade writes
  `comp_ref`, the modes and the DRLs by answering the decoder's own reads.
- Rate points against single-reference only, preset 6, 10 frames, keyint 30,
  mini-GOP 4, base_q_idx 80/120/160/200, luma: niklas 720p BD-rate -4.5%,
  encode time +8..17%; park_joy 160x90 BD-rate -0.2%, encode time +1..22%.
- Against AVM `--cpu-used=6 --kf-max-dist=30 --lag-in-frames=16`, park_joy
  160x90, 10 frames, base_q_idx 85..190, preset 6: luma BD-rate +37.7%;
  splot 0.6-3.8 fps, AVM 0.12-0.23 fps. AVM also uses temporal MVs, warped
  and overlapped motion, optical flow and TIP, which splot does not code yet.

### Encoder milestone 2: full intra coding

- Every AV2 intra mode: all 61 luma mode and angle-delta pairs, every chroma
  mode, explicit and derived CFL. Prediction and reconstruction run the
  decoder's own per-transform-block code through `encoder_syntax`.
- RD search: a recursive partition search, luma mode shortlist by SATD then
  full RD trials, chroma and CFL, transform partitions (TX_MODE_SELECT) and
  luma transform types of the selected set, RDOQ with CDF-backed rates.
  Trials run the tile writer's own syntax walk into a rate sink and roll
  back with a facade snapshot; no second syntax walk exists.
- Loop filters: deblocking, CDEF strength sets and luma PC-Wiener are searched
  on the reconstruction and coded in a second pass.
- Presets gate mode shortlists, transform-type depth, full trials, partition
  early exit and filter search from the preset table.
- Rate points (paris CIF, 5 frames, all-intra, base_q_idx 85..185, AVM
  `--cpu-used=6`), luma BD-rate: preset 0 -1.9%, preset 6 +10.3%, preset 10
  +54.1% (milestone 1: +66.9%). Speed, CIF, one thread: preset 0 0.03 fps,
  preset 6 0.27 fps, preset 10 2.3 fps; speed is milestone 5 work.

### Encoder milestone 1: intra-only DC stream

- `splot encode <in.y4m> -o <out.ivf>` with `--qp`, `--preset`, `--limit`,
  `--keyint`, `--stats`, `--threads`.
- `splot_encode::Encoder` (`send_frame`, `flush`, `receive_packet`), `Config`,
  speed presets 0-10 as one data table, `RateController` (constant QP),
  per-frame bits, PSNR and SSIM, Y4M reader and IVF writer.
- Every frame is a closed-loop key frame of any size, 8- or 10-bit 4:2:0,
  with `DC_PRED`, `DCT_DCT` and a variance-driven partition choice.
- The key-frame tile writer uses the decoder's CDF rows, contexts and partition
  rules through `splot_decode::encoder_syntax`. Tests require the encoder
  reconstruction to equal `splot-decode` output; AVM decodes the same samples.
- Forward transforms, the quantizer and SAD/SSE/SATD have scalar and
  `std::simd` versions behind one trait, compared by property tests.
- The symbol encoder streams in O(1) per symbol (1M symbols: 64 s -> 25 ms).
- Rate points, `paris_352_288` (CIF), 5 frames, all-intra, preset 6 against
  AVM `--cpu-used=6 --kf-max-dist=0` at the same base_q_idx 85..185:
  BD-rate +66.9% (luma; splot codes DC only, AVM all intra tools).
  Speed: splot 100-200 fps CIF and 25 fps 720p on one thread; AVM 0.07 fps CIF.
