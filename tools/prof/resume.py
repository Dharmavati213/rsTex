#!/usr/bin/env -S python3 -I
"""resume.py: what resuming in the middle of a document is worth, and whether it gives the same bytes.  A prototype's tool: crates/engine/src/checkpoint/pages.rs.

    resume.py --engine PATH --docs hello,main,... [--every N] [--at 0.1,0.5,0.9 | --all] [--verify] [--json FILE]

Each document compiles the way bench.py does, in one directory that keeps its path, with the oracle's last-pass side files put back before every run.  For each:

    cold    RSTEX_NO_SNAPSHOT=1
    write0  an empty cache, a preamble checkpoint only
    write   an empty cache, RSTEX_PAGE_CK=N: the preamble checkpoint and a chain of page records
    warm    RSTEX_RESUME=0: the preamble's checkpoint only
    k       RSTEX_RESUME=k: the k-th record, for each one asked for

Every run's outputs are compared with the cold run's, byte for byte: the PDF, the log and every side file.  Instructions are exact and first, as for everything on this machine.  Nothing of a
document is printed: the reasons a trace gives are shown for documents that are not private.
"""
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import proflib  # noqa: E402


def env_for(doc, cache):
    env = proflib.corpus().entry_env(doc.entry)
    env['XDG_CACHE_HOME'] = cache
    return env


def reset(work, doc):
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    proflib.corpus().restore_state(doc.prepare(), work)


def state(d):
    res = {}
    for root, _, names in os.walk(d):
        for n in names:
            p = os.path.join(root, n)
            with open(p, 'rb') as f:
                res[os.path.relpath(p, d)] = f.read()
    return res


def differs(a, b):
    out = []
    for name in sorted(set(a) | set(b)):
        if a.get(name) != b.get(name):
            ext = name.rsplit('.', 1)[-1]
            if name not in a or name not in b:
                out.append(ext + ':absent')
            else:
                x, y = a[name], b[name]
                n = min(len(x), len(y))
                k = next((i for i in range(n) if x[i] != y[i]), n)
                out.append(f'{ext}:{len(x)}/{len(y)}@{k}')
    return out


