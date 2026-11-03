#!/usr/bin/env -S python3 -I
r"""The commute library against real PGF, per diagram, with the boxes real PGF measured.  No engine in the loop.

    python3 -I tests/corpus/cd/compare_library.py instrument [-j N] [--filter REGEX]
    python3 -I tests/corpus/cd/compare_library.py compare --tool target/release/examples/libcheck [--filter REGEX] [--show N] [--policy-free]

`instrument` compiles every document of manifests cd.json and cd.local.json with the oracle, with tests/corpus/cd/library-instr.tex in front: streams uncompressed, and the
log holds the box of every node, the font parameters and the picture's box.  What it reads out goes to target/corpus/cd-library/<slug>/units.json.  Documents that have not
changed are not compiled again.

`compare` gives those boxes to Diagram::layout (crates/commute/examples/libcheck.rs), lets pdf::emit write the operators, and judges them against the oracle's page:

  box     minx, miny, maxx, maxy, baseline and the box's width, height and depth, to the scaled point
  paths   every stroke, fill and tip: the operator, the width, dash, cap, join, colours, and every coordinate, deviation in bp as the stream has them
  boxes   every cell and label: where its reference point lands, in bp, and its rotation and colour
  bytes   the operators pgf writes, token for token; what pdfTeX writes itself, the positions `1 0 0 1 x y cm`, and what is inside a box do not count

Policy is AllowAll here (spec section 1 of the report): this measures what the parser and layout do, so a macro that hides an arrow shows up as a diagram with too few arrows,
to be classified and not fixed.  For a private document (ids cd-local/) this prints counts and ids only.
"""
import argparse
import collections
import concurrent.futures as cf
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
ROOT = os.path.join(run.TARGET, 'cd-library')
INSTR = 'library-instr.tex'
SCHEMA = 6                  # bump when what `instrument` keeps changes
DIAGRAM = re.compile(rb'\\typeout\{\[\[unit-start ([^\]]+)\]\]\}\n\\setbox\\cdunit=\\hbox\{(.*?)\}\n\\typeout\{\[\[unit ', re.S)
BP_TOL = 0.05
HIST = (0.0, 0.0005, 0.001, 0.002, 0.005, 0.01, 0.02, 0.05)


def entries(args):
    files = [m for m in MANIFESTS if os.path.isfile(m)]
    es, _, _ = run.load_manifests(files)
    return [e for e in es if e.get('units') and (not args.filter or re.search(args.filter, e['id']))]


def is_private(entry_id):
    return entry_id.startswith('cd-local/')


# ------------------------------------------------------------------------------------------------------------------- reading the oracle

STREAM = re.compile(rb'(\d+) 0 obj\s*<<\s*/Length (\d+)\s*>>\s*stream\r?\n')
PAGE = re.compile(rb'/Type /Page\b(?!s)[^>]*?/Contents (\d+) 0 R', re.S)


def page_streams(pdf):
    b = open(pdf, 'rb').read()
    streams = {int(m.group(1)): (m.end(), int(m.group(2))) for m in STREAM.finditer(b)}
    pages = []
    for m in PAGE.finditer(b):
        n = int(m.group(1))
        if n in streams:
            a, ln = streams[n]
            pages.append(b[a:a + ln].decode('latin-1'))
        else:
            pages.append('')
    return pages


def dim(s):
    """`12.34pt` to scaled points, as TeX reads what it printed."""
    t = s.decode() if isinstance(s, bytes) else s
    if t.endswith('pt'):
        t = t[:-2]
    neg = t.startswith('-')
    if neg:
        t = t[1:]
    a, _, b = t.partition('.')
    # §102 round_decimals
    x = 0
    for c in reversed(b[:17]):
        x = (x + int(c) * 131072) // 10
    v = (int(a or 0) << 16) + (x + 1) // 2
    return -v if neg else v


