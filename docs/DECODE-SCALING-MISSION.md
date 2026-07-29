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
| SCALE-001 | Confirmed | dav2d uses a global queue over frame contexts and three row-progress passes: entropy, MV resolution, and reconstruction/filter progress. | Use this as the reference model. Splot must expose bounded row progress from multiple frames instead of submitting whole-frame phases. |
| SCALE-002 | Candidate | Enable split entropy/reconstruction at two workers. | Splot explicitly required at least four workers. Lowering the bound to two improved 2T by 17.0% in a 9-pair diagnostic run, with 1/4/8/10T within 0.3%. It does not close the 10T gap. Validate whether it is a safe standalone architectural step or superseded by SCALE-003. |
| SCALE-003 | Open | Queue entropy work from multiple frame contexts instead of synchronously parsing one whole frame on the driver. | The driver cannot start frame N+1 until frame N's full entropy result returns. Determine the exact CDF, header, reference-metadata, output-order, and scratch ownership dependencies, then design scheduler-owned parse contexts. |
| SCALE-004 | Rejected prototype; redesign open | Replace the all-at-once reconstruction task dump with a bounded dav2d-style wavefront that advances and republishes one unit at a time. | The current scheduler submits every precompute and later admits ordered commits behind the backlog. A simple worker-window tied to commits regressed 2.7-5.2%; batch size 1 and commit fusion also regressed. The successor must use persistent per-worker task contexts or continuations, not another layer of joins or a blocking commit mutex. |
| SCALE-005 | Open | Remove the whole-reference motion-field barrier with row/band publication. | Scheduled frame preparation waits for every named reference motion field to complete. Selecting only exact source slots was byte-exact but regressed 0.3-2.7%. Publish immutable motion-field bands and admit only the dependent resolve bands. |
| SCALE-006 | Open | Start filter progress before whole-frame reconstruction commit completes. | Current filters start only after the final reconstruction commit. A prior deblock-slab callback was byte-exact but slower because stages already saturated Rayon and mutable workspace ownership was unresolved. Redesign row ownership so reconstruction can transfer completed bands directly to filter tasks without nested scopes or copies. |
| SCALE-007 | Open | Partition reference sample ownership to eliminate the shared frame-progress `RwLock`. | Reference reads were the largest application lock family in the 10T sample (37 contended stack samples). Measure wall ceiling, then evaluate immutable published bands or per-band guards. |
| SCALE-008 | Open | Shorten or remove the serial pixel commit/intra reconstruction spine. | Skip-filter timing still scales only 0.559 s at 1T to 0.536 s at 2T. At 1/2T, ordered commit performs about 313 ms of inter work versus about 25 ms at 4/10T. Separate true walk-order dependencies from work that can execute in disjoint owned bands. |
| SCALE-009 | Open | Remove process-global recycler contention from hot paths. | Motion folding and coefficient recycling appear in contended stacks; the shared inter scratch pool does not. Attribute each recycler's wall ceiling before considering worker-local storage. |
| SCALE-010 | Open | Reduce scalar CPU work after the scaling barriers move. | The 0.079 s target is below the ideal 10-way scaling of the current 1T result. Reprofile the new architecture and require a combined scalar reduction of roughly 23% or more without SIMD or assembly. |

## Rejected experiments

- Reconstruction lane width 4 and width 10.
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
