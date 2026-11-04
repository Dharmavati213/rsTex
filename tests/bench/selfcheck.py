#!/usr/bin/env -S python3 -I
r"""selfcheck.py: cases that tools/prof/benchset.py and tests/bench/report.py must get right, or their numbers mean nothing.

    selfcheck.py --engine PATH [--only NAME,...] [-j 4] [--common PATH]

    a-a                one binary under two names gives 1.000 on instructions
    slowed             a wrapper that adds a known count of instructions shows that amount, and the report's interval covers it
    wrong-byte         one wrong byte in the page content, a raw byte of the PDF, the log or the .aux: each engine's rows count as wrong, and are out of the speed figure and charged in it
    killed             a killed run, and a line cut short, resume with no row twice and none missing, and with nothing measured again
    refused            a row from another binary or on another input hash is refused, by a run and by the report
    selection          select.py gives set.json byte for byte, and stops on a census that holds a column of rstex's
    fast-check         the cheap form of the output check agrees with compare_pdf.py where it says the same, and sends a mutated PDF on to it
    clock-guard        a timed row over the load limit has no clock figures, and its instructions stand
    peak               a run's peak is its own and not the size of what spawned it
    edit               an edit session changes the output where it should and no original file
    mirror             a save can never go through a link to a document's tree
    picture            the check of a diagram alone sees the same picture, another label, and an arrow half a point away
    report             the arithmetic of the tables on rows made up here: charging, N / M, intervals, weights, what is apart, what is refused
    common             what benchset.py keeps of tests/until_stable/common.py is what that file says, where that file is
    no-text            a row holds ids, numbers and words of this program's own, and a private document's holds nothing a trace said: on --rows FILE, or on a small run

Everything is written under target/.  Only public documents are compiled, the cheapest of `quick` by a rule, so nothing here names a document by hand or reads one of the hold-out.  The last line says PASS, or how many failed.
"""
import os
import sys

sys.path[:] = [p for p in sys.path if os.path.abspath(p or os.curdir) != os.path.dirname(os.path.abspath(__file__))]

import argparse  # noqa: E402
import json  # noqa: E402
import math  # noqa: E402
import re  # noqa: E402
import shutil  # noqa: E402
import signal  # noqa: E402
import stat  # noqa: E402
import subprocess  # noqa: E402
import time  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(REPO, 'tools', 'prof'))
import proflib  # noqa: E402

report = proflib.load_script('bench_report', os.path.join(HERE, 'report.py'))
bs = proflib.load_script('benchset_tool', os.path.join(REPO, 'tools', 'prof', 'benchset.py'))
BENCHSET = os.path.join(REPO, 'tools', 'prof', 'benchset.py')
SET = os.path.join(HERE, 'set.json')
SMALL_N = 12
PAD_LOOPS = 40000

MUTATOR = r'''#!/usr/bin/env -S python3 -I
import glob, os, re, subprocess, sys, zlib
REAL, MODE = %r, %r
argv = sys.argv[1:]
rc = subprocess.call([REAL] + argv)
out = next((a.split('=', 1)[1] for a in argv if a.startswith('-output-directory=')), '.')
if not any(a in ('-ini',) or 'rstexbenchprime' in a for a in argv):
    def only(pattern):
        found = sorted(glob.glob(os.path.join(out, pattern)))
        return found[0] if found else None
    if MODE == 'content' and only('*.pdf'):
        path = only('*.pdf')
        data = open(path, 'rb').read()
        for m in re.finditer(rb'stream\r?\n', data):
            d = zlib.decompressobj()
            try:
                plain = d.decompress(data[m.end():])
            except zlib.error:
                continue
            found = re.search(rb'\d+ Td', plain)
            if found:
                k = found.start()
                wrong = plain[:k] + (b'7' if plain[k:k + 1] != b'7' else b'8') + plain[k + 1:]
                end = len(data) - len(d.unused_data)
                open(path, 'wb').write(data[:m.end()] + zlib.compress(wrong, 9) + data[end:])
                break
    elif MODE == 'raw' and only('*.pdf'):
        path = only('*.pdf')
        data = bytearray(open(path, 'rb').read())
        data[len(data) // 2] ^= 1
        open(path, 'wb').write(data)
    elif MODE == 'log' and only('*.log'):
        path = only('*.log')
        data = open(path, 'rb').read()
        k = data.rfind(b'Output written on')
        k = k if k >= 0 else len(data) // 2
        open(path, 'wb').write(data[:k] + data[k:k + 1].swapcase() + data[k + 1:])
    elif MODE == 'aux' and only('*.aux'):
        path = only('*.aux')
        data = open(path, 'rb').read()
        open(path, 'wb').write(data + b'%%')
sys.exit(rc)
'''


class Check:
    def __init__(self):
        self.failed, self.passed, self.skipped = [], 0, 0

    def __call__(self, name, ok, detail=''):
        print(f'  {"ok  " if ok else "FAIL"} {name}{": " + str(detail) if detail != "" else ""}', flush=True)
        if ok:
            self.passed += 1
        else:
            self.failed.append(name)

    def skip(self, name, why):
        print(f'  skip {name}: {why}', flush=True)
        self.skipped += 1


def small_ids():
    """The cheapest real public documents of quick: enough for an interval, chosen by a rule."""
    with open(SET) as f:
        entries = json.load(f)['entries']
    real = [e for e in entries if 'quick' in e['tiers'] and e['kind'] != 'testfile' and not e.get('synthetic')]
    return [e['id'] for e in sorted(real, key=lambda e: (e['minstr'], e['id']))[:SMALL_N]]


def ids_pattern(ids):
    return '^(' + '|'.join(re.escape(i) for i in ids) + ')$'


