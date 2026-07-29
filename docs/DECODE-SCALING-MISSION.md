# Decode Scaling Mission

Feature IDs: `INFRA-DECODE-PARALLEL-STAGES`,
`INFRA-DECODE-FRAME-PIPELINING`

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

The 10-thread target is approximately 0.079 seconds. Perfect scaling of the
current 1-thread time would still be approximately 0.102 seconds, so the final
result requires both a scheduler/ownership redesign and a substantial reduction
in scalar CPU work.

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
the task table before work moves on.

## Task ledger

| ID | Status | Finding / task | Evidence and next action |
|---|---|---|---|
| SCALE-001 | Confirmed | dav2d uses a global queue over frame contexts and three row-progress passes: entropy, MV resolution, and reconstruction/filter progress. | With the canonical command, dav2d creates 2/4/8/10 persistent worker threads and the same number of frame contexts; the caller remains a separate admission thread. One tile task advances one superblock row, publishes, and continues or requeues. Use this as the reference model. |
| SCALE-002 | Rejected | Enable split entropy/reconstruction at two workers. | Lowering the four-worker bound to two improved 2T by 17.0% in a 9-pair diagnostic run and was byte-identical at 1/2/4/8/10T, but the full gate proved that the split path raises peak live reference storage from 110,592 to 122,880 bytes at both 2T and 3T. The source change was reverted and no code PR was opened. Revisit only after frame contexts preserve the serial reference-store peak. |
| SCALE-003 | Rejected standalone | Queue entropy work from multiple frame contexts instead of synchronously parsing one whole frame on the driver. | A three-context POC derived pending-slot geometry from the header, provisionally refreshed references with terminal CDF/CCSO handles, and scheduled entropy under exact primary/blend/CCSO dependencies. It was output-identical in focused 1/4/10T checks and frame-delay 1/2/3/10 tests, but an 11-pair 10T screen moved the median only from 294.8 ms to 286.5 ms, 2.8%; ten entropy contexts regressed. SCALE-004 did not unlock more value, so revert rather than merge this large foundation. Reintroduce pending entropy products only as part of a row-progress design that materially shortens the dependency chain. |
| SCALE-004 | Rejected | Replace the all-at-once reconstruction task dump with a bounded dav2d-style wavefront that advances and republishes one unit at a time. | An exact POC removed mutex-held commit replay with an owned baton, admitted four-unit precomputes lazily under a global pool-width budget, charged permits only after row conditions became ready, and prioritized entropy/commit before speculation. Baton ownership alone was approximately 0.8% slower. A nine-batch global window regressed approximately 2.5%; 18- and 27-batch windows only converged to neutral/slightly worse. The existing precompute/shadow-surface plus serial replay split is the wrong unit: bounding it removes useful work without advancing reference/filter publication. Revert and pursue SCALE-008's direct resumable row owner instead of tuning this scheduler. |
| SCALE-005 | Investigated; design open | Remove the whole-reference motion-field barrier with row/band publication. | Scheduled frame preparation waits for every named reference motion field to complete. The projection implementation is already band-separable with no neighboring vertical-band dependency: use immutable full-width 64-pixel source bands, combine two when a 128-pixel projection unit requires them, and keep resolve as an ordered continuation. Selecting only exact whole-field source slots regressed because it exposed contention without publishing any usable row earlier. |
| SCALE-006 | Investigated; design open | Start filter progress before whole-frame reconstruction commit completes. | Current filters start only after the final reconstruction commit. Deleting all filters has an approximately 28% 10T ceiling, so this cannot meet the target alone. The prior slab callback failed because ownership stayed whole-frame. The successor needs canonical full-width reconstruction bands, explicit last-use frontiers for GeneralIntra/local IntraBC/BAWP/interintra, and recon/deblock/filter watermarks without nested Rayon scopes or copy-out/copy-back ownership. |
| SCALE-007 | Investigated; deprioritized standalone | Partition reference sample ownership to eliminate the shared frame-progress `RwLock`. | The 594 ms sample contains 55 worker-ms in `RwLock::lock_contended`, an optimistic 9.3% wall ceiling, versus approximately 2,543 worker-ms idle. Of 56 contended call sites, 53 are reference readers and 3 are filter writers. Remove the convoy as part of immutable band ownership, but do not expect it to close the architecture gap alone. |
| SCALE-008 | Rejected standalone | Shorten or remove the serial pixel commit/intra reconstruction spine. | The approximately 313 ms to 25 ms `commit_inter` drop is a hard path switch: the 1/2T fused path replays 7,676 inter commands, while the 4/10T prepass leaves only 2,100. A byte-identical by-value tile-superblock-row owner correctly removed the scheduled shadow surfaces, publish copies, completion fanout, and commit mutex, but it also serialized the old motion derivation and replay within each frame. In 11 alternating paired samples, 1T was neutral (0.979014 s baseline, 0.979486 s candidate), while 8T regressed from 0.265387 s to 0.480559 s (paired median +80.98%) and 10T from 0.267624 s to 0.487116 s (+82.35%). Canonical row ownership must return only as part of a motion-band producer/consumer and filter/reference-row publication design that replaces the lost useful intra-frame fanout; do not repeat it standalone. |
| SCALE-009 | Investigated; deprioritized standalone | Remove process-global recycler contention from hot paths. | Individual recycler locks are too small for standalone PRs. Coefficient recycling has an optimistic 4.5% wall ceiling; filter source and residual pools are below 1%, while the inter scratch pool has no contended stack. Make worker-local scratch and frame-context-owned row/coefficient/filter arenas an acceptance requirement of SCALE-003/004; fold motion-field partitioning into SCALE-005. |
| SCALE-010 | Open | Reduce scalar CPU work after the scaling barriers move. | The 0.079 s target is below the ideal 10-way scaling of the current 1T result. Reprofile the new architecture and require a combined scalar reduction of roughly 23% or more without SIMD or assembly. |

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
- Removing motion-field folding/locking as a ceiling probe.
- The earlier cross-stage deblock slab scheduler.

These results reject the specific implementations, not the architectural tasks.
Do not repeat them without a materially different ownership model and a stated
reason the prior failure no longer applies.
