# Decode Scaling Mission

Feature IDs: `INFRA-DECODE-PARALLEL-STAGES`,
`INFRA-DECODE-FRAME-PIPELINING`

The live implementation checklist is
[`DECODE-SCALING-OPEN-TASKS.md`](DECODE-SCALING-OPEN-TASKS.md). Add discoveries
there immediately; keep measurements and completed or rejected evidence here.

## Goal

On `/Users/bartosztomczyk/Documents/SplotLabs/test.ivf`, decode 30 frames at
1, 2, 4, 8, and 10 threads faster than dav2d, with the 10-thread splot wall
time at most 50% of dav2d's and no 1-thread regression.

This is an architectural scaling mission. A candidate is not delivered merely
because it is statistically positive. Prefer changes that remove a measured
pipeline barrier, ownership bottleneck, or serial critical path. Small scalar
wins belong in a separate ledger and are reconsidered only after the scaling
architecture is fixed.

## Benchmark contract

Splot decoder-only timing:

```sh
SPLOT_DECODE_DISCARD_HASH=1 ./target/release/splot decode \
  --quiet --output-format hash --limit=30 \
  /Users/bartosztomczyk/Documents/SplotLabs/test.ivf \
  --threads=10 --frame-delay=10
```

Dav2d timing:

```sh
/Users/bartosztomczyk/Devel/dav2d/build/tools/dav2d \
  --demuxer ivf \
  -i /Users/bartosztomczyk/Documents/SplotLabs/test.ivf \
  -o /dev/null --threads 10 -l 30 --framedelay 10
```

Use rebuilt release binaries, warmups, alternating paired samples, and an idle
machine. Prove decoded output separately from timing. A deliverable candidate
must pass exact output comparison, the required corpus/oracle checks,
`cargo xtask check-duplication`, `cargo xtask ci`, and the 1/2/4/8/10 paired
timing matrix.

## Confirmed baseline

| Threads | splot wall | dav2d wall | splot average cores |
|---:|---:|---:|---:|
| 1 | 1.0177 s | 1.0561 s | 1.00 |
| 2 | 0.7714 s | 0.5456 s | 1.35 |
| 4 | 0.3560 s | 0.2858 s | 3.23 |
| 8 | 0.2754 s | 0.1600 s | 4.54 |
| 10 | 0.2807 s | 0.1581 s | 4.61 |

The current dav2d 10-thread result is 0.152885 seconds, so the target is at most
0.076443 seconds. Perfect scaling of splot's current 0.996527-second 1-thread
result would still be approximately 0.099653 seconds, so the final result
requires both a scheduler/ownership redesign and about 23.3% less scalar CPU
work.

## Delivery discipline

Only one implementation candidate is active at a time. For each candidate:

1. Start from the current `origin/main`.
2. State the bottleneck, evidence, expected ceiling, and acceptance threshold.
3. Implement the smallest architecture change that tests the hypothesis.
4. Check exact output before timing.
5. Run the full paired thread matrix.
6. Reject and fully revert a regression or immaterial result.
7. For a material result, run `rust-simplify`, correctness gates, commit, open a
   PR, merge it, refresh `origin/main`, and only then start the next candidate.

Every newly discovered bottleneck or rejected experiment must be recorded in
the open-task checklist before work moves on. Completed and rejected evidence
is retained in the task table below.

## Task ledger

