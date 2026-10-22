# Changelog

Encoder milestones. Rate points are luma PSNR and kilobits per frame from
`splot encode --stats`; BD-rate is against AVM on the same clip and settings.

## Unreleased

### Encoder milestone 3: single-reference inter blocks

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
