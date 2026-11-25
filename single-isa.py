"""Matched same-ISA native controls. Both executables decode through AVX-512."""
import csv
import hashlib
import os
from pathlib import Path
import subprocess
import time

corpus = Path(os.environ['RUNNER_TEMP'])/'ewouth'
cpus = sorted(os.sched_getaffinity(0))
modes = ('dual','single')
binaries = {}
for mode in modes:
    folder=Path('inputs')/('matched-'+mode)
    subprocess.run(['sha256sum','-c','SHA256SUMS'],cwd=folder,check=True)
    binaries[mode]=folder/'splot'
    binaries[mode].chmod(0o755)
with Path('single-isa.csv').open('w',newline='') as f:
    record=csv.writer(f)
    record.writerow(('stream','workers','frames','round','order','mode','seconds'))
    for line in (corpus/'simd-cases.tsv').read_text().splitlines():
        name,expected,frames,size=line.split('\t')
        for mode in modes:
            with subprocess.Popen([binaries[mode],'decode',corpus/name,'--output-format','raw','-o','-','--threads','1','--frame-delay','1'],stdout=subprocess.PIPE) as proc:
                actual=hashlib.file_digest(proc.stdout,'sha256').hexdigest()
                assert proc.wait()==0 and expected==actual,(mode,name,actual)
            print('HASH',name,mode,frames,actual,flush=True)
        if not (name.startswith(('BlueSky','RitualDance')) or 'filmgrain' in name):
            continue
        window=int(frames) if 'filmgrain' in name else min(int(frames),64)
        for workers in (1,7,12):
            for r in range(25):
                order=modes if r%2 else tuple(reversed(modes))
                for pos,mode in enumerate(order):
                    command=[binaries[mode],'decode',corpus/name,'--threads',str(workers),'--frame-delay',str(min(4,workers)),'--limit',str(window),'--output-format','null']
                    if workers==1:
                        command=['taskset','-c',str(cpus[0]),*command]
                    start=time.perf_counter()
                    subprocess.run(command,check=True,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
                    if r:
                        record.writerow((name,workers,window,r,pos+1,mode,time.perf_counter()-start))
                f.flush()
            print('PAIRS',name,workers,24,flush=True)
