#!/usr/bin/env -S python3 -I
r"""The native \pickup@font of crates/engine/src/native_font.rs, against the real macros.  tests/integration/native_font holds against the oracle what is here held against the same program with the path off.

    python3 -I tests/native_font/run.py --engine WRAPPER [--work DIR] [-j N] [--filter REGEX] [--corpus REGEX] [--show N]

WRAPPER is called as tests/corpus/run.py calls an engine, with -fmt=pdflatex.

Over the cases, tests/integration/native_font/*.tex:

  same     the log, the PDF and every other file the run wrote are the bytes of a run with RSTEX_NO_PF
  state    what stays between runs is the same: strings, names and meanings, the marks, the allocators' state.  A word of mem can hold a list's address, so only that hash is left out
  takes    the path was taken as often as the case's line `%% takes: N` says: at least N, and none at all for N=0.  So a table that has gone stale fails here, and does not just get slower
  margins  on peak*.tex under RSTEX_PF_PEAK, where the real macros always run: what they need of each stack is under the margins in native_font.rs, of the buffer no more than a font's own conditions count, and what they leave in cur_val, cur_val_level and radix is what the path leaves
  warm     a second run with the cache of the first is restored, says the trace, takes the path, and gives the same files
  key      that cache with RSTEX_NO_PF, or RSTEX_NO_SHARE: not restored, and the same files
  share    RSTEX_NO_SHARE gives the same files

--corpus does `same` and `state` over the entries of the corpus that it selects, one pass from the oracle's side files, and counts those that took the path.  Prints ids and counts only.  Exit status 1 if anything differs.
"""
import argparse
import concurrent.futures as cf
import glob
import hashlib
import importlib.util
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
CASES = os.path.join(REPO, 'tests', 'integration', 'native_font')
SOURCE = os.path.join(REPO, 'crates', 'engine', 'src', 'native_font.rs')
ENV = {'SOURCE_DATE_EPOCH': '1700000000', 'FORCE_SOURCE_DATE': '1', 'TZ': 'UTC', 'LC_ALL': 'C', 'LANG': 'C', 'PATH': os.environ['PATH'], 'HOME': os.environ['HOME'], 'RSTEX_NO_COMMUTE': '1'}
# What differs between the runs of a pair only because a list's address is in a word of mem.
STATE = re.compile(r'state: mem [0-9a-f]+ (.*)')
PF = re.compile(r'pf: not armed (\d+), context (\d+), guards (\d+), font (\d+), taken (\d+); arms (\d+)')
PEAK = re.compile(r'pf peak, between calls, over (\d+) calls: input \+(-?\d+) param \+(-?\d+) save \+(-?\d+) buffer \+(-?\d+); (\d+) in a row, cur_val (-?\d+) level (-?\d+) radix (-?\d+)')


def files_of(out):
    res = {}
    for root, _, names in os.walk(out):
        if 'cache' in root.split(os.sep):
            continue
        for n in sorted(names):
            if n.startswith('.'):
                continue
            with open(os.path.join(root, n), 'rb') as f:
                res[os.path.relpath(os.path.join(root, n), out)] = hashlib.sha256(f.read()).hexdigest()
    return res


def header(path):
    takes, env = None, {}
    with open(path, errors='replace') as f:
        for line in f:
            if not line.startswith('%'):
                break
            m = re.match(r'%% takes: (\d+)', line)
            if m:
                takes = int(m.group(1))
            m = re.match(r'% env: (.*)', line)
            if m:
                env.update(x.split('=', 1) for x in m.group(1).split())
    return takes, env


class Run:
    def __init__(self, rc, err, files):
        self.rc, self.files = rc, files
        self.text = err.decode('latin-1')
        m = PF.search(self.text)
        self.taken = int(m.group(5)) if m else None
        m = STATE.search(self.text)
        self.state = m.group(1) if m else None
        self.peak = PEAK.search(self.text)


def tex(engine, name, cwd, out, env, cache=None):
    """A case, in the one directory all runs of a pair use, for the same /ID and the same log."""
    shutil.rmtree(out, ignore_errors=True)
    os.makedirs(out)
    e = dict(ENV, RSTEX_PF_STATS='1', XDG_CACHE_HOME=cache or os.path.join(out, 'cache'), **env)
    if cache is None:
        e['RSTEX_NO_SNAPSHOT'] = '1'
    p = subprocess.run([engine, '-fmt=pdflatex', '-interaction=batchmode', '-no-shell-escape', '-output-directory=' + out, name + '.tex'], cwd=cwd, env=e, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=args.timeout)
    return Run(p.returncode, p.stderr, files_of(out))


