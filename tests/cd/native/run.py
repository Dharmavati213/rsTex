#!/usr/bin/env -S python3 -I
r"""The native diagram path, one requirement at a time: every file in cases/ against the oracle.

    python3 -I tests/cd/native/run.py --engine WRAPPER [--filter REGEX] [-j N] [--dims-slack PT] [--keep]

WRAPPER is called as tests/corpus/run.py calls an engine, with -fmt=pdflatex.  A case says in its first lines what must become of its diagrams, in order, as --commute-report tells it:

    %% expect: native fallback native         or none, for no report at all
    %% reason 2: a substring of the reason for the second report
    %% oracle-errors-allowed                   the log's ! lines are compared, and the oracle is allowed some

Then, per case:

  fates     what --commute-report says is what the case expects
  lines     every line [[...]] that the document writes, in order, is the oracle's: dimensions, \meaning, registers.  --dims-slack lets numbers with pt in them differ a little.
  errors    the ! lines of the log are the oracle's
  aux       the .aux files are the oracle's, byte for byte
  pages     the same number, the same text on each, and pixels at 400 dpi within what compare_native.py allows
  qpdf      qpdf --check is clean
  off       with --no-commute and with RSTEX_NO_COMMUTE=1 nothing is reported, and the PDF is the same with either
  fallback  if no diagram took the path, log and PDF are the same bytes as with the path off, apart from the report's lines: a decline changes nothing
  warm      the same document restored from a checkpoint reports the same, and gives the same PDF as the run that wrote it
"""
import argparse
import concurrent.futures as cf
import importlib.util
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
CASES = os.path.join(HERE, 'cases')
spec = importlib.util.spec_from_file_location('compare_native', os.path.join(REPO, 'tests', 'corpus', 'cd', 'compare_native.py'))
cn = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cn)
ENV = {'SOURCE_DATE_EPOCH': '1700000000', 'FORCE_SOURCE_DATE': '1', 'TZ': 'UTC', 'LC_ALL': 'C', 'LANG': 'C', 'PATH': os.environ['PATH'], 'HOME': os.environ['HOME']}
LINE = re.compile(rb'\[\[(?!commute )[^\]\n]*(?:\n[^\]\n]*)?\]\]')
REPORT = re.compile(rb'^\[\[commute (native|fallback|failure)\]\] ?(.*)$', re.M)


args_timeout = 300


def tex(engine, name, out, extra=(), env=None, cache=None, oracle=False):
    os.makedirs(out, exist_ok=True)
    argv = [engine] + list(extra) + ['-interaction=batchmode', '-no-shell-escape', '-output-directory=' + out]
    if not oracle:
        argv.insert(1, '-fmt=pdflatex')
    argv.append(name + '.tex')
    e = dict(ENV, TEXINPUTS=CASES + ':', **(env or {}))
    if not oracle:
        e['XDG_CACHE_HOME'] = cache or os.path.join(out, 'cache')
        if cache is None:
            e['RSTEX_NO_SNAPSHOT'] = '1'
    try:
        return subprocess.run(argv, cwd=CASES, env=e, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=args_timeout)
    except subprocess.TimeoutExpired:
        return None


def read(path):
    try:
        with open(path, 'rb') as f:
            return f.read()
    except OSError:
        return b''


def unwrap(log):
    return re.sub(rb'\n(?=[^\n]{0,78}\]\])', b'', log)


def lines_of(log):
    return [m.group().replace(b'\n', b'') for m in LINE.finditer(unwrap(log))]


def errors_of(log):
    return [l for l in log.split(b'\n') if l.startswith(b'! ')]


def close(a, b, slack):
    if a == b:
        return True
    if not slack:
        return False
    x, y = re.split(rb'(-?[0-9.]+pt)', a), re.split(rb'(-?[0-9.]+pt)', b)
    if len(x) != len(y):
        return False
    for p, q in zip(x, y):
        if p == q:
            continue
        if not (p.endswith(b'pt') and q.endswith(b'pt')) or abs(float(p[:-2]) - float(q[:-2])) > slack:
            return False
    return True


def strip_report(log):
    return re.sub(rb'^\[\[commute [^\n]*\n', b'', log, flags=re.M)


def header(name):
    expect, reasons = None, {}
    with open(os.path.join(CASES, name + '.tex'), errors='replace') as f:
        for line in f:
            if not line.startswith('%%'):
                break
            m = re.match(r'%% expect: (.*)', line)
            if m:
                expect = m.group(1).split()
            m = re.match(r'%% reason (\d+): (.*)', line)
            if m:
                reasons[int(m.group(1))] = m.group(2).strip()
    return expect, reasons


