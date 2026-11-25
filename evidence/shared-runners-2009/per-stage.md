# SIMD runner performance

Linux x86-64-v3 release profile, Rust 1.100.0 / LLVM 23.1.5, Intel i7-12700H, Callgrind 3.26.0. PMU access is blocked by the host, so the measurements count instrumented instructions, including runtime and system libraries. AVX2 is available under the instrumentor. Every measured command uses `--threads 1` and is pinned to one core. Decoder measurements use `--frame-delay 1 --output-format null` and the five stream prefixes defined by the repository performance gate. Encoder runs use deterministic synthetic Y4M, `--qp 80` and the stated presets.

Baseline and 7f27dc8b figures use the lower of two measurements. 244a52ae uses the lower of two except its single 10-bit encoder run, and em dashes indicate cases without complete repetitions and output. 503f8cb3 uses the lower of two decode counts and one count per encoder row. Encoder counts include scheduling variation. Intermediate and final output checks are byte comparisons of same-thread decode hash reports and encoded IVF files. Full stream cross-thread comparisons below compare frame records, which omit the reported thread policy.

## Decode instruction counts

| Case | Baseline Ir | 244a52ae delta | 7f27dc8b delta | 503f8cb3 delta | 503f8cb3 output |
|---|---:|---:|---:|---:|---|
| DOTA2 | 3,706,105,260 | -387 (-0.000010%) | +1,889 (+0.000051%) | -225 (-0.000006%) | match |
| RitualDance | 4,436,218,545 | -837 (-0.000019%) | +2,401,700 (+0.054138%) | -618 (-0.000014%) | match |
| Slides1 | 1,491,629,589 | -823 (-0.000055%) | +2,005 (+0.000134%) | -646 (-0.000043%) | match |
| Noise_Ocean | 1,305,669,065 | -406 (-0.000031%) | +1,883 (+0.000144%) | -244 (-0.000019%) | match |
| old_town_cross | 2,751,658,920 | -823 (-0.000030%) | +1,979 (+0.000072%) | -655 (-0.000024%) | match |

## Encoder instruction counts

| Case | Baseline Ir | 244a52ae delta | 7f27dc8b delta | 503f8cb3 delta | 503f8cb3 output |
|---|---:|---:|---:|---:|---|
| inter-128x96-p6 | 13,739,293,196 | +102,532 (+0.000746%) | +39,666,188 (+0.288706%) | +1,559,812 (+0.011353%) | match |
| inter-128x96-p10 | 1,543,864,174 | -327,841 (-0.021235%) | +4,820,114 (+0.312211%) | -58,484 (-0.003788%) | match |
| intra-176x144-p6 | 20,330,255,975 | +2,529,328 (+0.012441%) | +70,506,952 (+0.346808%) | +3,197,009 (+0.015725%) | match |
| intra-176x144-p10 | 2,100,452,706 | — | +11,550,712 (+0.549915%) | -589,131 (-0.028048%) | match |
| intra-64x64-p0 | 30,764,706,181 | — | +21,814,808 (+0.070909%) | +249,760 (+0.000812%) | match |
| inter-128x96-10bit-p6 | 18,019,203,568 | +1,002,422 (+0.005563%) | +43,737,202 (+0.242726%) | +1,112,095 (+0.006172%) | match |

## Full-stream frame parity

- DOTA2 (130 frames) matched at 1 and 12 threads across all four revisions. SHA-256 47ae28387f809213e7bc3a51d2fbfdc1771401f4b11603fa88db1e1fc93b677a
- RitualDance (600 frames) matched at 1 and 12 threads across all four revisions. SHA-256 609d20db488a07c4e27d6ccc128bdadb049aa2cb7e7982e5fd0a18c448738918

## Linked release code

- d04f4d61, 244a52ae and 503f8cb3 have byte-identical linked `.text` sections over 10,539,607 bytes. SHA-256 `3e47c5dbc905870ea86e2d0e944bfbecb2408a8b8003fb2b88369528e8fd5b5a`. The final binary also matches all loadable baseline sections except the build ID and 46 bytes of Rust source line numbers.
- Intermediate 7f27dc8b put two AVX2 u16 directional intra line kernels out of line and had a 2,672-byte smaller linked `.text`. RitualDance's directional intra routines account for 2,399,693 of the 2,402,537 extra Ir versus 244a52ae, or 99.88%.
- 503f8cb3 restored the two explicit inline attributes. The tested source matches the commit's changed files byte for byte.

Raw counts are in the TSV files accompanying this report. See `inline-elf.txt`, `stage1-elf.txt` and `final-linked-symbol-size.tsv` for the binary comparisons.

