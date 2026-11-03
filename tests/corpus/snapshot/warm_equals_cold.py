#!/usr/bin/env -S python3 -I
r"""Does a warm run give what a cold one gives?  On real documents, where run_snapshot.py has small ones and changes the world.

    python3 -I tests/corpus/snapshot/warm_equals_cold.py --engine ENGINE [--filter REGEX] [-j N] [--run-id ID] [--fls]

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

WRAPPER = r'''#!/usr/bin/env -S python3 -I
import json, os, shutil, subprocess, sys, time
ENGINE = @ENGINE@
REPORT = @REPORT@

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
    return b'\n'.join(l for l in stderr.split(b'\n') if not l.startswith(b'rstex: checkpoint: '))

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
cache = env.get('XDG_CACHE_HOME')
rec = {'args_tail': args[-1][-60:], 'out': d}
cold = run(['--no-snapshot'], args, env)
a = state(d)
put_back(d, before)
if cache:
    shutil.rmtree(os.path.join(cache, 'rstex', 'checkpoints'), ignore_errors=True)
first = run([], args, env)
b = state(d)
rec['first'] = said(first)
rec['write_differs'] = differs(a, b, cold, first)
put_back(d, before)
warm = run([], args, env)
c = state(d)
rec['second'] = said(warm)
rec['restored'] = 'restored' in rec['second']
rec['warm_differs'] = differs(a, c, cold, warm)
rec['sealed'] = False
rec['sealed_differs'] = None
if rec['restored']:
    # RACY_NS of checkpoint.rs, and a little.
    time.sleep(2.2)
    put_back(d, before)
    run([], args, env)
    put_back(d, before)
    warm = run([], args, env)
    rec['sealed'] = 'restored' in said(warm) and not any('passed its sum' in s for s in said(warm))
    rec['sealed_differs'] = differs(a, state(d), cold, warm)
with open(REPORT, 'a') as f:
    f.write(json.dumps(rec) + '\n')
sys.stdout.buffer.write(warm.stdout)
sys.stdout.buffer.flush()
sys.stderr.buffer.write(strip(warm.stderr))
sys.stderr.buffer.flush()
sys.exit(warm.returncode)
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--filter', default=r'^(ladder/|user/|local/sga3/SGA3-English$)')
    ap.add_argument('-j', type=int, default=4)
    ap.add_argument('--run-id', default=f'warm-{os.getpid()}')
    ap.add_argument('--fls', action='store_true', help='also -recorder')
    args = ap.parse_args()
    os.makedirs(ROOT, exist_ok=True)
    report = os.path.join(ROOT, args.run_id + '.jsonl')
    if os.path.exists(report):
        os.unlink(report)
    wrapper = os.path.join(ROOT, args.run_id + '-engine')
    with open(wrapper, 'w') as f:
        f.write(WRAPPER.replace('@ENGINE@', repr(os.path.abspath(args.engine))).replace('@REPORT@', repr(report)))
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
    print('compare, what the last run left against the oracle:')
    print(verdict.stdout[-1500:])
    # A document with no checkpoint proves nothing about a restore, and all of them having none would mean that the trigger has stopped working.  The same for a seal.
    ok = verdict.returncode == 0 and not wd and not cd and not sd and len(sealed) == len(restored) and restored
    print(f'{"PASS" if ok else "FAIL"} warm equals cold: {len(recs)} calls, {len(restored)} restored, {len(sealed)} sealed; report {report}')
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