class Runner:
    def __init__(self, engine, doc, root):
        self.engine, self.doc = engine, doc
        self.work = os.path.join(root, 'o')
        self.cache = os.path.join(root, 'cache')
        os.makedirs(self.cache, exist_ok=True)
        self.base_env = env_for(doc, self.cache)
        for kv in filter(None, os.environ.get('RESUME_EXTRA_ENV', '').split(',')):
            k, v = kv.split('=', 1)
            self.base_env[k] = v
        self.private = doc.id.startswith(('user/', 'local/', 'cd-local/'))

    def argv(self):
        argv, cwd, _ = self.engine.command(self.doc, self.work)
        return argv, cwd

    def run(self, extra, counted=True, keep_trace=False):
        """One run from the last pass's state.  Returns counters and what it left."""
        reset(self.work, self.doc)
        argv, cwd = self.argv()
        env = dict(self.base_env, **extra)
        rec = {}
        if counted:
            r = proflib.run_counted(argv, cwd, env)
            for k in ('instructions', 'cycles', 'page-faults'):
                if k in r['counters']:
                    rec[k] = r['counters'][k]['value']
            rec.update(cpu_ms=r['cpu_ms'], sys_ms=r['sys_ms'], rc=r['rc'], maxrss_kb=r['maxrss_kb'])
        else:
            r = subprocess.run(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            rec.update(rc=r.returncode, stdout=hashlib.sha256(r.stdout).hexdigest())
            rec['trace'] = [line[len('rstex: checkpoint: '):] for line in r.stderr.decode('latin-1').splitlines() if line.startswith('rstex: checkpoint: ')]
        return rec

    def chain_dir(self):
        d = os.path.join(self.cache, 'rstex', 'checkpoints')
        for n in sorted(os.listdir(d)) if os.path.isdir(d) else []:
            if n.endswith('.pg'):
                return os.path.join(d, n)
        return None

    def sizes(self):
        d = self.chain_dir()
        res = {}
        if d:
            for n in os.listdir(d):
                res[n] = os.path.getsize(os.path.join(d, n))
        return res

    def empty_cache(self):
        shutil.rmtree(os.path.join(self.cache, 'rstex', 'checkpoints'), ignore_errors=True)


def show(label, r, cold):
    i = r.get('instructions') or 0
    c = (cold or {}).get('instructions') or 1
    print(f'  {label:<10}{i / 1e6:>11.2f}{i / c:>8.3f}{(r.get("cycles") or 0) / 1e6:>11.1f}{r.get("page-faults") or 0:>9}{r.get("sys_ms", 0):>9.1f}  {r.get("same", "")}')
    sys.stdout.flush()


def one(engine, doc, args):
    doc.prepare()
    root = os.path.join(proflib.out_root(), 'resume', doc.name.replace('/', '_'))
    shutil.rmtree(root, ignore_errors=True)
    R = Runner(engine, doc, root)
    rec = {'id': doc.id, 'name': doc.name, 'every': args.every}
    print(doc.name)
    print(f'  {"":<10}{"Minstr":>11}{"/cold":>8}{"Mcycles":>11}{"faults":>9}{"sys ms":>9}')
    # The first run fills the tables of kpathsea and the map in the cache directory, which is not what a document costs.
    R.run({'RSTEX_NO_SNAPSHOT': '1'})
    rec['cold'] = R.run({'RSTEX_NO_SNAPSHOT': '1'})
    ref = state(R.work)
    cold_term = R.run({'RSTEX_NO_SNAPSHOT': '1'}, counted=False)
    rec['pdf_bytes'] = sum(len(v) for k, v in ref.items() if k.endswith('.pdf'))
    show('cold', rec['cold'], rec['cold'])
    R.empty_cache()
    rec['write0'] = R.run({})
    rec['write0']['same'] = differs(ref, state(R.work))
    show('write0', rec['write0'], rec['cold'])
    ckd = os.path.join(R.cache, 'rstex', 'checkpoints')
    ck0 = sum(os.path.getsize(os.path.join(ckd, n)) for n in os.listdir(ckd) if n.endswith('.ck')) if os.path.isdir(ckd) else 0
    if ck0 == 0:
        print('  no preamble checkpoint: 0 of 0 records resumed from, 0 differ')
        rec['no_preamble_checkpoint'] = True
        shutil.rmtree(root, ignore_errors=True)
        return rec
    rec['preamble_ck_bytes'] = ck0
    write_env = {'RSTEX_PAGE_CK': str(args.every)}
    if args.verify:
        write_env['RSTEX_PAGE_HASH'] = '1'
        write_env['RSTEX_DEFLATE_WORKERS'] = '0'
    R.empty_cache()
    rec['write'] = R.run(write_env)
    rec['write']['same'] = differs(ref, state(R.work))
    show('write', rec['write'], rec['cold'])
    rec['chain'] = R.sizes()
    print(f'  chain {sum(rec["chain"].values()) / 1e6:.2f} MB, preamble {ck0 / 1e6:.2f} MB; ' + ' '.join(f'{k}={v / 1e6:.2f}' for k, v in sorted(rec['chain'].items())))
    # What a writing run said, and how many records.
    R.empty_cache()
    t = R.run(dict(write_env, RSTEX_CHECKPOINT_TRACE='1'), counted=False)
    said = t['trace']
    rec['write_said'] = [s.split(':')[0] for s in said] if R.private else said
    for line in said:
        if line.startswith('pages:') and (not R.private or 'records' in line or line.startswith('pages: section') or line.startswith('pages: st') or line.startswith('pages: sk')):
            print('  ' + (line if not R.private or 'records' in line or 'section' in line else line.split(':')[1] + ': ...'))
    rec['write_trace_same'] = differs(ref, state(R.work))
    pdf = os.path.join(R.chain_dir() or '', 'f2')
    # The stream of the PDF has to be a prefix of the PDF.
    if os.path.exists(pdf):
        with open(pdf, 'rb') as f:
            p = f.read()
        whole = next((v for k, v in ref.items() if k.endswith('.pdf')), b'')
        rec['pdf_prefix'] = whole.endswith(whole[-1:]) and p == whole[len(whole) - len(whole):len(p)]
        print(f'  the PDF stream is a prefix of the PDF: {rec["pdf_prefix"]} ({len(p)} of {len(whole)} bytes)')
    # Which records there are.
    rec['warm'] = R.run({'RSTEX_RESUME': '0'})
    rec['warm']['same'] = differs(ref, state(R.work))
    show('warm', rec['warm'], rec['cold'])
    t = R.run({'RSTEX_RESUME': 'latest', 'RSTEX_CHECKPOINT_TRACE': '1'}, counted=False)
    n = 0
    for line in t['trace']:
        if line.startswith('pages:') and 'records' in line:
            n = int(line.split()[1])
    rec['records'] = n
    rec['same_latest'] = differs(ref, state(R.work))
    if args.all:
        ks = list(range(1, n + 1))
    else:
        ks = sorted({max(1, min(n, round(float(a) * n))) for a in args.at.split(',')}) if n else []
    rec['resumes'] = {}
    bad = 0
    for k in ks:
        extra = {'RSTEX_RESUME': str(k)}
        if args.all and not args.counted_all:
            r = R.run(dict(extra, RSTEX_CHECKPOINT_TRACE='1'), counted=False)
            r.pop('trace', None)
        elif args.verify:
            r = R.run(dict(extra, RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_VERIFY='1', RSTEX_PAGE_HASH='1', RSTEX_DEFLATE_WORKERS='0'), counted=False)
            r['verify'] = [s for s in r.pop('trace') if s.startswith('verify')]
        else:
            r = R.run(extra)
        r['same'] = differs(ref, state(R.work))
        if 'stdout' in r and (r['stdout'] != cold_term['stdout'] or r['rc'] != cold_term['rc']):
            r['same'].append('terminal or exit status')
        rec['resumes'][k] = r
        bad += bool(r['same'])
        if not args.all or r['same']:
            if args.verify:
                print(f'  k={k}: same={r["same"]} {r.get("verify", [])[:3]} {r.get("verify", [])[-1:]}')
            else:
                show(f'k={k}/{n}', r, rec['cold'])
    print(f'  {len(ks)} of {n} records resumed from, {bad} differ')
    if not args.keep:
        shutil.rmtree(root, ignore_errors=True)
    return rec


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--docs', required=True)
    ap.add_argument('--every', type=int, default=1)
    ap.add_argument('--at', default='0.1,0.5,0.9,1.0')
    ap.add_argument('--all', action='store_true')
    ap.add_argument('--counted-all', action='store_true')
    ap.add_argument('--verify', action='store_true')
    ap.add_argument('--keep', action='store_true')
    ap.add_argument('--json')
    args = ap.parse_args()
    engine = proflib.Engine('new', args.engine)
    recs = []
    for doc in proflib.resolve_docs(args.docs.split(',')):
        recs.append(one(engine, doc, args))
    if args.json:
        with open(args.json, 'w') as f:
            json.dump(recs, f, indent=1)


if __name__ == '__main__':
    main()
