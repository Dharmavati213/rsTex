#!/usr/bin/env -S python3 -I
r"""Does a warm run give what a cold one gives?  On real documents, where run_snapshot.py has small ones and changes the world.

    python3 -I tests/corpus/snapshot/warm_equals_cold.py --engine ENGINE [--filter REGEX] [-j N] [--run-id ID] [--fls] [--path-on] [--keep-pdf] [--check-mutants] [--synctex]

ENGINE is what tests/gate.sh leaves as target/gate/ID/rstex: it finds its own formats.  The default filter is the ladder, the user's documents and SGA3.

Every time tests/corpus/run.py compare calls the engine, a wrapper runs it up to five times from the same state of the output directory:

  1. cold    --no-snapshot
  2. write   no checkpoint in the cache: the run that writes one
  3. warm    the checkpoint is there.  The trace has to say it was restored, or the document counts as not warm, and the reason is kept
  4. and 5.  if it was: once the file is old enough for a stat to show any change, a restore that does the image's sum and leaves its seal, and a sealed one, that does no sum

The outputs of 2, 3 and 5 must equal those of 1 byte for byte: the PDF, the log, every side file, the standard output and the exit status.  Then compare judges
what the last of them left against the oracle, by every level the gate uses, up to the banner.  So a pass here says: a warm run is the same as a cold one, and both
equal the oracle.  Nothing is judged from a document that took no checkpoint except that it stays equal; those are counted, and listed by id, with the reason
for those that are not private.  Everything is written under target/.

tests/corpus/run.py asks for the literal engine, so all that is with the native diagram path off.  --path-on takes it back on, as a user has it: the five runs must still be equal, and say the same of
every diagram, but nothing is judged against the oracle: tests/corpus/cd/compare_native.py documents does that.  It fails if no diagram at all took the path, which would prove nothing.

--keep-pdf judges the output cache of DESIGN section 9 as well, which a run can only use with the last PDF in its place.  So 3 and 5 start from the PDF that 2 and 4 left, with only the side files put
back, as between two runs of a document.  They must still equal 1, and every stream of 3 and 5 must come from the old file, by the trace, or the document fails: with nothing changed, a stream
it could not find is a stream the index lost.  It fails too if no stream at all was found, which would prove nothing.

--check-mutants runs tests/corpus/snapshot/mutants.py's wrong caches, each on a few documents under --keep-pdf, and fails if one of them passes or fails for another reason than its own.

--synctex adds -synctex=1 to every run.  The file is among the outputs, so its bytes must be the same in all of them, members and all, and what goes with it is judged as above.  It fails if no run wrote one.
tests/synctex/documents.py judges its text against the oracle.
"""
import argparse
import json
import os
import re
import stat
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
ROOT = os.path.join(REPO, 'target', 'corpus', 'snapshot')
# What --check-mutants runs them on, and the line of this tool that must show each.
MUTANT_DOCS = r'^ladder/(04-latex-hello|07-amsmath-suite)$'
MUTANTS = [
    ('oc-swap', r'warm run differs from cold: (\d+)'),
    ('oc-stream-of-font', r'warm run differs from cold: (\d+)'),
    ('oc-font-lengths', r'warm run differs from cold: (\d+)'),
    ('oc-lost', r'runs that found less than all: (\d+)'),
    ('oc-sums', r'runs that found less than all: (\d+)'),
    ('oc-flip', r'runs that found less than all: (\d+)'),
]

