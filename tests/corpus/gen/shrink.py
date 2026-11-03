#!/usr/bin/env -S python3 -I
"""Shrink a failing test case: remove lines while the divergence between the oracle and a candidate persists.

    python3 -I tests/corpus/gen/shrink.py --engine target/release/rstex FILE.tex [--mode plain|latex] [--out SMALL.tex]
    python3 -I tests/corpus/gen/shrink.py --engine target/release/rstex --id coverage/fuzz/00012 [--out SMALL.tex]

Delta debugging (ddmin) over lines, then a last pass that tries each remaining line alone.  Both sides run exactly as in run.py: the same
environment, arguments and log normalization.  A trial counts as still failing when the normalized logs or the exit status differ, the oracle
did not time out, and, with --same-line, when the first differing oracle line has the same text as in the original failure.  With --contains TEXT
the first difference must also mention TEXT, which keeps the search on one failure when a case has several.
"""
import argparse
import importlib.util
import math
import os
import shutil
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location('corpus_run', os.path.join(os.path.dirname(HERE), 'run.py'))
run = importlib.util.module_from_spec(spec)
spec.loader.exec_module(run)


class Trial:
    def __init__(self, args, base_entry):
        self.args = args
        self.base = base_entry
        self.engine = run.engine_info(args.engine, args.engine_arg)['path']
        self.cache = {}
        self.count = 0
        self.dir = tempfile.mkdtemp(dir=run.WORK_ROOT, prefix='shrink-')
        self.want_text = None

    def close(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def side(self, exe, extra, e):
        w = run.new_workdir()
        out = os.path.join(w, 'o')
        r = run.run_proc(run.tex_argv(e, exe, extra, out), e['cwd'], run.entry_env(e), self.args.timeout, os.path.join(w, 'stdout'))
        log = run.normalize_log(run.open_bytes(os.path.join(out, e['jobname'] + '.log')), [out])
        shutil.rmtree(w, ignore_errors=True)
        return r, log

    def divergence(self, lines):
        """None if the two sides agree or the oracle cannot judge this input, else the first difference."""
        with open(os.path.join(self.dir, 'doc.tex'), 'wb') as f:
            f.write(b''.join(lines))
        e = dict(self.base)
        e.update(main=os.path.join(self.dir, 'doc.tex'), main_rel='doc.tex', cwd=self.dir, jobname='doc')
        self.count += 1
        ro, la = self.side(run.oracle_engine(e), [], e)
        if ro['timed_out'] or ro['signal']:
            return None
        rc, lb = self.side(self.engine, self.args.engine_arg, e)
        d = run.first_log_divergence(la, lb)
        if d is None and (ro['rc'] == rc['rc'] or self.args.log_only):
            return None
        return d or {'a': f'exit status {ro["rc"]}', 'b': f'exit status {rc["rc"]}', 'line_a': None, 'line_b': None}

    def fails(self, lines):
        key = b''.join(lines)
        if key in self.cache:
            return self.cache[key]
        if self.args.max_trials and self.count >= self.args.max_trials:
            return False
        d = self.divergence(lines)
        ok = d is not None
        if ok and self.args.contains and self.args.contains not in (d.get('a', '') + d.get('b', '')):
            ok = False
        if ok and self.args.same_line and self.want_text is not None and d.get('a') != self.want_text:
            ok = False
        self.cache[key] = ok
        return ok


def ddmin(lines, trial):
    n = 2
    while len(lines) >= 2:
        chunk = math.ceil(len(lines) / n)
        reduced = False
        for i in range(0, len(lines), chunk):
            candidate = lines[:i] + lines[i + chunk:]
            if candidate and trial.fails(candidate):
                lines = candidate
                n = max(n - 1, 2)
                reduced = True
                break
        if not reduced:
            if n >= len(lines):
                break
            n = min(n * 2, len(lines))
    # Lines that only became removable once others had gone.
    changed = True
    while changed and len(lines) > 1:
        changed = False
        for i in range(len(lines)):
            candidate = lines[:i] + lines[i + 1:]
            if candidate and trial.fails(candidate):
                lines = candidate
                changed = True
                break
    return lines


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('file', nargs='?')
    ap.add_argument('--id', help='take the file and mode from this manifest entry')
    ap.add_argument('--engine', required=True)
    ap.add_argument('--engine-arg', action='append', default=[])
    ap.add_argument('--mode', default='plain', choices=('plain', 'latex'))
    ap.add_argument('--out')
    ap.add_argument('--timeout', type=float, default=60)
    ap.add_argument('--contains', help='the first difference must mention this text')
    ap.add_argument('--same-line', action='store_true', help='the first differing oracle line must stay the same')
    ap.add_argument('--log-only', action='store_true', help='ignore a difference in exit status alone')
    ap.add_argument('--max-trials', type=int, default=0, help='stop testing after this many trials (0: no limit)')
    args = ap.parse_args()
    args.engine = os.path.abspath(args.engine) if '/' in args.engine else args.engine
    if args.out:
        args.out = os.path.abspath(args.out)
    start_cwd_file = os.path.abspath(args.file) if args.file else None
    os.makedirs(run.WORK_ROOT, exist_ok=True)
    os.chdir(run.REPO)
    raw = {'id': 'shrink', 'source': 'shrink', 'mode': args.mode, 'main': 'x.tex', 'cwd': '.', 'jobname': 'doc'}
    if args.id:
        entries, _, _ = run.load_manifests(None)
        match = [e for e in entries if e['id'] == args.id]
        if not match:
            sys.exit(f'no entry {args.id!r}')
        start_cwd_file = match[0]['main']
        raw['mode'] = match[0]['mode']
    if not start_cwd_file:
        sys.exit('give a file or --id')
    base = run.normalize_entry(raw, {}, 'shrink')
    with open(start_cwd_file, 'rb') as f:
        lines = f.read().splitlines(keepends=True)
    trial = Trial(args, base)
    t0 = time.time()
    try:
        first = trial.divergence(lines)
        if first is None:
            print('the two sides agree on the whole file: nothing to shrink')
            return 2
        print(f'{len(lines)} lines; first difference: oracle `{first.get("a")}` | candidate `{first.get("b")}`')
        trial.want_text = first.get('a')
        small = ddmin(lines, trial)
        last = trial.divergence(small)
        out = args.out or os.path.splitext(start_cwd_file)[0] + '.small.tex'
        with open(out, 'wb') as f:
            f.write(b''.join(small))
        print(f'{len(lines)} -> {len(small)} lines in {trial.count} trials, {time.time() - t0:.1f} s; written to {out}')
        if last:
            print(f'first difference now: oracle `{last.get("a")}` | candidate `{last.get("b")}`')
        return 0
    finally:
        trial.close()


if __name__ == '__main__':
    sys.exit(main())
