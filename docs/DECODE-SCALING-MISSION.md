# Decode Scaling Mission

Feature IDs: `INFRA-DECODE-PARALLEL-STAGES`, `INFRA-DECODE-FRAME-PIPELINING`

## Goal

For the scaling campaign, decode the reference stream faster than dav2d at
1, 2, 4, 8, and 10 threads, with the 10-thread wall time at most half of dav2d's
and no 1-thread regression. Retain consistent, byte-exact wins whose complexity
is proportionate; remeasure survivors independently and together after rebases.

Remaining work is in [the open checklist](DECODE-SCALING-OPEN-TASKS.md).

## Measurement

Build both revisions with matching release settings. Use an idle machine,
warmups, and alternating pairs. Compare decoded output separately from timing;
use baseline-versus-baseline controls for changes near the noise floor.

For example, compare decoder-only throughput on the same 30-frame prefix:

```sh
splot decode --quiet --output-format null --limit=30 --threads=10 --frame-delay=auto sample.ivf
dav2d --demuxer ivf -i sample.ivf -o /dev/null --threads 10 -l 30 --framedelay 10
```

Record the revisions, input hash, build settings, thread counts, frame limits,
raw samples, and keep/reject decision with the change. Re-profile the current
revision before selecting another optimization. A retained change must pass
output comparison, the decoder corpus/oracle checks, and `cargo xtask ci`.

## Historical evidence

The [experiment ledger at `699787d04`](https://github.com/splotlabs/splot/blob/699787d045db801fa3e7aee96bb3142c9e535f57/docs/DECODE-SCALING-MISSION.md)
contains the complete SCALE task history, measurements, rejected experiments,
and original benchmark contracts. Those results describe their recorded
revisions and workloads. Consult the recorded reasons before retrying a rejected
approach; current code and measurements determine whether they still apply.