def bench(args, out, *more, check=True):
    argv = [sys.executable, '-I', BENCHSET, 'run', '--tier', 'quick', '--out', out, '--no-private', '--quiet'] + list(more)
    p = subprocess.run(argv, capture_output=True, text=True, check=False)
    if check and p.returncode != 0:
        raise RuntimeError(f'benchset failed, exit {p.returncode}: {p.stderr[-2000:]}')
    return p


def rows_of(path, kind='m'):
    with open(path) as f:
        return [r for r in (json.loads(line) for line in f if line.strip()) if r.get('kind') == kind]


def write_script(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, 'w') as f:
        f.write(text)
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)
    return path


def tool_rstex(binary):
    """The binary under the name it has to have, for a wrapper to run."""
    return bs.Rstex('real', binary).path


def padding_wrapper(root, real):
    return write_script(os.path.join(root, 'slow', 'rstex'), f'#!/bin/sh\ni=0\nwhile [ $i -lt {PAD_LOOPS} ]; do i=$((i+1)); done\nexec {real} "$@"\n')


def count_version(runner, path):
    return runner.run([path, '--version'], '/', {k: os.environ[k] for k in ('PATH', 'HOME') if k in os.environ})['counters']['instructions'][0]


# ------------------------------------------------------------------------------------------------ the cases
def case_aa_slowed(args, root, chk):
    real = tool_rstex(args.engine)
    wrapper = padding_wrapper(root, real)
    runner = bs.Runner()
    added = count_version(runner, wrapper) - count_version(runner, real)
    # Twice: a shell's loop has no reason to vary by more than a little.
    added2 = count_version(runner, wrapper) - count_version(runner, real)
    runner.p.stdin.close()
    runner.p.wait()
    ids = small_ids()
    out = os.path.join(root, 'aa.jsonl')
    bench(args, out, '--engine', 'a=' + args.engine, '--engine', 'b=' + args.engine, '--engine', 'slow=' + wrapper, '--baselines', 'arch', '--method', 'counted', '--scenarios', 'last-cold', '--ids', ids_pattern(ids), '-j', str(args.jobs))
    rows = {(r['id'], r['side']): r for r in rows_of(out)}
    aa = [rows[(i, 'b')]['instructions'] / rows[(i, 'a')]['instructions'] for i in ids]
    g = report.geomean(aa)
    chk('a-a: one binary under two names is 1.000 on instructions', f'{g:.3f}' == '1.000' and max(abs(x - 1) for x in aa) < 0.002, f'geomean {g:.5f}, worst {max(abs(x - 1) for x in aa):.5f}, over {len(aa)} documents')
    # A measured run finds in the cache what the next one finds, also where a document's search path holds the output directory.
    with open(SET) as f:
        with_out = next(e['id'] for e in sorted(json.load(f)['entries'], key=lambda e: e['minstr']) if 'quick' in e['tiers'] and e['id'].startswith('knuth/woven/'))
    twice = os.path.join(root, 'twice.jsonl')
    bench(args, twice, '--engine', 'a=' + args.engine, '--baselines', '', '--method', 'timed', '--max-load', '1000', '--scenarios', 'last-cold', '--ids', ids_pattern([with_out]))
    twice_rows = {r['variant']: r['instructions'] for r in rows_of(twice) if r['round'] == 0}
    chk('a-a: a cold run pays for no index its next run does not', abs(twice_rows['a1'] / twice_rows['a2'] - 1) < 0.002, f'{with_out}: {twice_rows["a1"] / 1e6:.1f} M, then {twice_rows["a2"] / 1e6:.1f} M')
    res = report.analyze([out], SET, None, 2000)
    fig = res['tiers']['quick']['public']['figures']['last-cold']['by']
    a, b = fig['a']['arch']['real'], fig['b']['arch']['real']
    chk('a-a: the report gives the two the same figure', abs(a['charged'] / b['charged'] - 1) < 0.002, f'{a["charged"]:.4f} and {b["charged"]:.4f}')
    chk('a-a: all rows identical to the oracle', a['identical'] == a['n'] == len(ids), f'{a["identical"]} / {a["n"]}')
    expected = report.geomean([(rows[(i, 'a')]['instructions'] + added) / rows[(i, 'arch')]['instructions'] for i in ids])
    s = fig['slow']['arch']['real']
    chk('slowed: the known amount was measured', added > 1e6 and abs(added - added2) / added < 0.01, f'{added / 1e6:.1f} M and {added2 / 1e6:.1f} M per run')
    chk('slowed: the report shows that amount', abs(s['charged'] / expected - 1) < 0.005, f'{s["charged"]:.4f}, expected {expected:.4f}')
    lo, hi = s['interval']
    chk('slowed: its interval covers it', lo <= expected <= hi, f'{lo:.4f} to {hi:.4f}')
    gain = report.geomean([(rows[(i, 'slow')]['instructions']) / rows[(i, 'a')]['instructions'] for i in ids])
    exact = report.geomean([(rows[(i, 'a')]['instructions'] + added) / rows[(i, 'a')]['instructions'] for i in ids])
    chk('slowed: slowed over plain, per document', abs(gain / exact - 1) < 0.005, f'{gain:.4f}, expected {exact:.4f}')
    return out


