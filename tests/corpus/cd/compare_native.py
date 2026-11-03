#!/usr/bin/env -S python3 -I
r"""The native diagram path of DESIGN section 10, judged per diagram against real PGF on tests/corpus/manifests/cd*.json.

    python3 -I tests/corpus/cd/compare_native.py eligibility --commute target/release/commute
    python3 -I tests/corpus/cd/compare_native.py geometry --engine E [--engine-arg ARG]... [--dpi 400] [--filter REGEX] [-j N]
    python3 -I tests/corpus/cd/compare_native.py check-negative       # prove `geometry` fails when a diagram is off by 0.05 bp
    python3 -I tests/corpus/cd/compare_native.py documents --engine E [--engine-arg ARG]... [--dpi 200] [--filter REGEX] [-j N]

`eligibility` asks the `commute` CLI which diagrams it handles and why it declines the others.  It says nothing about correctness, and it is the denominator of the
claim that the fast path is worth having: "Worth ~480 ms on the thesis" holds only for diagrams it does not decline.

`geometry` is the gate.  The oracle has already drawn every diagram with real PGF on its own page (tests/corpus/cd/extract.py).  The candidate draws them with the native path. Per page:

  dims     \wd, \ht, \dp of the diagram's box, from the [[unit]] line in the log.  Exact: these feed the line breaking and page layout around the diagram.
  raster   pdftoppm at --dpi on both sides.  Pass when no pixel differs by more than --max-level on a 0-255 scale, after the small allowance for antialiasing.
           At 400 dpi 0.05 bp is 0.28 pixel, and that is enough to change the edge pixels of a line.
  text     the shown strings, in order, equal (a label in the wrong cell has the right pixels nowhere near).  In the order of the content stream, and without blanks: the blanks of pdftotext depend
           on gaps that a rounding can change.

`documents` judges whole documents with the path on, against the oracle's cached result, since a diagram's neighbours matter: the user's documents and the arXiv papers whose oracle loaded tikz-cd.
Per document, from the oracle's last-pass side files, as in tests/corpus/run.py:

  log      the normal form of run.py's compare, up to the line "Here is how much of TeX's memory you used".  The figures from there on, and the size of the PDF, are what the path honestly changes: real PGF
           makes strings, names and fonts' worth of work that it does not, so they are left out.  The page count on the line "Output written" is compared.  Nothing else may differ.
  aux      the files .aux, byte for byte
  pages    the count, the text of each, and the pixels of each at --dpi with the allowance of `geometry`
  qpdf     qpdf --check says nothing is wrong

For user/, local/ and cd-local/ entries only ids and counts are printed.

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


def strip_report(log):
    """The log without the lines of --commute-report, which is the log without the option."""
    log = re.sub(rb'\n\[\[commute [^\n]* %\n', b'', log)
    return re.sub(rb'(?m)^\[\[commute [^\n]*\n', b'', log)


def unit_fates(log):
    """What the engine says became of each diagram: None for the native path, the reason if it declined.  Printed by --commute-report between the two lines of a page."""
    fates, unit = {}, None
    for m in re.finditer(rb'\[\[unit-start ([^\]]+)\]\]|\[\[commute (native|fallback|failure)\]\] ?([^\n]*)', log):
        if m.group(1):
            unit = m.group(1).decode()
        elif unit is not None:
            fates[unit] = None if m.group(2) == b'native' else m.group(3).decode('utf-8', 'replace').removesuffix(' %') or m.group(2).decode()
            unit = None
    return fates


def dims_close(a, b, slack):
    """Equal, the gate's own test.  --dims-slack is for looking past errors of the last digits while working on something else, and says so in the output."""
    if a == b:
        return True
    if not slack or a is None or b is None:
        return False
    try:
        x, y = [float(v[:-2]) for v in a.split()], [float(v[:-2]) for v in b.split()]
    except ValueError:
        return False
    return len(x) == len(y) and all(abs(p - q) <= slack for p, q in zip(x, y))


def page_texts(pdf):
    p = subprocess.run(['pdftotext', '-raw', pdf, '-'], capture_output=True)
    return [re.sub(rb'\s+', b'', x) for x in p.stdout.split(b'\f')]


