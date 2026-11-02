#!/usr/bin/env python3
"""Run the micro-benchmarks and report ops/s.

usage: run.py [--engine CMD] [--runs N] [--out DIR] [--json FILE] [--compare BASE.json] [files...]

CMD defaults to pdflatex. Each file is run from this directory with the
output directory in DIR. A benchmark's time is the minimum over N runs.
net_ns subtracts the file's "empty" per-op time (tree overhead), divided by
the benchmark's inner ops per body.
"""
import argparse, glob, json, os, re, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))

def run_file(engine, path, out, runs):
    res = {}
    env = dict(os.environ, max_print_line='1000', TEXINPUTS=HERE + ':')
    for _ in range(runs):
        cmd = engine.split() + ['-interaction=nonstopmode', '-output-directory=' + out, os.path.basename(path)]
        p = subprocess.run(cmd, cwd=HERE, env=env, capture_output=True, text=True, errors='replace')
        seen = 0
        for m in re.finditer(r'^BENCH (\S+) ops=(\d+) inner=(\d+) t=(\d+)$', p.stdout, re.M):
            name, ops, inner, t = m.group(1), int(m.group(2)), int(m.group(3)), int(m.group(4))
            seen += 1
            if name not in res or t < res[name][1]:
                res[name] = (ops, t, inner)
        errs = re.findall(r'^! .*$', p.stdout, re.M)
        if errs:
            print(f'  {os.path.basename(path)}: {len(errs)} TeX errors, first: {errs[0]}', file=sys.stderr)
    return res

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--engine', default='pdflatex')
    ap.add_argument('--runs', type=int, default=3)
    ap.add_argument('--out')
    ap.add_argument('--json')
    ap.add_argument('--compare')
    ap.add_argument('files', nargs='*')
    a = ap.parse_args()
    out = a.out or tempfile.mkdtemp(prefix='rstex-micro-')
    os.makedirs(out, exist_ok=True)
    files = a.files or sorted(f for f in glob.glob(os.path.join(HERE, '[0-9][0-9]-*.tex')))
    base = json.load(open(a.compare)) if a.compare else None
    allr = {}
    for f in files:
        f = os.path.join(HERE, f) if not os.path.isabs(f) else f
        fname = os.path.basename(f)[:-4]
        r = run_file(a.engine, f, out, a.runs)
        empty = None
        if 'empty' in r:
            empty = r['empty'][1] / 65536 / r['empty'][0]
        print(f'== {fname}')
        allr[fname] = {}
        for name, (ops, t, inner) in r.items():
            sec = t / 65536
            per = sec / ops if ops else 0
            net = per - empty / inner if empty is not None and name != 'empty' else per
            net = max(net, 0)
            row = dict(ops=ops, seconds=round(sec, 5), ops_per_sec=round(ops / sec) if sec else None, ns_per_op=round(per * 1e9, 1), net_ns=round(net * 1e9, 1))
            allr[fname][name] = row
            cmpstr = ''
            if base and fname in base and name in base[fname] and row['ops_per_sec']:
                cmpstr = f"  x{row['ops_per_sec'] / base[fname][name]['ops_per_sec']:.2f}"
            print(f"{name:32s} {ops:9d} ops {sec:8.3f} s {row['ops_per_sec'] or 0:12d} ops/s {row['ns_per_op']:12.1f} ns/op {row['net_ns']:12.1f} net{cmpstr}")
    if a.json:
        json.dump(allr, open(a.json, 'w'), indent=1)

if __name__ == '__main__':
    main()