def case_wrong_byte(args, root, chk):
    real = tool_rstex(args.engine)
    ids = small_ids()[:4]
    engines = []
    for mode in ('content', 'raw', 'log', 'aux'):
        p = write_script(os.path.join(root, 'wrong-' + mode, 'rstex'), MUTATOR % (real, mode))
        engines += ['--engine', f'w{mode}={p}']
    out = os.path.join(root, 'wrong.jsonl')
    bench(args, out, '--engine', 'a=' + args.engine, *engines, '--baselines', 'arch', '--method', 'counted', '--scenarios', 'last-cold,first,warm-word', '--ids', ids_pattern(ids), '-j', str(args.jobs))
    rows = rows_of(out)
    by = {}
    for r in rows:
        by.setdefault((r['side'], r['scenario']), []).append(r)
    word = [r for r in by.get(('a', 'warm-word'), []) if r.get('applied')]
    chk('warm-word: a word in the body is right on the right engine, and the trace says what it did', word and all(r['check'] == 'identical' and 'restored' in r for r in word), f'{len(word)} rows: {sorted({r["check"] for r in word})}')
    wrong = [r for r in by.get(('wcontent', 'warm-word'), []) if r.get('applied')]
    chk('warm-word: a wrong byte shows there too', wrong and all(r['check'] == 'differs-content' for r in wrong), f'{len(wrong)} rows: {sorted({r["check"] for r in wrong})}')
    for mode, want in (('content', ('differs-content',)), ('raw', ('differs-content', 'differs-structure-only', 'candidate-crash')), ('log', ('differs-log-only',)), ('aux', ('differs-structure-only',))):
        part = by.get((f'w{mode}', 'last-cold'), [])
        # A plain TeX document writes no .aux, and that mutation has nothing to do there.
        if mode == 'aux':
            part = [r for r in part if r['id'] != 'ladder/00-plain-hello' and not r['id'].startswith('knuth/')]
        verdicts = sorted({r['check'] for r in part})
        chk(f'wrong-byte: {mode}: every row says so', part and all(r['check'] in want for r in part), f'{len(part)} rows: {verdicts}')
    first = by.get(('wcontent', 'first'), [])
    chk('wrong-byte: on a run with no oracle to hold it against too', first and all(r['check'] == 'differs-content' for r in first), f'{len(first)} rows: {sorted({r["check"] for r in first})}')
    chk('wrong-byte: the right engine is right on those', all(r['check'] == 'identical' for r in by[('a', 'last-cold')] + by[('a', 'first')]), sorted({r['check'] for r in by[('a', 'last-cold')] + by[('a', 'first')]}))
    res = report.analyze([out], SET, None, 200)
    fig = res['tiers']['quick']['public']['figures']['last-cold']['by']
    w, a = fig['wcontent']['arch']['real'], fig['a']['arch']['real']
    chk('wrong-byte: none of its rows counts as identical', w['identical'] == 0 and w['n'] == len(ids), f'{w["identical"]} / {w["n"]}')
    chk('wrong-byte: it has no uncharged figure', w['uncharged'] is None, w['uncharged'])
    chk('wrong-byte: charged, it is over 1: what it took plus the baseline', w['charged'] > 1 and w['charged'] > a['charged'], f'{w["charged"]:.3f}, the right engine {a["charged"]:.3f}')
    named = {x['side'] for x in res['tiers']['quick']['public']['wrong']}
    chk('wrong-byte: its rows are named', 'wcontent' in named and 'a' not in named, sorted(named))


