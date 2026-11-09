#!/usr/bin/env -S python3 -I
"""check_normalize.py -- how far to trust run.py's l3build log normal form and the way the suites are run.

    check_normalize.py lua [--limit N]    the Python port against l3build's own rewrite_log, on every prepared oracle log
    check_normalize.py tlg [--list]       the oracle's normalized log against the suite's .tlg, per test directory

`lua` needs only the installed l3build: it loads scripts/l3build/l3build-check.lua in texlua and calls rewrite_log.  Equal output on real
logs is what shows the port is right.

`tlg` measures the conventions.  The tests were written against a format built from their own tree with their own hyphen.cfg, and
this runner uses the installed format, so a miss is not a bug by itself; the oracle's own log remains the reference.  A high match
rate shows that the tests really ran as the LaTeX team meant them to, and the mismatches are listed to be read.
"""
import argparse
import collections
import importlib.util
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
L3BUILD = '/usr/share/texmf-dist/scripts/l3build/l3build-check.lua'
MANIFEST = os.path.join(REPO, 'tests', 'corpus', 'manifests', 'public.json')
HARNESS = r'''
maxprintline = 9999
stdengine = "pdftex"
asciiengines = {"pdftex"}
recordstatus = false
checkruns = 1
os_newline = "\n"
luatex_version = 120
testdir = "."
dofile(arg[1])
for line in io.lines(arg[2]) do
  local src, dst = line:match("^([^\t]*)\t(.*)$")
  rewrite_log(src, dst, "pdftex", {})
end
'''


def load_runner():
    spec = importlib.util.spec_from_file_location('corpus_run', os.path.join(REPO, 'tests', 'corpus', 'run.py'))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def prepared(run):
    """(entry, meta) for every l3build entry with a current oracle and a log."""
    entries, _, _ = run.load_manifests([MANIFEST])
    for e in entries:
        if e.get('log_normalize') != 'l3build' or e.get('excluded'):
            continue
        meta = run.load_valid_oracle(e)
        if meta:
            yield e, meta


def high_to_caret(data):
    return re.sub(rb'[\x80-\xff]', lambda m: b'^^%02x' % m.group()[0], data)


def cmd_lua(args):
    run = load_runner()
    with tempfile.TemporaryDirectory(dir=os.path.join(REPO, 'target', 'corpus')) as td:
        harness = os.path.join(td, 'harness.lua')
        with open(harness, 'w') as f:
            f.write(HARNESS)
        pairs, jobs = [], []
        for e, meta in prepared(run):
            log = os.path.join(meta['dir'], 'final', meta['jobname'] + '.log')
            raw = run.open_bytes(log)
            if b'START-TEST-LOG' not in raw:
                continue
            if b'--INSERT-PDF-TAGS' in raw:         # runs an external tool, show-pdf-tags, inside rewrite_log
                continue
            w = meta.get('workdir_path') or '/nonexistent'
            fed = high_to_caret(raw.replace(os.fsencode(w) + b"/", b"./"))
            i = len(jobs)
            src, dst = os.path.join(td, f'{i}.log'), os.path.join(td, f'{i}.new')
            with open(src, 'wb') as f:
                f.write(fed)
            pairs.append(f'{src}\t{dst}\n')
            jobs.append((e['id'], raw, w, dst))
            if args.limit and len(jobs) >= args.limit:
                break
        listing = os.path.join(td, 'list')
        with open(listing, 'w') as f:
            f.writelines(pairs)
        p = subprocess.run(['texlua', harness, L3BUILD, listing], capture_output=True, cwd=td)
        if p.returncode:
            print(p.stdout.decode('latin-1')[-2000:], p.stderr.decode('latin-1')[-2000:])
            return 2
        same, diffs = 0, []
        for name, raw, w, dst in jobs:
            with open(dst, 'rb') as f:
                ref = [x for x in high_to_caret(f.read()).split(b'\n') if x]
            got = [x[1] for x in run.l3build_normalize_log(raw, [w])]
            if ref == got:
                same += 1
            else:
                k = next((i for i in range(min(len(ref), len(got))) if ref[i] != got[i]), min(len(ref), len(got)))
                diffs.append((name, k, ref[k] if k < len(ref) else b'<end>', got[k] if k < len(got) else b'<end>'))
        print(f'port vs l3build rewrite_log: {same} / {len(jobs)} logs equal')
        for name, k, a, b in diffs[:40]:
            print(f'  {name} line {k + 1}\n    lua    {a[:160]!r}\n    python {b[:160]!r}')
    return 0 if not diffs else 1


