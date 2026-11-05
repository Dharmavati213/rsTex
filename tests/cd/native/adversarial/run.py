#!/usr/bin/env -S python3 -I
r"""Documents written to break the native diagram path, DESIGN.md section 10.  The rule of the game: a diagram that takes the path comes out as real PGF draws it, and a declined one is always fine.

    python3 -I tests/cd/native/adversarial/run.py --engine WRAPPER [--filter REGEX] [-j N] [--dpi 400] [--work DIR] [--oracle-cache DIR] [--show N] [--fates]

WRAPPER is called as tests/corpus/run.py calls an engine, with -fmt=pdflatex.  A file here holds many documents, each a section of it:

    <the head: everything up to the first %%%% line, \documentclass among it>
    %%%% case NAME
    %%%% pre            what follows goes before \begin{document}
    %%%% body           what follows goes after it
    %%%% whole          what follows is the whole document, without the head
    %%%% known WHY      it is known to differ from the oracle, and this says why: DESIGN.md section 10.  Reported as such, and a failure if it comes out equal
    %%%% expect FATE    what must become of its first diagram: native, restart for a run that gave up the path and started again, or fallback and words of the reason.  A second word native says the next one takes the path.

A document with no report of a diagram on the path, which is also what a run that started again leaves, is judged against the same engine with the path off: log and PDF the same bytes.  One with such a report is judged against the oracle:

  log      the normal form of tests/corpus/run.py, with the page count on the line "Output written" and the report's lines taken out; if the oracle reports an error, its ! lines only
  files    .aux, .toc, .lof, .lot, .out, byte for byte
  pages    the same number, the same text on each, and pixels at --dpi within what compare_native.py allows
  streams  each page's content stream, byte for byte but for a font's number
  qpdf     says of it what it says of the oracle's

A report of a failure, a crash or a timeout fails at once.  The oracle's results are kept by the hash of the document, so a second run costs only the engine.
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
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(HERE))))
sys.dont_write_bytecode = True
_spec = importlib.util.spec_from_file_location('compare_native', os.path.join(REPO, 'tests', 'corpus', 'cd', 'compare_native.py'))
cn = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(cn)
run = cn.run
ENV = {'SOURCE_DATE_EPOCH': '1700000000', 'FORCE_SOURCE_DATE': '1', 'TZ': 'UTC', 'LC_ALL': 'C', 'LANG': 'C', 'PATH': os.environ['PATH'], 'HOME': os.environ['HOME']}
REPORT = re.compile(rb'^\[\[commute (native|fallback|failure|restart)\]\] ?(.*)$', re.M)
SIDE = ('.aux', '.toc', '.lof', '.lot', '.out')
TIMEOUT = 120


def documents(path):
    """The documents of a file, as (name, text)."""
    with open(path, encoding='latin-1') as f:
        lines = f.read().split('\n')
    head, cases, cur = [], [], None
    for line in lines:
        if line.startswith('%%%% case '):
            cur = {'name': line[10:].strip(), 'pre': [], 'body': [], 'whole': False, 'known': None, 'expect': None, 'to': 'body'}
            cases.append(cur)
        elif cur is None:
            head.append(line)
        elif line.startswith('%%%% pre'):
            cur['to'] = 'pre'
        elif line.startswith('%%%% body'):
            cur['to'] = 'body'
        elif line.startswith('%%%% whole'):
            cur['whole'] = True
        elif line.startswith('%%%% known'):
            cur['known'] = line[10:].strip()
        elif line.startswith('%%%% expect'):
            cur['expect'] = line[11:].strip()
        else:
            cur[cur['to']].append(line)
    for c in cases:
        if c['whole']:
            yield c['name'], '\n'.join(c['body']) + '\n', c
        else:
            yield c['name'], '\n'.join(head + c['pre'] + ['\\begin{document}'] + c['body'] + ['\\par W', '\\end{document}', '']), c


def tex(argv, cwd, env):
    try:
        r = subprocess.run(argv + ['-interaction=batchmode', '-no-shell-escape', 'x.tex'], cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=TIMEOUT)
        return r.returncode
    except subprocess.TimeoutExpired:
        return 'timeout'


def read(path):
    try:
        with open(path, 'rb') as f:
            return f.read()
    except OSError:
        return b''


def side_files(d):
    """Two runs, as a document with a table of contents needs: the first one's files are what the second reads."""
    return {e: read(os.path.join(d, 'x' + e)) for e in SIDE}


def has_toc(text):
    return '\\tableofcontents' in text or '\\listoffigures' in text or '\\ref{' in text or '\\pageref{' in text


def prepared(work, text, name):
    d = os.path.join(work, name)
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, 'x.tex'), 'w', encoding='latin-1') as f:
        f.write(text)
    return d