def parse_log(data, units):
    """Per unit: font, cell boxes, label boxes, picture box, unit box, page number; lines are not wrapped under INST_ENV."""
    res, cur = {}, None
    shipped = 0         # \\count0 stays where it is under a bare \\shipout, so the page is counted here
    for ln in data.split(b'\n'):
        if cur is None or cur['dims'] is None or cur['page'] is None:
            # A message about the page may stand in front of its number on the line.
            at_end = cur is not None and cur['dims'] is not None
            if re.search(rb'(?:^| )\[\d+(?:[\]{ ]|$)', ln) if at_end else re.match(rb'\[\d', ln):
                shipped += 1
                if at_end:
                    cur['page'] = shipped
                continue
        if ln.startswith(b'[[unit-start '):
            name = ln[13:].split(b']')[0].decode()
            cur = res[name] = {'nodes': [], 'info': [], 'pic': None, 'dims': None, 'page': None, 'errors': 0}
        elif cur is None:
            continue
        elif ln.startswith(b'CDNODE|'):
            f = ln.split(b'|')
            if len(f) >= 6:
                cur['nodes'].append((f[1].decode('latin-1'), f[2].decode('latin-1'), dim(f[3]), dim(f[4]), dim(f[5])))
                cur['nodes'][-1] += (len(cur['info']),)
        elif ln.startswith(b'CDINFO|'):
            f = ln.split(b'|')
            cur['info'].append((f[1].decode('latin-1'), dim(f[2]), dim(f[3]), dim(f[4]), dim(f[5])))
        elif ln.startswith(b'CDPIC|'):
            f = ln.split(b'|')
            cur['pic'] = [dim(x) for x in f[1:6]]
        elif ln.startswith(b'[[unit '):
            m = re.match(rb'\[\[unit \S+ (\S*) (\S*) (\S*)\]\]', ln)
            if m:
                cur['dims'] = [dim(m.group(i)) for i in (1, 2, 3)]
        elif ln.startswith(b'! '):
            cur['errors'] += 1
    return res


def reduce_unit(u):
    """The nodes of one diagram: the last matrix's cells and what came after it."""
    if not u['info']:
        return None
    name, ex, em, rt, ax = u['info'][-1]
    k = len(u['info'])
    cells, labels = [], []
    pre = name + '-'
    for shape, nm, w, h, d, at in u['nodes']:
        if at == k - 1 and nm.startswith(pre):
            f = nm[len(pre):].split('-')
            if len(f) == 2 and f[0].isdigit() and f[1].isdigit():
                cells.append((int(f[0]), int(f[1]), w, h, d))
        elif at == k and shape != 'coordinate':
            labels.append((w, h, d))
    return {'font': (ex, em, rt, ax), 'cells': cells, 'labels': labels, 'pic': u['pic'], 'dims': u['dims'], 'page': u['page'], 'errors': u['errors']}


def source_diagrams(e):
    data = run.open_bytes(e['main'])
    return {m.group(1).decode(): m.group(2) for m in DIAGRAM.finditer(data)}, data


def balanced_arg(data, i):
    """data[i] is `{`: the index of its `}`."""
    depth, n = 0, len(data)
    while i < n:
        c = data[i]
        if c == 0x5c:
            i += 2
            continue
        if c == 0x7b:
            depth += 1
        elif c == 0x7d:
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return None


SETTERS = re.compile(rb'\\(tikzcdset|tikzset)\s*\{')


def settings_in(data):
    data = re.sub(rb'(?m)(?<!\\)%[^\n]*$', b'', data)
    parts = []
    for m in SETTERS.finditer(data):
        close = balanced_arg(data, m.end() - 1)
        if close is not None:
            parts.append(data[m.end():close])
    return parts


