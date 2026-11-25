"""Compare the frozen, current and release executables on the same native host."""
import csv
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import time

root = Path(os.environ["RUNNER_TEMP"])
corpus = root / "ewouth"
revisions = {"frozen": "b337fafe", "parent": "9df30e91", "candidate": "c9a9a960"}
for name, rev in revisions.items():
    path = root / name
    assert (path / "revision.txt").read_text().startswith(rev)
    subprocess.run(["sha256sum", "-c", "SHA256SUMS"], cwd=path, check=True)
    (path / "splot").chmod(0o755)
cores = sorted(os.sched_getaffinity(0))
print(f"Available CPUs {cores}", flush=True)
with (root / "xeon-release.csv").open("w", newline="") as file:
    out = csv.writer(file)
    out.writerow(("stream","threads","frames","round","revision","wall_seconds"))
    for line in (corpus / "simd-cases.tsv").read_text().splitlines():
        filename, oracle, frames, size = line.split("\t")
        for name in revisions:
            command = [root/name/"splot","decode",corpus/filename,"--threads","1","--frame-delay","1","--output-format","raw","-o","-"]
            with subprocess.Popen(command, stdout=subprocess.PIPE) as proc:
                raw = hashlib.file_digest(proc.stdout, "sha256").hexdigest()
                assert proc.wait() == 0 and raw == oracle
            print(f"{filename} {name} frames={frames} sha256={raw}", flush=True)
        for workers in (1, 7, 12):
            n = int(frames)
            # Full grain sequence for affected-work timing. Bound the other rotation windows.
            limit = n if 'filmgrain' in filename else min(64, n)
            names = list(revisions)
            for turn in range(13):
                order = names[turn%3:] + names[:turn%3]
                if turn%2: order.reverse()
                for name in order:
                    command = [root/name/"splot","decode",corpus/filename,"--threads",str(workers),"--frame-delay",str(min(workers,4)),"--output-format","null","--limit",str(limit)]
                    if workers == 1:
                        command = ["taskset","-c",str(cores[0]),*command]
                    start = time.perf_counter()
                    subprocess.run(command,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,check=True)
                    duration = time.perf_counter() - start
                    if turn:
                        out.writerow((filename,workers,limit,turn,name,f"{duration:.9f}"))
                file.flush()
            print(f"{filename} threads={workers} frames={limit} 12 rounds", flush=True)
