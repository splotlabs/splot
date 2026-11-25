"""Build a shareable release-only supplement from the qualification workspace."""
import csv
import hashlib
import io
from pathlib import Path
import re
import runpy
import tarfile

here = Path(__file__).resolve().parent.parent / 'qualification-1882' / 'v3-release'
out = Path(__file__).resolve().parent / 'evidence' / 'v3-release'
out.mkdir(exist_ok=True)
pack = runpy.run_path(str(out.parents[1] / 'pack-evidence.py'), run_name='release_sanitizer')
raw = {}
def add(path, name=None):
    if not path.is_file() or path.is_symlink(): return
    name = name or str(path.relative_to(here))
    data = path.read_bytes()
    if path.suffix == '.log':
        # GitHub's CLI prefixes each log line with job and step columns.
        data = re.sub(rb'(?m)^[^\t\r\n]+\t[^\t\r\n]+\t(?=20\d\d-)', b'', data)
    clean = pack['sanitize'](name, data)
    if clean is not None: raw[name] = clean
for glob in ('local-results.md','qualify.py','matched-*.py','native-wide-oracle.log','native-wide-final-oracle.log','native-wide-release-oracle.log','native-pairs.py',
             'api-compat.log','api-fast-gate.log','final-product/receipt.txt','final-product/build.time',
             'consumer-sizes/consumer.rs','consumer-sizes/consumer.c','consumer-sizes/Cargo.toml.consumer',
             '*-timings.csv','*-timings.md','*-perf-summary.txt','ci-perf-original.log','ci-perf-typed.log','ci-perf-release.log',
             'native-wide-*-result.log','wide-unavailable.log'):
    for f in here.glob(glob): add(f)
for sub in ('native','baseline','matched','typed-screen','final-screen','xeon-release','consumer-sizes','lifecycle','codegen-negative','platform-consumers','isa-control','portable-release'):
    for pattern in ('*.txt','*.log','*.csv','*.tsv','*.cg','*.py','*.md','*.c','*.patch','workflow.yml','SHA256SUMS','*.sha256','*.time','*/build.time','*/build.log','*/linked-sizes.txt','*/revision.txt','traces/*.cg','traces/*.log'):
        for f in (here/sub).glob(pattern): add(f)
for pattern in ('profile/*.cg','profile/*.log','profile/*.tsv','profile/*.md','profile/*.py','profile/*.txt','*.rs'):
    for f in (here/'portable-release').glob(pattern): add(f)
for pattern in ('syn-*.json',):
    for f in (here/'lifecycle').glob(pattern): add(f)
for f in (here/'codegen-negative').glob('*.asm'): add(f)
for pattern in ('artifacts/matched-*/*.sections','artifacts/matched-*/*.typed','artifacts/matched-*/toolchain.txt','artifacts/matched-*/SHA256SUMS'):
    for f in (here/'isa-control').glob(pattern): add(f)
for pattern in ('parent/*.txt','candidate/*.txt','parent/*.tsv','candidate/*.tsv','parent/*.csv','candidate/*.csv'):
    for f in (here/'platform-consumers').glob(pattern): add(f)
for sub in ('linked','linked-final'):
    for pattern in ('README.md','inspect.py','compare.py','check-paths.py','workflow.yml','*.tsv','*.SHA256SUMS','*-*/revision.txt',
                    '*-*/toolchain.txt','*-*/SHA256SUMS','*-analysis/*/*.tsv','*-analysis/*/*.asm',
                    '*-analysis/*/*.txt'):
        for f in (here/sub).glob(pattern): add(f)
for f in (here.parent/'partial-avx2/probe-integrated').glob('*'):
    if f.suffix in {'.log','.txt'}: add(f,'partial-floor/'+f.name)
apple = here.parent / 'apple-arm64-2006'
for name in ('avm-hashes.tsv','simd-cases.tsv','summary.md','timing-summary.txt','timings.csv','comparison.log'):
    add(apple / name, 'apple-m1/' + name)
for build in ('base','candidate'):
    for pattern in ('*.txt','*.toml','backend-tests.log'):
        for f in (apple/build).glob(pattern):
            add(f, f'apple-m1/{build}/{f.name}')
for folder in ('apple-arm64-83566cc2', 'neon-only-1993'):
    part = here.parent / folder
    for pattern in ('*.csv','*.tsv','*.md','*summary*.txt','*hash*.txt','*/revision.txt','*/linked-size.txt','*/backend-tests.log','backend-tests.log','linked-size.txt','comparison.log','check-linked-dispatch.py','disassembly.sha256'):
        for f in part.glob(pattern):
            add(f, folder + '/' + str(f.relative_to(part)))
for pattern in ('*.md','*.py','*.cg','*.tsv','*.SHA256SUMS','*job.log','*/sizes.txt','*/row.log','*/playback.log','*/*.csv','*/*.tsv','*/*.asm','*/SHA256SUMS','*/candidate.sha','*/control.sha'):
    for f in (here.parent/'grain-lut').glob(pattern):
        add(f,'grain-lut/'+str(f.relative_to(here.parent/'grain-lut')))
def digest(b): return hashlib.sha256(b).hexdigest()
with (out/'files.tsv').open('w') as file:
    w=csv.writer(file,delimiter='\t',lineterminator='\n')
    w.writerow(('file','sha256','bytes'))
    for name,data in sorted(raw.items()): w.writerow((name,digest(data),len(data)))
with tarfile.open(out/'measurements.tar.xz','w:xz') as archive:
    for name,data in sorted(raw.items()):
        entry=tarfile.TarInfo(name); entry.size=len(data); entry.mtime=0
        archive.addfile(entry,io.BytesIO(data))
with tarfile.open(out/'measurements.tar.xz','r:xz') as archive:
    assert len(archive.getmembers()) == len(raw)
    for e in archive:
        assert digest(archive.extractfile(e).read()) == digest(raw[e.name])
(out/'SHA256SUMS').write_text(''.join(f'{digest((out/f).read_bytes())}  {f}\n' for f in ('measurements.tar.xz','files.tsv')))
print('Verified',len(raw),'files, preserving all original local records.')