def presets(data, fls, cwd):
    """What the document's own files said to \\tikzcdset and \\tikzset before its first diagram, as one key list, in reading order: its preamble, what it
    read that is not in the tree, and the definitions in front of the first diagram.  This is text, so a call inside a macro counts too.  Inside the engine this is native's
    to find out, from the keys as they stand."""
    parts = []
    seen = set()
    main_done = False
    for ln in fls.split(b'\n'):
        if not ln.startswith(b'INPUT '):
            continue
        path = ln[6:].decode('utf-8', 'surrogateescape')
        if path.startswith(run.TDS_PREFIXES) or path in seen or not path.endswith(('.tex', '.sty', '.cls', '.ltx', '.def', '.cfg')) or path.endswith(os.sep + INSTR):
            continue
        seen.add(path)
        if os.path.basename(path) == 'diagrams.tex':
            main_done = True
            continue
        full = os.path.join(cwd, path)
        if os.path.isfile(full):
            parts.extend(settings_in(run.open_bytes(full)))
    head = data[:data.find(b'[[unit-start')] if b'[[unit-start' in data else data
    parts[:0] = settings_in(head) if main_done else []
    return b','.join(parts)


def instrument_entry(e):
    slug = run.slug(e['id'])
    d = os.path.join(ROOT, slug)
    done = os.path.join(d, 'done.json')
    key = run.sha_text(f'{SCHEMA} {run.config_hash(e)} {run.hash_file(os.path.join(HERE, INSTR))}')
    try:
        if run.read_json(done).get('key') == key:
            return e['id'], 'cached'
    except (OSError, ValueError):
        pass
    w = run.new_workdir()
    out = os.path.join(w, 'o')
    os.makedirs(out, exist_ok=True)
    try:
        env = run.entry_env(e)
        env.update(run.INST_ENV)
        env['TEXINPUTS'] = HERE + ':' + env.get('TEXINPUTS', ':')
        job = e['jobname']
        # The first \input would otherwise name the job.
        argv = run.tex_argv(e, shutil.which('pdflatex'), ['-jobname=' + job], out, recorder=True, prelude='\\input{%s}' % INSTR)
        r = run.run_proc(argv, e['cwd'], env, e['timeout'] * 3, os.path.join(w, 'stdout'))
        log = run.open_bytes(os.path.join(out, job + '.log'))
        units = parse_log(log, e['units'])
        pdf = os.path.join(out, job + '.pdf')
        pages = page_streams(pdf) if os.path.isfile(pdf) else []
        sources, data = source_diagrams(e)
        preset = presets(data, run.open_bytes(os.path.join(out, job + '.fls')), e['cwd']).decode('latin-1')
        res = {}
        for u in e['units']:
            x = units.get(u)
            if x is None:
                res[u] = {'absent': 'never reached'}
                continue
            y = reduce_unit(x)
            if y is None:
                res[u] = {'absent': 'no matrix'}
                continue
            y['stream'] = pages[y['page'] - 1] if y['page'] and y['page'] <= len(pages) else None
            y['source'] = sources[u].decode('latin-1') if u in sources else None
            res[u] = y
        tmp = d + '.tmp'
        shutil.rmtree(tmp, ignore_errors=True)
        os.makedirs(tmp)
        run.write_json(os.path.join(tmp, 'units.json'), {'preset': preset, 'units': res})
        run.write_json(os.path.join(tmp, 'done.json'), {'key': key, 'rc': r['rc']})
        shutil.rmtree(d, ignore_errors=True)
        os.rename(tmp, d)
        return e['id'], 'done'
    finally:
        shutil.rmtree(w, ignore_errors=True)


def instrument(args):
    os.makedirs(ROOT, exist_ok=True)
    run.install_limits()
    es = entries(args)
    tally = collections.Counter()
    with cf.ThreadPoolExecutor(args.jobs) as ex:
        for i, (ident, what) in enumerate(ex.map(instrument_entry, es)):
            tally[what] += 1
            if i % 50 == 49:
                print(f'{i + 1} of {len(es)}', flush=True)
    print(f'{len(es)} documents: {dict(tally)}')
    return 0