def tlg_of(e, main_dir, stem):
    """The reference l3build would pick for this entry: <name>.<engine>.tlg, else <name>.tlg, which belongs to stdengine."""
    engine = 'etex' if e.get('fmt') in ('latex', 'etex') else 'pdftex'
    for name in (f'{stem}.{engine}.tlg', f'{stem}.tlg'):
        p = os.path.join(main_dir, name)
        if os.path.isfile(p):
            return p, name
    return None, None


def cmd_tlg(args):
    run = load_runner()
    groups = collections.defaultdict(lambda: collections.Counter())
    misses = []
    first_kinds = collections.Counter()
    for e, meta in prepared(run):
        suite = e['id'].rsplit('/', 1)[0]
        c = groups[suite]
        c['entries'] += 1
        if not e['main'].endswith('.lvt'):
            c['not-lvt'] += 1
            continue
        stem = os.path.basename(e['main'])[:-4]
        tlg, name = tlg_of(e, os.path.dirname(e['main']), stem)
        if not tlg:
            c['no-tlg'] += 1
            continue
        raw = run.open_bytes(os.path.join(meta['dir'], 'final', meta['jobname'] + '.log'))
        if b'START-TEST-LOG' not in raw:
            c['no-start'] += 1
            misses.append((e['id'], 'the log has no START-TEST-LOG', ''))
            continue
        got = [x[1] for x in run.l3build_normalize_log(raw, [meta.get('workdir_path')])]
        with open(tlg, 'rb') as f:
            ref = [x for x in f.read().replace(b'\r\n', b'\n').split(b'\n') if x.strip(b' ')]
        if got == ref:
            c['match'] += 1
            continue
        c['differ'] += 1
        k = next((i for i in range(min(len(ref), len(got))) if ref[i] != got[i]), min(len(ref), len(got)))
        a = ref[k] if k < len(ref) else b'<end>'
        b = got[k] if k < len(got) else b'<end>'
        misses.append((e['id'], f'line {k + 1}: tlg {a[:150]!r}', f'oracle {b[:150]!r}'))
        first_kinds[re.sub(rb'[0-9]+', b'N', a[:40])] += 1
    total = collections.Counter()
    print(f'{"suite":<58}{"entries":>8}{"match":>7}{"differ":>7}{"no-tlg":>7}{"other":>7}')
    for s in sorted(groups):
        c = groups[s]
        total.update(c)
        print(f'{s:<58}{c["entries"]:>8}{c["match"]:>7}{c["differ"]:>7}{c["no-tlg"]:>7}{c["not-lvt"] + c["no-start"]:>7}')
    print(f'{"TOTAL":<58}{total["entries"]:>8}{total["match"]:>7}{total["differ"]:>7}{total["no-tlg"]:>7}{total["not-lvt"] + total["no-start"]:>7}')
    print(f'oracle log equals the suite .tlg on {total["match"]} / {total["match"] + total["differ"] + total["no-start"]} tests with a reference for this engine')
    print('most common first differing tlg lines:')
    for k, v in first_kinds.most_common(25):
        print(f'  {v:>5}  {k!r}')
    if args.list:
        for m in misses:
            print(' ', *m)
    return 0


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest='cmd', required=True)
    p = sub.add_parser('lua')
    p.add_argument('--limit', type=int)
    p.set_defaults(func=cmd_lua)
    p = sub.add_parser('tlg')
    p.add_argument('--list', action='store_true')
    p.set_defaults(func=cmd_tlg)
    args = ap.parse_args()
    return args.func(args)


if __name__ == '__main__':
    sys.exit(main())
