# Changelog

Encoder milestones. Rate points are luma PSNR and kilobits per frame from
`splot encode --stats`; BD-rate is against AVM on the same clip and settings.

## Unreleased

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