# --------------------------------------------------------------------------------------------------------------------- content streams

TOKEN = re.compile(rb'%[^\n]*|\[(?:[^\]\\()]|\\.|\((?:[^()\\]|\\.)*\))*\]|\((?:[^()\\]|\\.|\((?:[^()\\]|\\.)*\))*\)|<[0-9A-Fa-f]*>|/[^\s/\[\]()<>]+|[^\s\[\]()<>/%]+')
NUM = re.compile(rb'[-+]?\d*\.?\d+\Z')
COLOUR = {b'G', b'g', b'RG', b'rg', b'K', b'k'}


def op_list(stream):
    """The stream as (operator, operands), comments as operators."""
    ops, cur = [], []
    b = stream.encode('latin-1') if isinstance(stream, str) else stream
    for m in TOKEN.finditer(b):
        t = m.group()
        if t[:1] == b'%':
            ops.append((t, []))
        elif t[:1] in (b'[', b'(', b'<', b'/') or NUM.match(t):
            cur.append(t)
        else:
            ops.append((t, cur))
            cur = []
    return ops


def norm_operand(t):
    return b' '.join(t.split())


def mul(a, b):
    return (a[0] * b[0] + a[1] * b[2], a[0] * b[1] + a[1] * b[3], a[2] * b[0] + a[3] * b[2], a[2] * b[1] + a[3] * b[3],
            a[4] * b[0] + a[5] * b[2] + b[4], a[4] * b[1] + a[5] * b[3] + b[5])


def picture(ops):
    """Events and pgf's tokens of the first picture in `ops`, or None.  An event is ('path', op, gs, points) or ('box', x, y, a, b, c, d, colours)."""
    start = None
    for i, (op, args) in enumerate(ops):
        if op == b'w' and args == [b'0.3985'] and i >= 3 and ops[i - 3][0] == b'q' and ops[i - 2][0] in COLOUR and ops[i - 1][0] in COLOUR:
            start = i - 3
            break
    if start is None:
        return None
    ctm = (1.0, 0.0, 0.0, 1.0, 0.0, 0.0)
    gs = {'w': 0.3985, 'd': b'', 'J': 0, 'j': 0, 'RG': (), 'rg': ()}
    stack = []
    path = []
    events = []
    tokens = []
    i = start
    n = len(ops)
    depth = 0
    while i < n:
        op, args = ops[i]
        i += 1
        if op[:1] == b'%':
            continue
        if op == b'cm' and args[:4] == [b'1', b'0', b'0', b'1']:
            ctm = mul((1.0, 0.0, 0.0, 1.0, float(args[4]), float(args[5])), ctm)
            continue
        tokens.append(b' '.join([norm_operand(a) for a in args] + [op]))
        if op == b'q':
            stack.append((ctm, dict(gs)))
            depth += 1
        elif op == b'Q':
            ctm, gs = stack.pop()
            depth -= 1
            if depth == 0:
                break
        elif op == b'cm':
            v = tuple(float(a) for a in args[-6:])
            ctm = mul(v, ctm)
            # A box: `a b c d 0.0 0.0 cm`, `q`, colours.  The tips' `cm` is followed by their own settings.
            if v[4] == 0.0 and v[5] == 0.0 and i + 1 < n and ops[i][0] == b'q' and ops[i + 1][0] in COLOUR:
                colours = []
                j = i + 1
                while j < n and ops[j][0] in COLOUR:
                    colours.append(b' '.join(ops[j][1] + [ops[j][0]]))
                    j += 1
                events.append(('box', ctm[4], ctm[5], ctm[0], ctm[1], ctm[2], ctm[3], tuple(colours)))
                tokens.append(b'q')
                tokens.extend(colours)
                tokens.append(b'BOX')
                d = 1
                while j < n and d > 0:
                    if ops[j][0] == b'q':
                        d += 1
                    elif ops[j][0] == b'Q':
                        d -= 1
                    j += 1
                tokens.append(b'Q')
                i = j
        elif op == b'w':
            gs['w'] = float(args[-1])
        elif op == b'd':
            gs['d'] = norm_operand(args[0]) + b' ' + args[1]
        elif op == b'J':
            gs['J'] = int(args[-1])
        elif op == b'j':
            gs['j'] = int(args[-1])
        elif op in (b'G', b'RG', b'K'):
            gs['RG'] = tuple(float(a) for a in args)
        elif op in (b'g', b'rg', b'k'):
            gs['rg'] = tuple(float(a) for a in args)
        elif op in (b'm', b'l', b'c'):
            v = [float(a) for a in args]
            pts = []
            for k in range(0, len(v), 2):
                pts.append(ctm[0] * v[k] + ctm[2] * v[k + 1] + ctm[4])
                pts.append(ctm[1] * v[k] + ctm[3] * v[k + 1] + ctm[5])
            path.append((op, pts))
        elif op == b're':
            v = [float(a) for a in args]
            pts = [ctm[0] * v[0] + ctm[2] * v[1] + ctm[4], ctm[1] * v[0] + ctm[3] * v[1] + ctm[5], v[2], v[3]]
            path.append((op, pts))
        elif op == b'h':
            path.append((op, []))
        elif op in (b'S', b's', b'f', b'f*', b'B', b'B*', b'b', b'n', b'W', b'W*'):
            if path and op not in (b'W', b'W*'):
                if op != b'n':
                    events.append(('path', op, (gs['w'], gs['d'], gs['J'], gs['j'], gs['RG'], gs['rg']), path))
                path = []
    return events, tokens