| ID | Status | Finding / task | Evidence and next action |
|---|---|---|---|
| SCALE-001 | Confirmed | dav2d uses a global queue over frame contexts and three row-progress passes: entropy, MV resolution, and reconstruction/filter progress. | With the canonical command, dav2d creates 2/4/8/10 persistent worker threads and the same number of frame contexts; the caller remains a separate admission thread. One tile task advances one superblock row, publishes, and continues or requeues. Use this as the reference model. |
| SCALE-002 | Rejected | Enable split entropy/reconstruction at two workers. | Lowering the four-worker bound to two improved 2T by 17.0% in a 9-pair diagnostic run and was byte-identical at 1/2/4/8/10T, but the full gate proved that the split path raises peak live reference storage from 110,592 to 122,880 bytes at both 2T and 3T. The source change was reverted and no code PR was opened. Revisit only after frame contexts preserve the serial reference-store peak. |
| SCALE-003 | Rejected standalone | Queue entropy work from multiple frame contexts instead of synchronously parsing one whole frame on the driver. | A three-context POC derived pending-slot geometry from the header, provisionally refreshed references with terminal CDF/CCSO handles, and scheduled entropy under exact primary/blend/CCSO dependencies. It was output-identical in focused 1/4/10T checks and frame-delay 1/2/3/10 tests, but an 11-pair 10T screen moved the median only from 294.8 ms to 286.5 ms, 2.8%; ten entropy contexts regressed. SCALE-004 did not unlock more value, so revert rather than merge this large foundation. Reintroduce pending entropy products only as part of a row-progress design that materially shortens the dependency chain. |
| SCALE-004 | Rejected | Replace the all-at-once reconstruction task dump with a bounded dav2d-style wavefront that advances and republishes one unit at a time. | An exact POC removed mutex-held commit replay with an owned baton, admitted four-unit precomputes lazily under a global pool-width budget, charged permits only after row conditions became ready, and prioritized entropy/commit before speculation. Baton ownership alone was approximately 0.8% slower. A nine-batch global window regressed approximately 2.5%; 18- and 27-batch windows only converged to neutral/slightly worse. The existing precompute/shadow-surface plus serial replay split is the wrong unit: bounding it removes useful work without advancing reference/filter publication. Revert and pursue SCALE-008's direct resumable row owner instead of tuning this scheduler. |
| SCALE-005 | Rejected standalone | Remove the whole-reference motion-field barrier with row/band publication. | A byte-exact POC published immutable full-width 64-pixel source bands, projected only the current band from the exact named references, and advanced resolve through one ordered continuation. Its raw 30-frame output matched main at SHA-256 `48f0dc140be565069838bcf7141aba3c80cefaa14400284840b2e1475a3be945`. In 11 alternating pairs, 1T moved from 0.962320 s to 0.968369 s (paired +0.38%), 8T from 0.264656 s to 0.258346 s (-2.94%), and 10T from 0.267150 s to 0.261461 s (-1.92%). One 10T trace shortened `pipeline_inflight` from 276.671 ms to 253.575 ms, but `resolve_row` rose from 18.427 ms to 27.258 ms and `mode_record` from 15.049 ms to 24.437 ms while filter/reconstruction work remained whole-frame. Motion-band publication is real and correct but insufficient standalone. Reintroduce it only inside an integrated row DAG that also publishes filter and reference rows; do not merge approximately 1,600 lines of infrastructure for a 2–3% gain. |
| SCALE-006 | Rejected standalone | Start filter progress before whole-frame reconstruction commit completes. | An exact POC introduced canonical 64-luma-row postfilter owners, early deblock/transform-skip records, segmented immutable filter sources, two-band horizontal deblock, and explicit `ReconReady -> DeblockColsDone -> DeblockRowsDone -> FilterStripeDone` admission continuations. Its raw 30-frame output matched main at SHA-256 `48f0dc140be565069838bcf7141aba3c80cefaa14400284840b2e1475a3be945`. In 11 alternating pairs, 1T moved from 0.937173 s to 0.945433 s (paired +0.28%), 8T from 0.265516 s to 0.266778 s (+0.48%), and 10T from 0.269261 s to 0.263486 s (-1.82%). One 10T trace shortened `pipeline_inflight` from 269.829 ms to 254.573 ms and total from 272.848 ms to 257.318 ms, but the paired result is below the 10% architectural acceptance bar. Filter-row eligibility moved correctly, but the adjacent entropy, motion, canonical reconstruction, and reference-publication gates remained whole-frame. Fully revert the standalone implementation and reintroduce it only in SCALE-011. |
| SCALE-007 | Investigated; deprioritized standalone | Partition reference sample ownership to eliminate the shared frame-progress `RwLock`. | The 594 ms sample contains 55 worker-ms in `RwLock::lock_contended`, an optimistic 9.3% wall ceiling, versus approximately 2,543 worker-ms idle. Of 56 contended call sites, 53 are reference readers and 3 are filter writers. Remove the convoy as part of immutable band ownership, but do not expect it to close the architecture gap alone. |
| SCALE-008 | Rejected standalone | Shorten or remove the serial pixel commit/intra reconstruction spine. | The approximately 313 ms to 25 ms `commit_inter` drop is a hard path switch: the 1/2T fused path replays 7,676 inter commands, while the 4/10T prepass leaves only 2,100. A byte-identical by-value tile-superblock-row owner correctly removed the scheduled shadow surfaces, publish copies, completion fanout, and commit mutex, but it also serialized the old motion derivation and replay within each frame. In 11 alternating paired samples, 1T was neutral (0.979014 s baseline, 0.979486 s candidate), while 8T regressed from 0.265387 s to 0.480559 s (paired median +80.98%) and 10T from 0.267624 s to 0.487116 s (+82.35%). Canonical row ownership must return only as part of a motion-band producer/consumer and filter/reference-row publication design that replaces the lost useful intra-frame fanout; do not repeat it standalone. |
| SCALE-009 | Investigated; deprioritized standalone | Remove process-global recycler contention from hot paths. | Individual recycler locks are too small for standalone PRs. Coefficient recycling has an optimistic 4.5% wall ceiling; filter source and residual pools are below 1%, while the inter scratch pool has no contended stack. Make worker-local scratch and frame-context-owned row/coefficient/filter arenas an acceptance requirement of SCALE-003/004; fold motion-field partitioning into SCALE-005. |
| SCALE-010 | Open | Reduce scalar CPU work after the scaling barriers move. | The 0.076443 s target is below ideal 10-way scaling of the current 0.996527 s 1T result. Reprofile the new architecture and require a combined scalar reduction of roughly 23.3% or more without SIMD or assembly. |
| SCALE-011 | Completed | Build one cohesive dav2d-style row DAG across entropy, motion, canonical reconstruction, filters, and reference publication. | Merged in `#1140`. The integrated F1+B3+D/E candidate publishes source motion bands, projects and resolves dependent bands, reconstructs bounded ReferenceOnly work directly into canonical row-band ownership, advances deblock/filter/reference watermarks, and admits dependent rows across frame contexts. The exact 30-frame raw output is 186,624,000 bytes with SHA-256 `48f0dc140be565069838bcf7141aba3c80cefaa14400284840b2e1475a3be945` at 1T and 10T. The final post-CI alternating matrix measured 0.996527 s versus exact base 0.996898 s at 1T (+0.04%), 0.241250 s versus 0.272401 s at 8T (+12.91%), and 0.242316 s versus 0.275443 s at 10T (+13.67%). Serial settled motion fields retain monolithic storage instead of paying for unused band copies. Parse-time unresolved-leaf containment, `rust-simplify`, the 239-fixture decoder oracle, 245-vector conformance corpus, duplication gate, and `cargo xtask ci` pass. Dav2d remains faster at 10T (0.152885 s), so the final 50%-faster target remains open. A GeneralIntra band-target extension was byte-exact but regressed parallel performance and was fully reverted. |
| SCALE-011-K10.7 | Rejected; no PR | Persistent row cursors, horizontal leaves, and row-scoped reference leases. | Segmented storage, permanent halos, reference copies, and nested joins regressed 8T/10T by about 51%. Do not carry that ownership model forward. |
| SCALE-011-K10.8 | Rejected; no PR | Add a vertical readiness graph over the packed/contiguous path. | A whole-frame workspace claim collapsed effective 10T occupancy from 5.67 to 2.41 cores and regressed wall time by 123%. Do not retry whole-frame claims or lock-order/context-depth patches. |
| SCALE-011-K10.9 | Rejected; no PR | Use hybrid row bands for the remaining current-dependent frames. | Segmented deblock was 2.21x slower, generalized CfL was 11.71x slower, and 8T/10T regressed about 30%. Do not retry segmented whole-frame kernels. |
| SCALE-011-K10.10 | Rejected; no PR | Run leaf-DAG row continuations over contiguous band interiors. | The byte-exact/full-CI candidate added 6.94% instructions through 135-way joins, patches, shadows, and band initialization; 8T/10T regressed about 6%. Do not retry leaf/SB granularity while observable progress remains row-wide. |
| SCALE-011-K10.11 | Rejected; no PR | Use a scoped zero-copy horizontal row wavefront. | Direct writes saved compute, but 225 nested scopes fenced four chunks at a time; 10T occupancy fell from 5.69 to 4.97 cores and wall time regressed 12.7%. Reconstruction ownership variants are evidence-closed under safe Rust plus one contiguous mutable workspace. |
| SCALE-011-K10.12 | Open | Build one fused owned `PostFilterGraph` that reduces filter work and removes progressive-publication lock inversion. | Exact main copies or materializes about 33.9 MB per 1080p frame across deblocked windows, CDEF, LR, padded GDF source, and final publication. LR+GDF intermediates account for about 11.3 MB/frame, while completed stripes queue behind a whole-frame output `RwLock` held by next-frame readers. Implement the typed arena, globally scheduled x-slabs, and immutable stripe slots as one multiworker-only candidate from fresh main; preserve literal 1T and require at least 10% at both 8T and 10T before PR. |
| SCALE-012 | Open correctness task | Make deferred finish reporting happen-before in-flight harvest. | `PendingFinish::run_finish` in `crates/splot-decode/src/pipeline/inflight.rs` currently lets `FrameSlotWriter::complete` (or its failure Drop) settle the slot before writing `FinishOutcome.records` or `FinishOutcome.error`; `InflightRing::harvest_oldest` waits only for that slot and can therefore wake, observe an empty outcome, and lose recyclable records or the real filter diagnostic. Add a separate one-shot finish-report completion owned by `PendingFinish` and `InflightEntry`; publish it only after the outcome write, and make harvest wait for the report before consuming the outcome and slot. Cover success, failure, and exactly-once settlement in `crates/splot-decode/src/pipeline/inflight_tests.rs`. Keep this separate from SCALE-011 filter-seam work. |