def case_killed(args, root, chk):
    ids = small_ids()
    out = os.path.join(root, 'kill.jsonl')
    argv = [sys.executable, '-I', BENCHSET, 'run', '--tier', 'quick', '--out', out, '--no-private', '--quiet', '--engine', 'a=' + args.engine, '--baselines', 'arch', '--method', 'counted', '--scenarios', 'last-cold', '--ids', ids_pattern(ids), '-j', '2']
    p = subprocess.Popen(argv, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    t0 = time.monotonic()
    while time.monotonic() - t0 < 300:
        if os.path.exists(out) and len(rows_of_tolerant(out)) >= 5:
            break
        if p.poll() is not None:
            break
        time.sleep(0.02)
    alive = p.poll() is None
    p.send_signal(signal.SIGKILL)
    p.wait()
    time.sleep(1.0)
    chk('killed: it was killed while it ran', alive)
    survivors = processes_naming(out)
    chk('killed: no worker of it is left', not survivors, len(survivors))
    before = rows_of_tolerant(out)
    with open(out, 'a') as f:
        f.write('{"kind": "m", "key": "a line a kill cut sho')
    bench(args, out, '--engine', 'a=' + args.engine, '--baselines', 'arch', '--method', 'counted', '--scenarios', 'last-cold', '--ids', ids_pattern(ids), '-j', '2')
    rows = rows_of(out)
    keys = [r['key'] for r in rows]
    expected = {f'{i}|last-cold||{s}|counted|0' for i in ids for s in ('a', 'arch')}
    chk('killed: no row twice', len(keys) == len(set(keys)), f'{len(keys)} rows, {len(set(keys))} keys')
    chk('killed: none missing', set(keys) == expected, f'{len(expected - set(keys))} missing, {len(set(keys) - expected)} extra')
    heads = rows_of(out, 'run')
    chk('killed: nothing measured again', heads[-1]['resumed_rows'] == len(before) and len(before) >= 1, f'{heads[-1]["resumed_rows"]} rows there on resuming, of {len(rows)}')
    with open(out) as f:
        chk('killed: the line cut short is gone', not any('cut sho' in line for line in f))
    return out


def rows_of_tolerant(path):
    res = []
    with open(path) as f:
        for line in f:
            try:
                r = json.loads(line)
            except ValueError:
                continue
            if r.get('kind') == 'm':
                res.append(r)
    return res


def processes_naming(text):
    found = []
    for name in os.listdir('/proc'):
        if not name.isdigit():
            continue
        try:
            with open(f'/proc/{name}/cmdline', 'rb') as f:
                if text.encode() in f.read():
                    found.append(int(name))
        except OSError:
            pass
    return found


def case_refused(args, root, chk, base):
    real = tool_rstex(args.engine)
    other = padding_wrapper(root, real)
    ids = small_ids()
    p = bench(args, base, '--engine', 'a=' + other, '--baselines', 'arch', '--method', 'counted', '--scenarios', 'last-cold', '--ids', ids_pattern(ids), check=False)
    chk('refused: a run stops on a file with another binary under that name', p.returncode != 0 and 'another binary' in p.stderr, f'exit {p.returncode}')
    # A document's rows on another input: the run leaves it out, and says so.
    tampered = os.path.join(root, 'tampered.jsonl')
    with open(base) as f:
        lines = f.readlines()
    doc = ids[0]
    kept = []
    done = False
    for line in lines:
        r = json.loads(line)
        if r.get('kind') == 'm' and r['id'] == doc and r['side'] == 'a':
            continue
        if r.get('kind') == 'm' and r['id'] == doc and not done:
            r['source_closure'] = '0' * 20
            line = json.dumps(r) + '\n'
            done = True
        kept.append(line)
    with open(tampered, 'w') as f:
        f.writelines(kept)
    p = bench(args, tampered, '--engine', 'a=' + args.engine, '--baselines', 'arch', '--method', 'counted', '--scenarios', 'last-cold', '--ids', ids_pattern([doc]), check=False)
    fresh = [r for r in rows_of(tampered) if r['id'] == doc and r['side'] == 'a']
    chk('refused: a run leaves out a document whose rows are on another input hash', p.returncode == 3 and not fresh, f'exit {p.returncode}, {len(fresh)} new rows')
    held = subprocess.run([sys.executable, '-I', BENCHSET, 'run', '--tier', 'holdout', '--out', os.path.join(root, 'holdout.jsonl'), '--engine', 'a=' + args.engine], capture_output=True, text=True, check=False)
    chk('refused: the hold-out asks whether this is the one time', held.returncode != 0 and 'frozen' in held.stderr and not os.path.exists(os.path.join(root, 'holdout.jsonl.lock')), f'exit {held.returncode}')
    # The report: one row of each kind, in a copy.
    copy = os.path.join(root, 'tampered2.jsonl')
    rows = []
    with open(base) as f:
        for line in f:
            rows.append(json.loads(line))
    ms = [r for r in rows if r.get('kind') == 'm' and r['side'] == 'a']
    ms[0]['binary_sha'] = 'f' * 20
    ms[1]['source_closure'] = '0' * 20
    ms[2]['tree'] = '0' * 20
    with open(copy, 'w') as f:
        for r in rows:
            f.write(json.dumps(r) + '\n')
    res = report.analyze([copy], SET, None, 100)
    chk('refused: the report refuses and counts a row from another binary', res['refused'].get('from another binary') == 1, res['refused'])
    chk('refused: the report refuses and counts a row on another input', res['refused'].get('on another input or tree of the document') == 1, res['refused'])
    chk('refused: the report refuses and counts a row on another tree', res['refused'].get('on another TeX tree') == 1, res['refused'])
    fig = res['tiers']['quick']['public']['figures']['last-cold']['by']['a']['arch']['real']
    chk('refused: they are out of the figure', fig['n'] == len(ids) - 3, f'{fig["n"]} of {len(ids)} rows')
    # Two binaries under one name in two headers are no side at all.
    twin = os.path.join(root, 'twin.jsonl')
    with open(twin, 'w') as f:
        for r in rows:
            f.write(json.dumps(r) + '\n')
        for r in rows:
            if r.get('kind') == 'run':
                r = dict(r, sides={k: dict(v, sha='e' * 20) for k, v in r['sides'].items()})
                f.write(json.dumps(r) + '\n')
    q = subprocess.run([sys.executable, '-I', os.path.join(HERE, 'report.py'), twin, '--resamples', '10', '--md', os.path.join(root, 'twin.md')], capture_output=True, text=True, check=False)
    chk('refused: a file with two binaries under one name gives no report', q.returncode != 0 and 'two binaries' in q.stderr, f'exit {q.returncode}')


# What a string in a row may be.  A private document's text could only come in by one of these, so each is either fixed by benchset.py or of a form that holds no text.
FIXED_WORDS = {'first', 'last-cold', 'warm', 'hits', 'resume', 'edit', 'fmt', 'pixels', 'diagram-alone', 'native', 'old-map', 'write', 'restore', 'last', 'settle', 'word-start', 'word-middle', 'word-end', 'sentence', 'cite', 'preamble', 'noop', 'pgf', 'a1', 'a2', 'direct', 'direct-aa', 'latexmk', 'poppler', 'counted', 'timed',
               'identical', 'differs-content', 'differs-structure-only', 'differs-log-only', 'candidate-crash', 'candidate-timeout', 'harness-error', 'reference', 'no-reference', 'not-applicable', 'not-comparable', 'taken-back', 'unavailable', 'same-picture', 'differs-picture', 'declined',
               'normalized', 'compare_pdf', 'memo', 'none', 'status', 'no-pdf', 'picture', 'log', 'equal', 'differs', 'differs-unstable', 'same', 'names another map', 'quick', 'standard', 'holdout', 'full', 'bibtex', 'biber', 'makeindex'}
HASH = re.compile(r'^[0-9a-f]{20}$')


def scan_rows(path, sides):
    """(rows, what breaks the rule, by the name of the field only): nothing of a value is said."""
    problems = []
    n = 0
    with open(path) as f:
        for line in f:
            r = json.loads(line)
            if r.get('kind') != 'm':
                continue
            n += 1
            private = r['id'].startswith(('user/', 'local/', 'cd-local/'))
            if private and 'checkpoint_said' in r:
                problems.append('checkpoint_said in a private row')
            for k, v in r.items():
                if k in ('kind', 'key', 'id', 'machine', 'governor', 'stratum', 'unavailable', 'tree', 'binary_sha', 'source_closure', 'tree_closure') or (k == 'checkpoint_said' and not private) or v is None or isinstance(v, (bool, int, float)):
                    continue
                if isinstance(v, str):
                    if k == 'side' and v in sides or v in FIXED_WORDS:
                        continue
                    problems.append(f'field {k}')
                elif isinstance(v, dict):
                    if k != 'tools' or not all(a in FIXED_WORDS or a in ('pdflatex',) or a.endswith('latex') or a == 'latexmk' or a.startswith(('bibtex', 'biber', 'makeindex')) for a in v):
                        problems.append(f'field {k}')
                else:
                    problems.append(f'field {k}')
            if not re.match(r'^[^|]+\|[^|]+\|[^|]*\|[^|]+\|(counted|timed)\|\d+$', r['key']):
                problems.append('the form of a key')
            for k in ('binary_sha', 'tree'):
                if r.get(k) and not HASH.match(r[k]):
                    problems.append(f'the form of {k}')
    return n, sorted(set(problems))


def case_no_text(args, root, chk):
    path = args.rows
    if not path:
        path = os.path.join(root, 'text.jsonl')
        bench(args, path, '--engine', 'a=' + args.engine, '--baselines', 'arch', '--method', 'counted', '--scenarios', 'last-cold,warm', '--ids', ids_pattern(small_ids()[:3]))
    sides = {'arch', 'official', 'commute', '*'} | {s for h in rows_of(path, 'run') for s in h['sides']}
    n, problems = scan_rows(path, sides)
    chk('no-text: a row holds nothing that could be a document\'s', n > 0 and not problems, f'{n} rows, {problems}')


def case_selection(args, root, chk):
    census = os.path.join(REPO, 'target', 'bs-design', 'census.jsonl')
    if not os.path.exists(census):
        chk.skip('selection', 'no census on this machine: target/bs-design/census.jsonl')
        return
    out = os.path.join(root, 'selection')
    os.makedirs(out, exist_ok=True)
    tool = os.path.join(HERE, 'select.py')
    p = subprocess.run([sys.executable, '-I', tool, '--out', out], capture_output=True, text=True, check=False)
    chk('selection: select.py runs', p.returncode == 0, f'exit {p.returncode}')
    with open(SET, 'rb') as f:
        committed = f.read()
    with open(os.path.join(out, 'set.json'), 'rb') as f:
        made = f.read()
    chk('selection: set.json comes out byte for byte', committed == made, f'{len(made)} bytes')
    local = os.path.join(HERE, 'set.local.json')
    if os.path.exists(local):
        with open(local, 'rb') as f:
            a = f.read()
        with open(os.path.join(out, 'set.local.json'), 'rb') as f:
            b = f.read()
        chk('selection: the private part too, by its bytes', a == b, f'{len(b)} bytes')
    bad = os.path.join(root, 'bad-census.jsonl')
    with open(census) as f, open(bad, 'w') as sink:
        for n, line in enumerate(f):
            if n == 17:
                r = json.loads(line)
                r['candidate_ms'] = 1.0
                line = json.dumps(r) + '\n'
            sink.write(line)
    p = subprocess.run([sys.executable, '-I', tool, '--census', bad, '--out', os.path.join(root, 'selection-bad')], capture_output=True, text=True, check=False)
    chk('selection: a census with a column of rstex\'s stops it', p.returncode != 0 and 'candidate_ms' in p.stderr, f'exit {p.returncode}')
    chk('selection: and it wrote nothing', not os.path.exists(os.path.join(root, 'selection-bad', 'set.json')))


def case_fast_check(args, root, chk):
    c = proflib.corpus()
    entries, _, _ = c.load_manifests(None)
    by = {e['id']: e for e in entries}
    s = bs.Rstex('real', args.engine)
    runner = bs.Runner()
    ids = small_ids()[:6]
    same = 0
    for i in ids:
        e = by[i]
        meta = c.load_valid_oracle(e)
        if meta is None:
            chk.skip(f'fast-check {i}', 'no valid oracle here')
            continue
        doc = proflib.Doc(i, e)
        doc.meta = meta
        w = c.new_workdir(meta.get('workdir_path'))
        out = os.path.join(w, 'o')
        c.restore_state(meta, out)
        argv, cwd, env = s.engine.command(doc, out)
        env = dict(env, XDG_CACHE_HOME=os.path.join(w, 'cache'), RSTEX_NO_SNAPSHOT='1', RSTEX_NO_PAGES='1')
        env = {k: v.replace('@OUT@', out) for k, v in env.items()}
        os.makedirs(env['XDG_CACHE_HOME'])
        runner.run(argv, cwd, env, limit=600)
        a, b = os.path.join(meta['dir'], 'final', meta['jobname'] + '.pdf'), os.path.join(out, meta['jobname'] + '.pdf')
        fast = bs.compare_pdfs(a, b)
        slow = c.run_compare_pdf(a, b)
        chk(f'fast-check: {i}: the cheap form and compare_pdf.py agree', fast == (None, 'normalized') and slow['status'] == 'identical', f'{fast}, {slow["status"]}')
        same += 1
        with open(b, 'rb') as f:
            data = bytearray(f.read())
        data[len(data) // 2] ^= 1
        m = os.path.join(w, 'm.pdf')
        with open(m, 'wb') as f:
            f.write(data)
        v, how = bs.compare_pdfs(a, m)
        chk(f'fast-check: {i}: a flipped byte does not pass', v is not None, f'{v} by {how}')
        shutil.rmtree(w, ignore_errors=True)
    runner.p.stdin.close()
    runner.p.wait()
    if not same:
        chk('fast-check: at least one document had an oracle', False)


def case_clock_guard(args, root, chk):
    ids = small_ids()[:2]
    out = os.path.join(root, 'clock.jsonl')
    bench(args, out, '--engine', 'a=' + args.engine, '--baselines', 'arch', '--method', 'timed', '--scenarios', 'last-cold', '--ids', ids_pattern(ids), '--max-load', '0', '--load-patience', '0')
    rows = rows_of(out)
    chk('clock-guard: rows were made', len(rows) >= 2 * 3, len(rows))
    chk('clock-guard: none of them has a clock', not any(r['clock_ok'] for r in rows), sum(1 for r in rows if r['clock_ok']))
    chk('clock-guard: instructions stand', all(r.get('instructions') for r in rows))
    chk('clock-guard: the load is in each row', all(r.get('load_before') is not None and r.get('load_after') is not None for r in rows))
    res = report.analyze([out], SET, None, 100)
    tab = res['tiers']['quick']['public']['figures']['last-cold']['by']['a']['arch']
    chk('clock-guard: the report gives no clock from them', 'clock' not in tab)
    chk('clock-guard: and gives the instructions', tab['real']['n'] == len(ids), tab['real']['n'])


def case_peak(args, root, chk, base):
    head = rows_of(base, 'run')[0]
    rows = rows_of(base)
    floor = head['floor']['maxrss_kb']
    worker_size = 60000
    chk('peak: what runs spawn from is small', floor < 20000, f'{floor} kB')
    small = [r['maxrss_kb'] for r in rows if r['side'] == 'a']
    chk('peak: some run of rstex is below what a worker holds, so no parent\'s size is in it', min(small) < worker_size, f'lowest {min(small)} kB')
    chk('peak: and a measured peak has its own value', len({r['maxrss_kb'] for r in rows}) > 3, f'{len({r["maxrss_kb"] for r in rows})} values')


def case_edit(args, root, chk):
    with open(SET) as f:
        entries = json.load(f)['entries']
    chosen = sorted((e for e in entries if 'quick' in e['tiers'] and e.get('edit')), key=lambda e: e['minstr'])
    if not chosen:
        chk.skip('edit', 'no document in quick has an edit session')
        return
    doc = chosen[0]
    out = os.path.join(root, 'edit.jsonl')
    bench(args, out, '--engine', 'a=' + args.engine, '--baselines', 'arch', '--method', 'counted', '--scenarios', 'edit', '--ids', ids_pattern([doc['id']]), '-j', '1')
    rows = {(r['variant'], r['side']): r for r in rows_of(out)}
    kinds = ['word-start', 'word-middle', 'word-end']
    chk('edit: each word changes the PDF on both sides', all(rows[(k, x)].get('pdf_changed') for k in kinds for x in ('a', 'arch') if rows[(k, x)].get('applied')), sorted({str(rows[(k, 'a')].get('pdf_changed')) for k in kinds}))
    chk('edit: a save of the same bytes changes nothing', rows[('noop', 'a')].get('pdf_changed') is False, rows[('noop', 'a')].get('pdf_changed'))
    chk('edit: every row answers', all('check' in r for r in rows.values()) and len(rows) == 2 * 8, len(rows))
    a = [r for (k, x), r in rows.items() if x == 'a' and r.get('applied')]
    chk('edit: what applied has instructions on rstex\'s side', all(r.get('instructions') for r in a), len(a))
    chk('edit: the document\'s input hash is the pin it was', all(r.get('source_closure') == doc['source_closure'] for r in rows.values()), doc['source_closure'])
    c = proflib.corpus()
    meta = c.load_valid_oracle(next(e for e in c.load_manifests(None)[0] if e['id'] == doc['id']))
    chk('edit: and its oracle is still valid against the files: no original changed', meta is not None)


def case_mirror(args, root, chk):
    base = os.path.join(root, 'mirror')
    shutil.rmtree(base, ignore_errors=True)
    orig = os.path.join(base, 'orig')
    os.makedirs(os.path.join(orig, 'sub', 'deep'))
    for rel, text in (('main.tex', b'main'), ('sub/deep/ch.tex', b'chapter'), ('other.tex', b'other')):
        with open(os.path.join(orig, rel), 'wb') as f:
            f.write(text)
    e = {'main': os.path.join(orig, 'main.tex'), 'cwd': orig, 'jobname': 'main'}
    dst = os.path.join(base, 'dst')
    rel = bs.mirror(e, dst)
    chk('mirror: the main file is a copy', rel == 'main.tex' and not os.path.islink(os.path.join(dst, rel)))
    chk('mirror: the rest are links', os.path.islink(os.path.join(dst, 'other.tex')) and os.path.islink(os.path.join(dst, 'sub')))
    try:
        bs.save(os.path.join(dst, 'other.tex'), b'through a link')
        chk('mirror: a save through a link is refused', False)
    except RuntimeError:
        chk('mirror: a save through a link is refused', True)
    p = bs.make_own(dst, 'sub/deep/ch.tex')
    bs.save(p, b'edited')
    with open(os.path.join(orig, 'sub', 'deep', 'ch.tex'), 'rb') as f:
        chk('mirror: an edit in a linked directory leaves the original', f.read() == b'chapter')
    with open(p, 'rb') as f:
        chk('mirror: and is there in the mirror', f.read() == b'edited')
    chk('mirror: a directory outside target is refused', refuses_outside())


def case_picture(args, root, chk):
    c = proflib.corpus()
    env = {k: os.environ[k] for k in ('PATH', 'HOME') if k in os.environ}
    env.update(c.DETERMINISM, **c.NO_MKTEX, **c.HERMETIC)
    base = os.path.join(root, 'picture')
    os.makedirs(base, exist_ok=True)
    pdfs = {}
    for name, body in (('a', rb'A \arrow[r] & B'), ('same', rb'A \arrow[r] & B'), ('label', rb'A \arrow[r, "f"] & B'), ('moved', rb'A \arrow[r, shift left=0.5pt] & B')):
        with open(os.path.join(base, name + '.tex'), 'wb') as f:
            f.write(bs.census.STANDALONE % (rb'\begin{tikzcd}' + body + rb'\end{tikzcd}'))
        subprocess.run(['pdflatex', '-interaction=batchmode', '-no-shell-escape', name + '.tex'], cwd=base, env=env, capture_output=True, check=False)
        pdfs[name] = os.path.join(base, name + '.pdf')
    same = bs.DiagramJob.same_picture
    chk('picture: the same diagram is the same picture', same(pdfs['a'], pdfs['same']) == 'same-picture', same(pdfs['a'], pdfs['same']))
    chk('picture: another label is another', same(pdfs['a'], pdfs['label']) != 'same-picture', same(pdfs['a'], pdfs['label']))
    chk('picture: an arrow half a point away is another', same(pdfs['a'], pdfs['moved']) != 'same-picture', same(pdfs['a'], pdfs['moved']))


def refuses_outside():
    try:
        bs.under_target('/tmp/not-a-target-directory')
    except RuntimeError:
        return True
    return False


def case_report(args, root, chk):
    with open(SET) as f:
        public = json.load(f)
    # Real public documents of standard from a kind with many, so that an interval exists.
    real = sorted((e for e in public['entries'] if 'standard' in e['tiers'] and e['kind'] == 'math' and e['size'] == 'paper' and not e.get('pgo')), key=lambda e: e['id'])[:14]
    test = next(e for e in public['entries'] if 'standard' in e['tiers'] and e['kind'] == 'testfile')
    standin = next(e for e in public['entries'] if 'standard' in e['tiers'] and e.get('synthetic'))
    head = {'kind': 'run', 'tier': 'standard', 'seed': 'x', 'machine': 'm', 'cpus': 1, 'governor': 'performance', 'load': 0.1, 'tree': 'T', 'tree_is_pin': True, 'floor': {'maxrss_kb': 1}, 'methods': ['counted'], 'jobs': 1, 'cpu': None, 'max_load': None, 'thin': None, 'missing_tools': [], 'set_sha': 's', 'resumed_rows': 0,
            'sides': {'a': {'sha': 'A', 'kind': 'rstex'}, 'official': {'sha': 'O', 'kind': 'pdftex'}, 'arch': {'sha': 'R', 'kind': 'pdftex'}}}

    def row(e, side, instr, check='identical', method='counted', **more):
        return dict({'kind': 'm', 'key': f'{e["id"]}|last-cold||{side}|{method}|0', 'id': e['id'], 'scenario': 'last-cold', 'variant': None, 'side': side, 'method': method, 'round': 0, 'tree': 'T', 'binary_sha': head['sides'][side]['sha'], 'source_closure': e['source_closure'], 'tree_closure': e['tree_closure'], 'pin_ok': True, 'instructions': instr, 'check': check, 'maxrss_kb': 1000, 'page_faults': 10, 'clock_ok': False}, **more)
    rows = []
    ratios = {}
    for k, e in enumerate(real):
        r = 0.2 + 0.01 * k
        ratios[e['id']] = r
        rows += [row(e, 'official', 1000), row(e, 'arch', 1100), row(e, 'a', 1000 * r)]
    # One wrong row, one crash, and a test and a stand-in, which pool with nothing.
    wrong, crash = real[0], real[1]
    rows = [x for x in rows if not (x['side'] == 'a' and x['id'] in (wrong['id'], crash['id']))]
    rows += [row(wrong, 'a', 500, 'differs-content'), row(crash, 'a', 100, 'candidate-crash')]
    rows += [row(test, 'official', 1000), row(test, 'arch', 1000), row(test, 'a', 10), row(standin, 'official', 1000), row(standin, 'arch', 1000), row(standin, 'a', 10)]
    path = os.path.join(root, 'made-up.jsonl')
    with open(path, 'w') as f:
        f.write(json.dumps(head) + '\n')
        for x in rows:
            f.write(json.dumps(x) + '\n')
    res = report.analyze([path], SET, None, 2000)
    part = res['tiers']['standard']['public']
    fig = part['figures']['last-cold']['by']['a']['official']
    r = fig['real']
    chk('report: N / M counts identical rows over all rows', r['identical'] == len(real) - 2 and r['n'] == len(real), f'{r["identical"]} / {r["n"]}')
    values = [ratios[e['id']] for e in real[2:]] + [(500 + 1000) / 1000, (100 + 1000) / 1000]
    chk('report: a wrong row is charged what it took plus the baseline, over the baseline', abs(r['charged'] - report.geomean(values)) < 1e-9, f'{r["charged"]:.6f}')
    chk('report: the uncharged figure holds right rows only', abs(r['uncharged'] - report.geomean([ratios[e['id']] for e in real[2:]])) < 1e-9, f'{r["uncharged"]:.6f}')
    chk('report: they are named', {x['id'] for x in part['wrong']} == {wrong['id'], crash['id']}, len(part['wrong']))
    chk('report: tests and stand-ins are apart', fig['tests']['n'] == 1 and fig['standins']['n'] == 1 and r['n'] == len(real))
    weights = [e['weight'] for e in real]
    ws = [(w, v) for w, v in zip(weights, [(500 + 1000) / 1000, (100 + 1000) / 1000] + [ratios[e['id']] for e in real[2:]])]
    chk('report: the pooled figure is by weight', abs(fig['pooled']['value'] - report.pooled_mean(ws)) < 1e-9, f'{fig["pooled"]["value"]:.6f}')
    chk('report: its interval is around it', fig['pooled']['interval'][0] <= fig['pooled']['value'] <= fig['pooled']['interval'][1], fig['pooled']['interval'])
    few = [x for x in fig['strata'].values() if x['n'] < 10]
    many = [x for x in fig['strata'].values() if x['n'] >= 10]
    chk('report: a stratum under ten has its rows and no interval', few and all('interval' not in x and len(x.get('rows', ())) == x['n'] for x in few), f'{len(few)} strata')
    chk('report: one of ten or more has an interval and no rows', many and all(x.get('interval') and 'rows' not in x for x in many), f'{len(many)} strata')
    a = report.bootstrap([1.0 + i / 100 for i in range(12)], 500, 'a')
    b = report.bootstrap([1.0 + i / 100 for i in range(12)], 500, 'a')
    c = report.bootstrap([1.0 + i / 100 for i in range(12)], 500, 'b')
    chk('report: an interval depends on its seed and on nothing else', a == b and a != c, a)
    chk('report: under ten documents, no interval', report.bootstrap([1.0] * 9, 500, 'a') is None)
    worst = fig['worst'][0]
    chk('report: the worst ten begin with the worst', worst['id'] == wrong['id'], worst)
    text = report.markdown(res)
    chk('report: Markdown says official first', text.find('| official |') < text.find('| arch |'))
    no_official = dict(res, baselines=['arch'])
    chk('report: a report against Arch\'s alone says so', 'Against Arch\'s pdfTeX alone' in report.markdown(no_official))
    # A wrong row made up with a private id stays out of a public document's section, by its own part.
    with open(os.path.join(root, 'made-up.json'), 'w') as f:
        json.dump(res, f, default=str)
    chk('report: it writes JSON a page can read', os.path.getsize(os.path.join(root, 'made-up.json')) > 100)
    chk('report: geomean', abs(report.geomean([2.0, 8.0]) - 4.0) < 1e-12 and math.isclose(report.pooled_mean([(1, 2.0), (3, 2.0)]), 2.0))


def case_common(args, root, chk):
    path = args.common or os.path.join(REPO, 'tests', 'until_stable', 'common.py')
    if not os.path.exists(path):
        chk.skip('common', 'tests/until_stable/common.py is not in this tree')
        return
    common = proflib.load_script('until_stable_common', path)
    chk('common: Lemma\'s environment', common.LEMMA_ENV == bs.LEMMA_ENV, common.LEMMA_ENV)
    chk('common: Lemma\'s flags', common.LEMMA_FLAGS == bs.LEMMA_FLAGS, common.LEMMA_FLAGS)
    chk('common: what rstex says of a program', common.TOOL_LINE.pattern == bs.TOOL_LINE.pattern)
    chk('common: what rstex says of its passes', common.PASSES_LINE.pattern == bs.PASSES_LINE.pattern)
    chk('common: what is not compared', common.NOT_COMPARED == bs.NOT_COMPARED, common.NOT_COMPARED)
    chk('common: the status that names a program', common.STATUS_TOOL == bs.STATUS_TOOL, common.STATUS_TOOL)
    chk('common: what a mirror leaves out', common.OUTPUTS == bs.OUTPUTS)
    # A mirror of a document made here, by both.
    base = os.path.join(root, 'mirror-common')
    orig = os.path.join(base, 'orig')
    os.makedirs(os.path.join(orig, 'sub'), exist_ok=True)
    for rel in ('main.tex', 'main.aux', 'sub/x.tex', '.hidden'):
        with open(os.path.join(orig, rel), 'w') as f:
            f.write('x')
    e = {'main': os.path.join(orig, 'main.tex'), 'cwd': orig, 'jobname': 'main'}
    a, b = os.path.join(base, 'a'), os.path.join(base, 'b')
    bs.mirror(e, a)
    os.makedirs(b, exist_ok=True)
    # common.py's mirror asks to be under its own target; this copy of it is the point of the comparison only where its tree has the name.
    try:
        common.mirror(e, b)
        chk('common: the same mirror', sorted(os.listdir(a)) == sorted(os.listdir(b)), sorted(os.listdir(a)))
    except RuntimeError:
        chk.skip('common: the same mirror', 'that copy of common.py is not in a tree with this target')


CASES = ('selection', 'report', 'mirror', 'picture', 'common', 'no-text', 'a-a', 'wrong-byte', 'killed', 'fast-check', 'clock-guard', 'edit')


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--only', help='names of cases, separated by commas; a-a holds slowed, killed holds refused and peak')
    ap.add_argument('-j', '--jobs', type=int, default=4)
    ap.add_argument('--common', help='another copy of tests/until_stable/common.py to compare with')
    ap.add_argument('--rows', help='for no-text: the rows of a real run, private documents and all')
    ap.add_argument('--keep', action='store_true')
    args = ap.parse_args()
    root = os.path.join(proflib.out_root(), 'bench-selfcheck', str(os.getpid()))
    os.makedirs(root)
    wanted = args.only.split(',') if args.only else list(CASES)
    chk = Check()
    t0 = time.monotonic()
    table = {'selection': case_selection, 'report': case_report, 'mirror': case_mirror, 'picture': case_picture, 'common': case_common, 'no-text': case_no_text, 'a-a': case_aa_slowed, 'wrong-byte': case_wrong_byte, 'fast-check': case_fast_check, 'clock-guard': case_clock_guard, 'edit': case_edit}
    for name in wanted:
        print(f'{name}:', flush=True)
        t1 = time.monotonic()
        if name == 'killed':
            base = case_killed(args, root, chk)
            case_refused(args, root, chk, base)
            case_peak(args, root, chk, base)
        else:
            table[name](args, root, chk)
        print(f'  {time.monotonic() - t1:.0f} s', flush=True)
    if not args.keep:
        shutil.rmtree(root, ignore_errors=True)
    print(f'{chk.passed} passed, {len(chk.failed)} failed, {chk.skipped} skipped, {time.monotonic() - t0:.0f} s')
    for name in chk.failed:
        print('failed:', name)
    print('PASS' if not chk.failed else f'FAIL: {len(chk.failed)}')
    sys.exit(1 if chk.failed else 0)


if __name__ == '__main__':
    main()