def one(name, args):
    fails = []
    root = os.path.join(args.work, name)
    shutil.rmtree(root, ignore_errors=True)
    o, n, off, off2, warm = [os.path.join(root, d) for d in ('oracle', 'on', 'off', 'off2', 'warm')]
    tex('pdflatex', name, o, oracle=True)

    # Each to the same path, for the same /ID and the same log: the last one stays there.
    def moved(extra, env, to):
        same = os.path.join(root, 'run')
        tex(args.engine, name, same, extra=extra, env=env)
        os.rename(same, to)
    moved(['--no-commute', '--commute-report'], None, off)
    moved(['--commute-report'], {'RSTEX_NO_COMMUTE': '1'}, off2)
    moved(['--commute-report'], None, n)
    la, lb = read(os.path.join(o, name + '.log')), read(os.path.join(n, name + '.log'))
    if not lb:
        return name, ['no log']
    # fates
    expect, reasons = header(name)
    fates = [m.group(1).decode() for m in REPORT.finditer(lb)]
    said = [m.group(2).decode('utf-8', 'replace') for m in REPORT.finditer(lb)]
    want = [] if expect == ['none'] else expect
    if want is None:
        fails.append('no expect line')
    elif fates != want:
        fails.append(f'fates: expected {want}, got {fates}: {said}')
    for k, r in reasons.items():
        if k > len(said) or r not in said[k - 1]:
            fails.append(f'reason {k}: expected {r!r}, got {said[k - 1] if k <= len(said) else None!r}')
    if 'failure' in fates:
        fails.append('a failure was reported')
    # lines
    x, y = lines_of(la), lines_of(lb)
    if len(x) != len(y):
        fails.append(f'lines: {len(x)} from the oracle, {len(y)} here')
    for a, b in zip(x, y):
        if not close(a, b, args.dims_slack):
            fails.append(f'line: oracle {a.decode("latin-1")[:160]} here {b.decode("latin-1")[:160]}')
    # errors
    if errors_of(la) != errors_of(lb):
        fails.append(f'errors: oracle {errors_of(la)[:3]} here {errors_of(lb)[:3]}')
    # aux
    if read(os.path.join(o, name + '.aux')) != read(os.path.join(n, name + '.aux')):
        fails.append('aux differs')
    # pages
    pa, pb = os.path.join(o, name + '.pdf'), os.path.join(n, name + '.pdf')
    if os.path.exists(pa) != os.path.exists(pb):
        fails.append('a PDF on one side only')
    elif os.path.exists(pa):
        ta, tb = cn.page_texts(pa), cn.page_texts(pb)
        if ta != tb:
            fails.append('text differs')
        ra, rb = cn.page_rasters(pa, 400), cn.page_rasters(pb, 400)
        opts = argparse.Namespace(max_level=48, max_bad=0, dpi=400, word_slack=0.03)
        page = 0
        while True:
            p, q = next(ra, None), next(rb, None)
            page += 1
            if p is None or q is None:
                if p is not None or q is not None:
                    fails.append('page counts differ')
                break
            if p[:2] != q[:2]:
                fails.append(f'page {page}: another size')
            else:
                why = cn.raster_compare(pa, pb, page, p, q, opts)
                if why:
                    fails.append(f'page {page}: {why}')
        r = subprocess.run(['qpdf', '--check', pb], capture_output=True)
        if r.returncode != 0:
            fails.append('qpdf: ' + r.stdout.decode('latin-1')[-200:])
    # off
    pc, pd = os.path.join(off, name + '.pdf'), os.path.join(off2, name + '.pdf')
    for d in (off, off2):
        if REPORT.search(read(os.path.join(d, name + '.log'))):
            fails.append('reports with the path off')
    if read(pc) != read(pd):
        fails.append('--no-commute and RSTEX_NO_COMMUTE give different PDFs')
    # fallback
    if 'native' not in fates:
        if read(pb) != read(pc):
            fails.append('the PDF is not the same as with the path off')
        lon, loff = strip_report(lb), strip_report(read(os.path.join(off, name + '.log')))
        if lon != loff:
            for u, v in zip(lon.split(b'\n'), loff.split(b'\n')):
                if u != v:
                    fails.append(f'the log is not the same as with the path off: {u.decode("latin-1")[:80]} / {v.decode("latin-1")[:80]}')
                    break
            else:
                fails.append('the log is not the same as with the path off: its length')
    # warm
    cache = os.path.join(root, 'cache')
    tex(args.engine, name, warm, extra=['--commute-report'], cache=cache)
    first = read(os.path.join(warm, name + '.pdf'))
    tex(args.engine, name, warm, extra=['--commute-report'], cache=cache)
    lw = read(os.path.join(warm, name + '.log'))
    if [m.group(1) for m in REPORT.finditer(lw)] != [m.group(1) for m in REPORT.finditer(lb)]:
        fails.append('a warm run reports otherwise')
    if read(os.path.join(warm, name + '.pdf')) != first:
        fails.append('a warm run gives another PDF')
    if not args.keep:
        shutil.rmtree(root, ignore_errors=True)
    return name, fails


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--filter')
    ap.add_argument('-j', '--jobs', type=int, default=4)
    ap.add_argument('--dims-slack', type=float, default=0)
    ap.add_argument('--keep', action='store_true')
    ap.add_argument('--work', default=os.path.join(REPO, 'target', 'native', 'cases'))
    args = ap.parse_args()
    args.engine = os.path.abspath(args.engine)
    names = sorted(n[:-4] for n in os.listdir(CASES) if n.endswith('.tex') and not n.endswith('-part.tex') and (not args.filter or re.search(args.filter, n)))
    os.makedirs(args.work, exist_ok=True)
    bad = 0
    with cf.ThreadPoolExecutor(args.jobs) as ex:
        for name, fails in ex.map(lambda n: one(n, args), names):
            if fails:
                bad += 1
                print(f'FAIL {name}')
                for f in fails:
                    print(f'  {f}')
            else:
                print(f'ok   {name}')
    print(f'total {len(names) - bad} of {len(names)} passed' + (f', numbers to within {args.dims_slack} pt only' if args.dims_slack else ''))
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