def compare_pictures(a, b):
    """oracle, library: (class, deviation in bp, how many events)."""
    ea, ta = a
    eb, tb = b
    if len(ea) != len(eb):
        na = sum(1 for x in ea if x[0] == 'path')
        nb = sum(1 for x in eb if x[0] == 'path')
        return (f'{"fewer" if nb < na else "more"} paths than real PGF' if na != nb else 'boxes differ in number'), None, len(ea)
    worst = 0.0
    for x, y in zip(ea, eb):
        if x[0] != y[0]:
            return 'a path where real PGF has a box, or the reverse', None, len(ea)
        if x[0] == 'box':
            worst = max(worst, abs(x[1] - y[1]), abs(x[2] - y[2]))
            if max(abs(x[k] - y[k]) for k in (3, 4, 5, 6)) > 2e-5:
                return 'box rotated differently', None, len(ea)
            if x[7] != y[7]:
                return 'text colour differs', None, len(ea)
        else:
            if x[1] != y[1]:
                return 'path operator differs', None, len(ea)
            ga, gb = x[2], y[2]
            if abs(ga[0] - gb[0]) > 2e-5:
                return 'line width differs', None, len(ea)
            if ga[1] != gb[1] and not same_dash(ga[1], gb[1]):
                return 'dash differs', None, len(ea)
            if ga[2:4] != gb[2:4]:
                return 'cap or join differs', None, len(ea)
            if ga[4:6] != gb[4:6]:
                return 'path colour differs', None, len(ea)
            pa, pb = x[3], y[3]
            if len(pa) != len(pb) or any(p[0] != q[0] for p, q in zip(pa, pb)):
                return 'path segments differ', None, len(ea)
            for p, q in zip(pa, pb):
                for u, v in zip(p[1], q[1]):
                    worst = max(worst, abs(u - v))
    return 'ok', worst, len(ea)


def same_dash(a, b):
    na = [float(t) for t in a.replace(b'[', b' ').replace(b']', b' ').split()]
    nb = [float(t) for t in b.replace(b'[', b' ').replace(b']', b' ').split()]
    return len(na) == len(nb) and all(abs(p - q) < 2e-5 for p, q in zip(na, nb))


# ------------------------------------------------------------------------------------------------------------------------------ compare