## Rejected experiments

- Reconstruction lane width 4 and width 10.
- Split entropy/reconstruction at 2 workers without frame-context storage
  ownership; it improved 2T by 17% but exceeded the serial reference-store
  peak.
- Scheduler-owned whole-frame entropy with three pending contexts; exact, but
  only 2.8% faster at 10T and neutral at 1T.
- Owned four-unit commit baton with lazy global reconstruction windows of 9,
  18, and 27 batches; exact, but slower or neutral because it did not advance
  reference/filter publication.
- LIFO in place of FIFO task scopes.
- Batch size 1.
- Commit fusion with blocking and try-lock ownership.
- Commit-gated bounded precompute window.
- Removing the temporal scratch chain.
- Exact whole-field motion dependency selection.
- Motion-band publication plus ordered band resolve without filter/reference-row
  publication; exact, but only 1.9% faster at 10T and 2.9% at 8T.
- Canonical reconstruction bands plus scheduled deblock/filter stripes without
  the adjacent entropy, motion, and reference-row stages; exact, but only 1.8%
  faster at 10T and neutral/slower at 1T and 8T.
- Removing motion-field folding/locking as a ceiling probe.
- The earlier cross-stage deblock slab scheduler.

These results reject the specific implementations, not the architectural tasks.
Do not repeat them without a materially different ownership model and a stated
reason the prior failure no longer applies.
