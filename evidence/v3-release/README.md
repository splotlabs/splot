# x86-64-v3 release qualification

[Raw measurements and disassembly](measurements.tar.xz) · [Per-file checksums](files.tsv) · [Archive checksums](SHA256SUMS)

The snapshots retain adverse and inconclusive results. The Xeon runs compare forced AVX-512 and AVX2 in the same binary. The AVX2 workstation runs compare decoder revisions. Each record identifies the input, build and comparison method.

| Revision | Measurement |
|---|---|
| b337fafe | Frozen pre-epic x86-64-v3 and Apple M2 Max reference |
| d171d824, 9df30e91 | Released v3 comparator. Both produced byte-identical matched Ubuntu CLI and C library artifacts |
| a55cff38 | Initial v3 candidate with wide grain and an ISA tag at display output |
| 3cddb13a | Candidate carrying the selected backend through the display loop |
| c9a9a960 | Typed candidate supporting SSE2 when custom AVX2 compiler flags guarantee only part of the v3 feature set |
| 83566cc2 | Same decoding algorithms with measured artifact budgets, a build-floor policy check and a native timing workflow update |

## Results

- All 63 native streaming hash comparisons passed for the fully typed decoder on a Xeon 6975P-C using Auto, AVX2 and AVX-512 across seven frozen streams and three worker counts. Auto selected AVX-512 on this host. [Correctness and backend selection](https://github.com/splotlabs/splot/actions/runs/45674678371).
- In the first complete Xeon timing matrix, the AVX-512 grain row used 14.73% less elapsed time at 8 bits and 19.62% less at 10 bits. For the full grain stream, one-worker decode took 0.59% less time [−0.70%, −0.32%]. BlueSky at 12 workers took 1.45% more time [+0.14%, +2.43%]. The archive includes all 21 workload and worker combinations. [Initial matrix](https://github.com/splotlabs/splot/actions/runs/45670915732).
- Ten further pairs for typed grain output on native hardware gave −0.58%, −1.45% and −0.38% at 1, 7 and 12 workers. All three 95% intervals include zero. The two row medians were −14.14% and −18.54%. [Typed output measurements](https://github.com/splotlabs/splot/actions/runs/45673211431).
- AVX2 Callgrind measurements of the typed candidate's 16-frame windows ranged from −0.968% to +0.104% against the frozen build and from −0.392% to −0.001% against main. OldTown64 took 3.06% longer [+1.10%, +5.09%] when comparing two experimental builds. The final candidate took 1.75% less elapsed time [−3.06%, −0.37%] against current main across 20 alternating blocks. The candidate also ran at a higher median clock. The archive preserves CPU frequency observations and individual samples for both comparisons where recorded.
- The hosted Apple M1 VM passed 42 complete per-revision AVM hash checks, native lifecycle tests and the C consumer. Four eight-pair timing intervals included zero. [Apple M1 comparison](https://github.com/splotlabs/splot/actions/runs/45670988686).
- The final matched stripped CLI grew 43,328 bytes (+0.348%) and the C library grew 34,016 bytes (+0.398%). The linked audit covers 33 paths, the two wide grain leaves, code and data sections, and per-function stack reservations. [Exact artifact build](https://github.com/splotlabs/splot/actions/runs/45674741726).

The earlier portable-floor experiments remain in the [original archive](../measurements.tar.xz). The [performance tracker](https://github.com/splotlabs/splot/issues/1934) links the outstanding work.