WRAPPER = r'''#!/usr/bin/env -S python3 -I
import json, os, shutil, subprocess, sys, time
ENGINE = @ENGINE@
REPORT = @REPORT@
PATH_ON = @PATH_ON@
KEEP_PDF = @KEEP_PDF@
SYNCTEX = @SYNCTEX@

def outdir(args):
    for a in args:
        if a.startswith('-output-directory='):
            return a.split('=', 1)[1]
    return '.'

def state(d):
    res = {}
    for root, _, names in os.walk(d):
        for n in names:
            p = os.path.join(root, n)
            with open(p, 'rb') as f:
                res[os.path.relpath(p, d)] = f.read()
    return res

def put_back(d, s):
    for root, _, names in os.walk(d):
        for n in names:
            os.unlink(os.path.join(root, n))
    for name, data in s.items():
        p = os.path.join(d, name)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, 'wb') as f:
            f.write(data)

def run(extra, args, env):
    return subprocess.run([ENGINE] + extra + args, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

def said(p):
    out = []
    for line in p.stderr.split(b'\n'):
        if line.startswith(b'rstex: checkpoint: '):
            out.append(line[19:].decode('latin-1'))
    return out

def strip(stderr):
    return b'\n'.join(l for l in stderr.split(b'\n') if not l.startswith((b'rstex: checkpoint: ', b'rstex: outcache: ')))

def cache_said(p):
    """What the trace says a run took from the old PDF: all zeros for one that kept no index, as a run with no PDF to write does not."""
    for line in p.stderr.split(b'\n'):
        if line.startswith(b'rstex: outcache: '):
            w = line.decode('latin-1').split()[2:]
            return {w[i]: int(w[i + 1]) for i in range(0, len(w), 2)}
    return {}

def again(d, before, last):
    """The side files as they were, and what the last run left of the PDF if the cache is being judged."""
    s = before
    if KEEP_PDF:
        s = dict(before)
        s.update((n, v) for n, v in last.items() if n.endswith('.pdf') and '/' not in n)
    put_back(d, s)

def native_count(files):
    return sum(data.count(b'[[commute native]]') for name, data in files.items() if name.endswith('.log'))

def differs(a, b, ra, rb):
    if ra.returncode != rb.returncode:
        return f'exit status {ra.returncode} and {rb.returncode}'
    if ra.stdout != rb.stdout:
        return 'standard output'
    if strip(ra.stderr) != strip(rb.stderr):
        return 'standard error'
    for name in sorted(set(a) | set(b)):
        if a.get(name) != b.get(name):
            return 'file ' + ('absent' if name not in a or name not in b else name.rsplit('.', 1)[-1])
    return None

args = sys.argv[1:]
d = outdir(args)
before = state(d)
env = dict(os.environ)
env['RSTEX_CHECKPOINT_TRACE'] = '1'
env['RSTEX_OUTCACHE_TRACE'] = '1'
if PATH_ON:
    env.pop('RSTEX_NO_COMMUTE', None)
    args = ['--commute-report'] + args
if SYNCTEX:
    args = ['-synctex=1'] + args
cache = env.get('XDG_CACHE_HOME')
rec = {'args_tail': args[-1][-60:], 'out': d}
cold = run(['--no-snapshot'], args, env)
a = state(d)
put_back(d, before)
if cache:
    shutil.rmtree(os.path.join(cache, 'rstex', 'checkpoints'), ignore_errors=True)
    if KEEP_PDF:
        shutil.rmtree(os.path.join(cache, 'rstex', 'outcache'), ignore_errors=True)
first = run([], args, env)
b = state(d)
rec['first'] = said(first)
rec['write_differs'] = differs(a, b, cold, first)
again(d, before, b)
warm = run([], args, env)
c = state(d)
rec['second'] = said(warm)
rec['cache'] = cache_said(warm)
rec['restored'] = 'restored' in rec['second']
rec['warm_differs'] = differs(a, c, cold, warm)
rec['synctex_file'] = any(name.endswith('.synctex.gz') for name in a)
rec['native_cold'] = native_count(a)
rec['native_warm'] = native_count(c)
rec['sealed'] = False
rec['sealed_differs'] = None
if rec['restored']:
    # RACY_NS of checkpoint.rs, and a little.
    time.sleep(2.2)
    again(d, before, c)
    run([], args, env)
    last = state(d)
    again(d, before, last)
    warm = run([], args, env)
    rec['sealed'] = 'restored' in said(warm) and not any('passed its sum' in s for s in said(warm))
    rec['sealed_differs'] = differs(a, state(d), cold, warm)
    rec['cache_sealed'] = cache_said(warm)
with open(REPORT, 'a') as f:
    f.write(json.dumps(rec) + '\n')
sys.stdout.buffer.write(warm.stdout)
sys.stdout.buffer.flush()
sys.stderr.buffer.write(strip(warm.stderr))
sys.stderr.buffer.flush()
sys.exit(warm.returncode)
'''


