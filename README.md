# SIMD playback performance evidence

November 25, 2026 evidence snapshot for [performance tracker #1934](https://github.com/splotlabs/splot/issues/1934) and [SIMD epic #1882](https://github.com/splotlabs/splot/issues/1882). Distribution retains the **x86-64-v3 floor**. The selected direction permits **startup-selected runtime AVX-512 on capable hardware**, with AVX2 for narrower work. The local playback comparisons below ran on an AVX2-only i7-12700H. Qualification of actual wide compute comes from separate native hardware experiments.

[Download the measurements](evidence/measurements.tar.xz) · [Member manifest](evidence/files.tsv) · [Archive checksums](evidence/SHA256SUMS) · [Packaging record](evidence/packaging.txt)

The bundle includes full and partial timing samples, Callgrind traces, native gprofng PC, function and call-tree exports, assembly notes, conformance checks, artifact sizes, and reproduction scripts. Earlier snapshots remain in their original relative locations. `updated/` preserves newer versions when a local record grew or changed. Original archives and profiler experiments remain on the measurement host. Publication copies omit captured environments, machine identifiers, credentials, third-party source checkouts, opaque experiment containers, executables, and media inputs. The numerical native profile exports remain usable for attribution.

The local original was verified against its 599-member manifest before packaging. Its SHA-256 is `b0a9321a6731de39525e5edaf843c7ca72cbbf5d7caa3b3fd67ecce53ab2b8b7`. The member manifest linked above covers the sanitized copy.

## Same-floor v3 comparison

The baseline is main [`d171d824`](https://github.com/splotlabs/splot/commit/d171d82416a67140e1f6696eeba8e08e56af5763). Both comparison columns build draft [`29e00167`](https://github.com/splotlabs/splot/commit/29e001679265816c1e0fed9e09a5e32efe1a2d47) globally at v3. Variant A uses `--cfg disable_dispatch_avx512` and variant B omits that cfg.

Compiler flags for both variants include `-C target-cpu=x86-64-v3 -C link-arg=-Wl,-z,pack-relative-relocs`. Rust is 1.100.0. Measurements use 12 counterbalanced paired rounds on CPU 4, one decoder worker, frame delay 1 and null output. Values are median paired elapsed-time changes and 95% bootstrap intervals versus main. Positive values mean longer elapsed time.

| Stream | Frames | A vs main | B vs main |
|---|---:|---:|---:|
| WITCHER3 | 40 | −0.85% [−6.67%, +1.74%] | +2.77% [−0.66%, +4.80%] |
| TunnelFlag | 40 | +2.06% [−2.99%, +3.97%] | +4.22% [+1.51%, +7.82%] |
| Noise_Ocean | 64 | +3.33% [−3.15%, +5.23%] | −1.40% [−6.88%, +6.37%] |
| RitualDance | 64 | +2.75% [−1.45%, +5.16%] | +2.55% [−3.03%, +8.27%] |

The separate 16-frame Callgrind comparison measured **+0.544% on Noise_Ocean** and **+2.027% on RitualDance** for both draft configurations versus main. On RitualDance, the same 441,163 calls per row position to the rough transposed GDF kernel cost 425 instructions each in the draft and 384 in main. The two row positions account for 36,175,366 extra instructions. Their linked bodies already have direct AVX2 code at v3. The draft row shows greater register spill traffic. The artifact set also contains the merge-base control [`d49defcf`](https://github.com/splotlabs/splot/commit/d49defcf1768fc83db0ff6892421ad16eeeedf2e).

Data and commands are in `v3-screen/paired.csv`, `v3-screen/paired.txt`, `v3-screen/instructions.txt`, `v3-screen/playback-*/flags.txt`, and `v3-screen/bench.py` in the archive.

## Legacy portable compilation

The portable floor made the strict SSE2 CLI and external C consumer possible. AVX2 playback remained slower on the i7-12700H. The following medians and paired 95% intervals use the shipped-v3 reference [`10279846`](https://github.com/splotlabs/splot/commit/102798461d19a98b6e04b935538fb24f7b339701), 12 rounds per stream, one worker and null output.

| Stream | Frames | f03 portable vs shipped | 7c50 portable vs shipped |
|---|---:|---:|---:|
| WITCHER3 | 40 | +19.42% [+10.31%, +33.05%] | +9.46% [+5.60%, +12.21%] |
| TunnelFlag | 40 | +9.93% [+4.35%, +27.14%] | +10.30% [+8.35%, +11.30%] |
| Noise_Ocean | 16 | +23.98% [+4.91%, +34.77%] | +13.53% [+4.68%, +16.26%] |
| RitualDance | 16 | +16.74% [+10.66%, +23.02%] | +11.98% [+7.56%, +19.04%] |

The older signed head is [`f03b06e2`](https://github.com/splotlabs/splot/commit/f03b06e2419b27cf8ab4b55c62471695513cdbe1). The later merge [`7c50c1b4`](https://github.com/splotlabs/splot/commit/7c50c1b48e1bb930602f249be029a9768f196aa3) combines upstream changes with [TIP motion scaling](https://github.com/splotlabs/splot/commit/183e04ea245a911ca2110ad3ab3428419cb94137) and smooth intra scoping. All 21 full AVM checks and all 40 strict-SSE2 CLI/C checks passed on that merge. Its locally GNU-stripped CLI and C library measured 16,759,728 and 12,943,720 bytes. The later portable CI allowances passed, while the five-stream instruction gate measured +13.378% over its base. [Portable draft](https://github.com/splotlabs/splot/pull/1987) · [CI measurement](https://github.com/splotlabs/splot/actions/runs/45654396017/job/139347080197)

The older parser, subpel, GDF and CDEF comparisons are preserved as separate series. They use different heads, frame windows and controls. Read `parser-issue.md`, `kernels-issue.md`, `module-issue-updated.md`, and their associated CSV files for those settings. The published history also records the initial WITCHER3 and TunnelFlag portable screens at 2.06× and 2.70× elapsed and the revised screen at 1.335× and 1.582×. [Earlier results](https://github.com/splotlabs/splot/issues/1882#issuecomment-7159993595) · [CDEF module measurements](https://github.com/splotlabs/splot/issues/1934#issuecomment-7163536585)

## Latest four scoped changes

Signed screen head [`edff2502`](https://github.com/splotlabs/splot/commit/edff25026f6151ba86cafefd1c1059239a5da068) adds a build-only workflow after the four changes below. Its control is [`29e00167`](https://github.com/splotlabs/splot/commit/29e001679265816c1e0fed9e09a5e32efe1a2d47).

1. [Padded CDEF fallback work](https://github.com/splotlabs/splot/commit/3c37b3185c83fcde1d1d144922e8a8c42fa196e6)
2. [Deblock width decisions](https://github.com/splotlabs/splot/commit/cf8a436859467108f6041a9654695c6ca0e43723)
3. [Supplied compound blend](https://github.com/splotlabs/splot/commit/53f572dfea3ddf98d458b1245f55954c94ce20d6)
4. [TIP refinement units](https://github.com/splotlabs/splot/commit/ad35f115523cb562b0b4b88ccfbedaf979db73e0)

The batch matched full Noise_Ocean and RitualDance AVM hashes and byte counts. Over 16 frames it reduced whole-decode Ir by **8,125,108 on Noise_Ocean** and **189,280,331 on RitualDance** versus the draft. Matched RitualDance edges decreased by 98,787,932 Ir for TIP refinement, 39,278,718 Ir for compound blend, 44,338,250 Ir for deblocking, and 4,520,616 Ir for padded chroma CDEF. On Noise_Ocean, the matched compound edge increased by 27,653,161 Ir even as CDEF decreased by 17,610,168 Ir. These edge comparisons are from one batch containing all four changes. The linked CLI/C library in this screen measured 16,689,496 and 12,869,200 bytes under the screen's size check.

The playback window ended early. The archive retains **six completed four-stream rounds and one extra Noise_Ocean triplet**. Partial candidate-versus-shipped medians were +12.99% WITCHER3, +5.37% TunnelFlag, +14.67% Noise_Ocean, and +15.05% RitualDance. They are descriptive samples from the incomplete run. Four-stream playback completion and release qualification are outstanding. See `playback-screen/paired.csv`, `playback-screen/oracle.txt`, and `playback-screen/edges.py`.

## Failed, mixed, and incomplete experiments

- **GDF register pressure.** The prior tap-order candidate increased instructions on both affected clips. An explicit-argument leaf recovered 0.254% on WITCHER3 and 0.224% on TunnelFlag versus its portable control, with higher spill traffic. The matched v3 gap remains. See `gdf-instructions.txt`, `gdf-explicit-leaf/`, and the v3 traces.
- **Whole temporal-scan scope.** The signed local candidate recorded +1.27% Noise_Ocean and +0.38% RitualDance instructions and grew the stripped CLI by 13,088 bytes. Both full raw outputs matched. Native timing was skipped after the regression. See `temporal-scan/`.
- **CDEF calling conventions and tables.** The direct complete-block test increased Noise_Ocean wall time by +2.25% [+0.78%, +3.35%]. Its RitualDance interval included zero. Other CDEF context, table, fallback and calling-convention runs have separate raw records. See `cdef-boundary-ab/`, `cdef-context/`, and `cdef-fallback/`.
- **Restoration module and matched playback.** The independent restore layouts, compiler-attribution screens and consumer comparisons remain in `lr-*`, `compiler-attribution/`, and `dav2d-native/`. Their source revisions, linked section changes and timings are kept apart because their controls differ.
- **Sample outliers.** The native call-tree reports preserve every capture. One historical shipped pair concentrated 4.624 seconds on two PCs and was excluded. A later shipped pair concentrated 259 ms at one scheduling-function entry and was replaced. In the clean later comparison, temporal projection rose 21.3 ms and the entire CDEF group rose 71.3 ms. See `native-stacks/results.md` and `current-native/results.md`.
- **Prior runtime wide experiment.** The Wiener luma VNNI [`02c20b54`](https://github.com/splotlabs/splot/commit/02c20b54265acf4125b405a5bcc5a9a87399b3d0) measured +0.023% geometric elapsed change across 168 forced-policy pairs on an Intel Xeon 6975P-C KVM guest. RitualDance was slower at 1, 7 and 12 threads and both linked artifacts exceeded the then-current caps. The preserved [Xeon timing CSV](evidence/xeon-vnni-pairs.csv) contains the raw elapsed measurements. [Native timing job](https://github.com/splotlabs/splot/actions/runs/45590192972/job/139122054646) · [Decision](https://github.com/splotlabs/splot/issues/1891#issuecomment-7159816019)

## Reproduction

Verify the archive before extraction.

```sh
(cd evidence && sha256sum -c SHA256SUMS)
mkdir measurements
tar -xJf evidence/measurements.tar.xz -C measurements
```

Use the signed revisions and archived compiler flags for each series. For the linked symbolized v3 playback build:

```sh
RUSTFLAGS='-C target-cpu=x86-64-v3 -C link-arg=-Wl,-z,pack-relative-relocs --cfg disable_dispatch_avx512' \
  CARGO_PROFILE_RELEASE_STRIP=none cargo +1.100.0 build --release --locked -p splot-cli
taskset -c 4 ./target/release/splot decode INPUT.ivf \
  --threads 1 --frame-delay 1 --limit FRAMES --output-format null
```

Use the alternative archived flag line for the second v3 variant. Match the input manifest, link settings, frame prefix and selected backend. Reuse the archived counterbalanced scripts once the binaries and inputs are placed in their expected relative directories. `<WORKTREES>`, `<CI_WORKSPACE>` and `<LOCAL_PATH>` stand for sanitized paths. For instruction counts, use Valgrind 3.26.0 Callgrind with `--limit 16` and the same decoder flags. The native exports use gprofng one-millisecond clock samples on CPU 3, one worker, frame delay 1 and null output. The complete-stream AVM fixture hashes are in `ci-data/simd-cases.tsv`.

Portable throughput remediation, the incomplete four-change playback matrix, the matched GDF spill regression, and separate timing of runtime AVX-512 on capable hosts remain open. [Issue #1934](https://github.com/splotlabs/splot/issues/1934) · [Wide rollout gate #1894](https://github.com/splotlabs/splot/issues/1894)