def job_bytes(name, unit, preset):
    ex, em, rt, ax = unit['font']
    env = unit['source'].encode('latin-1')
    m = re.match(rb'\\begin\{tikzcd\}(.*)\\end\{tikzcd\}\Z', env, re.S)
    env = m.group(1) if m else env
    out = [f'JOB {name}\nFONT {ex} {em} {rt} {ax}\n'.encode()]
    for r, c, w, h, d in unit['cells']:
        out.append(f'CELL {r} {c} {w} {h} {d}\n'.encode())
    for w, h, d in unit['labels']:
        out.append(f'LABEL {w} {h} {d}\n'.encode())
    p = preset.encode('latin-1')
    out.append(f'PRESET {len(p)}\n'.encode() + p + b'\n')
    out.append(f'ENV {len(env)}\n'.encode() + env + b'\n')
    return b''.join(out)


def read_results(stdout):
    res = {}
    i, n = 0, len(stdout)
    cur = None
    while i < n:
        j = stdout.find(b'\n', i)
        if j < 0:
            break
        ln = stdout[i:j]
        i = j + 1
        if ln.startswith(b'RES '):
            f = ln.split(b' ', 3)
            cur = res[f[1].decode()] = {'kind': f[2].decode(), 'why': f[3].decode('latin-1') if len(f) > 3 else ''}
        elif ln.startswith(b'BOX '):
            cur['box'] = [int(x) for x in ln.split()[1:]]
        elif ln.startswith(b'OPS '):
            k = int(ln.split()[1])
            cur['ops'] = stdout[i:i + k]
            i += k + 1
    return res


def classify_box(unit, box):
    """The picture box to the scaled point."""
    pic = unit['pic']
    minx, miny, maxx, maxy, base, w, h, d = box
    if pic is None:
        return None
    worst = max(abs(minx - pic[0]), abs(miny - pic[1]), abs(maxx - pic[2]), abs(maxy - pic[3]), abs(base - pic[4]))
    if unit['dims']:
        worst = max(worst, abs(w - unit['dims'][0]), abs(max(h, 0) - unit['dims'][1]), abs(max(d, 0) - unit['dims'][2]))
    return worst