def raster_compare(x, y, opts):
    """Why two rasters of a page differ too much, or None."""
    if x[2] == y[2]:
        return None
    a, b, width = x[2], y[2], x[0]
    stride = width * 3
    bad = []
    for i in range(0, len(a), stride):
        ra, rb = a[i:i + stride], b[i:i + stride]
        if ra != rb:
            row = i // stride
            bad.extend((j // 3, row) for j, (p, q) in enumerate(zip(ra, rb)) if abs(p - q) > opts.max_level)
    if len(bad) <= opts.max_bad:
        return None
    return f'raster: {len(bad)} channel values differ by more than {opts.max_level}, at {opts.dpi} dpi, first at {bad[0]}'


def geometry_entry(e, opts):
    meta, _ = run.prepare_entry(e)
    res = {'id': e['id'], 'units': len(e['units']), 'pass': 0, 'fail': [], 'fallback': 0}
    if run.effective_class(meta, e) != 'normal':
        res['oracle'] = 'oracle-fails'
        return res
    w = run.new_workdir()
    out = os.path.join(w, 'o')
    try:
        env = run.candidate_env(e, out)
        env.pop('RSTEX_NO_COMMUTE', None)
        r = run.run_proc(run.tex_argv(e, opts.engine, opts.engine_arg, out), e['cwd'], env, e['timeout'] * 10, os.path.join(w, 'stdout'))
        job = e['jobname']
        final = os.path.join(meta['dir'], 'final')
        la = run.open_bytes(os.path.join(final, job + '.log'))
        lb = run.open_bytes(os.path.join(out, job + '.log'))
        res['fallback'] = lb.count(b'[[commute fallback]]')
        res['native'] = lb.count(b'[[commute native]]')
        res['failure'] = lb.count(b'[[commute failure]]')
        fates = unit_fates(lb)
        if private_id(e['id']):
            # What follows the fixed text can be a name from the document under --commute-report=names.  PGF's own are no secret.
            fates = {u: why if why is None or re.search(r': (pgf|tikz)', why) or ': ' not in why else why.split(': ')[0] for u, why in fates.items()}
        res['fates'] = fates
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
            if not dims_close(da.get(u), db.get(u), opts.dims_slack):
                why = f'dims: real PGF {da.get(u)}, candidate {db.get(u)}'
            elif x is None or y is None or x[:2] != y[:2]:
                why = f'page size: real PGF {x[:2] if x else None}, candidate {y[:2] if y else None}'
            elif (ta[k] if k < len(ta) else b'') != (tb[k] if k < len(tb) else b''):
                why = 'text on the page differs'
            else:
                why = raster_compare(x, y, opts)
            if why:
                res['fail'].append({'unit': u, 'why': why})
            else:
                res['pass'] += 1
        return res
    finally:
        shutil.rmtree(w, ignore_errors=True)


def private_id(entry_id):
    return entry_id.startswith(('user/', 'local/', 'cd-local/'))


def source_of(entry_id):
    """A coarse place the diagram came from: the id's first parts."""
    parts = entry_id.split('/')
    return ('cd-local' if parts[0] == 'cd-local' else parts[1] if len(parts) > 1 else parts[0])


def geometry(args):
    es = [e for e in entries() if not args.filter or re.search(args.filter, e['id'])]
    if args.sample:
        step = max(1, len(es) // args.sample)
        es = es[::step]
    args.engine = os.path.abspath(args.engine) if '/' in args.engine else args.engine
    run.install_limits()
    with cf.ThreadPoolExecutor(args.jobs) as ex:
        results = list(ex.map(lambda e: geometry_entry(e, args), es))
    units = sum(r['units'] for r in results)
    passed = sum(r['pass'] for r in results)
    fails = sum(len(r['fail']) for r in results)
    orac = sum(r['units'] for r in results if r.get('oracle'))
    fallback = sum(r['fallback'] for r in results)
    native = sum(r.get('native', 0) for r in results)
    print(f'PASS native diagrams: {passed} / {units - orac} that real PGF compiled; {orac} in documents the oracle fails on; {fallback} declined to real PGF by the engine\'s own count, {native} took the native path' + (f'; dimensions to within {args.dims_slack} pt only, so this is not the gate' if args.dims_slack else ''))
    if fallback and passed:
        print(f'  so at most {passed - fallback} passes tested the native path')
    # Which diagrams took the path, per source, and why the others did not.  What the engine says is a fixed text, with no word of the document in it.
    per = collections.defaultdict(lambda: [0, 0, 0, 0])
    reasons = collections.Counter()
    kinds = collections.Counter()
    native_fails = 0
    for r in results:
        row = per[source_of(r['id'])]
        fates = r.get('fates', {})
        row[0] += r['units']
        row[1] += sum(1 for u in r.get('fates', {}) if fates[u] is None)
        for u, why in fates.items():
            if why is not None:
                reasons[why] += 1
        row[2] += r['pass']
        row[3] += len(r['fail'])
        for f in r['fail']:
            kinds[f['why'].split(':')[0]] += 1
            if f['unit'] in fates and fates[f['unit']] is None:
                native_fails += 1
    print('per source: diagrams, took the native path, passed, failed')
    for src, row in sorted(per.items()):
        print(f'  {src:<16} {row[0]:>5} {row[1]:>5} ({100 * row[1] / max(1, row[0]):.1f}%) {row[2]:>5} {row[3]:>5}')
    print('top reasons for declining:')
    for k, v in reasons.most_common(args.top):
        print(f'  {v:>5} {k}')
    print(f'failures by kind ({native_fails} of {fails} on the native path):')
    for k, v in kinds.most_common():
        print(f'  {v:>5} {k}')
    shown = 0
    for r in results:
        for f in r['fail'][:3]:
            if shown < args.show:
                shown += 1
                print(f'  {r["id"]}#{f["unit"]}: {f["why"].split(":")[0] if private_id(r["id"]) else f["why"]}')
    os.makedirs(ROOT, exist_ok=True)
    run.write_json(os.path.join(ROOT, 'geometry.json'), results)
    return 1 if fails else 0


def log_body(e, data, workdir):
    """The normal lines up to the memory figures, and what the line "Output written" says of the pages."""
    lines = run.judged_log(e, data, [workdir])
    body, pages = [], None
    for no, line in lines:
        if line.startswith(b"Here is how much of TeX's memory you used:"):
            break
        body.append((no, line))
    m = re.search(rb'Output written on [^\n]*\((\d+) pages?', data.replace(b'\n', b''))
    if m:
        pages = int(m.group(1))
    return body, pages


def log_kind(line):
    """What sort of line it is, for a private document, where the line itself may not be printed."""
    m = re.match(r"(Overfull|Underfull|LaTeX Font Info|LaTeX Font Warning|LaTeX Warning|Package|Class|Output written|File|Document Class|\[\d|\(|\)|!|<end of log>|\\[a-z]+=|ABD|Here is|l\.\d|<)", line or '')
    return m.group(1) if m else 'other'


def document_entry(e, opts):
    res = {'id': e['id'], 'fails': [], 'native': 0, 'fallback': 0}
    meta, _ = run.prepare_entry(e)
    if run.effective_class(meta, e) != 'normal':
        res['skipped'] = 'the oracle fails on it'
        return res
    job = meta['jobname']
    final = os.path.join(meta['dir'], 'final')
    w = run.new_workdir(meta.get('workdir_path'))
    out = os.path.join(w, 'o')
    try:
        os.makedirs(out, exist_ok=True)
        run.restore_state(meta, out, stamp=True)
        env = run.candidate_env(e, out)
        env.pop('RSTEX_NO_COMMUTE', None)
        r = run.run_proc(run.tex_argv(e, opts.engine, list(opts.engine_arg) + ['--commute-report'], out), e['cwd'], env, e['timeout'] * 10, os.path.join(w, 'stdout'))
        pb = os.path.join(out, job + '.pdf')
        pa = os.path.join(final, job + '.pdf')
        if r['timed_out'] or r['signal'] or not os.path.isfile(pb):
            res['fails'].append(('run', f'rc {r["rc"]} signal {r["signal"]} timed_out {r["timed_out"]}'))
            return res
        la = run.open_bytes(os.path.join(final, job + '.log'))
        lb = run.open_bytes(os.path.join(out, job + '.log'))
        res['native'] = lb.count(b'[[commute native]]')
        res['fallback'] = lb.count(b'[[commute fallback]]')
        lb = strip_report(lb)
        (ba, na), (bb, nb) = log_body(e, la, meta.get('workdir_path')), log_body(e, lb, out)
        if na != nb:
            res['fails'].append(('pages', f'{na} and {nb}'))
        d = run.first_log_divergence(ba, bb)
        if d:
            res['fails'].append(('log', f'line {d["line_a"]}/{d["line_b"]}: oracle `{d["a"]}` candidate `{d["b"]}`'))
            res['log_kind'] = [log_kind(d['a']), log_kind(d['b'])]
        for name in sorted(os.listdir(final)):
            if name.endswith('.aux') and run.open_bytes(os.path.join(final, name)) != run.open_bytes(os.path.join(out, name)):
                res['fails'].append(('aux', name))
        ta, tb = page_texts(pa), page_texts(pb)
        if len(ta) != len(tb):
            res['fails'].append(('pages', f'text of {len(ta)} and {len(tb)} pages'))
        for k, (x, y) in enumerate(zip(ta, tb)):
            if x != y:
                res['fails'].append(('text', f'page {k + 1}'))
                break
        ra, rb = page_rasters(pa, opts.dpi), page_rasters(pb, opts.dpi)
        k = 0
        while True:
            x, y = next(ra, None), next(rb, None)
            k += 1
            if x is None or y is None:
                if x is not None or y is not None:
                    res['fails'].append(('pages', 'another number of rasters'))
                break
            if x[:2] != y[:2]:
                res['fails'].append(('raster', f'page {k}: another size'))
            else:
                why = raster_compare(x, y, opts)
                if why:
                    res['fails'].append(('raster', f'page {k}: {why}'))
        # What qpdf says of a figure the document includes, it says of the oracle's PDF too.
        q, oracle = subprocess.run(['qpdf', '--check', pb], capture_output=True), subprocess.run(['qpdf', '--check', pa], capture_output=True)
        if q.returncode != oracle.returncode:
            res['fails'].append(('qpdf', q.stdout.decode('latin-1')[-160:]))
        return res
    finally:
        shutil.rmtree(w, ignore_errors=True)


def documents(args):
    all_entries, _, _ = run.load_manifests(None)
    args.engine = os.path.abspath(args.engine) if '/' in args.engine else args.engine
    run.install_limits()
    es = []
    for e in all_entries:
        if e.get('excluded') or e['source'] not in ('user', 'local', 'arxiv'):
            continue
        if args.filter and not re.search(args.filter, e['id']):
            continue
        es.append(e)

    def uses_tikzcd(e):
        meta, _ = run.prepare_entry(e)
        return b'tikzlibrarycd.code.tex' in run.open_bytes(os.path.join(meta['dir'], 'final', meta['jobname'] + '.log'))
    with cf.ThreadPoolExecutor(args.jobs) as ex:
        chosen = [e for e, u in zip(es, ex.map(uses_tikzcd, es)) if u]
        results = list(ex.map(lambda e: document_entry(e, args), chosen))
    skipped = sum(1 for r in results if r.get('skipped'))
    good = sum(1 for r in results if not r['fails'] and not r.get('skipped'))
    kinds = collections.Counter(k for r in results for k, _ in r['fails'])
    native = sum(r['native'] for r in results)
    fallback = sum(r['fallback'] for r in results)
    print('per document, failures first:')
    for r in sorted(results, key=lambda r: not r['fails']):
        if r['fails']:
            shown = ', '.join(sorted({k for k, _ in r['fails']})) + (f' (log: {r["log_kind"]})' if r.get('log_kind') else '') if private_id(r['id']) else '; '.join(f'{k}: {v}' for k, v in r['fails'][:3])
            print(f'  {r["id"]}: native {r["native"]}, declined {r["fallback"]}: {shown[:300]}')
    for r in results:
        if not r['fails'] and not r.get('skipped') and (private_id(r['id']) or args.all):
            print(f'  {r["id"]}: native {r["native"]}, declined {r["fallback"]}: all equal')
    print('failures by kind: ' + (', '.join(f'{k} {v}' for k, v in kinds.most_common()) or 'none'))
    os.makedirs(ROOT, exist_ok=True)
    run.write_json(os.path.join(ROOT, 'documents.json'), [{'id': r['id'], 'native': r['native'], 'fallback': r['fallback'], 'kinds': sorted({k for k, _ in r['fails']})} for r in results])
    print(f'documents: {good} of {len(results) - skipped} equal, {skipped} the oracle fails on; {native} diagrams took the native path, {fallback} declined')
    return 1 if good != len(results) - skipped else 0


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
        args.dims_slack = 0
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
            p.add_argument('--sample', type=int, help='about this many documents, evenly spread')
            p.add_argument('--dims-slack', type=float, default=0, help='in pt.  Not the gate: for looking at text and pixels past small errors in the dimensions')
            p.add_argument('--top', type=int, default=30, help='how many reasons for declining to print')
            p.add_argument('--show', type=int, default=30, help='how many failures to print')
        p.add_argument('--dpi', type=int, default=400)
        p.add_argument('--max-level', type=int, default=48, help='a channel value may differ by this much, for antialiasing')
        p.add_argument('--max-bad', type=int, default=0, help='how many channel values may differ by more')
        p.add_argument('--filter')
        p.add_argument('-j', '--jobs', type=int, default=6)
        p.set_defaults(func=func)
    p = sub.add_parser('documents')
    p.add_argument('--engine', required=True)
    p.add_argument('--engine-arg', action='append', default=[])
    p.add_argument('--dpi', type=int, default=200)
    p.add_argument('--max-level', type=int, default=48)
    p.add_argument('--max-bad', type=int, default=0)
    p.add_argument('--filter')
    p.add_argument('--all', action='store_true', help='list public documents that are equal too')
    p.add_argument('-j', '--jobs', type=int, default=6)
    p.set_defaults(func=documents)
    args = ap.parse_args()
    os.makedirs(run.WORK_ROOT, exist_ok=True)
    os.makedirs(run.EMPTY_TEXMF, exist_ok=True)
    return args.func(args)


if __name__ == '__main__':
    sys.exit(main())