def oracle(cache, text):
    """The oracle's directory for this document.  One that reads a file from here has this directory's name in its log, so that is part of its key."""
    key = hashlib.sha1((text + HERE if 'adv-part' in text else text).encode('latin-1')).hexdigest()
    d = os.path.join(cache, key)
    if os.path.isfile(os.path.join(d, 'done')):
        return d
    t = d + '.tmp' + str(os.getpid()) + str(id(text))
    shutil.rmtree(t, ignore_errors=True)
    os.makedirs(t)
    with open(os.path.join(t, 'x.tex'), 'w', encoding='latin-1') as f:
        f.write(text)
    env = dict(ENV, TEXINPUTS=HERE + ':')
    for _ in range(2 if has_toc(text) else 1):
        # A run that did not get as far as its last line was killed by something else on this machine.
        for _ in range(3):
            tex(['pdflatex'], t, env)
            log = read(os.path.join(t, 'x.log'))
            if b'Output written on' in log.replace(b'\n', b'') or b'No pages of output' in log or b'Fatal error occurred' in log:
                break
    with open(os.path.join(t, 'done'), 'w') as f:
        f.write('')
    shutil.rmtree(d, ignore_errors=True)
    os.rename(t, d)
    return d


def engine_run(args, d, text, extra, env_extra=None):
    with open(os.path.join(d, 'x.tex'), 'w', encoding='latin-1') as f:
        f.write(text)
    env = dict(ENV, TEXINPUTS=HERE + ':', RSTEX_NO_SNAPSHOT='1', XDG_CACHE_HOME=os.path.join(d, 'cache'), **(env_extra or {}))
    for n in os.listdir(d):
        if n != 'x.tex' and os.path.isfile(os.path.join(d, n)):
            os.remove(os.path.join(d, n))
    rc = None
    for _ in range(2 if has_toc(text) else 1):
        rc = tex([args.engine, '-fmt=pdflatex'] + extra, d, env)
    return rc


def log_lines(data, d):
    return [line for _, line in run.normalize_log(cn.strip_report(data), [d])]


def judge_native(args, o, n):
    fails = []
    la, lb = read(os.path.join(o, 'x.log')), read(os.path.join(n, 'x.log'))
    a, b = log_lines(la, o), log_lines(lb, n)
    # An error's context shows how far TeX has read, which the path has read further: where the oracle reports an error it is the messages that count.
    if any(x.startswith(b'! ') for x in a):
        a, b = [x for x in a if x.startswith(b'! ')], [x for x in b if x.startswith(b'! ')]
    if a != b:
        k = next((i for i, (p, q) in enumerate(zip(a, b)) if p != q), min(len(a), len(b)))
        fails.append(f'log line {k}: oracle `{a[k].decode("latin-1")[:150] if k < len(a) else "<end>"}` here `{b[k].decode("latin-1")[:150] if k < len(b) else "<end>"}`')
    sa, sb = side_files(o), side_files(n)
    for e in SIDE:
        if sa[e] != sb[e]:
            fails.append(f'{e} differs')
    pa, pb = os.path.join(o, 'x.pdf'), os.path.join(n, 'x.pdf')
    if os.path.exists(pa) != os.path.exists(pb):
        fails.append('a PDF on one side only')
        return fails
    if not os.path.exists(pa):
        return fails
    ta, tb = cn.page_texts(pa), cn.page_texts(pb)
    if ta != tb:
        fails.append('text differs' if len(ta) == len(tb) else f'pages of text: {len(ta)} and {len(tb)}')
    opts = argparse.Namespace(max_level=48, max_bad=0, dpi=args.dpi)
    ra, rb = cn.page_rasters(pa, args.dpi), cn.page_rasters(pb, args.dpi)
    page = 0
    while True:
        x, y = next(ra, None), next(rb, None)
        page += 1
        if x is None or y is None:
            if x is not None or y is not None:
                fails.append('page counts differ')
            break
        if x[:2] != y[:2]:
            fails.append(f'page {page}: another size')
        else:
            why = cn.raster_compare(x, y, opts)
            if why:
                fails.append(f'page {page}: {why}')
    ca, cb = cn.page_streams(pa), cn.page_streams(pb)
    if ca != cb:
        k = next((i for i, (p, q) in enumerate(zip(ca, cb)) if p != q), min(len(ca), len(cb)))
        fails.append(f'content stream of page {k + 1} differs')
    qa, qb = subprocess.run(['qpdf', '--check', pa], capture_output=True), subprocess.run(['qpdf', '--check', pb], capture_output=True)
    if qa.returncode != qb.returncode:
        fails.append('qpdf: ' + qb.stdout.decode('latin-1')[-200:])
    return fails


def one(item, args):
    file, name, text, c = item
    fates, fails = judged(file, name, text, args)
    known = bool(fails)
    if c['known']:
        fails = [f'known to differ and does not: {c["known"]}'] if not fails else []
    elif c['expect']:
        fails = fails + expected(c['expect'], fates)
    # Each document's directory has a cache of its own, of map tables above all: kept, the 8500 of them come to more than 100 GB.  Only one that failed is of any use.
    if not fails:
        shutil.rmtree(os.path.join(args.work, file, name), ignore_errors=True)
    return file, name, fates, fails, known and bool(c['known'])