def check_mutants(args):
    """Each wrong cache of mutants.py must fail, and by the line it should."""
    bad = 0
    d = os.path.join(ROOT, args.run_id + '-mutants')
    os.makedirs(d, exist_ok=True)
    for kind, why in MUTANTS:
        wrapper = os.path.join(d, kind)
        with open(wrapper, 'w') as f:
            f.write(f'#!/bin/bash\nexec python3 -I {json.dumps(os.path.join(HERE, "mutants.py"))} {kind} --real={json.dumps(os.path.abspath(args.engine))} "$@"\n')
        os.chmod(wrapper, 0o755)
        r = subprocess.run(['python3', '-I', os.path.abspath(__file__), '--engine', wrapper, '--keep-pdf', '-j', str(args.j), '--run-id', f'{args.run_id}-{kind}', '--filter', MUTANT_DOCS],
                           cwd=REPO, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors='replace')
        said = re.search(why, r.stdout)
        ok = r.returncode != 0 and said and int(said.group(1)) > 0
        print(f'{"ok  " if ok else "FAIL"} {kind}: exit {r.returncode}, {said.group(0) if said else "no such line"}', flush=True)
        if not ok:
            print(r.stdout[-1500:])
        bad += not ok
    print(f'{len(MUTANTS)} wrong caches, {bad} not caught')
    return 1 if bad else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--filter', default=r'^(ladder/|user/|local/sga3/SGA3-English$)')
    ap.add_argument('-j', type=int, default=4)
    ap.add_argument('--run-id', default=f'warm-{os.getpid()}')
    ap.add_argument('--fls', action='store_true', help='also -recorder')
    ap.add_argument('--path-on', action='store_true', help='with the native diagram path, as a user has it, and nothing against the oracle')
    ap.add_argument('--keep-pdf', action='store_true', help='the last PDF stays in its place, and every stream of a warm run must come from it')
    ap.add_argument('--check-mutants', action='store_true', help='engines with a wrong output cache must fail --keep-pdf')
    ap.add_argument('--synctex', action='store_true', help='every run with -synctex=1')
    args = ap.parse_args()
    os.makedirs(ROOT, exist_ok=True)
    if args.check_mutants:
        return check_mutants(args)
    report = os.path.join(ROOT, args.run_id + '.jsonl')
    if os.path.exists(report):
        os.unlink(report)
    wrapper = os.path.join(ROOT, args.run_id + '-engine')
    with open(wrapper, 'w') as f:
        f.write(WRAPPER.replace('@ENGINE@', repr(os.path.abspath(args.engine))).replace('@REPORT@', repr(report)).replace('@PATH_ON@', repr(args.path_on)).replace('@KEEP_PDF@', repr(args.keep_pdf)).replace('@SYNCTEX@', repr(args.synctex)))
    os.chmod(wrapper, os.stat(wrapper).st_mode | stat.S_IXUSR)
    cmd = ['python3', '-I', os.path.join(REPO, 'tests', 'corpus', 'run.py'), 'compare', '--engine', wrapper, '--raster', 'none', '--mode', 'last-pass', '-j', str(args.j),
           '--run-id', args.run_id, '--box-log', 'never', '--timeout-scale', '15', '--filter', args.filter]
    if args.fls:
        cmd.append('--fls')
    out = subprocess.run(cmd, cwd=REPO, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors='replace')
    with open(os.path.join(ROOT, args.run_id + '.compare.out'), 'w') as f:
        f.write(out.stdout)
    verdict = subprocess.run(['python3', '-I', os.path.join(REPO, 'tests', 'gate_verdict.py'), os.path.join(REPO, 'target', 'corpus', 'runs', args.run_id, 'results.jsonl')],
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors='replace')
    recs = []
    if os.path.exists(report):
        with open(report) as f:
            recs = [json.loads(line) for line in f]
    restored = [r for r in recs if r['restored']]
    wd = [r for r in recs if r['write_differs']]
    cd = [r for r in recs if r['warm_differs']]
    sd = [r for r in recs if r['sealed_differs']]
    sealed = [r for r in recs if r['sealed']]
    print(f'{len(recs)} engine calls: {len(restored)} restored in the second run, {len(recs) - len(restored)} did not; {len(sealed)} of those restored did a sealed run')
    print(f'write run differs from cold: {len(wd)}; warm run differs from cold: {len(cd)}; sealed run differs from cold: {len(sd)}')
    for r in (wd + cd + sd)[:40]:
        print(f'  {r["out"][-40:]}: write: {r["write_differs"]}; warm: {r["warm_differs"]}; sealed: {r["sealed_differs"]}')
    reasons = {}
    for r in recs:
        if not r['restored']:
            key = '; '.join(s.split(':')[0][:60] for s in r['first'] + r['second'])
            reasons[key] = reasons.get(key, 0) + 1
    for k, n in sorted(reasons.items(), key=lambda x: -x[1]):
        print(f'  not restored {n}: {k}')
    # A document with no checkpoint proves nothing about a restore, and all of them having none would mean that the trigger has stopped working.  The same for a seal.
    ok = not wd and not cd and not sd and len(sealed) == len(restored) and restored
    if args.path_on:
        cold_on = sum(r['native_cold'] for r in recs)
        warm_on = sum(r['native_warm'] for r in recs if r['restored'])
        print(f'with the path on: {cold_on} diagrams took it in the cold runs, {warm_on} in the runs restored')
        ok = ok and warm_on > 0
    else:
        print('compare, what the last run left against the oracle:')
        print(verdict.stdout[-1500:])
        ok = ok and verdict.returncode == 0
    if args.keep_pdf:
        kept = [(r, c) for r in recs for c in (r['cache'], r.get('cache_sealed')) if c]
        total = {k: sum(c.get(k, 0) for _, c in kept) for k in ('streams', 'hits', 'fonts', 'font_hits', 'cmaps', 'cmap_hits', 'sum_failures')}
        short = [r for r, c in kept if c['hits'] + c.get(0) != c['streams']]
        print(f'output cache, {len(kept)} runs that should find everything: {total["hits"]} of {total["streams"]} streams, {total["font_hits"]} of {total["fonts"]} fonts, {total["cmap_hits"]} of {total["cmaps"]} CMaps, {total["sum_failures"]} sums failed')
        print(f'runs that found less than all: {len(short)}')
        for r in short[:40]:
            print(f'  {r["out"][-40:]}: {r["cache"]}')
        ok = ok and not short and total['hits'] > 0
    if args.synctex:
        files = sum(r['synctex_file'] for r in recs)
        print(f'with -synctex=1: {files} of {len(recs)} calls wrote a file')
        ok = ok and files > 0
    print(f'{"PASS" if ok else "FAIL"} warm equals cold: {len(recs)} calls, {len(restored)} restored, {len(sealed)} sealed; report {report}')
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
