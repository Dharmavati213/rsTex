#!/usr/bin/env -S python3 -I
r"""The native diagram path of DESIGN section 10, judged per diagram against real PGF on tests/corpus/manifests/cd*.json.

    python3 -I tests/corpus/cd/compare_native.py eligibility --commute target/release/commute
    python3 -I tests/corpus/cd/compare_native.py geometry --engine E [--engine-arg ARG]... [--dpi 400] [--filter REGEX] [-j N]
    python3 -I tests/corpus/cd/compare_native.py check-negative       # prove `geometry` fails when a diagram is off by 0.05 bp

`eligibility` asks the `commute` CLI which diagrams it handles and why it declines the others.  It says nothing about correctness, and it is the denominator of the
claim that the fast path is worth having: "Worth ~480 ms on the thesis" holds only for diagrams it does not decline.

`geometry` is the gate.  The oracle has already drawn every diagram with real PGF on its own page (tests/corpus/cd/extract.py).  The candidate draws them with the native path. Per page:

  dims     \wd, \ht, \dp of the diagram's box, from the [[unit]] line in the log.  Exact: these feed the line breaking and page layout around the diagram.
  raster   pdftoppm at --dpi on both sides.  Pass when no pixel differs by more than --max-level on a 0-255 scale, after the small allowance for antialiasing.
           At 400 dpi 0.05 bp is 0.28 pixel, and that is enough to change the edge pixels of a line.
  text     the shown strings, in order, equal (a label in the wrong cell has the right pixels nowhere near).

A candidate that falls back to real PGF for a diagram is identical there, which proves nothing about the native path, so an engine should report it.  The tool counts lines
`[[commute fallback]]` in the log, which the engine prints under --commute-report, and states the share: a pass count over diagrams that all fell back would be vacuous.
"""
import argparse
import collections
import concurrent.futures as cf
import hashlib
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
CORPUS = os.path.dirname(HERE)
spec = importlib.util.spec_from_file_location('corpus_run', os.path.join(CORPUS, 'run.py'))
run = importlib.util.module_from_spec(spec)
spec.loader.exec_module(run)
MANIFESTS = [os.path.join(run.MANIFEST_DIR, 'cd.json'), os.path.join(run.MANIFEST_DIR, 'cd.local.json')]
ROOT = os.path.join(run.TARGET, 'cd-native')
DIAGRAM = re.compile(rb'\\typeout\{\[\[unit-start ([^\]]+)\]\]\}\n\\setbox\\cdunit=\\hbox\{(.*?)\}\n\\typeout\{\[\[unit ', re.S)


def entries():
    files = [m for m in MANIFESTS if os.path.isfile(m)]
    es, _, _ = run.load_manifests(files)
    return [e for e in es if e.get('units')]


def eligibility(args):
    os.makedirs(ROOT, exist_ok=True)
    work = os.path.join(ROOT, 'elig')
    os.makedirs(work, exist_ok=True)
    per = collections.defaultdict(collections.Counter)
    reasons = collections.Counter()
    categories = collections.Counter()
    results = {}

    def one(item):
        name, source, text = item
        env = {k: os.environ[k] for k in ('PATH', 'HOME') if k in os.environ}
        p = subprocess.run([args.commute, '-o', work, '--pdf', '--stem', 'x' + hashlib.sha1(name.encode()).hexdigest()[:12], '-'], input=text, capture_output=True, env=env, timeout=60)
        err = p.stderr.decode('utf-8', 'replace').strip()
        m = re.search(r'not handled: (.*)', err)
        return name, source, p.returncode, (m.group(1) if m else err[:80])

    items = []
    for e in entries():
        data = run.open_bytes(e['main'])
        for m in DIAGRAM.finditer(data):
            items.append((e['id'] + '#' + m.group(1).decode(), e['source'], m.group(2)))
    with cf.ThreadPoolExecutor(args.jobs) as ex:
        for name, source, rc, why in ex.map(one, items):
            handled = rc == 0
            per[source]['handled' if handled else 'needs real PGF'] += 1
            if not handled:
                # A reason can quote a macro name, so a private diagram adds only to the category, the part before the colon.
                categories[why.split(':')[0][:60]] += 1
                if source != 'cd-local':
                    reasons[why[:60]] += 1
            results[name] = {'handled': handled, 'why': None if handled else why[:120] if source != 'cd-local' else why.split(':')[0][:60]}
    shutil.rmtree(work, ignore_errors=True)
    total = sum(sum(v.values()) for v in per.values())
    ok = sum(v['handled'] for v in per.values())
    print(f'eligibility: {ok} of {total} diagrams handled by commute ({100 * ok / max(1, total):.1f}%)')
    for s, v in sorted(per.items()):
        print(f'  {s:<10} handled {v["handled"]}, needs real PGF {v["needs real PGF"]}')
    print('why it declines, by category:')
    for k, v in categories.most_common():
        print(f'  {v:>5} {k}')
    print('the most common reasons in public sources:')
    for k, v in reasons.most_common(30):
        print(f'  {v:>5} {k}')
    run.write_json(os.path.join(ROOT, 'eligibility.json'), results)
    return 0