def compare(a, b, what, fails):
    if a.files != b.files:
        fails.append('%s: %s' % (what, sorted(k for k in set(a.files) | set(b.files) if a.files.get(k) != b.files.get(k))))
    if a.rc != b.rc:
        fails.append('%s: exit status %s and %s' % (what, a.rc, b.rc))


def constants():
    with open(SOURCE) as f:
        src = f.read()
    return {k: int(v) for k, v in re.findall(r'const (MARGIN_\w+): i32 = (\d+);', src)}


def one(path):
    name = os.path.splitext(os.path.basename(path))[0]
    fails = []
    takes, env = header(path)
    root = os.path.join(args.work, name)
    shutil.rmtree(root, ignore_errors=True)
    out = os.path.join(root, 'o')
    on = tex(args.engine, name, CASES, out, env)
    off = tex(args.engine, name, CASES, out, dict(env, RSTEX_NO_PF='1'))
    compare(on, off, 'same', fails)
    if on.state is None or on.state != off.state:
        fails.append('state: %s and %s' % (on.state, off.state))
    if takes is None:
        fails.append('no line %% takes: N')
    elif on.taken is None or (takes == 0 and on.taken != 0) or on.taken < takes:
        fails.append('takes: %s, where the case says %s' % (on.taken, takes))
    if off.taken != 0:
        fails.append('with RSTEX_NO_PF it still took the path %s times' % off.taken)
    if name.startswith('peak'):
        p = tex(args.engine, name, CASES, out, dict(env, RSTEX_PF_PEAK='1'))
        margins = constants()
        if not p.peak:
            fails.append('margins: nothing said')
        else:
            calls, up_in, up_param, up_save, up_buf, row, val, level, radix = (int(x) for x in p.peak.groups())
            if row < 100:
                fails.append('margins: %d calls in a row' % row)
            for k, up in (('MARGIN_IN', up_in), ('MARGIN_PARAM', up_param), ('MARGIN_SAVE', up_save)):
                if not 0 < up <= margins[k]:
                    fails.append('margins: %s is %d, and the real macros need %d' % (k, margins[k], up))
            if calls < 500 or up_buf <= 0:
                fails.append('margins: %d calls, the buffer rose by %d' % (calls, up_buf))
            # What a known font's own conditions count: `MT@subst@` and the name of the font last selected, which stays in \\font@name after a group: `OT1/cmr/m/it/10` or `T1/lmr/m/it/10`.
            need = 9 + len('T1/lmr/m/it/10' if 'alias' in name else 'OT1/cmr/m/it/10')
            if up_buf != need:
                fails.append('margins: the buffer rose by %d, and the path counts %d' % (up_buf, need))
            want = 35 if 'hook' in name else -1
            if (val, level, radix) != (want, 0, 0):
                fails.append('margins: the real macros leave cur_val %d, cur_val_level %d, radix %d, and the path %d, 0, 0' % (val, level, radix, want))
    if env.get('expand_depth') is None:
        cache = os.path.join(root, 'cache')
        c1 = tex(args.engine, name, CASES, out, dict(env, RSTEX_CHECKPOINT_POLICY='always', RSTEX_CHECKPOINT_TRACE='1'), cache)
        c2 = tex(args.engine, name, CASES, out, dict(env, RSTEX_CHECKPOINT_POLICY='always', RSTEX_CHECKPOINT_TRACE='1'), cache)
        restored = re.search(r'restored', c2.text, re.I)
        if not restored:
            fails.append('warm: not restored')
        compare(on, c1, 'a run that writes a checkpoint', fails)
        compare(on, c2, 'warm', fails)
        if takes and c2.taken is not None and c2.taken < takes:
            fails.append('warm: took the path %s times, where the case says %s' % (c2.taken, takes))
        for sw in ('RSTEX_NO_PF', 'RSTEX_NO_SHARE'):
            c3 = tex(args.engine, name, CASES, out, dict(env, RSTEX_CHECKPOINT_POLICY='always', RSTEX_CHECKPOINT_TRACE='1', **{sw: '1'}), cache)
            if re.search(r'restored', c3.text, re.I):
                fails.append('key: %s restored a checkpoint made without it' % sw)
            compare(on, c3, sw, fails)
    shutil.rmtree(root, ignore_errors=True)
    return name, on.taken, fails