def expected(want, fates):
    if want == 'fallback native':
        ok = [f for f, _ in fates] == ['fallback', 'native']
    elif want == 'native':
        ok = bool(fates) and all(f == 'native' for f, _ in fates)
    elif want == 'restart':
        ok = [f for f, _ in fates] == ['restart']
    else:
        ok = bool(fates) and fates[0][0] == 'fallback' and want[len('fallback'):].strip() in fates[0][1]
    return [] if ok else [f'expected {want}: {fates[:2]}']


def judged(file, name, text, args):
    d = prepared(args.work, text, os.path.join(file, name, 'n'))
    rc = engine_run(args, d, text, ['--commute-report'])
    if rc == 'timeout' or (isinstance(rc, int) and rc < 0):
        return [], [f'the engine: {rc}']
    log = read(os.path.join(d, 'x.log'))
    fates = [m.group(1).decode() for m in REPORT.finditer(log)]
    reasons = [m.group(2).decode('utf-8', 'replace').removesuffix('%').strip() for m in REPORT.finditer(log)]
    if 'failure' in fates:
        return list(zip(fates, reasons)), ['a failure was reported: ' + next(r for f, r in zip(fates, reasons) if f == 'failure')]
    if 'native' in fates:
        fails = judge_native(args, oracle(args.oracle_cache, text), d)
    else:
        f = prepared(args.work, text, os.path.join(file, name, 'n2'))
        # Each to the same path, for the same /ID.
        shutil.rmtree(d, ignore_errors=True)
        os.rename(f, d)
        rc = engine_run(args, d, text, [])
        plain_log, plain_pdf = read(os.path.join(d, 'x.log')), read(os.path.join(d, 'x.pdf'))
        rc2 = engine_run(args, d, text, ['--no-commute'])
        fails = []
        if rc != rc2:
            fails.append(f'exit codes {rc} and {rc2} with the path on and off')
        if plain_log != read(os.path.join(d, 'x.log')):
            fails.append('declined, and the log is not the one with the path off')
        if plain_pdf != read(os.path.join(d, 'x.pdf')):
            fails.append('declined, and the PDF is not the one with the path off')
    return list(zip(fates, reasons)), fails


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--filter')
    ap.add_argument('-j', '--jobs', type=int, default=4)
    ap.add_argument('--dpi', type=int, default=400)
    ap.add_argument('--work', default=os.path.join(REPO, 'target', 'native', 'adversarial'))
    ap.add_argument('--oracle-cache', default=os.path.join(REPO, 'target', 'native', 'adversarial-oracle'), help='where the oracle\'s results stay, by the hash of the document')
    ap.add_argument('--show', type=int, default=200)
    ap.add_argument('--fates', action='store_true', help='print the fate of each document and why it declined')
    ap.add_argument('--json', help='write the results here')
    args = ap.parse_args()
    args.engine = os.path.abspath(args.engine)
    os.makedirs(args.work, exist_ok=True)
    items = []
    for fn in sorted(os.listdir(HERE)):
        if fn.endswith('.tex'):
            base = fn[:-4]
            for name, text, c in documents(os.path.join(HERE, fn)):
                if not args.filter or re.search(args.filter, base + '/' + name):
                    items.append((base, name, text, c))
    if len({(i[0], i[1]) for i in items}) != len(items):
        print('two documents have one name: ' + ', '.join(f'{f}/{n}' for (f, n), c in collections.Counter((i[0], i[1]) for i in items).items() if c > 1))
        return 2
    bad, native, declined, docs_native, known_limits = 0, 0, 0, 0, 0
    reasons = collections.Counter()
    results = []
    with cf.ThreadPoolExecutor(args.jobs) as ex:
        for file, name, fates, fails, limit in ex.map(lambda i: one(i, args), items):
            known_limits += limit
            n = sum(f == 'native' for f, _ in fates)
            native += n
            docs_native += n > 0
            declined += sum(f == 'fallback' for f, _ in fates)
            for f, r in fates:
                if f == 'fallback':
                    reasons[r] += 1
            results.append({'file': file, 'name': name, 'fates': fates, 'fails': fails})
            if fails:
                bad += 1
                if bad <= args.show:
                    print(f'FAIL {file}/{name} ({" ".join(f for f, _ in fates)})')
                    for w in fails:
                        print(f'  {w}')
            elif args.fates:
                print(f'ok   {file}/{name}: ' + '; '.join(f if f == 'native' else f'declined: {r}' for f, r in fates))
    if args.json:
        with open(args.json, 'w') as f:
            json.dump(results, f, indent=0)
    print(f'adversarial: {len(items) - bad} of {len(items)} documents passed, {known_limits} of those by a known limit; {native} diagrams took the path, in {docs_native} documents, {declined} declined')
    if args.fates:
        for r, c in reasons.most_common():
            print(f'  {c:>5} {r}')
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