def page_rasters(pdf, dpi):
    """One PPM per page from one process."""
    p = subprocess.Popen(['pdftoppm', '-r', str(dpi), pdf], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    f = p.stdout
    try:
        while True:
            head = f.read(2)
            if not head:
                break
            fields, cur = [], b''
            while len(fields) < 3:
                c = f.read(1)
                if c.isspace():
                    if cur:
                        fields.append(int(cur))
                        cur = b''
                else:
                    cur += c
            yield fields[0], fields[1], f.read(fields[0] * fields[1] * 3)
    finally:
        f.close()
        p.wait()


def unit_dims(log):
    data = re.sub(rb'\n(?=[^\n]{0,78}\]\])', b'', log)
    return {m.group(1).decode(): m.group(2).decode() for m in re.finditer(rb'\[\[unit ([^ \]]+) ([^\]]*)\]\]', data)}


def page_texts(pdf):
    p = subprocess.run(['pdftotext', '-layout', pdf, '-'], capture_output=True)
    return [re.sub(rb'\s+', b' ', x).strip() for x in p.stdout.split(b'\f')]


def geometry_entry(e, opts):
    meta, _ = run.prepare_entry(e)
    res = {'id': e['id'], 'units': len(e['units']), 'pass': 0, 'fail': [], 'fallback': 0}
    if run.effective_class(meta, e) != 'normal':
        res['oracle'] = 'oracle-fails'
        return res
    w = run.new_workdir()
    out = os.path.join(w, 'o')
    try:
        r = run.run_proc(run.tex_argv(e, opts.engine, opts.engine_arg, out), e['cwd'], run.candidate_env(e, out), e['timeout'] * 10, os.path.join(w, 'stdout'))
        job = e['jobname']
        final = os.path.join(meta['dir'], 'final')
        la = run.open_bytes(os.path.join(final, job + '.log'))
        lb = run.open_bytes(os.path.join(out, job + '.log'))
        res['fallback'] = lb.count(b'[[commute fallback]]')
        if r['timed_out'] or r['signal'] or not os.path.isfile(os.path.join(out, job + '.pdf')):
            res['fail'] = [{'unit': '*', 'why': f'candidate rc {r["rc"]} signal {r["signal"]} timed_out {r["timed_out"]}'}]
            return res
        da, db = unit_dims(la), unit_dims(lb)
        ta, tb = page_texts(os.path.join(final, job + '.pdf')), page_texts(os.path.join(out, job + '.pdf'))
        ra = page_rasters(os.path.join(final, job + '.pdf'), opts.dpi)
        rb = page_rasters(os.path.join(out, job + '.pdf'), opts.dpi)
        for k, u in enumerate(e['units']):
            x, y = next(ra, None), next(rb, None)
            why = None
            if da.get(u) != db.get(u):
                why = f'dims: real PGF {da.get(u)}, candidate {db.get(u)}'
            elif x is None or y is None or x[:2] != y[:2]:
                why = f'page size: real PGF {x[:2] if x else None}, candidate {y[:2] if y else None}'
            elif (ta[k] if k < len(ta) else b'') != (tb[k] if k < len(tb) else b''):
                why = 'text on the page differs'
            elif x[2] != y[2]:
                a, b = x[2], y[2]
                bad = sum(1 for i in range(len(a)) if abs(a[i] - b[i]) > opts.max_level)
                if bad > opts.max_bad:
                    why = f'raster: {bad} channel values differ by more than {opts.max_level}, at {opts.dpi} dpi'
            if why:
                res['fail'].append({'unit': u, 'why': why})
            else:
                res['pass'] += 1
        return res
    finally:
        shutil.rmtree(w, ignore_errors=True)


def geometry(args):
    es = [e for e in entries() if not args.filter or re.search(args.filter, e['id'])]
    args.engine = os.path.abspath(args.engine) if '/' in args.engine else args.engine
    run.install_limits()
    with cf.ThreadPoolExecutor(args.jobs) as ex:
        results = list(ex.map(lambda e: geometry_entry(e, args), es))
    units = sum(r['units'] for r in results)
    passed = sum(r['pass'] for r in results)
    fails = sum(len(r['fail']) for r in results)
    orac = sum(r['units'] for r in results if r.get('oracle'))
    fallback = sum(r['fallback'] for r in results)
    print(f'PASS native diagrams: {passed} / {units - orac} that real PGF compiled; {orac} in documents the oracle fails on; {fallback} declined to real PGF by the engine\'s own count')
    if fallback and passed:
        print(f'  so at most {passed - fallback} passes tested the native path')
    for r in results:
        for f in r['fail'][:3]:
            print(f'  {r["id"]}#{f["unit"]}: {f["why"]}')
    os.makedirs(ROOT, exist_ok=True)
    run.write_json(os.path.join(ROOT, 'geometry.json'), results)
    return 1 if fails else 0


def check_negative(args):
    """A wrapper that shifts the arrows of every diagram by a small distance and changes nothing else must fail; the real pdflatex must pass."""
    os.makedirs(ROOT, exist_ok=True)
    wrappers = os.path.join(ROOT, 'wrappers')
    os.makedirs(wrappers, exist_ok=True)
    args.filter = args.filter or r'cd/manuals/tikz-cd/tikz-cd-doc|cd/ladder/14'
    args.max_level = args.max_level
    rows = []
    for label, shift in (('none', None), ('shift-0.5bp', '0.5bp'), ('shift-0.05bp', '0.05bp'), ('shift-0.02bp', '0.02bp')):
        path = os.path.join(wrappers, label)
        body = 'exec /usr/bin/pdflatex "$@"\n' if shift is None else (
            'args=("$@"); last="${args[-1]}"; unset \'args[-1]\'\n'
            f'pre=\'\\AddToHook{{begindocument/end}}{{\\tikzcdset{{every arrow/.append style={{transform canvas={{yshift={shift}}}}}}}}}\'\n'
            'if [[ "$last" == \\\\* ]]; then new="$pre$last"; else new="$pre\\input \\"$last\\""; fi\n'
            'exec /usr/bin/pdflatex "${args[@]}" "$new"\n')
        with open(path, 'w') as f:
            f.write('#!/bin/bash\n' + body)
        os.chmod(path, 0o755)
        args.engine = path
        args.engine_arg = []
        es = [e for e in entries() if re.search(args.filter, e['id'])]
        run.install_limits()
        res = [geometry_entry(e, args) for e in es]
        passed = sum(r['pass'] for r in res)
        failed = sum(len(r['fail']) for r in res)
        example = next((f['why'] for r in res for f in r['fail']), '')
        rows.append((label, passed, failed))
        print(f'{label:<14} pass {passed}, fail {failed}  {example[:100]}')
    ok = rows[0][2] == 0 and rows[0][1] > 1 and rows[1][1] == 0 and rows[2][2] > 0      # more than one: a filter that has gone stale must not leave a control of one diagram
    print('negative control ' + ('OK: the real engine passes, a shift of 0.05 bp fails' if ok else 'NOT OK'))
    return 0 if ok else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest='cmd', required=True)
    p = sub.add_parser('eligibility')
    p.add_argument('--commute', required=True)
    p.add_argument('-j', '--jobs', type=int, default=6)
    p.set_defaults(func=eligibility)
    for name, func in (('geometry', geometry), ('check-negative', check_negative)):
        p = sub.add_parser(name)
        if name == 'geometry':
            p.add_argument('--engine', required=True)
            p.add_argument('--engine-arg', action='append', default=[])
        p.add_argument('--dpi', type=int, default=400)
        p.add_argument('--max-level', type=int, default=48, help='a channel value may differ by this much, for antialiasing')
        p.add_argument('--max-bad', type=int, default=0, help='how many channel values may differ by more')
        p.add_argument('--filter')
        p.add_argument('-j', '--jobs', type=int, default=6)
        p.set_defaults(func=func)
    args = ap.parse_args()
    os.makedirs(run.WORK_ROOT, exist_ok=True)
    os.makedirs(run.EMPTY_TEXMF, exist_ok=True)
    return args.func(args)


if __name__ == '__main__':
    sys.exit(main())