def corpus_one(job):
    eid = job
    e = ENTRIES[eid]
    meta, _ = RUN.prepare_entry(e)
    if RUN.effective_class(meta, e) != 'normal':
        return eid, 'oracle-fails', None, []
    w = RUN.new_workdir()
    out = os.path.join(w, 'o')
    res = {}
    try:
        for tag, extra in (('on', {}), ('off', {'RSTEX_NO_PF': '1'})):
            shutil.rmtree(out, ignore_errors=True)
            os.makedirs(out)
            RUN.restore_state(meta, out)
            env = RUN.candidate_env(e, out)
            env.update(RSTEX_PF_STATS='1', RSTEX_NO_SNAPSHOT='1', **extra)
            stderr = os.path.join(w, 'stderr-' + tag)
            with open(stderr, 'wb') as err:
                p = subprocess.run(RUN.tex_argv(e, args.engine, [], out), cwd=e['cwd'], env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=err, timeout=e['timeout'] * 10)
            with open(stderr, 'rb') as f:
                res[tag] = Run(p.returncode, f.read(), files_of(out))
        fails = []
        a, b = res['on'], res['off']
        compare(a, b, 'same', fails)
        if a.state != b.state:
            fails.append('state: differs')
        return eid, 'normal', a.taken, fails
    except subprocess.TimeoutExpired:
        return eid, 'timeout', None, []
    finally:
        shutil.rmtree(w, ignore_errors=True)


def main():
    global args, ENTRIES, RUN
    ap = argparse.ArgumentParser()
    ap.add_argument('--engine', required=True)
    ap.add_argument('--work', default=os.path.join(REPO, 'target', 'native-font'))
    ap.add_argument('-j', '--jobs', type=int, default=4)
    ap.add_argument('--filter')
    ap.add_argument('--corpus')
    ap.add_argument('--timeout', type=float, default=600)
    ap.add_argument('--show', type=int, default=40)
    args = ap.parse_args()
    args.engine = os.path.abspath(args.engine)
    args.work = os.path.abspath(args.work)
    if args.corpus:
        spec = importlib.util.spec_from_file_location('corpus_run', os.path.join(REPO, 'tests', 'corpus', 'run.py'))
        RUN = importlib.util.module_from_spec(spec)
        sys.modules['corpus_run'] = RUN
        spec.loader.exec_module(RUN)
        entries, _, _ = RUN.load_manifests(None)
        ENTRIES = {e['id']: e for e in entries if re.search(args.corpus, e['id']) and not e.get('excluded') and e['mode'] not in ('ini', 'ini-format')}
        same = taken = differ = passed = 0
        with cf.ThreadPoolExecutor(args.jobs) as pool:
            for eid, kind, t, fails in pool.map(corpus_one, sorted(ENTRIES)):
                if kind != 'normal':
                    passed += 1
                    continue
                if fails:
                    differ += 1
                    if differ <= args.show:
                        print('DIFFERS %s: %s' % (eid, '; '.join(fails)), flush=True)
                else:
                    same += 1
                if t:
                    taken += 1
        print('corpus: %d of %d the same, with RSTEX_NO_PF and without; %d took the path; %d the oracle fails on or that timed out' % (same, same + differ, taken, passed))
        return 1 if differ else 0
    cases = sorted(glob.glob(os.path.join(CASES, '*.tex')))
    if args.filter:
        cases = [c for c in cases if re.search(args.filter, os.path.basename(c))]
    bad = 0
    with cf.ThreadPoolExecutor(args.jobs) as pool:
        for name, taken, fails in pool.map(one, cases):
            print('%-20s %s, took the path %s times' % (name, 'FAIL' if fails else 'pass', taken), flush=True)
            for f in fails:
                print('    ' + f)
            bad += bool(fails)
    print('native_font: %d of %d cases pass' % (len(cases) - bad, len(cases)))
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