def compare(args):
    es = entries(args)
    jobs, meta = [], {}
    tally = collections.Counter()
    for e in es:
        try:
            data = run.read_json(os.path.join(ROOT, run.slug(e['id']), 'units.json'))
        except (OSError, ValueError):
            tally['not instrumented'] += len(e['units'])
            continue
        for u in e['units']:
            x = data['units'].get(u)
            name = f'{len(meta)}'
            if x is None or x.get('absent'):
                tally['absent from the oracle: ' + (x or {}).get('absent', 'no record')] += 1
                continue
            if x.get('errors'):
                tally['the oracle reports a TeX error on it'] += 1
                continue
            if x.get('stream') is None or x.get('source') is None or x.get('pic') is None:
                tally['no page or no source'] += 1
                continue
            meta[name] = (e['id'], u, x)
            jobs.append(job_bytes(name, x, data['preset']))
    p = subprocess.run([args.tool], input=b''.join(jobs), capture_output=True)
    if p.returncode != 0:
        print('the tool failed:', p.stderr.decode('latin-1')[-2000:])
        return 2
    results = read_results(p.stdout)

    hist = collections.Counter()
    sphist = collections.Counter()
    declined = collections.Counter()
    declined_public = collections.defaultdict(list)
    wrong = collections.defaultdict(list)
    bytes_equal = paths_compared = 0
    worst_list = []
    for name, (ident, unit, x) in meta.items():
        r = results.get(name)
        priv = is_private(ident)
        tag = f'{ident}#{unit}'
        if r is None:
            tally['the tool wrote nothing'] += 1
            continue
        if r['kind'] == 'declined':
            declined[r['why']] += 1
            declined_public[r['why']].append(tag)
            continue
        if r['kind'] == 'boxes':
            k = re.sub(r'\d+', 'N', r['why'])
            wrong['node count: ' + k].append(tag)
            continue
        a = picture(op_list(x['stream']))
        b = picture(op_list(r['ops']))
        if a is None or b is None:
            wrong['no picture in the ' + ('oracle' if a is None else 'library')].append(tag)
            continue
        cls, dev, nev = compare_pictures(a, b)
        paths_compared += nev
        spdev = classify_box(x, r['box'])
        if spdev is None:
            wrong['no picture box from the oracle'].append(tag)
        else:
            sphist[min(spdev, 1000) if spdev < 4 else ('4-65' if spdev < 66 else '66-655' if spdev < 656 else '>=656 (0.01pt)')] += 1
            if spdev:
                wrong_box = wrong['box not exact to the scaled point']
                wrong_box.append((spdev, tag))
        if cls != 'ok':
            wrong[cls].append(tag)
            continue
        worst_list.append((dev, tag))
        for h in HIST:
            if dev <= h:
                hist[h] += 1
                break
        else:
            hist['more'] += 1
        if a[1] == b[1]:
            bytes_equal += 1
    total = len(meta) + sum(tally.values())
    print(f'{total} diagrams; judged {len(meta)}')
    for k, v in tally.most_common():
        print(f'  {v:>5} {k}')
    judged = sum(hist.values())
    acc = len(meta) - sum(declined.values())
    print(f'declined by the library under AllowAll: {sum(declined.values())}')
    for k, v in declined.most_common():
        print(f'  {v:>5} {k}')
    print(f'accepted: {acc}')
    print(f'deviation of paths and box origins, over {judged} diagrams with the same drawing as real PGF:')
    for h in HIST:
        print(f'  <= {h:<7} bp {hist[h]:>5}')
    print(f'  >  {HIST[-1]:<7} bp {hist["more"]:>5}')
    print(f'operators byte-equal: {bytes_equal} of {judged}')
    print('picture box, worst of its numbers against real PGF, in scaled points:')
    for k in sorted(sphist, key=str):
        print(f'  {k!s:>16} {sphist[k]:>5}')
    print('classes:')
    for k, v in sorted(wrong.items(), key=lambda kv: -len(kv[1])):
        print(f'  {len(v):>5} {k}')
    if args.show:
        worst_list.sort(key=lambda t: -t[0])
        print(f'the {args.show} worst:')
        for dev, tag in worst_list[:args.show]:
            print(f'  {dev:.5f} {tag}')
        for k, v in sorted(wrong.items(), key=lambda kv: -len(kv[1])):
            print(f'-- {k}')
            for t in v[:args.show]:
                print(f'   {t}')
    if args.dump:
        os.makedirs(args.dump, exist_ok=True)
        run.write_json(os.path.join(args.dump, 'classes.json'), {'wrong': {k: [str(t) for t in v] for k, v in wrong.items()}, 'declined': declined_public, 'worst': sorted([(d, t) for d, t in worst_list], reverse=True)[:400]})
    bad = sum(len(v) for k, v in wrong.items()) + hist['more']
    return 0 if not bad else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest='cmd', required=True)
    p = sub.add_parser('instrument')
    p.add_argument('-j', '--jobs', type=int, default=4)
    p.add_argument('--filter')
    p.set_defaults(func=instrument)
    p = sub.add_parser('compare')
    p.add_argument('--tool', required=True)
    p.add_argument('--filter')
    p.add_argument('--show', type=int, default=0)
    p.add_argument('--dump', help='a directory under target/ for the lists of ids')
    p.set_defaults(func=compare)
    args = ap.parse_args()
    os.makedirs(run.WORK_ROOT, exist_ok=True)
    os.makedirs(run.EMPTY_TEXMF, exist_ok=True)
    return args.func(args)


if __name__ == '__main__':
    sys.exit(main())
