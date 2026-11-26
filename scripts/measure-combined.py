import csv
import hashlib
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import time
import tomllib

root, corpus, manifest_path = map(Path, sys.argv[1:])
mac = platform.system() == "Darwin"
streams = tomllib.loads(manifest_path.read_text())["stream"]
cases = [s for s in streams if s["path"].startswith(("BlueSky_360p25_v2_qp110", "DOTA2_", "Slides1_", "Noise_Ocean_", "old_town_cross_1080p50_tiles4x2", "RitualDance_", "TunnelFlag_", "WITCHER3_")) and ("qp135" in s["path"] or "BlueSky" in s["path"])]
assert len(cases) == 9
def run(cmd, **kw):
    return subprocess.run(cmd, check=True, **kw)
def command(rev, stream, threads, fmt):
    return [str(root/rev), "decode", str(corpus/stream["path"]), "--threads", str(threads), "--output-format", fmt]
for s in cases:
    assert hashlib.sha256((corpus/s["path"]).read_bytes()).hexdigest() == s["input_sha256"]
    ref = None
    for threads in (1, 12):
        for rev in ("baseline", "combined"):
            result = run(command(rev,s,threads,"hash"), capture_output=True).stdout
            frames = [line for line in result.splitlines() if line.startswith(b"frame.")]
            assert sum(line.startswith(b"frame.hash:") for line in frames) == s["output_frames"]
            assert ref is None or frames == ref, (s["path"],threads,rev)
            ref = frames
    print("Hash match",s["path"],s["output_frames"],flush=True)

prefix = []
if not mac:
    cpu = min(os.sched_getaffinity(0))
    prefix = ["taskset","-c",str(cpu)]
    print("Pinned CPU",cpu,flush=True)
    import shutil
    perf = shutil.which("perf")
    has_pmu = bool(perf and subprocess.run([perf,"stat","-e","instructions:u","true"],capture_output=True).returncode == 0)
    print("Hardware instruction counter available",has_pmu,flush=True)
def measure(args):
    wrapper = ["/usr/bin/time","-l"] if mac else ([perf,"stat","-x",";","-e","instructions:u","--"] if has_pmu else [])
    start = time.perf_counter_ns()
    result = run(prefix+wrapper+args,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,text=True)
    ns = time.perf_counter_ns()-start
    if mac:
        ins = int(re.search(r"(\d+)\s+instructions retired",result.stderr)[1])
    elif has_pmu:
        ins = int(re.search(r"(\d+);[^;]*;instructions:u",result.stderr)[1])
    else:
        ins = ""
    return ins, ns
with (root/"observations.csv").open("w") as f:
    out=csv.writer(f)
    out.writerow(["batch","stream","pair","order","revision","instructions","nanoseconds"])
    for batch,limit in enumerate([24,60],1):
        for s in (cases if batch==1 else cases[::-1]):
            for pair in range(7):
                for order,rev in enumerate(("baseline","combined") if (batch+pair)%2 else ("combined","baseline"),1):
                    obs=measure(command(rev,s,1,"null")+["--frame-delay","1","--limit",str(limit)])
                    out.writerow([batch,s["path"],pair,order,rev,*obs])
                    f.flush()
            print("Measured",batch,s["path"],flush=True)
